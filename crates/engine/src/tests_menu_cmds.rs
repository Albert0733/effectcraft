//! Tests for the menu-parity command families (Layer / Edit / Animation / File / Composition /
//! View menus), including undo.

use effectcraft_keyframe::Value as KV;
use effectcraft_project::{FrameBlend, ItemKind, MatteKind, Quality, Sampling};
use serde_json::{Value, json};

use crate::{EngineError, Event, Session};

fn comp() -> Session {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Main", "width": 640, "height": 360, "frameRate": 30, "duration": 10})).unwrap();
    s
}

fn solid(s: &mut Session, color: &str) -> u64 {
    s.execute("layer.newSolid", json!({"color": color, "width": 200, "height": 100})).unwrap()["layer"].as_u64().unwrap()
}

fn layer(s: &Session, id: u64) -> effectcraft_project::Layer {
    s.active_comp().unwrap().layer(effectcraft_project::LayerId(id)).unwrap().clone()
}

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("effectcraft-menu-tests-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name).to_string_lossy().to_string()
}

fn frontend_events(s: &mut Session) -> Vec<(String, Value)> {
    s.drain_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Frontend { command, params } => Some((command, params)),
            _ => None,
        })
        .collect()
}

#[test]
fn layer_quality_sampling_frame_blending_and_undo() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    s.execute("layer.quality", json!({"quality": "wireframe"})).unwrap();
    s.execute("layer.sampling", json!({"sampling": "bicubic"})).unwrap();
    s.execute("layer.frameBlending", json!({"mode": "pixelMotion"})).unwrap();
    let l = layer(&s, a);
    assert_eq!((l.switches.quality, l.switches.sampling, l.switches.frame_blend), (Quality::Wireframe, Sampling::Bicubic, FrameBlend::PixelMotion));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&s, a).switches.frame_blend, FrameBlend::Off);
    assert!(s.execute("layer.quality", json!({"quality": "fancy"})).is_err());
}

#[test]
fn layer_switches_menu() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    let b = solid(&mut s, "#00ff00");
    s.execute("layer.select", json!({"layers": [b]})).unwrap();
    s.execute("layer.hideOtherVideo", json!({})).unwrap();
    assert!(!layer(&s, a).switches.video && layer(&s, b).switches.video);
    s.execute("layer.showAllVideo", json!({})).unwrap();
    assert!(layer(&s, a).switches.video);
    s.execute("layer.setSwitch", json!({"layers": [a, b], "switch": "lock"})).unwrap();
    assert!(layer(&s, a).switches.locked);
    s.execute("layer.unlockAll", json!({})).unwrap();
    assert!(!layer(&s, a).switches.locked && !layer(&s, b).switches.locked);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(layer(&s, a).switches.locked);
    // Expressions on / off.
    s.execute("prop.setExpression", json!({"layer": a, "path": "transform/opacity", "expression": "50"})).unwrap();
    s.execute("layer.expressions", json!({"layers": [a], "enabled": false})).unwrap();
    assert!(!layer(&s, a).props.prop("transform/opacity").unwrap().expr.as_ref().unwrap().enabled);
    s.execute("layer.expressions", json!({"layers": [a], "enabled": true})).unwrap();
    assert!(layer(&s, a).props.prop("transform/opacity").unwrap().has_expression());
}

