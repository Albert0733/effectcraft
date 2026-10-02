//! Layer menu.

use effectcraft_color::{BlendMode, Label};
use effectcraft_keyframe::{Justify, ShapePath, TextDoc, Value as KV};
use effectcraft_project::build::{self, Ids};
use effectcraft_project::{
    Comp, FrameBlend, GroupKind, ItemId, ItemKind, Layer, LayerId, LayerSource, LightKind, MaskMode, MatteKind, Project, PropGroup, Quality, Solid, TrackMatte,
};
use effectcraft_time::Tick;
use serde_json::{Value, json};

use super::{CommandSpec, b_p, bad, comp_id, f_p, has_comp, has_layers, layer_mut, layer_p, layers_p, merge_p, resolve_layer, str_p};
use crate::{EngineError, Result, Session, cmd};

fn color_p(p: &Value, k: &str) -> Option<[f32; 3]> {
    match p.get(k)? {
        Value::String(s) => effectcraft_color::Rgba::from_hex(s).map(|c| [c.r, c.g, c.b]),
        Value::Array(a) => {
            let g = |i: usize| a.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            Some([g(0), g(1), g(2)])
        }
        _ => None,
    }
}

/// Insert a new layer above the selection (or at the top) and select it.
fn insert_layer(proj: &mut Project, st: &mut crate::EditorState, cid: ItemId, layer: Layer) -> Result<LayerId> {
    let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
    let at = st.selected_layers.first().and_then(|id| comp.layers.iter().position(|l| l.id == *id)).unwrap_or(0);
    let id = layer.id;
    comp.layers.insert(at, layer);
    st.selected_layers = vec![id];
    st.selected_props.clear();
    st.selected_keys.clear();
    Ok(id)
}

fn solids_folder(proj: &mut Project) -> ItemId {
    proj.folder_named("Solids").unwrap_or_else(|| proj.add_item("Solids", Label::Yellow, None, ItemKind::Folder))
}

fn new_solid_like(s: &mut Session, p: &Value, adjustment: bool) -> Result<Value> {
    let cid = comp_id(s, p)?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let color = color_p(p, "color").unwrap_or(if adjustment { [1.0, 1.0, 1.0] } else { [0.85, 0.2, 0.2] });
    let w = p.get("width").and_then(Value::as_u64).map(|v| v as u32).unwrap_or(comp.width);
    let h = p.get("height").and_then(Value::as_u64).map(|v| v as u32).unwrap_or(comp.height);
    let default_name = if adjustment { "Adjustment Layer 1".to_string() } else { "Solid 1".to_string() };
    let name = str_p(p, "name").map(str::to_string).unwrap_or(default_name);
    let id = s.edit(if adjustment { "New Adjustment Layer" } else { "New Solid" }, None, |proj, st| {
        let folder = solids_folder(proj);
        let sid = proj.add_item(&name, Label::Red, Some(folder), ItemKind::Solid(Solid { color, width: w, height: h, pixel_aspect: 1.0 }));
        let mut l = build::layer(proj, &comp, &name, LayerSource::Solid { item: sid }, (w, h), None);
        if adjustment {
            l.switches.adjustment = true;
            l.label = Label::Purple;
        }
        insert_layer(proj, st, cid, l)
    })?;
    Ok(json!({"layer": id.0}))
}

fn new_solid(s: &mut Session, p: &Value) -> Result<Value> {
    new_solid_like(s, p, false)
}
fn new_adjustment(s: &mut Session, p: &Value) -> Result<Value> {
    new_solid_like(s, p, true)
}

fn text_doc_from(p: &Value, base: TextDoc) -> TextDoc {
    let mut d = base;
    if let Some(t) = str_p(p, "text") {
        d.text = t.to_string();
    }
    if let Some(v) = f_p(p, "size") {
        d.size = v.max(1.0);
    }
    if let Some(v) = str_p(p, "font") {
        d.font = v.to_string();
    }
    if let Some(v) = str_p(p, "style") {
        d.style = v.to_string();
    }
    if let Some(c) = color_p(p, "fill") {
        d.fill = [c[0], c[1], c[2], 1.0];
    }
    if let Some(c) = color_p(p, "stroke") {
        d.stroke = [c[0], c[1], c[2], 1.0];
        d.apply_stroke = true;
    }
    if let Some(v) = f_p(p, "strokeWidth") {
        d.stroke_width = v.max(0.0);
        d.apply_stroke = v > 0.0;
    }
    if let Some(v) = f_p(p, "tracking") {
        d.tracking = v;
    }
    match p.get("leading") {
        Some(Value::Number(n)) => d.leading = n.as_f64(),
        // "auto" or null: Auto Leading (120% of the font size).
        Some(Value::String(_) | Value::Null) => d.leading = None,
        _ => {}
    }
    if let Some(v) = b_p(p, "applyFill") {
        d.apply_fill = v;
    }
    if let Some(v) = b_p(p, "applyStroke") {
        d.apply_stroke = v;
    }
    if let Some(v) = b_p(p, "allCaps") {
        d.all_caps = v;
    }
    if let Some(v) = b_p(p, "smallCaps") {
        d.small_caps = v;
    }
    if let Some(v) = f_p(p, "hScale") {
        d.h_scale = v.clamp(1.0, 1000.0);
    }
    if let Some(v) = f_p(p, "vScale") {
        d.v_scale = v.clamp(1.0, 1000.0);
    }
    if let Some(v) = f_p(p, "baselineShift") {
        d.baseline_shift = v;
    }
    if let Some(v) = b_p(p, "strokeOverFill") {
        d.stroke_over_fill = v;
    }
    if let Some(v) = b_p(p, "fauxBold") {
        d.faux_bold = v;
    }
    if let Some(v) = b_p(p, "fauxItalic") {
        d.faux_italic = v;
    }
    if let Some(j) = str_p(p, "justify") {
        // AE's seven Paragraph alignment buttons.
        d.justify = match j.to_ascii_lowercase().as_str() {
            "center" | "centre" => Justify::Center,
            "right" => Justify::Right,
            "justify" | "justifyleft" | "justifylastleft" => Justify::JustifyLastLeft,
            "justifycenter" | "justifylastcenter" => Justify::JustifyLastCenter,
            "justifyright" | "justifylastright" => Justify::JustifyLastRight,
            "justifyall" => Justify::JustifyAll,
            _ => Justify::Left,
        };
    }
    d
}

