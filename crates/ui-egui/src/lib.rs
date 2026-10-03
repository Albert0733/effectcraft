//! The EffectCraft egui frontend (swappable): docking panels, the Composition viewer, the
//! Timeline with property twirl-downs and keyframes, Effect Controls, Effects & Presets, Preview,
//! Info, Character/Paragraph/Align, menus, dialogs and the control channel.
//!
//! Everything goes through `effectcraft_engine::Session::execute` (or `menus::invoke` for
//! frontend-only commands) so every gesture is also available to agents.

pub mod audio;
pub mod automation;
pub mod control;
pub mod dock;
pub mod dock_ui;
pub mod frames;
pub mod header;
pub mod icons;
pub mod menus;
pub mod native_menu;
pub mod panels;
pub mod state;
pub mod theme;
pub mod widgets;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

pub use control::ControlRequest;
use dock::PanelKind;
use effectcraft_engine::Session;
use effectcraft_engine::project::ItemId;
use effectcraft_engine::render::RenderOpts;
use effectcraft_engine::time::Tick;
use frames::{FrameKey, Frames, RenderSource};
use serde_json::json;
use state::UiState;
use theme::Tokens;

const SCREENSHOT_TIMEOUT_S: f64 = 4.0;

/// Modal dialogs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog {
    About,
    NewComp,
    CompSettings,
    SolidSettings,
    CommandPalette,
    /// Settings (Preferences) on `dialog_state.settings_page`.
    Settings,
    /// Edit ▸ Keyboard Shortcuts.
    Shortcuts,
    /// A generic parameter form for a command (`dialog_state.form`).
    Form,
    /// A message (`dialog_state.info`).
    Info,
    /// View ▸ View Options.
    ViewOptions,
    CameraSettings,
    LightSettings,
    KeyVelocity,
    KeyInterpolation,
    TimeStretch,
    /// Tracker ▸ Options…
    TrackOptions,
    /// Tracker ▸ Edit Target…
    TrackTarget,
    /// Tracker ▸ Apply (Transform / Stabilize): Apply Dimensions.
    TrackApply,
    /// Crash recovery: offer the latest auto-save (`EffectcraftApp::recovery`).
    Recovery,
    /// Composition/Layer Marker (double-click a marker).
    Marker,
    /// Layer ▸ Pre-compose.
    Precompose,
}

/// Host hooks provided by the native app (file pickers etc.).
#[derive(Default)]
pub struct Hooks {
    pub pick_files: Option<Box<dyn Fn(&[&str]) -> Vec<String>>>,
    pub pick_save: Option<Box<dyn Fn(&str) -> Option<String>>>,
    pub pick_open_project: Option<Box<dyn Fn() -> Option<String>>>,
    /// Opens the audio output for preview playback (the desktop app uses cpal).
    pub audio_device: Option<audio::AudioDeviceFactory>,
    /// Lists audio output devices (Settings ▸ Audio).
    pub audio_devices: Option<Box<dyn Fn() -> Vec<String>>>,
    /// Picks a folder (Settings paths).
    pub pick_folder: Option<Box<dyn Fn() -> Option<String>>>,
    /// Save dialog for other file kinds: (default name, extension).
    pub pick_save_file: Option<Box<dyn Fn(&str, &str) -> Option<String>>>,
    /// The system clipboard's text (native menu Edit ▸ Paste into a text field).
    pub clipboard_text: Option<Box<dyn Fn() -> Option<String>>>,
    /// Application actions the OS performs (`app.hide`, `app.hideOthers`, `app.showAll` on
    /// macOS). Returns false when the host doesn't handle the id.
    pub app_action: Option<Box<dyn Fn(&str) -> bool>>,
}

#[derive(Default)]
pub struct Playback {
    pub playing: bool,
    /// Wall-clock start and the comp time it corresponds to.
    pub start_wall: f64,
    pub start_time: Tick,
    pub start_frame: i64,
    /// Frames shown / dropped (for the Info/Preview readout).
    pub shown: u64,
    pub waiting: bool,
    pub fps: f64,
}

