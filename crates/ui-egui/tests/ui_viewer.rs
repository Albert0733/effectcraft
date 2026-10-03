//! Headless checks for the Composition viewer's interaction parity: bottom bar order, region of
//! interest drawing, rulers and guides, snapping, the shape Pen and motion-path key drags
//! (egui_kittest, UI logic only).

use effectcraft_engine::Session;
use effectcraft_engine::project::LayerId;
use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::state::Tool;
use egui::{Event, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn app() -> EffectcraftApp {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "View", "width": 640, "height": 360, "duration": 4})).unwrap();
    s.execute("layer.newSolid", json!({"name": "Plate", "color": "#406080", "width": 640, "height": 360})).unwrap();
    s.execute("layer.newSolid", json!({"name": "Box", "color": "#e04020", "width": 80, "height": 80})).unwrap();
    s.execute("edit.deselectAll", json!({})).unwrap();
    EffectcraftApp::new(s)
}

fn harness() -> Harness<'static, EffectcraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| app());
    h.run_steps(3);
    h
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}"));
    Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]))
}

fn click(h: &mut Harness<'_, EffectcraftApp>, p: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(from));
    h.input_mut().events.push(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for i in 1..=8 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

/// Comp pixel → screen point.
fn screen(h: &Harness<'_, EffectcraftApp>, p: [f32; 2]) -> Pos2 {
    let c = rect(h, "viewer.comp");
    let z = c.width() / 640.0;
    c.min + vec2(p[0] * z, p[1] * z)
}

#[test]
fn bottom_bar_in_after_effects_order() {
    let h = harness();
    let order = [
        "viewer.magnification",
        "viewer.resolution",
        "viewer.transparency",
        "viewer.masks",
        "viewer.roi",
        "viewer.grid",
        "viewer.channel",
        "viewer.resetExposure",
        "viewer.exposure",
        "viewer.snapshot",
        "viewer.showSnapshot",
        "viewer.fastPreviews",
        "viewer.timecode",
    ];
    let xs: Vec<f32> = order.iter().map(|id| rect(&h, id).min.x).collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
    let mag = &h.state().auto.find("viewer.magnification").unwrap().label;
    assert_eq!(mag, "Magnification");
}

#[test]
fn region_of_interest_is_drawn_in_the_viewer() {
    let mut h = harness();
    let at = rect(&h, "viewer.roi").center();
    click(&mut h, at);
    assert!(h.state().ui.viewer.roi_draw);
    let (a, b) = (screen(&h, [100.0, 50.0]), screen(&h, [300.0, 250.0]));
    drag(&mut h, a, b);
    let r = h.state().session.state.region_of_interest.expect("roi");
    assert!((r[0] - 100.0).abs() <= 2.0 && (r[1] - 50.0).abs() <= 2.0 && (r[2] - 200.0).abs() <= 3.0 && (r[3] - 200.0).abs() <= 3.0, "{r:?}");
    assert!(!h.state().ui.viewer.roi_draw);
    h.run_steps(3);
    assert!(h.state().auto.find("viewer.regionOfInterest").is_some());
    // The button clears it again.
    let at = rect(&h, "viewer.roi").center();
    click(&mut h, at);
    assert!(h.state().session.state.region_of_interest.is_none());
}

#[test]
fn rulers_make_guides_and_guides_move() {
    let mut h = harness();
    h.state_mut().ui.viewer.rulers = true;
    h.run_steps(3);
    let top = rect(&h, "viewer.ruler.top");
    let to = screen(&h, [0.0, 120.0]);
    drag(&mut h, pos2(to.x + 200.0, top.center().y), pos2(to.x + 200.0, to.y));
    let g = h.state().session.active_comp().unwrap().guides.clone();
    assert_eq!(g.len(), 1);
    assert!(!g[0].vertical && (g[0].position - 120.0).abs() <= 2.0, "{g:?}");
    // Drag the guide down with the Selection tool.
    let from = pos2(screen(&h, [500.0, 0.0]).x, screen(&h, [0.0, g[0].position as f32]).y);
    let to2 = pos2(from.x, screen(&h, [0.0, 200.0]).y);
    drag(&mut h, from, to2);
    let g = h.state().session.active_comp().unwrap().guides.clone();
    assert!((g[0].position - 200.0).abs() <= 2.0, "{g:?}");
}

#[test]
fn layer_drag_snaps_to_comp_centre_and_ctrl_disables() {
    let mut h = harness();
    let box_id = h.state().session.active_comp().unwrap().layers[0].id;
    h.state_mut().session.execute("prop.set", json!({"layer": box_id.0, "path": "transform/position", "value": [100, 100, 0]})).unwrap();
    h.run_steps(2);
    // Grab the box at its centre and drop it 3 px from the comp centre: it snaps there.
    let from = screen(&h, [100.0, 100.0]);
    let to = screen(&h, [323.0, 182.0]);
    drag(&mut h, from, to);
    let pos = |h: &Harness<'_, EffectcraftApp>| {
        let l = h.state().session.active_comp().unwrap().layer(box_id).unwrap().clone();
        l.props.prop("transform/position").unwrap().value.as_vec3()
    };
    let p = pos(&h);
    assert!((p[0] - 320.0).abs() < 0.01 && (p[1] - 180.0).abs() < 0.01, "{p:?}");
    // Snapping off: the same drag lands where the pointer is.
    h.state_mut().session.execute("view.snapping", json!({"value": false})).unwrap();
    h.state_mut().session.execute("prop.set", json!({"layer": box_id.0, "path": "transform/position", "value": [100, 100, 0]})).unwrap();
    h.run_steps(2);
    drag(&mut h, from, to);
    let p = pos(&h);
    assert!((p[0] - 320.0).abs() > 1.0, "{p:?}");
}

#[test]
fn shape_pen_draws_a_closed_shape_layer() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    h.run_steps(2);
    let n0 = h.state().session.active_comp().unwrap().layers.len();
    for p in [[200.0, 100.0], [400.0, 120.0], [300.0, 260.0], [200.0, 100.0]] {
        let at = screen(&h, p);
        click(&mut h, at);
    }
    let comp = h.state().session.active_comp().unwrap().clone();
    assert_eq!(comp.layers.len(), n0 + 1);
    let l = &comp.layers[0];
    assert!(matches!(l.source, effectcraft_engine::project::LayerSource::Shape));
    let g = l.props.sub("contents").unwrap().groups().next().unwrap();
    assert_eq!(g.name, "Shape 1");
    let path = g.sub("contents").unwrap().groups().next().unwrap();
    let sp = path.get("path").unwrap().value.as_path().unwrap().clone();
    assert_eq!(sp.vertices.len(), 3);
    assert!(sp.closed);
    assert!((sp.vertices[0][0] - (200.0 - 320.0)).abs() < 1.5, "{:?}", sp.vertices);
    // G cycles the Pen slot: Add Vertex, Delete Vertex, Convert Vertex, Mask Feather, Pen.
    let ctx = h.ctx.clone();
    let mut seen = vec![];
    for _ in 0..5 {
        effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "tool.pen", json!({})).unwrap();
        seen.push(h.state().ui.tool);
    }
    assert_eq!(seen, vec![Tool::PenAdd, Tool::PenDelete, Tool::PenConvert, Tool::MaskFeather, Tool::Pen]);
    // Add Vertex on the first segment.
    h.state_mut().ui.tool = Tool::PenAdd;
    h.run_steps(2);
    let at = screen(&h, [300.0, 110.0]);
    click(&mut h, at);
    let comp = h.state().session.active_comp().unwrap().clone();
    let sp = comp.layers[0]
        .props
        .sub("contents")
        .unwrap()
        .groups()
        .next()
        .unwrap()
        .sub("contents")
        .unwrap()
        .groups()
        .next()
        .unwrap()
        .get("path")
        .unwrap()
        .value
        .as_path()
        .unwrap()
        .clone();
    assert_eq!(sp.vertices.len(), 4);
}

