//! Color Correction effects.

use effectcraft_color::{hsl_to_rgb, luminance, rgb_to_hsl};
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;

use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Color Correction", params, render, gpu: false, float: true }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn tint(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let black = ctx.params.color("black");
    let white = ctx.params.color("white");
    let amt = ctx.params.f("amount") as f32 / 100.0;
    b.img.map_straight(|c| {
        let l = luminance(c[0], c[1], c[2]).clamp(0.0, 1.0);
        let t = [black[0] + (white[0] - black[0]) * l, black[1] + (white[1] - black[1]) * l, black[2] + (white[2] - black[2]) * l];
        mix(c, t, amt)
    });
    b
}

fn tritone(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let hi = ctx.params.color("highlights");
    let mid = ctx.params.color("midtones");
    let sh = ctx.params.color("shadows");
    let blend = 1.0 - ctx.params.f("blend") as f32 / 100.0;
    b.img.map_straight(|c| {
        let l = luminance(c[0], c[1], c[2]).clamp(0.0, 1.0);
        let t = if l < 0.5 { mix([sh[0], sh[1], sh[2]], [mid[0], mid[1], mid[2]], l * 2.0) } else { mix([mid[0], mid[1], mid[2]], [hi[0], hi[1], hi[2]], (l - 0.5) * 2.0) };
        mix(c, t, blend)
    });
    b
}

fn brightness_contrast(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let br = ctx.params.f("brightness") as f32 / 100.0;
    let ct = ctx.params.f("contrast") as f32 / 100.0;
    let k = if ct >= 0.0 { 1.0 / (1.0 - ct * 0.99) } else { 1.0 + ct };
    b.img.map_straight(|c| c.map(|v| ((v + br - 0.5) * k + 0.5).max(0.0)));
    b
}

fn hue_saturation(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let hue = ctx.params.f("hue") as f32 / 360.0;
    let sat = ctx.params.f("saturation") as f32 / 100.0;
    let light = ctx.params.f("lightness") as f32 / 100.0;
    let colorize = ctx.params.b("colorize");
    let ch = ctx.params.f("colorizeHue") as f32 / 360.0;
    let cs = ctx.params.f("colorizeSaturation") as f32 / 100.0;
    let cl = ctx.params.f("colorizeLightness") as f32 / 100.0;
    b.img.map_straight(|c| {
        let (h, s, l) = rgb_to_hsl(c[0], c[1], c[2]);
        let (h, s, l) = if colorize {
            (ch, cs, (l + cl * if cl > 0.0 { 1.0 - l } else { l }).clamp(0.0, 1.0))
        } else {
            let s2 = if sat >= 0.0 { s + (1.0 - s) * sat * s.min(1.0) } else { s * (1.0 + sat) };
            let l2 = if light >= 0.0 { l + (1.0 - l) * light } else { l * (1.0 + light) };
            ((h + hue).rem_euclid(1.0), s2.clamp(0.0, 1.0), l2)
        };
        let (r, g, bl) = hsl_to_rgb(h, s, l);
        [r, g, bl]
    });
    b
}

fn levels(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let ib = ctx.params.f("inBlack") as f32;
    let iw = ctx.params.f("inWhite") as f32;
    let g = ctx.params.f("gamma").max(0.01) as f32;
    let ob = ctx.params.f("outBlack") as f32;
    let ow = ctx.params.f("outWhite") as f32;
    let clip = !ctx.params.b("noClip");
    b.img.map_straight(|c| {
        c.map(|v| {
            let mut t = (v - ib) / (iw - ib).max(1e-6);
            if clip {
                t = t.clamp(0.0, 1.0);
            }
            let t = t.max(0.0).powf(1.0 / g);
            ob + (ow - ob) * t
        })
    });
    b
}

fn exposure(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let e = 2f32.powf(ctx.params.f("exposure") as f32);
    let off = ctx.params.f("offset") as f32;
    let g = ctx.params.f("gamma").max(0.01) as f32;
    b.img.map_straight(|c| {
        c.map(|v| {
            let lin = effectcraft_color::srgb_to_linear(v.max(0.0));
            let o = (lin * e + off).max(0.0).powf(1.0 / g);
            effectcraft_color::linear_to_srgb(o)
        })
    });
    b
}

