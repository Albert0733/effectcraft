//! Keying effects, batch 2: Difference Matte (against another layer), Inner/Outer Key (mattes
//! from masks with colour-based edge estimation and decontamination), Unmult and CC Simple
//! Wire Removal.

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::gaussian_blur;
use rayon::prelude::*;

use crate::util::{Plane, dist_to_poly, gauss_plane, layer_or_self, lerp4, morph_frac, point_in_poly, premul, smoothstep, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Keying", params, render, gpu: false, float: true }
}

fn pct() -> ParamUi {
    slider(0.0, 100.0, 0.0, 100.0, 1)
}

// ---------------------------------------------------------------- Difference Matte

fn difference_matte(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let view = ctx.params.e("view");
    if view == 1 {
        return b;
    }
    let tol = (ctx.params.f("matchingTolerance") / 100.0) as f32;
    let soft = (ctx.params.f("matchingSoftness") / 100.0) as f32;
    let blur = ctx.params.f("blurBeforeDifference") * b.scale;
    let other = layer_or_self(ctx, &b, "differenceLayer", true, ctx.params.e("ifLayerSizesDiffer") == 1);
    let (src, other) = if blur > 0.05 {
        (gaussian_blur(&b.img, blur * 0.5, blur * 0.5, true), gaussian_blur(&other, blur * 0.5, blur * 0.5, true))
    } else {
        (b.img.clone(), other)
    };
    let matte_only = view == 2;
    b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let (c, a) = unpremul(src.data[i]);
        let (oc, oa) = unpremul(other.data[i]);
        let d = (c[0] - oc[0]).abs().max((c[1] - oc[1]).abs()).max((c[2] - oc[2]).abs()).max((a - oa).abs());
        let m = if d <= tol {
            0.0
        } else if soft <= 1e-6 {
            1.0
        } else {
            ((d - tol) / soft).clamp(0.0, 1.0)
        };
        if matte_only {
            *px = [m * px[3], m * px[3], m * px[3], px[3]];
        } else {
            *px = px.map(|v| v * m);
        }
    });
    b
}

// ---------------------------------------------------------------- Inner/Outer Key

fn mask_options() -> ParamUi {
    popup(&["None", "Mask 1", "Mask 2", "Mask 3", "Mask 4", "Mask 5", "Mask 6", "Mask 7", "Mask 8", "Mask 9", "Mask 10"])
}

/// Coverage plane (1 inside) of the selected closed masks, in buffer pixels.
fn mask_plane(ctx: &EffectCtx, b: &Buf, idx: &[u32]) -> Option<Plane> {
    let shapes: Vec<&crate::MaskShape> = idx.iter().filter(|&&i| i > 0).filter_map(|&i| ctx.env.masks.get(i as usize - 1)).collect();
    if shapes.is_empty() {
        return None;
    }
    let (w, h) = (b.img.width as usize, b.img.height as usize);
    let mut pl = Plane::new(w, h);
    let inv = 1.0 / b.scale.max(1e-9);
    pl.data.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
        for (x, v) in row.iter_mut().enumerate() {
            let lx = (x as f64 + 0.5 - b.offset[0]) * inv;
            let ly = (y as f64 + 0.5 - b.offset[1]) * inv;
            *v = if shapes.iter().any(|s| point_in_poly(&s.points, lx, ly)) { 1.0 } else { 0.0 };
        }
    });
    Some(pl)
}

/// Signed distance (pixels, positive inside) to the selected mask outlines.
fn mask_sdf(ctx: &EffectCtx, b: &Buf, idx: &[u32]) -> Option<Plane> {
    let shapes: Vec<&crate::MaskShape> = idx.iter().filter(|&&i| i > 0).filter_map(|&i| ctx.env.masks.get(i as usize - 1)).collect();
    if shapes.is_empty() {
        return None;
    }
    let (w, h) = (b.img.width as usize, b.img.height as usize);
    let mut pl = Plane::new(w, h);
    let inv = 1.0 / b.scale.max(1e-9);
    pl.data.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
        for (x, v) in row.iter_mut().enumerate() {
            let lx = (x as f64 + 0.5 - b.offset[0]) * inv;
            let ly = (y as f64 + 0.5 - b.offset[1]) * inv;
            let inside = shapes.iter().any(|s| point_in_poly(&s.points, lx, ly));
            let d = shapes.iter().map(|s| dist_to_poly(&s.points, true, lx, ly)).fold(f64::INFINITY, f64::min) * b.scale;
            *v = if inside { d as f32 } else { -d as f32 };
        }
    });
    Some(pl)
}