#[test]
fn motion_path_key_drag_edits_that_key() {
    let mut h = harness();
    let box_id: LayerId = h.state().session.active_comp().unwrap().layers[0].id;
    let s = &mut h.state_mut().session;
    s.execute("layer.select", json!({"layers": [box_id.0]})).unwrap();
    s.execute("prop.addKey", json!({"layer": box_id.0, "path": "transform/position", "time": 0.0, "value": [100, 100, 0]})).unwrap();
    s.execute("prop.addKey", json!({"layer": box_id.0, "path": "transform/position", "time": 2.0, "value": [500, 100, 0]})).unwrap();
    s.execute("view.snapping", json!({"value": false})).unwrap();
    s.set_time(effectcraft_engine::time::Tick::from_seconds_f64(1.0));
    h.run_steps(3);
    assert!(h.state().auto.find(&format!("viewer.motionPath.{}.1", box_id.0)).is_some());
    let from = screen(&h, [500.0, 100.0]);
    let to2 = screen(&h, [500.0, 300.0]);
    drag(&mut h, from, to2);
    let l = h.state().session.active_comp().unwrap().layer(box_id).unwrap().clone();
    let k = &l.props.prop("transform/position").unwrap().keys;
    let v1 = k[1].value.as_vec3();
    assert!((v1[1] - 300.0).abs() < 2.0 && (v1[0] - 500.0).abs() < 2.0, "{v1:?}");
    assert_eq!(k[0].value.as_vec3(), [100.0, 100.0, 0.0]);
}