fn black_white(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let w = [ctx.params.f("reds"), ctx.params.f("yellows"), ctx.params.f("greens"), ctx.params.f("cyans"), ctx.params.f("blues"), ctx.params.f("magentas")].map(|v| v as f32 / 100.0);
    let tint_on = ctx.params.b("tint");
    let tc = ctx.params.color("tintColor");
    b.img.map_straight(|c| {
        let (h, s, _) = rgb_to_hsl(c[0], c[1], c[2]);
        // Interpolate the six hue weights around the colour wheel.
        let hp = h * 6.0;
        let i = hp.floor() as usize % 6;
        let f = hp - hp.floor();
        let wt = w[i] + (w[(i + 1) % 6] - w[i]) * f;
        let base = luminance(c[0], c[1], c[2]);
        let gray = (base + (wt - 0.5) * s * 0.6).max(0.0);
        if tint_on {
            let (th, ts, _) = rgb_to_hsl(tc[0], tc[1], tc[2]);
            let (r, g, bl) = hsl_to_rgb(th, ts, gray.min(1.0));
            [r, g, bl]
        } else {
            [gray, gray, gray]
        }
    });
    b
}

fn fill(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = ctx.params.color("color");
    let invert = ctx.params.b("invert");
    let opacity = ctx.params.f("opacity") as f32 / 100.0;
    b.img.data.iter_mut().for_each(|px| {
        let a = if invert { 1.0 - px[3] } else { px[3] };
        let f = [c[0] * a, c[1] * a, c[2] * a];
        for i in 0..3 {
            px[i] += (f[i] - px[i]) * opacity;
        }
        if invert {
            px[3] += (a - px[3]) * opacity;
        }
    });
    b
}

fn change_to_color(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let from = ctx.params.color("from");
    let to = ctx.params.color("to");
    let tol = ctx.params.f("tolerance") as f32 / 100.0;
    let soft = (ctx.params.f("softness") as f32 / 100.0).max(1e-3);
    let (fh, fs, fl) = rgb_to_hsl(from[0], from[1], from[2]);
    let (th, ts, tl) = rgb_to_hsl(to[0], to[1], to[2]);
    b.img.map_straight(|c| {
        let (h, s, l) = rgb_to_hsl(c[0], c[1], c[2]);
        let dh = ((h - fh + 0.5).rem_euclid(1.0) - 0.5).abs() * 2.0;
        let d = dh.max((s - fs).abs() * 0.5).max((l - fl).abs() * 0.5);
        let k = (1.0 - ((d - tol) / soft).clamp(0.0, 1.0)).clamp(0.0, 1.0);
        let (r, g, bl) = hsl_to_rgb((h + (th - fh)).rem_euclid(1.0), (s + (ts - fs)).clamp(0.0, 1.0), (l + (tl - fl)).clamp(0.0, 1.0));
        mix(c, [r, g, bl], k)
    });
    b
}

fn leave_color(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let keep = ctx.params.color("color");
    let amt = ctx.params.f("amount") as f32 / 100.0;
    let tol = ctx.params.f("tolerance") as f32 / 100.0;
    let (kh, _, _) = rgb_to_hsl(keep[0], keep[1], keep[2]);
    b.img.map_straight(|c| {
        let (h, s, _) = rgb_to_hsl(c[0], c[1], c[2]);
        let dh = ((h - kh + 0.5).rem_euclid(1.0) - 0.5).abs() * 2.0;
        let keepk = if s > 0.05 && dh <= tol { 1.0 } else { (1.0 - (dh - tol) * 8.0).clamp(0.0, 1.0) * s.min(1.0) };
        let g = luminance(c[0], c[1], c[2]);
        mix(c, [g, g, g], amt * (1.0 - keepk))
    });
    b
}

