//! The EffectCraft engine façade.
//!
//! Every user-visible action is a command with a stable id (`layer.newSolid`, `prop.set`,
//! `keys.easyEase`…) and JSON parameters, dispatched through [`Session::execute`]. The egui UI,
//! the CLI, the control channel and the MCP server all use this one entry point; that is what
//! makes the UI swappable and the whole app agent-drivable.
//!
//! The project is an `Arc<Project>` edited copy-on-write; undo keeps whole-project snapshots
//! (compositions are `Arc`s, so untouched comps are shared).

pub mod autosave;
pub mod camera_track;
pub mod commands;
pub mod config;
pub mod demo;
pub mod history;
pub mod links;
pub mod mask_track;
pub mod menus;
pub mod offload;
pub mod prefs;
pub mod psd_import;
pub mod render_queue;
pub mod roto;
pub mod scriptui;
mod session_settings;
pub mod shortcuts;
pub mod tracking;
pub mod vector;
pub mod viewer;
pub mod warp;

use std::sync::Arc;

use effectcraft_project::{Comp, ItemId, Layer, LayerId, Project, Uid};
use effectcraft_raster::Image;
use effectcraft_render::{ExprHost, FootageSource, LayerCache, NoFootage, RenderOpts, Renderer};
use effectcraft_time::Tick;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use commands::{CommandSpec, command_specs, find as find_command};
pub use effectcraft_color as color;
pub use effectcraft_effects as effects;
pub use effectcraft_geom as geom;
pub use effectcraft_keyframe as keyframe;
pub use effectcraft_project as project;
pub use effectcraft_render as render;
pub use effectcraft_text as text;
pub use effectcraft_time as time;
pub use effectcraft_track as track;
pub use history::{Branch, History, HistoryNode};
pub use render_queue::{ExportJob, ExportResult, Exporter, JobState};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no active composition")]
    NoComp,
    #[error("{0}")]
    Project(#[from] effectcraft_project::ProjectError),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Host services (file access) injected by the frontend.
pub trait Services: Send + Sync {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>>;
    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()>;
}

/// Native filesystem.
pub struct FsServices;
impl Services for FsServices {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }
    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        config::atomic_write(std::path::Path::new(path), data)
    }
}

/// Imports media files into the project (implemented by the media layer).
pub trait Importer: Send + Sync {
    /// Probe a file and return footage metadata.
    fn probe(&self, path: &str) -> std::result::Result<effectcraft_project::Footage, String>;
}

/// A keyframe reference: layer, property uid, key time (layer time).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyRef {
    pub layer: LayerId,
    pub prop: Uid,
    pub time: Tick,
}

/// Keyframes of one property on the keyframe clipboard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyClip {
    pub layer: LayerId,
    /// Match-id path relative to the layer (`transform/position`), portable between layers.
    pub path: String,
    /// Keys with times relative to the earliest copied key (layer time).
    pub keys: Vec<effectcraft_keyframe::Keyframe>,
}

/// A selected mask vertex: layer, mask group uid, vertex index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VertexRef {
    pub layer: LayerId,
    pub mask: Uid,
    pub index: usize,
}