fn new_text(s: &mut Session, p: &Value) -> Result<Value> {
    let cid = comp_id(s, p)?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let doc = text_doc_from(p, TextDoc { text: "Text".into(), justify: Justify::Center, ..Default::default() });
    let pos = p.get("position").and_then(|v| v.as_array()).map(|a| [a[0].as_f64().unwrap_or(0.0), a.get(1).and_then(Value::as_f64).unwrap_or(0.0)]);
    let id = s.edit("New Text Layer", None, |proj, st| {
        let name: String = doc.text.lines().next().unwrap_or("Text").chars().take(40).collect();
        let mut l = build::layer(proj, &comp, if name.is_empty() { "Text" } else { &name }, LayerSource::Text, (comp.width, comp.height), None);
        if let Some(pr) = l.props.prop_mut("text/sourceText") {
            pr.value = KV::Text(Box::new(doc.clone()));
        }
        if let Some(pos) = pos
            && let Some(pr) = l.props.prop_mut("transform/position")
        {
            pr.value = KV::Vec3([pos[0], pos[1], 0.0]);
        }
        insert_layer(proj, st, cid, l)
    })?;
    Ok(json!({"layer": id.0}))
}

/// Contents for a new shape (one group with path + fill + stroke).
fn shape_contents(ids: &mut Ids, kind: &str, size: [f64; 2], fill: Option<[f64; 4]>, stroke: Option<([f64; 4], f64)>) -> Option<PropGroup> {
    let (path, gname) = match kind {
        "rect" | "rectangle" => (build::shape_rect(ids, size, [0.0, 0.0], 0.0), "Rectangle 1"),
        "rounded" | "roundedRect" => (build::shape_rect(ids, size, [0.0, 0.0], size[0].min(size[1]) * 0.15), "Rectangle 1"),
        "ellipse" => (build::shape_ellipse(ids, size, [0.0, 0.0]), "Ellipse 1"),
        "star" => (build::shape_star(ids, true, 5.0, [0.0, 0.0], size[0] / 2.0, size[0] / 4.0), "Polystar 1"),
        "polygon" => (build::shape_star(ids, false, 6.0, [0.0, 0.0], size[0] / 2.0, 0.0), "Polystar 1"),
        _ => return None,
    };
    let mut items = vec![path];
    if let Some((c, w)) = stroke {
        items.push(build::shape_stroke(ids, c, w));
    }
    if let Some(c) = fill {
        items.push(build::shape_fill(ids, c));
    }
    Some(build::shape_group(ids, gname, items))
}

fn c4(c: Option<[f32; 3]>, d: [f64; 4]) -> [f64; 4] {
    c.map(|c| [c[0] as f64, c[1] as f64, c[2] as f64, 1.0]).unwrap_or(d)
}

fn new_shape(s: &mut Session, p: &Value) -> Result<Value> {
    let cid = comp_id(s, p)?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let kind = str_p(p, "kind").unwrap_or("none").to_string();
    let size = p
        .get("size")
        .and_then(Value::as_array)
        .map(|a| [a[0].as_f64().unwrap_or(200.0), a.get(1).and_then(Value::as_f64).unwrap_or(200.0)])
        .unwrap_or([300.0, 300.0]);
    let fill = Some(c4(color_p(p, "fill"), [0.25, 0.55, 1.0, 1.0]));
    let stroke = (f_p(p, "strokeWidth").unwrap_or(0.0) > 0.0).then(|| (c4(color_p(p, "stroke"), [1.0, 1.0, 1.0, 1.0]), f_p(p, "strokeWidth").unwrap_or(2.0)));
    let pos = p.get("position").and_then(|v| v.as_array()).map(|a| [a[0].as_f64().unwrap_or(0.0), a.get(1).and_then(Value::as_f64).unwrap_or(0.0)]);
    let name = str_p(p, "name").unwrap_or("Shape Layer 1").to_string();
    let id = s.edit("New Shape Layer", None, |proj, st| {
        let mut l = build::layer(proj, &comp, &name, LayerSource::Shape, (comp.width, comp.height), None);
        let mut next = proj.next_id;
        if let Some(g) = shape_contents(&mut Ids(&mut next), &kind, size, fill, stroke)
            && let Some(c) = l.props.sub_mut("contents")
        {
            c.children.push(g.into());
        }
        proj.next_id = next;
        if let Some(pos) = pos
            && let Some(pr) = l.props.prop_mut("transform/position")
        {
            pr.value = KV::Vec3([pos[0], pos[1], 0.0]);
        }
        insert_layer(proj, st, cid, l)
    })?;
    Ok(json!({"layer": id.0}))
}

fn new_simple(s: &mut Session, p: &Value, src: LayerSource, name: &str, label: &str) -> Result<Value> {
    let cid = comp_id(s, p)?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let name = str_p(p, "name").unwrap_or(name).to_string();
    let id = s.edit(label, None, |proj, st| {
        let l = build::layer(proj, &comp, &name, src, (comp.width, comp.height), None);
        insert_layer(proj, st, cid, l)
    })?;
    Ok(json!({"layer": id.0}))
}

