use serde_json::json;

use crate::Session;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

#[test]
fn demo_opens_and_renders() {
    let s = demo();
    let cid = s.active_comp_id().unwrap();
    let img = s.render(cid, s.time(), effectcraft_render::RenderOpts { scale: 0.25, ..Default::default() });
    assert_eq!((img.width, img.height), (480, 270));
    let lit = img.data.iter().filter(|p| p[3] > 0.99).count();
    assert!(lit > 100_000, "{lit}");
}

#[test]
fn every_command_has_unique_id_and_runs_or_reports() {
    let mut ids = std::collections::HashSet::new();
    for c in crate::command_specs() {
        assert!(ids.insert(c.id), "duplicate {}", c.id);
    }
    assert!(ids.len() >= 90, "{}", ids.len());
}

#[test]
fn layer_workflow_with_undo() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Test", "width": 640, "height": 360, "frameRate": 30, "duration": 4})).unwrap();
    let r = s.execute("layer.newSolid", json!({"color": "#336699"})).unwrap();
    let lid = r["layer"].as_u64().unwrap();
    s.execute("prop.set", json!({"layer": lid, "path": "transform/opacity", "value": 50})).unwrap();
    let tree = s.execute("layer.tree", json!({"layer": lid})).unwrap();
    assert!(tree.to_string().contains("\"value\":50.0"));
    s.execute("prop.toggleAnimation", json!({"layer": lid, "path": "transform/position"})).unwrap();
    s.execute("time.set", json!({"time": 2.0})).unwrap();
    s.execute("prop.set", json!({"layer": lid, "path": "transform/position", "value": [100, 100]})).unwrap();
    let comp = s.active_comp().unwrap();
    let pos = comp.layers[0].props.prop("transform/position").unwrap();
    assert_eq!(pos.keys.len(), 2);
    s.execute("prop.select", json!({"layer": lid, "path": "transform/position"})).unwrap();
    assert_eq!(s.state.selected_keys.len(), 2);
    s.execute("keys.easyEase", json!({})).unwrap();
    s.execute("keys.move", json!({"delta": 0.5})).unwrap();
    let pos = s.active_comp().unwrap().layers[0].props.prop("transform/position").unwrap().clone();
    assert!((pos.keys[0].time.seconds() - 0.5).abs() < 0.02);
    assert_eq!(pos.keys[0].out_interp, effectcraft_keyframe::Interp::Bezier);
    // undo back to before the move
    s.execute("edit.undo", json!({})).unwrap();
    let pos = s.active_comp().unwrap().layers[0].props.prop("transform/position").unwrap().clone();
    assert!(pos.keys[0].time.seconds().abs() < 0.02);
    s.execute("effect.apply", json!({"layer": lid, "effect": "Gaussian Blur"})).unwrap();
    s.execute("prop.set", json!({"layer": lid, "path": "effects/#1/blurriness", "value": 12})).unwrap();
    s.execute("edit.duplicate", json!({})).unwrap();
    assert_eq!(s.active_comp().unwrap().layers.len(), 2);
    s.execute("layer.precompose", json!({"layers": [1, 2], "name": "Pre"})).unwrap();
    assert_eq!(s.active_comp().unwrap().layers.len(), 1);
}

#[test]
fn text_shape_mask_matte_parent() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width": 800, "height": 450, "duration": 3})).unwrap();
    let t = s.execute("layer.newText", json!({"text": "Hello", "size": 90})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.addTextAnimator", json!({"layer": t, "properties": ["position", "opacity"]})).unwrap();
    let sh = s.execute("layer.newShape", json!({"kind": "star", "fill": "#ffcc00"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.addShapeItem", json!({"layer": sh, "kind": "trim"})).unwrap();
    s.execute("layer.addMask", json!({"layer": sh, "shape": "ellipse"})).unwrap();
    s.execute("layer.setMask", json!({"layer": sh, "mask": 1, "mode": "Subtract", "inverted": true})).unwrap();
    s.execute("layer.setTrackMatte", json!({"layer": sh, "matte": t, "kind": "luma"})).unwrap();
    s.execute("layer.setParent", json!({"layers": [sh], "parent": t})).unwrap();
    assert!(s.execute("layer.setParent", json!({"layers": [t], "parent": sh})).is_err());
    s.execute("layer.setBlendMode", json!({"layers": [sh], "mode": "Screen"})).unwrap();
    s.execute("layer.setSwitch", json!({"layers": [sh], "switch": "threeD"})).unwrap();
    let cid = s.active_comp_id().unwrap();
    let _ = s.render(cid, s.time(), Default::default());
    let info = s.execute("comp.info", json!({})).unwrap();
    assert_eq!(info["layers"].as_array().unwrap().len(), 2);
}

#[test]
fn save_and_open_roundtrip() {
    let mut s = demo();
    let dir = std::env::temp_dir().join(format!("ec-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("demo.ecproj").to_string_lossy().to_string();
    s.execute("file.saveAs", json!({"path": path})).unwrap();
    let before = s.project.clone();
    let mut s2 = Session::default();
    s2.execute("file.open", json!({"path": path})).unwrap();
    assert_eq!(*before, *s2.project);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn help_links_emit_urls() {
    let mut s = Session::default();
    let r = s.execute("help.discord", json!({})).unwrap();
    assert_eq!(r["url"], "https://discord.gg/artcraft");
    assert!(s.drain_events().iter().any(|e| matches!(e, crate::Event::OpenUrl(u) if u.contains("discord"))));
    assert_eq!(s.execute("help.github", json!({})).unwrap()["url"], "https://github.com/storytold/effectcraft");
}

#[test]
fn time_navigation() {
    let mut s = demo();
    s.execute("time.start", json!({})).unwrap();
    let r = s.execute("time.nextKey", json!({})).unwrap();
    assert!(r["time"].as_f64().unwrap() > 0.0);
    s.execute("time.set", json!({"frame": 45})).unwrap();
    assert_eq!(s.execute("time.step", json!({"frames": 5})).unwrap()["frame"], 50);
}

