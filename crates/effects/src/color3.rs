//! Color Correction effects, batch 3: Change Color, Video Limiter, CC Color Neutralizer,
//! PS Arbitrary Map and Lumetri Color (Basic Correction, Creative, Curves, Color Wheels,
//! Vignette).

use effectcraft_color::{hsl_to_rgb, luminance, rgb_to_hsl};
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use rayon::prelude::*;

use crate::util::{Plane, gauss_plane, layer_rect, lerp, premul, smoothstep, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Color Correction", params, render, gpu: false, float: true }
}

fn pct() -> ParamUi {
    slider(0.0, 100.0, 0.0, 100.0, 1)
}

fn bipolar() -> ParamUi {
    slider(-100.0, 100.0, -100.0, 100.0, 1)
}

/// Apply `f(straight colour) -> straight colour` to every pixel, keeping alpha.
fn map_colour(b: &mut Buf, f: impl Fn(usize, [f32; 3]) -> [f32; 3] + Sync) {
    b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let (c, a) = unpremul(*px);
        if a <= 0.0 {
            return;
        }
        *px = premul(f(i, c), a);
    });
}

// ---------------------------------------------------------------- Change Color

/// Similarity mask (1 = matches) of colour `c` to `key`.
#[inline]
fn colour_match(c: [f32; 3], key: [f32; 3], using: u32, tol: f32, soft: f32) -> f32 {
    let d = match using {
        0 => ((c[0] - key[0]).powi(2) + (c[1] - key[1]).powi(2) + (c[2] - key[2]).powi(2)).sqrt() / 3f32.sqrt(),
        1 => {
            let (h1, s1, _) = rgb_to_hsl(c[0], c[1], c[2]);
            let (h2, _, _) = rgb_to_hsl(key[0], key[1], key[2]);
            let dh = (h1 - h2).abs();
            let dh = dh.min(1.0 - dh) * 2.0;
            // Greys have no reliable hue.
            if s1 < 0.02 { 1.0 } else { dh }
        }
        _ => {
            let chroma = |c: [f32; 3]| {
                let s = (c[0] + c[1] + c[2]).max(1e-6);
                [c[0] / s, c[1] / s]
            };
            let (a, k) = (chroma(c), chroma(key));
            ((a[0] - k[0]).powi(2) + (a[1] - k[1]).powi(2)).sqrt() * 1.5
        }
    };
    if d <= tol {
        1.0
    } else if soft <= 1e-6 {
        0.0
    } else {
        (1.0 - (d - tol) / soft).clamp(0.0, 1.0)
    }
}

fn change_color(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let view_mask = ctx.params.e("view") == 1;
    let dh = (ctx.params.f("hueTransform") / 360.0) as f32;
    let dl = (ctx.params.f("lightnessTransform") / 100.0) as f32;
    let ds = (ctx.params.f("saturationTransform") / 100.0) as f32;
    let k = ctx.params.color("colorToChange");
    let key = [k[0], k[1], k[2]];
    let tol = (ctx.params.f("matchingTolerance") / 100.0) as f32;
    let soft = (ctx.params.f("matchingSoftness") / 100.0) as f32;
    let using = ctx.params.e("matchColors");
    let invert = ctx.params.b("invertColorCorrectionMask");
    if !view_mask && dh == 0.0 && dl == 0.0 && ds == 0.0 {
        return b;
    }
    map_colour(&mut b, |_, c| {
        let mut m = colour_match(c, key, using, tol, soft);
        if invert {
            m = 1.0 - m;
        }
        if view_mask {
            return [m; 3];
        }
        if m <= 0.0 {
            return c;
        }
        let (h, s, l) = rgb_to_hsl(c[0], c[1], c[2]);
        let h2 = (h + dh).rem_euclid(1.0);
        let s2 = (s + ds * if ds > 0.0 { 1.0 - s } else { s }).clamp(0.0, 1.0);
        let l2 = (l + dl * if dl > 0.0 { 1.0 - l } else { l }).clamp(0.0, 1.0);
        let (r, g, bb) = hsl_to_rgb(h2, s2, l2);
        [lerp(c[0], r, m), lerp(c[1], g, m), lerp(c[2], bb, m)]
    });
    b
}

// ---------------------------------------------------------------- Video Limiter

const CLIP_LEVELS: [f32; 5] = [0.9, 0.95, 1.0, 1.05, 1.09];

