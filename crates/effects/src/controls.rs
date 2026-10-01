//! Expression Controls: parameter-only effects whose values expressions read. They pass pixels
//! through unchanged.

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;

use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn passthrough(_: &EffectCtx, b: Buf) -> Buf {
    b
}

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>) -> EffectSpec {
    EffectSpec { id, name, category: "Expression Controls", params, render: passthrough, gpu: true, float: true }
}

pub fn specs() -> Vec<EffectSpec> {
    vec![
        spec("ec.control.slider", "Slider Control", vec![p("slider", "Slider", num(0.0), slider(-1_000_000.0, 1_000_000.0, 0.0, 100.0, 2))]),
        spec("ec.control.angle", "Angle Control", vec![p("angle", "Angle", num(0.0), ParamUi::Angle)]),
        spec("ec.control.checkbox", "Checkbox Control", vec![p("checkbox", "Checkbox", Value::Bool(false), ParamUi::Checkbox)]),
        spec("ec.control.color", "Color Control", vec![p("color", "Color", col(1.0, 0.0, 0.0), ParamUi::Color)]),
        spec("ec.control.point", "Point Control", vec![p("point", "Point", Value::Vec2([0.5, 0.5]), ParamUi::Point)]),
        spec("ec.control.point3d", "3D Point Control", vec![p("point", "3D Point", Value::Vec3([0.0, 0.0, 0.0]), ParamUi::Point3)]),
        spec("ec.control.layer", "Layer Control", vec![p("layer", "Layer", Value::Layer(None), ParamUi::Layer)]),
        spec("ec.control.dropdown", "Dropdown Menu Control", vec![p("menu", "Menu", Value::Enum(0), popup(&["Item 1", "Item 2", "Item 3"]))]),
    ]
}