fn inner_outer_key(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let fg_idx = [ctx.params.e("foreground"), ctx.params.e("additionalForeground")];
    let bg_idx = [ctx.params.e("background"), ctx.params.e("additionalBackground")];
    let radius = (ctx.params.f("singleMaskHighlightRadius") * b.scale) as f32;
    let thin = ctx.params.f("edgeThin") * b.scale;
    let feather = ctx.params.f("edgeFeather") * b.scale;
    let thr = (ctx.params.f("edgeThreshold") / 100.0) as f32;
    let invert = ctx.params.b("invertExtraction");
    let orig = (ctx.params.f("blendWithOriginal") / 100.0) as f32;
    let (w, h) = (b.img.width as usize, b.img.height as usize);
    // Trimap: 1 = known foreground, 0 = known background, 0.5 = unknown.
    let fg = mask_plane(ctx, &b, &fg_idx);
    let bg = mask_plane(ctx, &b, &bg_idx);
    let tri: Plane = match (fg, bg) {
        (None, _) => return b,
        (Some(f), Some(bk)) => f.zip_map(&bk, |f, bk| {
            if f > 0.5 {
                1.0
            } else if bk > 0.5 {
                0.5
            } else {
                0.0
            }
        }),
        (Some(f), None) => {
            if radius <= 0.0 {
                f
            } else {
                let sdf = mask_sdf(ctx, &b, &fg_idx).unwrap_or(f);
                sdf.map(|d| {
                    if d > radius {
                        1.0
                    } else if d < -radius {
                        0.0
                    } else {
                        0.5
                    }
                })
            }
        }
    };
    let unknown = tri.data.par_iter().any(|&t| t > 0.25 && t < 0.75);
    let mut alpha = tri.clone();
    let mut colours: Option<Vec<[f32; 3]>> = None;
    if unknown {
        // Local foreground / background colour estimates: normalised blurs over known regions.
        let sigma = (radius.max(4.0) as f64).max(w.min(h) as f64 * 0.02);
        let known_f = tri.map(|t| if t > 0.75 { 1.0 } else { 0.0 });
        let known_b = tri.map(|t| if t < 0.25 { 1.0 } else { 0.0 });
        let straight: Vec<[f32; 3]> = b.img.data.par_iter().map(|&px| unpremul(px).0).collect();
        let est = |known: &Plane| -> Vec<[f32; 3]> {
            let wsum = gauss_plane(known, sigma, sigma);
            let chans: Vec<Plane> = (0..3)
                .map(|k| {
                    let pl = Plane { w, h, data: straight.par_iter().zip(known.data.par_iter()).map(|(c, &m)| c[k] * m).collect() };
                    gauss_plane(&pl, sigma, sigma)
                })
                .collect();
            (0..w * h).into_par_iter().map(|i| [0, 1, 2].map(|k| chans[k].data[i] / wsum.data[i].max(1e-6))).collect()
        };
        let fc = est(&known_f);
        let bc = est(&known_b);
        alpha.data.par_iter_mut().enumerate().for_each(|(i, a)| {
            if *a > 0.25 && *a < 0.75 {
                let c = straight[i];
                let d = [fc[i][0] - bc[i][0], fc[i][1] - bc[i][1], fc[i][2] - bc[i][2]];
                let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                *a = if dd < 1e-6 { 0.5 } else { (((c[0] - bc[i][0]) * d[0] + (c[1] - bc[i][1]) * d[1] + (c[2] - bc[i][2]) * d[2]) / dd).clamp(0.0, 1.0) };
            }
        });
        // Decontaminate edge colours: remove the background contribution.
        colours = Some(
            (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let a = alpha.data[i];
                    let c = straight[i];
                    if tri.data[i] > 0.25 && tri.data[i] < 0.75 && a > 0.02 {
                        [0, 1, 2].map(|k| ((c[k] - (1.0 - a) * bc[i][k]) / a).clamp(0.0, 1.0))
                    } else {
                        c
                    }
                })
                .collect(),
        );
    }
    if thin.abs() > 0.01 {
        alpha = morph_frac(&alpha, thin.abs(), thin < 0.0);
    }
    if feather > 0.05 {
        alpha = gauss_plane(&alpha, feather * 0.5, feather * 0.5);
    }
    let src = b.img.clone();
    b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let mut m = alpha.data[i];
        if thr > 0.0 {
            m = if m < thr { 0.0 } else { m };
        }
        if invert {
            m = 1.0 - m;
        }
        let (c0, a) = unpremul(src.data[i]);
        let c = colours.as_ref().map(|v| v[i]).unwrap_or(c0);
        let keyed = premul(c, a * m);
        *px = lerp4(keyed, src.data[i], orig);
    });
    b
}