/// Soft knee: values above `start` approach `limit` smoothly.
#[inline]
fn knee(v: f32, start: f32, limit: f32) -> f32 {
    if v <= start || limit <= start {
        return v.min(limit);
    }
    let r = limit - start;
    start + r * (1.0 - (-(v - start) / r).exp())
}

fn video_limiter(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let level = CLIP_LEVELS[(ctx.params.e("clipLevel") as usize).min(CLIP_LEVELS.len() - 1)];
    let method = ctx.params.e("clipMethod");
    let comp = [0.0, 0.03, 0.05, 0.1, 0.2][(ctx.params.e("compressionBeforeClipping") as usize).min(4)];
    let warn = ctx.params.b("gamutWarning");
    let wc = ctx.params.color("gamutWarningColor");
    let start = level * (1.0 - comp);
    map_colour(&mut b, |_, c| {
        let y = luminance(c[0], c[1], c[2]);
        let mut o = c;
        // Luma: compress luminance, keep chroma offsets.
        if method != 1 {
            let y2 = knee(y.max(0.0), start, level);
            o = [o[0] - y + y2, o[1] - y + y2, o[2] - y + y2];
        }
        // Chroma: scale chroma towards luma until every channel is within 0..level.
        if method != 0 {
            let yl = luminance(o[0], o[1], o[2]);
            let mut t = 1.0f32;
            for &v in &o {
                if v > level && v - yl > 1e-6 {
                    t = t.min((level - yl) / (v - yl));
                }
                if v < 0.0 && yl - v > 1e-6 {
                    t = t.min(yl / (yl - v));
                }
            }
            let t = t.clamp(0.0, 1.0);
            o = [yl + (o[0] - yl) * t, yl + (o[1] - yl) * t, yl + (o[2] - yl) * t];
        }
        let o = o.map(|v| v.clamp(0.0, level));
        if warn && (c.iter().zip(o.iter()).any(|(a, b)| (a - b).abs() > 1e-4)) {
            return [wc[0], wc[1], wc[2]];
        }
        o
    });
    b
}

// ---------------------------------------------------------------- CC Color Neutralizer

fn color_neutralizer(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pairs = [("shadowsUnbalance", "shadowsBalance"), ("midtonesUnbalance", "midtonesBalance"), ("highlightsUnbalance", "highlightsBalance")];
    let off: Vec<[f32; 3]> = pairs
        .iter()
        .map(|(u, bal)| {
            let (u, v) = (ctx.params.color(u), ctx.params.color(bal));
            [v[0] - u[0], v[1] - u[1], v[2] - u[2]]
        })
        .collect();
    let pin = (ctx.params.f("pinning") / 100.0) as f32;
    let orig = (ctx.params.f("blendWOriginal") / 100.0) as f32;
    let contrast = (ctx.params.f("contrast") / 100.0) as f32;
    let darks = (ctx.params.f("darks") / 100.0) as f32;
    let brights = (ctx.params.f("brights") / 100.0) as f32;
    map_colour(&mut b, |_, c| {
        let l = luminance(c[0], c[1], c[2]).clamp(0.0, 1.0);
        let ws = 1.0 - smoothstep(0.0, 0.5, l);
        let wh = smoothstep(0.5, 1.0, l);
        let wm = 1.0 - ws - wh;
        // Pinning keeps pure black and pure white fixed.
        let keep = 1.0 - pin * (1.0 - 4.0 * l * (1.0 - l)).clamp(0.0, 1.0);
        let mut o = [0.0; 3];
        for k in 0..3 {
            let d = off[0][k] * ws + off[1][k] * wm + off[2][k] * wh;
            let mut v = c[k] + d * keep;
            v = 0.5 + (v - 0.5) * (1.0 + contrast);
            v += darks * 0.25 * (1.0 - l).powi(2) + brights * 0.25 * l * l;
            o[k] = v.max(0.0);
        }
        [lerp(o[0], c[0], orig), lerp(o[1], c[1], orig), lerp(o[2], c[2], orig)]
    });
    b
}

// ---------------------------------------------------------------- PS Arbitrary Map

