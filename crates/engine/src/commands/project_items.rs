//! Project panel item edits: select, rename, move into / out of folders, label and comment.
//! Every edit is undoable.

use effectcraft_project::{ItemId, Project};
use serde_json::{Value, json};

use super::{CommandSpec, always, b_p, bad, str_p};
use crate::{EngineError, Result, Session, cmd};

/// `item` (id or name) → id.
fn item_ref(s: &Session, v: &Value, cmd: &str) -> Result<ItemId> {
    match v {
        Value::Number(n) => n.as_u64().map(ItemId).filter(|i| s.project.item(*i).is_some()),
        Value::String(name) => s.project.find_by_name(name).map(|i| i.id),
        _ => None,
    }
    .ok_or_else(|| bad(cmd, format!("no project item {v}")))
}

/// `items` (array) / `item`, else the Project panel selection.
fn items_p(s: &Session, p: &Value, cmd: &str) -> Result<Vec<ItemId>> {
    if let Some(Value::Array(a)) = p.get("items") {
        return a.iter().map(|v| item_ref(s, v, cmd)).collect();
    }
    if let Some(v) = p.get("item") {
        return Ok(vec![item_ref(s, v, cmd)?]);
    }
    if s.state.project_selection.is_empty() {
        return Err(bad(cmd, "no `items` given and nothing selected in the Project panel"));
    }
    Ok(s.state.project_selection.clone())
}

/// Whether `folder` is `item` or inside it.
pub fn is_within(project: &Project, folder: ItemId, item: ItemId) -> bool {
    let mut cur = Some(folder);
    let mut guard = 0;
    while let Some(c) = cur {
        if c == item {
            return true;
        }
        cur = project.item(c).and_then(|i| i.parent);
        guard += 1;
        if guard > 256 {
            return true;
        }
    }
    false
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = match p.get("items") {
        Some(Value::Array(a)) => a.iter().map(|v| item_ref(s, v, "project.select")).collect::<Result<Vec<_>>>()?,
        _ => vec![],
    };
    if b_p(p, "add").unwrap_or(false) {
        for i in ids {
            if !s.state.project_selection.contains(&i) {
                s.state.project_selection.push(i);
            }
        }
    } else {
        s.state.project_selection = ids;
    }
    Ok(json!(s.state.project_selection.iter().map(|i| i.0).collect::<Vec<_>>()))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "project.rename";
    let id =
        item_ref(s, p.get("item").unwrap_or(&Value::Null), c).or_else(|_| s.state.project_selection.first().copied().ok_or_else(|| bad(c, "no `item`")))?;
    let name = str_p(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(c, "missing `name`"))?.to_string();
    s.edit("Rename", None, |proj, _| {
        proj.item_mut(id).ok_or_else(|| bad(c, "item vanished"))?.name = name;
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Move items into `folder` (id/name; `null` = the project root).
fn move_items(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "project.move";
    let ids = items_p(s, p, c)?;
    let folder = match p.get("folder") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let f = item_ref(s, v, c)?;
            if !s.project.item(f).is_some_and(|i| i.is_folder()) {
                return Err(bad(c, format!("{v} is not a folder")));
            }
            Some(f)
        }
    };
    if let Some(f) = folder
        && let Some(bad_id) = ids.iter().find(|i| is_within(&s.project, f, **i))
    {
        return Err(bad(c, format!("can't move folder {} into itself", bad_id.0)));
    }
    s.edit("Move Items", None, |proj, _| {
        for i in &ids {
            if let Some(it) = proj.item_mut(*i) {
                it.parent = folder;
            }
        }
        Ok(())
    })?;
    Ok(json!({"moved": ids.len()}))
}

fn set_label(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "project.setLabel";
    let ids = items_p(s, p, c)?;
    let lab = match p.get("label") {
        Some(Value::Number(n)) => effectcraft_color::Label::ALL.get(n.as_u64().unwrap_or(0) as usize).copied(),
        Some(Value::String(name)) => s.prefs.label_from_name(name),
        _ => None,
    }
    .ok_or_else(|| bad(c, "missing or unknown `label` (name or index 0-16)"))?;
    s.edit("Label", None, |proj, _| {
        for i in &ids {
            if let Some(it) = proj.item_mut(*i) {
                it.label = lab;
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn set_comment(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "project.setComment";
    let ids = items_p(s, p, c)?;
    let text = str_p(p, "comment").ok_or_else(|| bad(c, "missing `comment`"))?.to_string();
    s.edit("Comment", None, |proj, _| {
        for i in &ids {
            proj.item_mut(*i).ok_or(EngineError::Other("item vanished".into()))?.comment = text.clone();
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("project.select", "Select Project Items", [], None, "{items: [id|name], add?}", always, select),
        cmd!("project.rename", "Rename Item", [], None, "{item?: id|name (default: selected), name}", always, rename),
        cmd!("project.move", "Move to Folder", [], None, "{items?: [id|name] (default: selected), folder: id|name|null (root)}", always, move_items),
        cmd!("project.setLabel", "Item Label", [], None, "{items?, label: name|index}", always, set_label),
        cmd!("project.setComment", "Item Comment", [], None, "{items?, comment}", always, set_comment),
    ]
}
