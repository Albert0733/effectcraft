//! Puppet tools: pins on the layer's Puppet effect (Position, Advanced, Bend, Starch and
//! Overlap pins), mesh options, and a query of the current mesh for overlays and agents.
//!
//! Pin positions are layer space. A new pin is attached to the mesh under it: the click is
//! mapped back through the current deformation to the rest mesh (its hidden rest position), and
//! Position / Advanced pins get a Position keyframe at the current time, as in After Effects.

use effectcraft_effects::puppet::{self, MeshOpts, PinKind};
use effectcraft_effects::{Buf, Params};
use effectcraft_keyframe::Value as KV;
use effectcraft_project::build::Ids;
use effectcraft_project::{ItemId, LayerId, Node, PropGroup, Uid};
use effectcraft_render::{EvalCtx, RenderOpts, Renderer};
use effectcraft_time::Tick;
use serde_json::{Value, json};

use super::{CommandSpec, bad, f_p, has_comp, layer_mut, layer_p, merge_p, str_p};
use crate::{EngineError, Result, Session, cmd};

fn pt(v: Option<&Value>) -> Option<[f64; 2]> {
    let a = v?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

/// The Puppet effect's input pixels and evaluated parameters at comp time `t`.
pub fn puppet_eval(s: &Session, cid: ItemId, lid: LayerId, fx_uid: Uid, t: Tick) -> Option<(Buf, Params)> {
    let comp = s.project.comp(cid)?;
    let layer = comp.layer(lid)?;
    let fx = layer.effects()?;
    let index = fx.groups().position(|g| g.uid == fx_uid)?;
    let g = fx.groups().nth(index)?;
    let ctx = EvalCtx { project: &s.project, comp_id: cid, comp, time: t, expr: s.expr.as_deref() };
    let mut r = Renderer::new(&s.project, &*s.footage, RenderOpts::default());
    r.expr = s.expr.as_deref();
    r.cache = Some(&s.layer_cache);
    let buf = (*r.layer_input(&ctx, layer, index)?).clone();
    let params = effectcraft_effects::flatten_params(g, &mut |p| ctx.value(layer, p));
    Some((buf, params))
}

fn find_puppet(s: &Session, cid: ItemId, lid: LayerId) -> Option<Uid> {
    s.project.comp(cid)?.layer(lid)?.effects()?.groups().find(|g| puppet::is_puppet(g)).map(|g| g.uid)
}

fn snapped(s: &Session, cid: ItemId, p: &Value) -> Result<Tick> {
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
    Ok(comp.frame_rate.snap_nearest(super::time_p(s, p, Some(comp))))
}

fn layer_time(s: &Session, cid: ItemId, lid: LayerId, t: Tick) -> Result<Tick> {
    let l = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?;
    Ok(l.layer_time(t))
}

fn add_pin(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "puppet.addPin";
    let (cid, lid) = layer_p(s, p, cmd)?;
    let kind = match str_p(p, "kind") {
        Some(k) => PinKind::from_name(k).ok_or_else(|| bad(cmd, format!("unknown kind `{k}` (position | advanced | bend | starch | overlap)")))?,
        None => PinKind::Position,
    };
    let at = pt(p.get("position")).ok_or_else(|| bad(cmd, "missing `position` [x, y] (layer space)"))?;
    let t = snapped(s, cid, p)?;
    let lt = layer_time(s, cid, lid, t)?;
    {
        let l = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?;
        if l.effects().is_none() || !l.source.is_av() {
            return Err(bad(cmd, "the Puppet tools work on footage, solid, text, shape and precomp layers"));
        }
    }
    // Which mesh: the one under the click (through the current deformation), else a new one.
    let fx_uid = find_puppet(s, cid, lid);
    let want_mesh = p.get("mesh").and_then(Value::as_u64);
    let new_mesh = p.get("newMesh").and_then(Value::as_bool).unwrap_or(false);
    let mut target: Option<(Uid, [f64; 2])> = None;
    if let Some(fu) = fx_uid
        && !new_mesh
        && let Some((buf, params)) = puppet_eval(s, cid, lid, fu, t)
    {
        let meshes = puppet::overlay(&buf, &params);
        for (uid, mesh, def, _) in &meshes {
            if want_mesh.is_some_and(|w| w != *uid) || mesh.tris.is_empty() {
                continue;
            }
            let inside = mesh.tris.iter().any(|tr| {
                let (a, b, c) = (def[tr[0]], def[tr[1]], def[tr[2]]);
                let cr = |o: [f64; 2], u: [f64; 2], v: [f64; 2]| (u[0] - o[0]) * (v[1] - o[1]) - (u[1] - o[1]) * (v[0] - o[0]);
                cr(a, b, at) >= 0.0 && cr(b, c, at) >= 0.0 && cr(c, a, at) >= 0.0
            });
            let near = def.iter().map(|v| (v[0] - at[0]).hypot(v[1] - at[1])).fold(f64::MAX, f64::min) < 12.0;
            if inside || near || want_mesh.is_some() {
                target = Some((*uid, puppet::unmap(mesh, def, at)));
                break;
            }
        }
        if target.is_none()
            && let Some(w) = want_mesh
        {
            return Err(bad(cmd, format!("no mesh {w}")));
        }
    }
    let opts = MeshOpts {
        density: f_p(p, "density").unwrap_or(s.state.puppet.density),
        expansion: f_p(p, "expansion").unwrap_or(s.state.puppet.expansion),
        triangles: f_p(p, "triangles").unwrap_or(s.state.puppet.triangles),
    };
    let size = {
        let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
        let l = comp.layer(lid).ok_or(EngineError::NoComp)?;
        let (w, h) = effectcraft_render::source_size(&s.project, l);
        if w == 0 { [comp.width as f64, comp.height as f64] } else { [w as f64, h as f64] }
    };
    let label = match kind {
        PinKind::Starch => "Add Starch Pin",
        PinKind::Overlap => "Add Overlap Pin",
        _ => "Add Puppet Pin",
    };
    let (fx, mesh, pin) = s.edit(label, None, |proj, st| {
        let mut next = proj.next_id;
        let l = layer_mut(proj, cid, lid)?;
        let fxg = l.props.sub_mut("effects").ok_or_else(|| bad(cmd, "no effects"))?;
        let mut ids = Ids(&mut next);
        let fx_uid = match fx_uid {
            Some(u) => u,
            None => {
                let spec = effectcraft_effects::find(puppet::ID).ok_or_else(|| EngineError::Other("Puppet effect missing".into()))?;
                let g = effectcraft_effects::instantiate(spec, &mut ids, "Puppet", size);
                let u = g.uid;
                fxg.children.push(g.into());
                u
            }
        };
        let pg = fxg.find_group_mut(fx_uid).ok_or_else(|| bad(cmd, "Puppet effect gone"))?;
        let (mesh_uid, rest) = match target {
            Some((m, r)) => (m, r),
            None => {
                let n = puppet::meshes(pg).count();
                let g = puppet::mesh_group(&mut ids, &format!("Mesh {}", n + 1), at, &opts);
                let u = g.uid;
                pg.children.push(g.into());
                (u, at)
            }
        };
        let name = puppet::next_pin_name(pg, kind);
        let mg = pg.find_group_mut(mesh_uid).ok_or_else(|| bad(cmd, "mesh gone"))?;
        let mut pin = puppet::pin_group(&mut ids, &name, kind, rest);
        if let Some(pos) = pin.get_mut("position") {
            if kind.moves() {
                pos.value = KV::Vec2(at);
                pos.set_animated(true, lt);
            } else {
                pos.value = KV::Vec2(rest);
            }
        }
        let pin_uid = pin.uid;
        let deform = mg.sub_mut("deform").ok_or_else(|| bad(cmd, "mesh has no Deform group"))?;
        deform.children.push(pin.into());
        proj.next_id = next;
        st.selected_props = vec![(lid, pin_uid)];
        Ok((fx_uid, mesh_uid, pin_uid))
    })?;
    Ok(json!({"effect": fx, "mesh": mesh, "pin": pin, "rest": [target.map(|x| x.1).unwrap_or(at)[0], target.map(|x| x.1).unwrap_or(at)[1]]}))
}

/// Find a pin group (uid or name) in the layer's Puppet effects.
fn pin_uid(s: &Session, cid: ItemId, lid: LayerId, key: &Value, cmd: &str) -> Result<Uid> {
    let l = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?;
    let fx = l.effects().ok_or_else(|| bad(cmd, "no effects"))?;
    for g in fx.groups().filter(|g| puppet::is_puppet(g)) {
        for m in puppet::meshes(g) {
            for pin in puppet::pins(m) {
                let hit = match key {
                    Value::Number(n) => n.as_u64() == Some(pin.uid),
                    Value::String(name) => &pin.name == name,
                    _ => false,
                };
                if hit {
                    return Ok(pin.uid);
                }
            }
        }
    }
    Err(bad(cmd, format!("no puppet pin {key}")))
}

fn with_pin<T>(s: &mut Session, p: &Value, cmd: &str, label: &str, f: impl FnOnce(&mut PropGroup, Tick) -> Result<T>) -> Result<T> {
    let (cid, lid) = layer_p(s, p, cmd)?;
    let key = p.get("pin").cloned().ok_or_else(|| bad(cmd, "missing `pin` (uid or name)"))?;
    let uid = pin_uid(s, cid, lid, &key, cmd)?;
    let t = snapped(s, cid, p)?;
    let lt = layer_time(s, cid, lid, t)?;
    s.edit(label, merge_p(p), |proj, st| {
        let l = layer_mut(proj, cid, lid)?;
        let g = l.props.find_group_mut(uid).ok_or_else(|| bad(cmd, "pin gone"))?;
        let r = f(g, lt)?;
        st.selected_props = vec![(lid, uid)];
        Ok(r)
    })
}

fn move_pin(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "puppet.movePin";
    let to = pt(p.get("position")).ok_or_else(|| bad(cmd, "missing `position` [x, y] (layer space)"))?;
    with_pin(s, p, cmd, "Move Puppet Pin", |g, lt| {
        let kind = PinKind::from_index(g.get("kind").map(|k| k.value.as_enum()).unwrap_or(0));
        let pos = g.get_mut("position").ok_or_else(|| bad(cmd, "Bend pins have no position (use puppet.setPin rotation/scale)"))?;
        pos.set_value_at(lt, KV::Vec2(to));
        // Starch and Overlap pins sit on the rest mesh: moving them moves their region.
        if !kind.moves()
            && let Some(r) = g.get_mut("rest")
        {
            r.value = KV::Vec2(to);
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn set_pin(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "puppet.setPin";
    let fields: Vec<(&str, KV)> = [("scale", "scale"), ("rotation", "rotation"), ("amount", "amount"), ("extent", "extent"), ("inFront", "in_front")]
        .iter()
        .filter_map(|(k, m)| f_p(p, k).map(|v| (*m, KV::Scalar(v))))
        .chain(pt(p.get("position")).map(|v| ("position", KV::Vec2(v))))
        .collect();
    if fields.is_empty() {
        return Err(bad(cmd, "nothing to set (position, scale, rotation, amount, extent, inFront)"));
    }
    with_pin(s, p, cmd, "Edit Puppet Pin", |g, lt| {
        for (m, v) in fields {
            let pr = g.get_mut(m).ok_or_else(|| bad(cmd, format!("this pin has no `{m}`")))?;
            pr.set_value_at(lt, v);
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn remove_pin(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "puppet.removePin";
    let (cid, lid) = layer_p(s, p, cmd)?;
    let key = p.get("pin").cloned().ok_or_else(|| bad(cmd, "missing `pin` (uid or name)"))?;
    let uid = pin_uid(s, cid, lid, &key, cmd)?;
    s.edit("Delete Puppet Pin", None, |proj, st| {
        let l = layer_mut(proj, cid, lid)?;
        if let Some(parent) = l.props.parent_of_mut(uid) {
            parent.children.retain(|c| c.uid() != uid);
        }
        st.selected_props.retain(|(_, u)| *u != uid);
        Ok(())
    })?;
    Ok(Value::Null)
}

fn mesh_opts(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "puppet.mesh";
    let fields: Vec<(&str, f64)> = ["density", "expansion", "triangles"].iter().filter_map(|k| f_p(p, k).map(|v| (*k, v))).collect();
    if let Some(b) = p.get("showMesh").and_then(Value::as_bool) {
        s.state.puppet.show_mesh = b;
    }
    if fields.is_empty() {
        return Ok(serde_json::to_value(&s.state.puppet).unwrap_or(Value::Null));
    }
    // Without a layer: set the tool defaults for new meshes.
    let has_layer = p.get("layer").is_some() || !s.state.selected_layers.is_empty();
    let (cid, lid) = match layer_p(s, p, cmd) {
        Ok(x) => x,
        Err(_) if !has_layer => {
            for (k, v) in &fields {
                match *k {
                    "density" => s.state.puppet.density = v.clamp(0.0, 100.0),
                    "expansion" => s.state.puppet.expansion = *v,
                    _ => s.state.puppet.triangles = v.max(10.0),
                }
            }
            return Ok(serde_json::to_value(&s.state.puppet).unwrap_or(Value::Null));
        }
        Err(e) => return Err(e),
    };
    let fx = find_puppet(s, cid, lid).ok_or_else(|| bad(cmd, "the layer has no Puppet effect (add a pin first)"))?;
    let want = p.get("mesh").and_then(Value::as_u64);
    let t = snapped(s, cid, p)?;
    let lt = layer_time(s, cid, lid, t)?;
    for (k, v) in &fields {
        match *k {
            "density" => s.state.puppet.density = v.clamp(0.0, 100.0),
            "expansion" => s.state.puppet.expansion = *v,
            _ => s.state.puppet.triangles = v.max(10.0),
        }
    }
    s.edit("Puppet Mesh", merge_p(p), |proj, _| {
        let l = layer_mut(proj, cid, lid)?;
        let g = l.props.find_group_mut(fx).ok_or_else(|| bad(cmd, "Puppet effect gone"))?;
        let mut done = 0;
        for c in g.children.iter_mut() {
            let Node::Group(m) = c else { continue };
            if m.match_id != "mesh" || want.is_some_and(|w| w != m.uid) {
                continue;
            }
            for (k, v) in &fields {
                if let Some(pr) = m.get_mut(k) {
                    pr.set_value_at(lt, KV::Scalar(*v));
                }
            }
            done += 1;
        }
        if done == 0 { Err(bad(cmd, "no such mesh")) } else { Ok(()) }
    })?;
    Ok(Value::Null)
}

/// Meshes, pins and the deformed mesh at the current time (layer space).
fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "puppet.info")?;
    let Some(fx) = find_puppet(s, cid, lid) else { return Ok(json!({"meshes": []})) };
    let t = snapped(s, cid, p)?;
    let (buf, params) = puppet_eval(s, cid, lid, fx, t).ok_or_else(|| bad("puppet.info", "the layer has no pixels"))?;
    let meshes: Vec<Value> = puppet::overlay(&buf, &params)
        .into_iter()
        .map(|(uid, mesh, def, pins)| {
            json!({
                "mesh": uid,
                "triangles": mesh.tris.len(),
                "vertices": mesh.verts.len(),
                "outline": mesh.outline,
                "deformed": def,
                "tris": mesh.tris,
                "pins": pins.iter().map(|pn| json!({"pin": pn.uid, "kind": pn.kind.name(), "rest": pn.rest, "position": pn.position})).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({"effect": fx, "meshes": meshes}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "puppet.addPin",
            "Add Puppet Pin",
            [],
            None,
            "{layer?, kind?: position|advanced|bend|starch|overlap, position: [x,y] (layer space), time? (s) | frame?, mesh?: uid, newMesh?, density?, expansion?, triangles?}",
            has_comp,
            add_pin
        ),
        cmd!("puppet.movePin", "Move Puppet Pin", [], None, "{layer?, pin: uid | name, position: [x,y], time? | frame?, merge?}", has_comp, move_pin),
        cmd!(
            "puppet.setPin",
            "Edit Puppet Pin",
            [],
            None,
            "{layer?, pin: uid | name, position?, scale? (%), rotation? (°), amount? (%), extent? (px), inFront?, time? | frame?, merge?}",
            has_comp,
            set_pin
        ),
        cmd!("puppet.removePin", "Delete Puppet Pin", [], None, "{layer?, pin: uid | name}", has_comp, remove_pin),
        cmd!(
            "puppet.mesh",
            "Puppet Mesh Options",
            [],
            None,
            "{layer?, mesh?: uid, density?, expansion?, triangles?, showMesh?, time?, merge?}",
            super::always,
            mesh_opts
        ),
        crate::query!("puppet.info", "Puppet Mesh Info", "{layer?, time? | frame?}", info),
    ]
}

/// Puppet tool options (the Tools bar's Mesh: Show / Expansion / Density for new meshes).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PuppetOptions {
    pub density: f64,
    pub expansion: f64,
    pub triangles: f64,
    /// Show the mesh in the viewer.
    pub show_mesh: bool,
}

impl Default for PuppetOptions {
    fn default() -> Self {
        let d = MeshOpts::default();
        PuppetOptions { density: d.density, expansion: d.expansion, triangles: d.triangles, show_mesh: false }
    }
}
