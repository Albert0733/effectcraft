//! Keying effects: colour, luma, difference, range and screen keyers plus spill suppression and
//! matte clean-up. Written from public behaviour descriptions and the standard colour-difference /
//! blue-screen matting literature (Vlahos; Smith & Blinn, "Blue Screen Matting").

use effectcraft_color::{luminance, rgb_to_hsl, srgb_to_linear};
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::{Image, gaussian_blur};
use rayon::prelude::*;

use crate::util::{Plane, gauss_plane, morph_frac, premul, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, category: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category, params, render, gpu: false, float: true }
}

fn pct() -> ParamUi {
    slider(0.0, 100.0, 0.0, 100.0, 1)
}

fn view_popup() -> ParamUi {
    popup(&["Final Output", "Source Only", "Matte Only"])
}

/// Apply a keyed matte `m` (1 = keep) to the image according to the view mode.
fn apply_matte(img: &mut Image, m: &Plane, view: u32) {
    if view == 1 {
        return;
    }
    img.data.par_iter_mut().zip(m.data.par_iter()).for_each(|(px, &k)| {
        let k = k.clamp(0.0, 1.0);
        if view == 2 {
            let a = px[3] * k;
            *px = [a, a, a, 1.0];
        } else {
            for c in px.iter_mut() {
                *c *= k;
            }
        }
    });
}

/// Edge thin (positive erodes, negative grows) and feather (Gaussian) of a matte, in buffer pixels.
fn thin_feather(m: Plane, thin: f64, feather: f64) -> Plane {
    let m = if thin > 0.0 {
        morph_frac(&m, thin, false)
    } else if thin < 0.0 {
        morph_frac(&m, -thin, true)
    } else {
        m
    };
    if feather > 0.0 { gauss_plane(&m, feather * 0.5, feather * 0.5) } else { m }
}

/// Plane of `f(straight colour, alpha)` over the image.
fn matte_of(img: &Image, f: impl Fn([f32; 3], f32) -> f32 + Sync) -> Plane {
    Plane::from_image(img, |px| {
        let (c, a) = unpremul(px);
        f(c, a)
    })
}

/// Index of the largest component and the other two.
fn primary(c: [f32; 4]) -> (usize, usize, usize) {
    if c[1] >= c[0] && c[1] >= c[2] {
        (1, 0, 2)
    } else if c[2] >= c[0] && c[2] >= c[1] {
        (2, 0, 1)
    } else {
        (0, 1, 2)
    }
}

// ---- Linear Color Key ----

fn hue_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (ha, sa, _) = rgb_to_hsl(a[0], a[1], a[2]);
    let (hb, _, _) = rgb_to_hsl(b[0], b[1], b[2]);
    if sa < 0.02 {
        return 1.0;
    }
    let d = (ha - hb).abs();
    d.min(1.0 - d) * 2.0
}

fn chroma(c: [f32; 3]) -> [f32; 2] {
    let s = c[0] + c[1] + c[2];
    if s <= 1e-6 { [1.0 / 3.0, 1.0 / 3.0] } else { [c[0] / s, c[1] / s] }
}

fn linear_color_key(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let k = ctx.params.color("keyColor");
    let key = [k[0], k[1], k[2]];
    let mode = ctx.params.e("matchColors");
    let tol = ctx.params.f("tolerance") as f32 / 100.0;
    let soft = ctx.params.f("softness") as f32 / 100.0;
    let keep = ctx.params.e("keyOperation") == 1;
    let kc = chroma(key);
    let m = matte_of(&b.img, |c, _| {
        let d = match mode {
            1 => hue_dist(c, key),
            2 => {
                let pc = chroma(c);
                ((pc[0] - kc[0]).powi(2) + (pc[1] - kc[1]).powi(2)).sqrt() / std::f32::consts::SQRT_2
            }
            _ => ((c[0] - key[0]).powi(2) + (c[1] - key[1]).powi(2) + (c[2] - key[2]).powi(2)).sqrt() / 3f32.sqrt(),
        };
        let v = if soft > 1e-6 {
            ((d - tol) / soft).clamp(0.0, 1.0)
        } else if d > tol {
            1.0
        } else {
            0.0
        };
        if keep { 1.0 - v } else { v }
    });
    apply_matte(&mut b.img, &m, ctx.params.e("view"));
    b
}

