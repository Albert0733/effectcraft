//! Menu entries that used to be disabled stubs (M3.8 burn-down): mask and shape path commands,
//! mask options, Look at Layers, view layouts, Create Camera from 3D View, output modules,
//! Pre-render and Save Current Preview.

use std::sync::{Arc, Mutex};

use effectcraft_keyframe::Value as KV;
use effectcraft_project::render_queue::{OutputFormat, PostRenderAction};
use effectcraft_project::{FeatherFalloff, Footage, FootageKind, GroupKind, LayerId, LayerSource, MaskMotionBlur};
use effectcraft_time::{FrameRate, Tick};
use serde_json::json;

use crate::{Event, ExportJob, ExportResult, Exporter, Importer, Session};

fn comp() -> Session {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Main", "width": 400, "height": 300, "frameRate": 30, "duration": 4})).unwrap();
    s
}

fn layer(s: &Session, id: u64) -> effectcraft_project::Layer {
    s.active_comp().unwrap().layer(LayerId(id)).unwrap().clone()
}

/// A solid with one closed square mask; returns (layer id, mask uid).
fn masked(s: &mut Session) -> (u64, u64) {
    let l = s.execute("layer.newSolid", json!({"color": "#ffffff", "width": 400, "height": 300})).unwrap()["layer"].as_u64().unwrap();
    s.execute("mask.new", json!({"layer": l, "vertices": [[100, 100], [300, 100], [300, 200], [100, 200]], "closed": true})).unwrap();
    let m = layer(s, l).masks().unwrap().groups().next().unwrap().uid;
    (l, m)
}

fn mask_path(s: &Session, l: u64, m: u64) -> effectcraft_keyframe::ShapePath {
    let ly = layer(s, l);
    let g = ly.masks().unwrap().find_group(m).unwrap();
    g.get("path").unwrap().value.as_path().unwrap().clone()
}

fn mask_kind(s: &Session, l: u64) -> GroupKind {
    layer(s, l).masks().unwrap().groups().next().unwrap().kind.clone()
}

#[test]
fn roto_bezier_and_convert_to_bezier() {
    let mut s = comp();
    let (l, m) = masked(&mut s);
    assert!(mask_path(&s, l, m).out_tangents.iter().all(|t| *t == [0.0, 0.0]));
    s.execute("path.rotoBezier", json!({"layer": l})).unwrap();
    assert!(matches!(mask_kind(&s, l), GroupKind::Mask { roto_bezier: true, .. }));
    let p = mask_path(&s, l, m);
    // Vertex 0 at (100,100): neighbours (100,200) and (300,100) → tangent (200,-100)/6.
    assert!((p.out_tangents[0][0] - 200.0 / 6.0).abs() < 1e-9 && (p.out_tangents[0][1] + 100.0 / 6.0).abs() < 1e-9, "{:?}", p.out_tangents);
    s.execute("layer.select", json!({"layers": [l]})).unwrap();
    assert_eq!(crate::menus::checked(&s, "path.rotoBezier", &json!({})), Some(true));
    // Moving a vertex keeps the tangents automatic.
    s.execute("mask.selectVertices", json!({"vertices": [{"layer": l, "mask": m, "index": 2}]})).unwrap();
    s.execute("mask.moveVertices", json!({"delta": [50, 50]})).unwrap();
    let q = mask_path(&s, l, m);
    assert_ne!(q.out_tangents[1], p.out_tangents[1]);
    s.execute("path.convertToBezier", json!({"layer": l})).unwrap();
    assert!(matches!(mask_kind(&s, l), GroupKind::Mask { roto_bezier: false, .. }));
    assert_eq!(mask_path(&s, l, m), q, "convert keeps the tangents");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(matches!(mask_kind(&s, l), GroupKind::Mask { roto_bezier: true, .. }));
}

