//! Generate and Noise effects that synthesise images.

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::Px;
use rayon::prelude::*;

use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, category: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category, params, render, gpu: false, float: true }
}

/// Blend a generated straight colour `g` (with alpha `ga`) over the original pixel.
fn put(px: &mut Px, g: [f32; 3], ga: f32, blend_orig: f32) {
    let g2 = [g[0] * ga, g[1] * ga, g[2] * ga, ga];
    let k = 1.0 - blend_orig;
    for c in 0..4 {
        px[c] = px[c] * blend_orig + g2[c] * k;
    }
}

fn gradient_ramp(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let s = b.to_px(ctx.params.v2("start"));
    let e = b.to_px(ctx.params.v2("end"));
    let c0 = ctx.params.color("startColor");
    let c1 = ctx.params.color("endColor");
    let radial = ctx.params.e("shape") == 1;
    let blend = ctx.params.f("blend") as f32 / 100.0;
    let scatter = ctx.params.f("scatter") as f32 / 512.0;
    let (dx, dy) = (e.0 - s.0, e.1 - s.1);
    let len2 = (dx * dx + dy * dy).max(1e-9);
    let seed = ctx.seed;
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (vx, vy) = (x as f64 + 0.5 - s.0, y as f64 + 0.5 - s.1);
            let mut t = if radial { ((vx * vx + vy * vy) / len2).sqrt() } else { (vx * dx + vy * dy) / len2 } as f32;
            if scatter > 0.0 {
                t += (effectcraft_raster::hash_noise(x as u32, y as u32, seed) - 0.5) * scatter;
            }
            let t = t.clamp(0.0, 1.0);
            let c = [c0[0] + (c1[0] - c0[0]) * t, c0[1] + (c1[1] - c0[1]) * t, c0[2] + (c1[2] - c0[2]) * t];
            put(px, c, 1.0, blend);
        }
    });
    b
}

fn four_color(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pts: Vec<((f64, f64), [f32; 4])> = (1..=4).map(|i| (b.to_px(ctx.params.v2(&format!("point{i}"))), ctx.params.color(&format!("color{i}")))).collect();
    let blend = ctx.params.f("blend").max(1.0);
    let op = ctx.params.f("opacity") as f32 / 100.0;
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let mut acc = [0.0f64; 3];
            let mut wsum = 0.0;
            for ((qx, qy), c) in &pts {
                let d2 = (x as f64 + 0.5 - qx).powi(2) + (y as f64 + 0.5 - qy).powi(2);
                let w = 1.0 / (d2 / (blend * 100.0) + 1e-6).powf(1.5);
                for i in 0..3 {
                    acc[i] += c[i] as f64 * w;
                }
                wsum += w;
            }
            let c = acc.map(|v| (v / wsum) as f32);
            put(px, c, 1.0, 1.0 - op);
        }
    });
    b
}

fn checkerboard(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let anchor = b.to_px(ctx.params.v2("anchor"));
    let w = (ctx.params.f("width") * b.scale).max(1.0);
    let h = (ctx.params.f("height") * b.scale).max(1.0);
    let c = ctx.params.color("color");
    let op = ctx.params.f("opacity") as f32 / 100.0;
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let cx = ((x as f64 + 0.5 - anchor.0) / w).floor() as i64;
            let cy = ((y as f64 + 0.5 - anchor.1) / h).floor() as i64;
            if (cx + cy).rem_euclid(2) == 0 {
                put(px, [c[0], c[1], c[2]], op, 0.0);
            }
        }
    });
    b
}

fn grid(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let anchor = b.to_px(ctx.params.v2("anchor"));
    let w = (ctx.params.f("width") * b.scale).max(1.0);
    let h = (ctx.params.f("height") * b.scale).max(1.0);
    let border = (ctx.params.f("border") * b.scale).max(0.0);
    let c = ctx.params.color("color");
    let op = ctx.params.f("opacity") as f32 / 100.0;
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let fx = (x as f64 + 0.5 - anchor.0).rem_euclid(w);
            let fy = (y as f64 + 0.5 - anchor.1).rem_euclid(h);
            let d = fx.min(w - fx).min(fy.min(h - fy));
            let cov = ((border / 2.0 - d) + 0.5).clamp(0.0, 1.0) as f32;
            if cov > 0.0 {
                let mut g = *px;
                put(&mut g, [c[0], c[1], c[2]], op, 0.0);
                for i in 0..4 {
                    px[i] += (g[i] - px[i]) * cov;
                }
            }
        }
    });
    b
}

