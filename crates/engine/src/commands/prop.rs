//! Properties, keyframes and expressions (Animation menu + timeline/Effect Controls gestures).

use effectcraft_keyframe::{Ease, Interp, Keyframe, easy_ease, key_at, set_key};
use effectcraft_project::{ItemId, LayerId, Property, Uid};
use effectcraft_time::Tick;
use serde_json::{Value, json};

use super::{CommandSpec, b_p, bad, f_p, has_comp, has_keys, has_layers, layer_mut, layer_p, merge_p, str_p};
use crate::{EngineError, KeyRef, Result, Session, cmd, query};

/// Resolve `{layer, path}` (or `{layer, prop: uid}`) to (comp, layer, prop uid).
fn prop_ref(s: &Session, p: &Value, cmd: &str) -> Result<(ItemId, LayerId, Uid)> {
    let (cid, lid) = layer_p(s, p, cmd)?;
    let layer = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?;
    if let Some(u) = p.get("prop").and_then(Value::as_u64) {
        return layer.props.find(u).map(|pr| (cid, lid, pr.uid)).ok_or_else(|| bad(cmd, format!("no property @{u}")));
    }
    let path = str_p(p, "path").ok_or_else(|| bad(cmd, "missing `path` (e.g. transform/position) or `prop` uid"))?;
    let pr = layer.props.prop(path).ok_or_else(|| bad(cmd, format!("no property `{path}`")))?;
    Ok((cid, lid, pr.uid))
}

fn with_prop<T>(
    s: &mut Session,
    label: &str,
    merge: Option<&str>,
    cid: ItemId,
    lid: LayerId,
    uid: Uid,
    f: impl FnOnce(&mut Property, Tick) -> Result<T>,
) -> Result<T> {
    let t = s.time();
    s.edit(label, merge, |proj, _| {
        let l = layer_mut(proj, cid, lid)?;
        let lt = l.layer_time(t);
        let pr = l.props.find_mut(uid).ok_or_else(|| bad("prop", "property vanished"))?;
        f(pr, lt)
    })
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.set")?;
    let v = p.get("value").ok_or_else(|| bad("prop.set", "missing `value`"))?.clone();
    let at = f_p(p, "time").map(Tick::from_seconds_f64);
    let out = with_prop(s, "Change Property", merge_p(p), cid, lid, uid, |pr, lt| {
        let cur = pr.value_at(lt);
        let mut nv = cur.coerce_json(&v).ok_or_else(|| bad("prop.set", format!("can't use {v} for a {} property", cur.kind_name())))?;
        // Clamp to slider ranges.
        if let effectcraft_project::ParamUi::Slider { min, max, .. } = pr.ui
            && let effectcraft_keyframe::Value::Scalar(x) = nv
        {
            nv = effectcraft_keyframe::Value::Scalar(x.clamp(min, max));
        }
        pr.set_value_at(at.unwrap_or(lt), nv.clone());
        Ok(nv.to_json())
    })?;
    Ok(out)
}

/// Read one property: value at `time` (comp seconds, default the CTI), keyframes and expression.
fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.get")?;
    let l = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?;
    let pr = l.props.find(uid).ok_or_else(|| bad("prop.get", "property vanished"))?;
    let t = f_p(p, "time").map(Tick::from_seconds_f64).unwrap_or_else(|| s.time());
    let keys: Vec<Value> =
        pr.keys.iter().map(|k| json!({"time": k.time.seconds(), "value": k.value.to_json(), "in": k.in_interp.label(), "out": k.out_interp.label()})).collect();
    Ok(json!({
        "layer": lid.0, "uid": pr.uid, "match": pr.match_id, "name": pr.name, "type": pr.value.kind_name(),
        "time": t.seconds(), "value": pr.value_at(l.layer_time(t)).to_json(), "animated": !pr.keys.is_empty(),
        "keys": keys, "expression": pr.expr.as_ref().map(|e| e.text.clone()),
    }))
}

