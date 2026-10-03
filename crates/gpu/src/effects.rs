//! GPU effects: the same steps as the CPU effects in `effectcraft-effects` (padding, blur
//! radii, parameter conversions), with the pixel loops as compute kernels.

use effectcraft_effects::{Buf, EffectCtx};
use effectcraft_geom::{Mat3, vec2};
use effectcraft_raster::Sampling;
use effectcraft_render::FxStep;

use crate::context::{Enc, GpuImage, Params};
use crate::ops;

/// Effects with a GPU implementation (must equal `effectcraft_effects::GPU_EFFECTS`), plus the
/// expression controls (pass-throughs).
pub fn supports(id: &str) -> bool {
    effectcraft_effects::GPU_EFFECTS.contains(&id) || id.starts_with("ec.control.")
}

/// A layer buffer on the GPU (see [`Buf`]).
struct GBuf {
    img: GpuImage,
    offset: [f64; 2],
    scale: f64,
}

impl GBuf {
    fn to_px(&self, p: [f64; 2]) -> (f64, f64) {
        (p[0] * self.scale + self.offset[0], p[1] * self.scale + self.offset[1])
    }

    /// Buf::pad: grow by `pad` transparent pixels on every side.
    fn pad(&mut self, e: &mut Enc, pad: u32) -> Option<()> {
        if pad == 0 {
            return Some(());
        }
        let (w, h) = (self.img.width + 2 * pad, self.img.height + 2 * pad);
        if !e.g.fits(w, h) {
            return None;
        }
        let out = e.image(w, h);
        e.copy_into(&self.img, &out, pad, pad);
        self.img = out;
        self.offset[0] += pad as f64;
        self.offset[1] += pad as f64;
        Some(())
    }
}

/// Run a chain of GPU effects (one upload, one readback), quantising after each when `levels`
/// is set. `None` when anything cannot run here.
pub(crate) fn run_chain(e: &mut Enc, chain: &[FxStep], buf: &Buf, levels: Option<f32>) -> Option<Buf> {
    if !e.g.can_readback() || buf.img.is_empty() {
        return None;
    }
    let mut b = GBuf { img: e.g.upload_image(&buf.img)?, offset: buf.offset, scale: buf.scale };
    for step in chain {
        b = apply(e, step.spec.id, &step.ctx, b)?;
        if let Some(l) = levels {
            b.img = ops::quantize(e, &b.img, l);
        }
    }
    let img = e.download(&b.img)?;
    Some(Buf { img, offset: b.offset, scale: b.scale })
}

fn apply(e: &mut Enc, id: &str, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    match id {
        "ec.blur.gaussian" => gaussian(e, ctx, b),
        "ec.blur.fastbox" => box_blur(e, ctx, b),
        "ec.blur.directional" => directional(e, ctx, b),
        "ec.stylize.glow" => glow(e, ctx, b),
        "ec.perspective.dropshadow" => drop_shadow(e, ctx, b),
        "ec.distort.transform" => transform(e, ctx, b),
        "ec.color.curves" => curves(e, ctx, b),
        _ if id.starts_with("ec.control.") => Some(b),
        _ => pointwise(e, id, ctx, b),
    }
}

// ---------------------------------------------------------------- blurs

/// raster::blur::box_radii.
fn box_radii(sigma: f64, n: usize) -> Vec<usize> {
    if sigma <= 0.0 {
        return vec![0; n];
    }
    let w_ideal = (12.0 * sigma * sigma / n as f64 + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - (n as i64 * wl * wl) as f64 - 4.0 * n as f64 * wl as f64 - 3.0 * n as f64) / (-4.0 * wl as f64 - 4.0);
    let m = m_ideal.round() as i64;
    (0..n as i64).map(|i| (((if i < m { wl } else { wu }) - 1) / 2).max(0) as usize).collect()
}