#[test]
fn graph_editor_transform_box_scales_selected_keys_in_time() {
    let mut h = harness();
    let box_id: LayerId = h.state().session.active_comp().unwrap().layers[0].id;
    let s = &mut h.state_mut().session;
    s.execute("layer.select", json!({"layers": [box_id.0]})).unwrap();
    for (t, v) in [(0.0, 0.0), (1.0, 50.0), (2.0, 100.0)] {
        s.execute("prop.addKey", json!({"layer": box_id.0, "path": "transform/opacity", "time": t, "value": v})).unwrap();
    }
    let uid = s.active_comp().unwrap().layer(box_id).unwrap().props.prop("transform/opacity").unwrap().uid;
    s.execute("prop.select", json!({"layer": box_id.0, "prop": uid})).unwrap();
    let keys: Vec<_> = [0.0, 1.0, 2.0].iter().map(|t| json!({"layer": box_id.0, "prop": uid, "time": t})).collect();
    s.execute("keys.select", json!({"keys": keys})).unwrap();
    h.state_mut().ui.timeline.graph_editor = true;
    h.run_steps(4);
    for id in ["timeline.graph.snap", "timeline.graph.reference", "timeline.graph.transformBox", "timeline.graph.transformBox.5"] {
        assert!(h.state().auto.find(id).is_some(), "missing {id}");
    }
    // Drag the right edge handle to the right: the keys spread out in time, the first stays.
    let r = rect(&h, "timeline.graph.transformBox.5");
    let k0 = rect(&h, &format!("timeline.graph.key.{uid}.0.0")).center();
    let k2 = rect(&h, &format!("timeline.graph.key.{uid}.0.2")).center();
    let to = r.center() + vec2((k2.x - k0.x) * 0.5, 0.0);
    drag(&mut h, r.center(), to);
    let p = h.state().session.active_comp().unwrap().layer(box_id).unwrap().props.prop("transform/opacity").unwrap().clone();
    let ts: Vec<f64> = p.keys.iter().map(|k| k.time.seconds()).collect();
    assert_eq!(ts[0], 0.0);
    assert!(ts[2] > 2.5 && ts[2] < 3.5, "{ts:?}");
    assert_eq!(p.keys.iter().map(|k| k.value.as_f64()).collect::<Vec<_>>(), vec![0.0, 50.0, 100.0]);
    let undo = h.state().session.history.undo.iter().filter(|(l, _)| l == "Transform Keyframes").count();
    assert_eq!(undo, 1, "one undo step per drag");
}