fn toggle_anim(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.toggleAnimation")?;
    let on = b_p(p, "value");
    with_prop(s, "Toggle Animation", None, cid, lid, uid, |pr, lt| {
        if pr.static_only {
            return Err(bad("prop.toggleAnimation", "this property can't be animated"));
        }
        let target = on.unwrap_or(pr.keys.is_empty());
        pr.set_animated(target, lt);
        Ok(json!(target))
    })
}

fn add_key(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.addKey")?;
    let at = f_p(p, "time").map(Tick::from_seconds_f64);
    let value = p.get("value").cloned();
    with_prop(s, "Add Keyframe", None, cid, lid, uid, |pr, lt| {
        let t = at.unwrap_or(lt);
        let cur = pr.value_at(t);
        let v = match &value {
            Some(j) => cur.coerce_json(j).ok_or_else(|| bad("prop.addKey", "bad value"))?,
            None => cur,
        };
        let mut k = Keyframe::new(t, v);
        if pr.hold_only {
            k = k.hold();
        }
        set_key(&mut pr.keys, k);
        Ok(json!(pr.keys.len()))
    })
}

fn toggle_key(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.toggleKey")?;
    with_prop(s, "Add/Remove Keyframe", None, cid, lid, uid, |pr, lt| {
        if let Some(i) = key_at(&pr.keys, lt) {
            if pr.keys.len() == 1 {
                pr.value = pr.keys[0].value.clone();
            }
            pr.keys.remove(i);
            Ok(json!(false))
        } else {
            let mut k = Keyframe::new(lt, pr.value_at(lt));
            if pr.hold_only {
                k = k.hold();
            }
            set_key(&mut pr.keys, k);
            Ok(json!(true))
        }
    })
}

fn set_expr(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.setExpression")?;
    let text = p.get("expression").and_then(Value::as_str).map(str::to_string);
    let enabled = b_p(p, "enabled");
    with_prop(s, "Expression", merge_p(p), cid, lid, uid, |pr, _| {
        match (&text, enabled) {
            (Some(t), _) if t.trim().is_empty() => pr.expr = None,
            (Some(t), e) => pr.expr = Some(effectcraft_project::Expression { text: t.clone(), enabled: e.unwrap_or(true) }),
            (None, Some(e)) => {
                if let Some(x) = &mut pr.expr {
                    x.enabled = e;
                } else if e {
                    pr.expr = Some(effectcraft_project::Expression { text: default_expr(pr), enabled: true });
                }
            }
            (None, None) => {
                pr.expr = if pr.expr.is_some() { None } else { Some(effectcraft_project::Expression { text: default_expr(pr), enabled: true }) };
            }
        }
        Ok(json!(pr.expr.as_ref().map(|e| e.text.clone())))
    })
}

fn default_expr(pr: &Property) -> String {
    let _ = pr;
    "value".to_string()
}

fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.reset")?;
    let def = p.get("default").cloned();
    with_prop(s, "Reset Property", None, cid, lid, uid, |pr, _| {
        pr.keys.clear();
        pr.expr = None;
        if let Some(d) = def.as_ref().and_then(|d| pr.value.coerce_json(d)) {
            pr.value = d;
        }
        Ok(Value::Null)
    })
}

fn select_prop(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, lid) = layer_p(s, p, "prop.select")?;
    let uid = match p.get("prop").and_then(Value::as_u64) {
        Some(u) => u,
        None => prop_ref(s, p, "prop.select")?.2,
    };
    let add = b_p(p, "add").unwrap_or(false);
    if !add {
        s.state.selected_props.clear();
    }
    if !s.state.selected_layers.contains(&lid) {
        s.state.selected_layers = vec![lid];
    }
    s.state.selected_props.push((lid, uid));
    // Selecting a property selects all its keys (as in AE).
    if b_p(p, "selectKeys").unwrap_or(true)
        && let Some(pr) = s.active_comp().and_then(|c| c.layer(lid)).and_then(|l| l.props.find(uid))
    {
        let keys: Vec<KeyRef> = pr.keys.iter().map(|k| KeyRef { layer: lid, prop: uid, time: k.time }).collect();
        if !add {
            s.state.selected_keys.clear();
        }
        s.state.selected_keys.extend(keys);
    }
    Ok(Value::Null)
}

