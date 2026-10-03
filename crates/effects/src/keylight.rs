//! Key Light — a full screen-colour keyer with the controls of After Effects' bundled
//! "Keylight (1.2)" (that name is a trademark of its vendor, so the effect is registered as
//! **Key Light**; `lookup("Keylight (1.2)")` finds it through [`KEYLIGHT_ALIASES`]).
//!
//! The method is the textbook *screen difference* key, written from the public description of
//! the controls:
//!
//! * colours are first neutralised by the **Alpha Bias** / **Despill Bias** colours (each channel
//!   scaled so the bias colour becomes grey — mid grey = no change);
//! * the **screen difference** of a pixel is its screen-primary channel minus a balance-weighted
//!   mix of the other two (**Screen Balance**); the raw matte is
//!   `1 − Screen Gain × diff(pixel) / diff(screen colour)`;
//! * the **Screen Matte** is then clipped (Clip Black / White, with Clip Rollback restoring edge
//!   detail), despotted (morphological open/close: Despot Black / White), shrunk or grown and
//!   softened, combined with the **Inside** / **Outside** masks and the source alpha;
//! * the foreground is the source with the screen colour removed in proportion to the
//!   transparency (`(c − (1 − α)·S) / α`), then **despilled** (the primary is limited to the
//!   balanced secondary), with **Replace Method** colouring areas whose alpha was raised by
//!   clipping or the inside mask, and optional foreground and edge colour correction.
//!
//! Views: Source, Source Alpha, Corrected Source, Colour Correction Edges, Screen Matte, Inside
//! Mask, Outside Mask, Combined Matte, Status, Intermediate Result and Final Result.

use effectcraft_color::luminance;
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::{Image, gaussian_blur};
use rayon::prelude::*;

use crate::util::{Plane, gauss_plane, morph_frac, point_in_poly, premul, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

/// Other names the effect answers to in [`crate::lookup`].
pub const KEYLIGHT_ALIASES: &[&str] = &["Keylight (1.2)", "Keylight"];

pub const VIEWS: [&str; 11] = [
    "Source",
    "Source Alpha",
    "Corrected Source",
    "Colour Correction Edges",
    "Screen Matte",
    "Inside Mask",
    "Outside Mask",
    "Combined Matte",
    "Status",
    "Intermediate Result",
    "Final Result",
];

const REPLACE: [&str; 4] = ["None", "Source", "Hard Colour", "Soft Colour"];

fn pct(d: f64) -> (Value, ParamUi) {
    (num(d), slider(0.0, 100.0, 0.0, 100.0, 1))
}

fn mask_popup() -> ParamUi {
    popup(&["None", "Mask 1", "Mask 2", "Mask 3", "Mask 4", "Mask 5", "Mask 6", "Mask 7", "Mask 8", "Mask 9", "Mask 10"])
}

/// Index of the screen's primary channel and the two others (larger-in-screen first).
fn primary(s: [f32; 3]) -> (usize, usize, usize) {
    let pi = if s[1] >= s[0] && s[1] >= s[2] {
        1
    } else if s[2] >= s[0] {
        2
    } else {
        0
    };
    let (a, b) = match pi {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    if s[a] >= s[b] { (pi, a, b) } else { (pi, b, a) }
}

/// Screen difference: primary minus the balance-weighted secondaries.
#[inline]
pub fn screen_diff(c: [f32; 3], pi: usize, o1: usize, o2: usize, bal: f32) -> f32 {
    c[pi] - (bal * c[o1] + (1.0 - bal) * c[o2])
}

/// Scale channels so `bias` becomes neutral (grey keeps everything as is).
#[inline]
fn neutralise(c: [f32; 3], bias: [f32; 3]) -> [f32; 3] {
    let l = luminance(bias[0], bias[1], bias[2]).max(1e-4);
    [0, 1, 2].map(|i| c[i] * l / bias[i].max(1e-4))
}

fn mask_coverage(ctx: &EffectCtx, b: &Buf, idx: u32, softness: f64, invert: bool) -> Option<Plane> {
    let shape = ctx.env.masks.get((idx as usize).checked_sub(1)?)?;
    let (w, h) = (b.img.width as usize, b.img.height as usize);
    let mut pl = Plane::new(w, h);
    let inv = 1.0 / b.scale.max(1e-9);
    pl.data.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
        for (x, v) in row.iter_mut().enumerate() {
            let lx = (x as f64 + 0.5 - b.offset[0]) * inv;
            let ly = (y as f64 + 0.5 - b.offset[1]) * inv;
            *v = if point_in_poly(&shape.points, lx, ly) != shape.inverted { 1.0 } else { 0.0 };
        }
    });
    let s = softness * b.scale;
    if s > 0.0 {
        pl = gauss_plane(&pl, s * 0.5, s * 0.5);
    }
    if invert {
        pl = pl.map(|v| 1.0 - v);
    }
    Some(pl)
}

