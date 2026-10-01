//! Blur & Sharpen, Stylize, Perspective, Channel, Noise and Transition effects.

use effectcraft_color::luminance;
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::{Image, gaussian_blur, hash_noise};
use rayon::prelude::*;

use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, category: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category, params, render, gpu: false, float: true }
}

/// AE's Blurriness is roughly a radius; our Gaussian uses sigma = blurriness / 2.
fn blur_sigma(blurriness: f64) -> f64 {
    blurriness.max(0.0) * 0.5
}

fn dims_xy(dim: u32) -> (f64, f64) {
    match dim {
        1 => (1.0, 0.0),
        2 => (0.0, 1.0),
        _ => (1.0, 1.0),
    }
}

fn gaussian(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let s = blur_sigma(ctx.params.f("blurriness")) * b.scale;
    if s <= 0.0 {
        return b;
    }
    let (kx, ky) = dims_xy(ctx.params.e("dimensions"));
    let repeat = ctx.params.b("repeatEdge");
    if !repeat && !ctx.adjustment {
        b.pad((s * 3.0).ceil() as u32);
    }
    b.img = gaussian_blur(&b.img, s * kx, s * ky, repeat || ctx.adjustment);
    b
}

fn box_blur(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let r = (ctx.params.f("radius") * b.scale).round().max(0.0) as usize;
    if r == 0 {
        return b;
    }
    let it = ctx.params.f("iterations").round().clamp(1.0, 50.0) as usize;
    let (kx, ky) = dims_xy(ctx.params.e("dimensions"));
    let repeat = ctx.params.b("repeatEdge");
    if !repeat && !ctx.adjustment {
        b.pad((r * it) as u32 + 1);
    }
    b.img = effectcraft_raster::box_blur(&b.img, r * kx as usize, r * ky as usize, it, repeat || ctx.adjustment);
    b
}

fn directional(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let len = ctx.params.f("length") * b.scale;
    if len < 0.5 {
        return b;
    }
    if !ctx.adjustment {
        b.pad(len.ceil() as u32 + 1);
    }
    b.img = effectcraft_raster::directional_blur(&b.img, ctx.params.f("direction"), len);
    b
}

fn radial(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let amt = ctx.params.f("amount");
    let zoom = ctx.params.e("type") == 1;
    let c = b.to_px(ctx.params.v2("center"));
    b.img = effectcraft_raster::radial_blur(&b.img, c, if zoom { amt / 100.0 } else { amt }, zoom);
    b
}

fn unsharp_core(img: &Image, amount: f32, sigma: f64, threshold: f32) -> Image {
    let blurred = gaussian_blur(img, sigma, sigma, true);
    let mut out = img.clone();
    out.data.par_iter_mut().zip(blurred.data.par_iter()).for_each(|(o, bl)| {
        for c in 0..3 {
            let d = o[c] - bl[c];
            if d.abs() * 255.0 >= threshold {
                o[c] = (o[c] + d * amount).max(0.0);
            }
        }
    });
    out
}

fn sharpen(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let a = ctx.params.f("amount") as f32 / 50.0;
    b.img = unsharp_core(&b.img, a, 1.0 * b.scale, 0.0);
    b
}

fn unsharp(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let a = ctx.params.f("amount") as f32 / 100.0;
    b.img = unsharp_core(&b.img, a, (ctx.params.f("radius") * b.scale).max(0.1), ctx.params.f("threshold") as f32);
    b
}