// ---------------------------------------------------------------- keyframes

fn select_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let comp = s.active_comp().ok_or(EngineError::NoComp)?;
    let mut sel = vec![];
    if let Some(Value::Array(keys)) = p.get("keys") {
        for k in keys {
            let lid = k.get("layer").and_then(|v| super::resolve_layer(comp, v));
            let uid = k.get("prop").and_then(Value::as_u64);
            let t = k.get("time").and_then(Value::as_f64).map(Tick::from_seconds_f64);
            if let (Some(l), Some(u), Some(t)) = (lid, uid, t) {
                // Snap to the stored key time.
                let kt = comp.layer(l).and_then(|ly| ly.props.find(u)).and_then(|pr| pr.keys.iter().map(|k| k.time).min_by_key(|kt| (kt.0 - t.0).abs()));
                if let Some(kt) = kt {
                    sel.push(KeyRef { layer: l, prop: u, time: kt });
                }
            }
        }
    }
    if b_p(p, "add").unwrap_or(false) {
        for k in sel {
            if !s.state.selected_keys.contains(&k) {
                s.state.selected_keys.push(k);
            }
        }
    } else {
        s.state.selected_keys = sel;
    }
    Ok(json!(s.state.selected_keys.len()))
}

/// Apply `f` to every selected key; keeps the selection pointing at moved keys.
fn edit_keys(s: &mut Session, label: &str, merge: Option<&str>, f: impl Fn(&mut Vec<Keyframe>, usize, &mut Tick) -> bool) -> Result<Value> {
    let cid = s.active_comp_id().ok_or(EngineError::NoComp)?;
    let sel = s.state.selected_keys.clone();
    s.edit(label, merge, |proj, st| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        let mut new_sel = vec![];
        let mut groups: std::collections::BTreeMap<(LayerId, Uid), Vec<Tick>> = Default::default();
        for k in &sel {
            groups.entry((k.layer, k.prop)).or_default().push(k.time);
        }
        for ((lid, uid), times) in groups {
            let Some(pr) = comp.layer_mut(lid).and_then(|l| l.props.find_mut(uid)) else { continue };
            // Process in an order that avoids collisions when moving.
            let mut idx: Vec<(usize, Tick)> = times.iter().filter_map(|t| key_at(&pr.keys, *t).map(|i| (i, *t))).collect();
            idx.sort_by_key(|x| x.0);
            let mut moved: Vec<Keyframe> = vec![];
            let mut keep_times = vec![];
            for (i, _) in idx.iter().rev() {
                let mut t = pr.keys[*i].time;
                if f(&mut pr.keys, *i, &mut t) {
                    let mut k = pr.keys.remove(*i);
                    k.time = t;
                    moved.push(k);
                } else if *i < pr.keys.len() {
                    keep_times.push(pr.keys[*i].time);
                }
            }
            for k in moved {
                new_sel.push(KeyRef { layer: lid, prop: uid, time: k.time });
                match pr.keys.binary_search_by(|x| x.time.cmp(&k.time)) {
                    Ok(j) => pr.keys[j] = k,
                    Err(j) => pr.keys.insert(j, k),
                }
            }
            for t in keep_times {
                new_sel.push(KeyRef { layer: lid, prop: uid, time: t });
            }
            if pr.keys.is_empty() {
                // Deleting every key leaves the value at the last key.
            }
        }
        st.selected_keys = new_sel;
        Ok(json!(st.selected_keys.len()))
    })
}

fn move_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let d = Tick::from_seconds_f64(f_p(p, "delta").ok_or_else(|| bad("keys.move", "missing `delta` (seconds)"))?);
    let fr = s.active_comp().map(|c| c.frame_rate);
    edit_keys(s, "Move Keyframes", merge_p(p), move |_, _, t| {
        let nt = *t + d;
        *t = fr.map(|r| r.snap_nearest(nt)).unwrap_or(nt);
        true
    })
}

