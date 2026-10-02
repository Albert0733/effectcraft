//! Paint and Puppet commands: undoable edits, options, rendering through the session.

use effectcraft_render::RenderOpts;
use effectcraft_time::Tick;
use serde_json::{Value, json};

use crate::Session;

/// 200×100 comp, 2 s at 30 fps, with a 100×60 blue solid centred (comp 50..150 × 20..80).
fn setup() -> (Session, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "C", "width": 200, "height": 100, "duration": 2, "frameRate": 30})).unwrap();
    let id = s.execute("layer.newSolid", json!({"width": 100, "height": 60, "color": [0, 0, 1]})).unwrap()["layer"].as_u64().unwrap();
    (s, id)
}

fn px(s: &Session, t: f64, x: i64, y: i64) -> [f32; 4] {
    let cid = s.active_comp_id().unwrap();
    s.render(cid, Tick::from_seconds_f64(t), RenderOpts::default()).get(x, y)
}

fn layer(s: &Session, id: u64) -> effectcraft_project::Layer {
    s.active_comp().unwrap().layer(effectcraft_project::LayerId(id)).unwrap().clone()
}

#[test]
fn brush_stroke_is_an_undoable_effect() {
    let (mut s, id) = setup();
    s.execute("paint.options", json!({"color": [1, 0, 0, 1], "diameter": 8, "sizePressure": false})).unwrap();
    let r = s.execute("paint.stroke", json!({"layer": id, "points": [[10, 30], [90, 30]]})).unwrap();
    let fx = r["effect"].as_u64().unwrap();
    let l = layer(&s, id);
    let g = l.props.find_group(fx).unwrap();
    assert_eq!(g.name, "Paint");
    assert_eq!(effectcraft_effects::paint::strokes(g).count(), 1);
    assert_eq!(g.groups().next().unwrap().name, "Brush 1");
    // Layer (10..90, 30) → comp (60..140, 50).
    assert_eq!(px(&s, 0.0, 100, 50), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&s, 0.0, 100, 40), [0.0, 0.0, 1.0, 1.0]);
    // Second stroke goes into the same Paint effect.
    let r2 = s.execute("paint.stroke", json!({"layer": id, "points": [[50, 5]], "kind": "eraser", "diameter": 20})).unwrap();
    assert_eq!(r2["effect"].as_u64(), Some(fx));
    assert_eq!(px(&s, 0.0, 100, 25)[3], 0.0);
    assert!(s.undo());
    assert_eq!(px(&s, 0.0, 100, 25), [0.0, 0.0, 1.0, 1.0]);
    assert!(s.undo());
    assert!(layer(&s, id).effects().unwrap().children.is_empty());
    assert_eq!(px(&s, 0.0, 100, 50), [0.0, 0.0, 1.0, 1.0]);
    assert!(s.redo());
    assert_eq!(px(&s, 0.0, 100, 50), [1.0, 0.0, 0.0, 1.0]);
    // Another effect after Paint: the next stroke starts a new Paint instance.
    s.execute("effect.apply", json!({"layer": id, "effect": "Invert"})).unwrap();
    let r3 = s.execute("paint.stroke", json!({"layer": id, "points": [[20, 20]]})).unwrap();
    assert_ne!(r3["effect"].as_u64(), Some(fx));
    assert_eq!(layer(&s, id).props.find_group(r3["effect"].as_u64().unwrap()).unwrap().name, "Paint 2");
}

