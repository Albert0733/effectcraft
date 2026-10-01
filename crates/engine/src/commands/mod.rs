//! The command registry. Ids follow After Effects' menu structure; every menu item, panel button,
//! shortcut and viewer/timeline gesture maps to one of these.

mod comp;
mod edit;
mod effect;
mod file;
mod help;
mod layer;
mod prop;
mod query;
mod time;

use std::sync::OnceLock;

use effectcraft_project::{Comp, ItemId, Layer, LayerId};
use effectcraft_time::Tick;
use serde_json::Value;

use crate::{EngineError, Result, Session};

pub type Run = fn(&mut Session, &Value) -> Result<Value>;
pub type Enabled = fn(&Session) -> std::result::Result<(), String>;

/// Metadata + implementation for one command.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu placement, e.g. `["Layer", "New"]`. Empty = not in menus.
    pub menu: &'static [&'static str],
    /// Default shortcut (egui-style names, `Cmd` = ⌘ on macOS / Ctrl elsewhere).
    pub shortcut: Option<&'static str>,
    /// Parameter description for agents and the MCP schema.
    pub params: &'static str,
    pub enabled: Enabled,
    pub run: Run,
    /// Record in the journal (false for pure queries).
    pub journal: bool,
}

#[macro_export]
macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::commands::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
}
#[macro_export]
macro_rules! query {
    ($id:literal, $label:literal, $params:literal, $run:expr) => {
        $crate::commands::CommandSpec {
            id: $id,
            label: $label,
            menu: &[],
            shortcut: None,
            params: $params,
            enabled: $crate::commands::always,
            run: $run,
            journal: false,
        }
    };
}

pub fn command_specs() -> &'static [CommandSpec] {
    static SPECS: OnceLock<Vec<CommandSpec>> = OnceLock::new();
    SPECS.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(file::specs());
        v.extend(edit::specs());
        v.extend(comp::specs());
        v.extend(layer::specs());
        v.extend(prop::specs());
        v.extend(effect::specs());
        v.extend(time::specs());
        v.extend(help::specs());
        v.extend(query::specs());
        v
    })
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    command_specs().iter().find(|c| c.id == id)
}

// ---------- enablement ----------

pub fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}
pub(crate) fn has_comp(s: &Session) -> std::result::Result<(), String> {
    s.active_comp().map(|_| ()).ok_or_else(|| "no composition is open".into())
}
pub(crate) fn has_layers(s: &Session) -> std::result::Result<(), String> {
    has_comp(s)?;
    if s.state.selected_layers.is_empty() { Err("select a layer first".into()) } else { Ok(()) }
}
pub(crate) fn has_keys(s: &Session) -> std::result::Result<(), String> {
    has_comp(s)?;
    if s.state.selected_keys.is_empty() { Err("select keyframes first".into()) } else { Ok(()) }
}

// ---------- params ----------

pub(crate) fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
pub(crate) fn str_p<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}
pub(crate) fn f_p(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64)
}
pub(crate) fn b_p(p: &Value, k: &str) -> Option<bool> {
    p.get(k).and_then(Value::as_bool)
}
pub(crate) fn merge_p(p: &Value) -> Option<&str> {
    str_p(p, "merge")
}

/// `comp`: item id or name; default the active comp.
pub(crate) fn comp_id(s: &Session, p: &Value) -> Result<ItemId> {
    match p.get("comp") {
        Some(Value::Number(n)) => n.as_u64().map(ItemId).filter(|id| s.project.comp(*id).is_some()).ok_or(EngineError::NoComp),
        Some(Value::String(name)) => s.project.items.values().find(|i| &i.name == name && i.as_comp().is_some()).map(|i| i.id).ok_or(EngineError::NoComp),
        _ => s.active_comp_id().ok_or(EngineError::NoComp),
    }
}

/// Resolve a layer reference: id (number), `#n` / index (1-based), or name.
pub(crate) fn resolve_layer(comp: &Comp, v: &Value) -> Option<LayerId> {
    match v {
        Value::Number(n) => {
            let id = n.as_u64()?;
            if comp.layer(LayerId(id)).is_some() { Some(LayerId(id)) } else { comp.layers.get((id as usize).checked_sub(1)?).map(|l| l.id) }
        }
        Value::String(s) => {
            if let Some(i) = s.strip_prefix('#').and_then(|i| i.parse::<usize>().ok()) {
                return comp.layers.get(i.checked_sub(1)?).map(|l| l.id);
            }
            comp.layers.iter().find(|l| &l.name == s).map(|l| l.id)
        }
        _ => None,
    }
}

/// `layer` param or the first selected layer.
pub(crate) fn layer_p(s: &Session, p: &Value, cmd: &str) -> Result<(ItemId, LayerId)> {
    let cid = comp_id(s, p)?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
    if let Some(v) = p.get("layer") {
        return resolve_layer(comp, v).map(|l| (cid, l)).ok_or_else(|| bad(cmd, format!("no layer {v}")));
    }
    s.state
        .selected_layers
        .first()
        .copied()
        .filter(|l| comp.layer(*l).is_some())
        .map(|l| (cid, l))
        .ok_or_else(|| bad(cmd, "no `layer` given and no layer selected"))
}

/// `layers` param (array) or the selection.
pub(crate) fn layers_p(s: &Session, p: &Value) -> Result<(ItemId, Vec<LayerId>)> {
    let cid = comp_id(s, p)?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
    if let Some(Value::Array(a)) = p.get("layers") {
        return Ok((cid, a.iter().filter_map(|v| resolve_layer(comp, v)).collect()));
    }
    if let Some(v) = p.get("layer") {
        return Ok((cid, resolve_layer(comp, v).into_iter().collect()));
    }
    Ok((cid, s.state.selected_layers.iter().copied().filter(|l| comp.layer(*l).is_some()).collect()))
}

/// `time` (seconds) or `frame` param, else the CTI.
pub(crate) fn time_p(s: &Session, p: &Value, comp: Option<&Comp>) -> Tick {
    if let Some(t) = f_p(p, "time") {
        return Tick::from_seconds_f64(t);
    }
    if let (Some(f), Some(c)) = (p.get("frame").and_then(Value::as_i64), comp) {
        return c.frame_rate.tick_of(f);
    }
    s.time()
}

pub(crate) fn layer_mut<'a>(p: &'a mut effectcraft_project::Project, cid: ItemId, lid: LayerId) -> Result<&'a mut Layer> {
    p.comp_mut(cid).ok_or(EngineError::NoComp)?.layer_mut(lid).ok_or(EngineError::Project(effectcraft_project::ProjectError::NoLayer(lid)))
}
