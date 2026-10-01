//! Noise & Grain effects: film grain, median-family filters, denoising and procedural noise.

use effectcraft_color::{hsl_to_rgb, luminance, rgb_to_hsl};
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::{Image, Px};
use rayon::prelude::*;

use crate::generate::value_noise;
use crate::util::{Plane, gauss_plane, guided_filter, hash1, join, lerp, premul, smoothstep, split, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Noise & Grain", params, render, gpu: false, float: true }
}

fn pct() -> ParamUi {
    slider(0.0, 100.0, 0.0, 100.0, 1)
}

/// Fractional-octave fBm of value noise in 0..1 (octave amplitude 0.5, frequency 2).
pub(crate) fn fbm(u: f32, v: f32, z: f32, seed: u32, octaves: f32) -> f32 {
    let octaves = octaves.clamp(1.0, 20.0);
    let n = octaves.ceil() as usize;
    let frac = octaves - octaves.floor();
    let (mut sum, mut norm, mut amp, mut f) = (0.0, 0.0, 1.0, 1.0);
    for o in 0..n {
        let w = if o + 1 == n && frac > 0.0 { frac } else { 1.0 };
        sum += value_noise(u * f, v * f, z + o as f32 * 7.31, seed.wrapping_add(o as u32)) * amp * w;
        norm += amp * w;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm.max(1e-6)
}

// ---- Add Grain ----

fn add_grain(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let intensity = ctx.params.f("intensity") as f32;
    if intensity <= 0.0 {
        return b;
    }
    let size = (ctx.params.f("size") * b.scale).max(0.05) as f32;
    let aspect = ctx.params.f("aspectRatio").max(0.05) as f32;
    let soft = ctx.params.f("softness") * b.scale;
    let mono = ctx.params.b("monochromatic");
    let sat = ctx.params.f("saturation") as f32;
    let (ws, wm, wh) = (ctx.params.f("shadows") as f32, ctx.params.f("midtones") as f32, ctx.params.f("highlights") as f32);
    let seed = (ctx.params.f("randomSeed") as i64 as u32) ^ ctx.seed.wrapping_mul(0x9e37);
    let frame = (ctx.time * 24.0 * ctx.params.f("animationSpeed")).floor() as i64 as f32;
    let (w, h) = (b.img.width as usize, b.img.height as usize);
    let nch = if mono { 1 } else { 3 };
    let mut planes: Vec<Plane> = (0..nch)
        .map(|k| {
            let mut pl = Plane::new(w, h);
            pl.data.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
                for (x, v) in row.iter_mut().enumerate() {
                    let n = value_noise(x as f32 / size, y as f32 / (size * aspect), frame, seed.wrapping_add(k as u32 * 101));
                    *v = (n - 0.5) * 2.5;
                }
            });
            pl
        })
        .collect();
    if soft > 0.05 {
        planes = planes.iter().map(|pl| gauss_plane(pl, soft, soft * aspect as f64)).collect();
    }
    let amp = intensity * 0.1;
    b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let (c, a) = unpremul(*px);
        if a <= 0.0 {
            return;
        }
        let mut g = if mono { [planes[0].data[i]; 3] } else { [planes[0].data[i], planes[1].data[i], planes[2].data[i]] };
        let gm = (g[0] + g[1] + g[2]) / 3.0;
        g = g.map(|v| gm + (v - gm) * sat);
        let l = luminance(c[0], c[1], c[2]).clamp(0.0, 1.0);
        let s = 1.0 - smoothstep(0.0, 0.5, l);
        let hi = smoothstep(0.5, 1.0, l);
        let weight = ws * s + wm * (1.0 - s - hi) + wh * hi;
        let o = [0, 1, 2].map(|k| (c[k] + g[k] * amp * weight).max(0.0));
        *px = premul(o, a);
    });
    b
}

// ---- Median (Huang sliding histogram) ----

const NB: usize = 512;

#[inline]
fn quant(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * (NB - 1) as f32).round() as u16
}