fn delete_keys(s: &mut Session, _: &Value) -> Result<Value> {
    let cid = s.active_comp_id().ok_or(EngineError::NoComp)?;
    let sel = s.state.selected_keys.clone();
    s.edit("Delete Keyframes", None, |proj, st| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        for k in &sel {
            if let Some(pr) = comp.layer_mut(k.layer).and_then(|l| l.props.find_mut(k.prop))
                && let Some(i) = key_at(&pr.keys, k.time)
            {
                if pr.keys.len() == 1 {
                    pr.value = pr.keys[0].value.clone();
                }
                pr.keys.remove(i);
            }
        }
        st.selected_keys.clear();
        Ok(json!(sel.len()))
    })
}

fn ease(s: &mut Session, p: &Value) -> Result<Value> {
    let which = str_p(p, "which").unwrap_or("both").to_string();
    edit_keys(s, "Easy Ease", None, move |keys, i, _| {
        easy_ease(&mut keys[i], which != "out", which != "in");
        false
    })
}

fn interpolation(s: &mut Session, p: &Value) -> Result<Value> {
    let parse = |k: &str| match str_p(p, k).map(|v| v.to_ascii_lowercase()) {
        Some(v) if v == "linear" => Some(Interp::Linear),
        Some(v) if v == "bezier" => Some(Interp::Bezier),
        Some(v) if v == "hold" => Some(Interp::Hold),
        _ => None,
    };
    let both = parse("interpolation");
    let inn = parse("in").or(both);
    let out = parse("out").or(both);
    let auto = b_p(p, "autoBezier");
    edit_keys(s, "Keyframe Interpolation", None, move |keys, i, _| {
        let n = keys[i].value.dims().max(1);
        if let Some(x) = inn {
            keys[i].in_interp = x;
            if x == Interp::Bezier && keys[i].in_ease.is_empty() {
                keys[i].in_ease = vec![Ease::default(); n];
            }
        }
        if let Some(x) = out {
            keys[i].out_interp = x;
            if x == Interp::Bezier && keys[i].out_ease.is_empty() {
                keys[i].out_ease = vec![Ease::default(); n];
            }
        }
        if let Some(a) = auto {
            keys[i].auto_bezier = a;
            if a {
                keys[i].in_interp = Interp::Bezier;
                keys[i].out_interp = Interp::Bezier;
            }
        }
        false
    })
}

fn toggle_hold(s: &mut Session, _: &Value) -> Result<Value> {
    edit_keys(s, "Toggle Hold Keyframe", None, |keys, i, _| {
        let k = &mut keys[i];
        k.out_interp = if k.out_interp == Interp::Hold { Interp::Linear } else { Interp::Hold };
        false
    })
}

fn velocity(s: &mut Session, p: &Value) -> Result<Value> {
    let in_speed = f_p(p, "inSpeed");
    let in_inf = f_p(p, "inInfluence").map(|v| v / 100.0);
    let out_speed = f_p(p, "outSpeed");
    let out_inf = f_p(p, "outInfluence").map(|v| v / 100.0);
    edit_keys(s, "Keyframe Velocity", merge_p(p), move |keys, i, _| {
        let n = keys[i].value.dims().max(1);
        let k = &mut keys[i];
        if in_speed.is_some() || in_inf.is_some() {
            k.in_interp = Interp::Bezier;
            if k.in_ease.is_empty() {
                k.in_ease = vec![Ease::default(); n];
            }
            for e in &mut k.in_ease {
                if let Some(v) = in_speed {
                    e.speed = v;
                }
                if let Some(v) = in_inf {
                    e.influence = v.clamp(0.001, 1.0);
                }
            }
        }
        if out_speed.is_some() || out_inf.is_some() {
            k.out_interp = Interp::Bezier;
            if k.out_ease.is_empty() {
                k.out_ease = vec![Ease::default(); n];
            }
            for e in &mut k.out_ease {
                if let Some(v) = out_speed {
                    e.speed = v;
                }
                if let Some(v) = out_inf {
                    e.influence = v.clamp(0.001, 1.0);
                }
            }
        }
        k.auto_bezier = false;
        false
    })
}