fn photo_filter(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = ctx.params.color("color");
    let d = ctx.params.f("density") as f32 / 100.0;
    let keep = ctx.params.b("preserveLuminosity");
    b.img.map_straight(|x| {
        let f = [x[0] * c[0], x[1] * c[1], x[2] * c[2]];
        let mut o = mix(x, f, d);
        if keep {
            let l0 = luminance(x[0], x[1], x[2]);
            let l1 = luminance(o[0], o[1], o[2]).max(1e-6);
            o = o.map(|v| v * l0 / l1);
        }
        o
    });
    b
}

fn vibrance(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let vib = ctx.params.f("vibrance") as f32 / 100.0;
    let sat = ctx.params.f("saturation") as f32 / 100.0;
    b.img.map_straight(|c| {
        let (h, s, l) = rgb_to_hsl(c[0], c[1], c[2]);
        let s = (s * (1.0 + sat) + vib * (1.0 - s) * s.min(0.5)).clamp(0.0, 1.0);
        let (r, g, bl) = hsl_to_rgb(h, s, l);
        [r, g, bl]
    });
    b
}

fn color_balance(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let g = |k: &str| ctx.params.f(k) as f32 / 100.0;
    let sh = [g("shadowRed"), g("shadowGreen"), g("shadowBlue")];
    let md = [g("midRed"), g("midGreen"), g("midBlue")];
    let hl = [g("hiRed"), g("hiGreen"), g("hiBlue")];
    b.img.map_straight(|c| {
        let l = luminance(c[0], c[1], c[2]).clamp(0.0, 1.0);
        let ws = (1.0 - l * 2.0).clamp(0.0, 1.0);
        let wh = (l * 2.0 - 1.0).clamp(0.0, 1.0);
        let wm = 1.0 - ws - wh;
        let mut o = c;
        for i in 0..3 {
            o[i] = (c[i] + (sh[i] * ws + md[i] * wm + hl[i] * wh) * 0.5).max(0.0);
        }
        o
    });
    b
}

fn channel_mixer(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let g = |k: &str| ctx.params.f(k) as f32 / 100.0;
    let m = [[g("rr"), g("rg"), g("rb"), g("rc")], [g("gr"), g("gg"), g("gb"), g("gc")], [g("br"), g("bg"), g("bb"), g("bc")]];
    let mono = ctx.params.b("monochrome");
    b.img.map_straight(|c| {
        let row = |r: [f32; 4]| (c[0] * r[0] + c[1] * r[1] + c[2] * r[2] + r[3]).max(0.0);
        if mono {
            let v = row(m[0]);
            [v, v, v]
        } else {
            [row(m[0]), row(m[1]), row(m[2])]
        }
    });
    b
}

fn gamma_pg(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let ch = |n: &str| (ctx.params.f(&format!("{n}Gamma")).max(0.01) as f32, ctx.params.f(&format!("{n}Pedestal")) as f32, ctx.params.f(&format!("{n}Gain")) as f32);
    let r = ch("red");
    let g = ch("green");
    let bl = ch("blue");
    b.img.map_straight(|c| {
        let f = |v: f32, (gm, pd, gn): (f32, f32, f32)| (pd + (gn - pd) * v.max(0.0).powf(gm)).max(0.0);
        [f(c[0], r), f(c[1], g), f(c[2], bl)]
    });
    b
}

