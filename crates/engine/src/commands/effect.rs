//! Effect menu and Effect Controls gestures.

use effectcraft_project::build::Ids;
use effectcraft_project::{GroupKind, LayerId, PropGroup, Uid};
use serde_json::{Value, json};

use super::{CommandSpec, b_p, bad, has_layers, layer_mut, layers_p, str_p};
use crate::{EngineError, Result, Session, cmd};

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_p(p, "effect").ok_or_else(|| bad("effect.apply", "missing `effect` (id or name)"))?;
    let spec = effectcraft_effects::lookup(name).ok_or_else(|| bad("effect.apply", format!("unknown effect `{name}`")))?;
    let (cid, ids) = layers_p(s, p)?;
    if ids.is_empty() {
        return Err(bad("effect.apply", "no layer"));
    }
    let sizes: Vec<_> = {
        let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
        ids.iter()
            .filter_map(|id| comp.layer(*id))
            .map(|l| {
                let (w, h) = effectcraft_render::source_size(&s.project, l);
                (l.id, if w == 0 { [comp.width as f64, comp.height as f64] } else { [w as f64, h as f64] })
            })
            .collect()
    };
    let uids = s.edit(&format!("Apply {}", spec.name), None, |proj, st| {
        let mut out = vec![];
        for (lid, size) in &sizes {
            let mut next = proj.next_id;
            let l = layer_mut(proj, cid, *lid)?;
            let fx = l.props.sub_mut("effects").ok_or_else(|| bad("effect.apply", "this layer type has no effects"))?;
            let same = fx.groups().filter(|g| g.match_id == spec.id).count();
            let name = if same == 0 { spec.name.to_string() } else { format!("{} {}", spec.name, same + 1) };
            let g = effectcraft_effects::instantiate(spec, &mut Ids(&mut next), &name, *size);
            out.push(g.uid);
            fx.children.push(g.into());
            proj.next_id = next;
            st.selected_props = vec![(*lid, *out.last().unwrap_or(&0))];
        }
        st.last_effect = Some(spec.id.to_string());
        Ok(out)
    })?;
    Ok(json!({"effects": uids}))
}

fn find_fx(s: &Session, p: &Value, cmd: &str) -> Result<(effectcraft_project::ItemId, effectcraft_project::LayerId, Uid)> {
    let (cid, lid) = super::layer_p(s, p, cmd)?;
    let layer = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?;
    let fx = layer.effects().ok_or_else(|| bad(cmd, "no effects"))?;
    let g = match p.get("effect") {
        Some(Value::Number(n)) => {
            let n = n.as_u64().unwrap_or(0);
            fx.groups().find(|g| g.uid == n).or_else(|| fx.groups().nth(n.saturating_sub(1) as usize))
        }
        Some(Value::String(name)) => fx.groups().find(|g| &g.name == name || g.match_id == *name),
        _ => s.state.selected_props.iter().find_map(|(l, u)| (*l == lid).then(|| fx.groups().find(|g| g.uid == *u)).flatten()),
    }
    .ok_or_else(|| bad(cmd, "no such effect"))?;
    Ok((cid, lid, g.uid))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let cid = super::comp_id(s, p)?;
    let fx = effects_p(s, p, "effect.remove")?;
    let label = if fx.len() == 1 { "Remove Effect" } else { "Remove Effects" };
    s.edit(label, None, |proj, st| {
        for (lid, uid) in &fx {
            let l = layer_mut(proj, cid, *lid)?;
            if let Some(fx) = l.props.sub_mut("effects") {
                fx.children.retain(|c| c.uid() != *uid);
            }
            st.selected_props.retain(|(_, u)| u != uid);
        }
        Ok(())
    })?;
    Ok(json!(fx.len()))
}