/// Editing state that commands depend on (headless-relevant, serde for agents).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EditorState {
    pub active_comp: Option<ItemId>,
    /// Open compositions (viewer/timeline tabs), in tab order.
    pub open_comps: Vec<ItemId>,
    /// Current time per comp (the CTI).
    pub times: std::collections::BTreeMap<ItemId, Tick>,
    pub selected_layers: Vec<LayerId>,
    /// Selected properties / groups (by uid) of selected layers.
    pub selected_props: Vec<(LayerId, Uid)>,
    pub selected_keys: Vec<KeyRef>,
    pub project_selection: Vec<ItemId>,
    /// Layer clipboard (serialized layers) and keyframe clipboard.
    #[serde(skip)]
    pub clipboard: Vec<Layer>,
    /// Copied keyframes (times relative to the earliest copied key).
    #[serde(skip)]
    pub key_clipboard: Vec<KeyClip>,
    /// The last copy was keyframes (Paste pastes keys at the CTI).
    #[serde(skip)]
    pub clip_is_keys: bool,
    /// Copied effect instances (Edit ▸ Copy with effects selected); Paste adds them to the
    /// selected layers.
    #[serde(skip)]
    pub effect_clipboard: Vec<effectcraft_project::PropGroup>,
    /// Selected mask vertices (viewer Selection tool / pen).
    #[serde(default)]
    pub selected_vertices: Vec<VertexRef>,
    pub snapping: bool,
    /// Last applied effect id (Effect ▸ last effect).
    pub last_effect: Option<String>,
    /// Viewer region of interest `[x, y, w, h]` in comp pixels (Composition ▸ Crop Comp to Region
    /// of Interest).
    #[serde(default)]
    pub region_of_interest: Option<[f64; 4]>,
    /// Property-link clipboard (Edit ▸ Copy with Property Links / Copy Expression Only).
    #[serde(skip)]
    pub link_clipboard: Option<LinkClip>,
    /// File ▸ Interpret Footage ▸ Remember Interpretation.
    #[serde(skip)]
    pub interpretation: Option<effectcraft_project::Footage>,
    /// Viewer 3D view per comp (Active Camera / Front / … / Custom View 3 and edited view cameras).
    #[serde(default)]
    pub views3d: std::collections::BTreeMap<ItemId, effectcraft_render::three_d::Views3D>,
    /// Tracker panel ▸ Current Track: (tracked layer, tracker group uid) in the active comp.
    #[serde(default)]
    pub current_track: Option<(LayerId, Uid)>,
    /// Tracker panel ▸ Method in mask mode (Track Mask).
    #[serde(default)]
    pub mask_track_method: mask_track::MaskMethod,
    /// 3D Camera Tracker: the selected track points (ids) in the viewer.
    #[serde(default)]
    pub camera_points: Vec<u32>,
    /// Mask Interpolation panel options.
    #[serde(default)]
    pub mask_interp: commands::mask_interp::MaskInterpOptions,
    /// Paint and Brushes panel options (Brush, Clone Stamp and Eraser tools).
    #[serde(default)]
    pub paint: commands::paint::PaintOptions,
    /// Roto Brush and Refine Edge tool options.
    #[serde(default)]
    pub roto: roto::RotoOptions,
    /// Puppet tool options for new meshes.
    #[serde(default)]
    pub puppet: commands::puppet::PuppetOptions,
    /// View ▸ Switch View Layout: 1, 2 or 4 views side by side in the Composition viewer.
    #[serde(default = "one_view")]
    pub view_layout: u8,
    /// View ▸ Switch View Layout ▸ Share View Options (grid, guides… in every view).
    #[serde(default)]
    pub share_view_options: bool,
    /// Layer ▸ Mask ▸ Hide Locked Masks (viewer outlines).
    #[serde(default)]
    pub hide_locked_masks: bool,
    /// On-canvas text editing (Type tool): the edited layer, selection and insertion style.
    #[serde(default)]
    pub text_edit: Option<commands::text_edit::TextEdit>,
    /// Text copied while editing (with its formatting), for Paste / Paste Text Formatting Only.
    #[serde(skip)]
    pub text_clipboard: Option<effectcraft_keyframe::TextDoc>,
    /// Composition viewer display options (Show Channel, exposure, snapshot, Fast Previews).
    #[serde(default)]
    pub viewer: commands::viewer_cmds::ViewOptions,
    /// Essential Graphics panel ▸ Primary composition (`None` = the active comp).
    #[serde(default)]
    pub essential_primary: Option<ItemId>,
    /// Essential Graphics panel ▸ Solo Supported Properties (the timeline shows only properties
    /// Essential Graphics can expose).
    #[serde(default)]
    pub essential_solo: bool,
    /// View ▸ Split with New Locked Viewer: the second viewer's comp and 3D view.
    #[serde(default)]
    pub locked_viewer: Option<commands::viewer_cmds::LockedViewer>,
}

fn one_view() -> u8 {
    1
}