fn new_null(s: &mut Session, p: &Value) -> Result<Value> {
    new_simple(s, p, LayerSource::Null, "Null 1", "New Null Object")
}
fn new_camera(s: &mut Session, p: &Value) -> Result<Value> {
    new_simple(s, p, LayerSource::Camera, "Camera 1", "New Camera")
}
fn new_light(s: &mut Session, p: &Value) -> Result<Value> {
    let kind = str_p(p, "kind").and_then(|k| LightKind::ALL.into_iter().find(|l| l.label().eq_ignore_ascii_case(k))).unwrap_or(LightKind::Point);
    new_simple(s, p, LayerSource::Light { kind }, "Light 1", "New Light")
}

fn add_item(s: &mut Session, p: &Value) -> Result<Value> {
    let cid = comp_id(s, p)?;
    let item = match p.get("item") {
        Some(Value::Number(n)) => ItemId(n.as_u64().unwrap_or(0)),
        Some(Value::String(name)) => s.project.find_by_name(name).map(|i| i.id).ok_or_else(|| bad("layer.addItem", format!("no item `{name}`")))?,
        _ => *s.state.project_selection.first().ok_or_else(|| bad("layer.addItem", "missing `item`"))?,
    };
    if s.project.comp_contains(item, cid) {
        return Err(bad("layer.addItem", "a composition can't contain itself"));
    }
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let it = s.project.item(item).ok_or_else(|| bad("layer.addItem", "no such item"))?.clone();
    let (src, size, dur) = match &it.kind {
        ItemKind::Comp(c) => (LayerSource::Comp { item }, (c.width, c.height), Some(c.duration)),
        ItemKind::Footage(f) => (LayerSource::Footage { item }, (f.width, f.height), (f.kind != effectcraft_project::FootageKind::Still).then_some(f.duration)),
        ItemKind::Solid(so) => (LayerSource::Solid { item }, (so.width, so.height), None),
        ItemKind::Folder => return Err(bad("layer.addItem", "folders can't be layers")),
    };
    let fr = comp.frame_rate;
    let start = fr.snap_nearest(f_p(p, "time").map(Tick::from_seconds_f64).unwrap_or(Tick::ZERO));
    let id = s.edit("Add Footage to Comp", None, |proj, st| {
        let mut l = build::layer(proj, &comp, &it.name, src, size, dur);
        l.start_time = start;
        l.in_point = start;
        l.out_point = fr.snap_nearest((start + dur.unwrap_or(comp.duration)).min(comp.duration)).max(start + fr.frame_duration());
        insert_layer(proj, st, cid, l)
    })?;
    Ok(json!({"layer": id.0}))
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let cid = comp_id(s, p)?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
    let ids: Vec<LayerId> = match p.get("layers").or(p.get("layer")) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| resolve_layer(comp, v)).collect(),
        Some(v) => resolve_layer(comp, v).into_iter().collect(),
        None => vec![],
    };
    let add = b_p(p, "add").unwrap_or(false);
    let toggle = b_p(p, "toggle").unwrap_or(false);
    if toggle {
        for id in ids {
            if let Some(i) = s.state.selected_layers.iter().position(|l| *l == id) {
                s.state.selected_layers.remove(i);
            } else {
                s.state.selected_layers.push(id);
            }
        }
    } else if add {
        for id in ids {
            if !s.state.selected_layers.contains(&id) {
                s.state.selected_layers.push(id);
            }
        }
    } else {
        s.state.selected_layers = ids;
        s.state.selected_props.clear();
        s.state.selected_keys.clear();
        let keep = s.state.selected_layers.clone();
        s.state.selected_vertices.retain(|v| keep.contains(&v.layer));
    }
    Ok(json!(s.state.selected_layers.iter().map(|l| l.0).collect::<Vec<_>>()))
}

fn select_step(s: &mut Session, p: &Value, dir: i64) -> Result<Value> {
    let comp = s.active_comp().ok_or(EngineError::NoComp)?;
    if comp.layers.is_empty() {
        return Ok(Value::Null);
    }
    let cur = s.state.selected_layers.first().and_then(|id| comp.layers.iter().position(|l| l.id == *id));
    let n = comp.layers.len() as i64;
    let i = match cur {
        Some(i) => (i as i64 + dir).clamp(0, n - 1),
        None => {
            if dir > 0 {
                0
            } else {
                n - 1
            }
        }
    } as usize;
    let id = comp.layers[i].id;
    let add = b_p(p, "add").unwrap_or(false);
    if add {
        if !s.state.selected_layers.contains(&id) {
            s.state.selected_layers.insert(0, id);
        }
    } else {
        s.state.selected_layers = vec![id];
    }
    Ok(json!(id.0))
}

fn select_next(s: &mut Session, p: &Value) -> Result<Value> {
    select_step(s, p, 1)
}
fn select_prev(s: &mut Session, p: &Value) -> Result<Value> {
    select_step(s, p, -1)
}

fn set_switch(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, ids) = layers_p(s, p)?;
    let name = str_p(p, "switch").ok_or_else(|| bad("layer.setSwitch", "missing `switch`"))?.to_string();
    let v = b_p(p, "value");
    let label = format!("Layer Switch ({name})");
    let r = s.edit(&label, None, |proj, _| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        let mut last = false;
        // Toggle relative to the first layer so a multi-selection ends up consistent.
        let first = comp.layers.iter().find(|l| ids.contains(&l.id)).map(|l| switch_value(l, &name)).unwrap_or(Some(false));
        let Some(first) = first else { return Err(bad("layer.setSwitch", format!("unknown switch `{name}`"))) };
        let target = v.unwrap_or(!first);
        for l in comp.layers.iter_mut().filter(|l| ids.contains(&l.id)) {
            set_switch_value(l, &name, target);
            last = target;
        }
        Ok(last)
    })?;
    Ok(json!(r))
}