#[test]
fn durations_set_spans_and_write_on_keys() {
    let (mut s, id) = setup();
    s.set_time(Tick::from_seconds_f64(0.5));
    let r = s.execute("paint.stroke", json!({"layer": id, "points": [[10, 30], [90, 30]], "durationMode": "writeOn", "duration": 0.5})).unwrap();
    let l = layer(&s, id);
    let g = l.props.find_group(r["stroke"].as_u64().unwrap()).unwrap();
    let end = g.sub("stroke_options").unwrap().get("end").unwrap();
    assert_eq!(end.keys.len(), 2);
    assert_eq!(end.keys[0].time, Tick::from_seconds_f64(0.5));
    assert_eq!(end.keys[1].time, Tick::from_seconds_f64(1.0));
    // Written on over 0.5–1.0 s: invisible before, half at 0.75 s.
    assert_eq!(px(&s, 0.25, 100, 50), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(px(&s, 0.75, 70, 50)[1], 1.0);
    assert_eq!(px(&s, 0.75, 130, 50), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(px(&s, 1.5, 130, 50)[1], 1.0);

    // Single Frame: visible on its frame only.
    s.set_time(Tick::from_seconds_f64(1.0));
    s.execute("paint.stroke", json!({"layer": id, "points": [[50, 50]], "durationMode": "singleFrame", "color": [1, 0, 0, 1], "diameter": 6})).unwrap();
    assert_eq!(px(&s, 1.0, 100, 70), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&s, 1.0 + 1.0 / 30.0, 100, 70), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(px(&s, 0.9, 100, 70), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn eraser_last_stroke_only_targets_previous_stroke() {
    let (mut s, id) = setup();
    s.execute("paint.stroke", json!({"layer": id, "points": [[40, 30]], "color": [0, 1, 0, 1], "diameter": 30})).unwrap();
    let b = s.execute("paint.stroke", json!({"layer": id, "points": [[50, 30]], "color": [1, 0, 0, 1], "diameter": 30})).unwrap()["stroke"].as_u64().unwrap();
    let e = s.execute("paint.stroke", json!({"layer": id, "kind": "eraser", "eraseMode": "lastStrokeOnly", "points": [[50, 30]], "diameter": 10})).unwrap();
    let l = layer(&s, id);
    let eg = l.props.find_group(e["stroke"].as_u64().unwrap()).unwrap();
    assert_eq!(eg.get("target").unwrap().value.as_f64() as u64, b);
    assert_eq!(px(&s, 0.0, 100, 50), [0.0, 1.0, 0.0, 1.0]);
}

#[test]
fn clone_stroke_uses_source_point_and_aligned_offset() {
    let (mut s, id) = setup();
    // Paint a red dot at layer (20, 20), then clone it to (70, 40).
    s.execute("paint.stroke", json!({"layer": id, "points": [[20, 20]], "color": [1, 0, 0, 1], "diameter": 10, "sizePressure": false})).unwrap();
    s.execute("paint.setCloneSource", json!({"layer": id, "point": [20, 20]})).unwrap();
    let r = s.execute("paint.stroke", json!({"layer": id, "kind": "clone", "points": [[70, 40]], "diameter": 12})).unwrap();
    let l = layer(&s, id);
    let g = l.props.find_group(r["stroke"].as_u64().unwrap()).unwrap();
    assert_eq!(g.sub("stroke_options").unwrap().get("clone_position").unwrap().value.as_vec2(), [20.0, 20.0]);
    assert_eq!(s.state.paint.clone_offset, Some([-50.0, -20.0]));
    // Clone source is this layer before paint: blue copied over blue. With Paint on the
    // source layer itself the clone samples the unpainted source.
    assert_eq!(px(&s, 0.0, 120, 60), [0.0, 0.0, 1.0, 1.0]);
    // Aligned: the next stroke keeps the offset.
    let r = s.execute("paint.stroke", json!({"layer": id, "kind": "clone", "points": [[80, 45]]})).unwrap();
    let l = layer(&s, id);
    let g = l.props.find_group(r["stroke"].as_u64().unwrap()).unwrap();
    assert_eq!(g.sub("stroke_options").unwrap().get("clone_position").unwrap().value.as_vec2(), [30.0, 25.0]);
}

#[test]
fn clone_from_another_layer() {
    let (mut s, id) = setup();
    let red = s.execute("layer.newSolid", json!({"width": 100, "height": 60, "color": [1, 0, 0]})).unwrap()["layer"].as_u64().unwrap();
    // Hide the red layer by moving it out of view instead of relying on a switch command.
    s.execute("prop.set", json!({"layer": red, "path": "transform/position", "value": [1000, 1000, 0]})).unwrap();
    s.execute("paint.stroke", json!({"layer": id, "kind": "clone", "cloneSource": red, "clonePosition": [50, 30], "points": [[50, 30]], "diameter": 10}))
        .unwrap();
    assert_eq!(px(&s, 0.0, 100, 50), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&s, 0.0, 70, 50), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn paint_options_and_presets() {
    let mut s = Session::default();
    let o = s.execute("paint.options", json!({"opacity": 50, "mode": "Multiply", "channels": "RGB", "durationMode": "custom", "customFrames": 5})).unwrap();
    assert_eq!(o["opacity"], json!(50.0));
    assert_eq!(s.state.paint.mode, 2);
    assert_eq!(s.state.paint.channels, 1);
    assert_eq!(s.state.paint.duration, 3);
    assert!(s.execute("paint.options", json!({"mode": "Nope"})).is_err());
    let r = s.execute("paint.brushPreset", json!({"preset": "Soft Round 45 px"})).unwrap();
    assert_eq!(r["preset"], json!(13));
    assert_eq!(s.state.paint.diameter, 45.0);
    assert_eq!(s.state.paint.hardness, 0.0);
    let list = s.execute("paint.presets", json!({})).unwrap();
    assert!(list.as_array().unwrap().len() >= 20);
    // State is serde (agents read it).
    let j = serde_json::to_value(&s.state).unwrap();
    let back: crate::EditorState = serde_json::from_value(j).unwrap();
    assert_eq!(back.paint, s.state.paint);
}

fn pin_count(s: &Session, id: u64) -> usize {
    let l = layer(s, id);
    l.effects().unwrap().groups().map(effectcraft_effects::puppet::pin_count).sum()
}

#[test]
fn puppet_pins_deform_and_undo() {
    let (mut s, id) = setup();
    let a = s.execute("puppet.addPin", json!({"layer": id, "position": [10, 30]})).unwrap();
    let b = s.execute("puppet.addPin", json!({"layer": id, "position": [90, 30]})).unwrap();
    assert_eq!(a["mesh"], b["mesh"], "second pin lands on the same mesh");
    assert_eq!(pin_count(&s, id), 2);
    let l = layer(&s, id);
    let pin = l.props.find_group(a["pin"].as_u64().unwrap()).unwrap();
    assert_eq!(pin.name, "Puppet Pin 1");
    assert_eq!(pin.get("position").unwrap().keys.len(), 1, "position pins are keyframed");
    // Pins at rest: the layer looks the same.
    assert_eq!(px(&s, 0.0, 100, 50), [0.0, 0.0, 1.0, 1.0]);
    let info = s.execute("puppet.info", json!({"layer": id})).unwrap();
    assert!(info["meshes"][0]["triangles"].as_u64().unwrap() > 50);
    // Move both pins down 15 px at 1 s: the whole layer follows.
    s.set_time(Tick::from_seconds_f64(1.0));
    s.execute("puppet.movePin", json!({"layer": id, "pin": a["pin"], "position": [10, 45]})).unwrap();
    s.execute("puppet.movePin", json!({"layer": id, "pin": "Puppet Pin 2", "position": [90, 45]})).unwrap();
    let l = layer(&s, id);
    assert_eq!(l.props.find_group(a["pin"].as_u64().unwrap()).unwrap().get("position").unwrap().keys.len(), 2);
    assert_eq!(px(&s, 1.0, 100, 90), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(px(&s, 1.0, 100, 25)[3], 0.0);
    assert_eq!(px(&s, 0.0, 100, 25), [0.0, 0.0, 1.0, 1.0]);
    // A third pin added while deformed maps back to the rest mesh.
    let c = s.execute("puppet.addPin", json!({"layer": id, "position": [50, 45]})).unwrap();
    let rest = c["rest"].as_array().unwrap();
    assert!((rest[1].as_f64().unwrap() - 30.0).abs() < 0.5, "{rest:?}");
    s.undo();
    s.undo();
    s.undo();
    assert_eq!(pin_count(&s, id), 2);
    assert_eq!(px(&s, 1.0, 100, 25), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn puppet_pin_kinds_and_mesh_options() {
    let (mut s, id) = setup();
    for (k, pos) in [("position", [10, 30]), ("starch", [50, 30]), ("bend", [90, 30]), ("overlap", [30, 30]), ("advanced", [70, 30])] {
        s.execute("puppet.addPin", json!({"layer": id, "kind": k, "position": pos})).unwrap();
    }
    let l = layer(&s, id);
    let names: Vec<String> = {
        let fx = l.effects().unwrap().groups().next().unwrap();
        effectcraft_effects::puppet::meshes(fx).flat_map(|m| effectcraft_effects::puppet::pins(m).map(|p| p.name.clone())).collect()
    };
    assert_eq!(names, ["Puppet Pin 1", "Starch 1", "Puppet Pin 2", "Overlap 1", "Puppet Pin 3"]);
    assert!(s.execute("puppet.movePin", json!({"layer": id, "pin": "Puppet Pin 2", "position": [1, 1]})).is_err(), "bend pins have no position");
    s.execute("puppet.setPin", json!({"layer": id, "pin": "Puppet Pin 2", "rotation": 30})).unwrap();
    s.execute("puppet.setPin", json!({"layer": id, "pin": "Starch 1", "amount": 80, "extent": 25})).unwrap();
    s.execute("puppet.mesh", json!({"layer": id, "density": 80, "expansion": 5})).unwrap();
    let l = layer(&s, id);
    let fx = l.effects().unwrap().groups().next().unwrap();
    let m = effectcraft_effects::puppet::meshes(fx).next().unwrap();
    assert_eq!(m.get("density").unwrap().value.as_f64(), 80.0);
    assert_eq!(m.get("expansion").unwrap().value.as_f64(), 5.0);
    // Renders, deterministically.
    let cid = s.active_comp_id().unwrap();
    let a = s.render(cid, Tick::ZERO, RenderOpts::default());
    let b = s.render(cid, Tick::ZERO, RenderOpts::default());
    assert_eq!(a, b);
    s.execute("puppet.removePin", json!({"layer": id, "pin": "Overlap 1"})).unwrap();
    assert_eq!(pin_count(&s, id), 4);
}

#[test]
fn project_with_paint_and_puppet_round_trips() {
    let (mut s, id) = setup();
    s.execute("paint.stroke", json!({"layer": id, "points": [[10, 30, 0.5], [90, 30, 1.0]], "durationMode": "writeOn"})).unwrap();
    s.execute("puppet.addPin", json!({"layer": id, "position": [10, 30]})).unwrap();
    let j = s.project.to_json();
    let back = effectcraft_project::Project::from_json(&j).unwrap();
    assert_eq!(&back, s.project.as_ref());
    let v: Value = serde_json::from_str(&j).unwrap();
    assert!(v.to_string().contains("ec.paint.paint") && v.to_string().contains("ec.distort.puppet"));
}
