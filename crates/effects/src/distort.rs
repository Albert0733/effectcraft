//! Distort effects: inverse-mapped warps (each output pixel samples the source).

use effectcraft_geom::{Mat3, vec2};
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::Image;
use rayon::prelude::*;

use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Distort", params, render, gpu: false, float: true }
}

/// Resample through `f(x, y) -> source point` (pixel coordinates, centres at +0.5).
fn remap(src: &Image, repeat: bool, f: impl Fn(f64, f64) -> (f64, f64) + Sync) -> Image {
    let mut out = Image::new(src.width, src.height);
    out.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (sx, sy) = f(x as f64 + 0.5, y as f64 + 0.5);
            *px = if repeat { src.sample_bilinear_clamped(sx, sy) } else { src.sample_bilinear(sx, sy) };
        }
    });
    out
}

fn twirl(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let ang = ctx.params.f("angle").to_radians();
    let r = (ctx.params.f("radius") / 100.0 * b.img.width.min(b.img.height) as f64 * 0.5).max(1.0);
    let c = b.to_px(ctx.params.v2("center"));
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = (dx * dx + dy * dy).sqrt();
        if d >= r {
            return (x, y);
        }
        let t = 1.0 - d / r;
        let a = -ang * t * t;
        let (s, co) = a.sin_cos();
        (c.0 + dx * co - dy * s, c.1 + dx * s + dy * co)
    });
    b
}

fn spherize(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let r = (ctx.params.f("radius") * b.scale).max(1.0);
    let c = b.to_px(ctx.params.v2("center"));
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = (dx * dx + dy * dy).sqrt();
        if d >= r || d == 0.0 {
            return (x, y);
        }
        let nd = d / r;
        let k = (nd.asin() / std::f64::consts::FRAC_PI_2) / nd;
        (c.0 + dx * k, c.1 + dy * k)
    });
    b
}

fn bulge(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let rx = (ctx.params.f("hradius") * b.scale).max(1.0);
    let ry = (ctx.params.f("vradius") * b.scale).max(1.0);
    let c = b.to_px(ctx.params.v2("center"));
    let h = ctx.params.f("height");
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = ((x - c.0) / rx, (y - c.1) / ry);
        let d2 = dx * dx + dy * dy;
        if d2 >= 1.0 {
            return (x, y);
        }
        let k = 1.0 - h * (1.0 - d2) * 0.5;
        (c.0 + dx * rx * k, c.1 + dy * ry * k)
    });
    b
}

fn wave_warp(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let h = ctx.params.f("height") * b.scale;
    let w = (ctx.params.f("width") * b.scale).max(1.0);
    let dir = ctx.params.f("direction").to_radians();
    let speed = ctx.params.f("speed");
    let phase = ctx.params.f("phase").to_radians() + ctx.time * speed * std::f64::consts::TAU;
    let kind = ctx.params.e("waveType");
    let (dx, dy) = (dir.sin(), -dir.cos());
    if !ctx.adjustment {
        b.pad(h.abs().ceil() as u32 + 1);
    }
    let wave = move |t: f64| -> f64 {
        let ph = t.rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU;
        match kind {
            1 => {
                if ph < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            2 => 1.0 - 4.0 * (ph - 0.5).abs(),
            3 => 2.0 * ph - 1.0,
            _ => t.sin(),
        }
    };
    b.img = remap(&b.img, false, |x, y| {
        // Waves travel along `dir`; displacement is perpendicular to it.
        let along = x * dx + y * dy;
        let disp = wave(along / w * std::f64::consts::TAU + phase) * h;
        (x + dy * disp, y - dx * disp)
    });
    b
}

fn ripple(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let r = (ctx.params.f("radius") / 100.0 * b.img.width.max(b.img.height) as f64).max(1.0);
    let c = b.to_px(ctx.params.v2("center"));
    let w = (ctx.params.f("waveWidth") * b.scale).max(1.0);
    let h = ctx.params.f("waveHeight") * b.scale;
    let phase = ctx.params.f("phase").to_radians() - ctx.time * ctx.params.f("speed") * std::f64::consts::TAU;
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = (dx * dx + dy * dy).sqrt();
        if d >= r || d == 0.0 {
            return (x, y);
        }
        let fall = 1.0 - d / r;
        let o = (d / w * std::f64::consts::TAU + phase).sin() * h * fall;
        (x + dx / d * o, y + dy / d * o)
    });
    b
}

