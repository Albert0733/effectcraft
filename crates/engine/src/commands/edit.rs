//! Edit menu.

use effectcraft_color::Label;
use effectcraft_project::{Layer, LayerId};
use serde_json::{Value, json};

use super::{CommandSpec, has_comp, has_layers, layers_p, str_p};
use crate::{Result, Session, cmd};

fn can_undo(s: &Session) -> std::result::Result<(), String> {
    if s.history.undo.is_empty() { Err("nothing to undo".into()) } else { Ok(()) }
}
fn can_redo(s: &Session) -> std::result::Result<(), String> {
    if s.history.redo.is_empty() { Err("nothing to redo".into()) } else { Ok(()) }
}
fn has_clip(s: &Session) -> std::result::Result<(), String> {
    has_comp(s)?;
    if s.state.clipboard.is_empty() && s.state.key_clipboard.is_empty() { Err("the clipboard is empty".into()) } else { Ok(()) }
}

fn undo(s: &mut Session, _: &Value) -> Result<Value> {
    let label = s.history.undo.last().map(|u| u.0.clone());
    s.undo();
    Ok(json!({"undone": label}))
}
fn redo(s: &mut Session, _: &Value) -> Result<Value> {
    let label = s.history.redo.last().map(|u| u.0.clone());
    s.redo();
    Ok(json!({"redone": label}))
}

fn select_all(s: &mut Session, _: &Value) -> Result<Value> {
    if let Some(c) = s.active_comp() {
        s.state.selected_layers = c.layers.iter().map(|l| l.id).collect();
    }
    Ok(json!(s.state.selected_layers.len()))
}
fn deselect_all(s: &mut Session, _: &Value) -> Result<Value> {
    s.state.selected_layers.clear();
    s.state.selected_props.clear();
    s.state.selected_keys.clear();
    s.state.selected_vertices.clear();
    Ok(Value::Null)
}

/// Fresh ids for a copied layer (and its property uids).
pub(crate) fn reid(layer: &mut Layer, next: &mut u64) {
    layer.id = LayerId(*next);
    *next += 1;
    layer.props.reassign_uids(next);
    *next += 1;
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, ids) = layers_p(s, p)?;
    let new = s.edit("Duplicate", None, |proj, st| {
        let mut next = proj.next_id;
        let comp = proj.comp_mut(cid).ok_or(crate::EngineError::NoComp)?;
        let mut created = vec![];
        for id in &ids {
            let Some(i) = comp.layers.iter().position(|l| l.id == *id) else { continue };
            let mut l = comp.layers[i].clone();
            reid(&mut l, &mut next);
            l.name = comp.unique_layer_name(&l.name);
            created.push(l.id);
            comp.layers.insert(i, l);
        }
        proj.next_id = next;
        st.selected_layers = created.clone();
        Ok(created)
    })?;
    Ok(json!(new.iter().map(|l| l.0).collect::<Vec<_>>()))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    // Keyframes selected → delete keys; mask vertices → delete them; else layers.
    if !s.state.selected_keys.is_empty() && p.get("layers").is_none() {
        return s.execute("keys.delete", json!({}));
    }
    if !s.state.selected_vertices.is_empty() && p.get("layers").is_none() {
        return s.execute("mask.deleteVertices", json!({}));
    }
    let (cid, ids) = layers_p(s, p)?;
    s.edit("Clear", None, |proj, st| {
        let comp = proj.comp_mut(cid).ok_or(crate::EngineError::NoComp)?;
        comp.layers.retain(|l| !ids.contains(&l.id));
        for l in &mut comp.layers {
            if l.parent.is_some_and(|p| ids.contains(&p)) {
                l.parent = None;
            }
            if l.track_matte.is_some_and(|m| ids.contains(&m.layer)) {
                l.track_matte = None;
            }
        }
        st.selected_layers.clear();
        st.selected_props.clear();
        st.selected_keys.clear();
        Ok(())
    })?;
    Ok(json!(ids.len()))
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    // Keyframes selected → copy keys (pasted at the CTI).
    if !s.state.selected_keys.is_empty() && p.get("layers").is_none() {
        return s.execute("keys.copy", json!({}));
    }
    let (cid, ids) = layers_p(s, p)?;
    s.state.clip_is_keys = false;
    let comp = s.project.comp(cid).ok_or(crate::EngineError::NoComp)?;
    s.state.clipboard = comp.layers.iter().filter(|l| ids.contains(&l.id)).cloned().collect();
    Ok(json!(s.state.clipboard.len()))
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    if !s.state.selected_keys.is_empty() && p.get("layers").is_none() {
        s.execute("keys.copy", json!({}))?;
        return s.execute("keys.delete", json!({}));
    }
    copy(s, p)?;
    delete(s, p)
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    if s.state.clip_is_keys && !s.state.key_clipboard.is_empty() {
        return s.execute("keys.paste", p.clone());
    }
    let cid = super::comp_id(s, p)?;
    let clip = s.state.clipboard.clone();
    let new = s.edit("Paste", None, |proj, st| {
        let mut next = proj.next_id;
        let comp = proj.comp_mut(cid).ok_or(crate::EngineError::NoComp)?;
        let at = st.selected_layers.first().and_then(|id| comp.layers.iter().position(|l| l.id == *id)).unwrap_or(0);
        let mut created = vec![];
        for (k, mut l) in clip.into_iter().enumerate() {
            reid(&mut l, &mut next);
            l.name = comp.unique_layer_name(&l.name);
            l.parent = None;
            l.track_matte = None;
            created.push(l.id);
            comp.layers.insert(at + k, l);
        }
        proj.next_id = next;
        st.selected_layers = created.clone();
        Ok(created)
    })?;
    Ok(json!(new.iter().map(|l| l.0).collect::<Vec<_>>()))
}