fn glow(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let thr = ctx.params.f("threshold") as f32 / 100.0;
    let radius = ctx.params.f("radius") * b.scale;
    let intensity = ctx.params.f("intensity") as f32;
    let use_colors = ctx.params.e("colors") == 1;
    let ca = ctx.params.color("colorA");
    let cb = ctx.params.color("colorB");
    if !ctx.adjustment {
        b.pad((radius * 1.5).ceil() as u32 + 2);
    }
    // Bright pass.
    let mut bright = b.img.clone();
    bright.data.par_iter_mut().for_each(|px| {
        let a = px[3];
        if a <= 0.0 {
            *px = [0.0; 4];
            return;
        }
        let l = luminance(px[0] / a, px[1] / a, px[2] / a);
        let k = ((l - thr) / (1.0 - thr).max(1e-3)).clamp(0.0, 1.0);
        if use_colors {
            let t = l.clamp(0.0, 1.0);
            let c = [ca[0] + (cb[0] - ca[0]) * t, ca[1] + (cb[1] - ca[1]) * t, ca[2] + (cb[2] - ca[2]) * t];
            *px = [c[0] * k * a, c[1] * k * a, c[2] * k * a, k * a];
        } else {
            *px = px.map(|v| v * k);
        }
    });
    let s = (radius / 2.0).max(0.5);
    let blurred = gaussian_blur(&bright, s, s, false);
    let composite_behind = ctx.params.e("operation") == 1;
    b.img.data.par_iter_mut().zip(blurred.data.par_iter()).for_each(|(o, g)| {
        let g = g.map(|v| v * intensity);
        if composite_behind {
            let k = 1.0 - o[3];
            for c in 0..4 {
                o[c] += g[c] * k;
            }
        } else {
            // Add (screen-like add on premultiplied colour; alpha grows by the glow's alpha).
            for c in 0..3 {
                o[c] += g[c];
            }
            o[3] = (o[3] + g[3] * (1.0 - o[3])).min(1.0);
        }
        o[3] = o[3].clamp(0.0, 1.0);
    });
    b
}

fn drop_shadow(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let color = ctx.params.color("color");
    let opacity = ctx.params.f("opacity") as f32 / 100.0;
    let dir = ctx.params.f("direction").to_radians();
    let dist = ctx.params.f("distance") * b.scale;
    let soft = ctx.params.f("softness") * b.scale;
    let only = ctx.params.b("shadowOnly");
    let pad = (dist + soft * 1.5).ceil() as u32 + 2;
    if !ctx.adjustment {
        b.pad(pad);
    }
    let (dx, dy) = (dir.sin() * dist, -dir.cos() * dist);
    let mut sh = Image::new(b.img.width, b.img.height);
    let src = &b.img;
    sh.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let a = src.sample_bilinear(x as f64 + 0.5 - dx, y as f64 + 0.5 - dy)[3] * opacity * color[3];
            *px = [color[0] * a, color[1] * a, color[2] * a, a];
        }
    });
    if soft > 0.0 {
        sh = gaussian_blur(&sh, soft / 2.0, soft / 2.0, false);
    }
    if !only {
        sh.data.par_iter_mut().zip(b.img.data.par_iter()).for_each(|(s, o)| {
            let k = 1.0 - o[3];
            for c in 0..4 {
                s[c] = o[c] + s[c] * k;
            }
        });
    }
    b.img = sh;
    b
}

fn invert(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let ch = ctx.params.e("channel");
    let blend = 1.0 - ctx.params.f("blend") as f32 / 100.0;
    b.img.data.par_iter_mut().for_each(|px| {
        let a = px[3];
        if ch == 4 {
            let na = 1.0 - a;
            let k = if a > 0.0 { na / a } else { 0.0 };
            let inv = [px[0] * k, px[1] * k, px[2] * k, na];
            for c in 0..4 {
                px[c] += (inv[c] - px[c]) * blend;
            }
            return;
        }
        if a <= 0.0 {
            return;
        }
        let s = [px[0] / a, px[1] / a, px[2] / a];
        let mut o = s;
        match ch {
            1 => o[0] = 1.0 - s[0],
            2 => o[1] = 1.0 - s[1],
            3 => o[2] = 1.0 - s[2],
            _ => o = [1.0 - s[0], 1.0 - s[1], 1.0 - s[2]],
        }
        for c in 0..3 {
            px[c] = (s[c] + (o[c] - s[c]) * blend) * a;
        }
    });
    b
}

fn mosaic(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let hb = ctx.params.f("horizontal").round().max(1.0) as usize;
    let vb = ctx.params.f("vertical").round().max(1.0) as usize;
    let (w, h) = (b.img.width as usize, b.img.height as usize);
    let cw = (w as f64 / hb as f64).max(1.0);
    let ch = (h as f64 / vb as f64).max(1.0);
    let src = b.img.clone();
    b.img.rows_mut().for_each(|(y, row)| {
        let cy = ((y as f64 / ch).floor() + 0.5) * ch;
        for (x, px) in row.iter_mut().enumerate() {
            let cx = ((x as f64 / cw).floor() + 0.5) * cw;
            *px = src.get_clamped(cx as i64, cy as i64);
        }
    });
    b
}

