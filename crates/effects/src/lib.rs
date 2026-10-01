//! The effects library.
//!
//! Every effect is an [`EffectSpec`]: a stable id (`ec.blur.gaussian`), its display name and
//! category (the Effects & Presets tree), typed parameters with defaults and UI hints, and a CPU
//! render function. Effect instances on layers are ordinary property groups (see
//! [`instantiate`]), so parameters animate, take expressions and are addressable like any other
//! property.

mod blur2;
mod channel;
mod color2;
mod color_fx;
mod controls;
mod distort;
mod distort2;
mod generate;
mod generate2;
mod keying;
mod matte;
mod misc;
mod noise;
mod perspective;
mod stylize2;
mod transition;
pub mod util;
mod utility;

use std::collections::HashMap;
use std::sync::OnceLock;

use effectcraft_keyframe::Value;
use effectcraft_project::build::Ids;
use effectcraft_project::{GroupKind, ParamUi, PropGroup, Property};
pub use effectcraft_raster::Image;

/// Effect categories in Effects & Presets order.
pub const CATEGORIES: &[&str] = &[
    "3D Channel",
    "Audio",
    "Blur & Sharpen",
    "Channel",
    "Color Correction",
    "Distort",
    "Expression Controls",
    "Generate",
    "Immersive Video",
    "Keying",
    "Matte",
    "Noise & Grain",
    "Obsolete",
    "Perspective",
    "Simulation",
    "Stylize",
    "Text",
    "Time",
    "Transition",
    "Utility",
];

pub struct ParamSpec {
    /// Point defaults are fractions of the layer size (0.5, 0.5 = layer centre).
    pub id: &'static str,
    pub name: &'static str,
    pub default: Value,
    pub ui: ParamUi,
}

/// Parameter values evaluated at the current time.
#[derive(Clone, Debug, Default)]
pub struct Params {
    pub values: HashMap<String, Value>,
}

impl Params {
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.values.get(id)
    }
    pub fn f(&self, id: &str) -> f64 {
        self.values.get(id).map(Value::as_f64).unwrap_or(0.0)
    }
    pub fn v2(&self, id: &str) -> [f64; 2] {
        self.values.get(id).map(Value::as_vec2).unwrap_or([0.0; 2])
    }
    pub fn color(&self, id: &str) -> [f32; 4] {
        self.values.get(id).map(Value::as_color).unwrap_or([1.0; 4])
    }
    pub fn b(&self, id: &str) -> bool {
        self.values.get(id).map(Value::as_bool).unwrap_or(false)
    }
    pub fn e(&self, id: &str) -> u32 {
        self.values.get(id).map(Value::as_enum).unwrap_or(0)
    }
    /// String parameter (`Value::Str`), empty when missing or of another kind.
    pub fn s(&self, id: &str) -> &str {
        match self.values.get(id) {
            Some(Value::Str(s)) => s,
            _ => "",
        }
    }
}

/// A layer image in flight: layer-space point `p` sits at pixel `p * scale + offset`.
#[derive(Clone, Debug, Default)]
pub struct Buf {
    pub img: Image,
    pub offset: [f64; 2],
    pub scale: f64,
}

impl Buf {
    pub fn to_px(&self, p: [f64; 2]) -> (f64, f64) {
        (p[0] * self.scale + self.offset[0], p[1] * self.scale + self.offset[1])
    }
    /// Grow by `pad` transparent pixels on every side.
    pub fn pad(&mut self, pad: u32) {
        if pad == 0 {
            return;
        }
        self.img = self.img.padded(pad);
        self.offset[0] += pad as f64;
        self.offset[1] += pad as f64;
    }
}

/// What an effect gets to render with.
pub struct EffectCtx<'a> {
    pub params: &'a Params,
    /// Layer time in seconds.
    pub time: f64,
    /// Layer source size in layer pixels (the effect's "layer bounds").
    pub layer_size: [f64; 2],
    /// Instance seed (stable per effect instance).
    pub seed: u32,
    /// Rendering for an adjustment layer (operating on the comp below).
    pub adjustment: bool,
}

pub type RenderFn = fn(&EffectCtx, Buf) -> Buf;

pub struct EffectSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub params: Vec<ParamSpec>,
    pub render: RenderFn,
    /// Implemented on the GPU path too (shown with a badge).
    pub gpu: bool,
    /// Supports 32 bpc float (badge).
    pub float: bool,
}

