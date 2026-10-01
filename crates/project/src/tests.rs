use effectcraft_keyframe::{Keyframe, ShapePath, Value};
use effectcraft_time::{FrameRate, Tick};

use crate::build::{self, Ids};
use crate::{Comp, ItemKind, LayerSource, MaskMode, Project, Solid};

fn project_with_layer() -> (Project, crate::ItemId) {
    let mut p = Project::default();
    let comp = Comp::new(1920, 1080, FrameRate::FPS_29_97, Tick::from_seconds_f64(10.0));
    let cid = p.add_item("Main", effectcraft_color::Label::Sandstone, None, ItemKind::Comp(comp.clone().into()));
    let sid = p.add_item(
        "Red Solid",
        effectcraft_color::Label::Red,
        None,
        ItemKind::Solid(Solid { color: [1.0, 0.0, 0.0], width: 1920, height: 1080, pixel_aspect: 1.0 }),
    );
    let layer = build::layer(&mut p, &comp, "Red Solid 1", LayerSource::Solid { item: sid }, (1920, 1080), None);
    p.comp_mut(cid).unwrap().layers.push(layer);
    (p, cid)
}

#[test]
fn paths_resolve() {
    let (p, cid) = project_with_layer();
    let l = &p.comp(cid).unwrap().layers[0];
    assert_eq!(l.props.prop("transform/position").unwrap().value, Value::Vec3([960.0, 540.0, 0.0]));
    assert_eq!(l.props.prop("Transform/Opacity").unwrap().value, Value::Scalar(100.0));
    assert_eq!(l.props.prop("transform/#7").unwrap().match_id, "rotation");
    let uid = l.props.prop("transform/scale").unwrap().uid;
    let path = l.props.path_of(uid).unwrap();
    assert_eq!(l.props.prop(&path).unwrap().match_id, "scale");
    assert_eq!(l.props.name_path_of(uid).unwrap(), "Transform/Scale");
}

#[test]
fn masks_and_occurrence_paths() {
    let (mut p, cid) = project_with_layer();
    let mut next = p.next_id;
    let m1 = build::mask(&mut Ids(&mut next), "Mask 1", ShapePath::rect([0.0, 0.0], 10.0, 10.0), MaskMode::Add, [255, 0, 0]);
    let m2 = build::mask(&mut Ids(&mut next), "Mask 2", ShapePath::ellipse([0.0, 0.0], 10.0, 10.0), MaskMode::Subtract, [0, 255, 0]);
    p.next_id = next;
    let l = &mut p.comp_mut(cid).unwrap().layers[0];
    let masks = l.props.sub_mut("masks").unwrap();
    masks.children.push(m1.into());
    masks.children.push(m2.into());
    assert_eq!(l.props.group("masks/mask#2").unwrap().name, "Mask 2");
    assert_eq!(l.props.group("masks/Mask 2").unwrap().name, "Mask 2");
    assert!(l.props.prop("masks/mask#2/feather").is_some());
}

#[test]
fn animate_and_roundtrip_json() {
    let (mut p, cid) = project_with_layer();
    {
        let l = &mut p.comp_mut(cid).unwrap().layers[0];
        let op = l.props.prop_mut("transform/opacity").unwrap();
        op.set_animated(true, Tick::ZERO);
        op.set_value_at(Tick::from_seconds_f64(1.0), Value::Scalar(0.0));
        assert_eq!(op.keys.len(), 2);
        assert_eq!(op.value_at(Tick::from_seconds_f64(0.5)), Value::Scalar(50.0));
    }
    let json = p.to_json();
    let q = Project::from_json(&json).unwrap();
    assert_eq!(p, q);
    let _ = Keyframe::new(Tick::ZERO, Value::Scalar(0.0));
}

#[test]
fn unique_names_and_cycles() {
    let (p, cid) = project_with_layer();
    let c = p.comp(cid).unwrap();
    assert_eq!(c.unique_layer_name("Red Solid 1"), "Red Solid 2");
    assert_eq!(c.unique_layer_name("Shape Layer 1"), "Shape Layer 1");
    assert!(p.comp_contains(cid, cid));
}

#[test]
fn camera_and_light_layers_build() {
    let (mut p, cid) = project_with_layer();
    let comp = p.comp(cid).unwrap().clone();
    let cam = build::layer(&mut p, &comp, "Camera 1", LayerSource::Camera, (0, 0), None);
    assert!(cam.props.prop("cameraOptions/zoom").is_some());
    assert!(cam.props.prop("transform/poi").is_some());
    let light = build::layer(&mut p, &comp, "Light 1", LayerSource::Light { kind: crate::LightKind::Spot }, (0, 0), None);
    assert!(light.props.prop("lightOptions/coneAngle").is_some());
}
