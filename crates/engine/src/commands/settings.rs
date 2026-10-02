//! Settings (`prefs.*`).

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, str_p};
use crate::{Result, Session, cmd, query};

// ---------------------------------------------------------------- prefs

fn prefs_get(s: &mut Session, p: &Value) -> Result<Value> {
    let key = str_p(p, "key").unwrap_or("");
    s.prefs.get(key).ok_or_else(|| bad("prefs.get", format!("unknown setting `{key}`")))
}

fn prefs_set(s: &mut Session, p: &Value) -> Result<Value> {
    let mut pairs: Vec<(String, Value)> = vec![];
    if let Some(Value::Object(m)) = p.get("values") {
        pairs.extend(m.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    if let Some(k) = str_p(p, "key") {
        let v = p.get("value").cloned().ok_or_else(|| bad("prefs.set", "missing `value`"))?;
        pairs.push((k.to_string(), v));
    }
    if pairs.is_empty() {
        return Err(bad("prefs.set", "give `key` and `value` (or `values: {key: value}`)"));
    }
    let mut next = s.prefs.clone();
    for (k, v) in &pairs {
        next.set(k, v.clone()).map_err(|e| bad("prefs.set", e))?;
    }
    s.prefs = next;
    s.prefs_changed();
    s.save_prefs();
    let out: serde_json::Map<String, Value> = pairs.iter().map(|(k, _)| (k.clone(), s.prefs.get(k).unwrap_or(Value::Null))).collect();
    Ok(Value::Object(out))
}

fn prefs_reset(s: &mut Session, p: &Value) -> Result<Value> {
    let page = str_p(p, "page");
    s.prefs.reset(page).map_err(|e| bad("prefs.reset", e))?;
    s.prefs_changed();
    s.save_prefs();
    s.toast(match page {
        Some(pg) => format!("Reset {pg} settings"),
        None => "Reset all settings".into(),
    });
    Ok(json!({"reset": page.unwrap_or("all")}))
}

fn prefs_open(s: &mut Session, p: &Value) -> Result<Value> {
    let page = str_p(p, "page").unwrap_or("general");
    let id = crate::prefs::page_id(page).ok_or_else(|| bad("prefs.open", format!("unknown settings page `{page}`")))?;
    super::frontend(s, "app.settings", &json!({"page": id}))
}

fn prefs_pages(_: &mut Session, _: &Value) -> Result<Value> {
    Ok(crate::prefs::pages_json())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        query!("prefs.get", "Get Setting", "{key?: e.g. general.undoLevels (omit for all settings)}", prefs_get),
        cmd!("prefs.set", "Change Setting", [], None, "{key, value, values?: {key: value}}", always, prefs_set),
        cmd!("prefs.reset", "Reset Settings", [], None, "{page?: general|labels|project|…}", always, prefs_reset),
        cmd!(
            "prefs.open",
            "Open Settings",
            [],
            None,
            "{page?: general|startup|project|composition|previews|appearance|grids|labels|type|import|export|audio|disk|memory|video|3d|scripting}",
            always,
            prefs_open
        ),
        query!("prefs.pages", "Settings Pages", "{}", prefs_pages),
    ]
}
