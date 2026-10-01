//! Text layers: Source Text layout + text animators (range selectors) → per-character outlines.

use effectcraft_effects::Buf;
use effectcraft_geom::{Mat3, vec2};
use effectcraft_keyframe::{TextDoc, Value};
use effectcraft_path::{BezPath, FillRule, StrokeStyle};
use effectcraft_project::{Layer, PropGroup};
use effectcraft_raster::Image;
use effectcraft_text::{TextLayout, layout_doc};

use crate::eval::EvalCtx;

/// Evaluated per-character animation state.
#[derive(Clone, Copy, Debug)]
pub struct CharXf {
    pub offset: [f64; 2],
    pub anchor: [f64; 2],
    pub scale: [f64; 2],
    pub rotation: f64,
    pub skew: f64,
    pub opacity: f64,
    pub fill: Option<[f32; 4]>,
    pub fill_k: f32,
    pub stroke: Option<[f32; 4]>,
    pub stroke_k: f32,
    pub stroke_width: f64,
    pub tracking: f64,
}

impl Default for CharXf {
    fn default() -> Self {
        CharXf {
            offset: [0.0; 2],
            anchor: [0.0; 2],
            scale: [100.0; 2],
            rotation: 0.0,
            skew: 0.0,
            opacity: 100.0,
            fill: None,
            fill_k: 0.0,
            stroke: None,
            stroke_k: 0.0,
            stroke_width: 0.0,
            tracking: 0.0,
        }
    }
}

fn shape_value(shape: u32, f: f64) -> f64 {
    let f = f.clamp(0.0, 1.0);
    match shape {
        1 => f,                                      // ramp up
        2 => 1.0 - f,                                // ramp down
        3 => 1.0 - (2.0 * f - 1.0).abs(),            // triangle
        4 => (1.0 - (2.0 * f - 1.0).powi(2)).sqrt(), // round
        5 => {
            let t = 1.0 - (2.0 * f - 1.0).abs();
            t * t * (3.0 - 2.0 * t)
        } // smooth
        _ => 1.0,
    }
}

/// Selection amount (0..1) of unit `i` of `n` for a range selector.
fn range_amount(ctx: &EvalCtx, layer: &Layer, sel: &PropGroup, i: usize, n: usize) -> f64 {
    let adv = sel.sub("advanced");
    let units_index = adv.map(|a| ctx.e(layer, a, "units") == 1).unwrap_or(false);
    let shape = adv.map(|a| ctx.e(layer, a, "shape")).unwrap_or(0);
    let amount = adv.map(|a| ctx.f(layer, a, "amount", 100.0)).unwrap_or(100.0) / 100.0;
    let n = n.max(1) as f64;
    let (mut s, mut e, o) = if units_index {
        (ctx.f(layer, sel, "start", 0.0) / n, ctx.f(layer, sel, "end", n) / n, ctx.f(layer, sel, "offset", 0.0) / n)
    } else {
        (ctx.f(layer, sel, "start", 0.0) / 100.0, ctx.f(layer, sel, "end", 100.0) / 100.0, ctx.f(layer, sel, "offset", 0.0) / 100.0)
    };
    if s > e {
        std::mem::swap(&mut s, &mut e);
    }
    s += o;
    e += o;
    let (a, b) = (i as f64 / n, (i as f64 + 1.0) / n);
    let v = if shape == 0 {
        // Square: the fraction of this unit covered by the range (smooth partial selection).
        ((b.min(e) - a.max(s)) / (b - a)).clamp(0.0, 1.0)
    } else {
        if e - s <= 1e-9 {
            return 0.0;
        }
        let c = (a + b) * 0.5;
        if c < s || c > e { 0.0 } else { shape_value(shape, (c - s) / (e - s)) }
    };
    v * amount
}