#[test]
fn timeline_alt_drag_scales_a_key_group_in_time() {
    let mut h = harness();
    let box_id: LayerId = h.state().session.active_comp().unwrap().layers[0].id;
    let s = &mut h.state_mut().session;
    s.execute("layer.select", json!({"layers": [box_id.0]})).unwrap();
    for (t, v) in [(0.0, 0.0), (1.0, 50.0), (2.0, 100.0)] {
        s.execute("prop.addKey", json!({"layer": box_id.0, "path": "transform/opacity", "time": t, "value": v})).unwrap();
    }
    let uid = s.active_comp().unwrap().layer(box_id).unwrap().props.prop("transform/opacity").unwrap().uid;
    s.execute("keys.selectAll", json!({"layers": [box_id.0]})).unwrap();
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.reveal.opacity", json!({})).unwrap();
    h.run_steps(4);
    let mut ks: Vec<Pos2> =
        h.state().auto.query(&format!("timeline.key.{uid}.")).iter().map(|e| pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)).collect();
    ks.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert_eq!(ks.len(), 3, "{ks:?}");
    let (k0, k2) = (ks[0], ks[2]);
    // Alt-drag the last key to where 3 s would be: the group scales by 1.5 about the first key.
    let to = pos2(k0.x + (k2.x - k0.x) * 1.5, k2.y);
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    h.input_mut().events.push(Event::PointerMoved(k2));
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(alt));
    h.input_mut().events.push(Event::PointerButton { pos: k2, button: egui::PointerButton::Primary, pressed: true, modifiers: alt });
    h.step();
    for i in 1..=8 {
        h.input_mut().events.push(Event::PointerMoved(k2 + (to - k2) * (i as f32 / 8.0)));
        h.step();
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: alt });
    h.run_steps(2);
    let p = h.state().session.active_comp().unwrap().layer(box_id).unwrap().props.prop("transform/opacity").unwrap().clone();
    let ts: Vec<f64> = p.keys.iter().map(|k| k.time.seconds()).collect();
    assert_eq!(ts.len(), 3);
    assert_eq!(ts[0], 0.0);
    assert!((ts[2] - 3.0).abs() < 0.1 && (ts[1] - 1.5).abs() < 0.1, "{ts:?}");
}

#[test]
fn mask_feather_tool_adds_and_drags_feather_points() {
    let mut h = harness();
    // A mask on the Box (80×80 at the comp centre, comp 280…360 × 140…220): layer 10…70.
    let (box_id, mask) = {
        let s = &mut h.state_mut().session;
        let b = s.active_comp().unwrap().layers.iter().find(|l| l.name == "Box").unwrap().id;
        let m = s.execute("mask.new", json!({"layer": b.0, "vertices": [[10, 10], [70, 10], [70, 70], [10, 70]], "closed": true})).unwrap()["mask"]
            .as_u64()
            .unwrap();
        s.execute("layer.select", json!({"layers": [b.0]})).unwrap();
        (b, m)
    };
    h.state_mut().ui.tool = Tool::MaskFeather;
    h.run_steps(3);
    // Press on the top edge (comp 320,150) and drag 12 px up: an outer feather point of ≈12 px.
    let from = screen(&h, [320.0, 150.0]);
    let z = rect(&h, "viewer.comp").width() / 640.0;
    drag(&mut h, from, from - vec2(0.0, 12.0 * z));
    let pts = h.state_mut().session.execute("mask.featherPoint.list", json!({"layer": box_id.0, "mask": mask})).unwrap();
    let r = pts["points"][0]["radius"].as_f64().unwrap();
    assert!((r - 12.0).abs() < 1.5, "{pts}");
    assert_eq!(pts["points"][0]["segment"], json!(0));
    // The handle is registered; dragging it inwards makes it an inner feather point.
    h.run_steps(2);
    let handle = rect(&h, &format!("viewer.mask.{mask}.feather.0")).center();
    let to = screen(&h, [320.0, 160.0]);
    drag(&mut h, handle, to);
    let pts = h.state_mut().session.execute("mask.featherPoint.list", json!({"layer": box_id.0, "mask": mask})).unwrap();
    let r = pts["points"][0]["radius"].as_f64().unwrap();
    assert!((r + 10.0).abs() < 1.5, "{pts}");
    assert_eq!(pts["points"].as_array().unwrap().len(), 1);
    // One drag = one undo step each.
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    let pts = h.state_mut().session.execute("mask.featherPoint.list", json!({"layer": box_id.0, "mask": mask})).unwrap();
    assert!(pts["points"][0]["radius"].as_f64().unwrap() > 0.0);
}