fn remove_all(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, ids) = layers_p(s, p)?;
    s.edit("Remove All Effects", None, |proj, _| {
        for lid in &ids {
            if let Some(fx) = layer_mut(proj, cid, *lid)?.props.sub_mut("effects") {
                fx.children.clear();
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn toggle(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = find_fx(s, p, "effect.toggle")?;
    let v = b_p(p, "value");
    let r = s.edit("Toggle Effect", None, |proj, _| {
        let l = layer_mut(proj, cid, lid)?;
        let g = l.props.find_group_mut(uid).ok_or_else(|| bad("effect.toggle", "gone"))?;
        g.enabled = v.unwrap_or(!g.enabled);
        Ok(g.enabled)
    })?;
    Ok(json!(r))
}

fn reorder(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = find_fx(s, p, "effect.reorder")?;
    let to = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("effect.reorder", "missing `index` (1-based)"))? as usize;
    s.edit("Reorder Effect", None, |proj, _| {
        let fx = layer_mut(proj, cid, lid)?.props.sub_mut("effects").ok_or_else(|| bad("effect.reorder", "no effects"))?;
        let Some(i) = fx.children.iter().position(|c| c.uid() == uid) else { return Ok(()) };
        let g = fx.children.remove(i);
        let to = to.saturating_sub(1).min(fx.children.len());
        fx.children.insert(to, g);
        Ok(())
    })?;
    Ok(Value::Null)
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = find_fx(s, p, "effect.duplicate")?;
    let r = s.edit("Duplicate Effect", None, |proj, st| {
        let mut next = proj.next_id;
        let fx = layer_mut(proj, cid, lid)?.props.sub_mut("effects").ok_or_else(|| bad("effect.duplicate", "no effects"))?;
        let Some(i) = fx.children.iter().position(|c| c.uid() == uid) else { return Ok(0) };
        let mut g = fx.children[i].as_group().cloned().ok_or_else(|| bad("effect.duplicate", "not a group"))?;
        g.reassign_uids(&mut next);
        g.name = unique_name(fx, &g.name);
        let u = g.uid;
        fx.children.insert(i + 1, g.into());
        proj.next_id = next + 1;
        st.selected_props = vec![(lid, u)];
        Ok(u)
    })?;
    Ok(json!(r))
}

/// Effect groups selected in Effect Controls / the timeline: (layer, effect uid), in stack order.
pub(crate) fn selected_effects(s: &Session) -> Vec<(LayerId, Uid)> {
    let Some(c) = s.active_comp() else { return vec![] };
    let mut out = vec![];
    for l in &c.layers {
        let Some(fx) = l.effects() else { continue };
        for g in fx.groups() {
            if s.state.selected_props.iter().any(|(sl, u)| *sl == l.id && *u == g.uid) {
                out.push((l.id, g.uid));
            }
        }
    }
    out
}

/// Effects named by `effect` / `effects` (uids, names or 1-based indexes), else the selection.
fn effects_p(s: &Session, p: &Value, cmd: &str) -> Result<Vec<(LayerId, Uid)>> {
    if p.get("effect").is_some() {
        let (_, lid, uid) = find_fx(s, p, cmd)?;
        return Ok(vec![(lid, uid)]);
    }
    if let Some(Value::Array(a)) = p.get("effects") {
        let mut out = vec![];
        for v in a {
            let mut q = p.clone();
            if let Some(o) = q.as_object_mut() {
                o.remove("effects");
                o.insert("effect".into(), v.clone());
            }
            let (_, lid, uid) = find_fx(s, &q, cmd)?;
            out.push((lid, uid));
        }
        return Ok(out);
    }
    let sel = selected_effects(s);
    if sel.is_empty() {
        return Err(bad(cmd, "no effect selected"));
    }
    Ok(sel)
}

/// Edit ▸ Copy with effects selected: puts the effect instances on the effect clipboard.
fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    let fx = effects_p(s, p, "effect.copy")?;
    let comp = s.active_comp().ok_or(EngineError::NoComp)?;
    let groups: Vec<PropGroup> =
        fx.iter().filter_map(|(l, u)| comp.layer(*l).and_then(|l| l.effects()).and_then(|e| e.groups().find(|g| g.uid == *u)).cloned()).collect();
    s.state.clipboard.clear();
    s.state.key_clipboard.clear();
    s.state.link_clipboard = None;
    s.state.clip_is_keys = false;
    let n = groups.len();
    s.state.effect_clipboard = groups;
    Ok(json!({"effects": n}))
}

/// A unique instance name among `fx`'s effects (`Gaussian Blur`, `Gaussian Blur 2`…).
pub(crate) fn unique_name(fx: &PropGroup, base: &str) -> String {
    if !fx.groups().any(|g| g.name == base) {
        return base.to_string();
    }
    let stem = base.rsplit_once(' ').filter(|(_, n)| n.parse::<u32>().is_ok()).map(|(s, _)| s).unwrap_or(base);
    (2..).map(|i| format!("{stem} {i}")).find(|n| !fx.groups().any(|g| &g.name == n)).unwrap_or_else(|| base.to_string())
}

/// Edit ▸ Paste with effects on the clipboard: adds copies to every target layer, after the
/// layer's selected effect when it has one, else at the end of its stack.
fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    let clip = s.state.effect_clipboard.clone();
    if clip.is_empty() {
        return Err(bad("effect.paste", "no effects on the clipboard"));
    }
    let (cid, ids) = layers_p(s, p)?;
    if ids.is_empty() {
        return Err(bad("effect.paste", "select a layer to paste effects onto"));
    }
    let sel = selected_effects(s);
    let label = if clip.len() == 1 { format!("Paste {}", clip[0].name) } else { "Paste Effects".to_string() };
    let uids = s.edit(&label, None, |proj, st| {
        let mut out = vec![];
        let mut next = proj.next_id;
        let mut new_sel = vec![];
        for lid in &ids {
            let l = layer_mut(proj, cid, *lid)?;
            let Some(fx) = l.props.sub_mut("effects") else { continue };
            let at = sel
                .iter()
                .filter(|(sl, _)| sl == lid)
                .filter_map(|(_, u)| fx.children.iter().position(|c| c.uid() == *u))
                .max()
                .map(|i| i + 1)
                .unwrap_or(fx.children.len());
            for (k, g) in clip.iter().enumerate() {
                let mut g = g.clone();
                g.reassign_uids(&mut next);
                g.name = unique_name(fx, &g.name);
                out.push(g.uid);
                new_sel.push((*lid, g.uid));
                fx.children.insert(at + k, g.into());
            }
        }
        proj.next_id = next + 1;
        st.selected_props = new_sel;
        Ok(out)
    })?;
    Ok(json!({"effects": uids}))
}

/// Effect Controls' Reset: every parameter back to its default. Animated parameters get a
/// keyframe at the current time with the default value (as in After Effects); static ones
/// change their value.
fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = find_fx(s, p, "effect.reset")?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
    let layer = comp.layer(lid).ok_or(EngineError::NoComp)?;
    let (w, h) = effectcraft_render::source_size(&s.project, layer);
    let size = if w == 0 { [comp.width as f64, comp.height as f64] } else { [w as f64, h as f64] };
    let lt = layer.layer_time(s.time());
    let g = layer.effects().and_then(|fx| fx.groups().find(|g| g.uid == uid)).ok_or_else(|| bad("effect.reset", "gone"))?;
    let spec = match &g.kind {
        GroupKind::Effect { effect } => effectcraft_effects::find(effect),
        _ => None,
    }
    .ok_or_else(|| bad("effect.reset", "unknown effect"))?;
    let name = g.name.clone();
    s.edit(&format!("Reset {name}"), None, |proj, _| {
        let g = layer_mut(proj, cid, lid)?.props.find_group_mut(uid).ok_or_else(|| bad("effect.reset", "gone"))?;
        for ps in &spec.params {
            if let Some(pr) = g.get_mut(ps.id) {
                pr.set_value_at(lt, effectcraft_effects::default_value(ps, size));
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn last(s: &mut Session, p: &Value) -> Result<Value> {
    let id = s.state.last_effect.clone().ok_or_else(|| bad("effect.applyLast", "no effect applied yet"))?;
    let mut q = p.clone();
    if let Some(o) = q.as_object_mut() {
        o.insert("effect".into(), json!(id));
    } else {
        q = json!({"effect": id});
    }
    apply(s, &q)
}

fn list(_: &mut Session, p: &Value) -> Result<Value> {
    let filter = str_p(p, "filter").map(str::to_ascii_lowercase);
    let v: Vec<Value> = effectcraft_effects::registry()
        .iter()
        .filter(|e| filter.as_ref().is_none_or(|f| e.name.to_ascii_lowercase().contains(f) || e.id.contains(f.as_str())))
        .map(|e| json!({"id": e.id, "name": e.name, "category": e.category, "params": e.params.iter().map(|p| json!({"id": p.id, "name": p.name, "default": p.default.to_json()})).collect::<Vec<_>>()}))
        .collect();
    Ok(json!(v))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("effect.apply", "Apply Effect", [], None, "{effect: id|name (e.g. Gaussian Blur), layers?}", has_layers, apply),
        cmd!("effect.applyLast", "Last Effect", ["Effect"], Some("Cmd+Alt+Shift+E"), "{layers?}", has_layers, last),
        cmd!("effect.removeAll", "Remove All", ["Effect"], Some("Cmd+Shift+E"), "{layers?}", has_layers, remove_all),
        cmd!("effect.remove", "Remove Effect", [], None, "{layer?, effect: index|uid|name}", has_layers, remove),
        cmd!("effect.toggle", "Toggle Effect", [], None, "{layer?, effect, value?}", has_layers, toggle),
        cmd!("effect.reorder", "Reorder Effect", [], None, "{layer?, effect, index}", has_layers, reorder),
        cmd!("effect.duplicate", "Duplicate Effect", [], None, "{layer?, effect}", has_layers, duplicate),
        cmd!("effect.copy", "Copy Effects", [], None, "{layer?, effect? | effects?: [uid|name|index]} (default: the selected effects)", has_layers, copy),
        cmd!("effect.paste", "Paste Effects", [], None, "{layers?} — adds the copied effects to the layers", has_layers, paste),
        cmd!("effect.reset", "Reset Effect", [], None, "{layer?, effect}", has_layers, reset),
        crate::query!("effect.list", "List Effects", "{filter?}", list),
    ]
}