/// Parse a map string of 256 comma/space separated values (0..255, or 0..1 floats). An empty or
/// malformed map is the identity.
fn parse_map(s: &str) -> Vec<f32> {
    let vals: Vec<f32> = s.split(|ch: char| ch == ',' || ch.is_whitespace()).filter(|t| !t.is_empty()).filter_map(|t| t.parse::<f32>().ok()).collect();
    if vals.len() < 2 {
        return (0..256).map(|i| i as f32 / 255.0).collect();
    }
    let max = vals.iter().cloned().fold(0.0f32, f32::max);
    let k = if max > 1.0 { 1.0 / 255.0 } else { 1.0 };
    vals.iter().map(|v| (v * k).clamp(0.0, 1.0)).collect()
}

fn arbitrary_map(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let map = parse_map(ctx.params.s("map"));
    let phase = ctx.params.f("phase");
    let alpha = ctx.params.b("applyPhaseMapToAlpha");
    let n = map.len();
    let lookup = |v: f32| -> f32 {
        let x = (v.clamp(0.0, 1.0) as f64 * (n - 1) as f64 + phase * (n - 1) as f64 / 255.0).rem_euclid(n as f64);
        let i0 = x.floor() as usize % n;
        let i1 = if i0 + 1 < n { i0 + 1 } else { i0 };
        let t = (x - x.floor()) as f32;
        lerp(map[i0], map[i1], t)
    };
    b.img.data.par_iter_mut().for_each(|px| {
        let (c, a) = unpremul(*px);
        let o = c.map(lookup);
        let na = if alpha { lookup(a) } else { a };
        *px = premul(o, na);
    });
    b
}

// ---------------------------------------------------------------- Lumetri Color

/// Smooth tone-curve adjustment: offsets for shadows/midtones/highlights (each -1..1), with the
/// end points fixed.
#[inline]
fn tone_curve(x: f32, s: f32, m: f32, h: f32) -> f32 {
    if s == 0.0 && m == 0.0 && h == 0.0 {
        return x;
    }
    let t = x.clamp(0.0, 1.0);
    let bump = |c: f32| {
        let d = (t - c) / 0.25;
        (1.0 - d * d).max(0.0).powi(2)
    };
    x + 0.25 * (s * bump(0.25) + m * bump(0.5) + h * bump(0.75))
}

/// The colour part of a wheel/tint colour (offset from its own grey), 0 for neutral colours.
#[inline]
fn tint_offset(c: [f32; 4]) -> [f32; 3] {
    let l = (c[0] + c[1] + c[2]) / 3.0;
    [c[0] - l, c[1] - l, c[2] - l]
}