pub(crate) fn p(id: &'static str, name: &'static str, default: Value, ui: ParamUi) -> ParamSpec {
    ParamSpec { id, name, default, ui }
}
pub(crate) fn slider(min: f64, max: f64, smin: f64, smax: f64, decimals: u8) -> ParamUi {
    ParamUi::Slider { min, max, slider_min: smin, slider_max: smax, decimals }
}
pub(crate) fn popup(opts: &[&str]) -> ParamUi {
    ParamUi::Popup { options: opts.iter().map(|s| s.to_string()).collect() }
}
pub(crate) fn num(v: f64) -> Value {
    Value::Scalar(v)
}
pub(crate) fn col(r: f64, g: f64, b: f64) -> Value {
    Value::Color([r, g, b, 1.0])
}

/// All effects, sorted by category then name.
pub fn registry() -> &'static [EffectSpec] {
    static R: OnceLock<Vec<EffectSpec>> = OnceLock::new();
    R.get_or_init(|| {
        let mut v = Vec::new();
        v.extend(misc::specs());
        v.extend(color_fx::specs());
        v.extend(generate::specs());
        v.extend(distort::specs());
        v.extend(controls::specs());
        v.extend(keying::specs());
        v.extend(matte::specs());
        v.extend(channel::specs());
        v.extend(blur2::specs());
        v.extend(color2::specs());
        v.extend(stylize2::specs());
        v.extend(noise::specs());
        v.extend(generate2::specs());
        v.extend(distort2::specs());
        v.extend(perspective::specs());
        v.extend(transition::specs());
        v.extend(utility::specs());
        v.sort_by(|a, b| a.category.cmp(b.category).then(a.name.cmp(b.name)));
        v
    })
}

pub fn find(id: &str) -> Option<&'static EffectSpec> {
    registry().iter().find(|s| s.id == id)
}

/// Find by id or (case-insensitive) display name.
pub fn lookup(name_or_id: &str) -> Option<&'static EffectSpec> {
    find(name_or_id).or_else(|| registry().iter().find(|s| s.name.eq_ignore_ascii_case(name_or_id)))
}

/// Build the property group for a new instance of an effect.
pub fn instantiate(spec: &EffectSpec, ids: &mut Ids, instance_name: &str, layer_size: [f64; 2]) -> PropGroup {
    let mut g = ids.group(spec.id, instance_name);
    g.kind = GroupKind::Effect { effect: spec.id.to_string() };
    for ps in &spec.params {
        let default = match (&ps.ui, &ps.default) {
            (ParamUi::Point, Value::Vec2(f)) => Value::Vec2([f[0] * layer_size[0], f[1] * layer_size[1]]),
            _ => ps.default.clone(),
        };
        let mut pr = Property::new(ids.alloc(), ps.id, ps.name, default).with_ui(ps.ui.clone());
        if matches!(ps.ui, ParamUi::Point) {
            pr.spatial = true;
        }
        if matches!(ps.ui, ParamUi::Checkbox | ParamUi::Popup { .. } | ParamUi::Layer) {
            pr.hold_only = true;
        }
        g.children.push(pr.into());
    }
    g
}

/// Run an effect on a buffer.
pub fn apply(spec: &EffectSpec, ctx: &EffectCtx, buf: Buf) -> Buf {
    (spec.render)(ctx, buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_consistent() {
        let r = registry();
        assert!(r.len() >= 40, "{}", r.len());
        let mut ids = std::collections::HashSet::new();
        for s in r {
            assert!(ids.insert(s.id), "duplicate {}", s.id);
            assert!(CATEGORIES.contains(&s.category), "{} in unknown category {}", s.id, s.category);
            let mut pids = std::collections::HashSet::new();
            for p in &s.params {
                assert!(pids.insert(p.id), "{}: duplicate param {}", s.id, p.id);
            }
        }
    }

    /// Every effect runs on a small image with its default parameters without panicking and
    /// keeps alpha in range.
    #[test]
    fn all_effects_run_with_defaults() {
        let mut img = Image::new(48, 32);
        for y in 0..32 {
            for x in 0..48 {
                let a = if (8..40).contains(&x) && (6..26).contains(&y) { 1.0 } else { 0.0 };
                img.set(x, y, [x as f32 / 48.0 * a, y as f32 / 32.0 * a, 0.5 * a, a]);
            }
        }
        for s in registry() {
            let params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
            let ctx = EffectCtx { params: &params, time: 0.5, layer_size: [48.0, 32.0], seed: 1, adjustment: false };
            let out = apply(s, &ctx, Buf { img: img.clone(), offset: [0.0, 0.0], scale: 1.0 });
            assert!(!out.img.is_empty(), "{}", s.id);
            assert!(out.img.data.iter().all(|p| p[3].is_finite() && p[3] >= -1e-4 && p[3] <= 1.0001), "{} alpha out of range", s.id);
        }
    }
}