/// Per-channel median over a (2r+1)² window of the premultiplied channels (values quantised to
/// 512 levels in 0..1, edges repeated).
pub(crate) fn median_image(img: &Image, r: usize) -> Image {
    let (w, h) = (img.width as usize, img.height as usize);
    if r == 0 || w == 0 || h == 0 {
        return img.clone();
    }
    let q: Vec<[u16; 4]> = img.data.par_iter().map(|p| [quant(p[0]), quant(p[1]), quant(p[2]), quant(p[3])]).collect();
    let n = (2 * r + 1) * (2 * r + 1);
    let half = n / 2;
    let ri = r as i64;
    let mut out = Image::new(img.width, img.height);
    out.rows_mut().for_each(|(y, row)| {
        let mut hist = vec![0u32; NB * 4];
        let mut med = [0usize; 4];
        let mut lt = [0usize; 4];
        let at = |x: i64, y: i64| q[y.clamp(0, h as i64 - 1) as usize * w + x.clamp(0, w as i64 - 1) as usize];
        let col = |x: i64, f: &mut dyn FnMut([u16; 4])| {
            for dy in -ri..=ri {
                f(at(x, y as i64 + dy));
            }
        };
        for dx in -ri..=ri {
            col(dx, &mut |v| {
                for c in 0..4 {
                    hist[c * NB + v[c] as usize] += 1;
                }
            });
        }
        let rebalance = |hist: &[u32], med: &mut [usize; 4], lt: &mut [usize; 4]| {
            for c in 0..4 {
                let hc = &hist[c * NB..(c + 1) * NB];
                while lt[c] > half {
                    med[c] -= 1;
                    lt[c] -= hc[med[c]] as usize;
                }
                while lt[c] + hc[med[c]] as usize <= half {
                    lt[c] += hc[med[c]] as usize;
                    med[c] += 1;
                }
            }
        };
        rebalance(&hist, &mut med, &mut lt);
        for x in 0..w {
            row[x] = med.map(|m| m as f32 / (NB - 1) as f32);
            if x + 1 == w {
                break;
            }
            col(x as i64 - ri, &mut |v| {
                for c in 0..4 {
                    hist[c * NB + v[c] as usize] -= 1;
                    if (v[c] as usize) < med[c] {
                        lt[c] -= 1;
                    }
                }
            });
            col(x as i64 + ri + 1, &mut |v| {
                for c in 0..4 {
                    hist[c * NB + v[c] as usize] += 1;
                    if (v[c] as usize) < med[c] {
                        lt[c] += 1;
                    }
                }
            });
            rebalance(&hist, &mut med, &mut lt);
        }
    });
    out
}

/// Median result honouring "operate on alpha": otherwise keep the original alpha and take the
/// median's straight colour.
fn median_px(orig: Px, m: Px, on_alpha: bool) -> Px {
    if on_alpha {
        let a = m[3].clamp(0.0, 1.0);
        return [m[0].min(a), m[1].min(a), m[2].min(a), a];
    }
    let a = orig[3];
    if m[3] <= 1e-4 {
        return orig;
    }
    premul([m[0] / m[3], m[1] / m[3], m[2] / m[3]].map(|v| v.min(1.0)), a)
}

fn median(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let r = (ctx.params.f("radius") * b.scale).round().max(0.0) as usize;
    if r == 0 {
        return b;
    }
    let on_alpha = ctx.params.b("operateOnAlpha");
    let m = median_image(&b.img, r);
    b.img.data.par_iter_mut().zip(m.data.par_iter()).for_each(|(o, &mv)| *o = median_px(*o, mv, on_alpha));
    b
}

fn dust_scratches(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let r = (ctx.params.f("radius") * b.scale).round().max(0.0) as usize;
    if r == 0 {
        return b;
    }
    let thr = ctx.params.f("threshold") as f32 / 255.0;
    let on_alpha = ctx.params.b("operateOnAlpha");
    let m = median_image(&b.img, r);
    b.img.data.par_iter_mut().zip(m.data.par_iter()).for_each(|(o, &mv)| {
        let cand = median_px(*o, mv, on_alpha);
        let diff = (0..4).map(|c| (cand[c] - o[c]).abs()).fold(0.0f32, f32::max);
        if diff > thr {
            *o = cand;
        }
    });
    b
}

