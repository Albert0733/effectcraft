//! Matte effects: chokers (grey-scale morphology and blur/threshold stages) and edge-aware matte
//! refinement (guided filter, He et al. 2010).

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::gaussian_blur;
use rayon::prelude::*;

use crate::util::{Plane, gauss_plane, guided_filter, morph_frac, morph_plane, premul, set_alpha, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Matte", params, render, gpu: false, float: true }
}

fn matte_view(b: &mut Buf) {
    b.img.data.par_iter_mut().for_each(|px| {
        let a = px[3].clamp(0.0, 1.0);
        *px = [a, a, a, 1.0];
    });
}

fn simple_choker(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = ctx.params.f("chokeMatte") * b.scale;
    if c.abs() > 1e-6 {
        if c < 0.0 && !ctx.adjustment {
            b.pad((-c).ceil() as u32 + 1);
        }
        let a = Plane::alpha(&b.img);
        let na = if c > 0.0 { morph_frac(&a, c, false) } else { morph_frac(&a, -c, true) };
        set_alpha(&mut b.img, &na);
    }
    if ctx.params.e("view") == 1 {
        matte_view(&mut b);
    }
    b
}

/// One Matte Choker stage: blur by geometric softness, then a threshold ramp.
fn choke_stage(a: Plane, geo: f64, choke: f32, gray: f32) -> Plane {
    let a = if geo > 0.0 { gauss_plane(&a, geo * 0.5, geo * 0.5) } else { a };
    let t = 0.5 + choke / 255.0 * 0.5;
    let w = gray.max(1e-3);
    if (t - 0.5).abs() < 1e-6 && (w - 1.0).abs() < 1e-6 {
        return a;
    }
    a.map(|v| ((v - t) / w + 0.5).clamp(0.0, 1.0))
}

fn matte_choker(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let g1 = ctx.params.f("geometricSoftness1") * b.scale;
    let c1 = ctx.params.f("choke1") as f32;
    let s1 = ctx.params.f("grayLevelSoftness1") as f32 / 100.0;
    let g2 = ctx.params.f("geometricSoftness2") * b.scale;
    let c2 = ctx.params.f("choke2") as f32;
    let s2 = ctx.params.f("grayLevelSoftness2") as f32 / 100.0;
    let it = ctx.params.f("iterations").round().clamp(1.0, 100.0) as usize;
    if (c1 < 0.0 || c2 < 0.0 || g1 > 0.0 || g2 > 0.0) && !ctx.adjustment {
        b.pad(((g1 + g2) * 1.5 * it as f64).ceil() as u32 + 1);
    }
    let mut a = Plane::alpha(&b.img);
    for _ in 0..it {
        a = choke_stage(a, g1, c1, s1);
        a = choke_stage(a, g2, c2, s2);
    }
    set_alpha(&mut b.img, &a);
    b
}

fn refine(ctx: &EffectCtx, mut b: Buf, hard: bool) -> Buf {
    let r = (ctx.params.f("radius") * b.scale).round().max(1.0) as usize;
    let smooth = ctx.params.f("smooth") as f32 / 100.0;
    let feather = ctx.params.f("feather") / 100.0 * r as f64 * 0.5;
    let choke = ctx.params.f("choke") as f32 / 100.0 * 0.5;
    let decon = ctx.params.b("decontaminate");
    let decon_amt = ctx.params.f("decontaminationAmount") as f32 / 100.0;
    let invert = ctx.params.b("invert");
    let contrast = if hard { ctx.params.f("alphaContrast") as f32 / 100.0 } else { 1.0 };

    let a = Plane::alpha(&b.img);
    let guide = Plane::from_image(&b.img, |px| effectcraft_color::luminance(px[0], px[1], px[2]));
    let eps = 1e-4 + smooth * smooth * 0.05;
    let filtered = guided_filter(&guide, &a, r, eps);
    // Only the edge band (where the matte varies within the radius) is refined.
    let hi = morph_plane(&a, r, r, true);
    let lo = morph_plane(&a, r, r, false);
    let mut na = Plane::new(a.w, a.h);
    na.data.par_iter_mut().enumerate().for_each(|(i, v)| {
        *v = if hi.data[i] - lo.data[i] > 1e-3 { filtered.data[i].clamp(0.0, 1.0) } else { a.data[i] };
    });
    if feather > 0.0 {
        na = gauss_plane(&na, feather, feather);
    }
    na = na.map(|v| {
        let mut v = if choke > 0.0 {
            (v - choke) / (1.0 - choke)
        } else if choke < 0.0 {
            v / (1.0 + choke)
        } else {
            v
        };
        if (contrast - 1.0).abs() > 1e-6 {
            v = (v - 0.5) * contrast + 0.5;
        }
        let v = v.clamp(0.0, 1.0);
        if invert { 1.0 - v } else { v }
    });
    if decon && decon_amt > 0.0 {
        let est = gaussian_blur(&b.img, r as f64 * 0.5, r as f64 * 0.5, true);
        b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
            let (c, a0) = unpremul(*px);
            if a0 > 1e-4 && a0 < 0.999 {
                let (ec, ea) = unpremul(est.data[i]);
                if ea > 1e-4 {
                    let t = decon_amt * (1.0 - a0);
                    *px = premul([0, 1, 2].map(|k| c[k] + (ec[k] - c[k]) * t), a0);
                }
            }
        });
    }
    set_alpha(&mut b.img, &na);
    b
}