fn switch_value(l: &Layer, name: &str) -> Option<bool> {
    let sw = &l.switches;
    Some(match name {
        "video" | "eye" => sw.video,
        "audio" => sw.audio,
        "solo" => sw.solo,
        "lock" | "locked" => sw.locked,
        "shy" => sw.shy,
        "collapse" => sw.collapse,
        "quality" => sw.quality == Quality::Best,
        "fx" | "effects" => sw.effects,
        "frameBlend" => sw.frame_blend != FrameBlend::Off,
        "motionBlur" => sw.motion_blur,
        "adjustment" => sw.adjustment,
        "threeD" | "3d" | "3D" => sw.three_d,
        "guide" => sw.guide,
        "preserveTransparency" => l.preserve_transparency,
        _ => return None,
    })
}

fn set_switch_value(l: &mut Layer, name: &str, v: bool) {
    let sw = &mut l.switches;
    match name {
        "video" | "eye" => sw.video = v,
        "audio" => sw.audio = v,
        "solo" => sw.solo = v,
        "lock" | "locked" => sw.locked = v,
        "shy" => sw.shy = v,
        "collapse" => sw.collapse = v,
        "quality" => sw.quality = if v { Quality::Best } else { Quality::Draft },
        "fx" | "effects" => sw.effects = v,
        "frameBlend" => sw.frame_blend = if v { FrameBlend::FrameMix } else { FrameBlend::Off },
        "motionBlur" => sw.motion_blur = v,
        "adjustment" => sw.adjustment = v,
        "threeD" | "3d" | "3D" => sw.three_d = v,
        "guide" => sw.guide = v,
        "preserveTransparency" => l.preserve_transparency = v,
        _ => {}
    }
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "layer.rename")?;
    let name = str_p(p, "name").ok_or_else(|| bad("layer.rename", "missing `name`"))?.to_string();
    s.edit("Rename Layer", None, |proj, _| {
        layer_mut(proj, cid, lid)?.name = name.clone();
        Ok(())
    })?;
    Ok(Value::Null)
}

fn blend(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, ids) = layers_p(s, p)?;
    let mode = str_p(p, "mode").and_then(BlendMode::from_name);
    let step = p.get("step").and_then(Value::as_i64).unwrap_or(0);
    if mode.is_none() && step == 0 {
        return Err(bad("layer.setBlendMode", "need `mode` (e.g. Multiply) or `step` ±1"));
    }
    s.edit("Blending Mode", None, |proj, _| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        for l in comp.layers.iter_mut().filter(|l| ids.contains(&l.id)) {
            l.blend_mode = match mode {
                Some(m) => m,
                None if step > 0 => l.blend_mode.next(),
                None => l.blend_mode.previous(),
            };
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn track_matte(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "layer.setTrackMatte")?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
    let matte = match p.get("matte") {
        None | Some(Value::Null) => None,
        Some(v) => Some(resolve_layer(comp, v).ok_or_else(|| bad("layer.setTrackMatte", "no such matte layer"))?),
    };
    let kind = str_p(p, "kind")
        .map(|k| match k.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
            "alphainverted" | "alphainv" => MatteKind::AlphaInverted,
            "luma" => MatteKind::Luma,
            "lumainverted" | "lumainv" => MatteKind::LumaInverted,
            _ => MatteKind::Alpha,
        })
        .unwrap_or(MatteKind::Alpha);
    s.edit("Track Matte", None, |proj, _| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        if let Some(m) = matte {
            if m == lid {
                return Err(bad("layer.setTrackMatte", "a layer can't be its own matte"));
            }
            if let Some(ml) = comp.layer_mut(m) {
                ml.switches.video = false;
            }
        }
        comp.layer_mut(lid).ok_or(EngineError::NoComp)?.track_matte = matte.map(|layer| TrackMatte { layer, kind });
        Ok(())
    })?;
    Ok(Value::Null)
}

