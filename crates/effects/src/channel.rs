//! Channel effects: channel routing (Set/Shift Channels, Channel Combiner, Set Matte), morphology
//! (Minimax), per-channel arithmetic, solid compositing and un-matting.

use effectcraft_color::{BlendMode, blend_pixel, hsl_to_rgb, luminance, rgb_to_hsl};
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use rayon::prelude::*;

use crate::util::{Plane, SRC_NAMES, Src, join, morph_plane, pick, premul, src_at, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Channel", params, render, gpu: false, float: true }
}

/// Route four sources into R, G, B, A (straight colour), premultiplying the result.
fn route(b: &mut Buf, srcs: [Src; 4]) {
    if srcs == [Src::Red, Src::Green, Src::Blue, Src::Alpha] {
        return;
    }
    b.img.data.par_iter_mut().for_each(|px| {
        let (c, a) = unpremul(*px);
        let v = srcs.map(|s| pick(s, c, a));
        *px = premul([v[0], v[1], v[2]], v[3].clamp(0.0, 1.0));
    });
}

fn set_channels(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let s = ["setRedTo", "setGreenTo", "setBlueTo", "setAlphaTo"].map(|id| src_at(ctx.params.e(id)));
    route(&mut b, s);
    b
}

const SHIFT_ORDER: [Src; 11] =
    [Src::Alpha, Src::Red, Src::Green, Src::Blue, Src::Luminance, Src::Hue, Src::Lightness, Src::Saturation, Src::Full, Src::Half, Src::Off];
const SHIFT_NAMES: [&str; 11] = ["Alpha", "Red", "Green", "Blue", "Luminance", "Hue", "Lightness", "Saturation", "Full On", "Half On", "Full Off"];

fn shift_channels(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let s = ["takeRedFrom", "takeGreenFrom", "takeBlueFrom", "takeAlphaFrom"].map(|id| SHIFT_ORDER[(ctx.params.e(id) as usize).min(10)]);
    route(&mut b, s);
    b
}

fn rgb_to_yuv(c: [f32; 3]) -> [f32; 3] {
    let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    [y, 0.492 * (c[2] - y) + 0.5, 0.877 * (c[0] - y) + 0.5]
}

fn yuv_to_rgb(c: [f32; 3]) -> [f32; 3] {
    let (y, u, v) = (c[0], c[1] - 0.5, c[2] - 0.5);
    let r = y + v / 0.877;
    let b = y + u / 0.492;
    let g = (y - 0.299 * r - 0.114 * b) / 0.587;
    [r, g, b]
}

fn channel_combiner(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let from = ctx.params.e("from");
    let to = ctx.params.e("to");
    let inv = ctx.params.b("invert");
    b.img.data.par_iter_mut().for_each(|px| {
        let (c, mut a) = unpremul(*px);
        if a <= 0.0 && from != 7 {
            return;
        }
        let iv = |v: f32| if inv { 1.0 - v } else { v };
        let out = match from {
            0..=3 => {
                let o = match from {
                    0 => {
                        let (h, s, l) = rgb_to_hsl(c[0], c[1], c[2]);
                        [h, l, s]
                    }
                    1 => {
                        let (r, g, bb) = hsl_to_rgb(c[0], c[2], c[1]);
                        [r, g, bb]
                    }
                    2 => rgb_to_yuv(c),
                    _ => yuv_to_rgb(c),
                };
                o.map(iv)
            }
            _ => {
                let v = iv(match from {
                    4 => c[0],
                    5 => c[1],
                    6 => c[2],
                    7 => a,
                    8 => rgb_to_hsl(c[0], c[1], c[2]).2,
                    9 => rgb_to_hsl(c[0], c[1], c[2]).0,
                    10 => rgb_to_hsl(c[0], c[1], c[2]).1,
                    11 => luminance(c[0], c[1], c[2]),
                    12 => c[0].max(c[1]).max(c[2]),
                    _ => c[0].min(c[1]).min(c[2]),
                });
                let mut o = c;
                match to {
                    0 => o[0] = v,
                    1 => o[1] = v,
                    2 => o[2] = v,
                    3 => a = v.clamp(0.0, 1.0),
                    4..=6 => {
                        let (mut h, mut s, mut l) = rgb_to_hsl(c[0], c[1], c[2]);
                        match to {
                            4 => l = v,
                            5 => h = v,
                            _ => s = v,
                        }
                        let (r, g, bb) = hsl_to_rgb(h, s.clamp(0.0, 1.0), l);
                        o = [r, g, bb];
                    }
                    _ => o = [v, v, v],
                }
                o
            }
        };
        *px = premul(out.map(|v| v.max(0.0)), a);
    });
    b
}

