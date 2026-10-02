//! Color Correction, batch 2: Curves, Levels (Individual Controls), automatic corrections
//! (Auto Levels / Contrast / Color, Equalize), Shadow/Highlight, Selective Color, Color Balance
//! (HLS), Color Link, Broadcast Colors and the CC toners/offsets/kernel.

use effectcraft_color::{BlendMode, blend_pixel, hsl_to_rgb, luminance, rgb_to_hsl};
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::Image;
use rayon::prelude::*;

use crate::util::{Plane, gauss_plane, lerp3, premul, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Color Correction", params, render, gpu: false, float: true }
}

fn pct() -> ParamUi {
    slider(-100.0, 100.0, -100.0, 100.0, 1)
}

/// Map straight colour + alpha of every pixel (row-parallel).
fn map_ca(img: &mut Image, f: impl Fn([f32; 3], f32) -> ([f32; 3], f32) + Sync) {
    img.data.par_iter_mut().for_each(|px| {
        let (c, a) = unpremul(*px);
        if a <= 0.0 && px[0] == 0.0 && px[1] == 0.0 && px[2] == 0.0 {
            // Fully transparent: only alpha transforms can change it.
            let (_, na) = f([0.0; 3], 0.0);
            if na > 0.0 {
                *px = [0.0, 0.0, 0.0, na.clamp(0.0, 1.0)];
            }
            return;
        }
        let (nc, na) = f(c, a);
        *px = premul(nc, na.clamp(0.0, 1.0));
    });
}

// ---------------------------------------------------------------------------------------------
// Curves

/// A tone curve through control points, interpolated with a monotone cubic (Fritsch–Carlson
/// 1980) Hermite spline and extended linearly outside the first/last point.
#[derive(Clone, Debug)]
pub struct Curve {
    xs: Vec<f32>,
    ys: Vec<f32>,
    ms: Vec<f32>,
    lut: Vec<f32>,
}

const LUT_N: usize = 1024;

impl Curve {
    /// Parse "x,y x,y …" (separators: whitespace or ';'). Returns None for an identity curve or
    /// fewer than two valid points.
    pub fn parse(s: &str) -> Option<Curve> {
        let mut pts: Vec<(f32, f32)> = s
            .split(|c: char| c.is_whitespace() || c == ';')
            .filter(|t| !t.is_empty())
            .filter_map(|t| {
                let mut it = t.split(',');
                let x = it.next()?.trim().parse::<f32>().ok()?;
                let y = it.next()?.trim().parse::<f32>().ok()?;
                (x.is_finite() && y.is_finite()).then_some((x, y))
            })
            .collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
        if pts.len() < 2 || pts.iter().all(|(x, y)| (x - y).abs() < 1e-6) {
            return None;
        }
        let xs: Vec<f32> = pts.iter().map(|p| p.0).collect();
        let ys: Vec<f32> = pts.iter().map(|p| p.1).collect();
        let n = xs.len();
        let d: Vec<f32> = (0..n - 1).map(|k| (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k])).collect();
        let mut ms = vec![0.0f32; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for k in 1..n - 1 {
            ms[k] = if d[k - 1] * d[k] <= 0.0 { 0.0 } else { (d[k - 1] + d[k]) * 0.5 };
        }
        for k in 0..n - 1 {
            if d[k].abs() < 1e-12 {
                ms[k] = 0.0;
                ms[k + 1] = 0.0;
            } else {
                let a = ms[k] / d[k];
                let b = ms[k + 1] / d[k];
                let s = a * a + b * b;
                if s > 9.0 {
                    let t = 3.0 / s.sqrt();
                    ms[k] = t * a * d[k];
                    ms[k + 1] = t * b * d[k];
                }
            }
        }
        let mut c = Curve { xs, ys, ms, lut: Vec::new() };
        c.lut = (0..=LUT_N).map(|i| c.eval_exact(i as f32 / LUT_N as f32)).collect();
        Some(c)
    }

    fn eval_exact(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0] + (x - self.xs[0]) * self.ms[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1] + (x - self.xs[n - 1]) * self.ms[n - 1];
        }
        let k = self.xs.partition_point(|&v| v <= x).saturating_sub(1).min(n - 2);
        let h = self.xs[k + 1] - self.xs[k];
        let t = (x - self.xs[k]) / h;
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        h00 * self.ys[k] + h10 * h * self.ms[k] + h01 * self.ys[k + 1] + h11 * h * self.ms[k + 1]
    }

    pub fn eval(&self, x: f32) -> f32 {
        if !(0.0..=1.0).contains(&x) {
            return self.eval_exact(x);
        }
        let f = x * LUT_N as f32;
        let i = (f as usize).min(LUT_N - 1);
        let t = f - i as f32;
        self.lut[i] + (self.lut[i + 1] - self.lut[i]) * t
    }
}

fn ev(c: &Option<Curve>, v: f32) -> f32 {
    c.as_ref().map(|c| c.eval(v)).unwrap_or(v)
}

/// Curves. Each channel's curve is a hidden string parameter (`rgb`, `red`, `green`, `blue`,
/// `alpha`) holding control points as `"x,y x,y …"` in 0..1 (whitespace or `;` separated,
/// sorted on load); the default `"0,0 1,1"` is the identity. Colour channels go through the
/// master RGB curve first, then their own curve.
fn curves(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let [m, r, g, bl, a] = ["rgb", "red", "green", "blue", "alpha"].map(|id| Curve::parse(ctx.params.s(id)));
    if m.is_none() && r.is_none() && g.is_none() && bl.is_none() && a.is_none() {
        return b;
    }
    let chans = [r, g, bl];
    map_ca(&mut b.img, |c, al| {
        let o = [0, 1, 2].map(|i| ev(&chans[i], ev(&m, c[i])));
        (o, ev(&a, al))
    });
    b
}