fn convert_expr_to_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.convertExpressionToKeyframes")?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let layer = comp.layer(lid).ok_or(EngineError::NoComp)?.clone();
    let pr = layer.props.find(uid).ok_or_else(|| bad("prop.convertExpressionToKeyframes", "no property"))?.clone();
    let mut keys = vec![];
    let fd = comp.frame_duration();
    let mut t = layer.in_point;
    while t < layer.out_point {
        let ctx = effectcraft_render::EvalCtx { project: &s.project, comp_id: cid, comp: &comp, time: t, expr: s.expr.as_deref() };
        keys.push(Keyframe::new(layer.layer_time(t), ctx.value(&layer, &pr)));
        t += fd;
    }
    let n = keys.len();
    with_prop(s, "Convert Expression to Keyframes", None, cid, lid, uid, move |pr, _| {
        pr.keys = keys;
        if let Some(e) = &mut pr.expr {
            e.enabled = false;
        }
        Ok(json!(n))
    })
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        query!("prop.get", "Get Property", "{layer?, path|prop, time? (comp s)}", get),
        cmd!("prop.set", "Set Property Value", [], None, "{layer?, path|prop, value, time?, merge?}", has_layers, set),
        cmd!("prop.toggleAnimation", "Toggle Stopwatch", [], None, "{layer?, path|prop, value?}", has_layers, toggle_anim),
        cmd!("prop.addKey", "Add Keyframe", [], None, "{layer?, path|prop, time?, value?}", has_layers, add_key),
        cmd!("prop.toggleKey", "Add or Remove Keyframe at Current Time", [], None, "{layer?, path|prop}", has_layers, toggle_key),
        cmd!("prop.setExpression", "Add Expression", ["Animation"], Some("Alt+Shift+="), "{layer?, path|prop, expression?, enabled?}", has_layers, set_expr),
        cmd!("prop.reset", "Reset Property", [], None, "{layer?, path|prop, default?}", has_layers, reset),
        cmd!("prop.select", "Select Property", [], None, "{layer?, path|prop, add?, selectKeys?}", has_layers, select_prop),
        cmd!(
            "prop.convertExpressionToKeyframes",
            "Convert Expression to Keyframes",
            ["Animation", "Keyframe Assistant"],
            None,
            "{layer?, path|prop}",
            has_layers,
            convert_expr_to_keys
        ),
        cmd!("keys.select", "Select Keyframes", [], None, "{keys: [{layer, prop, time}], add?}", has_comp, select_keys),
        cmd!("keys.move", "Move Keyframes", [], None, "{delta (s), merge?}", has_keys, move_keys),
        cmd!("keys.delete", "Delete Keyframes", [], None, "{}", has_keys, delete_keys),
        cmd!("keys.easyEase", "Easy Ease", ["Animation", "Keyframe Assistant"], Some("F9"), "{which?: both|in|out}", has_keys, ease),
        cmd!("keys.easyEaseIn", "Easy Ease In", ["Animation", "Keyframe Assistant"], Some("Shift+F9"), "{}", has_keys, |s, _| ease(s, &json!({"which": "in"}))),
        cmd!("keys.easyEaseOut", "Easy Ease Out", ["Animation", "Keyframe Assistant"], Some("Cmd+Shift+F9"), "{}", has_keys, |s, _| ease(
            s,
            &json!({"which": "out"})
        )),
        cmd!("keys.toggleHold", "Toggle Hold Keyframe", ["Animation"], Some("Cmd+Alt+H"), "{}", has_keys, toggle_hold),
        cmd!(
            "keys.interpolation",
            "Keyframe Interpolation…",
            ["Animation"],
            Some("Cmd+Alt+K"),
            "{interpolation?|in?|out?: linear|bezier|hold, autoBezier?}",
            has_keys,
            interpolation
        ),
        cmd!(
            "keys.velocity",
            "Keyframe Velocity…",
            ["Animation"],
            Some("Cmd+Shift+K"),
            "{inSpeed?, inInfluence? %, outSpeed?, outInfluence? %}",
            has_keys,
            velocity
        ),
    ]
}
