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

#[test]
fn prop_get_and_render_rgba8() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "T", "width": 320, "height": 180, "duration": 2.0})).unwrap();
    let l = s.execute("layer.newSolid", json!({"color": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("prop.addKey", json!({"layer": l, "path": "transform/position", "time": 0.0, "value": [0, 0]})).unwrap();
    s.execute("prop.addKey", json!({"layer": l, "path": "transform/position", "time": 1.0, "value": [100, 50]})).unwrap();
    let v = s.execute("prop.get", json!({"layer": l, "path": "transform/position", "time": 1.0})).unwrap();
    assert_eq!(v["animated"], true);
    assert_eq!(v["keys"].as_array().unwrap().len(), 2);
    assert_eq!(v["value"][0].as_f64().unwrap(), 100.0);
    let cid = s.resolve_comp(Some(&json!("T"))).unwrap();
    let (w, h, rgba) = s.render_rgba8(cid, effectcraft_time::Tick::ZERO, 160).unwrap();
    assert_eq!((w, h), (160, 90));
    assert_eq!(rgba.len(), (w * h * 4) as usize);
}

#[test]
fn params_docs_parse() {
    use crate::commands::accepted_params;
    let k = |d: &str| accepted_params(d).unwrap();
    assert_eq!(k("{layers: [id|name|#n], add?, toggle?}"), ["layers", "add", "toggle"]);
    assert_eq!(k("{time? (s) | frame? | timecode?}"), ["time", "frame", "timecode"]);
    assert_eq!(k("{interpolation?|in?|out?: linear|bezier|hold, autoBezier?}"), ["interpolation", "in", "out", "autoBezier"]);
    assert_eq!(k("{keys: [{layer, prop, time}], add?}"), ["keys", "add"]);
    assert_eq!(k("{label: Red|Yellow|Aqua|…, layers?}"), ["label", "layers"]);
    assert_eq!(k("{name?, background? [r,g,b]|#hex, open?}"), ["name", "background", "open"]);
    assert!(k("{}").is_empty());
    assert!(accepted_params("free text").is_none());
    // Every registered command documents its params as a parseable key list.
    for c in crate::command_specs() {
        assert!(accepted_params(c.params).is_some(), "{}: {}", c.id, c.params);
    }
}

#[test]
fn execute_checked_rejects_unknown_params() {
    let mut s = demo();
    let e = s.execute_checked("layer.select", json!({"index": 2})).unwrap_err().to_string();
    assert!(e.contains("`index`") && e.contains("layers, add, toggle"), "{e}");
    s.execute_checked("layer.select", json!({"layers": ["#2"]})).unwrap();
    assert_eq!(s.state.selected_layers.len(), 1);
    // Aliases and always-accepted keys.
    s.execute_checked("layer.select", json!({"layer": "#1", "comp": s.active_comp_id().unwrap().0})).unwrap();
    s.execute_checked("time.set", json!({"frame": 3})).unwrap();
    assert!(s.execute_checked("edit.undo", json!({"steps": 2})).is_err());
    // Non-object params are not validated; the unchecked path stays lenient.
    s.execute("layer.select", json!({"layers": ["#1"], "index": 2})).unwrap();
}

#[test]
fn layer_tree_paths_resolve() {
    let mut s = demo();
    s.execute("effect.apply", json!({"layer": 1, "effect": "Gaussian Blur"})).unwrap();
    let comp = s.execute("comp.info", json!({})).unwrap();
    let mut checked = 0;
    for l in comp["layers"].as_array().unwrap() {
        let tree = s.execute("layer.tree", json!({"layer": l["id"]})).unwrap();
        let mut stack = vec![tree["properties"].clone()];
        while let Some(n) = stack.pop() {
            if let Some(ch) = n["children"].as_array() {
                stack.extend(ch.iter().cloned());
                continue;
            }
            let path = n["path"].as_str().unwrap();
            let got = s.execute("prop.get", json!({"layer": l["id"], "path": path})).unwrap();
            assert_eq!(got["uid"], n["uid"], "path {path} on layer {}", l["name"]);
            checked += 1;
        }
    }
    assert!(checked > 20, "{checked}");
}