// ---------------------------------------------------------------------------------------------
// Levels (Individual Controls)

const LV_IDS: [[&str; 5]; 5] = [
    ["rgbInBlack", "rgbInWhite", "rgbGamma", "rgbOutBlack", "rgbOutWhite"],
    ["redInBlack", "redInWhite", "redGamma", "redOutBlack", "redOutWhite"],
    ["greenInBlack", "greenInWhite", "greenGamma", "greenOutBlack", "greenOutWhite"],
    ["blueInBlack", "blueInWhite", "blueGamma", "blueOutBlack", "blueOutWhite"],
    ["alphaInBlack", "alphaInWhite", "alphaGamma", "alphaOutBlack", "alphaOutWhite"],
];
const LV_NAMES: [[&str; 5]; 5] = [
    ["Input Black", "Input White", "Gamma", "Output Black", "Output White"],
    ["Red Input Black", "Red Input White", "Red Gamma", "Red Output Black", "Red Output White"],
    ["Green Input Black", "Green Input White", "Green Gamma", "Green Output Black", "Green Output White"],
    ["Blue Input Black", "Blue Input White", "Blue Gamma", "Blue Output Black", "Blue Output White"],
    ["Alpha Input Black", "Alpha Input White", "Alpha Gamma", "Alpha Output Black", "Alpha Output White"],
];

#[derive(Clone, Copy)]
struct Lv {
    ib: f32,
    iw: f32,
    g: f32,
    ob: f32,
    ow: f32,
}

impl Lv {
    fn identity(&self) -> bool {
        self.ib == 0.0 && self.iw == 1.0 && self.g == 1.0 && self.ob == 0.0 && self.ow == 1.0
    }
    fn apply(&self, v: f32) -> f32 {
        if self.identity() {
            return v;
        }
        let t = ((v - self.ib) / (self.iw - self.ib).max(1e-6)).clamp(0.0, 1.0);
        self.ob + (self.ow - self.ob) * t.powf(1.0 / self.g.max(0.01))
    }
}

fn levels_ic(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let lv = LV_IDS.map(|ids| Lv {
        ib: ctx.params.f(ids[0]) as f32,
        iw: ctx.params.f(ids[1]) as f32,
        g: ctx.params.f(ids[2]) as f32,
        ob: ctx.params.f(ids[3]) as f32,
        ow: ctx.params.f(ids[4]) as f32,
    });
    if lv.iter().all(Lv::identity) {
        return b;
    }
    map_ca(&mut b.img, |c, a| ([0, 1, 2].map(|i| lv[i + 1].apply(lv[0].apply(c[i]))), lv[4].apply(a)));
    b
}

// ---------------------------------------------------------------------------------------------
// Histograms and automatic corrections

const BINS: usize = 1024;

/// Alpha-weighted histograms of straight R, G, B and luminance.
fn histograms(img: &Image) -> [Vec<f64>; 4] {
    img.data
        .par_chunks(4096)
        .map(|chunk| {
            let mut h: [Vec<f64>; 4] = std::array::from_fn(|_| vec![0.0; BINS]);
            for &px in chunk {
                let (c, a) = unpremul(px);
                if a <= 1e-4 {
                    continue;
                }
                let bin = |v: f32| ((v.clamp(0.0, 1.0) * (BINS - 1) as f32).round() as usize).min(BINS - 1);
                for k in 0..3 {
                    h[k][bin(c[k])] += a as f64;
                }
                h[3][bin(luminance(c[0], c[1], c[2]))] += a as f64;
            }
            h
        })
        .reduce(
            || std::array::from_fn(|_| vec![0.0; BINS]),
            |mut a, b| {
                for k in 0..4 {
                    for i in 0..BINS {
                        a[k][i] += b[k][i];
                    }
                }
                a
            },
        )
}

/// (low, high) values clipping `black`/`white` fractions of the histogram.
fn clip_points(h: &[f64], black: f64, white: f64) -> (f32, f32) {
    let total: f64 = h.iter().sum();
    if total <= 0.0 {
        return (0.0, 1.0);
    }
    let mut acc = 0.0;
    let mut lo = 0;
    for (i, v) in h.iter().enumerate() {
        acc += v;
        if acc > total * black {
            lo = i;
            break;
        }
    }
    acc = 0.0;
    let mut hi = BINS - 1;
    for (i, v) in h.iter().enumerate().rev() {
        acc += v;
        if acc > total * white {
            hi = i;
            break;
        }
    }
    let (lo, hi) = (lo as f32 / (BINS - 1) as f32, hi as f32 / (BINS - 1) as f32);
    if hi - lo < 1e-3 { (0.0, 1.0) } else { (lo, hi) }
}

