//! Masks drawn and edited in the viewer: Pen tool (new mask, add vertex, Bezier tangents, close
//! path), vertex selection, moving and deleting vertices, and Layer ▸ Mask ▸ Remove (All).
//!
//! Coordinates are in layer space. Edits of an animated Mask Path set a key at the current time.

use effectcraft_keyframe::{ShapePath, Value as KV};
use effectcraft_project::build::{self, Ids};
use effectcraft_project::{Layer, MaskMode, PropGroup, Uid};
use serde_json::{Value, json};

use super::{CommandSpec, b_p, bad, has_comp, has_layers, layer_mut, layer_p, merge_p, resolve_layer, str_p};
use crate::{EngineError, Result, Session, VertexRef, cmd};

fn pt(v: Option<&Value>) -> Option<[f64; 2]> {
    let a = v?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

fn pts(v: Option<&Value>) -> Vec<[f64; 2]> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|p| pt(Some(p))).collect()).unwrap_or_default()
}

/// Find a mask group by uid, 1-based index or name.
fn find_mask<'a>(masks: &'a mut PropGroup, key: &Value) -> Option<&'a mut PropGroup> {
    let i = match key {
        Value::Number(n) => {
            let n = n.as_u64()?;
            masks.children.iter().position(|c| c.uid() == n).or_else(|| (n as usize).checked_sub(1).filter(|i| *i < masks.children.len()))?
        }
        Value::String(s) => masks.children.iter().position(|c| c.name() == s)?,
        _ => return None,
    };
    masks.children.get_mut(i)?.as_group_mut()
}

/// Edit the mask path of (layer, mask) at the current time.
fn edit_path<T>(s: &mut Session, p: &Value, label: &str, f: impl FnOnce(&mut ShapePath) -> Result<T>) -> Result<(Uid, T)> {
    let (cid, lid) = layer_p(s, p, label)?;
    let key = p.get("mask").cloned().ok_or_else(|| bad(label, "missing `mask` (uid, index or name)"))?;
    let t = s.time();
    s.edit(label, merge_p(p), |proj, _| {
        let l = layer_mut(proj, cid, lid)?;
        let lt = l.layer_time(t);
        let masks = l.props.sub_mut("masks").ok_or_else(|| bad(label, "this layer has no masks"))?;
        let g = find_mask(masks, &key).ok_or_else(|| bad(label, "no such mask"))?;
        let uid = g.uid;
        let pr = g.get_mut("path").ok_or_else(|| bad(label, "mask has no path"))?;
        let KV::Path(mut sp) = pr.value_at(lt) else { return Err(bad(label, "not a path")) };
        let r = f(&mut sp)?;
        pr.set_value_at(lt, KV::Path(sp));
        Ok((uid, r))
    })
}

/// Pen tool: a new (open by default) mask from points.
fn new_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "mask.new")?;
    let v = pts(p.get("vertices"));
    if v.is_empty() {
        return Err(bad("mask.new", "need `vertices` [[x,y], …] in layer space"));
    }
    let n = v.len();
    let mut ins = pts(p.get("inTangents"));
    let mut outs = pts(p.get("outTangents"));
    ins.resize(n, [0.0; 2]);
    outs.resize(n, [0.0; 2]);
    let path = ShapePath { vertices: v, in_tangents: ins, out_tangents: outs, closed: b_p(p, "closed").unwrap_or(false) };
    let mode = str_p(p, "mode").and_then(MaskMode::from_name).unwrap_or(MaskMode::Add);
    let uid = s.edit("New Mask", None, |proj, st| {
        let mut next = proj.next_id;
        let l = layer_mut(proj, cid, lid)?;
        let masks = l.props.sub_mut("masks").ok_or_else(|| bad("mask.new", "this layer can't have masks"))?;
        let k = masks.children.len();
        let g = build::mask(&mut Ids(&mut next), &format!("Mask {}", k + 1), path, mode, build::MASK_COLORS[k % build::MASK_COLORS.len()]);
        let uid = g.uid;
        masks.children.push(g.into());
        proj.next_id = next;
        st.selected_vertices = vec![VertexRef { layer: lid, mask: uid, index: n - 1 }];
        Ok(uid)
    })?;
    Ok(json!({"mask": uid}))
}