fn posterize(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let lv = ctx.params.f("level").round().clamp(2.0, 255.0) as f32 - 1.0;
    b.img.map_straight(|c| c.map(|v| (v.clamp(0.0, 1.0) * lv).round() / lv));
    b
}

fn threshold(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let t = ctx.params.f("level") as f32 / 255.0;
    b.img.map_straight(|c| {
        let v = if luminance(c[0], c[1], c[2]) >= t { 1.0 } else { 0.0 };
        [v, v, v]
    });
    b
}

fn find_edges(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let inv = ctx.params.b("invert");
    let src = b.img.clone();
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let l = |dx: i64, dy: i64| {
                let p = src.get_clamped(x as i64 + dx, y as i64 + dy);
                luminance(p[0], p[1], p[2])
            };
            let gx = l(1, -1) + 2.0 * l(1, 0) + l(1, 1) - l(-1, -1) - 2.0 * l(-1, 0) - l(-1, 1);
            let gy = l(-1, 1) + 2.0 * l(0, 1) + l(1, 1) - l(-1, -1) - 2.0 * l(0, -1) - l(1, -1);
            let e = (gx * gx + gy * gy).sqrt().min(1.0);
            let v = if inv { e } else { 1.0 - e };
            let a = px[3];
            *px = [v * a, v * a, v * a, a];
        }
    });
    b
}

fn emboss(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let ang = ctx.params.f("direction").to_radians();
    let relief = ctx.params.f("relief") * b.scale;
    let contrast = ctx.params.f("contrast") as f32 / 100.0;
    let (dx, dy) = (ang.cos() * relief, -ang.sin() * relief);
    let src = b.img.clone();
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let a = src.sample_bilinear_clamped(x as f64 + 0.5 + dx, y as f64 + 0.5 + dy);
            let c = src.sample_bilinear_clamped(x as f64 + 0.5 - dx, y as f64 + 0.5 - dy);
            let d = luminance(a[0], a[1], a[2]) - luminance(c[0], c[1], c[2]);
            let v = (0.5 + d * contrast).clamp(0.0, 1.0);
            let al = px[3];
            *px = [v * al, v * al, v * al, al];
        }
    });
    b
}

fn noise(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let amt = ctx.params.f("amount") as f32 / 100.0;
    let color = ctx.params.b("color");
    let frame_seed = (ctx.time * 1000.0) as u32 ^ ctx.seed;
    let w = b.img.width;
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let a = px[3];
            if a <= 0.0 {
                continue;
            }
            let n = |k: u32| (hash_noise(x as u32 + k * w, y as u32, frame_seed) - 0.5) * amt;
            let (r, g, bl) = if color { (n(0), n(1), n(2)) } else { let v = n(0); (v, v, v) };
            px[0] = (px[0] + r * a).clamp(0.0, a);
            px[1] = (px[1] + g * a).clamp(0.0, a);
            px[2] = (px[2] + bl * a).clamp(0.0, a);
        }
    });
    b
}

fn linear_wipe(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let done = ctx.params.f("completion") / 100.0;
    let ang = ctx.params.f("angle").to_radians();
    let feather = (ctx.params.f("feather") * b.scale).max(0.001);
    let (w, h) = (b.img.width as f64, b.img.height as f64);
    // Direction the wipe travels: angle 90° wipes left→right.
    let (dx, dy) = (ang.sin(), -ang.cos());
    let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)];
    let proj: Vec<f64> = corners.iter().map(|(x, y)| x * dx + y * dy).collect();
    let (lo, hi) = (proj.iter().cloned().fold(f64::INFINITY, f64::min), proj.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
    let edge = lo - feather + (hi - lo + 2.0 * feather) * done;
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let d = (x as f64 + 0.5) * dx + (y as f64 + 0.5) * dy;
            let k = ((d - edge) / feather + 0.5).clamp(0.0, 1.0) as f32;
            for c in px.iter_mut() {
                *c *= k;
            }
        }
    });
    b
}