/// Horizontal passes with radii `rx`, then vertical passes with radii `ry` (raster::blur).
fn box_passes(e: &mut Enc, img: &GpuImage, rx: &[usize], ry: &[usize], repeat: bool) -> GpuImage {
    let mut cur = img.clone();
    let passes = rx.iter().map(|&r| (false, r)).chain(ry.iter().map(|&r| (true, r))).filter(|p| p.1 > 0);
    for (vertical, r) in passes {
        let block = if vertical { (4 * r).clamp(32, 256) } else { 256 } as u32;
        let (lines, n) = if vertical { (cur.width, cur.height) } else { (cur.height, cur.width) };
        let mut p = Params::default();
        p.u[0] = [r as u32, repeat as u32, block, 0];
        let out = e.image(cur.width, cur.height);
        e.dispatch(if vertical { "box_v" } else { "box_h" }, &p, &cur, None, &out, None, (lines.div_ceil(64), n.div_ceil(block)));
        cur = out;
    }
    cur
}

/// raster::gaussian_blur (3 box passes per axis).
fn gaussian_blur(e: &mut Enc, img: &GpuImage, sx: f64, sy: f64, repeat: bool) -> GpuImage {
    let rx = if sx > 0.05 { box_radii(sx, 3) } else { vec![] };
    let ry = if sy > 0.05 { box_radii(sy, 3) } else { vec![] };
    box_passes(e, img, &rx, &ry, repeat)
}

fn dims_xy(dim: u32) -> (f64, f64) {
    match dim {
        1 => (1.0, 0.0),
        2 => (0.0, 1.0),
        _ => (1.0, 1.0),
    }
}

fn gaussian(e: &mut Enc, ctx: &EffectCtx, mut b: GBuf) -> Option<GBuf> {
    let s = ctx.params.f("blurriness").max(0.0) * 0.5 * b.scale;
    if s <= 0.0 {
        return Some(b);
    }
    let (kx, ky) = dims_xy(ctx.params.e("dimensions"));
    let repeat = ctx.params.b("repeatEdge");
    if !repeat && !ctx.adjustment {
        b.pad(e, (s * 3.0).ceil() as u32)?;
    }
    b.img = gaussian_blur(e, &b.img, s * kx, s * ky, repeat || ctx.adjustment);
    Some(b)
}

fn box_blur(e: &mut Enc, ctx: &EffectCtx, mut b: GBuf) -> Option<GBuf> {
    let r = (ctx.params.f("radius") * b.scale).round().max(0.0) as usize;
    if r == 0 {
        return Some(b);
    }
    let it = ctx.params.f("iterations").round().clamp(1.0, 50.0) as usize;
    let (kx, ky) = dims_xy(ctx.params.e("dimensions"));
    let repeat = ctx.params.b("repeatEdge");
    if !repeat && !ctx.adjustment {
        b.pad(e, (r * it) as u32 + 1)?;
    }
    b.img = box_passes(e, &b.img, &vec![r * kx as usize; it], &vec![r * ky as usize; it], repeat || ctx.adjustment);
    Some(b)
}

fn directional(e: &mut Enc, ctx: &EffectCtx, mut b: GBuf) -> Option<GBuf> {
    let len = ctx.params.f("length") * b.scale;
    if len < 0.5 {
        return Some(b);
    }
    if !ctx.adjustment {
        b.pad(e, len.ceil() as u32 + 1)?;
    }
    let a = ctx.params.f("direction").to_radians();
    let n = (len.ceil() as usize * 2 + 1).clamp(3, 257);
    let mut p = Params::default();
    p.f[0] = [a.sin() as f32, -a.cos() as f32, len as f32, n as f32];
    let out = e.image(b.img.width, b.img.height);
    e.pixels("directional", &p, &b.img, None, &out, None);
    b.img = out;
    Some(b)
}