pub struct EffectcraftApp {
    pub session: Session,
    pub ui: UiState,
    pub tokens: Tokens,
    pub auto: automation::Registry,
    pub frames: Frames,
    pub playback: Playback,
    pub dialog: Option<Dialog>,
    pub hooks: Hooks,
    pub fps: f32,
    last_time: f64,
    styled: bool,
    fonts_ready: bool,
    pub(crate) control_rx: Option<Receiver<ControlRequest>>,
    pub(crate) deferred: Vec<(ControlRequest, f64)>,
    pub(crate) synthetic: Vec<egui::Event>,
    pub(crate) input_waiters: Vec<Sender<serde_json::Value>>,
    queued_screenshots: Vec<(u64, f64, u32)>,
    pending_screenshots: Vec<(u64, Option<String>, Option<[f32; 4]>, Sender<serde_json::Value>, f64)>,
    next_token: u64,
    pub(crate) last_ui_time: f64,
    pub(crate) toast: Option<(String, f64)>,
    /// Texture of the last CPU frame shown in the viewer and the key it came from.
    pub(crate) viewer_tex: Option<(egui::TextureHandle, FrameKey)>,
    /// The last GPU frame shown: its egui texture id (registered with egui-wgpu), key and texture.
    pub(crate) viewer_native: Option<(egui::TextureId, FrameKey, Arc<effectcraft_gpu::DisplayFrame>)>,
    /// The texture the viewer draws and its frame key (CPU or GPU frame).
    pub(crate) viewer_shown: Option<(egui::TextureId, FrameKey)>,
    /// Last displayed image (Info panel colour, eyedroppers, histograms). GPU frames fill it on
    /// demand ([`EffectcraftApp::viewer_pixels`]).
    pub(crate) viewer_image: Option<Arc<egui::ColorImage>>,
    /// egui-wgpu's device, once the first frame arrives (desktop and WebGPU).
    pub(crate) wgpu: Option<eframe::egui_wgpu::RenderState>,
    /// The GPU compositor on that device (None: no usable adapter → CPU only).
    pub(crate) gpu: Option<effectcraft_gpu::Gpu>,
    gpu_checked: bool,
    /// Commands from the native menu bar.
    pub command_inbox: Option<Receiver<String>>,
    /// Pointer position in comp pixels (Info panel).
    pub(crate) pointer_comp: Option<[f32; 2]>,
    pub(crate) dialog_state: panels::dialogs::DialogState,
    pub integrated_titlebar: bool,
    /// Audio preview while playing (audio clock drives playback).
    pub audio: Option<audio::AudioPlayback>,
    /// Audio panel VU meters.
    pub meter: audio::Meter,
    /// Waveform peak summaries per footage item (see `panels::waveform`).
    pub(crate) waveforms: panels::waveform::Cache,
    /// Last reveal shortcut and when (double-press shortcuts such as LL).
    pub(crate) last_reveal: Option<(String, f64)>,
    /// Settings revision applied to the theme, tooltips and caches.
    applied_prefs: Option<u64>,
    /// A previous run that didn't exit cleanly (shown by `Dialog::Recovery`).
    pub recovery: Option<effectcraft_engine::autosave::Recovery>,
    /// Docked groups laid out last frame: (active panel, group rect) — `~` maximizes the one
    /// under the pointer.
    pub(crate) dock_rects: Vec<(PanelKind, egui::Rect)>,
    /// Home screen: recent-project thumbnail textures by path (None = no thumbnail).
    pub(crate) home_thumbs: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    /// The (project path, saved revision) whose thumbnail was stored last.
    pub(crate) home_thumb_saved: Option<(String, u64)>,
}

impl EffectcraftApp {
    pub fn new(session: Session) -> Self {
        EffectcraftApp {
            session,
            ui: UiState::default(),
            tokens: Tokens::for_kind(Default::default()),
            auto: Default::default(),
            frames: Frames::default(),
            playback: Playback::default(),
            dialog: None,
            hooks: Hooks::default(),
            fps: 60.0,
            last_time: 0.0,
            styled: false,
            fonts_ready: false,
            control_rx: None,
            deferred: vec![],
            synthetic: vec![],
            input_waiters: vec![],
            queued_screenshots: vec![],
            pending_screenshots: vec![],
            next_token: 1,
            last_ui_time: 0.0,
            toast: None,
            viewer_tex: None,
            viewer_native: None,
            viewer_shown: None,
            viewer_image: None,
            wgpu: None,
            gpu: None,
            gpu_checked: false,
            command_inbox: None,
            pointer_comp: None,
            dialog_state: Default::default(),
            integrated_titlebar: false,
            audio: None,
            meter: Default::default(),
            waveforms: Default::default(),
            last_reveal: None,
            applied_prefs: None,
            recovery: None,
            dock_rects: vec![],
            home_thumbs: Default::default(),
            home_thumb_saved: None,
        }
        .with_ui_commands()
    }