// ---- Color Key (obsolete) ----

fn color_key(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let k = ctx.params.color("keyColor");
    let tol = ctx.params.f("colorTolerance") as f32 / 255.0;
    let m = matte_of(&b.img, |c, _| {
        let d = (c[0] - k[0]).abs().max((c[1] - k[1]).abs()).max((c[2] - k[2]).abs());
        if d <= tol { 0.0 } else { 1.0 }
    });
    let m = thin_feather(m, ctx.params.f("edgeThin") * b.scale, ctx.params.f("edgeFeather") * b.scale);
    apply_matte(&mut b.img, &m, 0);
    b
}

// ---- Luma Key ----

fn luma_key(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let kind = ctx.params.e("keyType");
    let thr = ctx.params.f("threshold") as f32 / 255.0;
    let tol = ctx.params.f("tolerance") as f32 / 255.0;
    let m = matte_of(&b.img, |c, _| {
        let l = luminance(c[0], c[1], c[2]);
        match kind {
            0 => {
                if tol > 0.0 {
                    ((thr + tol - l) / tol).clamp(0.0, 1.0)
                } else if l > thr {
                    0.0
                } else {
                    1.0
                }
            }
            2 => {
                if (l - thr).abs() <= tol {
                    0.0
                } else {
                    1.0
                }
            }
            3 => {
                if (l - thr).abs() <= tol {
                    1.0
                } else {
                    0.0
                }
            }
            _ => {
                if tol > 0.0 {
                    ((l - (thr - tol)) / tol).clamp(0.0, 1.0)
                } else if l < thr {
                    0.0
                } else {
                    1.0
                }
            }
        }
    });
    let m = thin_feather(m, ctx.params.f("edgeThin") * b.scale, ctx.params.f("edgeFeather") * b.scale);
    apply_matte(&mut b.img, &m, 0);
    b
}

// ---- Color Difference Key ----

fn color_difference_key(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let k = ctx.params.color("keyColor");
    let (pi, o1, o2) = primary(k);
    let kd = (k[pi] - k[o1].max(k[o2])).max(1e-3);
    let black = ctx.params.f("blackLevel") as f32 / 255.0;
    let white = ctx.params.f("whiteLevel") as f32 / 255.0;
    let gamma = ctx.params.f("gamma").max(0.01) as f32;
    let m = matte_of(&b.img, |c, _| {
        let diff = c[pi] - c[o1].max(c[o2]);
        let a = 1.0 - (diff / kd).clamp(0.0, 1.0);
        let v = ((a - black) / (white - black).max(1e-4)).clamp(0.0, 1.0);
        v.powf(1.0 / gamma)
    });
    apply_matte(&mut b.img, &m, ctx.params.e("view"));
    b
}

// ---- Color Range ----

fn to_lab(c: [f32; 3]) -> [f32; 3] {
    let l = c.map(|v| srgb_to_linear(v.clamp(0.0, 1.0)));
    let x = (0.4124 * l[0] + 0.3576 * l[1] + 0.1805 * l[2]) / 0.95047;
    let y = 0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2];
    let z = (0.0193 * l[0] + 0.1192 * l[1] + 0.9505 * l[2]) / 1.08883;
    let f = |t: f32| if t > 0.008_856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// Colour in the chosen space as three 0..255 components.
fn range_components(space: u32, c: [f32; 3]) -> [f32; 3] {
    match space {
        1 => {
            let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
            let u = -0.147_13 * c[0] - 0.288_86 * c[1] + 0.436 * c[2];
            let v = 0.615 * c[0] - 0.514_99 * c[1] - 0.100_01 * c[2];
            [y * 255.0, u * 255.0 + 128.0, v * 255.0 + 128.0]
        }
        2 => c.map(|v| v * 255.0),
        _ => {
            let lab = to_lab(c);
            [lab[0] * 2.55, lab[1] + 128.0, lab[2] + 128.0]
        }
    }
}