fn minimax(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let r = (ctx.params.f("radius") * b.scale).round().max(0.0) as usize;
    if r == 0 {
        return b;
    }
    let op = ctx.params.e("operation");
    let ch = ctx.params.e("channel");
    let (rx, ry) = match ctx.params.e("direction") {
        1 => (r, 0),
        2 => (0, r),
        _ => (r, r),
    };
    // Which straight channels (R, G, B, A) to process.
    let sel: [bool; 4] = match ch {
        1 => [false, false, false, true],
        2 => [true, true, true, true],
        3 => [true, false, false, false],
        4 => [false, true, false, false],
        5 => [false, false, true, false],
        _ => [true, true, true, false],
    };
    if sel[3] && op != 0 && !ctx.adjustment {
        b.pad((rx.max(ry)) as u32);
    }
    let mut planes = [0, 1, 2, 3].map(|k| Plane::from_image(&b.img, |px| if k == 3 { px[3] } else { unpremul(px).0[k] }));
    for (k, pl) in planes.iter_mut().enumerate() {
        if !sel[k] {
            continue;
        }
        *pl = match op {
            0 => morph_plane(pl, rx, ry, false),
            1 => morph_plane(pl, rx, ry, true),
            2 => morph_plane(&morph_plane(pl, rx, ry, false), rx, ry, true),
            _ => morph_plane(&morph_plane(pl, rx, ry, true), rx, ry, false),
        };
    }
    let mut img = join(&planes);
    img.data.par_iter_mut().for_each(|px| {
        let a = px[3].clamp(0.0, 1.0);
        *px = premul([px[0], px[1], px[2]], a);
    });
    b.img = img;
    b
}

fn arithmetic(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let op = ctx.params.e("operator");
    let k = ["redValue", "greenValue", "blueValue"].map(|id| ctx.params.f(id) as f32 / 255.0);
    let clip = ctx.params.b("clip");
    b.img.map_straight(|c| {
        let mut o = [0.0f32; 3];
        for i in 0..3 {
            let (v, kv) = (c[i], k[i]);
            let iv = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            let ik = (kv * 255.0).round() as u8;
            let r = match op {
                0 => (iv & ik) as f32 / 255.0,
                1 => (iv | ik) as f32 / 255.0,
                2 => (iv ^ ik) as f32 / 255.0,
                3 => v + kv,
                4 => v - kv,
                5 => (v - kv).abs(),
                6 => v.min(kv),
                7 => v.max(kv),
                8 => {
                    if v > kv {
                        0.0
                    } else {
                        v
                    }
                }
                9 => {
                    if v < kv {
                        0.0
                    } else {
                        v
                    }
                }
                10 => {
                    if v >= kv {
                        1.0
                    } else {
                        0.0
                    }
                }
                11 => v * kv,
                _ => 1.0 - (1.0 - v) * (1.0 - kv),
            };
            o[i] = if clip { r.clamp(0.0, 1.0) } else { r.max(0.0) };
        }
        o
    });
    b
}

const SOLID_MODES: [BlendMode; 12] = [
    BlendMode::Normal,
    BlendMode::Add,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Color,
    BlendMode::Luminosity,
];
const SOLID_MODE_NAMES: [&str; 12] =
    ["Normal", "Add", "Multiply", "Screen", "Overlay", "Soft Light", "Hard Light", "Darken", "Lighten", "Difference", "Color", "Luminosity"];