fn radial_wipe(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let done = ctx.params.f("completion") / 100.0;
    let start = ctx.params.f("startAngle");
    let c = b.to_px(ctx.params.v2("center"));
    let dir = ctx.params.e("wipe");
    let feather = (ctx.params.f("feather")).max(0.01);
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let a = ((x as f64 + 0.5 - c.0).atan2(-(y as f64 + 0.5 - c.1)).to_degrees() - start).rem_euclid(360.0);
            let a = if dir == 1 { 360.0 - a } else { a };
            let edge = done * 360.0;
            let k = ((a - edge) / feather + 0.5).clamp(0.0, 1.0) as f32;
            for ch in px.iter_mut() {
                *ch *= k;
            }
        }
    });
    b
}

fn venetian(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let done = ctx.params.f("completion") / 100.0;
    let ang = ctx.params.f("direction").to_radians();
    let width = (ctx.params.f("width") * b.scale).max(1.0);
    let feather = (ctx.params.f("feather") * b.scale).max(0.001);
    let (dx, dy) = (ang.cos(), ang.sin());
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let d = ((x as f64 + 0.5) * dx + (y as f64 + 0.5) * dy).rem_euclid(width);
            let k = ((d - done * width) / feather + 0.5).clamp(0.0, 1.0) as f32;
            for c in px.iter_mut() {
                *c *= k;
            }
        }
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let dims = || popup(&["Horizontal and Vertical", "Horizontal", "Vertical"]);
    vec![
        spec(
            "ec.blur.gaussian",
            "Gaussian Blur",
            "Blur & Sharpen",
            vec![
                p("blurriness", "Blurriness", num(0.0), slider(0.0, 3000.0, 0.0, 50.0, 1)),
                p("dimensions", "Blur Dimensions", Value::Enum(0), dims()),
                p("repeatEdge", "Repeat Edge Pixels", Value::Bool(false), ParamUi::Checkbox),
            ],
            gaussian,
        ),
        spec(
            "ec.blur.fastbox",
            "Fast Box Blur",
            "Blur & Sharpen",
            vec![
                p("radius", "Blur Radius", num(0.0), slider(0.0, 3000.0, 0.0, 50.0, 1)),
                p("iterations", "Iterations", num(3.0), slider(1.0, 50.0, 1.0, 5.0, 0)),
                p("dimensions", "Blur Dimensions", Value::Enum(0), dims()),
                p("repeatEdge", "Repeat Edge Pixels", Value::Bool(false), ParamUi::Checkbox),
            ],
            box_blur,
        ),
        spec(
            "ec.blur.directional",
            "Directional Blur",
            "Blur & Sharpen",
            vec![p("direction", "Direction", num(0.0), ParamUi::Angle), p("length", "Blur Length", num(0.0), slider(0.0, 1000.0, 0.0, 100.0, 1))],
            directional,
        ),
        spec(
            "ec.blur.radial",
            "Radial Blur",
            "Blur & Sharpen",
            vec![
                p("amount", "Amount", num(10.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("center", "Center", Value::Vec2([0.5, 0.5]), ParamUi::Point),
                p("type", "Type", Value::Enum(0), popup(&["Spin", "Zoom"])),
            ],
            radial,
        ),
        spec("ec.blur.sharpen", "Sharpen", "Blur & Sharpen", vec![p("amount", "Sharpen Amount", num(0.0), slider(0.0, 500.0, 0.0, 100.0, 0))], sharpen),
        spec(
            "ec.blur.unsharp",
            "Unsharp Mask",
            "Blur & Sharpen",
            vec![
                p("amount", "Amount", num(50.0), slider(0.0, 500.0, 0.0, 500.0, 0)),
                p("radius", "Radius", num(1.0), slider(0.1, 500.0, 0.1, 50.0, 1)),
                p("threshold", "Threshold", num(0.0), slider(0.0, 255.0, 0.0, 255.0, 0)),
            ],
            unsharp,
        ),
        spec(
            "ec.stylize.glow",
            "Glow",
            "Stylize",
            vec![
                p("based", "Glow Based On", Value::Enum(1), popup(&["Alpha Channel", "Color Channels"])),
                p("threshold", "Glow Threshold", num(60.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("radius", "Glow Radius", num(10.0), slider(0.0, 1000.0, 0.0, 100.0, 1)),
                p("intensity", "Glow Intensity", num(1.0), slider(0.0, 255.0, 0.0, 4.0, 1)),
                p("operation", "Composite Original", Value::Enum(0), popup(&["On Top", "Behind", "None"])),
                p("colors", "Glow Colors", Value::Enum(0), popup(&["Original Colors", "A & B Colors"])),
                p("colorA", "Color A", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("colorB", "Color B", col(0.0, 0.0, 0.0), ParamUi::Color),
            ],
            glow,
        ),
        spec(
            "ec.stylize.mosaic",
            "Mosaic",
            "Stylize",
            vec![
                p("horizontal", "Horizontal Blocks", num(10.0), slider(1.0, 4000.0, 1.0, 200.0, 0)),
                p("vertical", "Vertical Blocks", num(10.0), slider(1.0, 4000.0, 1.0, 200.0, 0)),
            ],
            mosaic,
        ),
        spec("ec.stylize.posterize", "Posterize", "Stylize", vec![p("level", "Level", num(6.0), slider(2.0, 255.0, 2.0, 32.0, 0))], posterize),
        spec("ec.stylize.threshold", "Threshold", "Stylize", vec![p("level", "Level", num(128.0), slider(0.0, 255.0, 0.0, 255.0, 0))], threshold),
        spec("ec.stylize.findedges", "Find Edges", "Stylize", vec![p("invert", "Invert", Value::Bool(false), ParamUi::Checkbox)], find_edges),
        spec(
            "ec.stylize.emboss",
            "Emboss",
            "Stylize",
            vec![
                p("direction", "Direction", num(45.0), ParamUi::Angle),
                p("relief", "Relief", num(1.5), slider(0.0, 10.0, 0.0, 10.0, 2)),
                p("contrast", "Contrast", num(100.0), slider(0.0, 500.0, 0.0, 500.0, 0)),
            ],
            emboss,
        ),
        spec(
            "ec.perspective.dropshadow",
            "Drop Shadow",
            "Perspective",
            vec![
                p("color", "Shadow Color", col(0.0, 0.0, 0.0), ParamUi::Color),
                p("opacity", "Opacity", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("direction", "Direction", num(135.0), ParamUi::Angle),
                p("distance", "Distance", num(5.0), slider(0.0, 32000.0, 0.0, 120.0, 1)),
                p("softness", "Softness", num(0.0), slider(0.0, 1000.0, 0.0, 250.0, 1)),
                p("shadowOnly", "Shadow Only", Value::Bool(false), ParamUi::Checkbox),
            ],
            drop_shadow,
        ),
        spec(
            "ec.channel.invert",
            "Invert",
            "Channel",
            vec![
                p("channel", "Channel", Value::Enum(0), popup(&["RGB", "Red", "Green", "Blue", "Alpha"])),
                p("blend", "Blend With Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
            ],
            invert,
        ),
        spec(
            "ec.noise.noise",
            "Noise",
            "Noise & Grain",
            vec![p("amount", "Amount of Noise", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)), p("color", "Use Color Noise", Value::Bool(true), ParamUi::Checkbox)],
            noise,
        ),
        spec(
            "ec.transition.linearwipe",
            "Linear Wipe",
            "Transition",
            vec![
                p("completion", "Transition Completion", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("angle", "Wipe Angle", num(90.0), ParamUi::Angle),
                p("feather", "Feather", num(0.0), slider(0.0, 32000.0, 0.0, 500.0, 1)),
            ],
            linear_wipe,
        ),
        spec(
            "ec.transition.radialwipe",
            "Radial Wipe",
            "Transition",
            vec![
                p("completion", "Transition Completion", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("startAngle", "Start Angle", num(0.0), ParamUi::Angle),
                p("center", "Wipe Center", Value::Vec2([0.5, 0.5]), ParamUi::Point),
                p("wipe", "Wipe", Value::Enum(0), popup(&["Clockwise", "Counterclockwise", "Both"])),
                p("feather", "Feather", num(0.0), slider(0.0, 360.0, 0.0, 50.0, 1)),
            ],
            radial_wipe,
        ),
        spec(
            "ec.transition.venetian",
            "Venetian Blinds",
            "Transition",
            vec![
                p("completion", "Transition Completion", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("direction", "Direction", num(90.0), ParamUi::Angle),
                p("width", "Width", num(30.0), slider(2.0, 1000.0, 2.0, 200.0, 0)),
                p("feather", "Feather", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            venetian,
        ),
    ]
}