#[test]
fn set_first_vertex_and_free_transform() {
    let mut s = comp();
    let (l, m) = masked(&mut s);
    s.state.selected_vertices.clear();
    assert!(!s.is_enabled("path.setFirstVertex"));
    s.execute("mask.selectVertices", json!({"vertices": [{"layer": l, "mask": m, "index": 2}]})).unwrap();
    assert!(s.is_enabled("path.setFirstVertex"));
    s.execute("path.setFirstVertex", json!({})).unwrap();
    let p = mask_path(&s, l, m);
    assert_eq!(p.vertices[0], [300.0, 200.0]);
    assert_eq!(p.vertices[3], [300.0, 100.0]);
    assert_eq!(s.state.selected_vertices[0].index, 0);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(mask_path(&s, l, m).vertices[0], [100.0, 100.0]);
    // Free Transform the whole mask: scale 50 % about its centre, then move.
    s.state.selected_vertices.clear();
    s.execute("path.freeTransform", json!({"layer": l, "scale": 50, "offset": [10, 0]})).unwrap();
    let p = mask_path(&s, l, m);
    assert_eq!(p.vertices[0], [160.0, 125.0]);
    assert_eq!(p.vertices[2], [260.0, 175.0]);
    // Rotation of selected vertices only.
    s.execute("mask.selectVertices", json!({"vertices": [{"layer": l, "mask": m, "index": 0}, {"layer": l, "mask": m, "index": 1}]})).unwrap();
    s.execute("path.freeTransform", json!({"rotation": 180})).unwrap();
    let q = mask_path(&s, l, m);
    assert!((q.vertices[0][0] - 260.0).abs() < 1e-9 && (q.vertices[1][0] - 160.0).abs() < 1e-9);
    assert_eq!(q.vertices[2], p.vertices[2]);
    assert!(s.execute("path.freeTransform", json!({})).is_err());
}

#[test]
fn group_and_ungroup_shapes() {
    let mut s = comp();
    let l = s.execute("layer.newShape", json!({"kind": "rect"})).unwrap()["layer"].as_u64().unwrap();
    let contents = layer(&s, l).props.sub("contents").unwrap().clone();
    let outer = contents.groups().next().unwrap().clone();
    let inner: Vec<u64> = outer.sub("contents").unwrap().groups().map(|g| g.uid).collect();
    assert!(inner.len() >= 2, "{outer:?}");
    s.execute("layer.select", json!({"layers": [l]})).unwrap();
    s.state.selected_props = inner.iter().map(|u| (LayerId(l), *u)).collect();
    assert!(s.is_enabled("path.groupShapes"));
    let g = s.execute("path.groupShapes", json!({})).unwrap()["group"].as_u64().unwrap();
    let ly = layer(&s, l);
    let o = ly.props.sub("contents").unwrap().find_group(outer.uid).unwrap();
    let kids: Vec<&str> = o.sub("contents").unwrap().groups().map(|x| x.match_id.as_str()).collect();
    assert_eq!(kids, ["group"]);
    assert_eq!(o.find_group(g).unwrap().sub("contents").unwrap().groups().count(), inner.len());
    let before = s.render_rgba8(s.active_comp_id().unwrap(), Tick::ZERO, 200).unwrap();
    // Move the new group, then ungroup: the offset is baked into its items.
    let gp = ly.props.sub("contents").unwrap().find_group(g).unwrap().sub("transform").unwrap().get("position").unwrap().uid;
    s.edit("t", None, |proj, _| {
        let cid = proj.comps().next().map(|(i, _)| *i).unwrap();
        let l = proj.comp_mut(cid).unwrap().layer_mut(LayerId(l)).unwrap();
        l.props.find_mut(gp).unwrap().value = KV::Vec2([0.0, 0.0]);
        Ok(())
    })
    .unwrap();
    s.state.selected_props = vec![(LayerId(l), g)];
    s.execute("path.ungroupShapes", json!({})).unwrap();
    let ly = layer(&s, l);
    let o = ly.props.sub("contents").unwrap().find_group(outer.uid).unwrap();
    assert_eq!(o.sub("contents").unwrap().groups().count(), inner.len());
    assert_eq!(s.render_rgba8(s.active_comp_id().unwrap(), Tick::ZERO, 200).unwrap(), before);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(layer(&s, l).props.find_group(g).is_some());
}