    /// Offer the frontend's commands (tools, timeline navigation…) for keyboard shortcuts.
    fn with_ui_commands(mut self) -> Self {
        let cmds = menus::UI_COMMANDS
            .iter()
            .map(|c| effectcraft_engine::shortcuts::UiCommand { id: c.id.into(), label: c.label.into(), shortcut: c.shortcut.map(str::to_string) })
            .collect();
        self.session.set_ui_commands(cmds);
        self
    }

    /// Show the crash-recovery dialog for a previous run that didn't exit cleanly.
    pub fn offer_recovery(&mut self, r: effectcraft_engine::autosave::Recovery) {
        if r.autosave.is_some() {
            self.recovery = Some(r);
            self.dialog = Some(Dialog::Recovery);
        }
    }

    /// Apply changed settings: theme and brightness, label colours, tool tips, preview caches.
    pub fn apply_prefs(&mut self, ctx: &egui::Context) {
        if self.applied_prefs == Some(self.session.prefs_revision) {
            return;
        }
        self.applied_prefs = Some(self.session.prefs_revision);
        let p = &self.session.prefs;
        self.ui.theme = theme::ThemeKind::from_name(&p.appearance.theme).unwrap_or_default();
        self.tokens = Tokens::from_prefs(p);
        theme::apply_visuals(ctx, &self.tokens);
        let tips = p.general.show_tool_tips;
        ctx.all_styles_mut(|s| s.interaction.tooltip_delay = if tips { 0.5 } else { f32::INFINITY });
        self.ui.cache_when_idle = p.previews.cache_frames_when_idle;
        self.ui.viewer.fast_preview = p.previews.fast_previews;
        self.frames.set_budget(p.preview_cache_bytes());
    }

    /// Change settings from the UI (applied next frame and saved).
    pub fn set_pref(&mut self, key: &str, value: serde_json::Value) -> Result<(), String> {
        self.session.prefs.set(key, value)?;
        self.session.prefs_changed();
        self.session.save_prefs();
        Ok(())
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, k: theme::ThemeKind) {
        let name = match k {
            theme::ThemeKind::Dark => "dark",
            theme::ThemeKind::Darker => "darker",
            theme::ThemeKind::Light => "light",
        };
        let _ = self.set_pref("appearance.theme", json!(name));
        self.apply_prefs(ctx);
    }

    pub fn set_workspace(&mut self, name: &str) {
        self.ui.workspace = name.to_string();
        self.ui.dock = self.ui.saved_workspaces.get(name).cloned().unwrap_or_else(|| dock::workspace(name));
        self.ui.floating = self.ui.saved_floating.get(name).cloned().unwrap_or_else(|| dock::workspace_floating(name));
        self.ui.maximized = None;
        // Learn: the Home screen (community links) in the Composition panel.
        if name == "Learn" {
            self.ui.start_screen = true;
        }
    }

    /// Built-in workspaces followed by the saved ones (Window ▸ Workspace ▸ Save as New Workspace).
    pub fn workspace_names(&self) -> Vec<String> {
        let mut v: Vec<String> = dock::WORKSPACES.iter().map(|s| s.to_string()).collect();
        v.extend(self.ui.saved_workspaces.keys().filter(|k| !dock::WORKSPACES.contains(&k.as_str())).cloned());
        v
    }

    pub fn show_panel(&mut self, p: PanelKind) {
        if let Some(f) = self.ui.floating.iter_mut().find(|f| f.panels.contains(&p)) {
            f.active = f.panels.iter().position(|x| *x == p).unwrap_or(0);
            self.ui.focused = p;
            return;
        }
        if self.ui.maximized.is_some_and(|m| m != p) {
            self.ui.maximized = None;
        }
        if !self.ui.dock.contains(p) {
            let near = match p {
                PanelKind::Layer | PanelKind::Flowchart => PanelKind::Composition,
                PanelKind::RenderQueue => PanelKind::Timeline,
                PanelKind::EffectControls | PanelKind::History => PanelKind::Project,
                _ => PanelKind::EffectsPresets,
            };
            self.ui.dock.open_near(p, near);
        }
        self.ui.dock.activate(p);
        self.ui.focused = p;
    }

