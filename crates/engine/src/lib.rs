//! The EffectCraft engine façade.
//!
//! Every user-visible action is a command with a stable id (`layer.newSolid`, `prop.set`,
//! `keys.easyEase`…) and JSON parameters, dispatched through [`Session::execute`]. The egui UI,
//! the CLI, the control channel and the MCP server all use this one entry point; that is what
//! makes the UI swappable and the whole app agent-drivable.
//!
//! The project is an `Arc<Project>` edited copy-on-write; undo keeps whole-project snapshots
//! (compositions are `Arc`s, so untouched comps are shared).

pub mod commands;
pub mod demo;
pub mod links;
pub mod render_queue;

use std::sync::Arc;

use effectcraft_project::{Comp, ItemId, Layer, LayerId, Project, Uid};
use effectcraft_raster::Image;
use effectcraft_render::{ExprHost, FootageSource, NoFootage, RenderOpts, Renderer};
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
pub use effectcraft_time as time;
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
        let p = std::path::Path::new(path);
        let tmp = p.with_extension("ecproj.tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, p)
    }
}

/// Imports media files into the project (implemented by the media layer).
pub trait Importer: Send + Sync {
    /// Probe a file and return footage metadata.
    fn probe(&self, path: &str) -> std::result::Result<effectcraft_project::Footage, String>;
}

/// Undo history of whole-project snapshots.
#[derive(Clone, Default)]
pub struct History {
    pub undo: Vec<(String, Arc<Project>)>,
    pub redo: Vec<(String, Arc<Project>)>,
    /// Key of the last merged step: a continuous gesture with the same key folds into one step.
    pub merge_key: Option<String>,
}

/// A keyframe reference: layer, property uid, key time (layer time).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyRef {
    pub layer: LayerId,
    pub prop: Uid,
    pub time: Tick,
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
    pub snapping: bool,
    /// Last applied effect id (Effect ▸ last effect).
    pub last_effect: Option<String>,
}

/// Events for frontends (drained each frame).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Event {
    ProjectChanged { revision: u64 },
    Toast { message: String, error: bool },
    OpenComp(ItemId),
    OpenUrl(String),
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
    pub importer: Option<Arc<dyn Importer>>,
    /// Render Queue encoder (the export layer); `None` = export unavailable.
    pub exporter: Option<Arc<dyn Exporter>>,
    /// The running (or finished, not yet polled) render.
    pub render_job: Option<render_queue::RenderJob>,
    pub events: Vec<Event>,
    /// Commands executed: (id, params).
    pub journal: Vec<(String, Value)>,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            project: Arc::new(Project::default()),
            history: History::default(),
            revision: 0,
            saved_revision: 0,
            path: None,
            state: EditorState { snapping: true, ..Default::default() },
            services: Arc::new(FsServices),
            footage: Arc::new(NoFootage),
            expr: None,
            importer: None,
            exporter: None,
            render_job: None,
            events: vec![],
            journal: vec![],
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
            let explicit = ["layer", "layers", "prop", "keys"].iter().any(|k| params.get(k).is_some()) && commands::has_comp(self).is_ok();
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

    pub fn is_enabled(&self, id: &str) -> bool {
        commands::find(id).is_some_and(|c| (c.enabled)(self).is_ok())
    }

    /// Apply an undoable edit. `merge`: consecutive edits with the same key fold into one step.
    pub fn edit<T>(&mut self, label: &str, merge: Option<&str>, f: impl FnOnce(&mut Project, &mut EditorState) -> Result<T>) -> Result<T> {
        let before = self.project.clone();
        let mut p = (*self.project).clone();
        let mut st = self.state.clone();
        let r = f(&mut p, &mut st)?;
        let same = merge.is_some() && merge.map(str::to_string) == self.history.merge_key;
        if !same {
            self.history.undo.push((label.to_string(), before));
            if self.history.undo.len() > 500 {
                self.history.undo.remove(0);
            }
        }
        self.history.merge_key = merge.map(str::to_string);
        self.history.redo.clear();
        self.project = Arc::new(p);
        self.state = st;
        self.bump();
        Ok(r)
    }

    pub fn bump(&mut self) {
        self.revision += 1;
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
        }
        self.state.project_selection.retain(|i| p.item(*i).is_some());
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

    /// Current time of the active comp.
    pub fn time(&self) -> Tick {
        self.state.active_comp.and_then(|c| self.state.times.get(&c).copied()).unwrap_or(Tick::ZERO)
    }

    pub fn set_time(&mut self, t: Tick) {
        if let Some(c) = self.state.active_comp {
            let comp = self.project.comp(c);
            let t = match comp {
                Some(comp) => comp.frame_rate.snap(t.clamp(Tick::ZERO, comp.duration - comp.frame_duration())),
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
        }
        self.events.push(Event::OpenComp(id));
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.events.push(Event::Toast { message: msg.into(), error: false });
    }

    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Render a comp frame with the session's footage + expression hosts.
    pub fn render(&self, comp: ItemId, t: Tick, opts: RenderOpts) -> Image {
        let mut r = Renderer::new(&self.project, self.footage.as_ref(), opts);
        r.expr = self.expr.as_deref();
        r.comp_frame(comp, t)
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
        p.fix_next_id();
        self.project = Arc::new(p);
        self.history = History::default();
        self.state = EditorState { snapping: true, ..Default::default() };
        self.path = path;
        self.bump();
        self.saved_revision = self.revision;
        let first = self.project.comps().next().map(|(id, _)| *id);
        if let Some(c) = first {
            self.open_comp(c);
        }
    }
}

#[cfg(test)]
mod rq_tests;
#[cfg(test)]
mod tests;

/// Font families available to text layers (bundled + scanned system fonts).
pub fn text_families() -> Vec<String> {
    effectcraft_text::families().into_iter().map(|(f, _)| f).collect()
}