#[test]
fn region_of_interest_resizes_by_its_handles() {
    let mut h = harness();
    h.state_mut().session.execute("view.setRegionOfInterest", json!({"rect": [100, 50, 200, 200]})).unwrap();
    h.run_steps(3);
    // Bottom-right corner (handle 2) to (400, 300): the top-left stays.
    let c = rect(&h, "viewer.regionOfInterest.handle.2").center();
    let to = screen(&h, [400.0, 300.0]);
    drag(&mut h, c, to);
    let r = h.state().session.state.region_of_interest.expect("roi");
    assert!((r[0] - 100.0).abs() <= 1.0 && (r[1] - 50.0).abs() <= 1.0 && (r[2] - 300.0).abs() <= 2.0 && (r[3] - 250.0).abs() <= 2.0, "{r:?}");
    // The left edge (handle 7) only moves x.
    h.run_steps(2);
    let c = rect(&h, "viewer.regionOfInterest.handle.7").center();
    let to = pos2(screen(&h, [150.0, 0.0]).x, c.y + 40.0);
    drag(&mut h, c, to);
    let r = h.state().session.state.region_of_interest.expect("roi");
    assert!((r[0] - 150.0).abs() <= 1.0 && (r[1] - 50.0).abs() <= 1.0 && (r[2] - 250.0).abs() <= 2.0 && (r[3] - 250.0).abs() <= 2.0, "{r:?}");
}

#[test]
fn pan_behind_snaps_the_anchor_point() {
    let mut h = harness();
    let box_id = h.state().session.active_comp().unwrap().layers[0].id;
    h.state_mut().session.execute("prop.set", json!({"layer": box_id.0, "path": "transform/position", "value": [100, 100, 0]})).unwrap();
    h.state_mut().session.execute("layer.select", json!({"layers": [box_id.0]})).unwrap();
    h.state_mut().ui.tool = Tool::PanBehind;
    h.run_steps(2);
    // Drag the anchor (at the box centre) to 3 px from the comp centre: it snaps there, and the
    // position follows so the box doesn't move.
    let from = screen(&h, [100.0, 100.0]);
    let to = screen(&h, [323.0, 182.0]);
    // (The gesture applies the pointer of the previous frame: hold the end point a frame.)
    h.input_mut().events.push(Event::PointerMoved(from));
    h.input_mut().events.push(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for i in 1..=10 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i.min(8) as f32 / 8.0)));
        h.step();
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    let l = h.state().session.active_comp().unwrap().layer(box_id).unwrap().clone();
    let p = l.props.prop("transform/position").unwrap().value.as_vec3();
    let a = l.props.prop("transform/anchor").unwrap().value.as_vec3();
    assert!((p[0] - 320.0).abs() < 0.01 && (p[1] - 180.0).abs() < 0.01, "anchor point snapped to the comp centre: {p:?}");
    assert!((a[0] - 260.0).abs() < 0.01 && (a[1] - 120.0).abs() < 0.01, "{a:?}");
}

#[test]
fn reference_axes_toggle_from_the_grid_menu() {
    let mut h = harness();
    // A 3D layer makes the axes relevant.
    let box_id = h.state().session.active_comp().unwrap().layers[0].id;
    h.state_mut().session.execute("layer.setSwitch", json!({"layers": [box_id.0], "switch": "threeD", "value": true})).unwrap();
    h.run_steps(3);
    assert!(h.state().auto.find("viewer.referenceAxes").is_some(), "on by default (Settings ▸ 3D)");
    let g = rect(&h, "viewer.grid").center();
    click(&mut h, g);
    h.run_steps(2);
    let item = rect(&h, "viewer.gridItem.6").center();
    click(&mut h, item);
    h.run_steps(3);
    assert!(!h.state().session.prefs.three_d.show_reference_axes);
    assert!(h.state().auto.find("viewer.referenceAxes").is_none());
}