fn remove_grain(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let amt = ctx.params.f("noiseReduction") as f32;
    if amt <= 0.0 {
        return b;
    }
    let passes = ctx.params.f("passes").round().clamp(1.0, 4.0) as usize;
    let r = (2.0 * b.scale).round().max(1.0) as usize;
    let eps = (0.02 * amt).powi(2);
    let mut ch = split(&b.img);
    for c in ch.iter_mut().take(3) {
        for _ in 0..passes {
            *c = guided_filter(c, c, r, eps);
        }
    }
    let alpha = ch[3].clone();
    let mut out = join(&ch);
    out.data.par_iter_mut().zip(alpha.data.par_iter()).for_each(|(p, &a)| {
        for c in 0..3 {
            p[c] = p[c].max(0.0);
        }
        p[3] = a;
    });
    b.img = out;
    b
}

// ---- Turbulent Noise ----

fn overflow(v: f32, mode: u32) -> f32 {
    match mode {
        1 => (0.5 + 0.5 * (2.0 * (v - 0.5)).tanh() / 1f32.tanh()).clamp(0.0, 1.0),
        2 => {
            let t = v.rem_euclid(2.0);
            if t > 1.0 { 2.0 - t } else { t }
        }
        _ => v.clamp(0.0, 1.0),
    }
}

fn turbulent_noise(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let kind = ctx.params.e("fractalType");
    let invert = ctx.params.b("invert");
    let contrast = ctx.params.f("contrast") as f32 / 100.0;
    let brightness = ctx.params.f("brightness") as f32 / 100.0;
    let ov = ctx.params.e("overflow");
    let scale = (ctx.params.f("scale") * b.scale).max(1.0) as f32;
    let rot = (ctx.params.f("rotation") as f32).to_radians();
    let octaves = ctx.params.f("complexity").clamp(1.0, 20.0) as f32;
    let infl = (ctx.params.f("subInfluence") as f32 / 100.0).clamp(0.0, 1.0);
    let sub = (ctx.params.f("subScaling") as f32 / 100.0).clamp(0.1, 1.0);
    let evo = ctx.params.f("evolution") as f32 / 360.0;
    let seed = (ctx.params.f("randomSeed") as i64 as u32) ^ 0x7a3d;
    let opacity = ctx.params.f("opacity") as f32 / 100.0;
    let blend = ctx.params.f("blend") as f32 / 100.0;
    let (sr, cr) = rot.sin_cos();
    let (ox, oy) = b.to_px(ctx.params.v2("offset"));
    let n_oct = octaves.ceil() as usize;
    let frac = octaves - octaves.floor();
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (dx, dy) = (x as f32 + 0.5 - ox as f32, y as f32 + 0.5 - oy as f32);
            let (mut u, mut v) = ((dx * cr + dy * sr) / scale, (-dx * sr + dy * cr) / scale);
            if kind == 3 {
                // Dynamic: domain-warped by a low-frequency field.
                u += (value_noise(u * 0.5, v * 0.5, evo, seed ^ 0x55) - 0.5) * 1.5;
                v += (value_noise(u * 0.5, v * 0.5, evo, seed ^ 0xaa) - 0.5) * 1.5;
            }
            let (mut sum, mut norm, mut amp, mut f) = (0.0, 0.0, 1.0, 1.0);
            for o in 0..n_oct {
                let w = if o + 1 == n_oct && frac > 0.0 { frac } else { 1.0 };
                let n = value_noise(u * f, v * f, evo + o as f32 * 5.17, seed.wrapping_add(o as u32 * 31));
                let n = match kind {
                    1 => 1.0 - (n * 2.0 - 1.0).powi(2),
                    2 => 1.0 - (n * 2.0 - 1.0).abs(),
                    _ => n,
                };
                sum += n * amp * w;
                norm += amp * w;
                amp *= infl;
                f /= sub;
            }
            let mut val = sum / norm.max(1e-6);
            val = overflow((val - 0.5) * contrast + 0.5 + brightness, ov);
            if invert {
                val = 1.0 - val;
            }
            let g = [val * opacity, val * opacity, val * opacity, opacity];
            for c in 0..4 {
                px[c] = px[c] * blend + g[c] * (1.0 - blend);
            }
        }
    });
    b
}