/// Evaluate animators for every glyph.
pub fn char_transforms(ctx: &EvalCtx, layer: &Layer, text: &PropGroup, lay: &TextLayout) -> Vec<CharXf> {
    let mut out = vec![CharXf::default(); lay.glyphs.len()];
    let Some(anims) = text.sub("animators") else { return out };
    for anim in anims.groups().filter(|g| g.enabled) {
        let Some(props) = anim.sub("properties") else { continue };
        let sels: Vec<&PropGroup> = anim.sub("selectors").map(|s| s.groups().filter(|g| g.enabled).collect()).unwrap_or_default();
        let get = |m: &str| props.get(m).map(|p| ctx.value(layer, p));
        let pos = get("position").map(|v| v.as_vec2());
        let anchor = get("anchor").map(|v| v.as_vec2());
        let scale = get("scale").map(|v| v.as_vec2());
        let rot = get("rotation").map(|v| v.as_f64());
        let skew = get("skew").map(|v| v.as_f64());
        let op = get("opacity").map(|v| v.as_f64());
        let fill = get("fillColor").map(|v| v.as_color());
        let stroke = get("strokeColor").map(|v| v.as_color());
        let sw = get("strokeWidth").map(|v| v.as_f64());
        let tracking = get("tracking").map(|v| v.as_f64());
        for (gi, g) in lay.glyphs.iter().enumerate() {
            let mut k = 1.0;
            for sel in &sels {
                let based = sel.sub("advanced").map(|a| ctx.e(layer, a, "basedOn")).unwrap_or(0);
                let (i, n) = match based {
                    1 => {
                        if g.is_space {
                            k = 0.0;
                            continue;
                        }
                        (g.char_index_no_space, lay.chars_no_space)
                    }
                    2 => (g.word_index, lay.words),
                    3 => (g.line_index, lay.lines),
                    _ => (g.char_index, lay.chars),
                };
                k *= range_amount(ctx, layer, sel, i, n);
            }
            if k <= 0.0 {
                continue;
            }
            let c = &mut out[gi];
            if let Some(p) = pos {
                c.offset[0] += p[0] * k;
                c.offset[1] += p[1] * k;
            }
            if let Some(a) = anchor {
                c.anchor[0] += a[0] * k;
                c.anchor[1] += a[1] * k;
            }
            if let Some(s) = scale {
                c.scale[0] *= 1.0 + (s[0] / 100.0 - 1.0) * k;
                c.scale[1] *= 1.0 + (s[1] / 100.0 - 1.0) * k;
            }
            if let Some(r) = rot {
                c.rotation += r * k;
            }
            if let Some(s) = skew {
                c.skew += s * k;
            }
            if let Some(o) = op {
                c.opacity *= 1.0 + (o / 100.0 - 1.0) * k;
            }
            if let Some(f) = fill {
                c.fill = Some(f);
                c.fill_k = (c.fill_k + k as f32).min(1.0);
            }
            if let Some(f) = stroke {
                c.stroke = Some(f);
                c.stroke_k = (c.stroke_k + k as f32).min(1.0);
            }
            if let Some(w) = sw {
                c.stroke_width += w * k;
            }
            if let Some(t) = tracking {
                c.tracking += t * k;
            }
        }
    }
    out
}

/// The Source Text value at the current time.
pub fn source_text(ctx: &EvalCtx, layer: &Layer) -> Option<TextDoc> {
    let text = layer.props.sub("text")?;
    match ctx.group_value(layer, text, "sourceText")? {
        Value::Text(t) => Some(*t),
        Value::Str(s) => Some(TextDoc { text: s, ..Default::default() }),
        _ => None,
    }
}

/// Outlines of all glyphs in layer space with their paint (for text → shapes and hit testing).
pub fn glyph_paths(ctx: &EvalCtx, layer: &Layer) -> Vec<(BezPath, CharXf)> {
    let Some(doc) = source_text(ctx, layer) else { return vec![] };
    let Some(text) = layer.props.sub("text") else { return vec![] };
    let lay = layout_doc(&doc);
    let xfs = char_transforms(ctx, layer, text, &lay);
    let mut track = 0.0;
    let mut out = Vec::with_capacity(lay.glyphs.len());
    let mut last_line = usize::MAX;
    for (g, x) in lay.glyphs.iter().zip(&xfs) {
        if g.line_index != last_line {
            track = 0.0;
            last_line = g.line_index;
        }
        let pivot = [g.origin.x + g.advance / 2.0 + track, g.origin.y];
        track += x.tracking / 1000.0 * doc.size;
        if g.is_space || g.path.elements().is_empty() {
            continue;
        }
        let m = Mat3::translate(vec2(pivot[0] + x.offset[0], pivot[1] + x.offset[1]))
            * Mat3::rotate_deg(x.rotation)
            * Mat3::skew_deg(-x.skew, 0.0)
            * Mat3::scale(vec2(x.scale[0] / 100.0, x.scale[1] / 100.0))
            * Mat3::translate(vec2(-g.advance / 2.0 - x.anchor[0], -x.anchor[1]));
        out.push((effectcraft_path::transform(std::slice::from_ref(&g.path), &m).remove(0), *x));
    }
    out
}