fn stretch(v: f32, lo: f32, hi: f32) -> f32 {
    ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// 0 = Auto Levels (per channel), 1 = Auto Contrast (shared), 2 = Auto Color (per channel + neutral mids).
fn auto_correct(ctx: &EffectCtx, mut b: Buf, kind: u32) -> Buf {
    let black = ctx.params.f("blackClip").clamp(0.0, 49.0) / 100.0;
    let white = ctx.params.f("whiteClip").clamp(0.0, 49.0) / 100.0;
    let blend = (ctx.params.f("blend") / 100.0).clamp(0.0, 1.0) as f32;
    let h = histograms(&b.img);
    let ranges: [(f32, f32); 3] = if kind == 1 {
        let mut combined = vec![0.0; BINS];
        for k in 0..3 {
            for i in 0..BINS {
                combined[i] += h[k][i];
            }
        }
        let r = clip_points(&combined, black, white);
        [r; 3]
    } else {
        [0, 1, 2].map(|k| clip_points(&h[k], black, white))
    };
    // Auto Color: per-channel gamma that moves each channel's mean to the common mean.
    let mut gam = [1.0f32; 3];
    if kind == 2 && ctx.params.b("snapNeutralMidtones") {
        let mut sums = [0.0f64; 4];
        for px in &b.img.data {
            let (c, a) = unpremul(*px);
            if a > 1e-4 {
                for k in 0..3 {
                    sums[k] += (stretch(c[k], ranges[k].0, ranges[k].1) * a) as f64;
                }
                sums[3] += a as f64;
            }
        }
        if sums[3] > 0.0 {
            let means = [0, 1, 2].map(|k| (sums[k] / sums[3]) as f32);
            let target = (means[0] + means[1] + means[2]) / 3.0;
            for k in 0..3 {
                if means[k] > 1e-3 && means[k] < 0.999 && target > 1e-3 && target < 0.999 {
                    gam[k] = (target.ln() / means[k].ln()).clamp(0.2, 5.0);
                }
            }
        }
    }
    map_ca(&mut b.img, |c, a| {
        let o = [0, 1, 2].map(|k| stretch(c[k], ranges[k].0, ranges[k].1).powf(gam[k]));
        (lerp3(o, c, blend), a)
    });
    b
}

fn auto_levels(ctx: &EffectCtx, b: Buf) -> Buf {
    auto_correct(ctx, b, 0)
}
fn auto_contrast(ctx: &EffectCtx, b: Buf) -> Buf {
    auto_correct(ctx, b, 1)
}
fn auto_color(ctx: &EffectCtx, b: Buf) -> Buf {
    auto_correct(ctx, b, 2)
}

fn cdf(h: &[f64]) -> Vec<f32> {
    let total: f64 = h.iter().sum::<f64>().max(1e-12);
    let mut acc = 0.0;
    h.iter()
        .map(|v| {
            acc += v;
            (acc / total) as f32
        })
        .collect()
}

fn equalize(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let amt = (ctx.params.f("amount") / 100.0).clamp(0.0, 1.0) as f32;
    if amt <= 0.0 {
        return b;
    }
    let style = ctx.params.e("style");
    let h = histograms(&b.img);
    let look = |c: &Vec<f32>, v: f32| c[((v.clamp(0.0, 1.0) * (BINS - 1) as f32).round() as usize).min(BINS - 1)];
    match style {
        1 => {
            let c = cdf(&h[3]);
            map_ca(&mut b.img, |col, a| {
                let l = luminance(col[0], col[1], col[2]);
                let nl = look(&c, l);
                let o = if l > 1e-5 { col.map(|v| v * nl / l) } else { [nl; 3] };
                (lerp3(col, o, amt), a)
            });
        }
        2 => {
            let mut combined = vec![0.0; BINS];
            for k in 0..3 {
                for i in 0..BINS {
                    combined[i] += h[k][i];
                }
            }
            let c = cdf(&combined);
            map_ca(&mut b.img, |col, a| (lerp3(col, col.map(|v| look(&c, v)), amt), a));
        }
        _ => {
            let cs = [cdf(&h[0]), cdf(&h[1]), cdf(&h[2])];
            map_ca(&mut b.img, |col, a| (lerp3(col, [0, 1, 2].map(|k| look(&cs[k], col[k])), amt), a));
        }
    }
    b
}

// ---------------------------------------------------------------------------------------------
// Shadow/Highlight

fn shadow_highlight(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let (mut s_amt, mut h_amt) = (ctx.params.f("shadowAmount") as f32 / 100.0, ctx.params.f("highlightAmount") as f32 / 100.0);
    let luma = Plane::luma(&b.img);
    if ctx.params.b("autoAmounts") {
        let (mut sum, mut wsum) = (0.0f64, 0.0f64);
        for (px, l) in b.img.data.iter().zip(&luma.data) {
            sum += (*l * px[3]) as f64;
            wsum += px[3] as f64;
        }
        let mean = if wsum > 0.0 { (sum / wsum) as f32 } else { 0.5 };
        s_amt = ((0.6 - mean) * 1.5).clamp(0.0, 1.0) * 0.8 + 0.1;
        h_amt = ((mean - 0.6) * 1.5).clamp(0.0, 1.0) * 0.8;
    }
    let s_tw = (ctx.params.f("shadowTonalWidth") as f32 / 100.0).max(0.01);
    let h_tw = (ctx.params.f("highlightTonalWidth") as f32 / 100.0).max(0.01);
    let s_r = ctx.params.f("shadowRadius").max(0.0) * b.scale / 2.0;
    let h_r = ctx.params.f("highlightRadius").max(0.0) * b.scale / 2.0;
    let cc = ctx.params.f("colorCorrection") as f32 / 100.0;
    let mc = ctx.params.f("midtoneContrast") as f32 / 100.0;
    let blend = (ctx.params.f("blend") / 100.0).clamp(0.0, 1.0) as f32;
    let base_s = gauss_plane(&luma, s_r, s_r);
    let base_h = if (h_r - s_r).abs() < 1e-9 { base_s.clone() } else { gauss_plane(&luma, h_r, h_r) };
    b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let (c, a) = unpremul(*px);
        if a <= 0.0 {
            return;
        }
        let ws = (1.0 - base_s.data[i] / s_tw).clamp(0.0, 1.0).powi(2);
        let wh = ((base_h.data[i] - (1.0 - h_tw)) / h_tw).clamp(0.0, 1.0).powi(2);
        let gain = (1.0 + s_amt * ws * 2.0) * (1.0 - h_amt * wh * 0.5);
        let l = luminance(c[0], c[1], c[2]);
        let nl = l * gain;
        let sat = gain.max(1e-3).powf(cc);
        let mut o = c.map(|v| nl + (v - l) * sat);
        if mc != 0.0 {
            let shift = (0.5 + (nl - 0.5) * (1.0 + mc)) - nl;
            o = o.map(|v| v + shift);
        }
        let o = lerp3(o.map(|v| v.max(0.0)), c, blend);
        *px = premul(o, a);
    });
    b
}