fn add_vertex(s: &mut Session, p: &Value) -> Result<Value> {
    let point = pt(p.get("point")).ok_or_else(|| bad("mask.addVertex", "missing `point` [x,y]"))?;
    let tin = pt(p.get("in")).unwrap_or([0.0; 2]);
    let tout = pt(p.get("out")).unwrap_or([0.0; 2]);
    let at = p.get("index").and_then(Value::as_u64).map(|i| i as usize);
    let (_, lid) = layer_p(s, p, "mask.addVertex")?;
    let (uid, i) = edit_path(s, p, "mask.addVertex", |sp| {
        let i = at.unwrap_or(sp.vertices.len()).min(sp.vertices.len());
        sp.vertices.insert(i, point);
        sp.in_tangents.insert(i, tin);
        sp.out_tangents.insert(i, tout);
        Ok(i)
    })?;
    s.state.selected_vertices = vec![VertexRef { layer: lid, mask: uid, index: i }];
    Ok(json!(i))
}

fn set_vertex(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("mask.setVertex", "missing `index`"))? as usize;
    let (point, tin, tout) = (pt(p.get("point")), pt(p.get("in")), pt(p.get("out")));
    edit_path(s, p, "mask.setVertex", |sp| {
        if i >= sp.vertices.len() {
            return Err(bad("mask.setVertex", "no such vertex"));
        }
        if let Some(v) = point {
            sp.vertices[i] = v;
        }
        if let Some(v) = tin {
            sp.in_tangents[i] = v;
        }
        if let Some(v) = tout {
            sp.out_tangents[i] = v;
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn set_closed(s: &mut Session, p: &Value) -> Result<Value> {
    let c = b_p(p, "closed").unwrap_or(true);
    edit_path(s, p, "mask.setClosed", |sp| {
        sp.closed = c;
        Ok(())
    })?;
    Ok(json!(c))
}

/// Selected (or given) vertices grouped per (layer, mask).
fn vertex_groups(s: &Session, p: &Value) -> Result<std::collections::BTreeMap<(u64, Uid), Vec<usize>>> {
    let comp = s.active_comp().ok_or(EngineError::NoComp)?;
    let mut g: std::collections::BTreeMap<(u64, Uid), Vec<usize>> = Default::default();
    if let Some(Value::Array(a)) = p.get("vertices") {
        for v in a {
            let l = v.get("layer").and_then(|x| resolve_layer(comp, x)).ok_or_else(|| bad("mask", "vertex needs `layer`"))?;
            let m = v.get("mask").and_then(Value::as_u64).ok_or_else(|| bad("mask", "vertex needs `mask` uid"))?;
            let i = v.get("index").and_then(Value::as_u64).ok_or_else(|| bad("mask", "vertex needs `index`"))?;
            g.entry((l.0, m)).or_default().push(i as usize);
        }
    } else {
        for v in &s.state.selected_vertices {
            g.entry((v.layer.0, v.mask)).or_default().push(v.index);
        }
    }
    Ok(g)
}

fn select_vertices(s: &mut Session, p: &Value) -> Result<Value> {
    let g = vertex_groups(s, &json!({"vertices": p.get("vertices").cloned().unwrap_or(json!([]))}))?;
    let sel: Vec<VertexRef> =
        g.into_iter().flat_map(|((l, m), is)| is.into_iter().map(move |i| VertexRef { layer: effectcraft_project::LayerId(l), mask: m, index: i })).collect();
    if b_p(p, "add").unwrap_or(false) {
        for v in sel {
            if let Some(i) = s.state.selected_vertices.iter().position(|x| *x == v) {
                if b_p(p, "toggle").unwrap_or(false) {
                    s.state.selected_vertices.remove(i);
                }
            } else {
                s.state.selected_vertices.push(v);
            }
        }
    } else {
        s.state.selected_vertices = sel;
    }
    Ok(json!(s.state.selected_vertices.len()))
}

fn has_vertices(s: &Session) -> std::result::Result<(), String> {
    has_comp(s)?;
    if s.state.selected_vertices.is_empty() { Err("select mask vertices first".into()) } else { Ok(()) }
}

/// Apply `f` to the selected vertices' paths (each mask once).
fn edit_vertices(s: &mut Session, p: &Value, label: &str, f: impl Fn(&mut ShapePath, &[usize])) -> Result<Value> {
    let cid = s.active_comp_id().ok_or(EngineError::NoComp)?;
    let groups = vertex_groups(s, p)?;
    let t = s.time();
    s.edit(label, merge_p(p), |proj, _| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        for ((l, m), idx) in &groups {
            let Some(layer) = comp.layer_mut(effectcraft_project::LayerId(*l)) else { continue };
            let lt = layer.layer_time(t);
            let Some(pr) = layer.props.sub_mut("masks").and_then(|ms| ms.find_group_mut(*m)).and_then(|g| g.get_mut("path")) else { continue };
            let KV::Path(mut sp) = pr.value_at(lt) else { continue };
            f(&mut sp, idx);
            pr.set_value_at(lt, KV::Path(sp));
        }
        Ok(())
    })?;
    Ok(json!(groups.values().map(Vec::len).sum::<usize>()))
}

fn move_vertices(s: &mut Session, p: &Value) -> Result<Value> {
    let d = pt(p.get("delta")).ok_or_else(|| bad("mask.moveVertices", "missing `delta` [dx,dy] (layer space)"))?;
    edit_vertices(s, p, "Move Mask Vertices", |sp, idx| {
        for &i in idx {
            if let Some(v) = sp.vertices.get_mut(i) {
                v[0] += d[0];
                v[1] += d[1];
            }
        }
    })
}

fn delete_vertices(s: &mut Session, p: &Value) -> Result<Value> {
    let n = edit_vertices(s, p, "Delete Mask Vertices", |sp, idx| {
        let mut idx = idx.to_vec();
        idx.sort_unstable();
        idx.dedup();
        for &i in idx.iter().rev() {
            if i < sp.vertices.len() {
                sp.vertices.remove(i);
                sp.in_tangents.remove(i);
                sp.out_tangents.remove(i);
            }
        }
        if sp.vertices.len() < 3 {
            sp.closed = false;
        }
    })?;
    s.state.selected_vertices.clear();
    Ok(n)
}

fn remove_masks(s: &mut Session, p: &Value, all: bool) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "mask.remove")?;
    let key = p.get("mask").cloned();
    if !all && key.is_none() {
        return Err(bad("mask.remove", "missing `mask`"));
    }
    s.edit(if all { "Remove All Masks" } else { "Remove Mask" }, None, |proj, st| {
        let l: &mut Layer = layer_mut(proj, cid, lid)?;
        let masks = l.props.sub_mut("masks").ok_or_else(|| bad("mask.remove", "this layer has no masks"))?;
        if all {
            masks.children.clear();
        } else if let Some(k) = &key {
            let uid = find_mask(masks, k).map(|g| g.uid).ok_or_else(|| bad("mask.remove", "no such mask"))?;
            masks.children.retain(|c| c.uid() != uid);
        }
        st.selected_vertices.retain(|v| v.layer != lid);
        Ok(())
    })?;
    Ok(Value::Null)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("mask.new", "New Mask from Points", [], None, "{layer?, vertices: [[x,y]…], inTangents?, outTangents?, closed?, mode?}", has_layers, new_mask),
        cmd!("mask.addVertex", "Add Mask Vertex", [], None, "{layer?, mask, point: [x,y], in?, out?, index?}", has_layers, add_vertex),
        cmd!("mask.setVertex", "Set Mask Vertex", [], None, "{layer?, mask, index, point?, in?, out?, merge?}", has_layers, set_vertex),
        cmd!("mask.setClosed", "Closed", ["Layer", "Mask and Shape Path"], None, "{layer?, mask, closed?}", has_layers, set_closed),
        cmd!("mask.selectVertices", "Select Mask Vertices", [], None, "{vertices: [{layer, mask, index}], add?, toggle?}", has_comp, select_vertices),
        cmd!("mask.moveVertices", "Move Mask Vertices", [], None, "{vertices?: [{layer, mask, index}], delta: [dx,dy], merge?}", has_vertices, move_vertices),
        cmd!("mask.deleteVertices", "Delete Mask Vertices", [], None, "{vertices?}", has_vertices, delete_vertices),
        cmd!("mask.remove", "Remove Mask", [], None, "{layer?, mask}", has_layers, |s, p| remove_masks(s, p, false)),
        cmd!("mask.removeAll", "Remove All Masks", [], None, "{layer?}", has_layers, |s, p| remove_masks(s, p, true)),
    ]
}