fn glow(e: &mut Enc, ctx: &EffectCtx, mut b: GBuf) -> Option<GBuf> {
    let thr = ctx.params.f("threshold") as f32 / 100.0;
    let radius = ctx.params.f("radius") * b.scale;
    let intensity = ctx.params.f("intensity") as f32;
    let use_colors = ctx.params.e("colors") == 1;
    let ca = ctx.params.color("colorA");
    let cb = ctx.params.color("colorB");
    let (kx, ky) = dims_xy(ctx.params.e("glowDimensions"));
    if !ctx.adjustment {
        b.pad(e, (radius * 1.5).ceil() as u32 + 2)?;
    }
    let mut p = Params::default();
    p.u[0] = [use_colors as u32, (ctx.params.e("based") == 0) as u32, ctx.params.e("colorLooping"), 0];
    p.f[0] = [thr, ctx.params.f("colorLoops") as f32, (ctx.params.f("colorPhase") / 360.0) as f32, ctx.params.f("abMidpoint") as f32 / 100.0];
    p.f[1] = ca;
    p.f[2] = cb;
    let bright = e.image(b.img.width, b.img.height);
    e.pixels("glow_bright", &p, &b.img, None, &bright, None);
    let s = (radius / 2.0).max(0.5);
    let blurred = gaussian_blur(e, &bright, s * kx, s * ky, false);
    let mut p = Params::default();
    p.u[0][0] = ctx.params.e("operation").min(2);
    p.f[0][0] = intensity;
    let out = e.image(b.img.width, b.img.height);
    e.pixels("glow_combine", &p, &b.img, Some(&blurred), &out, None);
    b.img = out;
    Some(b)
}

fn drop_shadow(e: &mut Enc, ctx: &EffectCtx, mut b: GBuf) -> Option<GBuf> {
    let color = ctx.params.color("color");
    let opacity = ctx.params.f("opacity") as f32 / 100.0;
    let dir = ctx.params.f("direction").to_radians();
    let dist = ctx.params.f("distance") * b.scale;
    let soft = ctx.params.f("softness") * b.scale;
    let only = ctx.params.b("shadowOnly");
    let pad = (dist + soft * 1.5).ceil() as u32 + 2;
    if !ctx.adjustment {
        b.pad(e, pad)?;
    }
    let (dx, dy) = (dir.sin() * dist, -dir.cos() * dist);
    let mut p = Params::default();
    p.f[0] = color;
    p.f[1] = [opacity, dx as f32, dy as f32, 0.0];
    let mut sh = e.image(b.img.width, b.img.height);
    e.pixels("shadow_make", &p, &b.img, None, &sh, None);
    if soft > 0.0 {
        sh = gaussian_blur(e, &sh, soft / 2.0, soft / 2.0, false);
    }
    if !only {
        let out = e.image(b.img.width, b.img.height);
        e.pixels("shadow_combine", &Params::default(), &sh, Some(&b.img), &out, None);
        sh = out;
    }
    b.img = sh;
    Some(b)
}

fn transform(e: &mut Enc, ctx: &EffectCtx, mut b: GBuf) -> Option<GBuf> {
    let anchor = b.to_px(ctx.params.v2("anchor"));
    let pos = b.to_px(ctx.params.v2("position"));
    let sh = ctx.params.f("scaleHeight");
    let sw = if ctx.params.b("uniform") { sh } else { ctx.params.f("scaleWidth") };
    let opacity = ctx.params.f("opacity") as f32 / 100.0;
    let m = Mat3::translate(vec2(pos.0, pos.1))
        * Mat3::rotate_deg(ctx.params.f("rotation"))
        * Mat3::skew_deg(ctx.params.f("skew"), ctx.params.f("skewAxis"))
        * Mat3::scale(vec2(sw / 100.0, sh / 100.0))
        * Mat3::translate(vec2(-anchor.0, -anchor.1));
    let sampling = if ctx.params.e("sampling") == 1 { Sampling::Bicubic } else { Sampling::Bilinear };
    let empty = e.image(b.img.width, b.img.height);
    b.img = ops::warp(e, &empty, &b.img, &m, sampling, effectcraft_color::BlendMode::Normal, opacity, 0, None);
    Some(b)
}

// ---------------------------------------------------------------- per-pixel

