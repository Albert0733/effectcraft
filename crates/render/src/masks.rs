//! Layer masks: path coverage with expansion, feather, opacity, invert, combined by mode.

use effectcraft_effects::Buf;
use effectcraft_geom::{Mat3, vec2};
use effectcraft_keyframe::Value;
use effectcraft_path::{FillRule, StrokeStyle};
use effectcraft_project::{GroupKind, Layer, MaskMode};
use effectcraft_raster::{Mask, gaussian_blur};
use rayon::prelude::*;

use crate::eval::EvalCtx;

/// Coverage of one mask in buffer pixels (before mode/opacity/invert).
fn coverage(ctx: &EvalCtx, layer: &Layer, g: &effectcraft_project::PropGroup, buf: &Buf) -> Option<Mask> {
    let Value::Path(sp) = ctx.group_value(layer, g, "path")? else { return None };
    let path = effectcraft_path::to_kurbo(&sp);
    let (w, h) = (buf.img.width, buf.img.height);
    let m = Mat3::translate(vec2(buf.offset[0], buf.offset[1])) * Mat3::scale(vec2(buf.scale, buf.scale));
    let mut cov = effectcraft_path::fill_coverage(std::slice::from_ref(&path), &m, w, h, FillRule::NonZero);
    let exp = ctx.f(layer, g, "expansion", 0.0);
    if exp.abs() > 0.01 {
        let ring = effectcraft_path::stroke_coverage(
            std::slice::from_ref(&path),
            &StrokeStyle { width: exp.abs() * 2.0, join: effectcraft_path::Join::Round, ..Default::default() },
            &m,
            w,
            h,
        );
        cov.data.par_iter_mut().zip(ring.data.par_iter()).for_each(|(c, r)| {
            *c = if exp > 0.0 { (*c + r - *c * r).min(1.0) } else { (*c * (1.0 - r)).max(0.0) };
        });
    }
    let feather = ctx.v2(layer, g, "feather", [0.0; 2]);
    if feather[0] > 0.0 || feather[1] > 0.0 {
        let img = cov.to_image();
        let b = gaussian_blur(&img, feather[0] * buf.scale / 2.0, feather[1] * buf.scale / 2.0, false);
        cov = Mask::from_alpha(&b);
    }
    Some(cov)
}

/// The layer's enabled masks at the context time, flattened to polylines in layer space (for
/// effects that use masks as paths: Stroke, Scribble, Inner/Outer Key, Reshape…).
pub fn shapes(ctx: &EvalCtx, layer: &Layer) -> Vec<effectcraft_effects::MaskShape> {
    let Some(masks) = layer.masks() else { return Vec::new() };
    let mut out = Vec::new();
    for g in masks.groups().filter(|g| g.enabled) {
        let GroupKind::Mask { inverted, .. } = g.kind else { continue };
        let Some(Value::Path(sp)) = ctx.group_value(layer, g, "path") else { continue };
        let path = effectcraft_path::to_kurbo(&sp);
        let mut pts: Vec<[f64; 2]> = Vec::new();
        let mut closed = false;
        kurbo::flatten(path.iter(), 0.25, |el| match el {
            effectcraft_path::PathEl::MoveTo(p) | effectcraft_path::PathEl::LineTo(p) => {
                if pts.last() != Some(&[p.x, p.y]) {
                    pts.push([p.x, p.y]);
                }
            }
            effectcraft_path::PathEl::ClosePath => closed = true,
            _ => {}
        });
        if closed && pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        out.push(effectcraft_effects::MaskShape { name: g.name.clone(), points: pts, closed: closed || sp.closed, inverted });
    }
    out
}

/// Apply the layer's masks to its buffer.
pub fn apply(ctx: &EvalCtx, layer: &Layer, buf: &mut Buf) {
    let Some(masks) = layer.masks() else { return };
    let list: Vec<_> = masks.groups().filter(|g| g.enabled && matches!(g.kind, GroupKind::Mask { mode, .. } if mode != MaskMode::None)).collect();
    if list.is_empty() {
        return;
    }
    // Masks may extend past the source: grow the buffer to cover them when feather/expansion pads.
    let n = (buf.img.width * buf.img.height) as usize;
    let first_sub = matches!(list[0].kind, GroupKind::Mask { mode: MaskMode::Subtract, .. });
    let mut acc = vec![if first_sub { 1.0f32 } else { 0.0 }; n];
    for g in list {
        let GroupKind::Mask { mode, inverted, .. } = g.kind else { continue };
        let Some(mut cov) = coverage(ctx, layer, g, buf) else { continue };
        let op = (ctx.f(layer, g, "opacity", 100.0) / 100.0) as f32;
        cov.data.par_iter_mut().for_each(|c| {
            if inverted {
                *c = 1.0 - *c;
            }
            *c *= op;
        });
        acc.par_iter_mut().zip(cov.data.par_iter()).for_each(|(a, &c)| {
            *a = match mode {
                MaskMode::Add => *a + c - *a * c,
                MaskMode::Subtract => *a * (1.0 - c),
                MaskMode::Intersect => *a * c,
                MaskMode::Lighten => a.max(c),
                MaskMode::Darken => a.min(c),
                MaskMode::Difference => (*a - c).abs(),
                MaskMode::None => *a,
            };
        });
    }
    buf.img.mul_mask(&acc);
}