fn colorama(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let shift = ctx.params.f("phaseShift") as f32 / 360.0;
    let cycles = ctx.params.f("cycles").max(0.0) as f32;
    let blend = 1.0 - ctx.params.f("blend") as f32 / 100.0;
    b.img.map_straight(|c| {
        let l = luminance(c[0], c[1], c[2]);
        let (r, g, bl) = effectcraft_color::hsv_to_rgb((l * cycles + shift).rem_euclid(1.0), 1.0, 1.0);
        mix(c, [r, g, bl], blend)
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let pct = || slider(-100.0, 100.0, -100.0, 100.0, 1);
    let mut gpg = Vec::new();
    for (n, label) in [("red", "Red"), ("green", "Green"), ("blue", "Blue")] {
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        gpg.push(p(leak(format!("{n}Gamma")), leak(format!("{label} Gamma")), num(1.0), slider(0.1, 10.0, 0.1, 4.0, 2)));
        gpg.push(p(leak(format!("{n}Pedestal")), leak(format!("{label} Pedestal")), num(0.0), slider(-2.0, 2.0, -1.0, 1.0, 2)));
        gpg.push(p(leak(format!("{n}Gain")), leak(format!("{label} Gain")), num(1.0), slider(0.0, 4.0, 0.0, 2.0, 2)));
    }
    vec![
        spec(
            "ec.color.tint",
            "Tint",
            vec![
                p("black", "Map Black To", col(0.0, 0.0, 0.0), ParamUi::Color),
                p("white", "Map White To", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("amount", "Amount to Tint", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            tint,
        ),
        spec(
            "ec.color.tritone",
            "Tritone",
            vec![
                p("highlights", "Highlights", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("midtones", "Midtones", col(0.5, 0.4, 0.3), ParamUi::Color),
                p("shadows", "Shadows", col(0.0, 0.0, 0.0), ParamUi::Color),
                p("blend", "Blend With Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            tritone,
        ),
        spec(
            "ec.color.brightnesscontrast",
            "Brightness & Contrast",
            vec![p("brightness", "Brightness", num(0.0), slider(-150.0, 150.0, -150.0, 150.0, 1)), p("contrast", "Contrast", num(0.0), pct())],
            brightness_contrast,
        ),
        spec(
            "ec.color.huesaturation",
            "Hue/Saturation",
            vec![
                p("hue", "Master Hue", num(0.0), ParamUi::Angle),
                p("saturation", "Master Saturation", num(0.0), pct()),
                p("lightness", "Master Lightness", num(0.0), pct()),
                p("colorize", "Colorize", Value::Bool(false), ParamUi::Checkbox),
                p("colorizeHue", "Colorize Hue", num(0.0), ParamUi::Angle),
                p("colorizeSaturation", "Colorize Saturation", num(25.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("colorizeLightness", "Colorize Lightness", num(0.0), pct()),
            ],
            hue_saturation,
        ),
        spec(
            "ec.color.levels",
            "Levels",
            vec![
                p("inBlack", "Input Black", num(0.0), slider(-1.0, 2.0, 0.0, 1.0, 3)),
                p("inWhite", "Input White", num(1.0), slider(-1.0, 2.0, 0.0, 1.0, 3)),
                p("gamma", "Gamma", num(1.0), slider(0.1, 10.0, 0.1, 3.0, 2)),
                p("outBlack", "Output Black", num(0.0), slider(-1.0, 2.0, 0.0, 1.0, 3)),
                p("outWhite", "Output White", num(1.0), slider(-1.0, 2.0, 0.0, 1.0, 3)),
                p("noClip", "Don't Clip", Value::Bool(false), ParamUi::Checkbox),
            ],
            levels,
        ),
        spec(
            "ec.color.exposure",
            "Exposure",
            vec![
                p("exposure", "Exposure", num(0.0), slider(-20.0, 20.0, -5.0, 5.0, 2)),
                p("offset", "Offset", num(0.0), slider(-2.0, 2.0, -0.5, 0.5, 4)),
                p("gamma", "Gamma Correction", num(1.0), slider(0.01, 9.99, 0.1, 3.0, 2)),
            ],
            exposure,
        ),
        spec(
            "ec.color.blackwhite",
            "Black & White",
            vec![
                p("reds", "Reds", num(40.0), slider(-200.0, 300.0, -200.0, 300.0, 0)),
                p("yellows", "Yellows", num(60.0), slider(-200.0, 300.0, -200.0, 300.0, 0)),
                p("greens", "Greens", num(40.0), slider(-200.0, 300.0, -200.0, 300.0, 0)),
                p("cyans", "Cyans", num(60.0), slider(-200.0, 300.0, -200.0, 300.0, 0)),
                p("blues", "Blues", num(20.0), slider(-200.0, 300.0, -200.0, 300.0, 0)),
                p("magentas", "Magentas", num(80.0), slider(-200.0, 300.0, -200.0, 300.0, 0)),
                p("tint", "Tint", Value::Bool(false), ParamUi::Checkbox),
                p("tintColor", "Tint Color", col(0.88, 0.76, 0.6), ParamUi::Color),
            ],
            black_white,
        ),
        EffectSpec {
            id: "ec.generate.fill",
            name: "Fill",
            category: "Generate",
            params: vec![
                p("color", "Color", col(1.0, 0.0, 0.0), ParamUi::Color),
                p("invert", "Invert", Value::Bool(false), ParamUi::Checkbox),
                p("opacity", "Opacity", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            render: fill,
            gpu: false,
            float: true,
        },
        spec(
            "ec.color.changetocolor",
            "Change to Color",
            vec![
                p("from", "From", col(1.0, 0.0, 0.0), ParamUi::Color),
                p("to", "To", col(0.0, 0.0, 1.0), ParamUi::Color),
                p("tolerance", "Tolerance", num(10.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("softness", "Softness", num(10.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            change_to_color,
        ),
        spec(
            "ec.color.leavecolor",
            "Leave Color",
            vec![
                p("amount", "Amount to Decolor", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("color", "Color To Leave", col(1.0, 0.0, 0.0), ParamUi::Color),
                p("tolerance", "Tolerance", num(15.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            leave_color,
        ),
        spec(
            "ec.color.photofilter",
            "Photo Filter",
            vec![
                p("color", "Color", col(0.93, 0.54, 0.09), ParamUi::Color),
                p("density", "Density", num(25.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("preserveLuminosity", "Preserve Luminosity", Value::Bool(true), ParamUi::Checkbox),
            ],
            photo_filter,
        ),
        spec("ec.color.vibrance", "Vibrance", vec![p("vibrance", "Vibrance", num(0.0), pct()), p("saturation", "Saturation", num(0.0), pct())], vibrance),
        spec(
            "ec.color.colorbalance",
            "Color Balance",
            ["shadowRed", "shadowGreen", "shadowBlue", "midRed", "midGreen", "midBlue", "hiRed", "hiGreen", "hiBlue"]
                .iter()
                .zip([
                    "Shadow Red Balance",
                    "Shadow Green Balance",
                    "Shadow Blue Balance",
                    "Midtone Red Balance",
                    "Midtone Green Balance",
                    "Midtone Blue Balance",
                    "Highlight Red Balance",
                    "Highlight Green Balance",
                    "Highlight Blue Balance",
                ])
                .map(|(id, n)| p(id, n, num(0.0), pct()))
                .collect(),
            color_balance,
        ),
        spec(
            "ec.color.channelmixer",
            "Channel Mixer",
            {
                let ids = ["rr", "rg", "rb", "rc", "gr", "gg", "gb", "gc", "br", "bg", "bb", "bc"];
                let names = [
                    "Red-Red", "Red-Green", "Red-Blue", "Red-Const", "Green-Red", "Green-Green", "Green-Blue", "Green-Const", "Blue-Red", "Blue-Green", "Blue-Blue",
                    "Blue-Const",
                ];
                let mut v: Vec<_> = ids
                    .iter()
                    .zip(names)
                    .map(|(id, n)| p(id, n, num(if matches!(*id, "rr" | "gg" | "bb") { 100.0 } else { 0.0 }), slider(-200.0, 200.0, -200.0, 200.0, 0)))
                    .collect();
                v.push(p("monochrome", "Monochrome", Value::Bool(false), ParamUi::Checkbox));
                v
            },
            channel_mixer,
        ),
        spec("ec.color.gammapedestalgain", "Gamma/Pedestal/Gain", gpg, gamma_pg),
        spec(
            "ec.color.colorama",
            "Colorama",
            vec![
                p("phaseShift", "Phase Shift", num(0.0), ParamUi::Angle),
                p("cycles", "Cycle Repetitions", num(1.0), slider(0.0, 100.0, 0.0, 10.0, 1)),
                p("blend", "Blend With Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("mode", "Get Phase From", Value::Enum(0), popup(&["Intensity", "Red", "Green", "Blue", "Hue", "Lightness", "Saturation", "Value", "Alpha"])),
            ],
            colorama,
        ),
    ]
}