// ---- Noise Alpha / Noise HLS ----

/// Per-pixel noise in 0..1 that changes smoothly with `phase` (whole units = new pattern).
#[inline]
fn phased(x: u32, y: u32, seed: u32, phase: f32) -> f32 {
    let k = phase.floor();
    let t = phase - k;
    let k = k as i64 as u32;
    let a = hash1(x, y, seed.wrapping_add(k.wrapping_mul(0x632b)));
    let b = hash1(x, y, seed.wrapping_add(k.wrapping_add(1).wrapping_mul(0x632b)));
    lerp(a, b, t * t * (3.0 - 2.0 * t))
}

fn noise_alpha(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let k = ctx.params.f("amount") as f32 / 100.0;
    if k <= 0.0 {
        return b;
    }
    let kind = ctx.params.e("noise");
    let orig = ctx.params.e("originalAlpha");
    let ov = ctx.params.e("overflow");
    let seed = (ctx.params.f("randomSeed") as i64 as u32) ^ ctx.seed.wrapping_mul(0x2f1);
    let phase = ctx.params.f("noisePhase") as f32 / 360.0 + if kind >= 2 { ctx.time as f32 } else { 0.0 };
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let mut n = if kind >= 2 { phased(x as u32, y as u32, seed, phase) } else { phased(x as u32, y as u32, seed, phase.floor()) };
            if kind % 2 == 1 {
                n *= n;
            }
            let (c, a) = unpremul(*px);
            let na = match orig {
                0 => a + n * k,
                1 => {
                    if a > 0.0 {
                        a + n * k
                    } else {
                        0.0
                    }
                }
                2 => a * (1.0 + (2.0 * n - 1.0) * k),
                _ => a + (2.0 * n - 1.0) * k * 4.0 * a * (1.0 - a),
            };
            let na = match ov {
                1 => {
                    let t = na.rem_euclid(2.0);
                    if t > 1.0 { 2.0 - t } else { t }
                }
                2 => {
                    if !(0.0..=1.0).contains(&na) {
                        na.rem_euclid(1.0)
                    } else {
                        na
                    }
                }
                _ => na,
            }
            .clamp(0.0, 1.0);
            *px = premul(c, na);
        }
    });
    b
}

fn noise_hls_core(ctx: &EffectCtx, mut b: Buf, phase: f32) -> Buf {
    let hue = ctx.params.f("hue") as f32 / 100.0;
    let light = ctx.params.f("lightness") as f32 / 100.0;
    let satv = ctx.params.f("saturation") as f32 / 100.0;
    if hue == 0.0 && light == 0.0 && satv == 0.0 {
        return b;
    }
    let kind = ctx.params.e("noise");
    let gs = (ctx.params.f("grainSize") * b.scale).max(0.1) as f32;
    let seed = ctx.seed.wrapping_mul(0x51f3);
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (c, a) = unpremul(*px);
            if a <= 0.0 {
                continue;
            }
            let n = |k: u32| -> f32 {
                let s = seed.wrapping_add(k * 7919);
                match kind {
                    2 => ((value_noise(x as f32 / gs, y as f32 / gs, phase, s) - 0.5) * 3.0).clamp(-1.0, 1.0),
                    1 => {
                        let v = phased(x as u32, y as u32, s, phase) * 2.0 - 1.0;
                        v * v.abs()
                    }
                    _ => phased(x as u32, y as u32, s, phase) * 2.0 - 1.0,
                }
            };
            let (mut h, mut s, mut l) = rgb_to_hsl(c[0], c[1], c[2]);
            if hue != 0.0 {
                h = (h + n(0) * hue * 0.5).rem_euclid(1.0);
            }
            if light != 0.0 {
                l = (l + n(1) * light).clamp(0.0, 1.0);
            }
            if satv != 0.0 {
                s = (s + n(2) * satv).clamp(0.0, 1.0);
            }
            let (r, g, bl) = hsl_to_rgb(h, s, l);
            *px = premul([r, g, bl], a);
        }
    });
    b
}