#[test]
fn transform_dialog_values_and_center_anchor() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    s.execute("layer.setTransform", json!({"prop": "position", "value": [100, 50]})).unwrap();
    s.execute("layer.setTransform", json!({"prop": "rotation", "value": 90})).unwrap();
    s.execute("layer.setTransform", json!({"prop": "opacity", "value": 25})).unwrap();
    let l = layer(&s, a);
    assert_eq!(l.props.prop("transform/position").unwrap().value, KV::Vec3([100.0, 50.0, 0.0]));
    assert_eq!(l.props.prop("transform/opacity").unwrap().value.as_f64(), 25.0);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&s, a).props.prop("transform/opacity").unwrap().value.as_f64(), 100.0);
    assert!(s.execute("layer.setTransform", json!({"prop": "skew", "value": 1})).is_err());

    // Move the anchor off-centre, then Center Anchor Point keeps the layer in place.
    s.execute("layer.setTransform", json!({"prop": "anchor", "value": [0, 0]})).unwrap();
    s.execute("layer.setTransform", json!({"prop": "rotation", "value": 0})).unwrap();
    let before = layer(&s, a).props.prop("transform/position").unwrap().value.as_vec3();
    s.execute("layer.centerAnchor", json!({})).unwrap();
    let l = layer(&s, a);
    assert_eq!(l.props.prop("transform/anchor").unwrap().value.as_vec2(), [100.0, 50.0]);
    let after = l.props.prop("transform/position").unwrap().value.as_vec3();
    assert!((after[0] - before[0] - 100.0).abs() < 1e-6 && (after[1] - before[1] - 50.0).abs() < 1e-6, "{before:?} → {after:?}");

    s.execute("layer.autoOrient", json!({"mode": "alongPath"})).unwrap();
    assert_eq!(layer(&s, a).auto_orient, effectcraft_project::AutoOrient::AlongPath);
}

#[test]
fn mask_menu_on_existing_masks() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    assert!(!s.is_enabled("layer.mask.reset"));
    s.execute("layer.addMask", json!({})).unwrap();
    s.execute("layer.addMask", json!({"shape": "ellipse"})).unwrap();
    assert!(s.is_enabled("layer.mask.reset"));
    let masks = |s: &Session| layer(s, a).masks().unwrap().groups().cloned().collect::<Vec<_>>();
    s.execute("layer.mask.set", json!({"field": "feather", "value": 12, "mask": 1})).unwrap();
    assert_eq!(masks(&s)[0].get("feather").unwrap().value, KV::Vec2([12.0, 12.0]));
    s.execute("layer.mask.set", json!({"field": "opacity", "value": 40})).unwrap();
    assert!(masks(&s).iter().all(|m| m.get("opacity").unwrap().value.as_f64() == 40.0));
    s.execute("layer.mask.invert", json!({"mask": 2})).unwrap();
    s.execute("layer.mask.mode", json!({"mode": "Subtract", "mask": 2})).unwrap();
    s.execute("layer.mask.lock", json!({"mask": 1})).unwrap();
    let kinds: Vec<_> = masks(&s).iter().map(|m| m.kind.clone()).collect();
    assert!(matches!(kinds[1], effectcraft_project::GroupKind::Mask { inverted: true, mode: effectcraft_project::MaskMode::Subtract, .. }));
    assert!(matches!(kinds[0], effectcraft_project::GroupKind::Mask { locked: true, .. }));
    s.execute("layer.mask.unlockAll", json!({})).unwrap();
    assert!(matches!(masks(&s)[0].kind, effectcraft_project::GroupKind::Mask { locked: false, .. }));
    s.execute("layer.mask.lockOthers", json!({"mask": 1})).unwrap();
    assert!(matches!(masks(&s)[1].kind, effectcraft_project::GroupKind::Mask { locked: true, .. }));
    s.execute("layer.mask.reset", json!({"mask": 1})).unwrap();
    assert_eq!(masks(&s)[0].get("opacity").unwrap().value.as_f64(), 100.0);
    s.execute("layer.mask.shape", json!({"mask": 1, "rect": [10, 10, 50, 40], "shape": "ellipse"})).unwrap();
    s.execute("layer.mask.remove", json!({"mask": 2})).unwrap();
    assert_eq!(masks(&s).len(), 1);
    s.execute("layer.mask.removeAll", json!({})).unwrap();
    assert!(masks(&s).is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(masks(&s).len(), 1);
}