/// What Copy with Property Links / Copy Expression Only put on the clipboard.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkClip {
    /// Expressions that link back to the source properties: (match path, expression text).
    Links { relative: bool, links: Vec<(String, String)> },
    /// Expressions only: (match path, expression).
    Expressions(Vec<(String, effectcraft_project::Expression)>),
}

/// Events for frontends (drained each frame).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Event {
    ProjectChanged {
        revision: u64,
    },
    Toast {
        message: String,
        error: bool,
    },
    OpenComp(ItemId),
    OpenUrl(String),
    /// A frontend-only command (viewer zoom, panels, dialogs…) ran: the UI performs it. Headless
    /// sessions ignore these.
    Frontend {
        command: String,
        params: Value,
    },
    /// Drop cached frames / renders (Edit ▸ Purge).
    PurgeCaches,
}

pub struct Session {
    pub project: Arc<Project>,
    pub history: History,
    pub revision: u64,
    pub saved_revision: u64,
    pub path: Option<String>,
    pub state: EditorState,
    pub services: Arc<dyn Services>,
    pub footage: Arc<dyn FootageSource>,
    pub expr: Option<Arc<dyn ExprHost>>,
    /// GPU compositor (Mercury GPU Acceleration), when the frontend has one: used by renders
    /// that ask for [`effectcraft_render::Backend::Gpu`]/`Auto`, and by Render Queue exports
    /// when the project's renderer is the GPU.
    pub accel: Option<Arc<dyn effectcraft_render::Accelerator>>,
    /// Expression syntax checker (set by the host that links the expression engine).
    pub expr_check: Option<fn(&str) -> std::result::Result<(), String>>,
    pub importer: Option<Arc<dyn Importer>>,
    /// Render Queue encoder (the export layer); `None` = export unavailable.
    pub exporter: Option<Arc<dyn Exporter>>,
    /// The running (or finished, not yet polled) render.
    pub render_job: Option<render_queue::RenderJob>,
    /// The running (or finished, not yet polled) track analysis.
    pub track_job: Option<tracking::TrackJob>,
    /// The running (or finished, not yet polled) mask track.
    pub mask_job: Option<mask_track::MaskTrackJob>,
    /// The running (or finished, not yet polled) Warp Stabilizer analysis.
    pub warp_job: Option<warp::WarpJob>,
    /// Warp Stabilizers waiting for (re-)analysis: (comp, layer, effect uid).
    pub warp_pending: Vec<(ItemId, LayerId, Uid)>,
    /// The running (or finished, not yet polled) 3D Camera Tracker analysis.
    pub camera_job: Option<camera_track::CameraJob>,
    /// 3D Camera Trackers waiting for (re-)analysis: (comp, layer, effect uid).
    pub camera_pending: Vec<(ItemId, LayerId, Uid)>,
    /// The running (or finished, not yet polled) Roto Brush propagation / Freeze.
    pub roto_job: Option<roto::RotoJob>,
    /// Roto Brush instances edited since their last propagation: (comp, layer, effect uid).
    pub roto_pending: Vec<(ItemId, LayerId, Uid)>,
    pub events: Vec<Event>,
    /// Commands executed: (id, params).
    pub journal: Vec<(String, Value)>,
    /// Processed-layer pixels reused across frames and edits (content-keyed, never stale).
    pub layer_cache: Arc<LayerCache>,
    /// The persistent disk cache (Settings ▸ Media & Disk Cache), when enabled.
    pub disk_cache: Option<Arc<effectcraft_render::disk_cache::DiskCache>>,
    /// Settings (Preferences).
    pub prefs: prefs::Prefs,
    /// Bumped whenever settings change (frontends re-apply theme, labels…).
    pub prefs_revision: u64,
    /// Where settings, shortcut presets and the crash-recovery sentinel are stored (`None` =
    /// nothing persists).
    pub config: Option<Arc<dyn config::ConfigStore>>,
    /// Keyboard shortcut presets.
    pub keymaps: shortcuts::Keymaps,
    /// Frontend-only commands offered for binding.
    pub ui_commands: Vec<shortcuts::UiCommand>,
    /// Cache of the resolved active preset (read it with [`Session::shortcuts`]).
    pub shortcut_table: std::sync::OnceLock<shortcuts::ShortcutTable>,
    /// Auto-save bookkeeping.
    pub autosave: autosave::AutoSaveState,
    /// The viewer snapshot (Take Snapshot / Show Snapshot).
    pub snapshot: Option<viewer::Snapshot>,
    /// The JavaScript scripting engine (set by the host that links `effectcraft-script`):
    /// `script.run`, File ▸ Scripts ▸ Run Script File… (`.jsx`/`.js`) and the Script Console.
    pub script: Option<ScriptRunner>,
    /// Loads WebAssembly effect plug-ins (set by the host that links `effectcraft-plugin` with
    /// its runtime): `effect.plugins.load`.
    pub plugin_loader: Option<PluginLoader>,
    /// ScriptUI windows and panels opened by scripts (see [`scriptui`]).
    pub script_ui: scriptui::ScriptUi,
    /// Runs renders and analyses off the UI thread where there are no threads (the web app's
    /// Web Workers, see [`offload`]); `None` = threads (desktop) or inline (wasm32).
    pub offload: Option<Arc<dyn offload::Offload>>,
    /// Analyses running in the [`Session::offload`] worker.
    pub offloaded: Vec<offload::OffloadedJob>,
}

