use effectcraft_color::{BlendMode, Label};
use effectcraft_keyframe::{Keyframe, ShapePath, TextDoc, Value};
use effectcraft_project::build::{self, Ids};
use effectcraft_project::{Comp, ItemId, ItemKind, LayerSource, MaskMode, MatteKind, Project, Solid, TrackMatte};
use effectcraft_time::{FrameRate, Tick};

use crate::render_frame;

fn setup() -> (Project, ItemId, Comp) {
    let mut p = Project::default();
    let comp = Comp::new(200, 100, FrameRate::FPS_30, Tick::from_seconds_f64(2.0));
    let cid = p.add_item("Comp", Label::Sandstone, None, ItemKind::Comp(comp.clone().into()));
    (p, cid, comp)
}

fn solid(p: &mut Project, comp: &Comp, color: [f32; 3], w: u32, h: u32) -> effectcraft_project::Layer {
    let sid = p.add_item("Solid", Label::Red, None, ItemKind::Solid(Solid { color, width: w, height: h, pixel_aspect: 1.0 }));
    build::layer(p, comp, "Solid", LayerSource::Solid { item: sid }, (w, h), None)
}

#[test]
fn solid_fills_comp() {
    let (mut p, cid, comp) = setup();
    let l = solid(&mut p, &comp, [1.0, 0.0, 0.0], 200, 100);
    p.comp_mut(cid).unwrap().layers.push(l);
    let img = render_frame(&p, cid, Tick::ZERO, 1.0);
    assert_eq!(img.get(10, 10), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(img.get(199, 99), [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn position_keyframes_move_layer() {
    let (mut p, cid, comp) = setup();
    let mut l = solid(&mut p, &comp, [0.0, 1.0, 0.0], 20, 20);
    let pos = l.props.prop_mut("transform/position").unwrap();
    pos.keys = vec![Keyframe::new(Tick::ZERO, Value::Vec3([20.0, 50.0, 0.0])), Keyframe::new(Tick::from_seconds_f64(1.0), Value::Vec3([180.0, 50.0, 0.0]))];
    p.comp_mut(cid).unwrap().layers.push(l);
    let a = render_frame(&p, cid, Tick::ZERO, 1.0);
    assert!(a.get(20, 50)[3] > 0.99 && a.get(100, 50)[3] < 0.01);
    let b = render_frame(&p, cid, Tick::from_seconds_f64(0.5), 1.0);
    assert!(b.get(100, 50)[3] > 0.99 && b.get(20, 50)[3] < 0.01);
}

#[test]
fn opacity_and_blend_modes() {
    let (mut p, cid, comp) = setup();
    let bottom = solid(&mut p, &comp, [0.5, 0.5, 0.5], 200, 100);
    let mut top = solid(&mut p, &comp, [0.5, 0.5, 0.5], 200, 100);
    top.blend_mode = BlendMode::Multiply;
    let c = p.comp_mut(cid).unwrap();
    c.layers.push(top);
    c.layers.push(bottom);
    let img = render_frame(&p, cid, Tick::ZERO, 1.0);
    assert!((img.get(50, 50)[0] - 0.25).abs() < 1e-4);
}

#[test]
fn half_resolution() {
    let (mut p, cid, comp) = setup();
    let l = solid(&mut p, &comp, [1.0, 1.0, 1.0], 200, 100);
    p.comp_mut(cid).unwrap().layers.push(l);
    let img = render_frame(&p, cid, Tick::ZERO, 0.5);
    assert_eq!((img.width, img.height), (100, 50));
    assert!((img.get(50, 25)[3] - 1.0).abs() < 1e-4);
}

#[test]
fn mask_cuts_layer() {
    let (mut p, cid, comp) = setup();
    let mut l = solid(&mut p, &comp, [1.0, 1.0, 1.0], 200, 100);
    let mut next = p.next_id;
    let m = build::mask(&mut Ids(&mut next), "Mask 1", ShapePath::rect([50.0, 50.0], 40.0, 40.0), MaskMode::Add, [255, 255, 0]);
    p.next_id = next;
    l.props.sub_mut("masks").unwrap().children.push(m.into());
    p.comp_mut(cid).unwrap().layers.push(l);
    let img = render_frame(&p, cid, Tick::ZERO, 1.0);
    assert!(img.get(50, 50)[3] > 0.99);
    assert!(img.get(150, 50)[3] < 0.01);
}

#[test]
fn track_matte_alpha() {
    let (mut p, cid, comp) = setup();
    let mut matte = solid(&mut p, &comp, [1.0, 1.0, 1.0], 50, 50);
    matte.props.prop_mut("transform/position").unwrap().value = Value::Vec3([50.0, 50.0, 0.0]);
    matte.switches.video = false;
    let mut fill = solid(&mut p, &comp, [0.0, 0.0, 1.0], 200, 100);
    fill.track_matte = Some(TrackMatte { layer: matte.id, kind: MatteKind::Alpha });
    let c = p.comp_mut(cid).unwrap();
    c.layers.push(matte);
    c.layers.push(fill);
    let img = render_frame(&p, cid, Tick::ZERO, 1.0);
    assert!(img.get(50, 50)[2] > 0.99);
    assert!(img.get(150, 50)[3] < 0.01);
}

#[test]
fn shape_layer_draws_fill_and_stroke() {
    let (mut p, cid, comp) = setup();
    let mut l = build::layer(&mut p, &comp, "Shape Layer 1", LayerSource::Shape, (200, 100), None);
    let mut next = p.next_id;
    let mut ids = Ids(&mut next);
    let rect = build::shape_rect(&mut ids, [40.0, 40.0], [0.0, 0.0], 0.0);
    let fill = build::shape_fill(&mut ids, [1.0, 0.0, 0.0, 1.0]);
    let stroke = build::shape_stroke(&mut ids, [1.0, 1.0, 1.0, 1.0], 4.0);
    let g = build::shape_group(&mut ids, "Rectangle 1", vec![rect, stroke, fill]);
    p.next_id = next;
    l.props.sub_mut("contents").unwrap().children.push(g.into());
    p.comp_mut(cid).unwrap().layers.push(l);
    let img = render_frame(&p, cid, Tick::ZERO, 1.0);
    let centre = img.get(100, 50);
    assert!(centre[0] > 0.99 && centre[1] < 0.01, "{centre:?}");
    let edge = img.get(80, 50);
    assert!(edge[1] > 0.5, "stroke on top {edge:?}");
}

#[test]
fn text_layer_renders_glyphs() {
    let (mut p, cid, comp) = setup();
    let mut l = build::layer(&mut p, &comp, "Text", LayerSource::Text, (200, 100), None);
    l.props.prop_mut("text/sourceText").unwrap().value = Value::Text(Box::new(TextDoc { text: "HI".into(), size: 60.0, ..Default::default() }));
    l.props.prop_mut("transform/position").unwrap().value = Value::Vec3([60.0, 80.0, 0.0]);
    p.comp_mut(cid).unwrap().layers.push(l);
    let img = render_frame(&p, cid, Tick::ZERO, 1.0);
    let covered = img.data.iter().filter(|px| px[3] > 0.5).count();
    assert!(covered > 300, "{covered}");
}

#[test]
fn precomp_and_3d_render() {
    let (mut p, cid, comp) = setup();
    let inner = Comp::new(100, 100, FrameRate::FPS_30, Tick::from_seconds_f64(2.0));
    let iid = p.add_item("Inner", Label::Sandstone, None, ItemKind::Comp(inner.clone().into()));
    let s = solid(&mut p, &inner, [1.0, 0.5, 0.0], 100, 100);
    p.comp_mut(iid).unwrap().layers.push(s);
    let mut pre = build::layer(&mut p, &comp, "Inner", LayerSource::Comp { item: iid }, (100, 100), None);
    pre.switches.three_d = true;
    p.comp_mut(cid).unwrap().layers.push(pre);
    let img = render_frame(&p, cid, Tick::ZERO, 1.0);
    // A 3D layer at z = 0 with the default camera looks exactly like 2D.
    let c = img.get(100, 50);
    assert!((c[0] - 1.0).abs() < 1e-3 && (c[1] - 0.5).abs() < 1e-3, "{c:?}");
    assert!(img.get(10, 50)[3] < 0.01);
}

#[test]
fn effects_run_in_pipeline() {
    let (mut p, cid, comp) = setup();
    let mut l = solid(&mut p, &comp, [1.0, 1.0, 1.0], 50, 50);
    let spec = effectcraft_effects::find("ec.color.tint").unwrap();
    let mut next = p.next_id;
    let mut g = effectcraft_effects::instantiate(spec, &mut Ids(&mut next), "Tint", [50.0, 50.0]);
    p.next_id = next;
    if let Some(pr) = g.prop_mut("white") {
        pr.value = Value::Color([0.0, 1.0, 0.0, 1.0]);
    }
    l.props.sub_mut("effects").unwrap().children.push(g.into());
    p.comp_mut(cid).unwrap().layers.push(l);
    let img = render_frame(&p, cid, Tick::ZERO, 1.0);
    let c = img.get(100, 50);
    assert!(c[1] > 0.99 && c[0] < 0.01, "{c:?}");
}