#[test]
fn mask_motion_blur_feather_falloff_and_hide_locked() {
    let mut s = comp();
    let (l, _) = masked(&mut s);
    s.execute("layer.mask.set", json!({"layer": l, "field": "feather", "value": 40})).unwrap();
    let cid = s.active_comp_id().unwrap();
    let smooth = s.render_rgba8(cid, Tick::ZERO, 0).unwrap();
    s.execute("layer.mask.featherFalloff", json!({"layer": l, "mode": "linear"})).unwrap();
    assert!(matches!(mask_kind(&s, l), GroupKind::Mask { feather_falloff: FeatherFalloff::Linear, .. }));
    let linear = s.render_rgba8(cid, Tick::ZERO, 0).unwrap();
    assert_ne!(smooth, linear);
    s.execute("layer.select", json!({"layers": [l]})).unwrap();
    assert_eq!(crate::menus::checked(&s, "layer.mask.featherFalloff", &json!({"mode": "linear"})), Some(true));
    s.execute("layer.mask.motionBlur", json!({"layer": l, "mode": "on"})).unwrap();
    assert!(matches!(mask_kind(&s, l), GroupKind::Mask { motion_blur: MaskMotionBlur::On, .. }));
    assert!(s.execute("layer.mask.motionBlur", json!({"layer": l, "mode": "sometimes"})).is_err());
    // Mask motion blur blurs an animated mask path.
    s.execute("layer.mask.set", json!({"layer": l, "field": "feather", "value": 0})).unwrap();
    let still = s.render_rgba8(cid, Tick::from_seconds_f64(0.5), 0).unwrap();
    s.execute("prop.addKey", json!({"layer": l, "path": "masks/#1/path", "time": 0.0})).unwrap();
    s.execute("time.set", json!({"time": 1.0})).unwrap();
    s.execute("mask.selectVertices", json!({"vertices": []})).unwrap();
    s.execute("path.freeTransform", json!({"layer": l, "offset": [80, 0]})).unwrap();
    let blurred = s.render_rgba8(cid, Tick::from_seconds_f64(0.5), 0).unwrap();
    let soft = |img: &(u32, u32, Vec<u8>)| img.2.chunks(4).filter(|p| p[0] > 10 && p[0] < 245).count();
    assert!(soft(&blurred) > soft(&still) + 100, "{} vs {}", soft(&blurred), soft(&still));
    s.execute("layer.mask.motionBlur", json!({"layer": l, "mode": "off"})).unwrap();
    let sharp = s.render_rgba8(cid, Tick::from_seconds_f64(0.5), 0).unwrap();
    assert!(soft(&sharp) < soft(&blurred));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(matches!(mask_kind(&s, l), GroupKind::Mask { motion_blur: MaskMotionBlur::On, .. }));
    // Hide Locked Masks is a viewer toggle.
    assert!(s.execute("layer.mask.hideLocked", json!({})).unwrap().as_bool().unwrap());
    assert_eq!(crate::menus::checked(&s, "layer.mask.hideLocked", &json!({})), Some(true));
}