fn color_range(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let space = ctx.params.e("colorSpace");
    let lo = ["minL", "minA", "minB"].map(|i| ctx.params.f(i) as f32);
    let hi = ["maxL", "maxA", "maxB"].map(|i| ctx.params.f(i) as f32);
    let fuzz = ctx.params.f("fuzziness") as f32;
    let m = matte_of(&b.img, |c, _| {
        let v = range_components(space, c);
        let mut out = 0.0f32;
        for i in 0..3 {
            out = out.max(lo[i] - v[i]).max(v[i] - hi[i]);
        }
        if out <= 0.0 {
            0.0
        } else if fuzz > 0.0 {
            (out / fuzz).clamp(0.0, 1.0)
        } else {
            1.0
        }
    });
    apply_matte(&mut b.img, &m, 0);
    b
}

// ---- Extract ----

fn extract(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let ch = ctx.params.e("channel");
    let bp = ctx.params.f("blackPoint") as f32;
    let wp = ctx.params.f("whitePoint") as f32;
    let bs = ctx.params.f("blackSoftness") as f32;
    let ws = ctx.params.f("whiteSoftness") as f32;
    let inv = ctx.params.b("invert");
    let m = matte_of(&b.img, |c, a| {
        let v = 255.0
            * match ch {
                1 => c[0],
                2 => c[1],
                3 => c[2],
                4 => a,
                _ => luminance(c[0], c[1], c[2]),
            };
        let mut k = 1.0f32;
        if v < bp {
            k = if bs > 0.0 { (1.0 - (bp - v) / bs).clamp(0.0, 1.0) } else { 0.0 };
        } else if v > wp {
            k = if ws > 0.0 { (1.0 - (v - wp) / ws).clamp(0.0, 1.0) } else { 0.0 };
        }
        if inv { 1.0 - k } else { k }
    });
    apply_matte(&mut b.img, &m, 0);
    b
}

// ---- Screen Key (Keylight-style) ----

/// Secondary-channel estimate: `balance` weights the larger of the two secondaries.
#[inline]
fn secondary(c: [f32; 3], o1: usize, o2: usize, bal: f32) -> f32 {
    let (hi, lo) = if c[o1] > c[o2] { (c[o1], c[o2]) } else { (c[o2], c[o1]) };
    bal * hi + (1.0 - bal) * lo
}

fn screen_key(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let view = ctx.params.e("view");
    if view == 3 {
        return b;
    }
    let sc = ctx.params.color("screenColor");
    let screen = [sc[0], sc[1], sc[2]];
    let (pi, o1, o2) = primary(sc);
    let gain = ctx.params.f("screenGain") as f32 / 100.0;
    let bal = ctx.params.f("screenBalance") as f32 / 100.0;
    let despill = ctx.params.f("despill") as f32 / 100.0;
    let cb = ctx.params.f("clipBlack") as f32 / 100.0;
    let cw = ctx.params.f("clipWhite") as f32 / 100.0;
    let preblur = ctx.params.f("screenPreblur") * b.scale;
    let sg = ctx.params.f("screenShrinkGrow") * b.scale;
    let soft = ctx.params.f("screenSoftness") * b.scale;
    let source_edges = ctx.params.e("edgeColor") == 1;
    let sd = (screen[pi] - secondary(screen, o1, o2, bal)).max(1e-3);

    let blurred;
    let key_src = if preblur > 0.0 {
        blurred = gaussian_blur(&b.img, preblur * 0.5, preblur * 0.5, true);
        &blurred
    } else {
        &b.img
    };
    let mut m = matte_of(key_src, |c, _| {
        let d = c[pi] - secondary(c, o1, o2, bal);
        let t = (d / sd * gain).clamp(0.0, 1.0);
        ((1.0 - t - cb) / (cw - cb).max(1e-3)).clamp(0.0, 1.0)
    });
    if sg < 0.0 {
        m = morph_frac(&m, -sg, false);
    } else if sg > 0.0 {
        m = morph_frac(&m, sg, true);
    }
    if soft > 0.0 {
        m = gauss_plane(&m, soft * 0.5, soft * 0.5);
    }
    b.img.data.par_iter_mut().zip(m.data.par_iter()).for_each(|(px, &k)| {
        let k = k.clamp(0.0, 1.0);
        let (c, a0) = unpremul(*px);
        let alpha = a0 * k;
        match view {
            1 => *px = [alpha, alpha, alpha, 1.0],
            2 => {
                let v = if alpha <= 1e-3 {
                    0.0
                } else if alpha >= 0.999 {
                    1.0
                } else {
                    0.5
                };
                *px = [v, v, v, 1.0];
            }
            _ => {
                let mut fg = if source_edges || k <= 1e-4 {
                    c
                } else {
                    let s = 1.0 - k;
                    [0, 1, 2].map(|i| ((c[i] - s * screen[i]).max(0.0) / k).min(c[i].max(1.0)))
                };
                let spill = (fg[pi] - secondary(fg, o1, o2, bal)).max(0.0);
                fg[pi] -= spill * despill;
                *px = premul(fg, alpha);
            }
        }
    });
    b
}

