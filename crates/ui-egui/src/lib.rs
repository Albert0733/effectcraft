//! The EffectCraft egui frontend (swappable): docking panels, the Composition viewer, the
//! Timeline with property twirl-downs and keyframes, Effect Controls, Effects & Presets, Preview,
//! Info, Character/Paragraph/Align, menus, dialogs and the control channel.
//!
//! Everything goes through `effectcraft_engine::Session::execute` (or `menus::invoke` for
//! frontend-only commands) so every gesture is also available to agents.

pub mod automation;
pub mod control;
pub mod dock;
pub mod frames;
pub mod header;
pub mod icons;
pub mod menus;
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
}

/// Host hooks provided by the native app (file pickers etc.).
#[derive(Default)]
pub struct Hooks {
    pub pick_files: Option<Box<dyn Fn(&[&str]) -> Vec<String>>>,
    pub pick_save: Option<Box<dyn Fn(&str) -> Option<String>>>,
    pub pick_open_project: Option<Box<dyn Fn() -> Option<String>>>,
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
    /// Texture of the frame shown in the viewer and the key it came from.
    pub(crate) viewer_tex: Option<(egui::TextureHandle, FrameKey)>,
    /// Last displayed image (for the Info panel colour picker).
    pub(crate) viewer_image: Option<Arc<egui::ColorImage>>,
    /// Commands from the native menu bar.
    pub command_inbox: Option<Receiver<String>>,
    /// Pointer position in comp pixels (Info panel).
    pub(crate) pointer_comp: Option<[f32; 2]>,
    pub(crate) dialog_state: panels::dialogs::DialogState,
    pub integrated_titlebar: bool,
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
            viewer_image: None,
            command_inbox: None,
            pointer_comp: None,
            dialog_state: Default::default(),
            integrated_titlebar: false,
        }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, k: theme::ThemeKind) {
        self.ui.theme = k;
        self.tokens = Tokens::for_kind(k);
        theme::apply_visuals(ctx, &self.tokens);
    }

    pub fn set_workspace(&mut self, name: &str) {
        self.ui.workspace = name.to_string();
        self.ui.dock = dock::workspace(name);
    }

    pub fn show_panel(&mut self, p: PanelKind) {
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
        RenderSource { project: self.session.project.clone(), footage: self.session.footage.clone(), expr: self.session.expr.clone() }
    }

    /// The render scale used by the viewer right now.
    pub fn viewer_scale(&self, zoom: f32, ppp: f32) -> f64 {
        if self.playback.playing && self.ui.viewer.res == state::Resolution::Auto {
            // Keep previews snappy: never more than the displayed size.
            return self.ui.viewer.res.scale(zoom, ppp).min(0.5);
        }
        self.ui.viewer.res.scale(zoom, ppp)
    }

    pub fn frame_key(&self, comp: ItemId, frame: i64, scale: f64) -> FrameKey {
        FrameKey { revision: self.session.revision, comp: comp.0, frame, scale: (scale * 1000.0).round() as u32 }
    }

    /// Request a frame render (no-op if cached/in flight).
    pub fn request_frame(&self, comp: ItemId, frame: i64, scale: f64) {
        let Some(c) = self.session.project.comp(comp) else { return };
        let key = self.frame_key(comp, frame, scale);
        let t = c.frame_rate.tick_of(frame);
        let opts = RenderOpts { scale, motion_blur: true, guides: true, draft: self.ui.viewer.fast_preview };
        self.frames.request(&self.render_source(), key, comp, t, opts);
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
    }

    pub fn stop(&mut self) {
        self.playback.playing = false;
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
        let ahead = (self.frames_parallelism() * 2).max(4) as i64;
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
        if !self.playback.playing {
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
        self.handle_events(&ctx);
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

    /// Tab labels that name what the panel shows, as After Effects does: "Composition Intro",
    /// "Effect Controls Title", "Properties: Title", and the Timeline tab named after its comp.
    fn tab_titles(&self) -> Vec<(PanelKind, String)> {
        let mut out = Vec::new();
        let Some(comp) = self.session.active_comp() else { return out };
        let cname = self.session.active_comp_id().and_then(|id| self.session.project.item(id)).map(|i| i.name.clone()).unwrap_or_default();
        out.push((PanelKind::Composition, format!("Composition {cname}")));
        out.push((PanelKind::Timeline, cname));
        if let Some(l) = self.session.state.selected_layers.first().and_then(|id| comp.layer(*id)) {
            out.push((PanelKind::EffectControls, format!("Effect Controls {}", l.name)));
            out.push((PanelKind::Properties, format!("Properties: {}", l.name)));
        }
        out
    }

    fn dock_area(&mut self, ui: &mut egui::Ui, body: egui::Rect) {
        let t = self.tokens;
        let mut dock = std::mem::replace(&mut self.ui.dock, dock::DockNode::Tabs { panels: vec![], active: 0 });
        let mut groups = Vec::new();
        dock::layout(ui, &mut dock, body, &t, "", &mut groups, &mut self.auto);
        let mut actions = Vec::new();
        let titles = self.tab_titles();
        let title = |p: PanelKind| titles.iter().find(|(k, _)| *k == p).map(|(_, s)| s.clone()).unwrap_or_else(|| p.title().to_string());
        for g in &groups {
            actions.extend(dock::draw_group_chrome(ui, g, self.ui.focused, &t, &mut self.auto, &title));
        }
        self.ui.dock = dock;
        for g in &groups {
            let Some(p) = g.panels.get(g.active).copied() else { continue };
            self.auto.add(&format!("panel.{}", p.id()), g.content, p.title());
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(g.content).id_salt(("panel", p.id())));
            child.set_clip_rect(g.content.intersect(ui.clip_rect()));
            panels::show(self, &mut child, p, g.content);
        }
        for a in actions {
            match a {
                dock::DockAction::Activate(p) => {
                    self.ui.dock.activate(p);
                }
                dock::DockAction::Focus(p) => self.ui.focused = p,
                dock::DockAction::Close(p) => self.ui.dock.close(p),
                dock::DockAction::PanelMenu(p, pos) => {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("panel-menu"), (p, pos)));
                }
            }
        }
        panels::panel_menu_popup(self, ui);
    }
}

impl EffectcraftApp {
    /// Take the pending synthetic input (from `ui.click`, `ui.key`, …). Hosts that don't call
    /// [`eframe::App::raw_input_hook`] (the headless test harness) feed these in themselves.
    pub fn take_synthetic_input(&mut self) -> Vec<egui::Event> {
        std::mem::take(&mut self.synthetic)
    }
}

impl eframe::App for EffectcraftApp {
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

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.fonts_ready {
            ui.ctx().request_repaint();
            return;
        }
        self.frame(ui);
        let ctx = ui.ctx().clone();
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