#[test]
fn layer_markers_lock_and_delete() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    s.execute("time.set", json!({"time": 1.0})).unwrap();
    s.execute("layer.addMarker", json!({"comment": "hit"})).unwrap();
    s.execute("layer.addMarker", json!({"time": 2.0})).unwrap();
    assert_eq!(layer(&s, a).markers.len(), 2);
    s.execute("layer.markersLock", json!({})).unwrap();
    assert!(layer(&s, a).markers_locked);
    s.execute("layer.deleteAllMarkers", json!({})).unwrap();
    assert_eq!(layer(&s, a).markers.len(), 2, "locked markers survive");
    s.execute("layer.markersLock", json!({"value": false})).unwrap();
    s.execute("layer.deleteAllMarkers", json!({})).unwrap();
    assert!(layer(&s, a).markers.is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&s, a).markers.len(), 2);
}

#[test]
fn track_matte_menu() {
    let mut s = comp();
    let below = solid(&mut s, "#ff0000");
    let top = solid(&mut s, "#00ff00");
    s.execute("layer.select", json!({"layers": [below]})).unwrap();
    s.execute("layer.trackMatte", json!({"op": "luma"})).unwrap();
    let m = layer(&s, below).track_matte.unwrap();
    assert_eq!((m.layer.0, m.kind), (top, MatteKind::Luma));
    assert!(!layer(&s, top).switches.video);
    s.execute("layer.trackMatte", json!({"op": "alphaInverted"})).unwrap();
    assert_eq!(layer(&s, below).track_matte.unwrap().kind, MatteKind::AlphaInverted);
    s.execute("layer.trackMatte", json!({"op": "none"})).unwrap();
    assert!(layer(&s, below).track_matte.is_none());
    s.execute("layer.select", json!({"layers": [top]})).unwrap();
    assert!(s.execute("layer.trackMatte", json!({"op": "above"})).is_err());
    s.execute("layer.trackMatte", json!({"op": "below"})).unwrap();
    assert_eq!(layer(&s, top).track_matte.unwrap().layer.0, below);
}