fn solid_composite(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let so = ctx.params.f("sourceOpacity") as f32 / 100.0;
    let c = ctx.params.color("color");
    let op = (ctx.params.f("opacity") as f32 / 100.0 * c[3]).clamp(0.0, 1.0);
    let mode = SOLID_MODES[(ctx.params.e("blendingMode") as usize).min(SOLID_MODES.len() - 1)];
    let solid = premul([c[0], c[1], c[2]], op);
    b.img.data.par_iter_mut().for_each(|px| {
        let src = px.map(|v| v * so);
        let mut o = blend_pixel(mode, solid, src, 0.5);
        o[3] = o[3].clamp(0.0, 1.0);
        for v in o.iter_mut().take(3) {
            *v = v.max(0.0);
        }
        *px = o;
    });
    b
}

fn remove_color_matting(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let bg = ctx.params.color("backgroundColor");
    let clip = ctx.params.b("clipHdr");
    b.img.data.par_iter_mut().for_each(|px| {
        let a = px[3];
        if a <= 1e-6 {
            *px = [0.0; 4];
            return;
        }
        for i in 0..3 {
            let s = px[i] / a;
            let mut v = (s - bg[i] * (1.0 - a)).max(0.0);
            if clip {
                v = v.min(a);
            }
            px[i] = v;
        }
    });
    b
}

const MATTE_ORDER: [Src; 10] = [Src::Red, Src::Green, Src::Blue, Src::Alpha, Src::Luminance, Src::Hue, Src::Lightness, Src::Saturation, Src::Full, Src::Off];

