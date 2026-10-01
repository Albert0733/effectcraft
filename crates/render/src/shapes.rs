//! Shape-layer contents: groups, parametric paths, fills, strokes, gradients and path operators,
//! with the After Effects stacking semantics (operators and paint items act on the paths above
//! them in the same group; items higher in the list draw on top).

use effectcraft_effects::Buf;
use effectcraft_geom::{Mat3, vec2};
use effectcraft_keyframe::{Gradient, Value};
use effectcraft_path::{BezPath, Cap, FillRule, Join, StrokeStyle};
use effectcraft_project::{Layer, Node, PropGroup};
use effectcraft_raster::{Image, Mask};
use rayon::prelude::*;

use crate::eval::EvalCtx;

#[derive(Clone, Debug)]
enum Paint {
    Fill { color: [f32; 4], rule: FillRule },
    Stroke { color: [f32; 4], style: StrokeStyle },
    Gradient { g: Gradient, radial: bool, start: [f64; 2], end: [f64; 2], rule: FillRule },
}

#[derive(Clone, Debug)]
struct Draw {
    paint: Paint,
    paths: Vec<BezPath>,
    /// Group-space → layer-space for gradients and stroke scaling.
    xf: Mat3,
    opacity: f32,
}

fn mat_paths(paths: &[BezPath], m: &Mat3) -> Vec<BezPath> {
    effectcraft_path::transform(paths, m)
}

fn group_matrix(ctx: &EvalCtx, layer: &Layer, tr: &PropGroup) -> (Mat3, f32) {
    let anchor = ctx.v2(layer, tr, "anchor", [0.0; 2]);
    let pos = ctx.v2(layer, tr, "position", [0.0; 2]);
    let scale = ctx.v2(layer, tr, "scale", [100.0; 2]);
    let rot = ctx.f(layer, tr, "rotation", 0.0);
    let skew = ctx.f(layer, tr, "skew", 0.0);
    let skew_axis = ctx.f(layer, tr, "skewAxis", 0.0);
    let m = Mat3::translate(vec2(pos[0], pos[1]))
        * Mat3::rotate_deg(rot)
        * Mat3::skew_deg(-skew, skew_axis)
        * Mat3::scale(vec2(scale[0] / 100.0, scale[1] / 100.0))
        * Mat3::translate(vec2(-anchor[0], -anchor[1]));
    (m, (ctx.f(layer, tr, "opacity", 100.0) / 100.0) as f32)
}

fn rule(ctx: &EvalCtx, layer: &Layer, g: &PropGroup) -> FillRule {
    if ctx.e(layer, g, "rule") == 1 { FillRule::EvenOdd } else { FillRule::NonZero }
}

