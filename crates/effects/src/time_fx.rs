//! Time effects (Effect > Time): Echo, Posterize Time, Time Difference, Time Displacement,
//! Timewarp, CC Force Motion Blur, CC Wide Time and Pixel Motion Blur.
//!
//! These read the layer at *other* layer times through [`EffectHost::self_at`] with zero
//! preceding effects: like After Effects' Time effects they see the layer's source with its
//! masks and ignore effects applied before them (precompose to include those). Without a host
//! (tests, thumbnails) they pass their input through.
//!
//! Pixel-motion based methods (Timewarp's Pixel Motion, Pixel Motion Blur) are approximated by
//! frame mixing / sub-frame sampling; there is no optical-flow estimation yet.
//!
//! [`EffectHost::self_at`]: crate::EffectHost::self_at

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::{Image, Px};
use rayon::prelude::*;

use crate::util::{fit_layer, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Time", params, render, gpu: false, float: true }
}

pub fn specs() -> Vec<EffectSpec> {
    vec![
        spec(
            "ec.time.echo",
            "Echo",
            vec![
                p("echoTime", "Echo Time (seconds)", num(-1.0 / 30.0), slider(-30.0, 30.0, -5.0, 5.0, 3)),
                p("numberOfEchoes", "Number Of Echoes", num(1.0), slider(0.0, 255.0, 0.0, 30.0, 0)),
                p("startingIntensity", "Starting Intensity", num(1.0), slider(0.0, 1.0, 0.0, 1.0, 2)),
                p("decay", "Decay", num(1.0), slider(0.0, 1.0, 0.0, 1.0, 2)),
                p(
                    "echoOperator",
                    "Echo Operator",
                    Value::Enum(0),
                    popup(&["Add", "Maximum", "Minimum", "Screen", "Composite In Back", "Composite In Front", "Blend"]),
                ),
            ],
            echo,
        ),
        spec("ec.time.posterizetime", "Posterize Time", vec![p("frameRate", "Frame Rate", num(12.0), slider(0.1, 99.0, 1.0, 60.0, 1))], posterize_time),
        spec(
            "ec.time.timedifference",
            "Time Difference",
            vec![
                p("targetLayer", "Target", Value::Layer(None), ParamUi::Layer),
                p("timeOffset", "Time Offset (sec)", num(0.0), slider(-30.0, 30.0, -5.0, 5.0, 3)),
                p("contrast", "Contrast", num(0.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("absoluteDifference", "Absolute Difference", Value::Bool(true), ParamUi::Checkbox),
                p(
                    "alphaChannel",
                    "Alpha Channel",
                    Value::Enum(0),
                    popup(&[
                        "Original",
                        "Target",
                        "Blend",
                        "Max",
                        "Full On",
                        "Lightness of Result",
                        "Max of Result",
                        "Alpha Difference",
                        "Alpha Difference Only",
                    ]),
                ),
            ],
            time_difference,
        ),
        spec(
            "ec.time.timedisplacement",
            "Time Displacement",
            vec![
                p("displacementMapLayer", "Time Displacement Layer", Value::Layer(None), ParamUi::Layer),
                p("maxDisplacementTime", "Max Displacement Time [sec]", num(1.0), slider(-30.0, 30.0, -5.0, 5.0, 2)),
                p("timeResolution", "Time Resolution [fps]", num(60.0), slider(1.0, 999.0, 1.0, 120.0, 1)),
                p("stretchMap", "If Layer Sizes Differ: Stretch Map to Fit", Value::Bool(true), ParamUi::Checkbox),
            ],
            time_displacement,
        ),
        spec(
            "ec.time.timewarp",
            "Timewarp",
            vec![
                p("method", "Method", Value::Enum(2), popup(&["Whole Frames", "Frame Mix", "Pixel Motion"])),
                p("adjustTimeBy", "Adjust Time By", Value::Enum(0), popup(&["Speed", "Source Frame"])),
                p("speed", "Speed", num(50.0), slider(-10000.0, 10000.0, 0.0, 200.0, 1)),
                p("sourceFrame", "Source Frame", num(0.0), slider(-100000.0, 100000.0, 0.0, 300.0, 1)),
                p("vectorDetail", "Vector Detail", num(20.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("enableMotionBlur", "Enable Motion Blur", Value::Bool(false), ParamUi::Checkbox),
                p("shutterControl", "Shutter Control", Value::Enum(0), popup(&["Automatic", "Manual"])),
                p("shutterAngle", "Shutter Angle", num(180.0), slider(0.0, 720.0, 0.0, 360.0, 1)),
                p("shutterSamples", "Shutter Samples", num(5.0), slider(1.0, 64.0, 1.0, 32.0, 0)),
            ],
            timewarp,
        ),
        spec(
            "ec.time.ccforcemotionblur",
            "CC Force Motion Blur",
            vec![
                p("motionBlurLevels", "Motion Blur Levels", num(8.0), slider(2.0, 64.0, 2.0, 32.0, 0)),
                p("overrideShutterAngle", "Override Shutter Angle", Value::Bool(true), ParamUi::Checkbox),
                p("shutterAngle", "Shutter Angle", num(180.0), slider(0.0, 3600.0, 0.0, 360.0, 1)),
                p("nativeMotionBlur", "Native Motion Blur", Value::Enum(0), popup(&["Off", "On"])),
            ],
            force_motion_blur,
        ),
        spec(
            "ec.time.ccwidetime",
            "CC Wide Time",
            vec![
                p("forwardSteps", "Forward Steps", num(3.0), slider(0.0, 45.0, 0.0, 15.0, 0)),
                p("backwardSteps", "Backward Steps", num(3.0), slider(0.0, 45.0, 0.0, 15.0, 0)),
                p("nativeMotionBlur", "Native Motion Blur", Value::Enum(0), popup(&["Off", "On"])),
            ],
            wide_time,
        ),
        spec(
            "ec.time.pixelmotionblur",
            "Pixel Motion Blur",
            vec![
                p("shutterControl", "Shutter Control", Value::Enum(0), popup(&["Automatic", "Manual"])),
                p("shutterAngle", "Shutter Angle", num(180.0), slider(0.0, 720.0, 0.0, 360.0, 1)),
                p("shutterSamples", "Shutter Samples", num(16.0), slider(1.0, 64.0, 1.0, 32.0, 0)),
                p("vectorDetail", "Vector Detail", num(20.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
            ],
            pixel_motion_blur,
        ),
    ]
}

// ---------------------------------------------------------------- frame access

/// Frames of the layer at several layer times, resampled onto one common pixel grid.
struct Frames {
    /// Template buffer (geometry of the output: offset/scale/size).
    grid: Buf,
    imgs: Vec<Image>,
}

/// Fetch the layer (source + masks) at layer times `times`. `None` without a host.
fn fetch(ctx: &EffectCtx, times: &[f64]) -> Option<Frames> {
    let host = ctx.env.host?;
    // Dedupe times (sub-microsecond differences are the same frame).
    let mut uniq: Vec<i64> = times.iter().map(|t| (t * 1e6).round() as i64).collect();
    uniq.sort_unstable();
    uniq.dedup();
    let bufs: Vec<(i64, Option<Buf>)> = uniq.iter().map(|&k| (k, host.self_at(k as f64 / 1e6, 0))).collect();
    let first = bufs.iter().find_map(|(_, b)| b.as_ref())?;
    let scale = first.scale;
    // Union of every frame's extent in layer pixels (offset = where layer (0,0) sits).
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for b in bufs.iter().filter_map(|(_, b)| b.as_ref()) {
        let k = scale / b.scale.max(1e-9);
        x0 = x0.min(-b.offset[0] * k);
        y0 = y0.min(-b.offset[1] * k);
        x1 = x1.max((b.img.width as f64 - b.offset[0]) * k);
        y1 = y1.max((b.img.height as f64 - b.offset[1]) * k);
    }
    let (x0, y0) = (x0.floor(), y0.floor());
    let w = ((x1.ceil() - x0) as u32).clamp(1, 16384);
    let h = ((y1.ceil() - y0) as u32).clamp(1, 16384);
    let grid = Buf { img: Image::new(w, h), offset: [-x0, -y0], scale };
    let place = |b: &Buf| -> Image {
        if (b.scale - scale).abs() < 1e-9 && b.img.width == w && b.img.height == h && (b.offset[0] + x0).abs() < 1e-6 && (b.offset[1] + y0).abs() < 1e-6 {
            return b.img.clone();
        }
        let k = b.scale / scale;
        let same_scale = (k - 1.0).abs() < 1e-9;
        let dx = b.offset[0] + x0;
        let dy = b.offset[1] + y0;
        let mut out = Image::new(w, h);
        let ow = w as usize;
        out.data.par_chunks_mut(ow).enumerate().for_each(|(y, row)| {
            for (x, px) in row.iter_mut().enumerate() {
                *px = if same_scale && dx.fract() == 0.0 && dy.fract() == 0.0 {
                    b.img.get(x as i64 + dx as i64, y as i64 + dy as i64)
                } else {
                    let sx = (x as f64 + 0.5) * k + dx - 0.5;
                    let sy = (y as f64 + 0.5) * k + dy - 0.5;
                    b.img.sample_bilinear(sx + 0.5, sy + 0.5)
                };
            }
        });
        out
    };
    let placed: Vec<(i64, Image)> = bufs.iter().map(|(k, b)| (*k, b.as_ref().map(place).unwrap_or_else(|| Image::new(w, h)))).collect();
    let imgs = times
        .iter()
        .map(|t| {
            let k = (t * 1e6).round() as i64;
            placed.iter().find(|(q, _)| *q == k).map(|(_, i)| i.clone()).unwrap_or_else(|| Image::new(w, h))
        })
        .collect();
    Some(Frames { grid, imgs })
}

fn with_img(grid: Buf, img: Image) -> Buf {
    Buf { img, ..grid }
}

/// Weighted average of frames (weights sum to 1 for a plain average).
fn average(frames: &[Image], weights: &[f32]) -> Image {
    let w = frames[0].width;
    let h = frames[0].height;
    let mut out = Image::new(w, h);
    out.data.par_iter_mut().enumerate().for_each(|(i, o)| {
        let mut acc = [0.0f32; 4];
        for (f, k) in frames.iter().zip(weights) {
            let p = f.data[i];
            for c in 0..4 {
                acc[c] += p[c] * k;
            }
        }
        *o = acc;
    });
    out
}

fn mean_of(ctx: &EffectCtx, b: Buf, times: &[f64]) -> Buf {
    if times.is_empty() {
        return b;
    }
    let Some(fr) = fetch(ctx, times) else { return b };
    let k = 1.0 / times.len() as f32;
    let img = average(&fr.imgs, &vec![k; times.len()]);
    with_img(fr.grid, img)
}

// ---------------------------------------------------------------- Echo

fn echo(ctx: &EffectCtx, b: Buf) -> Buf {
    let n = ctx.params.f("numberOfEchoes").round().clamp(0.0, 255.0) as usize;
    let dt = ctx.params.f("echoTime");
    let start = ctx.params.f("startingIntensity").clamp(0.0, 1.0) as f32;
    let decay = ctx.params.f("decay").clamp(0.0, 1.0) as f32;
    let op = ctx.params.e("echoOperator");
    let times: Vec<f64> = (0..=n).map(|i| ctx.time + i as f64 * dt).collect();
    let Some(fr) = fetch(ctx, &times) else { return b };
    let weights: Vec<f32> = (0..=n).map(|i| start * decay.powi(i as i32)).collect();
    let count = (n + 1) as f32;
    let w = fr.grid.img.width;
    let h = fr.grid.img.height;
    let mut out = Image::new(w, h);
    let imgs = &fr.imgs;
    out.data.par_iter_mut().enumerate().for_each(|(i, o)| {
        let mut acc: Px = match op {
            2 => [f32::MAX; 4],
            _ => [0.0; 4],
        };
        for (f, &k) in imgs.iter().zip(&weights) {
            let p = f.data[i];
            let q = [p[0] * k, p[1] * k, p[2] * k, p[3] * k];
            match op {
                // Maximum / Minimum
                1 => (0..4).for_each(|c| acc[c] = acc[c].max(q[c])),
                2 => (0..4).for_each(|c| acc[c] = acc[c].min(q[c])),
                // Screen
                3 => (0..4).for_each(|c| acc[c] = acc[c] + q[c] - acc[c] * q[c]),
                // Composite In Back: each later echo goes behind what is there.
                4 => {
                    let ia = 1.0 - acc[3];
                    (0..4).for_each(|c| acc[c] += q[c] * ia);
                }
                // Composite In Front: each later echo goes on top.
                5 => {
                    let ia = 1.0 - q[3];
                    (0..4).for_each(|c| acc[c] = q[c] + acc[c] * ia);
                }
                // Blend (average) and Add
                _ => (0..4).for_each(|c| acc[c] += q[c]),
            }
        }
        if op == 6 {
            acc.iter_mut().for_each(|c| *c /= count);
        }
        if op == 2 && acc[0] == f32::MAX {
            acc = [0.0; 4];
        }
        acc[3] = acc[3].clamp(0.0, 1.0);
        for c in 0..3 {
            acc[c] = acc[c].max(0.0);
        }
        *o = acc;
    });
    with_img(fr.grid, out)
}

// ---------------------------------------------------------------- Posterize Time

/// The layer time Posterize Time holds at `t` for `rate` frames per second.
pub fn posterized_time(t: f64, rate: f64) -> f64 {
    if rate <= 0.0 {
        return t;
    }
    ((t * rate) + 1e-6).floor() / rate
}

fn posterize_time(ctx: &EffectCtx, b: Buf) -> Buf {
    let tt = posterized_time(ctx.time, ctx.params.f("frameRate"));
    if ctx.env.host.is_none() {
        return b;
    }
    match fetch(ctx, &[tt]) {
        Some(mut fr) => {
            let img = fr.imgs.pop().unwrap_or_default();
            with_img(fr.grid, img)
        }
        None => b,
    }
}

// ---------------------------------------------------------------- Time Difference

fn time_difference(ctx: &EffectCtx, b: Buf) -> Buf {
    let Some(host) = ctx.env.host else { return b };
    let off = ctx.params.f("timeOffset");
    let gain = 1.0 + ctx.params.f("contrast") as f32 / 25.0;
    let abs = ctx.params.b("absoluteDifference");
    let amode = ctx.params.e("alphaChannel");
    let target_id = ctx.params.get("targetLayer").and_then(Value::as_layer);
    let (cur, target) = match target_id.and_then(|id| host.layer_at(id, ctx.env.comp_time + off, false)) {
        Some(other) => {
            let Some(mut fr) = fetch(ctx, &[ctx.time]) else { return b };
            let cur = fr.imgs.pop().unwrap_or_default();
            let tgt = fit_layer(ctx, &with_img(fr.grid.clone(), cur.clone()), &other, true);
            (with_img(fr.grid, cur), tgt)
        }
        None => {
            let Some(mut fr) = fetch(ctx, &[ctx.time, ctx.time + off]) else { return b };
            let tgt = fr.imgs.pop().unwrap_or_default();
            let cur = fr.imgs.pop().unwrap_or_default();
            (with_img(fr.grid, cur), tgt)
        }
    };
    let mut out = cur.img.clone();
    out.data.par_iter_mut().zip(target.data.par_iter()).for_each(|(o, t)| {
        let (c, ca) = unpremul(*o);
        let (d, da) = unpremul(*t);
        let mut rgb = [0.0f32; 3];
        for i in 0..3 {
            let diff = c[i] - d[i];
            rgb[i] = if abs { (diff.abs() * gain).clamp(0.0, 1.0) } else { (0.5 + diff * 0.5 * gain).clamp(0.0, 1.0) };
        }
        let light = (rgb[0].max(rgb[1]).max(rgb[2]) + rgb[0].min(rgb[1]).min(rgb[2])) * 0.5;
        let a = match amode {
            1 => da,
            2 => (ca + da) * 0.5,
            3 => ca.max(da),
            4 => 1.0,
            5 => light,
            6 => rgb[0].max(rgb[1]).max(rgb[2]),
            7 => (ca - da).abs() * gain,
            8 => {
                let a = ((ca - da).abs() * gain).clamp(0.0, 1.0);
                rgb = [1.0; 3];
                a
            }
            _ => ca,
        }
        .clamp(0.0, 1.0);
        *o = [rgb[0] * a, rgb[1] * a, rgb[2] * a, a];
    });
    Buf { img: out, ..cur }
}

// ---------------------------------------------------------------- Time Displacement

fn time_displacement(ctx: &EffectCtx, b: Buf) -> Buf {
    if ctx.env.host.is_none() {
        return b;
    }
    let max = ctx.params.f("maxDisplacementTime");
    let res = ctx.params.f("timeResolution").clamp(1.0, 999.0);
    let stretch = ctx.params.b("stretchMap");
    let Some(mut cur) = fetch(ctx, &[ctx.time]) else { return b };
    let cur_img = cur.imgs.pop().unwrap_or_default();
    let grid = with_img(cur.grid, cur_img);
    let map = match ctx.layer_param("displacementMapLayer", true) {
        Some(o) => fit_layer(ctx, &grid, &o, stretch),
        None => grid.img.clone(),
    };
    // Per pixel: luminance 0..1 → offset −max..+max, quantised to the time resolution. Keep at
    // most 64 distinct sample times (coarser steps for huge ranges).
    let span = 2.0 * max.abs() * res;
    let step = if span > 63.0 { 2.0 * max.abs() / 63.0 } else { 1.0 / res };
    let idx: Vec<i32> = map
        .data
        .par_iter()
        .map(|p| {
            let (c, a) = unpremul(*p);
            let l = if a > 0.0 { effectcraft_color::luminance(c[0], c[1], c[2]) } else { 0.5 };
            let off = (l as f64 - 0.5) * 2.0 * max;
            (off / step).round() as i32
        })
        .collect();
    let mut keys: Vec<i32> = idx.clone();
    keys.sort_unstable();
    keys.dedup();
    if keys == [0] {
        return grid;
    }
    let times: Vec<f64> = keys.iter().map(|k| ctx.time + *k as f64 * step).collect();
    let Some(fr) = fetch(ctx, &times) else { return grid };
    // Frames come back on the union grid; map the current grid into it.
    let dx = (fr.grid.offset[0] - grid.offset[0]).round() as i64;
    let dy = (fr.grid.offset[1] - grid.offset[1]).round() as i64;
    let w = grid.img.width as usize;
    let mut out = grid.img.clone();
    out.data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let k = idx[y * w + x];
            let fi = keys.binary_search(&k).unwrap_or(0);
            *o = fr.imgs[fi].get(x as i64 + dx, y as i64 + dy);
        }
    });
    with_img(grid, out)
}

// ---------------------------------------------------------------- Timewarp

/// Source layer time Timewarp shows at layer time `t`.
pub fn timewarp_source_time(t: f64, by_frame: bool, speed: f64, source_frame: f64, fps: f64) -> f64 {
    if by_frame { source_frame / fps } else { t * speed / 100.0 }
}

fn timewarp(ctx: &EffectCtx, b: Buf) -> Buf {
    if ctx.env.host.is_none() {
        return b;
    }
    let fps = ctx.fps();
    let by_frame = ctx.params.e("adjustTimeBy") == 1;
    let speed = ctx.params.f("speed");
    let src = timewarp_source_time(ctx.time, by_frame, speed, ctx.params.f("sourceFrame"), fps);
    let method = ctx.params.e("method");
    // Sample times (with weights) for one instant of source time.
    let instant = |s: f64| -> Vec<(f64, f32)> {
        let f = s * fps;
        let f0 = (f + 1e-6).floor();
        let frac = (f - f0) as f32;
        if method == 0 || frac < 1e-4 { vec![(f0 / fps, 1.0)] } else { vec![(f0 / fps, 1.0 - frac), ((f0 + 1.0) / fps, frac)] }
    };
    let mut samples: Vec<(f64, f32)> = Vec::new();
    if ctx.params.b("enableMotionBlur") {
        let angle = if ctx.params.e("shutterControl") == 1 { ctx.params.f("shutterAngle") } else { 180.0 };
        let n = ctx.params.f("shutterSamples").round().clamp(1.0, 64.0) as usize;
        let rate = if by_frame { 1.0 } else { speed / 100.0 };
        let span = angle / 360.0 * rate / fps;
        for i in 0..n {
            let s = src + if n > 1 { span * i as f64 / (n - 1) as f64 } else { 0.0 };
            samples.extend(instant(s).into_iter().map(|(t, w)| (t, w / n as f32)));
        }
    } else {
        samples = instant(src);
    }
    let times: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let weights: Vec<f32> = samples.iter().map(|s| s.1).collect();
    let Some(fr) = fetch(ctx, &times) else { return b };
    let img = average(&fr.imgs, &weights);
    with_img(fr.grid, img)
}

// ---------------------------------------------------------------- CC Force Motion Blur / CC Wide Time / Pixel Motion Blur

fn force_motion_blur(ctx: &EffectCtx, b: Buf) -> Buf {
    let n = ctx.params.f("motionBlurLevels").round().clamp(2.0, 64.0) as usize;
    let angle = if ctx.params.b("overrideShutterAngle") { ctx.params.f("shutterAngle") } else { 180.0 };
    let span = angle.clamp(0.0, 3600.0) / 360.0 / ctx.fps();
    let times: Vec<f64> = (0..n).map(|i| ctx.time + span * i as f64 / n as f64).collect();
    mean_of(ctx, b, &times)
}

fn wide_time(ctx: &EffectCtx, b: Buf) -> Buf {
    let fwd = ctx.params.f("forwardSteps").round().clamp(0.0, 45.0) as i64;
    let back = ctx.params.f("backwardSteps").round().clamp(0.0, 45.0) as i64;
    let fd = 1.0 / ctx.fps();
    let times: Vec<f64> = (-back..=fwd).map(|k| ctx.time + k as f64 * fd).collect();
    mean_of(ctx, b, &times)
}

fn pixel_motion_blur(ctx: &EffectCtx, b: Buf) -> Buf {
    let angle = if ctx.params.e("shutterControl") == 1 { ctx.params.f("shutterAngle") } else { 180.0 };
    let n = ctx.params.f("shutterSamples").round().clamp(1.0, 64.0) as usize;
    let span = angle.clamp(0.0, 720.0) / 360.0 / ctx.fps();
    let times: Vec<f64> = (0..n).map(|i| ctx.time - span * 0.5 + if n > 1 { span * i as f64 / (n - 1) as f64 } else { span * 0.5 }).collect();
    mean_of(ctx, b, &times)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, EffectHost, LayerPixels, run_fx};

    /// A layer whose every pixel at layer time `t` is grey `t` (clamped), alpha 1, on a 6×4 grid.
    struct Clock {
        static_layer: bool,
    }
    impl EffectHost for Clock {
        fn layer(&self, _: u64, _: bool) -> Option<LayerPixels> {
            None
        }
        fn audio(&self, _: u64, _: f64, _: usize, _: u32) -> Option<Vec<f32>> {
            None
        }
        fn self_at(&self, t: f64, effects: usize) -> Option<Buf> {
            assert_eq!(effects, 0);
            let v = if self.static_layer { 0.4 } else { t as f32 };
            Some(Buf { img: Image::filled(6, 4, [v, v, v, 1.0]), offset: [0.0; 2], scale: 1.0 })
        }
    }

    fn run(id: &str, vals: &[(&str, Value)], t: f64, host: &Clock) -> Buf {
        let env = EffectEnv { host: Some(host), frame_rate: 10.0, comp_time: t, ..Default::default() };
        run_fx(id, vals, Image::filled(6, 4, [0.9, 0.0, 0.0, 1.0]), t, env)
    }

    #[test]
    fn posterize_time_holds_frames() {
        let h = Clock { static_layer: false };
        for (t, held) in [(0.0, 0.0), (0.1, 0.0), (0.24, 0.0), (0.25, 0.25), (0.49, 0.25), (0.5, 0.5)] {
            let out = run("ec.time.posterizetime", &[("frameRate", num(4.0))], t, &h);
            assert!((out.img.data[0][0] - held).abs() < 1e-5, "t={t}: {}", out.img.data[0][0]);
        }
    }

    #[test]
    fn echo_blends_known_frames_with_decay() {
        let h = Clock { static_layer: false };
        let vals = [("echoTime", num(-0.1)), ("numberOfEchoes", num(2.0)), ("startingIntensity", num(1.0)), ("decay", num(0.5))];
        let out = run("ec.time.echo", &vals, 0.8, &h);
        // Add: f(0.8) + 0.5 f(0.7) + 0.25 f(0.6)
        let want = 0.8 + 0.5 * 0.7 + 0.25 * 0.6;
        assert!((out.img.data[0][0] - want as f32).abs() < 1e-4, "{}", out.img.data[0][0]);
        assert_eq!(out.img.data[0][3], 1.0);
        // Maximum picks the brightest weighted frame; Blend averages.
        let mut v = vals.to_vec();
        v.push(("echoOperator", Value::Enum(1)));
        let out = run("ec.time.echo", &v, 0.8, &h);
        assert!((out.img.data[0][0] - 0.8).abs() < 1e-5);
        v.pop();
        v.push(("echoOperator", Value::Enum(6)));
        let out = run("ec.time.echo", &v, 0.8, &h);
        assert!((out.img.data[0][0] - want as f32 / 3.0).abs() < 1e-4);
    }

    #[test]
    fn time_difference_of_static_layer_is_black() {
        let h = Clock { static_layer: true };
        let out = run("ec.time.timedifference", &[("timeOffset", num(-0.5))], 1.0, &h);
        assert!(out.img.data.iter().all(|p| p[0] == 0.0 && p[1] == 0.0 && p[2] == 0.0 && p[3] == 1.0));
        // A changing layer is not black: |1.0 - 0.5| = 0.5.
        let h = Clock { static_layer: false };
        let out = run("ec.time.timedifference", &[("timeOffset", num(-0.5))], 1.0, &h);
        assert!((out.img.data[0][0] - 0.5).abs() < 1e-5, "{}", out.img.data[0][0]);
    }

    #[test]
    fn timewarp_and_wide_time() {
        let h = Clock { static_layer: false };
        // Speed 50 % at t = 1 s shows source 0.5 s (frame-aligned at 10 fps).
        let out = run("ec.time.timewarp", &[("method", Value::Enum(0))], 1.0, &h);
        assert!((out.img.data[0][0] - 0.5).abs() < 1e-5);
        // Frame Mix between 0.5 and 0.6 at source 0.55.
        let out = run("ec.time.timewarp", &[("method", Value::Enum(1))], 1.1, &h);
        assert!((out.img.data[0][0] - 0.55).abs() < 1e-4, "{}", out.img.data[0][0]);
        // Source Frame mode: frame 3 at 10 fps.
        let out = run("ec.time.timewarp", &[("method", Value::Enum(0)), ("adjustTimeBy", Value::Enum(1)), ("sourceFrame", num(3.0))], 2.0, &h);
        assert!((out.img.data[0][0] - 0.3).abs() < 1e-5);
        // Wide Time is symmetric around t: mean of t-0.3..t+0.3 = t.
        let out = run("ec.time.ccwidetime", &[], 1.0, &h);
        assert!((out.img.data[0][0] - 1.0).abs() < 1e-4);
        // Pixel Motion Blur is centred too.
        let out = run("ec.time.pixelmotionblur", &[], 1.0, &h);
        assert!((out.img.data[0][0] - 1.0).abs() < 1e-4);
        // Force Motion Blur looks forward over the shutter.
        let out = run("ec.time.ccforcemotionblur", &[], 1.0, &h);
        assert!(out.img.data[0][0] > 1.0);
    }

    #[test]
    fn time_displacement_uses_luma_as_offset() {
        // Static map from a bright self frame: luminance 1.0 → +max seconds.
        let h = Clock { static_layer: true };
        let out = run("ec.time.timedisplacement", &[], 0.0, &h);
        assert!((out.img.data[0][0] - 0.4).abs() < 1e-5);
    }
}