fn set_matte(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let src = MATTE_ORDER[(ctx.params.e("takeMatteFrom") as usize).min(9)];
    let inv = ctx.params.b("invertMatte");
    let comp = ctx.params.b("compositeMatteWithOriginal");
    let pre = ctx.params.b("premultiplyMatteLayer");
    b.img.data.par_iter_mut().for_each(|px| {
        let (c, a) = unpremul(*px);
        let mut m = pick(src, c, a);
        if pre && !matches!(src, Src::Alpha | Src::Full | Src::Off) {
            m *= a;
        }
        let m = m.clamp(0.0, 1.0);
        let m = if inv { 1.0 - m } else { m };
        // The layer's own (non-inverted) alpha already is the original matte: don't square it.
        let na = if comp && (src != Src::Alpha || inv) { a * m } else { m };
        *px = premul(c, na.clamp(0.0, 1.0));
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let srcp = |id: &'static str, name: &'static str, d: u32| p(id, name, Value::Enum(d), popup(&SRC_NAMES));
    let shp = |id: &'static str, name: &'static str, d: u32| p(id, name, Value::Enum(d), popup(&SHIFT_NAMES));
    let v255 = || slider(0.0, 255.0, 0.0, 255.0, 0);
    vec![
        spec(
            "ec.channel.setchannels",
            "Set Channels",
            vec![
                srcp("setRedTo", "Set Red To Source 1's", 0),
                srcp("setGreenTo", "Set Green To Source 2's", 1),
                srcp("setBlueTo", "Set Blue To Source 3's", 2),
                srcp("setAlphaTo", "Set Alpha To Source 4's", 3),
            ],
            set_channels,
        ),
        spec(
            "ec.channel.shiftchannels",
            "Shift Channels",
            vec![
                shp("takeAlphaFrom", "Take Alpha From", 0),
                shp("takeRedFrom", "Take Red From", 1),
                shp("takeGreenFrom", "Take Green From", 2),
                shp("takeBlueFrom", "Take Blue From", 3),
            ],
            shift_channels,
        ),
        spec(
            "ec.channel.combiner",
            "Channel Combiner",
            vec![
                p(
                    "from",
                    "From",
                    Value::Enum(8),
                    popup(&[
                        "RGB to HLS",
                        "HLS to RGB",
                        "RGB to YUV",
                        "YUV to RGB",
                        "Red",
                        "Green",
                        "Blue",
                        "Alpha",
                        "Lightness",
                        "Hue",
                        "Saturation",
                        "Luminance",
                        "Max RGB",
                        "Min RGB",
                    ]),
                ),
                p("to", "To", Value::Enum(7), popup(&["Red Only", "Green Only", "Blue Only", "Alpha Only", "Lightness", "Hue", "Saturation", "Grayscale"])),
                p("invert", "Invert", Value::Bool(false), ParamUi::Checkbox),
            ],
            channel_combiner,
        ),
        spec(
            "ec.channel.minimax",
            "Minimax",
            vec![
                p("operation", "Operation", Value::Enum(1), popup(&["Minimum", "Maximum", "Minimum then Maximum", "Maximum then Minimum"])),
                p("radius", "Radius", num(0.0), slider(0.0, 500.0, 0.0, 100.0, 0)),
                p("channel", "Channel", Value::Enum(0), popup(&["Color", "Alpha", "Alpha and Color", "Red", "Green", "Blue"])),
                p("direction", "Direction", Value::Enum(0), popup(&["Horizontal & Vertical", "Horizontal Only", "Vertical Only"])),
            ],
            minimax,
        ),
        spec(
            "ec.channel.arithmetic",
            "Arithmetic",
            vec![
                p(
                    "operator",
                    "Operator",
                    Value::Enum(3),
                    popup(&["And", "Or", "Xor", "Add", "Subtract", "Difference", "Min", "Max", "Block Above", "Block Below", "Slice", "Multiply", "Screen"]),
                ),
                p("redValue", "Red Value", num(0.0), v255()),
                p("greenValue", "Green Value", num(0.0), v255()),
                p("blueValue", "Blue Value", num(0.0), v255()),
                p("clip", "Clip Result Values", Value::Bool(true), ParamUi::Checkbox),
            ],
            arithmetic,
        ),
        spec(
            "ec.channel.solidcomposite",
            "Solid Composite",
            vec![
                p("sourceOpacity", "Source Opacity", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("color", "Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("opacity", "Opacity", num(100.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("blendingMode", "Blending Mode", Value::Enum(0), popup(&SOLID_MODE_NAMES)),
            ],
            solid_composite,
        ),
        spec(
            "ec.channel.removecolormatting",
            "Remove Color Matting",
            vec![p("backgroundColor", "Background Color", col(0.0, 0.0, 0.0), ParamUi::Color), p("clipHdr", "Clipping", Value::Bool(true), ParamUi::Checkbox)],
            remove_color_matting,
        ),
        spec(
            "ec.channel.setmatte",
            "Set Matte",
            vec![
                p(
                    "takeMatteFrom",
                    "Use For Matte",
                    Value::Enum(3),
                    popup(&["Red", "Green", "Blue", "Alpha", "Luminance", "Hue", "Lightness", "Saturation", "Full", "Off"]),
                ),
                p("invertMatte", "Invert Matte", Value::Bool(false), ParamUi::Checkbox),
                p("compositeMatteWithOriginal", "Composite Matte with Original", Value::Bool(true), ParamUi::Checkbox),
                p("premultiplyMatteLayer", "Premultiply Matte Layer", Value::Bool(true), ParamUi::Checkbox),
            ],
            set_matte,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Params, apply, find};
    use effectcraft_raster::Image;

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

    fn sample() -> Image {
        let mut img = Image::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                let a = if x < 6 { 1.0 } else { 0.5 };
                img.set(x, y, [0.2 * a, 0.5 * a, 0.9 * a, a]);
            }
        }
        img
    }

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn set_channels_defaults_are_identity_and_swap_works() {
        let img = sample();
        assert_eq!(run("ec.channel.setchannels", &[], img.clone()), img);
        let out = run("ec.channel.setchannels", &[("setRedTo", Value::Enum(2)), ("setBlueTo", Value::Enum(0))], img);
        assert!(close(out.get(0, 0), [0.9, 0.5, 0.2, 1.0]));
    }

    #[test]
    fn shift_channels_takes_red_from_green() {
        let out = run("ec.channel.shiftchannels", &[("takeRedFrom", Value::Enum(2))], sample());
        assert!(close(out.get(7, 0), [0.25, 0.25, 0.45, 0.5]));
    }

    #[test]
    fn channel_combiner_hls_round_trip() {
        let img = sample();
        let hls = run("ec.channel.combiner", &[("from", Value::Enum(0))], img.clone());
        let back = run("ec.channel.combiner", &[("from", Value::Enum(1))], hls);
        assert!(close(back.get(0, 0), img.get(0, 0)), "{:?}", back.get(0, 0));
    }

    #[test]
    fn minimax_maximum_grows_bright_point() {
        let mut img = Image::filled(9, 9, [0.0, 0.0, 0.0, 1.0]);
        img.set(4, 4, [1.0, 1.0, 1.0, 1.0]);
        let out = run("ec.channel.minimax", &[("radius", num(2.0))], img);
        assert_eq!(out.get(2, 2)[0], 1.0);
        assert_eq!(out.get(1, 4)[0], 0.0);
        let n: usize = out.data.iter().filter(|p| p[0] > 0.5).count();
        assert_eq!(n, 25);
    }

    #[test]
    fn arithmetic_add_and_slice() {
        let img = Image::filled(1, 1, [0.4, 0.4, 0.4, 1.0]);
        let out = run("ec.channel.arithmetic", &[("redValue", num(51.0))], img.clone());
        assert!((out.get(0, 0)[0] - 0.6).abs() < 1e-5 && (out.get(0, 0)[1] - 0.4).abs() < 1e-6);
        let out = run("ec.channel.arithmetic", &[("operator", Value::Enum(10)), ("redValue", num(200.0)), ("greenValue", num(10.0))], img);
        assert_eq!(&out.get(0, 0)[..3], &[0.0, 1.0, 1.0]);
    }

    #[test]
    fn remove_color_matting_inverts_matting() {
        // Foreground colour f, alpha a, matted against bg: stored straight value = f·a + bg·(1−a).
        let (f, a, bg) = ([0.8f32, 0.3, 0.1], 0.4f32, [0.2f32, 0.6, 0.9]);
        let s: Vec<f32> = (0..3).map(|i| f[i] * a + bg[i] * (1.0 - a)).collect();
        let img = Image::filled(1, 1, [s[0] * a, s[1] * a, s[2] * a, a]);
        let out = run("ec.channel.removecolormatting", &[("backgroundColor", Value::Color([0.2, 0.6, 0.9, 1.0]))], img);
        let px = out.get(0, 0);
        for i in 0..3 {
            assert!((px[i] - f[i] * a).abs() < 1e-5, "{px:?}");
        }
    }

    #[test]
    fn set_matte_from_luminance() {
        let mut img = Image::new(2, 1);
        img.set(0, 0, [1.0, 1.0, 1.0, 1.0]);
        img.set(1, 0, [0.0, 0.0, 0.0, 1.0]);
        let out = run("ec.channel.setmatte", &[("takeMatteFrom", Value::Enum(4))], img);
        assert!((out.get(0, 0)[3] - 1.0).abs() < 1e-5);
        assert_eq!(out.get(1, 0)[3], 0.0);
    }

    #[test]
    fn solid_composite_fills_behind() {
        let mut img = Image::new(2, 1);
        img.set(0, 0, [0.0, 0.0, 1.0, 1.0]);
        let out = run("ec.channel.solidcomposite", &[("color", Value::Color([1.0, 0.0, 0.0, 1.0]))], img);
        assert!(close(out.get(0, 0), [0.0, 0.0, 1.0, 1.0]));
        assert!(close(out.get(1, 0), [1.0, 0.0, 0.0, 1.0]));
    }
}