/// A script to run (see [`Session::script`]).
#[derive(Clone, Copy, Debug)]
pub struct ScriptRequest<'a> {
    pub code: &'a str,
    /// File name shown in errors and `$.fileName` (`console` for the Script Console).
    pub name: &'a str,
    /// Run in the Script Console's persistent context (variables survive between runs).
    pub console: bool,
}

/// Runs a script against the session and reports `{ok, result, output, error: {message, line,
/// column, file} | null}`.
pub type ScriptRunner = fn(&mut Session, &ScriptRequest) -> Value;

/// Loads one WebAssembly effect plug-in (`bytes`, `source` path) into the effect registry and
/// describes it (`{id, name, category, version, api, params}`).
pub type PluginLoader = fn(&[u8], &str) -> std::result::Result<Value, String>;

impl Default for Session {
    fn default() -> Self {
        Session {
            project: Arc::new(Project::default()),
            history: History::default(),
            revision: 0,
            saved_revision: 0,
            path: None,
            state: EditorState { snapping: true, view_layout: 1, ..Default::default() },
            services: Arc::new(FsServices),
            footage: Arc::new(NoFootage),
            expr: None,
            accel: None,
            expr_check: None,
            importer: None,
            exporter: None,
            render_job: None,
            track_job: None,
            mask_job: None,
            warp_job: None,
            warp_pending: vec![],
            camera_job: None,
            camera_pending: vec![],
            roto_job: None,
            roto_pending: vec![],
            events: vec![],
            journal: vec![],
            layer_cache: Arc::new(LayerCache::default()),
            disk_cache: None,
            prefs: prefs::Prefs::default(),
            prefs_revision: 0,
            config: None,
            keymaps: shortcuts::Keymaps::default(),
            ui_commands: vec![],
            shortcut_table: std::sync::OnceLock::new(),
            autosave: autosave::AutoSaveState::default(),
            snapshot: None,
            script: None,
            plugin_loader: None,
            script_ui: scriptui::ScriptUi::default(),
            offload: None,
            offloaded: vec![],
        }
    }
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// Run a command by id.
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value> {
        let spec = commands::find(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        if let Err(why) = (spec.enabled)(self) {
            // Explicit targets (agents, scripts) don't need a UI selection.
            let explicit = (["layer", "layers", "prop", "keys"].iter().any(|k| params.get(k).is_some()) && commands::has_comp(self).is_ok())
                || ["item", "items"].iter().any(|k| params.get(k).is_some());
            if !explicit {
                return Err(EngineError::Disabled(id.to_string(), why));
            }
        }
        let r = (spec.run)(self, &params)?;
        if spec.journal {
            self.journal.push((id.to_string(), params));
            if self.journal.len() > 10_000 {
                self.journal.drain(..1000);
            }
        }
        Ok(r)
    }

    /// [`Session::execute`] for agents and scripts (MCP, CLI, control channel): unknown top-level
    /// parameters are rejected with the accepted keys instead of being silently ignored.
    pub fn execute_checked(&mut self, id: &str, params: Value) -> Result<Value> {
        let spec = commands::find(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        commands::check_params(spec, &params)?;
        self.execute(id, params)
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        commands::find(id).is_some_and(|c| (c.enabled)(self).is_ok())
    }

    /// Apply an undoable edit. `merge`: consecutive edits with the same key fold into one step.
    pub fn edit<T>(&mut self, label: &str, merge: Option<&str>, f: impl FnOnce(&mut Project, &mut EditorState) -> Result<T>) -> Result<T> {
        let before = self.project.clone();
        let mut p = (*self.project).clone();
        let mut st = self.state.clone();
        let r = f(&mut p, &mut st)?;
        // Layer styles: one Global Light per comp, whichever layer edited it.
        effectcraft_project::styles::sync_global_light(&before, &mut p);
        // Essential Properties of precomp layers follow their comps' Essential Graphics.
        effectcraft_project::essential::sync_project(&before, &mut p);
        // Warp Stabilizer analyses made from other frames are cleared (and queued again).
        for w in warp::invalidate(&before, &mut p) {
            if !self.warp_pending.contains(&w) {
                self.warp_pending.push(w);
            }
        }
        // So are 3D Camera Tracker tracks (and solves made with other settings).
        for w in camera_track::invalidate(&before, &mut p) {
            if !self.camera_pending.contains(&w) {
                self.camera_pending.push(w);
            }
        }
        // Roto Brush Input Keys follow the frames; edited instances propagate again.
        for r in roto::sync(&before, &mut p) {
            if !self.roto_pending.contains(&r) {
                self.roto_pending.push(r);
            }
        }
        let same = merge.is_some() && merge.map(str::to_string) == self.history.merge_key;
        if !same {
            // Undone steps aren't lost: they stay in the History panel as a branch.
            let levels = self.prefs.general.undo_levels.max(1) as usize;
            self.history.record(label, before, levels);
        } else {
            self.history.abandon_redo(&before);
        }
        self.history.merge_key = merge.map(str::to_string);
        self.project = Arc::new(p);
        self.state = st;
        self.bump();
        Ok(r)
    }

    pub fn bump(&mut self) {
        self.revision += 1;
        if self.disk_cache.is_some() {
            self.layer_cache.set_disk_salt(effectcraft_render::disk_cache::footage_salt(&self.project));
        }
        self.events.push(Event::ProjectChanged { revision: self.revision });
    }

    pub fn undo(&mut self) -> bool {
        let Some((label, p)) = self.history.undo.pop() else { return false };
        let cur = std::mem::replace(&mut self.project, p);
        self.history.redo.push((label, cur));
        self.history.merge_key = None;
        self.sanitize_state();
        self.bump();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((label, p)) = self.history.redo.pop() else { return false };
        let cur = std::mem::replace(&mut self.project, p);
        self.history.undo.push((label, cur));
        self.history.merge_key = None;
        self.sanitize_state();
        self.bump();
        true
    }

    /// Drop selections that no longer exist.
    pub fn sanitize_state(&mut self) {
        let p = self.project.clone();
        self.state.open_comps.retain(|c| p.comp(*c).is_some());
        if self.state.active_comp.is_some_and(|c| p.comp(c).is_none()) {
            self.state.active_comp = self.state.open_comps.first().copied();
        }
        if let Some(c) = self.active_comp() {
            let ids: Vec<LayerId> = c.layers.iter().map(|l| l.id).collect();
            self.state.selected_layers.retain(|l| ids.contains(l));
            self.state.selected_props.retain(|(l, _)| ids.contains(l));
            self.state.selected_keys.retain(|k| ids.contains(&k.layer));
            self.state.selected_vertices.retain(|v| ids.contains(&v.layer));
        }
        if let Some((l, t)) = self.state.current_track
            && self.active_comp().and_then(|c| c.layer(l)).and_then(|l| l.tracker(t)).is_none()
        {
            self.state.current_track = None;
        }
        self.state.project_selection.retain(|i| p.item(*i).is_some());
        commands::text_edit::sanitize(self);
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn active_comp(&self) -> Option<&Comp> {
        self.project.comp(self.state.active_comp?)
    }
    pub fn active_comp_id(&self) -> Option<ItemId> {
        self.state.active_comp.filter(|c| self.project.comp(*c).is_some())
    }

    /// Current time of a comp.
    pub fn time_of(&self, comp: ItemId) -> Tick {
        self.state.times.get(&comp).copied().unwrap_or(Tick::ZERO)
    }

    /// Current time of the active comp.
    pub fn time(&self) -> Tick {
        self.state.active_comp.and_then(|c| self.state.times.get(&c).copied()).unwrap_or(Tick::ZERO)
    }

    pub fn set_time(&mut self, t: Tick) {
        if let Some(c) = self.state.active_comp {
            let comp = self.project.comp(c);
            let t = match comp {
                Some(comp) => comp.frame_rate.snap_nearest(t.clamp(Tick::ZERO, comp.duration - comp.frame_duration())),
                None => t,
            };
            self.state.times.insert(c, t);
        }
    }

    /// Open a comp in the viewer/timeline and make it active.
    pub fn open_comp(&mut self, id: ItemId) {
        if self.project.comp(id).is_none() {
            return;
        }
        if !self.state.open_comps.contains(&id) {
            self.state.open_comps.push(id);
        }
        if self.state.active_comp != Some(id) {
            self.state.active_comp = Some(id);
            self.state.selected_layers.clear();
            self.state.selected_props.clear();
            self.state.selected_keys.clear();
            self.state.selected_vertices.clear();
            self.state.current_track = None;
        }
        self.events.push(Event::OpenComp(id));
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.events.push(Event::Toast { message: msg.into(), error: false });
    }

    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The 3D view camera of a comp's viewer (None = the active camera).
    pub fn view_camera(&self, comp: ItemId) -> Option<effectcraft_render::three_d::CameraState> {
        let c = self.project.comp(comp)?;
        if !c.has_3d() {
            return None;
        }
        self.state.views3d.get(&comp)?.override_camera(c.width as f64, c.height as f64)
    }

    /// Render a comp frame with the session's footage + expression hosts.
    pub fn render(&self, comp: ItemId, t: Tick, opts: RenderOpts) -> Image {
        let mut r = Renderer::new(&self.project, self.footage.as_ref(), opts);
        r.expr = self.expr.as_deref();
        r.cache = Some(&self.layer_cache);
        r.accel = self.accel.as_deref();
        r.comp_frame(comp, t)
    }

    /// Resolve a comp reference (item id number or name; `None`/null = the active comp).
    pub fn resolve_comp(&self, comp: Option<&Value>) -> Result<ItemId> {
        let p = match comp {
            Some(v) if !v.is_null() => serde_json::json!({ "comp": v }),
            _ => serde_json::json!({}),
        };
        commands::comp_id(self, &p)
    }

    /// Render a comp frame (comp time `t`) as opaque 8-bit RGBA over the comp background, scaled
    /// so the longest side is at most `max_side` pixels (0 = full size). Returns (width, height, rgba).
    pub fn render_rgba8(&self, comp: ItemId, t: Tick, max_side: u32) -> Result<(u32, u32, Vec<u8>)> {
        let c = self.project.comp(comp).ok_or(EngineError::NoComp)?;
        let long = c.width.max(c.height).max(1) as f64;
        let scale = if max_side == 0 { 1.0 } else { (max_side as f64 / long).min(1.0) };
        let img = self.render(comp, t, RenderOpts { scale, backend: effectcraft_render::Backend::Auto, ..Default::default() });
        Ok((img.width, img.height, img.to_rgba8_over(c.background)))
    }

    /// Replace the whole project (open/new), resetting history and state.
    pub fn replace_project(&mut self, mut p: Project, path: Option<String>) {
        if self.stop_render()
            && let Some(mut job) = self.render_job.take()
        {
            // Let the worker notice the cancel; its updates refer to the old project.
            job.wait();
        }
        self.render_job = None;
        self.stop_track();
        self.track_job = None;
        self.stop_mask_track();
        self.mask_job = None;
        self.stop_warp();
        self.warp_job = None;
        self.warp_pending.clear();
        self.stop_camera();
        self.camera_job = None;
        self.camera_pending.clear();
        self.stop_roto();
        self.roto_job = None;
        self.roto_pending.clear();
        p.fix_next_id();
        upgrade_effects(&mut p);
        self.project = Arc::new(p);
        self.history = History::default();
        self.state = EditorState { snapping: true, view_layout: 1, ..Default::default() };
        self.path = path;
        self.bump();
        self.saved_revision = self.revision;
        let first = self.project.comps().next().map(|(id, _)| *id);
        if let Some(c) = first {
            self.open_comp(c);
        }
    }
}

/// Bring effect instances saved by earlier versions up to date with the effect registry
/// (renamed / regrouped / added parameters, reordered popups; see
/// [`effectcraft_effects::migrate`]).
pub fn upgrade_effects(p: &mut Project) {
    let mut sizes = std::collections::HashMap::new();
    for (cid, c) in p.comps() {
        for l in &c.layers {
            let (w, h) = effectcraft_render::source_size(p, l);
            sizes.insert((*cid, l.id), if w == 0 { [c.width as f64, c.height as f64] } else { [w as f64, h as f64] });
        }
    }
    let mut next = p.next_id;
    for (cid, it) in p.items.iter_mut() {
        let effectcraft_project::ItemKind::Comp(c) = &mut it.kind else { continue };
        for l in &mut std::sync::Arc::make_mut(c).layers {
            let size = sizes.get(&(*cid, l.id)).copied().unwrap_or([100.0, 100.0]);
            let Some(fx) = l.props.sub_mut("effects") else { continue };
            for n in &mut fx.children {
                let effectcraft_project::Node::Group(g) = n else { continue };
                let effectcraft_project::GroupKind::Effect { effect } = &g.kind else { continue };
                if let Some(spec) = effectcraft_effects::find(effect) {
                    effectcraft_effects::migrate::upgrade_instance(spec, g, &mut effectcraft_project::build::Ids(&mut next), size);
                }
            }
        }
    }
    p.next_id = next;
}

#[cfg(test)]
mod rq_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_3d;
#[cfg(test)]
mod tests_anim_tools;
#[cfg(test)]
mod tests_camera_track;
#[cfg(test)]
mod tests_color_view;
#[cfg(test)]
mod tests_disk_cache;
#[cfg(test)]
mod tests_effects;
#[cfg(test)]
mod tests_essential;
#[cfg(test)]
mod tests_fidelity;
#[cfg(test)]
mod tests_lottie;
#[cfg(test)]
mod tests_markers;
#[cfg(test)]
mod tests_mask_warp;
#[cfg(test)]
mod tests_menu_cmds;
#[cfg(test)]
mod tests_model3d;
#[cfg(test)]
mod tests_project_items;
#[cfg(test)]
mod tests_proxy;
#[cfg(test)]
mod tests_rig3d;
#[cfg(test)]
mod tests_settings;
#[cfg(test)]
mod tests_stubs;
#[cfg(test)]
mod tests_styles;
#[cfg(test)]
mod tests_tail;
#[cfg(test)]
mod tests_text;
#[cfg(test)]
mod tests_text_edit;
#[cfg(test)]
mod tests_timeline;
#[cfg(test)]
mod tests_track;
#[cfg(test)]
mod tests_vector_import;
#[cfg(test)]
mod tests_viewer;

/// Font families available to text layers (bundled + scanned system fonts).
/// Text animation presets: (id, name), for menus.
pub fn text_presets() -> Vec<(String, String)> {
    commands::text_preset_list()
}

pub fn text_families() -> Vec<String> {
    effectcraft_text::families().into_iter().map(|(f, _)| f).collect()
}
#[cfg(test)]
mod tests_history;
#[cfg(test)]
mod tests_paint;
#[cfg(test)]
mod tests_roto;