fn curves(e: &mut Enc, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let parsed = ["rgb", "red", "green", "blue", "alpha"].map(|id| effectcraft_effects::Curve::parse(ctx.params.s(id)));
    if parsed.iter().all(Option::is_none) {
        return Some(b);
    }
    let mut data = vec![];
    let mut offs = [-1.0f32; 5];
    for (i, c) in parsed.iter().enumerate() {
        let Some(c) = c else { continue };
        let (xs, ys, ms, lut) = c.tables();
        offs[i] = data.len() as f32;
        data.push(xs.len() as f32);
        data.extend_from_slice(xs);
        data.extend_from_slice(ys);
        data.extend_from_slice(ms);
        data.extend_from_slice(lut);
    }
    let buf = e.data(&data);
    let mut p = Params::default();
    p.u[0][0] = 8;
    p.f[0] = [offs[0], offs[1], offs[2], offs[3]];
    p.f[1][0] = offs[4];
    let out = e.image(b.img.width, b.img.height);
    e.pixels("pointwise", &p, &b.img, None, &out, Some(&buf));
    Some(GBuf { img: out, ..b })
}

fn rgb(c: [f32; 4]) -> [f32; 4] {
    [c[0], c[1], c[2], 0.0]
}

fn pointwise(e: &mut Enc, id: &str, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let f = |k: &str| ctx.params.f(k);
    let mut p = Params::default();
    match id {
        "ec.color.tint" => {
            p.u[0][0] = 1;
            p.f[0] = rgb(ctx.params.color("black"));
            p.f[1] = rgb(ctx.params.color("white"));
            p.f[2][0] = f("amount") as f32 / 100.0;
        }
        "ec.color.brightnesscontrast" => {
            let br = f("brightness") as f32 / 100.0;
            let ct = f("contrast") as f32 / 100.0;
            let k = if ct >= 0.0 { 1.0 / (1.0 - ct * 0.99) } else { 1.0 + ct };
            let legacy = ctx.params.b("useLegacy");
            // Mode: 0 = identity, 1 = legacy linear, 2 = tone curves (see `bc_modern`).
            let mode = if br == 0.0 && ct == 0.0 {
                0
            } else if legacy {
                1
            } else {
                2
            };
            p.u[0] = [2, mode, 0, 0];
            p.f[0] = [br, k, ct, 0.0];
        }
        "ec.color.huesaturation" => {
            // Colour ranges (Channel Control) render on the CPU.
            if !effectcraft_effects::huesat_ranges_identity(ctx) {
                return None;
            }
            p.u[0] = [3, ctx.params.b("colorize") as u32, 0, 0];
            p.f[0] = [f("hue") as f32 / 360.0, f("saturation") as f32 / 100.0, f("lightness") as f32 / 100.0, 0.0];
            p.f[1] = [f("colorizeHue") as f32 / 360.0, f("colorizeSaturation") as f32 / 100.0, f("colorizeLightness") as f32 / 100.0, 0.0];
        }
        "ec.color.levels" => {
            // Red / Green / Blue / Alpha controls render on the CPU.
            if !effectcraft_effects::levels_channels_identity(ctx) {
                return None;
            }
            let (clip_b, clip_w) = effectcraft_effects::levels_clip(ctx);
            p.u[0] = [4, clip_b as u32, clip_w as u32, 0];
            p.f[0] = [f("inBlack") as f32, f("inWhite") as f32, f("gamma").max(0.01) as f32, f("outBlack") as f32];
            p.f[1][0] = f("outWhite") as f32;
        }
        "ec.color.exposure" => {
            let s = effectcraft_effects::exposure_settings(ctx);
            p.u[0] = [5, ctx.params.b("bypassLinearLight") as u32, 0, 0];
            for i in 0..3 {
                p.f[0][i] = s[i].0;
                p.f[1][i] = s[i].1;
                p.f[2][i] = s[i].2;
            }
        }
        "ec.channel.invert" => {
            // The shader does the RGB channels and Alpha; HLS / YIQ inversions run on the CPU.
            let ch = match ctx.params.e("channel") {
                c @ 0..=3 => c,
                effectcraft_effects::INVERT_ALPHA => 4,
                _ => return None,
            };
            p.u[0] = [6, ch, 0, 0];
            p.f[0][0] = 1.0 - f("blend") as f32 / 100.0;
        }
        "ec.generate.fill" => {
            // Fill Mask / All Masks render on the CPU.
            if effectcraft_effects::fill_uses_masks(ctx) {
                return None;
            }
            p.u[0] = [7, ctx.params.b("invert") as u32, 0, 0];
            p.f[0] = ctx.params.color("color");
            p.f[1][0] = f("opacity") as f32 / 100.0;
        }
        "ec.generate.gradientramp" => {
            let s = b.to_px(ctx.params.v2("start"));
            let en = b.to_px(ctx.params.v2("end"));
            let (dx, dy) = (en.0 - s.0, en.1 - s.1);
            let len2 = (dx * dx + dy * dy).max(1e-9);
            p.u[0] = [9, (ctx.params.e("shape") == 1) as u32, ctx.seed, 0];
            p.f[0] = [s.0 as f32, s.1 as f32, en.0 as f32, en.1 as f32];
            p.f[1] = rgb(ctx.params.color("startColor"));
            p.f[2] = rgb(ctx.params.color("endColor"));
            p.f[3] = [len2 as f32, f("scatter") as f32 / 512.0, f("blend") as f32 / 100.0, 0.0];
        }
        "ec.noise.fractal" => {
            // The shader covers Soft Linear noise, the Basic / Turbulent Smooth / Turbulent Basic
            // types, all Overflow modes, independent width / height scaling, Sub Influence and Sub
            // Scaling, and the None / Normal blending modes; anything else renders on the CPU.
            let pr = ctx.params;
            let kind = pr.e("fractalType");
            let mode = pr.e("blendingMode");
            let sub_offset = pr.v2("subSettings/subOffset");
            if pr.e("noiseType") != 2
                || kind > 2
                || mode > 1
                || pr.b("evolutionOptions/cycleEvolution")
                || pr.b("transform/perspectiveOffset")
                || f("subSettings/subRotation") != 0.0
                || (!pr.b("subSettings/centerSubscale") && sub_offset != [0.0, 0.0])
            {
                return None;
            }
            let octaves = f("complexity").clamp(1.0, 20.0);
            let (sr, cr) = (f("transform/rotation") as f32).to_radians().sin_cos();
            let (ox, oy) = b.to_px(pr.v2("transform/offset"));
            let s = f("transform/scale");
            let (sw, sh) = if pr.b("transform/uniformScaling") { (s, s) } else { (f("transform/scaleWidth"), f("transform/scaleHeight")) };
            let sub = (f("subSettings/subScaling") / 100.0).clamp(0.1, 1.0);
            p.u[0] = [10, kind, octaves.ceil() as u32, f("evolutionOptions/seed") as u32 ^ 0x51ed];
            p.u[1] = [pr.b("invert") as u32, pr.e("overflow"), mode, 0];
            p.f[0] = [f("contrast") as f32 / 100.0, f("brightness") as f32 / 100.0, (sw * b.scale).max(1.0) as f32, (octaves - octaves.floor()) as f32];
            p.f[1] = [ox as f32, oy as f32, sr, cr];
            p.f[2] = [f("evolution") as f32 / 360.0, f("blend") as f32 / 100.0, (f("opacity") as f32 / 100.0).clamp(0.0, 1.0), (sh * b.scale).max(1.0) as f32];
            p.f[3] = [(f("subSettings/subInfluence") as f32 / 100.0).clamp(0.0, 1.0), 1.0 / sub as f32, 0.0, 0.0];
        }
        _ => return None,
    }
    let out = e.image(b.img.width, b.img.height);
    e.pixels("pointwise", &p, &b.img, None, &out, None);
    Some(GBuf { img: out, ..b })
}