/// Collect draws and accumulated paths of a contents group (in group space).
fn collect(ctx: &EvalCtx, layer: &Layer, contents: &PropGroup) -> (Vec<Draw>, Vec<BezPath>) {
    let mut draws: Vec<Draw> = Vec::new();
    let mut acc: Vec<BezPath> = Vec::new();
    for node in &contents.children {
        let Node::Group(g) = node else { continue };
        if !g.enabled {
            continue;
        }
        match g.match_id.as_str() {
            "rect" => {
                let size = ctx.v2(layer, g, "size", [100.0; 2]);
                let pos = ctx.v2(layer, g, "position", [0.0; 2]);
                acc.push(effectcraft_path::rect(size, pos, ctx.f(layer, g, "roundness", 0.0)));
            }
            "ellipse" => {
                let size = ctx.v2(layer, g, "size", [100.0; 2]);
                let pos = ctx.v2(layer, g, "position", [0.0; 2]);
                acc.push(effectcraft_path::ellipse(size, pos));
            }
            "star" => {
                let star = ctx.e(layer, g, "type") == 0;
                acc.push(effectcraft_path::polystar(
                    star,
                    ctx.f(layer, g, "points", 5.0),
                    ctx.v2(layer, g, "position", [0.0; 2]),
                    ctx.f(layer, g, "rotation", 0.0),
                    ctx.f(layer, g, "innerRadius", 50.0),
                    ctx.f(layer, g, "outerRadius", 100.0),
                    ctx.f(layer, g, "innerRoundness", 0.0),
                    ctx.f(layer, g, "outerRoundness", 0.0),
                ));
            }
            "path" => {
                if let Some(Value::Path(p)) = ctx.group_value(layer, g, "path") {
                    acc.push(effectcraft_path::to_kurbo(&p));
                }
            }
            "fill" => {
                let mut c = ctx.color(layer, g, "color");
                c[3] = 1.0;
                draws.push(Draw { paint: Paint::Fill { color: c, rule: rule(ctx, layer, g) }, paths: acc.clone(), xf: Mat3::IDENTITY, opacity: (ctx.f(layer, g, "opacity", 100.0) / 100.0) as f32 });
            }
            "stroke" => {
                let mut c = ctx.color(layer, g, "color");
                c[3] = 1.0;
                let dash = g.sub("dashes").and_then(|d| {
                    let dash = ctx.f(layer, d, "dash", 0.0);
                    let gap = ctx.f(layer, d, "gap", 0.0);
                    (dash > 0.0).then(|| (vec![dash, if gap > 0.0 { gap } else { dash }], ctx.f(layer, d, "offset", 0.0)))
                });
                let style = StrokeStyle {
                    width: ctx.f(layer, g, "width", 2.0),
                    cap: [Cap::Butt, Cap::Round, Cap::Square][ctx.e(layer, g, "cap").min(2) as usize],
                    join: [Join::Miter, Join::Round, Join::Bevel][ctx.e(layer, g, "join").min(2) as usize],
                    miter: ctx.f(layer, g, "miter", 4.0),
                    dash,
                };
                draws.push(Draw { paint: Paint::Stroke { color: c, style }, paths: acc.clone(), xf: Mat3::IDENTITY, opacity: (ctx.f(layer, g, "opacity", 100.0) / 100.0) as f32 });
            }
            "gfill" => {
                let gr = match ctx.group_value(layer, g, "colors") {
                    Some(Value::Gradient(gr)) => gr,
                    _ => Gradient::default(),
                };
                draws.push(Draw {
                    paint: Paint::Gradient {
                        g: gr,
                        radial: ctx.e(layer, g, "type") == 1,
                        start: ctx.v2(layer, g, "start", [0.0; 2]),
                        end: ctx.v2(layer, g, "end", [100.0, 0.0]),
                        rule: rule(ctx, layer, g),
                    },
                    paths: acc.clone(),
                    xf: Mat3::IDENTITY,
                    opacity: (ctx.f(layer, g, "opacity", 100.0) / 100.0) as f32,
                });
            }
            "trim" => {
                let (s, e, o) = (ctx.f(layer, g, "start", 0.0), ctx.f(layer, g, "end", 100.0), ctx.f(layer, g, "offset", 0.0));
                acc = effectcraft_path::ops::trim(&acc, s, e, o);
                for d in &mut draws {
                    // Paint items above the trim are not affected (AE semantics).
                    let _ = d;
                }
            }
            "pucker" => {
                acc = effectcraft_path::ops::pucker_bloat(&acc, ctx.f(layer, g, "amount", 0.0));
            }
            "group" => {
                let Some(inner) = g.sub("contents") else { continue };
                let (cd, cp) = collect(ctx, layer, inner);
                let (m, op) = g.sub("transform").map(|t| group_matrix(ctx, layer, t)).unwrap_or((Mat3::IDENTITY, 1.0));
                for mut d in cd {
                    d.paths = mat_paths(&d.paths, &m);
                    d.xf = m * d.xf;
                    d.opacity *= op;
                    draws.push(d);
                }
                acc.extend(mat_paths(&cp, &m));
            }
            "repeater" => {
                let copies = ctx.f(layer, g, "copies", 3.0).max(0.0);
                let offset = ctx.f(layer, g, "offset", 0.0);
                let n = copies.ceil() as usize;
                let Some(tr) = g.sub("transform") else { continue };
                let anchor = ctx.v2(layer, tr, "anchor", [0.0; 2]);
                let pos = ctx.v2(layer, tr, "position", [100.0, 0.0]);
                let scale = ctx.v2(layer, tr, "scale", [100.0; 2]);
                let rot = ctx.f(layer, tr, "rotation", 0.0);
                let so = (ctx.f(layer, tr, "startOpacity", 100.0) / 100.0) as f32;
                let eo = (ctx.f(layer, tr, "endOpacity", 100.0) / 100.0) as f32;
                let step = |k: f64| {
                    Mat3::translate(vec2(pos[0] * k, pos[1] * k))
                        * Mat3::translate(vec2(anchor[0], anchor[1]))
                        * Mat3::rotate_deg(rot * k)
                        * Mat3::scale(vec2((scale[0] / 100.0).powf(k), (scale[1] / 100.0).powf(k)))
                        * Mat3::translate(vec2(-anchor[0], -anchor[1]))
                };
                let base_draws = std::mem::take(&mut draws);
                let base_acc = std::mem::take(&mut acc);
                let above = ctx.e(layer, g, "composite") == 1;
                let mut order: Vec<usize> = (0..n).collect();
                if !above {
                    order.reverse();
                }
                for i in order {
                    let k = i as f64 + offset;
                    let m = step(k);
                    let t = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
                    let opk = so + (eo - so) * t;
                    let partial = if i + 1 == n && copies.fract() > 0.0 { copies.fract() as f32 } else { 1.0 };
                    for d in &base_draws {
                        let mut d = d.clone();
                        d.paths = mat_paths(&d.paths, &m);
                        d.xf = m * d.xf;
                        d.opacity *= opk * partial;
                        draws.push(d);
                    }
                    acc.extend(mat_paths(&base_acc, &m));
                }
            }
            _ => {}
        }
    }
    (draws, acc)
}