#[test]
fn keyframe_clipboard_paste_and_paste_reversed() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    let b = solid(&mut s, "#00ff00");
    s.execute("prop.toggleAnimation", json!({"layer": a, "path": "transform/opacity"})).unwrap();
    s.execute("time.set", json!({"time": 1.0})).unwrap();
    s.execute("prop.set", json!({"layer": a, "path": "transform/opacity", "value": 0})).unwrap();
    s.execute("prop.select", json!({"layer": a, "path": "transform/opacity"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    assert_eq!(s.state.key_clipboard.iter().map(|c| c.keys.len()).sum::<usize>(), 2);
    s.execute("layer.select", json!({"layers": [b]})).unwrap();
    s.execute("time.set", json!({"time": 2.0})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let keys = layer(&s, b).props.prop("transform/opacity").unwrap().keys.clone();
    assert_eq!(keys.len(), 2);
    assert!((keys[0].time.seconds() - 2.0).abs() < 1e-6 && keys[0].value.as_f64() == 100.0);
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.pasteReversedKeyframes", json!({})).unwrap();
    let keys = layer(&s, b).props.prop("transform/opacity").unwrap().keys.clone();
    assert_eq!((keys[0].value.as_f64(), keys[1].value.as_f64()), (0.0, 100.0));
}

#[test]
fn property_links_and_expression_only() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    let b = solid(&mut s, "#00ff00");
    s.execute("prop.select", json!({"layer": a, "path": "transform/position"})).unwrap();
    s.execute("edit.copyWithPropertyLinks", json!({})).unwrap();
    s.execute("layer.select", json!({"layers": [b]})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let e = layer(&s, b).props.prop("transform/position").unwrap().expr.clone().unwrap();
    let name = layer(&s, a).name;
    assert_eq!(e.text, format!("comp(\"Main\").layer({}).transform(\"Position\")", serde_json::to_string(&name).unwrap()));
    // Relative links use thisComp.
    s.execute("prop.select", json!({"layer": a, "path": "transform/opacity"})).unwrap();
    s.execute("edit.copyWithRelativePropertyLinks", json!({})).unwrap();
    s.execute("layer.select", json!({"layers": [b]})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    assert!(layer(&s, b).props.prop("transform/opacity").unwrap().expr.as_ref().unwrap().text.starts_with("thisComp.layer("));
    // Copy Expression Only.
    s.execute("prop.setExpression", json!({"layer": a, "path": "transform/rotation", "expression": "time * 90"})).unwrap();
    s.execute("prop.select", json!({"layer": a, "path": "transform/rotation"})).unwrap();
    s.execute("edit.copyExpressionOnly", json!({})).unwrap();
    s.execute("layer.select", json!({"layers": [b]})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(layer(&s, b).props.prop("transform/rotation").unwrap().expr.as_ref().unwrap().text, "time * 90");
    // Layers with property links: the pasted copy follows the original.
    s.state.selected_props.clear();
    s.execute("layer.select", json!({"layers": [a]})).unwrap();
    s.execute("edit.copyWithPropertyLinks", json!({})).unwrap();
    let new = s.execute("edit.paste", json!({})).unwrap();
    let nid = new[0].as_u64().unwrap();
    assert!(layer(&s, nid).props.prop("transform/scale").unwrap().has_expression());
}

#[test]
fn lift_and_extract_work_area() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    s.execute("comp.workArea", json!({"start": 2.0, "end": 4.0})).unwrap();
    s.execute("edit.liftWorkArea", json!({})).unwrap();
    let c = s.active_comp().unwrap();
    assert_eq!(c.layers.len(), 2, "split around the gap");
    let mut spans: Vec<(f64, f64)> = c.layers.iter().map(|l| (l.in_point.seconds(), l.out_point.seconds())).collect();
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    assert!((spans[0].1 - 2.0).abs() < 1e-6 && (spans[1].0 - 4.0).abs() < 1e-6);
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("layer.select", json!({"layers": [a]})).unwrap();
    s.execute("edit.extractWorkArea", json!({})).unwrap();
    let c = s.active_comp().unwrap();
    let mut spans: Vec<(f64, f64)> = c.layers.iter().map(|l| (l.in_point.seconds(), l.out_point.seconds())).collect();
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    assert!((spans[1].0 - 2.0).abs() < 1e-6 && (spans[1].1 - 8.0).abs() < 1e-6, "{spans:?}");
}

#[test]
fn label_group_purge_and_edit_original() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    let b = solid(&mut s, "#00ff00");
    let _c = solid(&mut s, "#0000ff");
    s.execute("edit.label", json!({"layers": [a, b], "label": "Cyan"})).unwrap();
    s.execute("layer.select", json!({"layers": [a]})).unwrap();
    s.execute("edit.selectLabelGroup", json!({})).unwrap();
    assert_eq!(s.state.selected_layers.len(), 2);
    s.drain_events();
    s.execute("edit.purge", json!({"what": "memory"})).unwrap();
    assert!(s.drain_events().contains(&Event::PurgeCaches));
    assert!(!s.is_enabled("edit.editOriginal"));
}

#[test]
fn animation_menu_keys_presets_text() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    // Add Keyframe on selected properties.
    s.execute("prop.select", json!({"layer": a, "path": "transform/scale"})).unwrap();
    s.execute("anim.addKeyframe", json!({})).unwrap();
    s.execute("time.set", json!({"time": 1.0})).unwrap();
    s.execute("prop.set", json!({"layer": a, "path": "transform/scale", "value": [400, 400]})).unwrap();
    assert_eq!(layer(&s, a).props.prop("transform/scale").unwrap().keys.len(), 2);
    // Exponential Scale: a key per frame, geometric in between.
    s.execute("prop.select", json!({"layer": a, "path": "transform/scale"})).unwrap();
    s.execute("keys.exponentialScale", json!({})).unwrap();
    let keys = layer(&s, a).props.prop("transform/scale").unwrap().keys.clone();
    assert_eq!(keys.len(), 31);
    assert!((keys[15].value.as_vec3()[0] - 200.0).abs() < 1e-6, "{:?}", keys[15].value);
    s.execute("edit.undo", json!({})).unwrap();
    // Time-Reverse Keyframes.
    s.execute("prop.select", json!({"layer": a, "path": "transform/scale"})).unwrap();
    s.execute("keys.timeReverse", json!({})).unwrap();
    let keys = layer(&s, a).props.prop("transform/scale").unwrap().keys.clone();
    assert_eq!((keys[0].value.as_vec3()[0], keys[1].value.as_vec3()[0]), (400.0, 100.0));

    // Presets round-trip through a file onto another layer at the CTI.
    let path = tmp("scale.ecpreset");
    s.execute("prop.select", json!({"layer": a, "path": "transform/scale"})).unwrap();
    s.execute("anim.savePreset", json!({"path": path})).unwrap();
    let b = solid(&mut s, "#00ff00");
    s.execute("time.set", json!({"time": 3.0})).unwrap();
    let r = s.execute("anim.applyPreset", json!({"path": path, "layers": [b]})).unwrap();
    assert_eq!(r["applied"], 1);
    let keys = layer(&s, b).props.prop("transform/scale").unwrap().keys.clone();
    assert_eq!(keys.len(), 2);
    assert!((keys[0].time.seconds() - 3.0).abs() < 1e-6);

    // Text: animate, add selector, reveal, remove all.
    let t = s.execute("layer.newText", json!({"text": "Hi"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.addTextAnimator", json!({"properties": ["anchor", "position", "scale", "skew", "rotation", "opacity"]})).unwrap();
    s.execute("text.addSelector", json!({"kind": "range"})).unwrap();
    let anim = layer(&s, t).props.group("text/animators").unwrap().groups().next().unwrap().clone();
    assert_eq!(anim.sub("selectors").unwrap().children.len(), 2);
    // Skew brings Skew Axis along.
    assert_eq!(anim.sub("properties").unwrap().children.len(), 7);
    s.execute("text.addSelector", json!({"kind": "wiggly"})).unwrap();
    assert!(s.execute("text.addSelector", json!({"kind": "other"})).is_err());
    s.execute("text.removeAllAnimators", json!({})).unwrap();
    assert!(layer(&s, t).props.group("text/animators").unwrap().children.is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&s, t).props.group("text/animators").unwrap().children.len(), 1);
}

#[test]
fn reveal_properties() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    s.execute("prop.toggleAnimation", json!({"layer": a, "path": "transform/rotation"})).unwrap();
    s.execute("prop.setExpression", json!({"layer": a, "path": "transform/opacity", "expression": "50"})).unwrap();
    s.execute("prop.set", json!({"layer": a, "path": "transform/scale", "value": [50, 50]})).unwrap();
    s.drain_events();
    let n = |v: &Value| v["props"].as_array().unwrap().len();
    assert_eq!(n(&s.execute("anim.reveal", json!({"kind": "keyframes"})).unwrap()), 1);
    assert_eq!(n(&s.execute("anim.reveal", json!({"kind": "animation"})).unwrap()), 2);
    assert_eq!(n(&s.execute("anim.reveal", json!({"kind": "modified"})).unwrap()), 3);
    assert!(frontend_events(&mut s).iter().all(|(c, _)| c == "timeline.revealProps"));
}

#[test]
fn sequence_layers() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    let b = solid(&mut s, "#00ff00");
    let c = solid(&mut s, "#0000ff");
    for (id, out) in [(a, 2.0), (b, 3.0), (c, 1.0)] {
        s.execute("layer.timing", json!({"layers": [id], "out": out})).unwrap();
    }
    s.execute("layer.sequence", json!({"layers": [a, b, c], "overlap": true, "duration": 0.5, "transition": "dissolveFront"})).unwrap();
    assert!((layer(&s, b).in_point.seconds() - 1.5).abs() < 1e-6);
    assert!((layer(&s, c).in_point.seconds() - 4.0).abs() < 1e-6);
    assert_eq!(layer(&s, c).props.prop("transform/opacity").unwrap().keys.len(), 2);
}

#[test]
fn file_menu_project_ops() {
    let mut s = comp();
    let f = s.execute("project.newFolder", json!({"name": "Stuff"})).unwrap()["item"].as_u64().unwrap();
    assert!(s.project.item(effectcraft_project::ItemId(f)).unwrap().is_folder());
    let p1 = s.execute("file.importPlaceholder", json!({"name": "Shot 1", "width": 320, "height": 240})).unwrap()["item"].as_u64().unwrap();
    let p2 = s.execute("file.importPlaceholder", json!({"name": "Unused"})).unwrap()["item"].as_u64().unwrap();
    s.execute("file.importSolid", json!({"name": "Grey"})).unwrap();
    s.state.project_selection = vec![effectcraft_project::ItemId(p1)];
    let r = s.execute("file.newCompFromSelection", json!({})).unwrap();
    let nc = effectcraft_project::ItemId(r["comps"][0].as_u64().unwrap());
    assert_eq!((s.project.comp(nc).unwrap().width, s.project.comp(nc).unwrap().layers.len()), (320, 1));
    // Missing footage = the placeholders.
    let miss = s.execute("file.findMissing", json!({"what": "footage"})).unwrap();
    assert_eq!(miss["items"].as_array().unwrap().len(), 2);
    assert!(s.execute("file.findMissing", json!({"what": "fonts"})).is_ok());
    // Interpretation.
    s.state.project_selection = vec![effectcraft_project::ItemId(p1)];
    s.execute("file.interpretFootage", json!({"frameRate": 24, "alpha": "premultiplied", "loop": 3})).unwrap();
    s.execute("file.rememberInterpretation", json!({})).unwrap();
    s.state.project_selection = vec![effectcraft_project::ItemId(p2)];
    s.execute("file.applyInterpretation", json!({})).unwrap();
    match &s.project.item(effectcraft_project::ItemId(p2)).unwrap().kind {
        ItemKind::Footage(f) => assert_eq!((f.alpha, f.loop_count, f.frame_rate.as_f64().round()), (effectcraft_project::AlphaMode::Premultiplied, 3, 24.0)),
        _ => panic!(),
    }
    // Remove Unused Footage drops the unused placeholder and the unused solid.
    let before = s.project.items.len();
    s.execute("file.removeUnusedFootage", json!({})).unwrap();
    assert_eq!(s.project.items.len(), before - 2);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.items.len(), before);
    // Replace with a solid retargets the layer.
    s.state.project_selection = vec![effectcraft_project::ItemId(p1)];
    s.execute("file.replaceWithSolid", json!({"color": "#336699"})).unwrap();
    assert!(matches!(s.project.comp(nc).unwrap().layers[0].source, effectcraft_project::LayerSource::Solid { .. }));
    // Reduce Project keeps only the selected comp and what it uses (+ folders of kept items).
    s.state.project_selection = vec![nc];
    s.execute("file.reduceProject", json!({})).unwrap();
    assert!(s.project.items.len() <= 2, "{:?}", s.project.items.values().map(|i| &i.name).collect::<Vec<_>>());
    // Save a copy leaves the session path alone.
    let path = tmp("copy.ecproj");
    s.execute("file.saveCopy", json!({"path": path})).unwrap();
    assert!(s.path.is_none() && std::fs::metadata(&path).is_ok());
    s.execute("file.closeProject", json!({})).unwrap();
    assert!(s.project.items.is_empty());
}

#[test]
fn consolidate_duplicate_footage() {
    let mut s = comp();
    let f = |s: &mut Session| s.execute("file.importPlaceholder", json!({})).unwrap()["item"].as_u64().unwrap();
    let a = f(&mut s);
    let b = f(&mut s);
    // Give both the same file path.
    let mut p = (*s.project).clone();
    for id in [a, b] {
        if let Some(ItemKind::Footage(ft)) = p.item_mut(effectcraft_project::ItemId(id)).map(|i| &mut i.kind) {
            ft.path = "/media/shot.mov".into();
        }
    }
    s.project = std::sync::Arc::new(p);
    s.execute("layer.addItem", json!({"item": b})).unwrap();
    assert_eq!(s.execute("file.consolidateFootage", json!({})).unwrap()["removed"], 1);
    let c = s.active_comp().unwrap();
    assert_eq!(c.layers[0].source.item().unwrap().0, a);
}

#[test]
fn run_script_steps() {
    let mut s = comp();
    let path = tmp("script.jsonl");
    std::fs::write(&path, "// make two solids\n{\"command\":\"layer.newSolid\",\"params\":{\"color\":\"#ff0000\"}}\n{\"method\":\"engine.execute\",\"params\":{\"id\":\"layer.newNull\",\"params\":{}}}\n").unwrap();
    let r = s.execute("file.runScript", json!({"path": path})).unwrap();
    assert_eq!(r["steps"], 2);
    assert_eq!(s.active_comp().unwrap().layers.len(), 2);
    assert!(s.execute("file.runScript", json!({"steps": [{"command": "nope.nope"}]})).is_err());
}

#[test]
fn crop_comp_and_save_frame() {
    let mut s = comp();
    let a = solid(&mut s, "#ff0000");
    // ROI crop moves layers so nothing shifts on screen.
    assert!(!s.is_enabled("comp.cropToRegionOfInterest"));
    s.execute("view.setRegionOfInterest", json!({"rect": [100, 50, 200, 100]})).unwrap();
    s.execute("comp.cropToRegionOfInterest", json!({})).unwrap();
    let c = s.active_comp().unwrap();
    assert_eq!((c.width, c.height), (200, 100));
    assert_eq!(layer(&s, a).props.prop("transform/position").unwrap().value.as_vec2(), [220.0, 130.0]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active_comp().unwrap().width, 640);
    // Crop to the selected layer's bounds (a 200×100 solid centred in 640×360).
    s.execute("layer.select", json!({"layers": [a]})).unwrap();
    let r = s.execute("comp.cropToLayerBounds", json!({})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(200), Some(100)));
    assert_eq!(layer(&s, a).props.prop("transform/position").unwrap().value.as_vec2(), [100.0, 50.0]);
    // Save Frame As ▸ File… writes a PNG.
    let path = tmp("frame.png");
    let r = s.execute("comp.saveFrameAs", json!({"path": path, "scale": 0.5})).unwrap();
    assert_eq!(r["width"], 100);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    // Responsive Design — Time.
    s.execute("time.set", json!({"time": 2.0})).unwrap();
    s.execute("comp.responsiveTime", json!({"op": "intro"})).unwrap();
    let m = &s.active_comp().unwrap().markers[0];
    assert!(m.protected && (m.duration.seconds() - 2.0).abs() < 1e-6);
}