// ---------------------------------------------------------------- Unmult

/// Turns black into transparency: alpha becomes the brightest straight channel and colour is
/// divided by it (premultiplied colour is unchanged).
fn unmult(_: &EffectCtx, mut b: Buf) -> Buf {
    b.img.data.par_iter_mut().for_each(|px| {
        let (c, a) = unpremul(*px);
        let m = c[0].max(c[1]).max(c[2]).clamp(0.0, 1.0);
        if m <= 1e-6 {
            *px = [0.0; 4];
            return;
        }
        *px = premul([c[0] / m, c[1] / m, c[2] / m], a * m);
    });
    b
}

// ---------------------------------------------------------------- CC Simple Wire Removal

fn wire_removal(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pa = ctx.params.v2("pointA");
    let pb = ctx.params.v2("pointB");
    let style = ctx.params.e("removalStyle");
    let half = (ctx.params.f("thickness") * b.scale * 0.5).max(0.0);
    if half <= 0.0 {
        return b;
    }
    let slope = (ctx.params.f("slope") / 100.0).clamp(0.0, 1.0) as f32;
    let mirror = (ctx.params.f("mirrorBlend") / 100.0).clamp(0.0, 1.0) as f32;
    let (ax, ay) = b.to_px(pa);
    let (bx, by) = b.to_px(pb);
    let (dx, dy) = (bx - ax, by - ay);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-6 {
        return b;
    }
    let (tx, ty) = (dx / len, dy / len);
    let (nx, ny) = (-ty, tx);
    let src = b.img.clone();
    let soft_w = half * slope as f64;
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (qx, qy) = (x as f64 + 0.5 - ax, y as f64 + 0.5 - ay);
            let d = qx * nx + qy * ny;
            let ad = d.abs();
            if ad > half + soft_w {
                continue;
            }
            // Positions just outside the wire on both sides.
            let base = (x as f64 + 0.5 - d * nx, y as f64 + 0.5 - d * ny);
            let edge = half + 1.0;
            let pos = |s: f64| src.sample_bilinear_clamped(base.0 + nx * s, base.1 + ny * s);
            let repl = match style {
                0 => {
                    let t = ((d + edge) / (2.0 * edge)) as f32;
                    lerp4(pos(-edge), pos(edge), t.clamp(0.0, 1.0))
                }
                // Displace: shift the image across the wire from the nearer side.
                2 | 1 => {
                    let s = if d >= 0.0 { d + 2.0 * (half - ad) + 1.0 } else { d - 2.0 * (half - ad) - 1.0 };
                    let sh = pos(s);
                    if mirror > 0.0 { lerp4(sh, pos(-s), mirror * 0.5) } else { sh }
                }
                _ => {
                    // Displace Horizontal: sample left/right of the wire along x.
                    let sx = if nx.abs() > 1e-6 { (half - ad + 1.0) / nx.abs() } else { 0.0 };
                    let dir = if d * nx >= 0.0 { 1.0 } else { -1.0 };
                    let sh = src.sample_bilinear_clamped(x as f64 + 0.5 + dir * sx, y as f64 + 0.5);
                    let mi = src.sample_bilinear_clamped(x as f64 + 0.5 - dir * sx, y as f64 + 0.5);
                    if mirror > 0.0 { lerp4(sh, mi, mirror * 0.5) } else { sh }
                }
            };
            let k = if ad <= half { 1.0 } else { 1.0 - smoothstep(half as f32, (half + soft_w) as f32, ad as f32) };
            *px = lerp4(*px, repl, k);
        }
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    vec![
        spec(
            "ec.key.differencematte",
            "Difference Matte",
            vec![
                p("view", "View", Value::Enum(0), popup(&["Final Output", "Source Only", "Matte Only"])),
                p("differenceLayer", "Difference Layer", Value::Layer(None), ParamUi::Layer),
                p("ifLayerSizesDiffer", "If Layer Sizes Differ", Value::Enum(0), popup(&["Center", "Stretch to Fit"])),
                p("matchingTolerance", "Matching Tolerance", num(15.0), pct()),
                p("matchingSoftness", "Matching Softness", num(0.0), pct()),
                p("blurBeforeDifference", "Blur Before Difference", num(0.0), slider(0.0, 1000.0, 0.0, 20.0, 1)),
            ],
            difference_matte,
        ),
        spec(
            "ec.key.innerouter",
            "Inner/Outer Key",
            vec![
                p("foreground", "Foreground (Inside)", Value::Enum(0), mask_options()),
                p("additionalForeground", "Additional Foreground", Value::Enum(0), mask_options()),
                p("background", "Background (Outside)", Value::Enum(0), mask_options()),
                p("additionalBackground", "Additional Background", Value::Enum(0), mask_options()),
                p("singleMaskHighlightRadius", "Single Mask Highlight Radius", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("edgeThin", "Edge Thin", num(0.0), slider(-10.0, 10.0, -10.0, 10.0, 1)),
                p("edgeFeather", "Edge Feather", num(0.0), slider(0.0, 100.0, 0.0, 50.0, 1)),
                p("edgeThreshold", "Edge Threshold", num(0.0), pct()),
                p("invertExtraction", "Invert Extraction", Value::Bool(false), ParamUi::Checkbox),
                p("blendWithOriginal", "Blend with Original", num(0.0), pct()),
            ],
            inner_outer_key,
        ),
        spec("ec.key.unmult", "Unmult", vec![], unmult),
        spec(
            "ec.key.ccsimplewireremoval",
            "CC Simple Wire Removal",
            vec![
                p("pointA", "Point A", Value::Vec2([0.25, 0.25]), ParamUi::Point),
                p("pointB", "Point B", Value::Vec2([0.75, 0.75]), ParamUi::Point),
                p("removalStyle", "Removal Style", Value::Enum(2), popup(&["Fade", "Frame Offset", "Displace", "Displace Horizontal"])),
                p("thickness", "Thickness", num(5.0), slider(0.0, 100.0, 0.0, 30.0, 1)),
                p("slope", "Slope", num(0.0), pct()),
                p("mirrorBlend", "Mirror Blend", num(0.0), pct()),
                p("frameOffset", "Frame Offset", num(0.0), slider(-100.0, 100.0, -10.0, 10.0, 0)),
            ],
            wire_removal,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel2::tests::{FakeHost, sample};
    use crate::{EffectEnv, MaskShape, run_fx};
    use effectcraft_raster::Image;

    #[test]
    fn difference_matte_self_is_transparent_and_other_keeps() {
        let img = sample();
        let out = run_fx("ec.key.differencematte", &[], img.clone(), 0.0, EffectEnv::default());
        assert!(out.img.data.iter().all(|p| p[3] == 0.0));
        let host = FakeHost(Image::filled(8, 6, [0.0, 0.0, 0.4, 1.0]));
        let env = EffectEnv { host: Some(&host), ..Default::default() };
        let out = run_fx("ec.key.differencematte", &[("differenceLayer", Value::Layer(Some(7)))], img.clone(), 0.0, env);
        assert_eq!(out.img.get(5, 5), img.get(5, 5));
        assert_eq!(out.img.get(0, 0)[3], 0.0, "matching pixel keyed out");
        let again = run_fx("ec.key.differencematte", &[("differenceLayer", Value::Layer(Some(7)))], img.clone(), 0.0, env);
        assert_eq!(out.img.data, again.img.data);
    }

    #[test]
    fn inner_outer_key_inside_mask() {
        let img = Image::filled(20, 20, [0.8, 0.2, 0.2, 1.0]);
        let masks = vec![MaskShape { name: "Mask 1".into(), points: vec![[5.0, 5.0], [15.0, 5.0], [15.0, 15.0], [5.0, 15.0]], closed: true, inverted: false }];
        let env = EffectEnv { masks: &masks, ..Default::default() };
        let out = run_fx("ec.key.innerouter", &[("foreground", Value::Enum(1))], img.clone(), 0.0, env);
        assert!((out.img.get(10, 10)[3] - 1.0).abs() < 1e-6);
        assert_eq!(out.img.get(1, 1)[3], 0.0);
        // Two masks: unknown band estimated from colour.
        let mut img2 = Image::filled(20, 20, [0.0, 0.0, 1.0, 1.0]);
        for y in 0..20 {
            for x in 7..13 {
                img2.set(x, y, [1.0, 0.0, 0.0, 1.0]);
            }
        }
        let masks2 = vec![
            MaskShape { name: "in".into(), points: vec![[8.0, 0.0], [12.0, 0.0], [12.0, 20.0], [8.0, 20.0]], closed: true, inverted: false },
            MaskShape { name: "out".into(), points: vec![[4.0, 0.0], [16.0, 0.0], [16.0, 20.0], [4.0, 20.0]], closed: true, inverted: false },
        ];
        let env2 = EffectEnv { masks: &masks2, ..Default::default() };
        let o2 = run_fx("ec.key.innerouter", &[("foreground", Value::Enum(1)), ("background", Value::Enum(2))], img2.clone(), 0.0, env2);
        assert!(o2.img.get(7, 10)[3] > 0.8, "{:?}", o2.img.get(7, 10));
        assert!(o2.img.get(5, 10)[3] < 0.2, "{:?}", o2.img.get(5, 10));
        assert_eq!(o2.img.get(1, 10)[3], 0.0);
        let o3 = run_fx("ec.key.innerouter", &[("foreground", Value::Enum(1)), ("background", Value::Enum(2))], img2, 0.0, env2);
        assert_eq!(o2.img.data, o3.img.data);
    }

    #[test]
    fn unmult_black_becomes_transparent() {
        let mut img = Image::filled(4, 4, [0.0, 0.0, 0.0, 1.0]);
        img.set(1, 1, [0.5, 0.25, 0.0, 1.0]);
        let out = run_fx("ec.key.unmult", &[], img, 0.0, EffectEnv::default());
        assert_eq!(out.img.get(0, 0), [0.0; 4]);
        let p = out.img.get(1, 1);
        assert!((p[3] - 0.5).abs() < 1e-6 && (p[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn wire_removal_hides_a_wire() {
        let mut img = Image::filled(20, 20, [0.5, 0.5, 0.5, 1.0]);
        for y in 0..20 {
            img.set(10, y, [0.0, 0.0, 0.0, 1.0]);
        }
        for style in 0..4u32 {
            let out = run_fx(
                "ec.key.ccsimplewireremoval",
                &[("pointA", Value::Vec2([10.5, 0.0])), ("pointB", Value::Vec2([10.5, 20.0])), ("removalStyle", Value::Enum(style)), ("thickness", num(3.0))],
                img.clone(),
                0.0,
                EffectEnv::default(),
            );
            assert!((out.img.get(10, 10)[0] - 0.5).abs() < 0.05, "style {style}: {:?}", out.img.get(10, 10));
            assert_eq!(out.img.get(2, 10), img.get(2, 10));
        }
    }
}