// ---------------------------------------------------------------------------------------------
// Selective Color

const SC_IDS: [[&str; 4]; 9] = [
    ["redsCyan", "redsMagenta", "redsYellow", "redsBlack"],
    ["yellowsCyan", "yellowsMagenta", "yellowsYellow", "yellowsBlack"],
    ["greensCyan", "greensMagenta", "greensYellow", "greensBlack"],
    ["cyansCyan", "cyansMagenta", "cyansYellow", "cyansBlack"],
    ["bluesCyan", "bluesMagenta", "bluesYellow", "bluesBlack"],
    ["magentasCyan", "magentasMagenta", "magentasYellow", "magentasBlack"],
    ["whitesCyan", "whitesMagenta", "whitesYellow", "whitesBlack"],
    ["neutralsCyan", "neutralsMagenta", "neutralsYellow", "neutralsBlack"],
    ["blacksCyan", "blacksMagenta", "blacksYellow", "blacksBlack"],
];
const SC_NAMES: [[&str; 4]; 9] = [
    ["Reds Cyan", "Reds Magenta", "Reds Yellow", "Reds Black"],
    ["Yellows Cyan", "Yellows Magenta", "Yellows Yellow", "Yellows Black"],
    ["Greens Cyan", "Greens Magenta", "Greens Yellow", "Greens Black"],
    ["Cyans Cyan", "Cyans Magenta", "Cyans Yellow", "Cyans Black"],
    ["Blues Cyan", "Blues Magenta", "Blues Yellow", "Blues Black"],
    ["Magentas Cyan", "Magentas Magenta", "Magentas Yellow", "Magentas Black"],
    ["Whites Cyan", "Whites Magenta", "Whites Yellow", "Whites Black"],
    ["Neutrals Cyan", "Neutrals Magenta", "Neutrals Yellow", "Neutrals Black"],
    ["Blacks Cyan", "Blacks Magenta", "Blacks Yellow", "Blacks Black"],
];

/// How much a colour belongs to each Selective Color range.
fn sc_weights(c: [f32; 3]) -> [f32; 9] {
    let c = c.map(|v| v.clamp(0.0, 1.0));
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|&a, &b| c[b].total_cmp(&c[a]));
    let (mx, md, mn) = (c[idx[0]], c[idx[1]], c[idx[2]]);
    let mut w = [0.0f32; 9];
    // Primaries: weight = max − mid when that primary is the largest component.
    w[[0, 2, 4][idx[0]]] = mx - md;
    // Secondaries: weight = mid − min when the complementary primary is the smallest.
    w[[3, 5, 1][idx[2]]] = md - mn;
    w[6] = ((mn - 0.5) * 2.0).max(0.0);
    w[8] = ((0.5 - mx) * 2.0).max(0.0);
    w[7] = (1.0 - ((mx - 0.5).abs() + (mn - 0.5).abs())).clamp(0.0, 1.0);
    w
}

fn selective_color(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let adj = SC_IDS.map(|ids| ids.map(|id| ctx.params.f(id) as f32 / 100.0));
    if adj.iter().all(|r| r.iter().all(|v| *v == 0.0)) {
        return b;
    }
    let relative = ctx.params.e("method") == 0;
    map_ca(&mut b.img, |c, a| {
        let w = sc_weights(c);
        let mut o = c;
        for (r, wr) in w.iter().enumerate() {
            if *wr <= 0.0 {
                continue;
            }
            let k = adj[r][3];
            for i in 0..3 {
                let ink = 1.0 - c[i];
                let delta = adj[r][i] + k;
                let delta = if relative { delta * ink.max(0.0) } else { delta };
                o[i] -= wr * delta;
            }
        }
        (o.map(|v| v.max(0.0)), a)
    });
    b
}

// ---------------------------------------------------------------------------------------------
// Color Balance (HLS), Color Link, Broadcast Colors