fn circle(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = b.to_px(ctx.params.v2("center"));
    let r = ctx.params.f("radius") * b.scale;
    let feather = (ctx.params.f("feather") * b.scale).max(0.5);
    let color = ctx.params.color("color");
    let op = ctx.params.f("opacity") as f32 / 100.0;
    let invert = ctx.params.b("invert");
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let d = ((x as f64 + 0.5 - c.0).powi(2) + (y as f64 + 0.5 - c.1).powi(2)).sqrt();
            let mut cov = ((r - d) / feather + 0.5).clamp(0.0, 1.0) as f32;
            if invert {
                cov = 1.0 - cov;
            }
            if cov > 0.0 {
                let mut g = *px;
                put(&mut g, [color[0], color[1], color[2]], op, 0.0);
                for i in 0..4 {
                    px[i] += (g[i] - px[i]) * cov;
                }
            }
        }
    });
    b
}

// ---- value noise / fBm (our own lattice noise with quintic fade) ----

#[inline]
fn lattice(ix: i32, iy: i32, iz: i32, seed: u32) -> f32 {
    let mut h =
        (ix as u32).wrapping_mul(0x27d4_eb2d) ^ (iy as u32).wrapping_mul(0x1656_67b1) ^ (iz as u32).wrapping_mul(0x9e37_79b9) ^ seed.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// 3D value noise in 0..1 (z = evolution).
pub fn value_noise(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let (x0, y0, z0) = (x.floor(), y.floor(), z.floor());
    let (fx, fy, fz) = (fade(x - x0), fade(y - y0), fade(z - z0));
    let (ix, iy, iz) = (x0 as i32, y0 as i32, z0 as i32);
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let c = |dx: i32, dy: i32, dz: i32| lattice(ix + dx, iy + dy, iz + dz, seed);
    let a = l(l(c(0, 0, 0), c(1, 0, 0), fx), l(c(0, 1, 0), c(1, 1, 0), fx), fy);
    let b = l(l(c(0, 0, 1), c(1, 0, 1), fx), l(c(0, 1, 1), c(1, 1, 1), fx), fy);
    l(a, b, fz)
}

fn fractal_noise(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let kind = ctx.params.e("fractalType");
    let contrast = ctx.params.f("contrast") as f32 / 100.0;
    let brightness = ctx.params.f("brightness") as f32 / 100.0;
    let invert = ctx.params.b("invert");
    let scale = (ctx.params.f("scale") * b.scale).max(1.0) as f32;
    let off = ctx.params.v2("offset");
    let rot = (ctx.params.f("rotation") as f32).to_radians();
    let octaves = ctx.params.f("complexity").clamp(1.0, 20.0);
    let evo = ctx.params.f("evolution") as f32 / 360.0;
    let seed = ctx.params.f("seed") as u32 ^ 0x51ed;
    let blend = ctx.params.f("blend") as f32 / 100.0;
    let opacity = ctx.params.f("opacity") as f32 / 100.0;
    let (sr, cr) = rot.sin_cos();
    let n_oct = octaves.ceil() as usize;
    let frac = (octaves - octaves.floor()) as f32;
    let (ox, oy) = b.to_px(off);
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (dx, dy) = (x as f32 + 0.5 - ox as f32, y as f32 + 0.5 - oy as f32);
            let (u, v) = ((dx * cr + dy * sr) / scale, (-dx * sr + dy * cr) / scale);
            let mut sum = 0.0;
            let mut amp = 1.0;
            let mut norm = 0.0;
            let mut f = 1.0;
            for o in 0..n_oct {
                let w = if o + 1 == n_oct && frac > 0.0 { frac } else { 1.0 };
                let mut n = value_noise(u * f, v * f, evo + o as f32 * 7.31, seed + o as u32);
                if kind == 1 {
                    n = 1.0 - (n * 2.0 - 1.0).abs(); // turbulent / ridged
                }
                sum += n * amp * w;
                norm += amp * w;
                amp *= 0.5;
                f *= 2.0;
            }
            let mut val = sum / norm.max(1e-6);
            val = ((val - 0.5) * contrast + 0.5 + brightness).clamp(0.0, 1.0);
            if invert {
                val = 1.0 - val;
            }
            put(px, [val, val, val], opacity, blend);
        }
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let pt = |x, y| Value::Vec2([x, y]);
    vec![
        spec(
            "ec.generate.gradientramp",
            "Gradient Ramp",
            "Generate",
            vec![
                p("start", "Start of Ramp", pt(0.5, 0.0), ParamUi::Point),
                p("startColor", "Start Color", col(0.0, 0.0, 0.0), ParamUi::Color),
                p("end", "End of Ramp", pt(0.5, 1.0), ParamUi::Point),
                p("endColor", "End Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("shape", "Ramp Shape", Value::Enum(0), popup(&["Linear Ramp", "Radial Ramp"])),
                p("scatter", "Ramp Scatter", num(0.0), slider(0.0, 512.0, 0.0, 512.0, 1)),
                p("blend", "Blend With Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            gradient_ramp,
        ),
        spec(
            "ec.generate.fourcolor",
            "4-Color Gradient",
            "Generate",
            vec![
                p("point1", "Point 1", pt(0.1, 0.1), ParamUi::Point),
                p("color1", "Color 1", col(1.0, 1.0, 0.0), ParamUi::Color),
                p("point2", "Point 2", pt(0.9, 0.1), ParamUi::Point),
                p("color2", "Color 2", col(0.0, 1.0, 0.0), ParamUi::Color),
                p("point3", "Point 3", pt(0.1, 0.9), ParamUi::Point),
                p("color3", "Color 3", col(1.0, 0.0, 1.0), ParamUi::Color),
                p("point4", "Point 4", pt(0.9, 0.9), ParamUi::Point),
                p("color4", "Color 4", col(0.0, 0.0, 1.0), ParamUi::Color),
                p("blend", "Blend", num(100.0), slider(1.0, 1000.0, 1.0, 1000.0, 1)),
                p("opacity", "Opacity", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            four_color,
        ),
        spec(
            "ec.generate.checkerboard",
            "Checkerboard",
            "Generate",
            vec![
                p("anchor", "Anchor", pt(0.0, 0.0), ParamUi::Point),
                p("width", "Width", num(64.0), slider(1.0, 4000.0, 1.0, 400.0, 1)),
                p("height", "Height", num(64.0), slider(1.0, 4000.0, 1.0, 400.0, 1)),
                p("color", "Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("opacity", "Opacity", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            checkerboard,
        ),
        spec(
            "ec.generate.grid",
            "Grid",
            "Generate",
            vec![
                p("anchor", "Anchor", pt(0.0, 0.0), ParamUi::Point),
                p("width", "Width", num(64.0), slider(1.0, 4000.0, 1.0, 400.0, 1)),
                p("height", "Height", num(64.0), slider(1.0, 4000.0, 1.0, 400.0, 1)),
                p("border", "Border", num(2.0), slider(0.0, 400.0, 0.0, 40.0, 1)),
                p("color", "Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("opacity", "Opacity", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            grid,
        ),
        spec(
            "ec.generate.circle",
            "Circle",
            "Generate",
            vec![
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("radius", "Radius", num(75.0), slider(0.0, 4000.0, 0.0, 1000.0, 1)),
                p("feather", "Feather Outer Edge", num(0.0), slider(0.0, 1000.0, 0.0, 100.0, 1)),
                p("invert", "Invert Circle", Value::Bool(false), ParamUi::Checkbox),
                p("color", "Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("opacity", "Opacity", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            circle,
        ),
        spec(
            "ec.noise.fractal",
            "Fractal Noise",
            "Noise & Grain",
            vec![
                p("fractalType", "Fractal Type", Value::Enum(0), popup(&["Basic", "Turbulent Smooth"])),
                p("invert", "Invert", Value::Bool(false), ParamUi::Checkbox),
                p("contrast", "Contrast", num(100.0), slider(0.0, 10000.0, 0.0, 400.0, 1)),
                p("brightness", "Brightness", num(0.0), slider(-10000.0, 10000.0, -200.0, 200.0, 1)),
                p("scale", "Scale", num(100.0), slider(1.0, 10000.0, 20.0, 600.0, 1)),
                p("offset", "Offset Turbulence", pt(0.5, 0.5), ParamUi::Point),
                p("rotation", "Rotation", num(0.0), ParamUi::Angle),
                p("complexity", "Complexity", num(6.0), slider(1.0, 20.0, 1.0, 10.0, 1)),
                p("evolution", "Evolution", num(0.0), ParamUi::Angle),
                p("seed", "Random Seed", num(0.0), slider(0.0, 100000.0, 0.0, 1000.0, 0)),
                p("opacity", "Opacity", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("blend", "Blend With Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            fractal_noise,
        ),
    ]
}