fn mirror(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = b.to_px(ctx.params.v2("center"));
    let a = ctx.params.f("angle").to_radians();
    let n = (a.cos(), a.sin());
    b.img = remap(&b.img, false, |x, y| {
        let d = (x - c.0) * n.0 + (y - c.1) * n.1;
        if d > 0.0 { (x - 2.0 * d * n.0, y - 2.0 * d * n.1) } else { (x, y) }
    });
    b
}

fn offset(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let shift = ctx.params.v2("shift");
    let (w, h) = (b.img.width as f64, b.img.height as f64);
    let blend = ctx.params.f("blend") as f32 / 100.0;
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (sx, sy) = b.to_px(shift);
    let orig = b.img.clone();
    let mut o = remap(&b.img, false, |x, y| ((x - (sx - cx)).rem_euclid(w), (y - (sy - cy)).rem_euclid(h)));
    if blend > 0.0 {
        for (p, q) in o.data.iter_mut().zip(&orig.data) {
            for i in 0..4 {
                p[i] += (q[i] - p[i]) * blend;
            }
        }
    }
    b.img = o;
    b
}

fn polar(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let amt = ctx.params.f("interpolation") / 100.0;
    let to_polar = ctx.params.e("conversion") == 1;
    let (w, h) = (b.img.width as f64, b.img.height as f64);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let rmax = cx.min(cy);
    b.img = remap(&b.img, true, |x, y| {
        let (px, py) = if to_polar {
            // output is polar: angle → x, radius → y
            let dx = x - cx;
            let dy = y - cy;
            let a = dx.atan2(-dy).rem_euclid(std::f64::consts::TAU);
            let r = (dx * dx + dy * dy).sqrt() / rmax;
            (a / std::f64::consts::TAU * w, h - r * h)
        } else {
            let a = x / w * std::f64::consts::TAU;
            let r = (h - y) / h * rmax;
            (cx + r * a.sin(), cy - r * a.cos())
        };
        (x + (px - x) * amt, y + (py - y) * amt)
    });
    b
}

fn transform(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let anchor = b.to_px(ctx.params.v2("anchor"));
    let pos = b.to_px(ctx.params.v2("position"));
    let uniform = ctx.params.b("uniform");
    let sh = ctx.params.f("scaleHeight");
    let sw = if uniform { sh } else { ctx.params.f("scaleWidth") };
    let skew = ctx.params.f("skew");
    let skew_axis = ctx.params.f("skewAxis");
    let rot = ctx.params.f("rotation");
    let opacity = ctx.params.f("opacity") as f32 / 100.0;
    let m = Mat3::translate(vec2(pos.0, pos.1))
        * Mat3::rotate_deg(rot)
        * Mat3::skew_deg(skew, skew_axis)
        * Mat3::scale(vec2(sw / 100.0, sh / 100.0))
        * Mat3::translate(vec2(-anchor.0, -anchor.1));
    let mut out = Image::new(b.img.width, b.img.height);
    effectcraft_raster::composite_warp(&mut out, &b.img, &m, &effectcraft_raster::WarpOpts { opacity, ..Default::default() });
    b.img = out;
    b
}

fn corner_pin(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let (w, h) = (b.layer_w(ctx), b.layer_h(ctx));
    let q = ["ul", "ur", "lr", "ll"].map(|k| {
        let p = b.to_px(ctx.params.v2(k));
        vec2(p.0, p.1)
    });
    let quad = Mat3::square_to_quad(q);
    let o = b.to_px([0.0, 0.0]);
    let m = quad * Mat3::scale(vec2(1.0 / (w * b.scale), 1.0 / (h * b.scale))) * Mat3::translate(vec2(-o.0, -o.1));
    let mut out = Image::new(b.img.width, b.img.height);
    effectcraft_raster::composite_warp(&mut out, &b.img, &m, &Default::default());
    b.img = out;
    b
}

impl Buf {
    fn layer_w(&self, ctx: &EffectCtx) -> f64 {
        ctx.layer_size[0].max(1.0)
    }
    fn layer_h(&self, ctx: &EffectCtx) -> f64 {
        ctx.layer_size[1].max(1.0)
    }
}