fn lerp4(a: [f32; 4], b: [f32; 4], k: f32) -> [f32; 4] {
    [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k, a[3] + (b[3] - a[3]) * k]
}

/// Render a text layer into a layer-space buffer at scale `s`.
pub fn render(ctx: &EvalCtx, layer: &Layer, s: f64) -> Buf {
    let Some(doc) = source_text(ctx, layer) else { return Buf { img: Image::new(1, 1), offset: [0.0; 2], scale: s } };
    let glyphs = glyph_paths(ctx, layer);
    let stroke_w = if doc.apply_stroke { doc.stroke_width } else { 0.0 };
    let mut bounds: Option<kurbo::Rect> = None;
    for (p, x) in &glyphs {
        if let Some(b) = effectcraft_path::bounds(std::slice::from_ref(p)) {
            let b = b.inflate(stroke_w + x.stroke_width + 2.0, stroke_w + x.stroke_width + 2.0);
            bounds = Some(bounds.map_or(b, |a| a.union(b)));
        }
    }
    let Some(b) = bounds else { return Buf { img: Image::new(1, 1), offset: [0.0; 2], scale: s } };
    let b = b.intersect(kurbo::Rect::new(-20000.0, -20000.0, 20000.0, 20000.0));
    let w = ((b.width() * s).ceil() as u32 + 4).clamp(1, 16384);
    let h = ((b.height() * s).ceil() as u32 + 4).clamp(1, 16384);
    let offset = [-b.x0 * s + 2.0, -b.y0 * s + 2.0];
    let mut img = Image::new(w, h);
    let base = Mat3::translate(vec2(offset[0], offset[1])) * Mat3::scale(vec2(s, s));
    for (p, x) in &glyphs {
        let op = (x.opacity / 100.0).clamp(0.0, 1.0) as f32;
        if op <= 0.0 {
            continue;
        }
        let Some(gb) = effectcraft_path::bounds(std::slice::from_ref(p)) else { continue };
        let sw = stroke_w + x.stroke_width;
        let gb = gb.inflate(sw + 1.0, sw + 1.0);
        let x0 = ((gb.x0 * s + offset[0]).floor() as i64).max(0);
        let y0 = ((gb.y0 * s + offset[1]).floor() as i64).max(0);
        let x1 = ((gb.x1 * s + offset[0]).ceil() as i64).min(w as i64);
        let y1 = ((gb.y1 * s + offset[1]).ceil() as i64).min(h as i64);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let (gw, gh) = ((x1 - x0) as u32, (y1 - y0) as u32);
        let m = Mat3::translate(vec2(-x0 as f64, -y0 as f64)) * base;
        let fill_col = x.fill.map(|f| lerp4(doc.fill, f, x.fill_k)).unwrap_or(doc.fill);
        let stroke_col = x.stroke.map(|f| lerp4(doc.stroke, f, x.stroke_k)).unwrap_or(doc.stroke);
        let mut layers: Vec<(effectcraft_raster::Mask, [f32; 4])> = Vec::new();
        let fill_cov = doc.apply_fill.then(|| (effectcraft_path::fill_coverage(std::slice::from_ref(p), &m, gw, gh, FillRule::NonZero), fill_col));
        let stroke_cov = (sw > 0.0).then(|| {
            let st = StrokeStyle { width: sw, join: effectcraft_path::Join::Round, ..Default::default() };
            (effectcraft_path::stroke_coverage(std::slice::from_ref(p), &st, &m, gw, gh), stroke_col)
        });
        if doc.stroke_over_fill {
            layers.extend(fill_cov);
            layers.extend(stroke_cov);
        } else {
            layers.extend(stroke_cov);
            layers.extend(fill_cov);
        }
        for (cov, col) in layers {
            for yy in 0..gh as usize {
                let row = (y0 as usize + yy) * w as usize + x0 as usize;
                for xx in 0..gw as usize {
                    let c = cov.data[yy * gw as usize + xx];
                    if c <= 0.0 {
                        continue;
                    }
                    let a = c * op * col[3];
                    let px = &mut img.data[row + xx];
                    let k = 1.0 - a;
                    px[0] = col[0] * a + px[0] * k;
                    px[1] = col[1] * a + px[1] * k;
                    px[2] = col[2] * a + px[2] * k;
                    px[3] = a + px[3] * k;
                }
            }
        }
    }
    Buf { img, offset, scale: s }
}