#[test]
fn guides_add_clear_import_export() {
    let mut s = comp();
    s.execute("view.addGuide", json!({"orientation": "horizontal", "position": 40})).unwrap();
    s.execute("view.addGuide", json!({})).unwrap();
    assert_eq!(s.active_comp().unwrap().guides.len(), 2);
    assert_eq!(s.active_comp().unwrap().guides[1].position, 320.0);
    let path = tmp("guides.json");
    s.execute("view.exportGuides", json!({"path": path})).unwrap();
    s.execute("view.clearGuides", json!({})).unwrap();
    assert!(s.active_comp().unwrap().guides.is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active_comp().unwrap().guides.len(), 2);
    s.execute("view.clearGuides", json!({})).unwrap();
    s.execute("view.importGuides", json!({"path": path})).unwrap();
    assert_eq!(s.active_comp().unwrap().guides.len(), 2);
}

#[test]
fn frontend_commands_emit_events_and_stubs_are_disabled() {
    let mut s = comp();
    s.drain_events();
    s.execute("view.zoomIn", json!({})).unwrap();
    s.execute("window.panel", json!({"panel": "align"})).unwrap();
    let ev = frontend_events(&mut s);
    assert_eq!(ev[0].0, "view.zoomIn");
    assert_eq!(ev[1], ("window.panel".to_string(), json!({"panel": "align"})));
    for id in ["keys.audioToKeyframes", "camera.stereoRig", "view.displayColorManagement", "view.3d.default", "layer.create", "track.motion"] {
        assert!(!s.is_enabled(id), "{id}");
        assert!(matches!(s.execute(id, json!({})), Err(EngineError::Disabled(..))), "{id}");
    }
}