/// Saturation / contrast / brightness correction of a straight colour.
fn correct(c: [f32; 3], sat: f32, contrast: f32, bright: f32) -> [f32; 3] {
    let l = luminance(c[0], c[1], c[2]);
    let c = c.map(|v| l + (v - l) * sat);
    c.map(|v| ((v - 0.5) * contrast + 0.5) * bright)
}

fn key_light(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let view = pr.e("view");
    if view == 0 {
        return b;
    }
    let sc = pr.color("screenColour");
    let screen = [sc[0], sc[1], sc[2]];
    let (pi, o1, o2) = primary(screen);
    let gain = pr.f("screenGain") as f32 / 100.0;
    let bal = pr.f("screenBalance") as f32 / 100.0;
    let dbias = pr.color("despillBias");
    let abias = if pr.b("lockBiasesTogether") { dbias } else { pr.color("alphaBias") };
    let (dbias, abias) = ([dbias[0], dbias[1], dbias[2]], [abias[0], abias[1], abias[2]]);
    let s_alpha = neutralise(screen, abias);
    let sd = screen_diff(s_alpha, pi, o1, o2, bal).max(1e-3);
    let cb = pr.f("clipBlack") as f32 / 100.0;
    let cw = pr.f("clipWhite") as f32 / 100.0;
    let rollback = pr.f("clipRollback") * b.scale;
    let preblur = pr.f("screenPreblur") * b.scale;
    let sg = pr.f("screenShrinkGrow") * b.scale;
    let soft = pr.f("screenSoftness") * b.scale;
    let despot_b = pr.f("screenDespotBlack") * b.scale;
    let despot_w = pr.f("screenDespotWhite") * b.scale;

    let src = b.img.clone();
    let blurred;
    let key_src: &Image = if preblur > 0.0 {
        blurred = gaussian_blur(&src, preblur * 0.5, preblur * 0.5, true);
        &blurred
    } else {
        &src
    };
    // Raw screen matte (opacity of the foreground).
    let raw = Plane::from_image(key_src, |px| {
        let (c, _) = unpremul(px);
        let d = screen_diff(neutralise(c, abias), pi, o1, o2, bal);
        (1.0 - gain * d / sd).clamp(0.0, 1.0)
    });
    let clip = |v: f32| ((v - cb) / (cw - cb).max(1e-4)).clamp(0.0, 1.0);
    let mut m = raw.map(clip);
    if rollback > 0.0 {
        // Restore edge detail lost by clipping within `rollback` pixels of the clipped matte.
        let grown = morph_frac(&m, rollback, true);
        let shrunk = morph_frac(&m, rollback, false);
        m = Plane {
            w: m.w,
            h: m.h,
            data: (0..m.data.len())
                .into_par_iter()
                .map(|i| if grown.data[i] > shrunk.data[i] + 1e-4 { m.data[i].max(raw.data[i]).min(grown.data[i]) } else { m.data[i] })
                .collect(),
        };
    }
    if despot_b > 0.0 {
        // Fill small holes: close (dilate then erode).
        m = morph_frac(&morph_frac(&m, despot_b, true), despot_b, false);
    }
    if despot_w > 0.0 {
        // Remove small specks: open (erode then dilate).
        m = morph_frac(&morph_frac(&m, despot_w, false), despot_w, true);
    }
    if sg < 0.0 {
        m = morph_frac(&m, -sg, false);
    } else if sg > 0.0 {
        m = morph_frac(&m, sg, true);
    }
    if soft > 0.0 {
        m = gauss_plane(&m, soft * 0.5, soft * 0.5);
    }
    let screen_matte = m.clone();
    let inside = mask_coverage(ctx, &b, pr.e("insideMask"), pr.f("insideMaskSoftness"), pr.b("invertInsideMask"));
    let outside = mask_coverage(ctx, &b, pr.e("outsideMask"), pr.f("outsideMaskSoftness"), pr.b("invertOutsideMask"));
    let src_alpha_mode = pr.e("sourceAlpha");
    let n = src.data.len();
    let combined: Vec<f32> = (0..n)
        .into_par_iter()
        .map(|i| {
            let mut a = screen_matte.data[i];
            let sa = src.data[i][3].clamp(0.0, 1.0);
            let ins = inside.as_ref().map_or(0.0, |p| p.data[i]);
            let ins = if src_alpha_mode == 1 { ins.max(sa) } else { ins };
            a = a.max(ins);
            if let Some(o) = &outside {
                a *= 1.0 - o.data[i];
            }
            if src_alpha_mode == 2 {
                a *= sa;
            }
            a.clamp(0.0, 1.0)
        })
        .collect();
    let edges: Option<Vec<f32>> = pr.b("enableEdgeColourCorrection").then(|| {
        let grow = pr.f("edgeGrow") * b.scale;
        let hard = (pr.f("edgeHardness") / 100.0) as f32;
        let esoft = pr.f("edgeSoftness") * b.scale;
        // Edge band: where the (grown) combined matte is partial.
        let cm = Plane { w: m.w, h: m.h, data: combined.clone() };
        let band = cm.map(|a| if a > 1e-3 && a < 0.999 { 1.0 } else { 0.0 });
        let mut band = if grow > 0.0 { morph_frac(&band, grow, true) } else { band };
        if esoft > 0.0 {
            band = gauss_plane(&band, esoft * 0.5, esoft * 0.5);
        }
        band.data.iter().map(|v| (v * (1.0 + hard * 4.0)).min(1.0)).collect()
    });
    let replace = pr.e("replaceMethod");
    let rc = pr.color("replaceColour");
    let in_replace = pr.e("insideReplaceMethod");
    let in_rc = pr.color("insideReplaceColour");
    let fg_cc = pr.b("enableColourCorrection");
    let (fs, fc, fbr) = (pr.f("saturation") as f32 / 100.0, pr.f("contrast") as f32 / 100.0 + 1.0, pr.f("brightness") as f32 / 100.0 + 1.0);
    let (es, ec, ebr) = (pr.f("edgeSaturation") as f32 / 100.0, pr.f("edgeContrast") as f32 / 100.0 + 1.0, pr.f("edgeBrightness") as f32 / 100.0 + 1.0);
    let unpremultiply = pr.b("unpremultiplyResult");
    let crops = [pr.f("cropLeft"), pr.f("cropRight"), pr.f("cropTop"), pr.f("cropBottom")].map(|v| v / 100.0);
    let (lw, lh) = (ctx.layer_size[0], ctx.layer_size[1]);
    let w = b.img.width as usize;
    let (scale, off) = (b.scale, b.offset);
    b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let (c, _) = unpremul(src.data[i]);
        let a = combined[i];
        let sm = screen_matte.data[i];
        // Source crops (layer fractions from each edge).
        let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
        let (lx, ly) = ((x - off[0]) / scale / lw.max(1e-9), (y - off[1]) / scale / lh.max(1e-9));
        let cropped = lx < crops[0] || lx > 1.0 - crops[1] || ly < crops[2] || ly > 1.0 - crops[3];
        // Screen removal on the raw (unclipped) transparency, then despill.
        let ar = raw.data[i].max(1e-4);
        let mut fg = [0, 1, 2].map(|k| ((c[k] - (1.0 - ar) * screen[k]) / ar).max(0.0));
        let nfg = neutralise(fg, dbias);
        let spill = screen_diff(nfg, pi, o1, o2, bal).max(0.0);
        let l0 = (luminance(dbias[0], dbias[1], dbias[2]) / dbias[pi].max(1e-4)).max(1e-4);
        fg[pi] -= spill / l0;
        let intermediate = fg;
        // Replace colour where the alpha was raised beyond the raw screen matte.
        let raised = (sm - raw.data[i]).max(0.0);
        let raised_in = inside.as_ref().map_or(0.0, |p| (p.data[i] - sm).max(0.0));
        let apply_replace = |fg: [f32; 3], method: u32, rc: [f32; 4], amt: f32| -> [f32; 3] {
            if amt <= 0.0 {
                return fg;
            }
            match method {
                1 => [0, 1, 2].map(|k| fg[k] + (c[k] - fg[k]) * amt),
                2 => [rc[0], rc[1], rc[2]],
                3 => {
                    let l = luminance(c[0], c[1], c[2]) / luminance(rc[0], rc[1], rc[2]).max(1e-4);
                    [0, 1, 2].map(|k| fg[k] + (rc[k] * l - fg[k]) * amt)
                }
                _ => fg,
            }
        };
        fg = apply_replace(fg, replace, rc, raised);
        fg = apply_replace(fg, in_replace, in_rc, raised_in);
        if fg_cc {
            fg = correct(fg, fs, fc, fbr);
        }
        let edge = edges.as_ref().map_or(0.0, |e| e[i]);
        if edge > 0.0 {
            let ce = correct(fg, es, ec, ebr);
            fg = [0, 1, 2].map(|k| fg[k] + (ce[k] - fg[k]) * edge);
        }
        let grey = |v: f32| [v, v, v, 1.0];
        *px = match view {
            1 => grey(src.data[i][3]),
            2 => {
                let mut cs = neutralise(c, dbias);
                let s2 = screen_diff(cs, pi, o1, o2, bal).max(0.0);
                cs[pi] -= s2;
                if fg_cc {
                    cs = correct(cs, fs, fc, fbr);
                }
                [cs[0], cs[1], cs[2], 1.0]
            }
            3 => grey(edge),
            4 => grey(sm),
            5 => grey(inside.as_ref().map_or(0.0, |p| p.data[i])),
            6 => grey(outside.as_ref().map_or(0.0, |p| p.data[i])),
            7 => grey(a),
            8 => {
                let v = if a <= 1e-3 {
                    0.0
                } else if a >= 0.999 {
                    1.0
                } else {
                    0.5
                };
                grey(v)
            }
            9 => premul(intermediate, sm),
            _ => {
                if cropped {
                    [0.0; 4]
                } else if unpremultiply {
                    premul(fg, a)
                } else {
                    // Keylight's premultiplied output read as straight colour.
                    premul(fg.map(|v| v * a), a)
                }
            }
        };
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let pp = |id: &'static str, name: &'static str, v: (Value, ParamUi)| p(id, name, v.0, v.1);
    let cc = |id: &'static str, name: &'static str| p(id, name, num(0.0), slider(-100.0, 100.0, -100.0, 100.0, 1));
    vec![EffectSpec {
        id: "ec.keying.keylight",
        name: "Key Light",
        category: "Keying",
        params: vec![
            p("view", "View", Value::Enum(10), popup(&VIEWS)),
            p("unpremultiplyResult", "Unpremultiply Result", Value::Bool(true), ParamUi::Checkbox),
            p("screenColour", "Screen Colour", col(0.0, 1.0, 0.0), ParamUi::Color),
            p("screenGain", "Screen Gain", num(100.0), slider(0.0, 200.0, 0.0, 200.0, 1)),
            p("screenBalance", "Screen Balance", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            p("despillBias", "Despill Bias", col(0.5, 0.5, 0.5), ParamUi::Color),
            p("alphaBias", "Alpha Bias", col(0.5, 0.5, 0.5), ParamUi::Color),
            p("lockBiasesTogether", "Lock Biases Together", Value::Bool(true), ParamUi::Checkbox),
            p("screenPreblur", "Screen Pre-blur", num(0.0), slider(0.0, 100.0, 0.0, 10.0, 1)),
            // Screen Matte
            pp("clipBlack", "Clip Black", pct(0.0)),
            pp("clipWhite", "Clip White", pct(100.0)),
            p("clipRollback", "Clip Rollback", num(0.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
            p("screenShrinkGrow", "Screen Shrink/Grow", num(0.0), slider(-100.0, 100.0, -10.0, 10.0, 1)),
            p("screenSoftness", "Screen Softness", num(0.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
            p("screenDespotBlack", "Screen Despot Black", num(0.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
            p("screenDespotWhite", "Screen Despot White", num(0.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
            p("replaceMethod", "Replace Method", Value::Enum(3), popup(&REPLACE)),
            p("replaceColour", "Replace Colour", col(0.5, 0.5, 0.5), ParamUi::Color),
            // Inside Mask
            p("insideMask", "Inside Mask", Value::Enum(0), mask_popup()),
            p("insideMaskSoftness", "Inside Mask Softness", num(0.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
            p("invertInsideMask", "Invert", Value::Bool(false), ParamUi::Checkbox),
            p("insideReplaceMethod", "Inside Replace Method", Value::Enum(1), popup(&REPLACE)),
            p("insideReplaceColour", "Inside Replace Colour", col(0.5, 0.5, 0.5), ParamUi::Color),
            p("sourceAlpha", "Source Alpha", Value::Enum(2), popup(&["Ignore", "Add to Inside Mask", "Normal"])),
            // Outside Mask
            p("outsideMask", "Outside Mask", Value::Enum(0), mask_popup()),
            p("outsideMaskSoftness", "Outside Mask Softness", num(0.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
            p("invertOutsideMask", "Invert Outside Mask", Value::Bool(false), ParamUi::Checkbox),
            // Foreground Colour Correction
            p("enableColourCorrection", "Enable Colour Correction", Value::Bool(false), ParamUi::Checkbox),
            p("saturation", "Saturation", num(100.0), slider(0.0, 400.0, 0.0, 200.0, 1)),
            cc("contrast", "Contrast"),
            cc("brightness", "Brightness"),
            // Edge Colour Correction
            p("enableEdgeColourCorrection", "Enable Edge Colour Correction", Value::Bool(false), ParamUi::Checkbox),
            pp("edgeHardness", "Edge Hardness", pct(0.0)),
            p("edgeSoftness", "Edge Softness", num(0.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
            p("edgeGrow", "Edge Grow", num(1.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
            p("edgeSaturation", "Edge Saturation", num(100.0), slider(0.0, 400.0, 0.0, 200.0, 1)),
            cc("edgeContrast", "Edge Contrast"),
            cc("edgeBrightness", "Edge Brightness"),
            // Source Crops
            pp("cropLeft", "Left", pct(0.0)),
            pp("cropRight", "Right", pct(0.0)),
            pp("cropTop", "Top", pct(0.0)),
            pp("cropBottom", "Bottom", pct(0.0)),
        ],
        render: key_light,
        gpu: false,
        float: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, MaskShape, run_fx};

    /// Green screen with a red square (fg) and a half-transparent mix strip.
    fn plate() -> Image {
        crate::util::gen_image(40, 30, |x, y| {
            if (10..20).contains(&x) && (10..20).contains(&y) {
                [0.8, 0.2, 0.1, 1.0]
            } else if (25..30).contains(&x) {
                // 50 % red over green.
                [0.4, 0.6, 0.05, 1.0]
            } else {
                [0.1, 0.9, 0.1, 1.0]
            }
        })
    }

    fn green() -> Value {
        col(0.1, 0.9, 0.1)
    }

    #[test]
    fn keys_the_screen_and_keeps_the_foreground() {
        let out = run_fx("ec.keying.keylight", &[("screenColour", green())], plate(), 0.0, EffectEnv::default());
        let bg = out.img.get(2, 2);
        let fg = out.img.get(15, 15);
        let mix = out.img.get(27, 5);
        assert!(bg[3] < 0.02, "{bg:?}");
        assert!(fg[3] > 0.98 && (fg[0] - 0.8).abs() < 0.05, "{fg:?}");
        assert!(mix[3] > 0.3 && mix[3] < 0.7, "{mix:?}");
        // The recovered edge colour has the green removed (no spill).
        let (c, _) = unpremul(mix);
        assert!(c[1] <= c[0] + 0.05, "{c:?}");
        // Deterministic.
        let again = run_fx("ec.keying.keylight", &[("screenColour", green())], plate(), 0.0, EffectEnv::default());
        assert_eq!(out.img.data, again.img.data);
    }

    #[test]
    fn clip_gain_views_and_masks() {
        // Clip White 60 %: the half-transparent strip becomes solid.
        let out = run_fx("ec.keying.keylight", &[("screenColour", green()), ("clipWhite", num(40.0))], plate(), 0.0, EffectEnv::default());
        assert!(out.img.get(27, 5)[3] > 0.99);
        // Screen Matte and Status views are grey-scale and opaque.
        let sm = run_fx("ec.keying.keylight", &[("screenColour", green()), ("view", Value::Enum(4))], plate(), 0.0, EffectEnv::default());
        assert!(sm.img.get(2, 2)[0] < 0.02 && sm.img.get(15, 15)[0] > 0.98 && sm.img.get(2, 2)[3] == 1.0);
        let st = run_fx("ec.keying.keylight", &[("screenColour", green()), ("view", Value::Enum(8))], plate(), 0.0, EffectEnv::default());
        assert_eq!(st.img.get(27, 5)[0], 0.5);
        // Source view is the identity.
        let src = run_fx("ec.keying.keylight", &[("view", Value::Enum(0))], plate(), 0.0, EffectEnv::default());
        assert_eq!(src.img.data, plate().data);
        // Inside mask forces opacity; outside mask forces transparency.
        let sq = |x0: f64, y0: f64, x1: f64, y1: f64| MaskShape {
            name: "m".into(),
            points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
            closed: true,
            inverted: false,
        };
        let masks = [sq(0.0, 0.0, 6.0, 6.0), sq(12.0, 12.0, 18.0, 18.0)];
        let env = EffectEnv { masks: &masks, ..Default::default() };
        let m = run_fx("ec.keying.keylight", &[("screenColour", green()), ("insideMask", Value::Enum(1)), ("outsideMask", Value::Enum(2))], plate(), 0.0, env);
        assert!(m.img.get(2, 2)[3] > 0.99, "{:?}", m.img.get(2, 2));
        assert!(m.img.get(15, 15)[3] < 0.01);
        // Screen gain above 100 % keys more of the strip.
        let g = run_fx("ec.keying.keylight", &[("screenColour", green()), ("screenGain", num(150.0))], plate(), 0.0, EffectEnv::default());
        assert!(g.img.get(27, 5)[3] < out.img.get(27, 5)[3]);
        assert_eq!(crate::lookup("Keylight (1.2)").map(|s| s.id), Some("ec.keying.keylight"));
    }
}