fn composite_coverage(img: &mut Image, cov: &Mask, color: [f32; 4], opacity: f32) {
    img.data.par_iter_mut().zip(cov.data.par_iter()).for_each(|(px, &c)| {
        let a = c * opacity * color[3];
        if a <= 0.0 {
            return;
        }
        let k = 1.0 - a;
        px[0] = color[0] * a + px[0] * k;
        px[1] = color[1] * a + px[1] * k;
        px[2] = color[2] * a + px[2] * k;
        px[3] = a + px[3] * k;
    });
}

/// Render a shape layer's contents into a layer-space buffer at scale `s`.
pub fn render(ctx: &EvalCtx, layer: &Layer, contents: &PropGroup, s: f64) -> Buf {
    let (draws, _) = collect(ctx, layer, contents);
    let mut bounds: Option<kurbo::Rect> = None;
    for d in &draws {
        if let Some(b) = effectcraft_path::bounds(&d.paths) {
            let grow = match &d.paint {
                Paint::Stroke { style, .. } => style.width * style.miter.max(1.0) * 0.5 * d.xf.mean_scale().max(1.0) + 1.0,
                _ => 1.0,
            };
            let b = b.inflate(grow, grow);
            bounds = Some(bounds.map_or(b, |a| a.union(b)));
        }
    }
    let Some(b) = bounds else { return Buf { img: Image::new(1, 1), offset: [0.0, 0.0], scale: s } };
    // Keep buffers sane for runaway geometry.
    let b = b.intersect(kurbo::Rect::new(-20000.0, -20000.0, 20000.0, 20000.0));
    let w = ((b.width() * s).ceil() as u32 + 4).clamp(1, 16384);
    let h = ((b.height() * s).ceil() as u32 + 4).clamp(1, 16384);
    let offset = [-b.x0 * s + 2.0, -b.y0 * s + 2.0];
    let m = Mat3::translate(vec2(offset[0], offset[1])) * Mat3::scale(vec2(s, s));
    let mut img = Image::new(w, h);
    for d in draws.iter().rev() {
        if d.opacity <= 0.0 || d.paths.is_empty() {
            continue;
        }
        match &d.paint {
            Paint::Fill { color, rule } => {
                let cov = effectcraft_path::fill_coverage(&d.paths, &m, w, h, *rule);
                composite_coverage(&mut img, &cov, *color, d.opacity);
            }
            Paint::Stroke { color, style } => {
                // Strokes scale with their group transform.
                let mut st = style.clone();
                st.width *= d.xf.mean_scale();
                let cov = effectcraft_path::stroke_coverage(&d.paths, &st, &m, w, h);
                composite_coverage(&mut img, &cov, *color, d.opacity);
            }
            Paint::Gradient { g, radial, start, end, rule } => {
                let cov = effectcraft_path::fill_coverage(&d.paths, &m, w, h, *rule);
                let to_local = (m * d.xf).inverse().unwrap_or(Mat3::IDENTITY);
                let (sx, sy) = (start[0], start[1]);
                let (dx, dy) = (end[0] - sx, end[1] - sy);
                let len2 = (dx * dx + dy * dy).max(1e-9);
                let op = d.opacity;
                img.data.par_chunks_mut(w as usize).zip(cov.data.par_chunks(w as usize)).enumerate().for_each(|(y, (row, crow))| {
                    for x in 0..row.len() {
                        let c = crow[x];
                        if c <= 0.0 {
                            continue;
                        }
                        let p = to_local.apply(vec2(x as f64 + 0.5, y as f64 + 0.5));
                        let (vx, vy) = (p.x - sx, p.y - sy);
                        let t = if *radial { ((vx * vx + vy * vy) / len2).sqrt() } else { (vx * dx + vy * dy) / len2 };
                        let col = g.sample(t);
                        let a = c * op * col[3];
                        let k = 1.0 - a;
                        let px = &mut row[x];
                        px[0] = col[0] * a + px[0] * k;
                        px[1] = col[1] * a + px[1] * k;
                        px[2] = col[2] * a + px[2] * k;
                        px[3] = a + px[3] * k;
                    }
                });
            }
        }
    }
    Buf { img, offset, scale: s }
}