fn lumetri(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let temp = (pr.f("temperature") / 100.0) as f32;
    let tint = (pr.f("tint") / 100.0) as f32;
    let expo = 2f32.powf(pr.f("exposure") as f32);
    let contrast = (pr.f("contrast") / 100.0) as f32;
    let (hi, sh, wh, bl) =
        ((pr.f("highlights") / 100.0) as f32, (pr.f("shadows") / 100.0) as f32, (pr.f("whites") / 100.0) as f32, (pr.f("blacks") / 100.0) as f32);
    let sat = (pr.f("saturation") / 100.0) as f32;
    let intensity = (pr.f("lookIntensity") / 100.0) as f32;
    let faded = (pr.f("fadedFilm") / 100.0) as f32;
    let vib = (pr.f("vibrance") / 100.0) as f32;
    let csat = (pr.f("creativeSaturation") / 100.0) as f32;
    let st = tint_offset(pr.color("shadowTint"));
    let ht = tint_offset(pr.color("highlightTint"));
    let tbal = (pr.f("tintBalance") / 100.0) as f32;
    let curve = |k: &str| {
        [(pr.f(&format!("{k}Shadows")) / 100.0) as f32, (pr.f(&format!("{k}Midtones")) / 100.0) as f32, (pr.f(&format!("{k}Highlights")) / 100.0) as f32]
    };
    let (cm, cr, cg, cb) = (curve("curveMaster"), curve("curveRed"), curve("curveGreen"), curve("curveBlue"));
    let ws = tint_offset(pr.color("shadowsWheel"));
    let wm = tint_offset(pr.color("midtonesWheel"));
    let wh_ = tint_offset(pr.color("highlightsWheel"));
    let vig = pr.f("vignetteAmount") as f32;
    let vmid = (pr.f("vignetteMidpoint") / 100.0) as f32;
    let vround = (pr.f("vignetteRoundness") / 100.0) as f32;
    let vfeather = (pr.f("vignetteFeather") / 100.0) as f32;
    let sharpen = (pr.f("sharpen") / 100.0) as f32;
    let (lx, ly, lw, lh) = layer_rect(ctx, &b);
    let w = b.img.width as usize;
    let (cx, cy) = (lx + lw * 0.5, ly + lh * 0.5);
    map_colour(&mut b, |i, c| {
        let mut c = c;
        // Basic Correction: white balance, exposure, contrast, tone, saturation.
        if temp != 0.0 || tint != 0.0 {
            c = [c[0] * (1.0 + 0.25 * temp), c[1] * (1.0 - 0.25 * tint), c[2] * (1.0 - 0.25 * temp)];
        }
        if expo != 1.0 {
            c = c.map(|v| v * expo);
        }
        if contrast != 0.0 {
            c = c.map(|v| 0.5 + (v - 0.5) * (1.0 + contrast));
        }
        if hi != 0.0 || sh != 0.0 || wh != 0.0 || bl != 0.0 {
            let l = luminance(c[0], c[1], c[2]).max(0.0);
            let lt = l.min(1.0);
            let d = 0.25 * hi * smoothstep(0.5, 1.0, lt) + 0.25 * sh * (1.0 - smoothstep(0.0, 0.5, lt)) + 0.2 * wh * lt * lt + 0.2 * bl * (1.0 - lt).powi(2);
            c = c.map(|v| v + d);
        }
        let l = luminance(c[0], c[1], c[2]);
        if sat != 1.0 {
            c = c.map(|v| l + (v - l) * sat);
        }
        // Creative.
        if intensity != 0.0 && (faded != 0.0 || vib != 0.0 || csat != 1.0 || st != [0.0; 3] || ht != [0.0; 3]) {
            let mut d = c;
            if faded != 0.0 {
                d = d.map(|v| v * (1.0 - 0.25 * faded) + 0.12 * faded);
            }
            let l = luminance(d[0], d[1], d[2]);
            if vib != 0.0 {
                let mx = d[0].max(d[1]).max(d[2]);
                let mn = d[0].min(d[1]).min(d[2]);
                let s = if mx > 1e-6 { (mx - mn) / mx } else { 0.0 };
                let k = 1.0 + vib * (1.0 - s.clamp(0.0, 1.0));
                d = d.map(|v| l + (v - l) * k);
            }
            if csat != 1.0 {
                d = d.map(|v| l + (v - l) * csat);
            }
            let pivot = (0.5 + 0.25 * tbal).clamp(0.05, 0.95);
            let lt = l.clamp(0.0, 1.0);
            let wsh = 1.0 - smoothstep(0.0, pivot, lt);
            let whi = smoothstep(pivot, 1.0, lt);
            for k in 0..3 {
                d[k] += 0.5 * (st[k] * wsh + ht[k] * whi);
            }
            c = [lerp(c[0], d[0], intensity), lerp(c[1], d[1], intensity), lerp(c[2], d[2], intensity)];
        }
        // Curves.
        c = c.map(|v| tone_curve(v, cm[0], cm[1], cm[2]));
        c = [tone_curve(c[0], cr[0], cr[1], cr[2]), tone_curve(c[1], cg[0], cg[1], cg[2]), tone_curve(c[2], cb[0], cb[1], cb[2])];
        // Color wheels.
        if ws != [0.0; 3] || wm != [0.0; 3] || wh_ != [0.0; 3] {
            let lt = luminance(c[0], c[1], c[2]).clamp(0.0, 1.0);
            let a = 1.0 - smoothstep(0.0, 0.5, lt);
            let z = smoothstep(0.5, 1.0, lt);
            let m = 1.0 - a - z;
            for k in 0..3 {
                c[k] += 0.5 * (ws[k] * a + wm[k] * m + wh_[k] * z);
            }
        }
        // Vignette.
        if vig != 0.0 {
            let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            let nx = ((x - cx) / (lw * 0.5).max(1e-6)) as f32;
            let ny = ((y - cy) / (lh * 0.5).max(1e-6)) as f32;
            // Roundness: 0 follows the frame's aspect, +1 a circle, -1 squarer.
            let aspect = (lw / lh.max(1e-6)) as f32;
            let (nx, ny) = if vround > 0.0 { (nx * lerp(1.0, aspect.max(1.0), vround), ny * lerp(1.0, (1.0 / aspect).max(1.0), vround)) } else { (nx, ny) };
            let pw = 2.0 + (-vround).max(0.0) * 6.0;
            let r = (nx.abs().powf(pw) + ny.abs().powf(pw)).powf(1.0 / pw) / std::f32::consts::SQRT_2;
            let start = vmid * 0.9;
            let f = smoothstep(start, start + 0.05 + vfeather * (1.05 - start), r);
            let k = if vig < 0.0 { 1.0 + vig * 0.2 * f } else { 1.0 };
            c = c.map(|v| if vig > 0.0 { v + (1.0 - v) * (vig * 0.2 * f).min(1.0) } else { v * k.max(0.0) });
        }
        c.map(|v| v.max(0.0))
    });
    if sharpen != 0.0 {
        let luma = Plane::luma(&b.img);
        let blur = gauss_plane(&luma, 1.0 * b.scale, 1.0 * b.scale);
        b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
            let (c, a) = unpremul(*px);
            if a <= 0.0 {
                return;
            }
            let d = (luma.data[i] - blur.data[i]) * sharpen * 2.0;
            *px = premul(c.map(|v| (v + d).max(0.0)), a);
        });
    }
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let grey = || col(0.5, 0.5, 0.5);
    let mut lum = vec![
        p("inputLut", "Input LUT", Value::Enum(0), popup(&["None"])),
        p("temperature", "Temperature", num(0.0), bipolar()),
        p("tint", "Tint", num(0.0), bipolar()),
        p("exposure", "Exposure", num(0.0), slider(-5.0, 5.0, -5.0, 5.0, 2)),
        p("contrast", "Contrast", num(0.0), bipolar()),
        p("highlights", "Highlights", num(0.0), bipolar()),
        p("shadows", "Shadows", num(0.0), bipolar()),
        p("whites", "Whites", num(0.0), bipolar()),
        p("blacks", "Blacks", num(0.0), bipolar()),
        p("saturation", "Saturation", num(100.0), slider(0.0, 200.0, 0.0, 200.0, 1)),
        p("look", "Look", Value::Enum(0), popup(&["None"])),
        p("lookIntensity", "Intensity", num(100.0), slider(0.0, 200.0, 0.0, 200.0, 1)),
        p("fadedFilm", "Faded Film", num(0.0), pct()),
        p("sharpen", "Sharpen", num(0.0), bipolar()),
        p("vibrance", "Vibrance", num(0.0), bipolar()),
        p("creativeSaturation", "Saturation", num(100.0), slider(0.0, 200.0, 0.0, 200.0, 1)),
        p("shadowTint", "Shadow Tint", grey(), ParamUi::Color),
        p("highlightTint", "Highlight Tint", grey(), ParamUi::Color),
        p("tintBalance", "Tint Balance", num(0.0), bipolar()),
    ];
    const CURVES: [(&str, &str); 12] = [
        ("curveMasterShadows", "RGB Curve Shadows"),
        ("curveMasterMidtones", "RGB Curve Midtones"),
        ("curveMasterHighlights", "RGB Curve Highlights"),
        ("curveRedShadows", "Red Curve Shadows"),
        ("curveRedMidtones", "Red Curve Midtones"),
        ("curveRedHighlights", "Red Curve Highlights"),
        ("curveGreenShadows", "Green Curve Shadows"),
        ("curveGreenMidtones", "Green Curve Midtones"),
        ("curveGreenHighlights", "Green Curve Highlights"),
        ("curveBlueShadows", "Blue Curve Shadows"),
        ("curveBlueMidtones", "Blue Curve Midtones"),
        ("curveBlueHighlights", "Blue Curve Highlights"),
    ];
    for (id, name) in CURVES {
        lum.push(p(id, name, num(0.0), bipolar()));
    }
    lum.extend([
        p("shadowsWheel", "Shadows", grey(), ParamUi::Color),
        p("midtonesWheel", "Midtones", grey(), ParamUi::Color),
        p("highlightsWheel", "Highlights", grey(), ParamUi::Color),
        p("vignetteAmount", "Amount", num(0.0), slider(-5.0, 5.0, -5.0, 5.0, 2)),
        p("vignetteMidpoint", "Midpoint", num(50.0), pct()),
        p("vignetteRoundness", "Roundness", num(0.0), bipolar()),
        p("vignetteFeather", "Feather", num(50.0), pct()),
    ]);
    vec![
        spec(
            "ec.color.changecolor",
            "Change Color",
            vec![
                p("view", "View", Value::Enum(0), popup(&["Corrected Layer", "Color Correction Mask"])),
                p("hueTransform", "Hue Transform", num(0.0), slider(-360.0, 360.0, -180.0, 180.0, 1)),
                p("lightnessTransform", "Lightness Transform", num(0.0), bipolar()),
                p("saturationTransform", "Saturation Transform", num(0.0), bipolar()),
                p("colorToChange", "Color To Change", col(0.8, 0.2, 0.2), ParamUi::Color),
                p("matchingTolerance", "Matching Tolerance", num(15.0), pct()),
                p("matchingSoftness", "Matching Softness", num(0.0), pct()),
                p("matchColors", "Match colors", Value::Enum(1), popup(&["Using RGB", "Using Hue", "Using Chroma"])),
                p("invertColorCorrectionMask", "Invert Color Correction Mask", Value::Bool(false), ParamUi::Checkbox),
            ],
            change_color,
        ),
        spec(
            "ec.color.videolimiter",
            "Video Limiter",
            vec![
                p("clipLevel", "Clip Level", Value::Enum(2), popup(&["90%", "95%", "100%", "105%", "109%"])),
                p("clipMethod", "Clip Method", Value::Enum(2), popup(&["Luma", "Chroma", "Smart Limit"])),
                p("compressionBeforeClipping", "Compression before Clipping", Value::Enum(2), popup(&["None", "3%", "5%", "10%", "20%"])),
                p("gamutWarning", "Gamut Warning", Value::Bool(false), ParamUi::Checkbox),
                p("gamutWarningColor", "Gamut Warning Color", col(1.0, 0.0, 0.0), ParamUi::Color),
            ],
            video_limiter,
        ),
        spec(
            "ec.color.cccolorneutralizer",
            "CC Color Neutralizer",
            vec![
                p("shadowsUnbalance", "Shadows Unbalance", col(0.0, 0.0, 0.0), ParamUi::Color),
                p("shadowsBalance", "Shadows Balance", col(0.0, 0.0, 0.0), ParamUi::Color),
                p("midtonesUnbalance", "Midtones Unbalance", grey(), ParamUi::Color),
                p("midtonesBalance", "Midtones Balance", grey(), ParamUi::Color),
                p("highlightsUnbalance", "Highlights Unbalance", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("highlightsBalance", "Highlights Balance", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("pinning", "Pinning", num(0.0), pct()),
                p("blendWOriginal", "Blend w. Original", num(0.0), pct()),
                p("darks", "Darks", num(0.0), bipolar()),
                p("brights", "Brights", num(0.0), bipolar()),
                p("contrast", "Contrast", num(0.0), bipolar()),
            ],
            color_neutralizer,
        ),
        spec(
            "ec.color.psarbitrarymap",
            "PS Arbitrary Map",
            vec![
                p("phase", "Phase", num(0.0), slider(-1000.0, 1000.0, 0.0, 255.0, 0)),
                p("applyPhaseMapToAlpha", "Apply Phase Map To Alpha", Value::Bool(false), ParamUi::Checkbox),
                p("map", "Map", Value::Str(String::new()), ParamUi::Hidden),
            ],
            arbitrary_map,
        ),
        spec("ec.color.lumetri", "Lumetri Color", lum, lumetri),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, run_fx};
    use effectcraft_raster::Image;

    fn ramp() -> Image {
        let mut img = Image::new(16, 8);
        for y in 0..8 {
            for x in 0..16 {
                let a = if y < 7 { 1.0 } else { 0.5 };
                img.set(x, y, [x as f32 / 16.0 * a, (y as f32 / 8.0) * a, 0.3 * a, a]);
            }
        }
        img
    }

    fn close(a: &Image, b: &Image, tol: f32) -> bool {
        a.data.iter().zip(b.data.iter()).all(|(p, q)| p.iter().zip(q.iter()).all(|(x, y)| (x - y).abs() <= tol))
    }

    fn run(id: &str, vals: &[(&str, Value)], img: Image) -> Image {
        run_fx(id, vals, img, 0.0, EffectEnv::default()).img
    }

    #[test]
    fn change_color_zero_is_identity_and_hue_shift_moves_red() {
        let img = ramp();
        assert!(close(&run("ec.color.changecolor", &[], img.clone()), &img, 0.0));
        let red = Image::filled(4, 4, [0.8, 0.2, 0.2, 1.0]);
        let out = run("ec.color.changecolor", &[("hueTransform", num(120.0))], red.clone());
        let p = out.get(1, 1);
        assert!(p[1] > p[0] && p[1] > p[2], "{p:?}");
        let blue = Image::filled(4, 4, [0.1, 0.2, 0.9, 1.0]);
        assert!(close(&run("ec.color.changecolor", &[("hueTransform", num(120.0))], blue.clone()), &blue, 1e-6));
        let mask = run("ec.color.changecolor", &[("view", Value::Enum(1))], red);
        assert!((mask.get(0, 0)[0] - 1.0).abs() < 1e-6);
        assert_eq!(out.data, run("ec.color.changecolor", &[("hueTransform", num(120.0))], Image::filled(4, 4, [0.8, 0.2, 0.2, 1.0])).data);
    }

    #[test]
    fn video_limiter_keeps_legal_and_limits_hot() {
        let mid = Image::filled(4, 4, [0.4, 0.5, 0.3, 1.0]);
        assert!(close(&run("ec.color.videolimiter", &[], mid.clone()), &mid, 1e-6));
        let hot = Image::filled(4, 4, [1.6, 1.2, 0.9, 1.0]);
        let out = run("ec.color.videolimiter", &[], hot);
        assert!(out.data.iter().all(|p| p[0] <= 1.0 + 1e-6 && p[1] <= 1.0 + 1e-6));
        let warn = run("ec.color.videolimiter", &[("gamutWarning", Value::Bool(true))], Image::filled(2, 2, [1.6, 1.2, 0.9, 1.0]));
        assert_eq!(warn.get(0, 0), [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn color_neutralizer_defaults_identity_and_removes_cast() {
        let img = ramp();
        assert!(close(&run("ec.color.cccolorneutralizer", &[], img.clone()), &img, 1e-6));
        let cast = Image::filled(4, 4, [0.6, 0.5, 0.4, 1.0]);
        let out = run("ec.color.cccolorneutralizer", &[("midtonesUnbalance", col(0.6, 0.5, 0.4)), ("midtonesBalance", col(0.5, 0.5, 0.5))], cast);
        let p = out.get(0, 0);
        assert!((p[0] - p[2]).abs() < 0.1, "{p:?}");
    }

    #[test]
    fn arbitrary_map_identity_and_inverting_map() {
        let img = ramp();
        assert!(close(&run("ec.color.psarbitrarymap", &[], img.clone()), &img, 1e-5));
        let inv: Vec<String> = (0..256).map(|i| (255 - i).to_string()).collect();
        let out = run("ec.color.psarbitrarymap", &[("map", Value::Str(inv.join(",")))], Image::filled(2, 2, [0.25, 0.5, 1.0, 1.0]));
        let p = out.get(0, 0);
        assert!((p[0] - 0.75).abs() < 0.01 && p[2].abs() < 0.01, "{p:?}");
    }

    #[test]
    fn lumetri_defaults_identity_and_controls_act() {
        let img = ramp();
        assert!(close(&run("ec.color.lumetri", &[], img.clone()), &img, 1e-5));
        let g = Image::filled(8, 8, [0.4, 0.4, 0.4, 1.0]);
        let e = run("ec.color.lumetri", &[("exposure", num(1.0))], g.clone());
        assert!((e.get(0, 0)[0] - 0.8).abs() < 1e-5);
        let warm = run("ec.color.lumetri", &[("temperature", num(50.0))], g.clone());
        assert!(warm.get(0, 0)[0] > warm.get(0, 0)[2]);
        let bw = run("ec.color.lumetri", &[("saturation", num(0.0))], img.clone());
        let p = bw.get(5, 3);
        assert!((p[0] - p[1]).abs() < 1e-5 && (p[1] - p[2]).abs() < 1e-5);
        let v = run("ec.color.lumetri", &[("vignetteAmount", num(-3.0))], g.clone());
        assert!(v.get(0, 0)[0] < v.get(4, 4)[0]);
        let c = run("ec.color.lumetri", &[("curveMasterMidtones", num(50.0))], g.clone());
        assert!(c.get(0, 0)[0] > 0.4);
        assert_eq!(c.data, run("ec.color.lumetri", &[("curveMasterMidtones", num(50.0))], g).data);
    }
}