// ---- Spill suppression ----

fn spill_suppressor(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let k = ctx.params.color("colorToSuppress");
    let (pi, o1, o2) = primary(k);
    let amt = ctx.params.f("suppression") as f32 / 100.0;
    b.img.map_straight(|mut c| {
        let spill = (c[pi] - c[o1].max(c[o2])).max(0.0);
        c[pi] -= spill * amt;
        c
    });
    b
}

const LUMA_W: [f32; 3] = [0.2126, 0.7152, 0.0722];

fn advanced_spill(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let ultra = ctx.params.e("method") == 1;
    let pi = if ultra {
        primary(ctx.params.color("keyColor")).0
    } else {
        // Dominant key primary: the channel with the largest total excess over the other two.
        let ex = b
            .img
            .data
            .par_iter()
            .map(|px| {
                let (c, a) = unpremul(*px);
                [(c[0] - c[1].max(c[2])).max(0.0) * a, (c[1] - c[0].max(c[2])).max(0.0) * a, (c[2] - c[0].max(c[1])).max(0.0) * a]
            })
            .reduce(|| [0.0; 3], |x, y| [x[0] + y[0], x[1] + y[1], x[2] + y[2]]);
        if ex[1] >= ex[0] && ex[1] >= ex[2] {
            1
        } else if ex[2] >= ex[0] {
            2
        } else {
            0
        }
    };
    let (o1, o2) = match pi {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let amt = ctx.params.f("suppression") as f32 / 100.0;
    let range = ctx.params.f("spillRange") as f32 / 100.0;
    let luma = ctx.params.f("lumaCorrection") as f32 / 100.0;
    b.img.map_straight(|mut c| {
        let mx = c[o1].max(c[o2]);
        let avg = 0.5 * (c[o1] + c[o2]);
        let limit = mx + (avg - mx) * range;
        let spill = (c[pi] - limit).max(0.0) * amt;
        c[pi] -= spill;
        let back = spill * LUMA_W[pi] * luma;
        c.map(|v| v + back)
    });
    b
}

// ---- Key Cleaner (approximation) ----

fn key_cleaner(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let r = (ctx.params.f("additionalEdgeRadius") * b.scale).max(0.0);
    let contrast = ctx.params.f("alphaContrast") as f32 / 100.0;
    let strength = ctx.params.f("strength") as f32 / 100.0;
    let mut a = Plane::alpha(&b.img);
    if ctx.params.b("reduceChatter") {
        a = gauss_plane(&a, 1.0, 1.0).map(|v| ((v - 0.5) * 1.5 + 0.5).clamp(0.0, 1.0));
    }
    let est = if r > 0.0 && strength > 0.0 { Some(gaussian_blur(&b.img, r * 0.5, r * 0.5, true)) } else { None };
    b.img.data.par_iter_mut().zip(a.data.par_iter()).enumerate().for_each(|(i, (px, &na))| {
        let (mut c, a0) = unpremul(*px);
        if let Some(e) = &est
            && a0 > 1e-4
            && a0 < 0.999
        {
            let (ec, ea) = unpremul(e.data[i]);
            if ea > 1e-4 {
                let t = strength * (1.0 - a0);
                c = [0, 1, 2].map(|k| c[k] + (ec[k] - c[k]) * t);
            }
        }
        let na = ((na - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
        let na = if a0 <= 1e-6 { 0.0 } else { na };
        *px = premul(c, na);
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let thin = || slider(-5.0, 5.0, -5.0, 5.0, 1);
    let feather = || slider(0.0, 100.0, 0.0, 20.0, 1);
    let b255 = || slider(0.0, 255.0, 0.0, 255.0, 0);
    vec![
        spec(
            "ec.key.linearcolor",
            "Linear Color Key",
            "Keying",
            vec![
                p("keyColor", "Key Color", col(0.0, 0.0, 1.0), ParamUi::Color),
                p("matchColors", "Match colors", Value::Enum(0), popup(&["Using RGB", "Using Hue", "Using Chroma"])),
                p("tolerance", "Matching Tolerance", num(10.0), pct()),
                p("softness", "Matching Softness", num(10.0), pct()),
                p("keyOperation", "Key Operation", Value::Enum(0), popup(&["Key Colors", "Keep Colors"])),
                p("view", "View", Value::Enum(0), view_popup()),
            ],
            linear_color_key,
        ),
        spec(
            "ec.key.colorkey",
            "Color Key",
            "Obsolete",
            vec![
                p("keyColor", "Key Color", col(0.0, 0.0, 1.0), ParamUi::Color),
                p("colorTolerance", "Color Tolerance", num(0.0), b255()),
                p("edgeThin", "Edge Thin", num(0.0), thin()),
                p("edgeFeather", "Edge Feather", num(0.0), feather()),
            ],
            color_key,
        ),
        spec(
            "ec.key.luma",
            "Luma Key",
            "Obsolete",
            vec![
                p("keyType", "Key Type", Value::Enum(1), popup(&["Key Out Brighter", "Key Out Darker", "Key Out Similar", "Key Out Dissimilar"])),
                p("threshold", "Threshold", num(0.0), b255()),
                p("tolerance", "Tolerance", num(0.0), b255()),
                p("edgeThin", "Edge Thin", num(0.0), thin()),
                p("edgeFeather", "Edge Feather", num(0.0), feather()),
            ],
            luma_key,
        ),
        spec(
            "ec.key.colordifference",
            "Color Difference Key",
            "Keying",
            vec![
                p("keyColor", "Key Color", col(0.0, 0.0, 1.0), ParamUi::Color),
                p("blackLevel", "Matte Black", num(0.0), b255()),
                p("whiteLevel", "Matte White", num(255.0), b255()),
                p("gamma", "Matte Gamma", num(1.0), slider(0.01, 10.0, 0.1, 3.0, 2)),
                p("view", "View", Value::Enum(0), view_popup()),
            ],
            color_difference_key,
        ),
        spec(
            "ec.key.colorrange",
            "Color Range",
            "Keying",
            vec![
                p("colorSpace", "Color Space", Value::Enum(0), popup(&["Lab", "YUV", "RGB"])),
                p("fuzziness", "Fuzziness", num(0.0), b255()),
                p("minL", "Min (L, Y, R)", num(0.0), b255()),
                p("maxL", "Max (L, Y, R)", num(255.0), b255()),
                p("minA", "Min (a, U, G)", num(0.0), b255()),
                p("maxA", "Max (a, U, G)", num(100.0), b255()),
                p("minB", "Min (b, V, B)", num(128.0), b255()),
                p("maxB", "Max (b, V, B)", num(255.0), b255()),
            ],
            color_range,
        ),
        spec(
            "ec.key.extract",
            "Extract",
            "Keying",
            vec![
                p("channel", "Channel", Value::Enum(0), popup(&["Luminance", "Red", "Green", "Blue", "Alpha"])),
                p("blackPoint", "Black Point", num(0.0), b255()),
                p("whitePoint", "White Point", num(255.0), b255()),
                p("blackSoftness", "Black Softness", num(0.0), b255()),
                p("whiteSoftness", "White Softness", num(0.0), b255()),
                p("invert", "Invert", Value::Bool(false), ParamUi::Checkbox),
            ],
            extract,
        ),
        spec(
            "ec.key.screen",
            "Screen Key",
            "Keying",
            vec![
                p("view", "View", Value::Enum(0), popup(&["Final Result", "Screen Matte", "Status", "Source"])),
                p("screenColor", "Screen Colour", col(0.15, 0.75, 0.25), ParamUi::Color),
                p("screenGain", "Screen Gain", num(100.0), slider(0.0, 200.0, 0.0, 200.0, 1)),
                p("screenBalance", "Screen Balance", num(50.0), pct()),
                p("despill", "Despill", num(100.0), pct()),
                p("clipBlack", "Clip Black", num(0.0), pct()),
                p("clipWhite", "Clip White", num(100.0), pct()),
                p("screenPreblur", "Screen Pre-blur", num(0.0), slider(0.0, 100.0, 0.0, 10.0, 1)),
                p("screenShrinkGrow", "Screen Shrink/Grow", num(0.0), slider(-50.0, 50.0, -10.0, 10.0, 1)),
                p("screenSoftness", "Screen Softness", num(0.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
                p("edgeColor", "Edge Colour", Value::Enum(0), popup(&["Screen Subtract", "Source"])),
            ],
            screen_key,
        ),
        spec(
            "ec.key.spill",
            "Spill Suppressor",
            "Obsolete",
            vec![
                p("colorToSuppress", "Color To Suppress", col(0.0, 0.0, 1.0), ParamUi::Color),
                p("suppression", "Suppression", num(100.0), slider(0.0, 200.0, 0.0, 100.0, 0)),
            ],
            spill_suppressor,
        ),
        spec(
            "ec.key.advancedspill",
            "Advanced Spill Suppressor",
            "Keying",
            vec![
                p("method", "Method", Value::Enum(0), popup(&["Standard", "Ultra"])),
                p("suppression", "Suppression", num(100.0), slider(0.0, 200.0, 0.0, 100.0, 0)),
                p("keyColor", "Key Color", col(0.0, 1.0, 0.0), ParamUi::Color),
                p("spillRange", "Spill Range", num(50.0), pct()),
                p("lumaCorrection", "Luma Correction", num(0.0), pct()),
            ],
            advanced_spill,
        ),
        spec(
            "ec.key.keycleaner",
            "Key Cleaner",
            "Keying",
            vec![
                p("additionalEdgeRadius", "Additional Edge Radius", num(3.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
                p("reduceChatter", "Reduce Chatter", Value::Bool(false), ParamUi::Checkbox),
                p("alphaContrast", "Alpha Contrast", num(100.0), slider(100.0, 500.0, 100.0, 300.0, 0)),
                p("strength", "Strength", num(100.0), pct()),
            ],
            key_cleaner,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Params, apply, find};

    /// Left half green screen, right half red foreground.
    fn green_screen() -> Image {
        let mut img = Image::new(16, 8);
        for y in 0..8 {
            for x in 0..16 {
                img.set(x, y, if x < 8 { [0.1, 0.8, 0.15, 1.0] } else { [0.9, 0.2, 0.1, 1.0] });
            }
        }
        img
    }

    fn run(id: &str, over: &[(&str, Value)], img: Image) -> Image {
        let s = find(id).unwrap();
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        for (k, v) in over {
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx =
            EffectCtx { params: &params, time: 0.0, layer_size: [img.width as f64, img.height as f64], seed: 1, adjustment: false, env: Default::default() };
        apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 }).img
    }

    fn green() -> Value {
        Value::Color([0.1, 0.8, 0.15, 1.0])
    }

    #[test]
    fn linear_color_key_removes_green() {
        let out = run("ec.key.linearcolor", &[("keyColor", green())], green_screen());
        assert!(out.get(2, 2)[3] < 0.01);
        assert!(out.get(12, 2)[3] > 0.99);
    }

    #[test]
    fn color_difference_key_removes_green() {
        let out = run("ec.key.colordifference", &[("keyColor", green())], green_screen());
        assert!(out.get(2, 2)[3] < 0.01, "{:?}", out.get(2, 2));
        assert!(out.get(12, 2)[3] > 0.99);
    }

    #[test]
    fn screen_key_removes_green_and_despills() {
        let mut img = green_screen();
        img.set(12, 4, [0.6, 0.65, 0.5, 1.0]);
        let out = run("ec.key.screen", &[("screenColor", green())], img);
        assert!(out.get(2, 2)[3] < 0.01);
        assert!(out.get(12, 2)[3] > 0.99);
        let px = out.get(12, 4);
        let (c, a) = unpremul(px);
        assert!(a > 0.5 && a < 1.0, "{a}");
        assert!(c[1] <= 0.5 * (c[0] + c[2]) + 1e-3, "{c:?}");
    }

    #[test]
    fn screen_key_matte_view_is_opaque() {
        let out = run("ec.key.screen", &[("screenColor", green()), ("view", Value::Enum(1))], green_screen());
        assert!(out.data.iter().all(|p| p[3] == 1.0));
        assert!(out.get(2, 2)[0] < 0.01 && out.get(12, 2)[0] > 0.99);
    }

    #[test]
    fn color_range_keys_green_in_lab() {
        let out = run("ec.key.colorrange", &[], green_screen());
        assert!(out.get(2, 2)[3] < 0.01);
        assert!(out.get(12, 2)[3] > 0.99);
    }

    #[test]
    fn luma_key_darker() {
        let mut img = Image::new(4, 1);
        img.set(0, 0, [0.1, 0.1, 0.1, 1.0]);
        img.set(1, 0, [0.9, 0.9, 0.9, 1.0]);
        let out = run("ec.key.luma", &[("threshold", num(128.0))], img);
        assert_eq!(out.get(0, 0)[3], 0.0);
        assert_eq!(out.get(1, 0)[3], 1.0);
    }

    #[test]
    fn extract_keeps_range() {
        let mut img = Image::new(3, 1);
        img.set(0, 0, [0.05, 0.05, 0.05, 1.0]);
        img.set(1, 0, [0.5, 0.5, 0.5, 1.0]);
        img.set(2, 0, [1.0, 1.0, 1.0, 1.0]);
        let out = run("ec.key.extract", &[("blackPoint", num(50.0)), ("whitePoint", num(200.0))], img);
        assert_eq!(out.get(0, 0)[3], 0.0);
        assert_eq!(out.get(1, 0)[3], 1.0);
        assert_eq!(out.get(2, 0)[3], 0.0);
    }

    #[test]
    fn spill_suppressor_limits_green() {
        let img = Image::filled(2, 2, [0.3, 0.8, 0.4, 1.0]);
        let out = run("ec.key.spill", &[("colorToSuppress", green())], img);
        let px = out.get(0, 0);
        assert!((px[1] - 0.4).abs() < 1e-5 && px[0] == 0.3 && px[2] == 0.4);
    }

    #[test]
    fn advanced_spill_standard_detects_green() {
        let img = Image::filled(2, 2, [0.3, 0.8, 0.3, 1.0]);
        let out = run("ec.key.advancedspill", &[], img);
        assert!((out.get(0, 0)[1] - 0.3).abs() < 1e-5);
    }

    #[test]
    fn key_cleaner_defaults_keep_opaque_pixels() {
        let img = green_screen();
        let out = run("ec.key.keycleaner", &[], img.clone());
        assert_eq!(out.get(12, 2), img.get(12, 2));
    }
}