    pub fn render_source(&self) -> RenderSource {
        RenderSource {
            project: self.session.project.clone(),
            footage: self.session.footage.clone(),
            expr: self.session.expr.clone(),
            layer_cache: self.session.layer_cache.clone(),
            gpu: self.gpu.clone(),
            gpu_display: false,
        }
    }

    /// Set up the GPU compositor on egui-wgpu's device (once). Without an adapter that runs
    /// compute shaders (WebGL2, software GL) everything stays on the CPU.
    pub(crate) fn init_gpu(&mut self, frame: &eframe::Frame) {
        if self.gpu_checked {
            return;
        }
        self.gpu_checked = true;
        let Some(rs) = frame.wgpu_render_state() else { return };
        match effectcraft_gpu::Gpu::new(&rs.adapter, rs.device.clone(), rs.queue.clone()) {
            Ok(g) => {
                log::info!("GPU compositor: {}", effectcraft_engine::render::Accelerator::name(&g));
                self.session.accel = Some(Arc::new(g.clone()));
                self.gpu = Some(g);
                self.wgpu = Some(rs.clone());
            }
            Err(e) => log::info!("GPU compositor unavailable: {e}"),
        }
    }

    /// The viewer shows a frame composited on the GPU (drawn from its wgpu texture).
    pub fn viewer_on_gpu(&self) -> bool {
        matches!((&self.viewer_shown, &self.viewer_native), (Some(s), Some(n)) if s.1 == n.1)
    }

    /// The GPU compositor's adapter, when the viewer has one.
    pub fn gpu_adapter(&self) -> Option<String> {
        self.gpu.as_ref().map(effectcraft_engine::render::Accelerator::name)
    }

    /// The viewer's current pixels as 8-bit premultiplied RGBA, reading a GPU frame back the
    /// first time something asks for it.
    pub fn viewer_pixels(&mut self) -> Option<Arc<egui::ColorImage>> {
        if self.viewer_image.is_none()
            && let (Some((_, _, f)), Some(g)) = (&self.viewer_native, &self.gpu)
            && self.viewer_shown.as_ref().map(|s| s.1) == self.viewer_native.as_ref().map(|n| n.1)
            && let Some(bytes) = g.read_display(f)
        {
            let px = bytes.chunks_exact(4).map(|c| egui::Color32::from_rgba_premultiplied(c[0], c[1], c[2], c[3])).collect();
            self.viewer_image = Some(Arc::new(egui::ColorImage::new([f.width as usize, f.height as usize], px)));
        }
        self.viewer_image.clone()
    }

    /// The render scale used by the viewer right now.
    pub fn viewer_scale(&self, zoom: f32, ppp: f32) -> f64 {
        if self.playback.playing && self.ui.viewer.res == state::Resolution::Auto {
            // Keep previews snappy: half the displayed size, but no lower than Settings ▸
            // Previews ▸ Adaptive Resolution Limit.
            let full = self.ui.viewer.res.scale(zoom, ppp);
            return full.min((full * 0.5).max(self.session.prefs.adaptive_limit()));
        }
        let full = self.ui.viewer.res.scale(zoom, ppp);
        // Fast Previews ▸ Adaptive Resolution / Fast Draft: lower resolution while dragging.
        let (_, k) = self.session.state.viewer.fast_previews.render(self.ui.viewer.interacting);
        if k < 1.0 && self.ui.viewer.res == state::Resolution::Auto {
            return full.min((full * 0.5).max(self.session.prefs.adaptive_limit()));
        }
        full
    }

    pub fn frame_key(&self, comp: ItemId, frame: i64, scale: f64) -> FrameKey {
        FrameKey { revision: self.session.revision, comp: comp.0, frame, scale: (scale * 1000.0).round() as u32, view: self.view_hash(comp) }
    }