fn set_parent(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, ids) = layers_p(s, p)?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
    let parent = match p.get("parent") {
        None | Some(Value::Null) => None,
        Some(v) => Some(resolve_layer(comp, v).ok_or_else(|| bad("layer.setParent", "no such parent layer"))?),
    };
    // Cycle check.
    if let Some(par) = parent {
        let mut cur = Some(par);
        while let Some(c) = cur {
            if ids.contains(&c) {
                return Err(bad("layer.setParent", "parenting would create a cycle"));
            }
            cur = comp.layer(c).and_then(|l| l.parent);
        }
    }
    // Like AE's pick-whip, parenting keeps the layer where it is: its transform is re-expressed
    // in the new parent's space (Position, Rotation, Scale), unless `compensate: false`.
    let compensate = b_p(p, "compensate").unwrap_or(true);
    let ectx = effectcraft_render::EvalCtx { project: &s.project, comp_id: cid, comp, time: s.time(), expr: s.expr.as_deref() };
    let space = |id: Option<LayerId>| -> effectcraft_geom::Mat3 {
        id.and_then(|i| comp.layer(i)).filter(|l| !l.is_3d()).map(|l| ectx.layer_to_comp(l).0).unwrap_or(effectcraft_geom::Mat3::IDENTITY)
    };
    let decompose = |m: &effectcraft_geom::Mat3| {
        let a = m.0;
        let sx = (a[0][0] * a[0][0] + a[1][0] * a[1][0]).sqrt();
        let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
        (a[1][0].atan2(a[0][0]).to_degrees(), sx, if sx > 1e-12 { det / sx } else { 0.0 })
    };
    // Per layer: (id, position transform, rotation delta, scale factors).
    let mut fixes: Vec<(LayerId, effectcraft_geom::Mat3, f64, [f64; 2])> = vec![];
    if compensate {
        let new_m = space(parent);
        for l in comp.layers.iter().filter(|l| ids.contains(&l.id) && !l.is_3d() && l.parent != parent) {
            let old_m = space(l.parent);
            let Some(inv) = new_m.inverse() else { continue };
            let (ro, sxo, syo) = decompose(&old_m);
            let (rn, sxn, syn) = decompose(&new_m);
            let k = [if sxn.abs() > 1e-12 { sxo / sxn } else { 1.0 }, if syn.abs() > 1e-12 { syo / syn } else { 1.0 }];
            fixes.push((l.id, inv * old_m, ro - rn, k));
        }
    }
    s.edit("Parent", None, |proj, _| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        for l in comp.layers.iter_mut().filter(|l| ids.contains(&l.id)) {
            l.parent = parent;
            let Some((_, m, dr, k)) = fixes.iter().find(|f| f.0 == l.id) else { continue };
            let Some(tr) = l.props.sub_mut("transform") else { continue };
            let map = |pr: &mut effectcraft_project::Property, f: &dyn Fn(&KV) -> KV| {
                pr.value = f(&pr.value);
                for key in &mut pr.keys {
                    key.value = f(&key.value);
                }
            };
            if tr.get("positionX").is_none()
                && let Some(pr) = tr.get_mut("position")
            {
                map(pr, &|v| {
                    let c = v.as_vec3();
                    let q = m.apply(effectcraft_geom::vec2(c[0], c[1]));
                    KV::Vec3([q.x, q.y, c[2]])
                });
            }
            if let Some(pr) = tr.get_mut("rotation") {
                map(pr, &|v| KV::Scalar(v.as_f64() + dr));
            }
            if let Some(pr) = tr.get_mut("scale") {
                map(pr, &|v| {
                    let c = v.as_vec3();
                    KV::Vec3([c[0] * k[0], c[1] * k[1], c[2]])
                });
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn timing(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, ids) = layers_p(s, p)?;
    let t = s.time();
    let op = str_p(p, "op").unwrap_or("set").to_string();
    // Layer times stay on frame boundaries of the comp (as in AE).
    let fr = s.project.comp(cid).ok_or(EngineError::NoComp)?.frame_rate;
    let snap = |k: &str| f_p(p, k).map(|v| fr.snap_nearest(Tick::from_seconds_f64(v)));
    let (delta, start, inp, outp) = (snap("delta"), snap("start"), snap("in"), snap("out"));
    s.edit("Layer Timing", merge_p(p), |proj, _| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        let fd = comp.frame_duration();
        for l in comp.layers.iter_mut().filter(|l| ids.contains(&l.id)) {
            let shift = |l: &mut Layer, d: Tick| {
                l.start_time += d;
                l.in_point += d;
                l.out_point += d;
            };
            match op.as_str() {
                "moveInToTime" => {
                    let d = t - l.in_point;
                    shift(l, d);
                }
                "moveOutToTime" => {
                    let d = t - l.out_point;
                    shift(l, d);
                }
                "trimInToTime" => l.in_point = t.min(l.out_point - fd),
                "trimOutToTime" => l.out_point = t.max(l.in_point + fd),
                _ => {
                    if let Some(d) = delta {
                        shift(l, d);
                    }
                    if let Some(st) = start {
                        let d = st - l.start_time;
                        shift(l, d);
                    }
                    if let Some(i) = inp {
                        l.in_point = i.min(l.out_point - fd);
                    }
                    if let Some(o) = outp {
                        l.out_point = o.max(l.in_point + fd);
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn arrange(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, ids) = layers_p(s, p)?;
    let how = str_p(p, "to").unwrap_or("front").to_string();
    let index = p.get("index").and_then(Value::as_u64).map(|i| i as usize);
    s.edit("Arrange Layers", None, |proj, _| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        let n = comp.layers.len();
        let mut picked: Vec<Layer> = Vec::new();
        let first_idx = comp.layers.iter().position(|l| ids.contains(&l.id)).unwrap_or(0);
        let last_idx = comp.layers.iter().rposition(|l| ids.contains(&l.id)).unwrap_or(0);
        comp.layers.retain(|l| {
            if ids.contains(&l.id) {
                picked.push(l.clone());
                false
            } else {
                true
            }
        });
        let at = match (how.as_str(), index) {
            (_, Some(i)) => i.saturating_sub(1).min(comp.layers.len()),
            ("front", _) => 0,
            ("back", _) => comp.layers.len(),
            ("forward", _) => first_idx.saturating_sub(1),
            ("backward", _) => (last_idx + 2 - picked.len()).min(comp.layers.len()),
            _ => 0,
        };
        let _ = n;
        for (k, l) in picked.into_iter().enumerate() {
            comp.layers.insert((at + k).min(comp.layers.len()), l);
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn precompose(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, ids) = layers_p(s, p)?;
    if ids.is_empty() {
        return Err(bad("layer.precompose", "no layers"));
    }
    let name = str_p(p, "name").unwrap_or("Pre-comp 1").to_string();
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let new = s.edit("Pre-compose", None, |proj, st| {
        let mut inner = Comp::new(comp.width, comp.height, comp.frame_rate, comp.duration);
        inner.background = comp.background;
        inner.layers = comp.layers.iter().filter(|l| ids.contains(&l.id)).cloned().collect();
        for l in &mut inner.layers {
            if l.parent.is_some_and(|p| !ids.contains(&p)) {
                l.parent = None;
            }
            if l.track_matte.is_some_and(|m| !ids.contains(&m.layer)) {
                l.track_matte = None;
            }
        }
        let iid = proj.add_item(&name, Label::Sandstone, None, ItemKind::Comp(inner.into()));
        let mut l = build::layer(proj, &comp, &name, LayerSource::Comp { item: iid }, (comp.width, comp.height), Some(comp.duration));
        l.name = name.clone();
        let c = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        let at = c.layers.iter().position(|l| ids.contains(&l.id)).unwrap_or(0);
        c.layers.retain(|l| !ids.contains(&l.id));
        let lid = l.id;
        c.layers.insert(at.min(c.layers.len()), l);
        st.selected_layers = vec![lid];
        Ok((iid, lid))
    })?;
    Ok(json!({"comp": new.0.0, "layer": new.1.0}))
}

fn add_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "layer.addMask")?;
    let layer = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?.clone();
    let (w, h) = effectcraft_render::source_size(&s.project, &layer);
    let (w, h) = if w == 0 { (400.0, 300.0) } else { (w as f64, h as f64) };
    let shape = str_p(p, "shape").unwrap_or("rect");
    let r = p.get("rect").and_then(Value::as_array).map(|a| [0, 1, 2, 3].map(|i| a.get(i).and_then(Value::as_f64).unwrap_or(0.0)));
    let (cx, cy, rw, rh) = match r {
        Some([x, y, rw, rh]) => (x + rw / 2.0, y + rh / 2.0, rw, rh),
        None => (if w > 0.0 { w / 2.0 } else { 0.0 }, h / 2.0, w * 0.6, h * 0.6),
    };
    let path = match shape {
        "ellipse" => ShapePath::ellipse([cx, cy], rw, rh),
        _ => ShapePath::rect([cx, cy], rw, rh),
    };
    let mode = str_p(p, "mode").and_then(MaskMode::from_name).unwrap_or(MaskMode::Add);
    let uid = s.edit("New Mask", None, |proj, _| {
        let mut next = proj.next_id;
        let l = layer_mut(proj, cid, lid)?;
        let masks = l.props.sub_mut("masks").ok_or_else(|| bad("layer.addMask", "this layer can't have masks"))?;
        let n = masks.children.len();
        let g = build::mask(&mut Ids(&mut next), &format!("Mask {}", n + 1), path, mode, build::MASK_COLORS[n % build::MASK_COLORS.len()]);
        let uid = g.uid;
        masks.children.push(g.into());
        proj.next_id = next;
        Ok(uid)
    })?;
    Ok(json!({"mask": uid}))
}

fn mask_props(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "layer.setMask")?;
    let target = p.get("mask").cloned().unwrap_or(json!(1));
    let mode = str_p(p, "mode").and_then(MaskMode::from_name);
    let inverted = b_p(p, "inverted");
    s.edit("Mask Settings", None, |proj, _| {
        let l = layer_mut(proj, cid, lid)?;
        let masks = l.props.sub_mut("masks").ok_or_else(|| bad("layer.setMask", "no masks"))?;
        let g = match &target {
            Value::Number(n) => {
                let n = n.as_u64().unwrap_or(1);
                if let Some(i) = masks.children.iter().position(|c| c.uid() == n) {
                    masks.children[i].as_group_mut()
                } else {
                    masks.children.get_mut(n.saturating_sub(1) as usize).and_then(|c| c.as_group_mut())
                }
            }
            Value::String(name) => masks.children.iter_mut().find(|c| c.name() == name).and_then(|c| c.as_group_mut()),
            _ => None,
        }
        .ok_or_else(|| bad("layer.setMask", "no such mask"))?;
        if let GroupKind::Mask { mode: m, inverted: inv, .. } = &mut g.kind {
            if let Some(mm) = mode {
                *m = mm;
            }
            if let Some(i) = inverted {
                *inv = i;
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn add_shape_item(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "layer.addShapeItem")?;
    let kind = str_p(p, "kind").ok_or_else(|| bad("layer.addShapeItem", "missing `kind`"))?.to_string();
    let group_uid = p.get("group").and_then(Value::as_u64);
    let uid = s.edit("Add Shape Item", None, |proj, _| {
        let mut next = proj.next_id;
        let mut ids = Ids(&mut next);
        let g = match kind.as_str() {
            "group" => build::shape_group(&mut ids, "Group 1", vec![]),
            "rect" | "rectangle" => build::shape_rect(&mut ids, [200.0, 200.0], [0.0, 0.0], 0.0),
            "ellipse" => build::shape_ellipse(&mut ids, [200.0, 200.0], [0.0, 0.0]),
            "star" => build::shape_star(&mut ids, true, 5.0, [0.0, 0.0], 100.0, 50.0),
            "polygon" => build::shape_star(&mut ids, false, 5.0, [0.0, 0.0], 100.0, 0.0),
            "fill" => build::shape_fill(&mut ids, [1.0, 0.0, 0.0, 1.0]),
            "stroke" => build::shape_stroke(&mut ids, [1.0, 1.0, 1.0, 1.0], 2.0),
            "gfill" | "gradientFill" => build::shape_gradient_fill(&mut ids, false, [-100.0, 0.0], [100.0, 0.0], Default::default()),
            "trim" | "trimPaths" => build::shape_trim(&mut ids, 0.0, 100.0, 0.0),
            "repeater" => build::shape_repeater(&mut ids, 3.0, [100.0, 0.0]),
            k => build::shape_simple_op(&mut ids, k).ok_or_else(|| bad("layer.addShapeItem", format!("unknown kind `{k}`")))?,
        };
        let uid = g.uid;
        proj.next_id = next;
        let l = layer_mut(proj, cid, lid)?;
        let contents = l.props.sub_mut("contents").ok_or_else(|| bad("layer.addShapeItem", "not a shape layer"))?;
        let target = match group_uid {
            Some(u) => contents
                .find_group_mut(u)
                .and_then(|g| if g.match_id == "contents" { Some(g) } else { g.sub_mut("contents") })
                .ok_or_else(|| bad("layer.addShapeItem", "no such group"))?,
            None => contents,
        };
        target.children.push(g.into());
        Ok(uid)
    })?;
    Ok(json!({"uid": uid}))
}

fn set_text(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "layer.setText")?;
    let t = s.time();
    s.edit("Edit Text", merge_p(p), |proj, _| {
        let l = layer_mut(proj, cid, lid)?;
        let lt = l.layer_time(t);
        let pr = l.props.prop_mut("text/sourceText").ok_or_else(|| bad("layer.setText", "not a text layer"))?;
        let cur = match pr.value_at(lt) {
            KV::Text(d) => *d,
            _ => TextDoc::default(),
        };
        let doc = text_doc_from(p, cur);
        pr.set_value_at(lt, KV::Text(Box::new(doc.clone())));
        if let Some(first) = doc.text.lines().next()
            && str_p(p, "text").is_some()
            && !first.is_empty()
            && l.name.starts_with("Text")
        {
            l.name = first.chars().take(40).collect();
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn add_animator(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "layer.addTextAnimator")?;
    let props: Vec<String> = match p.get("properties").or(p.get("property")) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        Some(Value::String(x)) => vec![x.clone()],
        _ => vec!["opacity".into()],
    };
    let uid = s.edit("Add Text Animator", None, |proj, _| {
        let mut next = proj.next_id;
        let mut ids = Ids(&mut next);
        let list: Vec<_> = props.iter().filter_map(|k| build::text_anim_prop(&mut ids, k)).collect();
        if list.is_empty() {
            return Err(bad("layer.addTextAnimator", "unknown animator property"));
        }
        let l = layer_mut(proj, cid, lid)?;
        let anims = l.props.group_mut("text/animators").ok_or_else(|| bad("layer.addTextAnimator", "not a text layer"))?;
        let name = format!("Animator {}", anims.children.len() + 1);
        let g = build::text_animator(&mut ids, &name, list);
        let uid = g.uid;
        anims.children.push(g.into());
        proj.next_id = next;
        Ok(uid)
    })?;
    Ok(json!({"animator": uid}))
}

fn transform_op(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, ids) = layers_p(s, p)?;
    let op = str_p(p, "op").unwrap_or("reset").to_string();
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let t = s.time();
    let sizes: Vec<(LayerId, (u32, u32))> =
        comp.layers.iter().filter(|l| ids.contains(&l.id)).map(|l| (l.id, effectcraft_render::source_size(&s.project, l))).collect();
    s.edit("Transform", None, |proj, _| {
        for (lid, (w, h)) in &sizes {
            let l = layer_mut(proj, cid, *lid)?;
            let lt = l.layer_time(t);
            let (cw, ch) = (comp.width as f64, comp.height as f64);
            let (w, h) = (*w as f64, *h as f64);
            let Some(tr) = l.transform_mut() else { continue };
            let set = |tr: &mut PropGroup, m: &str, v: KV| {
                if let Some(pr) = tr.get_mut(m) {
                    pr.set_value_at(lt, v);
                }
            };
            match op.as_str() {
                "reset" => {
                    set(tr, "anchor", KV::Vec3([w / 2.0, h / 2.0, 0.0]));
                    set(tr, "position", KV::Vec3([cw / 2.0, ch / 2.0, 0.0]));
                    set(tr, "scale", KV::Vec3([100.0; 3]));
                    set(tr, "rotation", KV::Scalar(0.0));
                    set(tr, "rotationX", KV::Scalar(0.0));
                    set(tr, "rotationY", KV::Scalar(0.0));
                    set(tr, "orientation", KV::Vec3([0.0; 3]));
                    set(tr, "opacity", KV::Scalar(100.0));
                }
                "center" => set(tr, "position", KV::Vec3([cw / 2.0, ch / 2.0, 0.0])),
                "fit" | "fitWidth" | "fitHeight" if w > 0.0 && h > 0.0 => {
                    let (sx, sy) = match op.as_str() {
                        "fitWidth" => (cw / w, cw / w),
                        "fitHeight" => (ch / h, ch / h),
                        _ => (cw / w, ch / h),
                    };
                    set(tr, "scale", KV::Vec3([sx * 100.0, sy * 100.0, 100.0]));
                    set(tr, "position", KV::Vec3([cw / 2.0, ch / 2.0, 0.0]));
                    set(tr, "anchor", KV::Vec3([w / 2.0, h / 2.0, 0.0]));
                }
                "flipH" | "flipV" => {
                    let cur = tr.get("scale").map(|p| p.value_at(lt).as_vec3()).unwrap_or([100.0; 3]);
                    let v = if op == "flipH" { [-cur[0], cur[1], cur[2]] } else { [cur[0], -cur[1], cur[2]] };
                    set(tr, "scale", KV::Vec3(v));
                }
                _ => {}
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn layer_settings(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid) = layer_p(s, p, "layer.settings")?;
    let color = color_p(p, "color");
    let w = p.get("width").and_then(Value::as_u64).map(|v| v as u32);
    let h = p.get("height").and_then(Value::as_u64).map(|v| v as u32);
    let name = str_p(p, "name").map(str::to_string);
    s.edit("Layer Settings", None, |proj, _| {
        let l = layer_mut(proj, cid, lid)?;
        if let Some(n) = &name {
            l.name = n.clone();
        }
        let src = l.source.clone();
        if let LayerSource::Solid { item } = src
            && let Some(it) = proj.item_mut(item)
            && let ItemKind::Solid(so) = &mut it.kind
        {
            if let Some(c) = color {
                so.color = c;
            }
            if let Some(w) = w {
                so.width = w.max(1);
            }
            if let Some(h) = h {
                so.height = h.max(1);
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("layer.newText", "Text", ["Layer", "New"], Some("Cmd+Alt+Shift+T"), "{text?, size?, font?, fill?, position? [x,y], justify?}", has_comp, new_text),
        cmd!("layer.newSolid", "Solid…", ["Layer", "New"], Some("Cmd+Y"), "{name?, color? #hex|[r,g,b], width?, height?}", has_comp, new_solid),
        cmd!("layer.newLight", "Light…", ["Layer", "New"], Some("Cmd+Alt+Shift+L"), "{kind?: Parallel|Spot|Point|Ambient}", has_comp, new_light),
        cmd!("layer.newCamera", "Camera…", ["Layer", "New"], Some("Cmd+Alt+Shift+C"), "{name?}", has_comp, new_camera),
        cmd!("layer.newNull", "Null Object", ["Layer", "New"], Some("Cmd+Alt+Shift+Y"), "{name?}", has_comp, new_null),
        cmd!(
            "layer.newShape",
            "Shape Layer",
            ["Layer", "New"],
            None,
            "{kind?: rect|rounded|ellipse|star|polygon|none, size?, fill?, stroke?, strokeWidth?, position?}",
            has_comp,
            new_shape
        ),
        cmd!("layer.newAdjustment", "Adjustment Layer", ["Layer", "New"], Some("Cmd+Alt+Y"), "{name?}", has_comp, new_adjustment),
        cmd!("layer.settings", "Layer Settings…", ["Layer"], Some("Cmd+Shift+Y"), "{layer?, name?, color?, width?, height?}", has_layers, layer_settings),
        cmd!("layer.addItem", "Add Footage to Comp", ["File"], Some("Cmd+/"), "{item: id|name, time?}", has_comp, add_item),
        cmd!("layer.select", "Select Layers", [], None, "{layers: [id|name|#n], add?, toggle?}", has_comp, select),
        cmd!("layer.selectNext", "Select Next Layer", [], Some("Cmd+ArrowDown"), "{add?}", has_comp, select_next),
        cmd!("layer.selectPrevious", "Select Previous Layer", [], Some("Cmd+ArrowUp"), "{add?}", has_comp, select_prev),
        cmd!(
            "layer.setSwitch",
            "Layer Switch",
            [],
            None,
            "{layers?, switch: video|audio|solo|lock|shy|collapse|quality|fx|frameBlend|motionBlur|adjustment|threeD|guide|preserveTransparency, value?}",
            has_layers,
            set_switch
        ),
        cmd!("layer.rename", "Rename", [], Some("Enter"), "{layer?, name}", has_layers, rename),
        cmd!("layer.setBlendMode", "Blending Mode", [], None, "{layers?, mode?: Normal|Multiply|Screen|…, step?: ±1}", has_layers, blend),
        cmd!(
            "layer.setTrackMatte",
            "Track Matte",
            [],
            None,
            "{layer?, matte: layer|null, kind?: alpha|alphaInverted|luma|lumaInverted}",
            has_layers,
            track_matte
        ),
        cmd!("layer.setParent", "Parent", [], None, "{layers?, parent: layer|null}", has_layers, set_parent),
        cmd!(
            "layer.timing",
            "Layer Timing",
            [],
            None,
            "{layers?, op?: moveInToTime|moveOutToTime|trimInToTime|trimOutToTime, delta?, start?, in?, out?, merge?}",
            has_layers,
            timing
        ),
        cmd!("layer.arrange", "Arrange", ["Layer", "Arrange"], None, "{layers?, to: front|forward|backward|back, index?}", has_layers, arrange),
        cmd!("layer.precompose", "Pre-compose…", ["Layer"], Some("Cmd+Shift+C"), "{layers?, name?}", has_layers, precompose),
        cmd!(
            "layer.addMask",
            "New Mask",
            ["Layer", "Mask"],
            Some("Cmd+Shift+N"),
            "{layer?, shape?: rect|ellipse, rect? [x,y,w,h], mode?}",
            has_layers,
            add_mask
        ),
        cmd!("layer.setMask", "Mask Mode", [], None, "{layer?, mask: index|uid|name, mode?, inverted?}", has_layers, mask_props),
        cmd!(
            "layer.addShapeItem",
            "Add (Shape)",
            [],
            None,
            "{layer?, kind: group|rect|ellipse|star|polygon|fill|stroke|gfill|trim|repeater|round|offset|pucker|twist|zigzag|wiggle|merge, group?: uid}",
            has_layers,
            add_shape_item
        ),
        cmd!(
            "layer.setText",
            "Edit Text",
            [],
            None,
            "{layer?, text?, size?, font?, style?, fill?, stroke?, applyFill?, applyStroke?, strokeWidth?, tracking?, leading?: px|\"auto\", justify?: left|center|right|justifyLeft|justifyCenter|justifyRight|justifyAll, allCaps?, smallCaps?, fauxBold?, fauxItalic?, hScale? %, vScale? %, baselineShift? px, strokeOverFill?}",
            has_layers,
            set_text
        ),
        cmd!(
            "layer.addTextAnimator",
            "Animate Text",
            ["Animation"],
            None,
            "{layer?, properties: [position|scale|rotation|opacity|fillColor|tracking|…]}",
            has_layers,
            add_animator
        ),
        cmd!(
            "layer.transform",
            "Transform",
            ["Layer", "Transform"],
            None,
            "{layers?, op: reset|center|fit|fitWidth|fitHeight|flipH|flipV}",
            has_layers,
            transform_op
        ),
    ]
}

#[allow(dead_code)]
fn unused(_: &Comp) {}