fn refine_soft(ctx: &EffectCtx, b: Buf) -> Buf {
    refine(ctx, b, false)
}

fn refine_hard(ctx: &EffectCtx, b: Buf) -> Buf {
    refine(ctx, b, true)
}

pub fn specs() -> Vec<EffectSpec> {
    let refine_params = |hard: bool| {
        let mut v = vec![
            p("radius", "Additional Edge Radius", num(if hard { 3.0 } else { 10.0 }), slider(1.0, 200.0, 1.0, 50.0, 1)),
            p("smooth", "Smooth", num(if hard { 20.0 } else { 0.0 }), slider(0.0, 100.0, 0.0, 100.0, 0)),
            p("feather", "Feather", num(if hard { 0.0 } else { 10.0 }), slider(0.0, 100.0, 0.0, 100.0, 0)),
            p("choke", "Choke", num(0.0), slider(-100.0, 100.0, -100.0, 100.0, 0)),
        ];
        if hard {
            v.push(p("alphaContrast", "Alpha Contrast", num(300.0), slider(100.0, 1000.0, 100.0, 500.0, 0)));
        }
        v.push(p("decontaminate", "Decontaminate Edge Colors", Value::Bool(true), ParamUi::Checkbox));
        v.push(p("decontaminationAmount", "Decontamination Amount", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 0)));
        v.push(p("invert", "Invert", Value::Bool(false), ParamUi::Checkbox));
        v
    };
    let soft = || slider(0.0, 1000.0, 0.0, 100.0, 1);
    let ch = || slider(-127.0, 127.0, -127.0, 127.0, 0);
    let gl = || slider(0.0, 100.0, 0.0, 100.0, 1);
    vec![
        spec(
            "ec.matte.simplechoker",
            "Simple Choker",
            vec![
                p("view", "View", Value::Enum(0), popup(&["Final Output", "Matte"])),
                p("chokeMatte", "Choke Matte", num(0.0), slider(-100.0, 100.0, -10.0, 10.0, 2)),
            ],
            simple_choker,
        ),
        spec(
            "ec.matte.mattechoker",
            "Matte Choker",
            vec![
                p("geometricSoftness1", "Geometric Softness 1", num(4.0), soft()),
                p("choke1", "Choke 1", num(75.0), ch()),
                p("grayLevelSoftness1", "Gray Level Softness 1", num(10.0), gl()),
                p("geometricSoftness2", "Geometric Softness 2", num(0.0), soft()),
                p("choke2", "Choke 2", num(0.0), ch()),
                p("grayLevelSoftness2", "Gray Level Softness 2", num(100.0), gl()),
                p("iterations", "Iterations", num(1.0), slider(1.0, 100.0, 1.0, 10.0, 0)),
            ],
            matte_choker,
        ),
        spec("ec.matte.refinesoft", "Refine Soft Matte", refine_params(false), refine_soft),
        spec("ec.matte.refinehard", "Refine Hard Matte", refine_params(true), refine_hard),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Params, apply, find};
    use effectcraft_raster::Image;

    fn square() -> Image {
        let mut img = Image::new(24, 24);
        for y in 6..18 {
            for x in 6..18 {
                img.set(x, y, [0.8, 0.4, 0.2, 1.0]);
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
            EffectCtx { params: &params, time: 0.0, layer_size: [img.width as f64, img.height as f64], seed: 1, adjustment: true, env: Default::default() };
        apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 }).img
    }

    fn alpha_sum(img: &Image) -> f32 {
        img.data.iter().map(|p| p[3]).sum()
    }

    #[test]
    fn simple_choker_shrinks_and_grows() {
        let img = square();
        let base = alpha_sum(&img);
        let shrunk = run("ec.matte.simplechoker", &[("chokeMatte", num(2.0))], img.clone());
        assert!((alpha_sum(&shrunk) - 64.0).abs() < 1e-3, "{}", alpha_sum(&shrunk));
        let grown = run("ec.matte.simplechoker", &[("chokeMatte", num(-2.0))], img);
        assert!(alpha_sum(&grown) > base);
        // Grown pixels pick up the layer colour, not black.
        let (c, a) = unpremul(grown.get(5, 10));
        assert!(a > 0.99 && c[0] > 0.5, "{c:?}");
    }

    #[test]
    fn matte_choker_defaults_shrink() {
        let img = square();
        let out = run("ec.matte.mattechoker", &[], img.clone());
        assert!(alpha_sum(&out) < alpha_sum(&img));
        assert!(out.get(12, 12)[3] > 0.99);
    }

    #[test]
    fn refine_soft_matte_keeps_interior_and_range() {
        let img = square();
        let out = run("ec.matte.refinesoft", &[], img);
        assert!(out.get(12, 12)[3] > 0.99);
        assert!(out.get(0, 0)[3] < 0.01);
        assert!(out.data.iter().all(|p| (0.0..=1.0).contains(&p[3])));
    }
}