fn color_balance_hls(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let dh = (ctx.params.f("hue") / 360.0) as f32;
    let dl = (ctx.params.f("lightness") / 100.0) as f32;
    let ds = (ctx.params.f("saturation") / 100.0) as f32;
    if dh == 0.0 && dl == 0.0 && ds == 0.0 {
        return b;
    }
    b.img.map_straight(|c| {
        let (h, s, l) = rgb_to_hsl(c[0], c[1], c[2]);
        let l = if dl > 0.0 { l + (1.0 - l) * dl } else { l + l * dl };
        let s = if ds > 0.0 { s + (1.0 - s) * ds } else { s + s * ds };
        let (r, g, bb) = hsl_to_rgb(h + dh, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        [r, g, bb]
    });
    b
}

const LINK_MODES: [(&str, BlendMode); 13] = [
    ("Normal", BlendMode::Normal),
    ("Add", BlendMode::Add),
    ("Multiply", BlendMode::Multiply),
    ("Screen", BlendMode::Screen),
    ("Overlay", BlendMode::Overlay),
    ("Soft Light", BlendMode::SoftLight),
    ("Hard Light", BlendMode::HardLight),
    ("Darken", BlendMode::Darken),
    ("Lighten", BlendMode::Lighten),
    ("Difference", BlendMode::Difference),
    ("Hue", BlendMode::Hue),
    ("Color", BlendMode::Color),
    ("Luminosity", BlendMode::Luminosity),
];

/// Representative colour of an image: 0 average, 1 brightest, 2 darkest, 3 max RGB, 4 min RGB.
fn sample_color(img: &Image, how: u32) -> [f32; 3] {
    let mut sum = [0.0f64; 4];
    let mut best: Option<([f32; 3], f32)> = None;
    let mut mx = [0.0f32; 3];
    let mut mn = [f32::INFINITY; 3];
    for px in &img.data {
        let (c, a) = unpremul(*px);
        if a <= 1e-3 {
            continue;
        }
        for k in 0..3 {
            sum[k] += (c[k] * a) as f64;
            mx[k] = mx[k].max(c[k]);
            mn[k] = mn[k].min(c[k]);
        }
        sum[3] += a as f64;
        let l = luminance(c[0], c[1], c[2]);
        let better = match best {
            None => true,
            Some((_, bl)) => (how == 1 && l > bl) || (how == 2 && l < bl),
        };
        if better {
            best = Some((c, l));
        }
    }
    if sum[3] <= 0.0 {
        return [0.0; 3];
    }
    match how {
        1 | 2 => best.map(|b| b.0).unwrap_or([0.0; 3]),
        3 => mx,
        4 => mn,
        _ => [0, 1, 2].map(|k| (sum[k] / sum[3]) as f32),
    }
}

fn color_link(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = sample_color(&b.img, ctx.params.e("sampleSource"));
    let op = (ctx.params.f("opacity") / 100.0).clamp(0.0, 1.0) as f32;
    let mode = LINK_MODES[(ctx.params.e("blendingMode") as usize).min(LINK_MODES.len() - 1)].1;
    let stencil = ctx.params.b("stencilOriginalAlpha");
    let src = [c[0] * op, c[1] * op, c[2] * op, op];
    b.img.data.par_iter_mut().for_each(|px| {
        let a0 = px[3];
        let mut o = blend_pixel(mode, *px, src, 0.5);
        o[3] = o[3].clamp(0.0, 1.0);
        if stencil {
            let (sc, _) = unpremul(o);
            o = premul(sc, a0);
        }
        *px = o;
    });
    b
}

fn broadcast_colors(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pal = ctx.params.e("locale") == 1;
    let how = ctx.params.e("howToMakeSafe");
    let limit = ctx.params.f("maxSignal") as f32;
    let (setup, span) = if pal { (0.0, 100.0) } else { (7.5, 92.5) };
    let chroma = |c: [f32; 3]| -> (f32, f32) {
        let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
        let ch = if pal {
            let (u, v) = (0.492 * (c[2] - y), 0.877 * (c[0] - y));
            (u * u + v * v).sqrt()
        } else {
            let i = 0.596 * c[0] - 0.274 * c[1] - 0.322 * c[2];
            let q = 0.211 * c[0] - 0.523 * c[1] + 0.312 * c[2];
            (i * i + q * q).sqrt()
        };
        (y, ch)
    };
    let max_norm = (limit - setup) / span;
    b.img.data.par_iter_mut().for_each(|px| {
        let (c, a) = unpremul(*px);
        if a <= 0.0 {
            return;
        }
        let (y, ch) = chroma(c);
        let unsafe_ = y + ch > max_norm + 1e-5;
        match how {
            2 => {
                if unsafe_ {
                    *px = [0.0; 4];
                }
            }
            3 => {
                if !unsafe_ {
                    *px = [0.0; 4];
                }
            }
            1 => {
                if unsafe_ {
                    let target = (max_norm - y).max(0.0);
                    let s = if ch > 1e-6 { target / ch } else { 1.0 };
                    let mut o = c.map(|v| y + (v - y) * s);
                    if y > max_norm {
                        o = o.map(|v| v * max_norm / y);
                    }
                    *px = premul(o, a);
                }
            }
            _ => {
                if unsafe_ {
                    let k = max_norm / (y + ch);
                    *px = premul(c.map(|v| v * k), a);
                }
            }
        }
    });
    b
}

// ---------------------------------------------------------------------------------------------
// CC Toner, CC Color Offset, CC Kernel

fn cc_toner(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let g = |id: &str| {
        let c = ctx.params.color(id);
        [c[0], c[1], c[2]]
    };
    let stops: Vec<[f32; 3]> = match ctx.params.e("tones") {
        0 => vec![g("shadows"), g("highlights")],
        2 => vec![g("shadows"), g("darktones"), g("midtones"), g("brights"), g("highlights")],
        _ => vec![g("shadows"), g("midtones"), g("highlights")],
    };
    let blend = (ctx.params.f("blend") / 100.0).clamp(0.0, 1.0) as f32;
    let n = stops.len() - 1;
    b.img.map_straight(|c| {
        let t = luminance(c[0], c[1], c[2]).clamp(0.0, 1.0) * n as f32;
        let k = (t.floor() as usize).min(n - 1);
        let tone = lerp3(stops[k], stops[k + 1], t - k as f32);
        lerp3(tone, c, blend)
    });
    b
}

fn cc_color_offset(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let offs = ["redPhase", "greenPhase", "bluePhase"].map(|id| (ctx.params.f(id) / 360.0).rem_euclid(1.0) as f32);
    if offs.iter().all(|o| *o == 0.0) {
        return b;
    }
    let overflow = ctx.params.e("overflow");
    b.img.map_straight(|c| {
        [0, 1, 2].map(|k| {
            if offs[k] == 0.0 {
                return c[k];
            }
            let x = c[k] + offs[k];
            if x <= 1.0 {
                return x;
            }
            match overflow {
                1 => (2.0 - x).max(0.0),
                2 => {
                    if x - 1.0 < 0.5 {
                        0.0
                    } else {
                        1.0
                    }
                }
                _ => x.rem_euclid(1.0),
            }
        })
    });
    b
}

const K_IDS: [&str; 9] = ["k1", "k2", "k3", "k4", "k5", "k6", "k7", "k8", "k9"];
const K_NAMES: [&str; 9] = ["Top Left", "Top", "Top Right", "Left", "Center", "Right", "Bottom Left", "Bottom", "Bottom Right"];

fn cc_kernel(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let k = K_IDS.map(|id| ctx.params.f(id) as f32);
    let scale = ctx.params.f("scale") as f32;
    let offset = ctx.params.f("offset") as f32;
    let abs = ctx.params.b("absolute");
    let identity = k.iter().enumerate().all(|(i, v)| if i == 4 { *v == 1.0 } else { *v == 0.0 });
    if identity && scale == 1.0 && offset == 0.0 {
        return b;
    }
    let src = b.img.clone();
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let a = px[3];
            if a <= 0.0 {
                continue;
            }
            let mut acc = [0.0f32; 3];
            for j in 0..3 {
                for i in 0..3 {
                    let w = k[j * 3 + i];
                    if w == 0.0 {
                        continue;
                    }
                    let q = src.get_clamped(x as i64 + i as i64 - 1, y as i64 + j as i64 - 1);
                    let (c, _) = unpremul(q);
                    for ch in 0..3 {
                        acc[ch] += c[ch] * w;
                    }
                }
            }
            let o = acc.map(|v| {
                let v = v * scale + offset;
                (if abs { v.abs() } else { v }).max(0.0)
            });
            *px = premul(o, a);
        }
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let mut lv_params = Vec::new();
    for (ids, names) in LV_IDS.iter().zip(LV_NAMES.iter()) {
        lv_params.push(p(ids[0], names[0], num(0.0), slider(-1.0, 2.0, 0.0, 1.0, 3)));
        lv_params.push(p(ids[1], names[1], num(1.0), slider(-1.0, 2.0, 0.0, 1.0, 3)));
        lv_params.push(p(ids[2], names[2], num(1.0), slider(0.1, 10.0, 0.1, 3.0, 2)));
        lv_params.push(p(ids[3], names[3], num(0.0), slider(-1.0, 2.0, 0.0, 1.0, 3)));
        lv_params.push(p(ids[4], names[4], num(1.0), slider(-1.0, 2.0, 0.0, 1.0, 3)));
    }
    let mut sc_params = vec![p("method", "Method", Value::Enum(0), popup(&["Relative", "Absolute"]))];
    for (ids, names) in SC_IDS.iter().zip(SC_NAMES.iter()) {
        for k in 0..4 {
            sc_params.push(p(ids[k], names[k], num(0.0), pct()));
        }
    }
    let clip_params = |extra: bool| {
        let mut v = vec![
            p("blackClip", "Black Clip", num(0.1), slider(0.0, 49.0, 0.0, 10.0, 2)),
            p("whiteClip", "White Clip", num(0.1), slider(0.0, 49.0, 0.0, 10.0, 2)),
            p("blend", "Blend With Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
        ];
        if extra {
            v.insert(0, p("snapNeutralMidtones", "Snap Neutral Midtones", Value::Bool(true), ParamUi::Checkbox));
        }
        v
    };
    let mut k_params: Vec<crate::ParamSpec> =
        K_IDS.iter().zip(K_NAMES.iter()).map(|(id, n)| p(id, n, num(if *id == "k5" { 1.0 } else { 0.0 }), slider(-100.0, 100.0, -10.0, 10.0, 2))).collect();
    k_params.push(p("scale", "Scale", num(1.0), slider(-100.0, 100.0, 0.0, 4.0, 3)));
    k_params.push(p("offset", "Offset", num(0.0), slider(-1.0, 1.0, -1.0, 1.0, 3)));
    k_params.push(p("absolute", "Absolute Value", Value::Bool(false), ParamUi::Checkbox));
    let curve = |id: &'static str, name: &'static str| p(id, name, Value::Str("0,0 1,1".into()), ParamUi::Hidden);
    vec![
        spec(
            "ec.color.curves",
            "Curves",
            vec![curve("rgb", "RGB"), curve("red", "Red"), curve("green", "Green"), curve("blue", "Blue"), curve("alpha", "Alpha")],
            curves,
        ),
        spec("ec.color.levelsic", "Levels (Individual Controls)", lv_params, levels_ic),
        spec("ec.color.autolevels", "Auto Levels", clip_params(false), auto_levels),
        spec("ec.color.autocontrast", "Auto Contrast", clip_params(false), auto_contrast),
        spec("ec.color.autocolor", "Auto Color", clip_params(true), auto_color),
        spec(
            "ec.color.equalize",
            "Equalize",
            vec![
                p("style", "Equalize", Value::Enum(0), popup(&["RGB", "Brightness", "Photoshop Style"])),
                p("amount", "Amount to Equalize", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            equalize,
        ),
        spec(
            "ec.color.shadowhighlight",
            "Shadow/Highlight",
            vec![
                p("autoAmounts", "Auto Amounts", Value::Bool(true), ParamUi::Checkbox),
                p("shadowAmount", "Shadow Amount", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("highlightAmount", "Highlight Amount", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("shadowTonalWidth", "Shadow Tonal Width", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("shadowRadius", "Shadow Radius", num(30.0), slider(0.0, 2500.0, 0.0, 100.0, 0)),
                p("highlightTonalWidth", "Highlight Tonal Width", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 0)),
                p("highlightRadius", "Highlight Radius", num(30.0), slider(0.0, 2500.0, 0.0, 100.0, 0)),
                p("colorCorrection", "Color Correction", num(20.0), slider(-100.0, 100.0, -100.0, 100.0, 0)),
                p("midtoneContrast", "Midtone Contrast", num(0.0), pct()),
                p("blend", "Blend With Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            shadow_highlight,
        ),
        spec("ec.color.selectivecolor", "Selective Color", sc_params, selective_color),
        spec(
            "ec.color.colorbalancehls",
            "Color Balance (HLS)",
            vec![p("hue", "Hue", num(0.0), ParamUi::Angle), p("lightness", "Lightness", num(0.0), pct()), p("saturation", "Saturation", num(0.0), pct())],
            color_balance_hls,
        ),
        spec(
            "ec.color.colorlink",
            "Color Link",
            vec![
                p("sampleSource", "Sample", Value::Enum(0), popup(&["Average", "Brightest", "Darkest", "Max RGB", "Min RGB"])),
                p("stencilOriginalAlpha", "Stencil Original Alpha", Value::Bool(true), ParamUi::Checkbox),
                p("opacity", "Opacity", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("blendingMode", "Blending Mode", Value::Enum(0), popup(&LINK_MODES.map(|m| m.0))),
            ],
            color_link,
        ),
        spec(
            "ec.color.broadcast",
            "Broadcast Colors",
            vec![
                p("locale", "Broadcast Locale", Value::Enum(0), popup(&["NTSC", "PAL"])),
                p(
                    "howToMakeSafe",
                    "How to Make Color Safe",
                    Value::Enum(0),
                    popup(&["Reduce Luminance", "Reduce Saturation", "Key Out Unsafe", "Key Out Safe"]),
                ),
                p("maxSignal", "Maximum Signal Amplitude (IRE)", num(110.0), slider(90.0, 120.0, 90.0, 120.0, 0)),
            ],
            broadcast_colors,
        ),
        spec(
            "ec.color.cctoner",
            "CC Toner",
            vec![
                p("tones", "Tones", Value::Enum(1), popup(&["Duotone", "Tritone", "Pentone"])),
                p("highlights", "Highlights", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("brights", "Brights", col(0.86, 0.78, 0.64), ParamUi::Color),
                p("midtones", "Midtones", col(0.55, 0.43, 0.28), ParamUi::Color),
                p("darktones", "Darktones", col(0.26, 0.18, 0.1), ParamUi::Color),
                p("shadows", "Shadows", col(0.0, 0.0, 0.0), ParamUi::Color),
                p("blend", "Blend w. Original", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            cc_toner,
        ),
        spec(
            "ec.color.cccoloroffset",
            "CC Color Offset",
            vec![
                p("redPhase", "Red Phase", num(0.0), ParamUi::Angle),
                p("greenPhase", "Green Phase", num(0.0), ParamUi::Angle),
                p("bluePhase", "Blue Phase", num(0.0), ParamUi::Angle),
                p("overflow", "Overflow", Value::Enum(0), popup(&["Wrap", "Solarize", "Polarize"])),
            ],
            cc_color_offset,
        ),
        spec("ec.color.cckernel", "CC Kernel", k_params, cc_kernel),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Params, find};

    fn run(id: &str, img: &Image, over: &[(&str, Value)]) -> Image {
        let s = find(id).unwrap();
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        for (k, v) in over {
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx =
            EffectCtx { params: &params, time: 0.0, layer_size: [img.width as f64, img.height as f64], seed: 1, adjustment: false, env: Default::default() };
        (s.render)(&ctx, Buf { img: img.clone(), offset: [0.0, 0.0], scale: 1.0 }).img
    }

    fn ramp() -> Image {
        let mut img = Image::new(16, 8);
        for y in 0..8 {
            for x in 0..16 {
                let v = x as f32 / 15.0;
                let a = if y == 0 { 0.5 } else { 1.0 };
                img.set(x, y, [v * a, (1.0 - v) * a, 0.3 * a, a]);
            }
        }
        img
    }

    fn close(a: &Image, b: &Image, eps: f32) -> bool {
        a.data.iter().zip(&b.data).all(|(p, q)| (0..4).all(|i| (p[i] - q[i]).abs() <= eps))
    }

    #[test]
    fn curve_parsing() {
        assert!(Curve::parse("0,0 1,1").is_none());
        assert!(Curve::parse("garbage").is_none());
        let c = Curve::parse("1,1; 0,0 0.5,0.7").unwrap();
        assert_eq!(c.xs, vec![0.0, 0.5, 1.0]);
        assert!((c.eval(0.5) - 0.7).abs() < 1e-3);
        // Monotone: never decreases.
        let mut last = -1.0;
        for i in 0..=100 {
            let v = c.eval(i as f32 / 100.0);
            assert!(v >= last - 1e-6);
            last = v;
        }
    }

    #[test]
    fn curves_identity_is_identity() {
        let img = ramp();
        assert_eq!(run("ec.color.curves", &img, &[]), img);
        let explicit = run("ec.color.curves", &img, &[("rgb", Value::Str("0,0 0.25,0.25 1,1".into()))]);
        assert!(close(&explicit, &img, 1e-4));
    }

    #[test]
    fn curves_midpoint_brightens() {
        let img = Image::filled(4, 4, [0.5, 0.5, 0.5, 1.0]);
        let out = run("ec.color.curves", &img, &[("rgb", Value::Str("0,0 0.5,0.75 1,1".into()))]);
        assert!((out.data[0][0] - 0.75).abs() < 1e-3);
        let red = run("ec.color.curves", &img, &[("red", Value::Str("0,1 1,1".into()))]);
        assert!((red.data[0][0] - 1.0).abs() < 1e-4 && (red.data[0][1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn levels_ic_identity_and_red_only() {
        let img = ramp();
        assert_eq!(run("ec.color.levelsic", &img, &[]), img);
        let out = run("ec.color.levelsic", &img, &[("redOutWhite", num(0.5))]);
        let p = out.get(15, 3);
        assert!((p[0] - 0.5).abs() < 1e-4 && (p[1] - 0.0).abs() < 1e-4);
    }

    #[test]
    fn auto_levels_stretches_low_contrast() {
        let mut img = Image::new(32, 4);
        for y in 0..4 {
            for x in 0..32 {
                let v = 0.4 + 0.2 * x as f32 / 31.0;
                img.set(x, y, [v, v, v, 1.0]);
            }
        }
        let out = run("ec.color.autolevels", &img, &[("blackClip", num(0.0)), ("whiteClip", num(0.0))]);
        let lo = out.data.iter().map(|p| p[0]).fold(1.0, f32::min);
        let hi = out.data.iter().map(|p| p[0]).fold(0.0, f32::max);
        assert!(lo < 0.02 && hi > 0.98, "{lo} {hi}");
        let ac = run("ec.color.autocontrast", &img, &[("blackClip", num(0.0)), ("whiteClip", num(0.0))]);
        assert!(ac.data.iter().map(|p| p[0]).fold(0.0, f32::max) > 0.98);
    }

    #[test]
    fn auto_color_neutralises_cast() {
        let mut img = Image::new(16, 4);
        for y in 0..4 {
            for x in 0..16 {
                let v = x as f32 / 15.0;
                img.set(x, y, [v * 0.6 + 0.1, v * 0.9, v * 0.5, 1.0]);
            }
        }
        let out = run("ec.color.autocolor", &img, &[]);
        let p = out.get(8, 1);
        assert!((p[0] - p[1]).abs() < 0.08 && (p[1] - p[2]).abs() < 0.08, "{p:?}");
    }

    #[test]
    fn equalize_flattens_histogram() {
        let mut img = Image::new(64, 1);
        for x in 0..64 {
            let v = if x < 48 { 0.1 + x as f32 * 0.001 } else { 0.9 };
            img.set(x, 0, [v, v, v, 1.0]);
        }
        let out = run("ec.color.equalize", &img, &[]);
        // The crowded dark values are spread across the range.
        assert!(out.get(47, 0)[0] > 0.6, "{}", out.get(47, 0)[0]);
        assert!(out.get(0, 0)[0] < 0.1);
        assert_eq!(run("ec.color.equalize", &img, &[("amount", num(0.0))]), img);
    }

    #[test]
    fn selective_color_zero_is_identity_and_reds_respond() {
        let img = ramp();
        assert_eq!(run("ec.color.selectivecolor", &img, &[]), img);
        let red = Image::filled(2, 2, [1.0, 0.1, 0.1, 1.0]);
        let out = run("ec.color.selectivecolor", &red, &[("redsCyan", num(100.0)), ("method", Value::Enum(1))]);
        assert!(out.data[0][0] < 0.3, "{:?}", out.data[0]);
        let blue = Image::filled(2, 2, [0.1, 0.1, 1.0, 1.0]);
        assert_eq!(run("ec.color.selectivecolor", &blue, &[("redsCyan", num(100.0))]), blue);
    }

    #[test]
    fn color_offset_full_turn_is_identity() {
        let img = ramp();
        let out = run("ec.color.cccoloroffset", &img, &[("redPhase", num(360.0)), ("greenPhase", num(720.0))]);
        assert_eq!(out, img);
        let out = run("ec.color.cccoloroffset", &img, &[("redPhase", num(180.0))]);
        assert!((out.get(0, 3)[0] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn broadcast_reduces_unsafe_color() {
        let img = Image::filled(2, 2, [1.0, 1.0, 0.0, 1.0]);
        let out = run("ec.color.broadcast", &img, &[]);
        assert!(out.data[0][0] < 1.0);
        let key = run("ec.color.broadcast", &img, &[("howToMakeSafe", Value::Enum(2))]);
        assert_eq!(key.data[0][3], 0.0);
        let gray = Image::filled(2, 2, [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(run("ec.color.broadcast", &gray, &[]), gray);
    }

    #[test]
    fn color_balance_hls_zero_identity_and_hue_shift() {
        let img = ramp();
        assert_eq!(run("ec.color.colorbalancehls", &img, &[]), img);
        let red = Image::filled(1, 1, [1.0, 0.0, 0.0, 1.0]);
        let out = run("ec.color.colorbalancehls", &red, &[("hue", num(120.0))]);
        assert!(out.data[0][1] > 0.99 && out.data[0][0] < 0.01);
    }

    #[test]
    fn color_link_fills_with_average_keeping_alpha() {
        let mut img = Image::new(2, 1);
        img.set(0, 0, [1.0, 0.0, 0.0, 1.0]);
        img.set(1, 0, [0.0, 0.0, 0.5, 0.5]);
        let out = run("ec.color.colorlink", &img, &[]);
        assert!((out.data[1][3] - 0.5).abs() < 1e-5);
        let (c, _) = unpremul(out.data[0]);
        assert!((c[0] - 2.0 / 3.0).abs() < 1e-3 && (c[2] - 1.0 / 3.0).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn cc_kernel_identity_and_toner_duotone() {
        let img = ramp();
        assert_eq!(run("ec.color.cckernel", &img, &[]), img);
        let g = Image::filled(1, 1, [0.5, 0.5, 0.5, 1.0]);
        let t = run("ec.color.cctoner", &g, &[("tones", Value::Enum(0)), ("highlights", col(1.0, 0.0, 0.0))]);
        assert!((t.data[0][0] - 0.5).abs() < 1e-3 && t.data[0][1] < 1e-3);
    }

    #[test]
    fn shadow_highlight_lifts_shadows() {
        let img = Image::filled(8, 8, [0.1, 0.1, 0.1, 1.0]);
        let out = run("ec.color.shadowhighlight", &img, &[("autoAmounts", Value::Bool(false))]);
        assert!(out.data[0][0] > 0.15);
    }
}