#[test]
fn look_at_layers_2d_and_3d() {
    let mut s = comp();
    let a = s.execute("layer.newSolid", json!({"color": "#ff0000", "width": 40, "height": 20})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setTransform", json!({"layer": a, "prop": "position", "value": [50, 60]})).unwrap();
    s.drain_events();
    s.execute("layer.select", json!({"layers": [a]})).unwrap();
    let r = s.execute("view.lookAtSelected", json!({})).unwrap();
    assert_eq!(r["rect"], json!([30.0, 50.0, 70.0, 70.0]));
    assert!(s.drain_events().iter().any(|e| matches!(e, Event::Frontend { command, .. } if command == "view.lookAt")));
    // 3D: a custom view frames the layers; the active camera view moves the camera (undoable).
    s.execute("layer.setSwitch", json!({"layer": a, "switch": "threeD", "value": true})).unwrap();
    s.execute("layer.newCamera", json!({})).unwrap();
    let cid = s.active_comp_id().unwrap();
    s.execute("view.3d.custom1", json!({})).unwrap();
    let r = s.execute("view.lookAtAll", json!({})).unwrap();
    let poi = r["poi"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
    assert!((poi[0] - 50.0).abs() < 1e-6 && (poi[1] - 60.0).abs() < 1e-6, "{r}");
    s.execute("view.3d.activeCamera", json!({})).unwrap();
    let cam = s.active_comp().unwrap().layers.iter().find(|l| l.is_camera()).unwrap().id;
    let before = s.active_comp().unwrap().layer(cam).unwrap().props.prop("transform/position").unwrap().value.clone();
    s.execute("layer.select", json!({"layers": [a]})).unwrap();
    s.execute("view.lookAtSelected", json!({})).unwrap();
    let c = s.project.comp(cid).unwrap().layer(cam).unwrap();
    assert_eq!(c.props.prop("transform/poi").unwrap().value.as_vec3()[..2], [50.0, 60.0]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.comp(cid).unwrap().layer(cam).unwrap().props.prop("transform/position").unwrap().value, before);
}

#[test]
fn camera_from_view_and_view_layout() {
    let mut s = comp();
    s.execute("layer.newSolid", json!({"color": "#ff0000"})).unwrap();
    assert!(s.execute("camera.fromView", json!({})).is_err(), "needs a 3D view");
    s.execute("view.3d.custom2", json!({})).unwrap();
    let cid = s.active_comp_id().unwrap();
    let vc = s.state.views3d[&cid].cam(effectcraft_render::three_d::View3D::Custom2, 400.0, 300.0);
    let r = s.execute("camera.fromView", json!({})).unwrap();
    let cam = layer(&s, r["layer"].as_u64().unwrap());
    assert!(cam.is_camera());
    let pos = cam.props.prop("transform/position").unwrap().value.as_vec3();
    assert!((0..3).all(|i| (pos[i] - vc.eye[i]).abs() < 1e-6), "{pos:?} {:?}", vc.eye);
    assert_eq!(s.state.views3d[&cid].current, effectcraft_render::three_d::View3D::ActiveCamera);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(!s.active_comp().unwrap().layers.iter().any(|l| l.is_camera()));
    // Switch View Layout.
    assert_eq!(s.state.view_layout, 1);
    s.execute("view.layout", json!({"views": 2})).unwrap();
    assert_eq!(s.state.view_layout, 2);
    assert_eq!(crate::menus::checked(&s, "view.layout", &json!({"views": 2})), Some(true));
    assert!(s.execute("view.layout", json!({"views": 3})).is_err());
    assert!(s.execute("view.shareViewOptions", json!({})).unwrap().as_bool().unwrap());
}

struct MockExporter {
    log: Mutex<Vec<String>>,
}

impl Exporter for MockExporter {
    fn formats(&self) -> Vec<OutputFormat> {
        OutputFormat::ALL.to_vec()
    }
    fn export(&self, job: &ExportJob, _: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
        self.log.lock().unwrap().push(format!("{:?}:{}", job.item.output.format, job.path));
        Ok(ExportResult { path: job.path.to_string(), frames: 1, ..Default::default() })
    }
}

struct MockImporter;

impl Importer for MockImporter {
    fn probe(&self, path: &str) -> Result<Footage, String> {
        Ok(Footage {
            path: path.into(),
            kind: FootageKind::Video,
            width: 400,
            height: 300,
            pixel_aspect: 1.0,
            frame_rate: FrameRate::FPS_30,
            native_rate: None,
            duration: Tick::from_seconds_f64(4.0),
            has_video: true,
            has_audio: false,
            alpha: Default::default(),
            premul_color: [0.0; 3],
            loop_count: 1,
            codec: "mock".into(),
            missing: false,
            sequence: vec![],
            color_profile: None,
        })
    }
}

fn rq() -> (Session, Arc<MockExporter>) {
    let mut s = comp();
    let ex = Arc::new(MockExporter { log: Default::default() });
    s.exporter = Some(ex.clone());
    s.importer = Some(Arc::new(MockImporter));
    (s, ex)
}

#[test]
fn add_output_module_encodes_twice() {
    let (mut s, ex) = rq();
    assert!(!s.is_enabled("render.addOutputModule"));
    s.execute("renderQueue.add", json!({"output": "/tmp/om/main.mp4"})).unwrap();
    let r = s.execute("render.addOutputModule", json!({"format": "png"})).unwrap();
    assert_eq!(r["module"], 2);
    assert_eq!(r["outputModules"], 2);
    let extra = r["extraOutputs"][0]["outputPath"].as_str().unwrap().to_string();
    assert!(extra.ends_with(".png"), "{extra}");
    // Module 2 is editable on its own.
    s.execute("renderQueue.setOutput", json!({"index": 1, "module": 2, "path": "/tmp/om/second.mov"})).unwrap();
    assert!(s.execute("renderQueue.setOutput", json!({"index": 1, "module": 3, "path": "/tmp/x.mov"})).is_err());
    s.execute("renderQueue.render", json!({})).unwrap();
    assert_eq!(*ex.log.lock().unwrap(), vec!["H264:/tmp/om/main.mp4".to_string(), "ProRes:/tmp/om/second.mov".to_string()]);
    assert_eq!(s.project.render_queue[0].status.label(), "Done");
    assert_eq!(s.project.render_queue[0].last_output.as_deref(), Some("/tmp/om/main.mp4"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.render_queue[0].extra_outputs.len(), 1, "undo the Output To edit");
}

#[test]
fn pre_render_imports_and_replaces() {
    let (mut s, ex) = rq();
    let inner = s.active_comp_id().unwrap();
    s.execute("layer.newSolid", json!({"color": "#00ff00"})).unwrap();
    s.execute("comp.new", json!({"name": "Outer", "width": 400, "height": 300, "frameRate": 30, "duration": 4})).unwrap();
    let outer = s.active_comp_id().unwrap();
    s.execute("layer.addItem", json!({"item": inner.0})).unwrap();
    s.open_comp(inner);
    let r = s.execute("render.preRender", json!({"output": "/tmp/pre/inner.mov"})).unwrap();
    assert_eq!(r["postRenderAction"], "Import & Replace Usage");
    assert_eq!(s.project.render_queue[0].post_render, PostRenderAction::ImportAndReplace);
    s.execute("renderQueue.render", json!({})).unwrap();
    assert_eq!(ex.log.lock().unwrap().len(), 1);
    let l = &s.project.comp(outer).unwrap().layers[0];
    let LayerSource::Footage { item } = l.source else { panic!("not replaced: {:?}", l.source) };
    assert!(matches!(&s.project.item(item).unwrap().kind, effectcraft_project::ItemKind::Footage(f) if f.path == "/tmp/pre/inner.mov"));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(matches!(s.project.comp(outer).unwrap().layers[0].source, LayerSource::Comp { .. }));
}

#[test]
fn save_current_preview_writes_a_movie() {
    let (mut s, ex) = rq();
    let r = s.execute("render.saveCurrentPreview", json!({"path": "/tmp/prev/clip.mov"})).unwrap();
    assert_eq!(r["path"], "/tmp/prev/clip.mov");
    assert_eq!(*ex.log.lock().unwrap(), vec!["ProRes:/tmp/prev/clip.mov".to_string()]);
    assert!(s.project.render_queue.is_empty(), "not queued");
}

#[test]
fn implemented_stubs_are_enabled_and_workspaces_are_frontend() {
    let mut s = comp();
    for id in ["view.layout", "view.lookAtAll", "camera.fromView", "render.saveCurrentPreview", "render.preRender", "layer.mask.hideLocked"] {
        assert!(s.is_enabled(id), "{id}");
    }
    for id in ["window.saveWorkspace", "window.saveWorkspaceAs", "window.editWorkspaces"] {
        s.execute(id, json!({"name": "Mine"})).unwrap();
        assert!(s.drain_events().iter().any(|e| matches!(e, Event::Frontend { command, .. } if command == id)));
    }
    // What remains a stub is still disabled.
    for id in ["view.displayColorManagement", "file.createProxy"] {
        assert!(!s.is_enabled(id), "{id}");
    }
}