fn noise_hls(ctx: &EffectCtx, b: Buf) -> Buf {
    let phase = ctx.params.f("noisePhase") as f32 / 360.0;
    noise_hls_core(ctx, b, phase)
}

fn noise_hls_auto(ctx: &EffectCtx, b: Buf) -> Buf {
    let phase = ctx.time as f32 * ctx.params.f("noiseAnimationSpeed") as f32;
    noise_hls_core(ctx, b, phase)
}

pub fn specs() -> Vec<EffectSpec> {
    let hls_common = || {
        vec![
            p("noise", "Noise", Value::Enum(0), popup(&["Uniform", "Squared", "Grain"])),
            p("hue", "Hue", num(0.0), pct()),
            p("lightness", "Lightness", num(0.0), pct()),
            p("saturation", "Saturation", num(0.0), pct()),
            p("grainSize", "Grain Size", num(1.0), slider(0.1, 100.0, 0.5, 10.0, 2)),
        ]
    };
    let mut hls = hls_common();
    hls.push(p("noisePhase", "Noise Phase", num(0.0), ParamUi::Angle));
    let mut hls_auto = hls_common();
    hls_auto.push(p("noiseAnimationSpeed", "Noise Animation Speed", num(10.0), slider(0.0, 1000.0, 0.0, 30.0, 1)));
    vec![
        spec(
            "ec.noise.addgrain",
            "Add Grain",
            vec![
                p("intensity", "Intensity", num(1.0), slider(0.0, 100.0, 0.0, 10.0, 2)),
                p("size", "Size", num(1.0), slider(0.05, 100.0, 0.1, 10.0, 2)),
                p("softness", "Softness", num(0.0), slider(0.0, 100.0, 0.0, 5.0, 2)),
                p("aspectRatio", "Aspect Ratio", num(1.0), slider(0.05, 20.0, 0.25, 4.0, 2)),
                p("monochromatic", "Monochromatic", Value::Bool(false), ParamUi::Checkbox),
                p("saturation", "Saturation", num(1.0), slider(0.0, 10.0, 0.0, 2.0, 2)),
                p("shadows", "Shadows", num(1.0), slider(0.0, 10.0, 0.0, 2.0, 2)),
                p("midtones", "Midtones", num(1.0), slider(0.0, 10.0, 0.0, 2.0, 2)),
                p("highlights", "Highlights", num(1.0), slider(0.0, 10.0, 0.0, 2.0, 2)),
                p("randomSeed", "Random Seed", num(0.0), slider(0.0, 100000.0, 0.0, 1000.0, 0)),
                p("animationSpeed", "Animation Speed", num(1.0), slider(0.0, 100.0, 0.0, 5.0, 2)),
            ],
            add_grain,
        ),
        spec(
            "ec.noise.median",
            "Median",
            vec![
                p("radius", "Radius", num(0.0), slider(0.0, 255.0, 0.0, 50.0, 0)),
                p("operateOnAlpha", "Operate on Alpha Channel", Value::Bool(false), ParamUi::Checkbox),
            ],
            median,
        ),
        spec(
            "ec.noise.dustscratches",
            "Dust & Scratches",
            vec![
                p("radius", "Radius", num(1.0), slider(0.0, 255.0, 0.0, 50.0, 0)),
                p("threshold", "Threshold", num(0.0), slider(0.0, 255.0, 0.0, 255.0, 0)),
                p("operateOnAlpha", "Operate on Alpha Channel", Value::Bool(false), ParamUi::Checkbox),
            ],
            dust_scratches,
        ),
        spec(
            "ec.noise.removegrain",
            "Remove Grain",
            vec![
                p("noiseReduction", "Noise Reduction", num(1.0), slider(0.0, 5.0, 0.0, 5.0, 2)),
                p("passes", "Passes", num(1.0), slider(1.0, 4.0, 1.0, 4.0, 0)),
            ],
            remove_grain,
        ),
        spec(
            "ec.noise.turbulent",
            "Turbulent Noise",
            vec![
                p("fractalType", "Fractal Type", Value::Enum(1), popup(&["Basic", "Turbulent Smooth", "Turbulent Sharp", "Dynamic"])),
                p("invert", "Invert", Value::Bool(false), ParamUi::Checkbox),
                p("contrast", "Contrast", num(100.0), slider(0.0, 10000.0, 0.0, 400.0, 1)),
                p("brightness", "Brightness", num(0.0), slider(-10000.0, 10000.0, -200.0, 200.0, 1)),
                p("overflow", "Overflow", Value::Enum(0), popup(&["Clip", "Soft Clamp", "Wrap Back"])),
                p("scale", "Scale", num(100.0), slider(1.0, 10000.0, 20.0, 600.0, 1)),
                p("rotation", "Rotation", num(0.0), ParamUi::Angle),
                p("offset", "Offset Turbulence", Value::Vec2([0.5, 0.5]), ParamUi::Point),
                p("complexity", "Complexity", num(6.0), slider(1.0, 20.0, 1.0, 10.0, 1)),
                p("subInfluence", "Sub Influence (%)", num(70.0), slider(0.0, 100.0, 25.0, 100.0, 1)),
                p("subScaling", "Sub Scaling", num(56.0), slider(10.0, 100.0, 25.0, 100.0, 1)),
                p("evolution", "Evolution", num(0.0), ParamUi::Angle),
                p("randomSeed", "Random Seed", num(0.0), slider(0.0, 100000.0, 0.0, 1000.0, 0)),
                p("opacity", "Opacity", num(100.0), pct()),
                p("blend", "Blend With Original", num(0.0), pct()),
            ],
            turbulent_noise,
        ),
        spec(
            "ec.noise.noisealpha",
            "Noise Alpha",
            vec![
                p("noise", "Noise", Value::Enum(0), popup(&["Uniform Random", "Squared Random", "Uniform Animation", "Squared Animation"])),
                p("amount", "Amount", num(0.0), pct()),
                p("originalAlpha", "Original Alpha", Value::Enum(1), popup(&["Add", "Clamp", "Scale", "Edges"])),
                p("overflow", "Overflow", Value::Enum(0), popup(&["Clip", "Wrap Back", "Wrap"])),
                p("randomSeed", "Random Seed", num(0.0), slider(0.0, 100000.0, 0.0, 1000.0, 0)),
                p("noisePhase", "Noise Phase", num(0.0), ParamUi::Angle),
            ],
            noise_alpha,
        ),
        spec("ec.noise.noisehls", "Noise HLS", hls, noise_hls),
        spec("ec.noise.noisehlsauto", "Noise HLS Auto", hls_auto, noise_hls_auto),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params;

    fn run(id: &str, vals: &[(&str, Value)], img: Image, time: f64) -> Buf {
        let s = crate::find(id).unwrap();
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        for (k, v) in vals {
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx = EffectCtx { params: &params, time, layer_size: [img.width as f64, img.height as f64], seed: 3, adjustment: false };
        crate::apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 })
    }

    fn gray(w: u32, h: u32, v: f32) -> Image {
        Image::filled(w, h, [v, v, v, 1.0])
    }

    #[test]
    fn median_removes_salt_noise() {
        let mut img = gray(16, 16, 0.5);
        for (x, y) in [(3, 3), (8, 2), (12, 12), (5, 10)] {
            img.set(x, y, [1.0, 1.0, 1.0, 1.0]);
        }
        let out = run("ec.noise.median", &[("radius", num(1.0))], img, 0.0);
        for p in &out.img.data {
            assert!((p[0] - 0.5).abs() < 2.0 / 511.0, "{p:?}");
        }
    }

    #[test]
    fn median_matches_brute_force() {
        let (w, h) = (9u32, 7u32);
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = |k: u32| ((x * 37 + y * 101 + k * 13) % 511) as f32 / 511.0;
                img.set(x, y, [v(0), v(1), v(2), v(3)]);
            }
        }
        for r in 1..3usize {
            let m = median_image(&img, r);
            for y in 0..h as i64 {
                for x in 0..w as i64 {
                    for c in 0..4 {
                        let mut vals: Vec<f32> = vec![];
                        for dy in -(r as i64)..=r as i64 {
                            for dx in -(r as i64)..=r as i64 {
                                vals.push(img.get_clamped(x + dx, y + dy)[c]);
                            }
                        }
                        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        let want = vals[vals.len() / 2];
                        let got = m.data[y as usize * w as usize + x as usize][c];
                        assert!((want - got).abs() < 1e-4, "r={r} ({x},{y}) c={c}: {want} vs {got}");
                    }
                }
            }
        }
    }

    #[test]
    fn dust_threshold_keeps_detail() {
        let mut img = gray(12, 12, 0.5);
        img.set(6, 6, [0.56, 0.56, 0.56, 1.0]);
        let keep = run("ec.noise.dustscratches", &[("radius", num(1.0)), ("threshold", num(40.0))], img.clone(), 0.0);
        assert!((keep.img.get(6, 6)[0] - 0.56).abs() < 1e-6);
        let rm = run("ec.noise.dustscratches", &[("radius", num(1.0)), ("threshold", num(5.0))], img, 0.0);
        assert!((rm.img.get(6, 6)[0] - 0.5).abs() < 2.0 / 511.0);
    }

    #[test]
    fn turbulent_noise_seeded() {
        let a = run("ec.noise.turbulent", &[("randomSeed", num(4.0))], gray(24, 16, 0.0), 0.0);
        let b = run("ec.noise.turbulent", &[("randomSeed", num(4.0))], gray(24, 16, 0.0), 0.0);
        let c = run("ec.noise.turbulent", &[("randomSeed", num(5.0))], gray(24, 16, 0.0), 0.0);
        assert_eq!(a.img, b.img);
        assert_ne!(a.img, c.img);
        assert!(a.img.data.iter().all(|p| (0.0..=1.0).contains(&p[0])));
    }

    #[test]
    fn noise_alpha_in_range() {
        for orig in 0..4 {
            for ov in 0..3 {
                let mut img = gray(10, 10, 0.5);
                img.data[3] = [0.0; 4];
                img.data[4] = [0.1, 0.1, 0.1, 0.2];
                let out = run("ec.noise.noisealpha", &[("amount", num(80.0)), ("originalAlpha", Value::Enum(orig)), ("overflow", Value::Enum(ov))], img, 0.3);
                assert!(out.img.data.iter().all(|p| (0.0..=1.0).contains(&p[3]) && p[0] <= p[3] + 1e-6));
            }
        }
    }

    #[test]
    fn noise_hls_changes_and_animates() {
        let mut img = Image::new(8, 8);
        for (i, p) in img.data.iter_mut().enumerate() {
            *p = premul([0.6, 0.3, (i % 7) as f32 / 7.0], 1.0);
        }
        let a = run("ec.noise.noisehlsauto", &[("lightness", num(30.0))], img.clone(), 0.0);
        let b = run("ec.noise.noisehlsauto", &[("lightness", num(30.0))], img.clone(), 0.55);
        assert_ne!(a.img, img);
        assert_ne!(a.img, b.img);
        let still = run("ec.noise.noisehls", &[], img.clone(), 0.0);
        assert_eq!(still.img, img);
    }

    #[test]
    fn add_grain_keeps_flat_mean() {
        let out = run("ec.noise.addgrain", &[("intensity", num(2.0))], gray(64, 64, 0.5), 0.0);
        let mean: f32 = out.img.data.iter().map(|p| p[0]).sum::<f32>() / out.img.data.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
        assert!(out.img.data.iter().any(|p| (p[0] - 0.5).abs() > 0.01));
    }

    #[test]
    fn remove_grain_smooths_noise() {
        let mut img = gray(32, 32, 0.5);
        for (i, p) in img.data.iter_mut().enumerate() {
            let n = hash1(i as u32, 0, 9) * 0.1 - 0.05;
            *p = [0.5 + n, 0.5 + n, 0.5 + n, 1.0];
        }
        let var = |im: &Image| im.data.iter().map(|p| (p[0] - 0.5).powi(2)).sum::<f32>();
        let out = run("ec.noise.removegrain", &[("noiseReduction", num(3.0))], img.clone(), 0.0);
        assert!(var(&out.img) < var(&img) * 0.5);
    }
}
