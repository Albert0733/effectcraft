//! Edit menu.

use effectcraft_color::Label;
use effectcraft_project::{Expression, ItemKind, Layer, LayerId, LayerSource};
use effectcraft_time::Tick;
use serde_json::{Value, json};

use super::{CommandSpec, has_comp, has_layers, layers_p, match_path_of, selected_leaf_props, str_p};
use crate::{EngineError, LinkClip, Result, Session, cmd};

fn can_undo(s: &Session) -> std::result::Result<(), String> {
    if s.history.undo.is_empty() { Err("nothing to undo".into()) } else { Ok(()) }
}
fn can_redo(s: &Session) -> std::result::Result<(), String> {
    if s.history.redo.is_empty() { Err("nothing to redo".into()) } else { Ok(()) }
}
fn has_clip(s: &Session) -> std::result::Result<(), String> {
    has_comp(s)?;
    if s.state.text_edit.is_some() {
        // Text editing pastes the system clipboard's text (passed as `text`) or copied text.
        return Ok(());
    }
    if s.state.clipboard.is_empty() && s.state.key_clipboard.is_empty() && s.state.effect_clipboard.is_empty() && s.state.link_clipboard.is_none() {
        Err("the clipboard is empty".into())
    } else {
        Ok(())
    }
}
fn has_key_clip(s: &Session) -> std::result::Result<(), String> {
    has_layers(s)?;
    if s.state.key_clipboard.is_empty() { Err("no keyframes on the clipboard".into()) } else { Ok(()) }
}
fn has_props_or_layers(s: &Session) -> std::result::Result<(), String> {
    has_comp(s)?;
    if s.state.selected_layers.is_empty() && s.state.selected_props.is_empty() { Err("select layers or properties first".into()) } else { Ok(()) }
}
fn has_props_or_layers_or_items(s: &Session) -> std::result::Result<(), String> {
    if has_props_or_layers(s).is_ok() || !s.state.project_selection.is_empty() { Ok(()) } else { Err("select layers or project items first".into()) }
}
fn has_selected_footage(s: &Session) -> std::result::Result<(), String> {
    if original_path(s).is_some() { Ok(()) } else { Err("select a footage item or layer".into()) }
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
    if s.state.text_edit.is_some() {
        return s.execute("text.setSelection", json!({"select": "all"}));
    }
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
    // Effects selected → duplicate them (Edit ▸ Duplicate in Effect Controls).
    if p.get("layers").is_none() {
        let fx = super::effect::selected_effects(s);
        if !fx.is_empty() {
            let mut out = vec![];
            for (lid, uid) in fx {
                out.push(s.execute("effect.duplicate", json!({"layer": lid.0, "effect": uid}))?);
            }
            return Ok(json!({"effects": out}));
        }
    }
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
    if s.state.text_edit.is_some() && p.get("layers").is_none() {
        return s.execute("text.delete", json!({}));
    }
    // Keyframes selected → delete keys; mask vertices → delete them; else layers.
    if !s.state.selected_keys.is_empty() && p.get("layers").is_none() {
        return s.execute("keys.delete", json!({}));
    }
    if !s.state.selected_vertices.is_empty() && p.get("layers").is_none() {
        return s.execute("mask.deleteVertices", json!({}));
    }
    // Effects selected → remove them.
    if p.get("layers").is_none() && !super::effect::selected_effects(s).is_empty() {
        return s.execute("effect.remove", json!({}));
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

fn clear_clipboards(s: &mut Session) {
    s.state.clipboard.clear();
    s.state.key_clipboard.clear();
    s.state.effect_clipboard.clear();
    s.state.link_clipboard = None;
    s.state.clip_is_keys = false;
}

/// Edit ▸ Copy: selected keyframes (when any) go to the keyframe clipboard, else layers.
fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    if s.state.text_edit.is_some() && p.get("layers").is_none() {
        return super::text_edit::copy(s);
    }
    // Keyframes selected → copy keys (pasted at the CTI).
    if !s.state.selected_keys.is_empty() && p.get("layers").is_none() {
        s.state.link_clipboard = None;
        s.state.effect_clipboard.clear();
        return s.execute("keys.copy", json!({}));
    }
    // Effects selected (Effect Controls / timeline) → copy the effects.
    if p.get("layers").is_none() && !super::effect::selected_effects(s).is_empty() {
        return s.execute("effect.copy", json!({}));
    }
    let (cid, ids) = layers_p(s, p)?;
    s.state.clip_is_keys = false;
    let comp = s.project.comp(cid).ok_or(crate::EngineError::NoComp)?;
    let layers: Vec<Layer> = comp.layers.iter().filter(|l| ids.contains(&l.id)).cloned().collect();
    clear_clipboards(s);
    s.state.clipboard = layers;
    Ok(json!(s.state.clipboard.len()))
}

/// The expression accessor for a property, e.g. `comp("Main").layer("Solid").transform("Position")`.
pub(crate) fn link_expression(s: &Session, lid: LayerId, uid: effectcraft_project::Uid, relative: bool) -> Option<String> {
    let cid = s.active_comp_id()?;
    let comp = s.project.comp(cid)?;
    let l = comp.layer(lid)?;
    let names = l.props.name_path_of(uid)?;
    let matches = match_path_of(&l.props, uid)?;
    let names: Vec<&str> = names.split('/').collect();
    let first = matches.split('/').next()?.to_string();
    let q = |x: &str| serde_json::to_string(x).unwrap_or_default();
    let base = if relative { format!("thisComp.layer({})", q(&l.name)) } else { format!("comp({}).layer({})", q(&s.project.item(cid)?.name), q(&l.name)) };
    let mut out = match first.as_str() {
        "transform" => format!("{base}.transform"),
        "effects" => format!("{base}.effect({})", q(names.get(1)?)),
        "masks" => format!("{base}.mask({})", q(names.get(1)?)),
        "text" => format!("{base}.text"),
        "contents" => format!("{base}.content({})", q(names.get(1)?)),
        _ => return None,
    };
    let skip = if matches!(first.as_str(), "effects" | "masks" | "contents") { 2 } else { 1 };
    for n in names.iter().skip(skip) {
        out.push_str(&format!("({})", q(n)));
    }
    Some(out)
}

fn copy_links(s: &mut Session, p: &Value, relative: bool) -> Result<Value> {
    let props = selected_leaf_props(s);
    if props.is_empty() {
        // Layers: copy them with every animatable transform/effect property linked to the source.
        let (cid, ids) = layers_p(s, p)?;
        let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
        let mut layers = vec![];
        for l in comp.layers.iter().filter(|l| ids.contains(&l.id)) {
            let mut copy = l.clone();
            let mut uids = vec![];
            for g in ["transform", "effects"] {
                if let Some(grp) = l.props.sub(g) {
                    grp.walk("", &mut |_, pr| {
                        if !pr.static_only && !pr.hold_only {
                            uids.push(pr.uid);
                        }
                    });
                }
            }
            for uid in uids {
                if let (Some(e), Some(pr)) = (link_expression(s, l.id, uid, relative), copy.props.find_mut(uid)) {
                    pr.expr = Some(Expression { text: e, enabled: true });
                }
            }
            layers.push(copy);
        }
        let n = layers.len();
        clear_clipboards(s);
        s.state.clipboard = layers;
        return Ok(json!({"layers": n}));
    }
    let comp = s.active_comp().ok_or(EngineError::NoComp)?;
    let mut links = vec![];
    for (lid, uid) in &props {
        let Some(path) = comp.layer(*lid).and_then(|l| match_path_of(&l.props, *uid)) else { continue };
        if let Some(e) = link_expression(s, *lid, *uid, relative) {
            links.push((path, e));
        }
    }
    let n = links.len();
    clear_clipboards(s);
    s.state.link_clipboard = Some(LinkClip::Links { relative, links });
    Ok(json!({"links": n}))
}

fn copy_expression_only(s: &mut Session, _: &Value) -> Result<Value> {
    let comp = s.active_comp().ok_or(EngineError::NoComp)?;
    let mut out = vec![];
    for (lid, uid) in selected_leaf_props(s) {
        let Some(l) = comp.layer(lid) else { continue };
        if let (Some(pr), Some(path)) = (l.props.find(uid), match_path_of(&l.props, uid))
            && let Some(e) = &pr.expr
        {
            out.push((path, e.clone()));
        }
    }
    if out.is_empty() {
        return Err(EngineError::Other("the selected properties have no expressions".into()));
    }
    let n = out.len();
    clear_clipboards(s);
    s.state.link_clipboard = Some(LinkClip::Expressions(out));
    Ok(json!({"expressions": n}))
}

/// Paste the keyframe clipboard at the CTI onto the selected layers (same property paths).
fn paste_keys(s: &mut Session, reversed: bool) -> Result<Value> {
    let cid = s.active_comp_id().ok_or(EngineError::NoComp)?;
    let ids = s.state.selected_layers.clone();
    if ids.is_empty() {
        return Err(EngineError::Other("select a layer to paste keyframes into".into()));
    }
    let clip = s.state.key_clipboard.clone();
    let t = s.time();
    let n = s.edit(if reversed { "Paste Reversed Keyframes" } else { "Paste Keyframes" }, None, |proj, st| {
        let mut n = 0;
        st.selected_keys.clear();
        for lid in &ids {
            let l = super::layer_mut(proj, cid, *lid)?;
            let lt = l.layer_time(t);
            for c in &clip {
                let Some(pr) = l.props.prop_mut(&c.path) else { continue };
                let span = c.keys.last().map(|k| k.time).unwrap_or(Tick::ZERO);
                for k in &c.keys {
                    let mut k = k.clone();
                    if std::mem::discriminant(&k.value) != std::mem::discriminant(&pr.value) {
                        continue;
                    }
                    if reversed {
                        k.time = span - k.time;
                        std::mem::swap(&mut k.in_interp, &mut k.out_interp);
                        std::mem::swap(&mut k.in_ease, &mut k.out_ease);
                        std::mem::swap(&mut k.spatial_in, &mut k.spatial_out);
                    }
                    k.time += lt;
                    st.selected_keys.push(crate::KeyRef { layer: *lid, prop: pr.uid, time: k.time });
                    effectcraft_keyframe::set_key(&mut pr.keys, k);
                    n += 1;
                }
            }
        }
        Ok(n)
    })?;
    Ok(json!({"keys": n}))
}

fn paste_links(s: &mut Session, clip: LinkClip) -> Result<Value> {
    let cid = s.active_comp_id().ok_or(EngineError::NoComp)?;
    let ids = s.state.selected_layers.clone();
    if ids.is_empty() {
        return Err(EngineError::Other("select a layer to paste into".into()));
    }
    let pairs: Vec<(String, Expression)> = match clip {
        LinkClip::Links { links, .. } => links.into_iter().map(|(p, e)| (p, Expression { text: e, enabled: true })).collect(),
        LinkClip::Expressions(v) => v,
    };
    let n = s.edit("Paste", None, |proj, _| {
        let mut n = 0;
        for lid in &ids {
            let l = super::layer_mut(proj, cid, *lid)?;
            for (path, e) in &pairs {
                if let Some(pr) = l.props.prop_mut(path) {
                    pr.expr = Some(e.clone());
                    n += 1;
                }
            }
        }
        Ok(n)
    })?;
    Ok(json!({"expressions": n}))
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    if s.state.text_edit.is_some() && p.get("layers").is_none() {
        return super::text_edit::cut(s);
    }
    if !s.state.selected_keys.is_empty() && p.get("layers").is_none() {
        s.execute("keys.copy", json!({}))?;
        return s.execute("keys.delete", json!({}));
    }
    copy(s, p)?;
    delete(s, p)
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    if s.state.text_edit.is_some() {
        return super::text_edit::paste(s, p);
    }
    if let Some(clip) = s.state.link_clipboard.clone() {
        return paste_links(s, clip);
    }
    if s.state.clip_is_keys && !s.state.key_clipboard.is_empty() {
        return s.execute("keys.paste", p.clone());
    }
    if !s.state.effect_clipboard.is_empty() {
        return s.execute("effect.paste", p.clone());
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
    let lab = s.prefs.label_from_name(name).ok_or_else(|| super::bad("edit.label", format!("unknown label `{name}`")))?;
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

fn paste_reversed(s: &mut Session, _: &Value) -> Result<Value> {
    paste_keys(s, true)
}

/// Lift (leave a gap) or extract (close the gap) the work area from the selected layers (all
/// unlocked layers when none are selected).
fn work_area_cut(s: &mut Session, p: &Value, extract: bool) -> Result<Value> {
    let cid = super::comp_id(s, p)?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
    let (a, b) = comp.work_area;
    let ids: Vec<LayerId> = match layers_p(s, p)?.1 {
        v if v.is_empty() => comp.layers.iter().map(|l| l.id).collect(),
        v => v,
    };
    let gap = b - a;
    s.edit(if extract { "Extract Work Area" } else { "Lift Work Area" }, None, |proj, st| {
        let mut next = proj.next_id;
        let c = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        let shift = |l: &mut Layer, d: Tick| {
            l.start_time += d;
            l.in_point += d;
            l.out_point += d;
        };
        let mut i = 0;
        while i < c.layers.len() {
            if !ids.contains(&c.layers[i].id) || c.layers[i].switches.locked {
                i += 1;
                continue;
            }
            let (lin, lout) = (c.layers[i].in_point, c.layers[i].out_point);
            if lout <= a {
                i += 1;
            } else if lin >= b {
                if extract {
                    shift(&mut c.layers[i], Tick::ZERO - gap);
                }
                i += 1;
            } else if lin >= a && lout <= b {
                c.layers.remove(i);
            } else if lin < a && lout > b {
                // Split around the work area: the layer keeps the head, a copy takes the tail.
                let mut tail = c.layers[i].clone();
                reid(&mut tail, &mut next);
                tail.in_point = b;
                if extract {
                    shift(&mut tail, Tick::ZERO - gap);
                }
                c.layers[i].out_point = a;
                c.layers.insert(i, tail);
                i += 2;
            } else if lin < a {
                c.layers[i].out_point = a;
                i += 1;
            } else {
                c.layers[i].in_point = b;
                if extract {
                    shift(&mut c.layers[i], Tick::ZERO - gap);
                }
                i += 1;
            }
        }
        let alive: Vec<LayerId> = c.layers.iter().map(|l| l.id).collect();
        st.selected_layers.retain(|l| alive.contains(l));
        proj.next_id = next;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn lift(s: &mut Session, p: &Value) -> Result<Value> {
    work_area_cut(s, p, false)
}
fn extract(s: &mut Session, p: &Value) -> Result<Value> {
    work_area_cut(s, p, true)
}

fn select_label_group(s: &mut Session, _: &Value) -> Result<Value> {
    let comp = s.active_comp().ok_or(EngineError::NoComp)?;
    let labels: Vec<Label> = s.state.selected_layers.iter().filter_map(|l| comp.layer(*l)).map(|l| l.label).collect();
    if labels.is_empty() {
        // Project panel: items with the same labels.
        let items: Vec<Label> = s.state.project_selection.iter().filter_map(|i| s.project.item(*i)).map(|i| i.label).collect();
        s.state.project_selection = s.project.items.values().filter(|i| items.contains(&i.label)).map(|i| i.id).collect();
        return Ok(json!({"items": s.state.project_selection.len()}));
    }
    s.state.selected_layers = comp.layers.iter().filter(|l| labels.contains(&l.label)).map(|l| l.id).collect();
    Ok(json!({"layers": s.state.selected_layers.len()}))
}

fn purge_caches(s: &mut Session, p: &Value) -> Result<Value> {
    let what = str_p(p, "what").unwrap_or("all").to_string();
    s.events.push(crate::Event::PurgeCaches);
    s.toast(format!("Purged {what} cache"));
    Ok(json!({"purged": what}))
}

/// Path of the footage behind the selected layer or project item.
fn original_path(s: &Session) -> Option<String> {
    let comp = s.active_comp();
    let from_layer = s.state.selected_layers.first().and_then(|l| comp?.layer(*l)).and_then(|l| match l.source {
        LayerSource::Footage { item } => Some(item),
        _ => None,
    });
    let item = from_layer.or_else(|| s.state.project_selection.first().copied())?;
    match &s.project.item(item)?.kind {
        ItemKind::Footage(f) if !f.path.is_empty() => Some(f.path.clone()),
        _ => None,
    }
}

fn edit_original(s: &mut Session, _: &Value) -> Result<Value> {
    let path = original_path(s).ok_or_else(|| super::bad("edit.editOriginal", "select a footage item or layer"))?;
    let url = format!("file://{path}");
    s.events.push(crate::Event::OpenUrl(url.clone()));
    Ok(json!({"url": url}))
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
        cmd!(
            "edit.copyWithPropertyLinks",
            "Copy with Property Links",
            ["Edit"],
            Some("Cmd+Alt+C"),
            "{layers?} (selected properties, or layers, as expressions linking to the originals)",
            has_props_or_layers,
            |s, p| copy_links(s, p, false)
        ),
        cmd!(
            "edit.copyWithRelativePropertyLinks",
            "Copy with Relative Property Links",
            ["Edit"],
            None,
            "{layers?} (like Copy with Property Links, using thisComp)",
            has_props_or_layers,
            |s, p| copy_links(s, p, true)
        ),
        cmd!("edit.copyExpressionOnly", "Copy Expression Only", ["Edit"], None, "{}", has_props_or_layers, copy_expression_only),
        cmd!("edit.paste", "Paste", ["Edit"], Some("Cmd+V"), "{} (layers, keyframes at the CTI, or property links / expressions)", has_clip, paste),
        cmd!("edit.pasteReversedKeyframes", "Paste Reversed Keyframes", ["Edit"], None, "{}", has_key_clip, paste_reversed),
        cmd!("edit.clear", "Clear", ["Edit"], Some("Delete"), "{layers?}", layers_or_keys, delete),
        cmd!("edit.duplicate", "Duplicate", ["Edit"], Some("Cmd+D"), "{layers?}", has_layers, duplicate),
        cmd!("edit.splitLayer", "Split Layer", ["Edit"], Some("Cmd+Shift+D"), "{layers?}", has_layers, split),
        cmd!("edit.liftWorkArea", "Lift Work Area", ["Edit"], None, "{layers?}", has_comp, lift),
        cmd!("edit.extractWorkArea", "Extract Work Area", ["Edit"], None, "{layers?}", has_comp, extract),
        cmd!("edit.selectAll", "Select All", ["Edit"], Some("Cmd+A"), "{}", has_comp, select_all),
        cmd!("edit.deselectAll", "Deselect All", ["Edit"], Some("Cmd+Shift+A"), "{}", always_ok, deselect_all),
        cmd!("edit.label", "Label", ["Edit", "Label"], None, "{label: Red|Yellow|Aqua|…, layers?}", always_ok, label),
        cmd!("edit.selectLabelGroup", "Select Label Group", ["Edit", "Label"], None, "{}", has_props_or_layers_or_items, select_label_group),
        cmd!("edit.purgeUndo", "Undo", ["Edit", "Purge"], None, "{}", always_ok, purge),
        cmd!("edit.purge", "Purge", [], None, "{what?: all|memoryAndDisk|memory|disk|3d|image|snapshot}", always_ok, purge_caches),
        cmd!("edit.editOriginal", "Edit Original...", ["Edit"], Some("Cmd+E"), "{}", has_selected_footage, edit_original),
    ]
}

fn layers_or_keys(s: &Session) -> std::result::Result<(), String> {
    if !s.state.selected_keys.is_empty() || !s.state.selected_vertices.is_empty() { super::has_comp(s) } else { has_layers(s) }
}

fn always_ok(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}