pub fn specs() -> Vec<EffectSpec> {
    let pt = |x, y| Value::Vec2([x, y]);
    vec![
        spec(
            "ec.distort.twirl",
            "Twirl",
            vec![
                p("angle", "Angle", num(0.0), ParamUi::Angle),
                p("radius", "Twirl Radius", num(30.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("center", "Twirl Center", pt(0.5, 0.5), ParamUi::Point),
            ],
            twirl,
        ),
        spec(
            "ec.distort.spherize",
            "Spherize",
            vec![p("radius", "Radius", num(0.0), slider(0.0, 2500.0, 0.0, 2500.0, 1)), p("center", "Center of Sphere", pt(0.5, 0.5), ParamUi::Point)],
            spherize,
        ),
        spec(
            "ec.distort.bulge",
            "Bulge",
            vec![
                p("hradius", "Horizontal Radius", num(50.0), slider(0.0, 4000.0, 0.0, 1000.0, 1)),
                p("vradius", "Vertical Radius", num(50.0), slider(0.0, 4000.0, 0.0, 1000.0, 1)),
                p("center", "Bulge Center", pt(0.5, 0.5), ParamUi::Point),
                p("height", "Bulge Height", num(1.0), slider(-4.0, 4.0, -4.0, 4.0, 2)),
            ],
            bulge,
        ),
        spec(
            "ec.distort.wavewarp",
            "Wave Warp",
            vec![
                p("waveType", "Wave Type", Value::Enum(0), popup(&["Sine", "Square", "Triangle", "Sawtooth"])),
                p("height", "Wave Height", num(10.0), slider(-1000.0, 1000.0, -100.0, 100.0, 0)),
                p("width", "Wave Width", num(40.0), slider(1.0, 10000.0, 1.0, 500.0, 0)),
                p("direction", "Direction", num(90.0), ParamUi::Angle),
                p("speed", "Wave Speed", num(1.0), slider(-100.0, 100.0, -5.0, 5.0, 1)),
                p("phase", "Phase", num(0.0), ParamUi::Angle),
            ],
            wave_warp,
        ),
        spec(
            "ec.distort.ripple",
            "Ripple",
            vec![
                p("radius", "Radius", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("center", "Center of Ripple", pt(0.5, 0.5), ParamUi::Point),
                p("waveWidth", "Wave Width", num(20.0), slider(1.0, 1000.0, 1.0, 100.0, 1)),
                p("waveHeight", "Wave Height", num(10.0), slider(0.0, 400.0, 0.0, 100.0, 1)),
                p("speed", "Wave Speed", num(1.0), slider(-15.0, 15.0, -5.0, 5.0, 1)),
                p("phase", "Ripple Phase", num(0.0), ParamUi::Angle),
            ],
            ripple,
        ),
        spec(
            "ec.distort.mirror",
            "Mirror",
            vec![p("center", "Reflection Center", pt(0.5, 0.5), ParamUi::Point), p("angle", "Reflection Angle", num(0.0), ParamUi::Angle)],
            mirror,
        ),
        spec(
            "ec.distort.offset",
            "Offset",
            vec![p("shift", "Shift Center To", pt(0.5, 0.5), ParamUi::Point), p("blend", "Blend With Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1))],
            offset,
        ),
        spec(
            "ec.distort.polar",
            "Polar Coordinates",
            vec![
                p("interpolation", "Interpolation", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("conversion", "Type of Conversion", Value::Enum(0), popup(&["Rect to Polar", "Polar to Rect"])),
            ],
            polar,
        ),
        spec(
            "ec.distort.transform",
            "Transform",
            vec![
                p("anchor", "Anchor Point", pt(0.5, 0.5), ParamUi::Point),
                p("position", "Position", pt(0.5, 0.5), ParamUi::Point),
                p("uniform", "Uniform Scale", Value::Bool(true), ParamUi::Checkbox),
                p("scaleHeight", "Scale Height", num(100.0), slider(-10000.0, 10000.0, 0.0, 600.0, 1)),
                p("scaleWidth", "Scale Width", num(100.0), slider(-10000.0, 10000.0, 0.0, 600.0, 1)),
                p("skew", "Skew", num(0.0), slider(-70.0, 70.0, -70.0, 70.0, 1)),
                p("skewAxis", "Skew Axis", num(0.0), ParamUi::Angle),
                p("rotation", "Rotation", num(0.0), ParamUi::Angle),
                p("opacity", "Opacity", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            transform,
        ),
        spec(
            "ec.distort.cornerpin",
            "Corner Pin",
            vec![
                p("ul", "Upper Left", pt(0.0, 0.0), ParamUi::Point),
                p("ur", "Upper Right", pt(1.0, 0.0), ParamUi::Point),
                p("ll", "Lower Left", pt(0.0, 1.0), ParamUi::Point),
                p("lr", "Lower Right", pt(1.0, 1.0), ParamUi::Point),
            ],
            corner_pin,
        ),
    ]
}