    /// Hash of the comp viewer's 3D view camera (0 for the active camera view).
    pub fn view_hash(&self, comp: ItemId) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // The region of interest changes what is rendered too.
        let roi = self.session.state.region_of_interest.filter(|_| self.session.active_comp_id() == Some(comp));
        if let Some(r) = roi {
            r.map(f64::to_bits).hash(&mut h);
        }
        let Some(cam) = self.session.view_camera(comp) else { return if roi.is_some() { h.finish() | 1 } else { 0 } };
        for row in cam.view.0 {
            for v in row {
                v.to_bits().hash(&mut h);
            }
        }
        cam.zoom.to_bits().hash(&mut h);
        cam.ortho.hash(&mut h);
        h.finish() | 1
    }

    /// Request a prefetch frame render (no-op if cached/in flight).
    pub fn request_frame(&self, comp: ItemId, frame: i64, scale: f64) {
        self.request_frame_with(comp, frame, scale, false);
    }

    /// Request the frame on screen: rendered before any queued prefetch.
    pub fn request_frame_urgent(&self, comp: ItemId, frame: i64, scale: f64) {
        self.request_frame_with(comp, frame, scale, true);
    }

    fn request_frame_with(&self, comp: ItemId, frame: i64, scale: f64, urgent: bool) {
        let Some(c) = self.session.project.comp(comp) else { return };
        let key = self.frame_key(comp, frame, scale);
        let t = c.frame_rate.tick_of(frame);
        let (draft, _) = self.session.state.viewer.fast_previews.render(self.ui.viewer.interacting);
        let roi = self.session.state.region_of_interest.filter(|_| self.session.active_comp_id() == Some(comp));
        let opts = RenderOpts {
            scale,
            motion_blur: true,
            guides: true,
            draft: self.ui.viewer.fast_preview || draft,
            view: self.session.view_camera(comp),
            roi,
            backend: effectcraft_engine::render::Backend::Auto,
        };
        if urgent {
            self.frames.request_urgent(&self.render_source(), key, comp, t, opts);
        } else {
            self.frames.request(&self.render_source(), key, comp, t, opts);
        }
    }

    // ---------------------------------------------------------------- playback

    pub fn play(&mut self, now: f64) {
        let Some(c) = self.session.active_comp().cloned() else { return };
        let mut t = self.session.time();
        let (wa, wb) = c.work_area;
        if t >= wb - c.frame_duration() || t < wa {
            t = wa;
        }
        self.session.set_time(t);
        self.playback = Playback {
            playing: true,
            start_wall: now,
            start_time: t,
            start_frame: c.frame_rate.frame_at(t),
            shown: 0,
            waiting: false,
            fps: c.frame_rate.as_f64(),
        };
        self.start_audio(t);
    }

    /// Start audio preview from comp time `t` when the Preview panel includes audio, the comp
    /// has something audible and an output device opens.
    fn start_audio(&mut self, t: Tick) {
        self.audio = None;
        self.meter.clipped = [false; 2];
        if !self.ui.preview_audio {
            return;
        }
        let Some(cid) = self.session.active_comp_id() else { return };
        let Some(c) = self.session.project.comp(cid).cloned() else { return };
        if !effectcraft_engine::render::audio::comp_has_audio(&self.session.project, cid) {
            return;
        }
        let out = audio::AudioOutput::from_prefs(&self.session.prefs);
        let Some(dev) = self.hooks.audio_device.as_ref().and_then(|f| f(&out)) else { return };
        match audio::AudioPlayback::start(dev, self.render_source(), cid, t, c.work_area.0, c.work_area.1, self.ui.preview_loop) {
            Ok(a) => self.audio = Some(a),
            Err(e) => log::warn!("audio preview: {e}"),
        }
    }

    pub fn stop(&mut self) {
        self.playback.playing = false;
        self.audio = None;
    }

    pub fn toggle_play(&mut self, now: f64) {
        if self.playback.playing { self.stop() } else { self.play(now) }
    }

    /// RAM preview: advance at the comp frame rate when frames are cached; otherwise hold and
    /// keep rendering ahead (the green cache bar fills, then playback runs in real time).
    fn advance_playback(&mut self, ctx: &egui::Context, scale: f64) {
        let Some(cid) = self.session.active_comp_id() else { return };
        let Some(c) = self.session.project.comp(cid).cloned() else { return };
        let now = ctx.input(|i| i.time);
        let fr = c.frame_rate;
        let (wa, wb) = (fr.frame_at(c.work_area.0), fr.frame_at(c.work_area.1 - c.frame_duration()));
        // Prefetch ahead.
        let cur = fr.frame_at(self.session.time());
        // While paused (scrubbing, editing) the viewer's frame comes first: prefetch only a few
        // frames, and only once it is done, so prefetch never delays what is on screen.
        let ahead = if self.playback.playing {
            (self.frames_parallelism() * 2).max(4) as i64
        } else if self.frames.urgent_pending() {
            0
        } else {
            (self.frames_parallelism() / 2).max(2) as i64
        };
        let mut queued = self.frames.inflight();
        for k in 0..ahead * 3 {
            if queued >= ahead as usize {
                break;
            }
            let mut f = cur + k;
            if f > wb {
                if !self.ui.preview_loop {
                    break;
                }
                f = wa + (f - wb - 1).rem_euclid((wb - wa + 1).max(1));
            }
            let key = self.frame_key(cid, f, scale);
            if !self.frames.is_cached(&key) {
                self.request_frame(cid, f, scale);
                queued += 1;
            }
        }
        if let Some(a) = &self.audio {
            self.meter.update(a.feed.take_peaks(), now);
        } else if self.meter.active() {
            self.meter.update([0.0; 2], now);
            ctx.request_repaint();
        }
        if !self.playback.playing {
            return;
        }
        // Audio clock drives playback: show the frame under the audible sample (frames that
        // are not rendered yet are dropped, never delayed).
        if let Some(a) = &self.audio {
            if a.finished() {
                self.session.set_time(fr.tick_of(wb));
                self.stop();
                return;
            }
            let target = audio::frame_of_sample(a.clock(), a.rate, fr).clamp(wa, wb);
            self.playback.waiting = !self.frames.is_cached(&self.frame_key(cid, target, scale));
            if target != cur {
                self.session.set_time(fr.tick_of(target));
                self.playback.shown += 1;
            }
            ctx.request_repaint();
            return;
        }
        let elapsed = now - self.playback.start_wall;
        let mut target = self.playback.start_frame + (elapsed * fr.as_f64()).floor() as i64;
        if target > wb {
            if self.ui.preview_loop {
                target = wa + (target - wa).rem_euclid((wb - wa + 1).max(1));
            } else {
                self.session.set_time(fr.tick_of(wb));
                self.stop();
                return;
            }
        }
        let key = self.frame_key(cid, target, scale);
        if self.frames.is_cached(&key) {
            self.playback.waiting = false;
            if target != cur {
                self.session.set_time(fr.tick_of(target));
                self.playback.shown += 1;
            }
        } else {
            // Not cached yet: hold the clock at the current frame (cache first, then play).
            self.playback.waiting = true;
            let next = cur + 1;
            let next = if next > wb { wa } else { next };
            if self.frames.is_cached(&self.frame_key(cid, next, scale)) {
                self.session.set_time(fr.tick_of(next));
            }
            self.playback.start_wall = now;
            self.playback.start_frame = fr.frame_at(self.session.time());
        }
        ctx.request_repaint();
    }

    fn frames_parallelism(&self) -> usize {
        if cfg!(target_arch = "wasm32") {
            // frames render one at a time on the UI thread (`Frames::pump`)
            return 1;
        }
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
    }

    // ---------------------------------------------------------------- control channel

    pub(crate) fn raise_for_control(&mut self, ctx: &egui::Context) {
        ctx.request_repaint();
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        let now = ctx.input(|i| i.time);
        let mut reqs: Vec<(ControlRequest, f64)> = std::mem::take(&mut self.deferred);
        while let Ok(req) = rx.try_recv() {
            reqs.push((req, now + 3.0));
        }
        for (req, deadline) in reqs {
            let reply = req.reply.clone();
            match control::handle(self, ctx, &req) {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::Retry(msg) => {
                    if now < deadline {
                        self.deferred.push((req, deadline));
                        ctx.request_repaint();
                    } else {
                        let _ = reply.send(json!({"ok": false, "error": msg}));
                    }
                }
                control::Outcome::AfterInput => self.input_waiters.push(reply),
                control::Outcome::Screenshot { path, crop } => {
                    // A covered or minimized window presents no frames (macOS skips its redraws), so
                    // a screenshot would never arrive and nothing would tick its timeout.
                    if ctx.input(|i| i.viewport().occluded == Some(true) || i.viewport().minimized == Some(true)) {
                        let _ = reply.send(json!({"ok": false, "error": "window is covered or minimized: call ui.focus first (render.frame renders comp pixels without the window)"}));
                        continue;
                    }
                    let token = self.next_token;
                    self.next_token += 1;
                    let settle = now + 0.35;
                    self.queued_screenshots.push((token, settle, 0));
                    self.pending_screenshots.push((token, path, crop, reply, settle + SCREENSHOT_TIMEOUT_S));
                }
            }
        }
        self.control_rx = Some(rx);
    }

    fn issue_screenshots(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let mut any = false;
        let busy = self.frames.inflight() > 0;
        self.queued_screenshots.retain_mut(|(token, at, frames)| {
            *frames += 1;
            any = true;
            // Wait for the viewer's frame to finish rendering (bounded by the timeout below).
            if now >= *at && *frames >= 3 && (!busy || now > *at + 2.5) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(*token)));
                false
            } else {
                true
            }
        });
        self.pending_screenshots.retain(|(.., reply, deadline)| {
            if now > *deadline {
                let _ = reply.send(json!({"ok": false, "error": "no frame was presented (window hidden or display asleep)"}));
                false
            } else {
                true
            }
        });
        if any {
            ctx.request_repaint();
        } else if !self.pending_screenshots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn collect_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_screenshots.is_empty() {
            return;
        }
        let events: Vec<_> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => {
                        let token = user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).copied()?;
                        Some((token, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (token, image) in events {
            if let Some(i) = self.pending_screenshots.iter().position(|(t, ..)| *t == token) {
                let (_, path, crop, reply, _) = self.pending_screenshots.remove(i);
                let r = control::save_screenshot(ctx, &image, path.as_deref(), crop);
                let _ = reply.send(r);
            }
        }
    }

    // ---------------------------------------------------------------- frame

    fn handle_events(&mut self, ctx: &egui::Context) {
        for ev in self.session.drain_events() {
            match ev {
                effectcraft_engine::Event::OpenComp(_) => {
                    self.ui.dock.activate(PanelKind::Composition);
                    self.ui.timeline.pps = None;
                }
                effectcraft_engine::Event::Toast { message, .. } => self.toast = Some((message, ctx.input(|i| i.time))),
                effectcraft_engine::Event::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
                effectcraft_engine::Event::ProjectChanged { .. } => {}
                effectcraft_engine::Event::Frontend { command, params } => {
                    if let Err(e) = crate::menus::frontend(self, ctx, &command, params) {
                        self.ui.status = e;
                    }
                }
                effectcraft_engine::Event::PurgeCaches => self.frames.clear(),
            }
        }
    }

    fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.auto.begin_frame();
        self.frames.set_context(&ctx);
        if self.session.render_job.is_some() {
            self.session.poll_render();
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.session.track_job.is_some() {
            self.session.poll_track();
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.session.mask_job.is_some() {
            self.session.poll_mask_track();
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // Warp Stabilizer: finish analyses and start queued (re-)analyses in the background.
        if self.session.warp_job.is_some() || !self.session.warp_pending.is_empty() {
            self.session.poll_warp(true);
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.apply_prefs(&ctx);
        self.handle_events(&ctx);
        self.tick_autosave(&ctx);
        if let Some(rx) = self.command_inbox.take() {
            while let Ok(id) = rx.try_recv() {
                if let Err(e) = menus::invoke(self, &ctx, &id, json!({})) {
                    self.ui.status = e;
                }
            }
            self.command_inbox = Some(rx);
        }
        menus::handle_shortcuts(self, &ctx);
        let t = self.tokens;
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, t.app_bg);
        let mut top = full.min.y;
        if self.ui.show_menu_bar {
            let mb = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), 24.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(mb));
            child.painter().rect_filled(mb, 0.0, t.header_bg);
            menus::menu_bar(self, &mut child);
            top = mb.max.y;
        }
        let header_h = 40.0;
        let header = egui::Rect::from_min_size(egui::pos2(full.min.x, top), egui::vec2(full.width(), header_h));
        header::show(self, ui, header);
        let body = egui::Rect::from_min_max(egui::pos2(full.min.x + 4.0, header.max.y + 2.0), egui::pos2(full.max.x - 4.0, full.max.y - 4.0));
        self.dock_area(ui, body);
        panels::precomp::mini_flowchart(self, &ctx);
        panels::home::capture_thumbnail(self);
        panels::dialogs::show(self, &ctx);
        self.draw_toast(ui, full);
    }

    fn draw_toast(&mut self, ui: &mut egui::Ui, full: egui::Rect) {
        let now = ui.input(|i| i.time);
        let msg = if !self.ui.status.is_empty() {
            Some(self.ui.status.clone())
        } else {
            self.toast.as_ref().filter(|(_, at)| now - at < 4.0).map(|(m, _)| m.clone())
        };
        if let Some(m) = msg {
            let t = &self.tokens;
            let galley = ui.painter().layout_no_wrap(m, Tokens::ui(12.0), t.text);
            let r = egui::Rect::from_min_size(egui::pos2(full.min.x + 16.0, full.max.y - 44.0), galley.size() + egui::vec2(24.0, 14.0));
            ui.painter().rect_filled(r, 6.0, egui::Color32::from_rgba_premultiplied(30, 30, 30, 235));
            ui.painter().rect_stroke(r, 6.0, egui::Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
            ui.painter().galley(r.min + egui::vec2(12.0, 7.0), galley, t.text);
            let resp = ui.interact(r, egui::Id::new("toast"), egui::Sense::click());
            if resp.clicked() {
                self.ui.status.clear();
                self.toast = None;
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            if !self.ui.status.is_empty() {
                let since: f64 = ui.data(|d| d.get_temp(egui::Id::new("status-since")).unwrap_or(now));
                ui.data_mut(|d| d.insert_temp(egui::Id::new("status-since"), since));
                if now - since > 5.0 {
                    self.ui.status.clear();
                    ui.data_mut(|d| d.remove::<f64>(egui::Id::new("status-since")));
                }
            }
        }
    }
}

impl EffectcraftApp {
    /// Auto-save a dirty project when the interval has passed (Settings ▸ Project ▸ Auto-Save).
    fn tick_autosave(&mut self, ctx: &egui::Context) {
        let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        if let Some(Err(e)) = self.session.autosave_tick(now) {
            self.ui.status = e.to_string();
        }
        if self.session.prefs.auto_save.enabled {
            ctx.request_repaint_after(std::time::Duration::from_secs(30));
        }
    }

    /// Take the pending synthetic input (from `ui.click`, `ui.key`, …). Hosts that don't call
    /// [`eframe::App::raw_input_hook`] (the headless test harness) feed these in themselves.
    pub fn take_synthetic_input(&mut self) -> Vec<egui::Event> {
        std::mem::take(&mut self.synthetic)
    }
}

impl eframe::App for EffectcraftApp {
    fn on_exit(&mut self) {
        // Clean exit: no crash recovery next launch.
        self.session.end_recovery();
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if !self.synthetic.is_empty() {
            // Pointer events go one per frame so egui sees press → moves → release as a real drag.
            let pointer = |e: &egui::Event| matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::MouseWheel { .. });
            let n = if pointer(&self.synthetic[0]) {
                1
            } else {
                self.synthetic
                    .iter()
                    .position(|e| pointer(e) || matches!(e, egui::Event::Key { pressed: false, .. }))
                    .map_or(self.synthetic.len(), |i| if pointer(&self.synthetic[i]) { i.max(1) } else { i + 1 })
            };
            raw_input.events.extend(self.synthetic.drain(..n));
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.styled {
            theme::install(ctx, &self.tokens);
            self.styled = true;
            ctx.request_repaint();
        } else {
            self.fonts_ready = true;
        }
        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_time) as f32;
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + (1.0 / dt).min(480.0) * 0.1;
        }
        self.last_time = now;
        self.drain_control(ctx);
        if !self.synthetic.is_empty() {
            ctx.request_repaint();
        } else if !self.input_waiters.is_empty() {
            for w in self.input_waiters.drain(..) {
                let _ = w.send(json!({"ok": true, "result": null}));
            }
        }
        self.issue_screenshots(ctx);
        self.collect_screenshots(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.init_gpu(frame);
        if !self.fonts_ready {
            ui.ctx().request_repaint();
            return;
        }
        self.frame(ui);
        let ctx = ui.ctx().clone();
        // wasm32: no frame threads; render queued frames now, between UI frames.
        if cfg!(target_arch = "wasm32") && self.frames.pump(std::time::Duration::from_millis(if self.playback.playing { 24 } else { 40 })) {
            ctx.request_repaint();
        }
        self.last_ui_time = ctx.input(|i| i.time);
        if !self.synthetic.is_empty() {
            ctx.request_repaint();
        } else if !self.input_waiters.is_empty() {
            ctx.request_repaint();
            for w in self.input_waiters.drain(..) {
                let _ = w.send(json!({"ok": true, "result": null}));
            }
        }
        if self.frames.inflight() > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
}

/// Advance playback with the viewer's scale (called by the viewer panel each frame).
pub(crate) fn tick_playback(app: &mut EffectcraftApp, ctx: &egui::Context, scale: f64) {
    app.advance_playback(ctx, scale);
}