fn split(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, ids) = layers_p(s, p)?;
    let t = s.time();
    let new = s.edit("Split Layer", None, |proj, st| {
        let mut next = proj.next_id;
        let comp = proj.comp_mut(cid).ok_or(crate::EngineError::NoComp)?;
        let mut created = vec![];
        for id in &ids {
            let Some(i) = comp.layers.iter().position(|l| l.id == *id) else { continue };
            if !(t > comp.layers[i].in_point && t < comp.layers[i].out_point) {
                continue;
            }
            let mut b = comp.layers[i].clone();
            reid(&mut b, &mut next);
            b.in_point = t;
            comp.layers[i].out_point = t;
            created.push(b.id);
            comp.layers.insert(i, b);
        }
        proj.next_id = next;
        st.selected_layers = created.clone();
        Ok(created)
    })?;
    Ok(json!(new.iter().map(|l| l.0).collect::<Vec<_>>()))
}

fn label(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_p(p, "label").unwrap_or("Red");
    let lab = Label::from_name(name).ok_or_else(|| super::bad("edit.label", format!("unknown label `{name}`")))?;
    let (cid, ids) = layers_p(s, p)?;
    let items = s.state.project_selection.clone();
    s.edit("Label", None, |proj, _| {
        if ids.is_empty() {
            for i in &items {
                if let Some(it) = proj.item_mut(*i) {
                    it.label = lab;
                }
            }
        }
        if let Some(comp) = proj.comp_mut(cid) {
            for l in comp.layers.iter_mut().filter(|l| ids.contains(&l.id)) {
                l.label = lab;
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn purge(s: &mut Session, _: &Value) -> Result<Value> {
    s.history.undo.clear();
    s.history.redo.clear();
    s.toast("Purged undo history");
    Ok(Value::Null)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{}", can_undo, undo),
        cmd!("edit.redo", "Redo", ["Edit"], Some("Cmd+Shift+Z"), "{}", can_redo, redo),
        cmd!("edit.cut", "Cut", ["Edit"], Some("Cmd+X"), "{layers?} (keyframes when keys are selected)", layers_or_keys, cut),
        cmd!("edit.copy", "Copy", ["Edit"], Some("Cmd+C"), "{layers?} (keyframes when keys are selected)", layers_or_keys, copy),
        cmd!("edit.paste", "Paste", ["Edit"], Some("Cmd+V"), "{}", has_clip, paste),
        cmd!("edit.clear", "Clear", ["Edit"], Some("Delete"), "{layers?}", layers_or_keys, delete),
        cmd!("edit.duplicate", "Duplicate", ["Edit"], Some("Cmd+D"), "{layers?}", has_layers, duplicate),
        cmd!("edit.splitLayer", "Split Layer", ["Edit"], Some("Cmd+Shift+D"), "{layers?}", has_layers, split),
        cmd!("edit.selectAll", "Select All", ["Edit"], Some("Cmd+A"), "{}", has_comp, select_all),
        cmd!("edit.deselectAll", "Deselect All", ["Edit"], Some("Cmd+Shift+A"), "{}", always_ok, deselect_all),
        cmd!("edit.label", "Label", ["Edit", "Label"], None, "{label: Red|Yellow|Aqua|…, layers?}", always_ok, label),
        cmd!("edit.purgeUndo", "Undo", ["Edit", "Purge"], None, "{}", always_ok, purge),
    ]
}

fn layers_or_keys(s: &Session) -> std::result::Result<(), String> {
    if !s.state.selected_keys.is_empty() || !s.state.selected_vertices.is_empty() { super::has_comp(s) } else { has_layers(s) }
}

fn always_ok(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}
