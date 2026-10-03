//! Rasterisation of a parsed SVG into a premultiplied `f32` image (anti-aliased coverage from
//! `effectcraft-path`; group opacity composites the group as a whole, as SVG specifies).

use effectcraft_geom::Mat3;
use effectcraft_raster::{Image, Mask};
use kurbo::{Affine, Point, Shape as _};
use rayon::prelude::*;

use crate::{Doc, FillRule, Gradient, GradientKind, Group, Node, Paint, Shape, Spread};

fn mat3(a: Affine) -> Mat3 {
    let c = a.as_coeffs();
    Mat3([[c[0], c[2], c[4]], [c[1], c[3], c[5]], [0.0, 0.0, 1.0]])
}

/// Rasterise `doc` into a `w`×`h` image, scaled by `scale` (1 = the document's pixel size).
pub fn rasterize(doc: &Doc, w: u32, h: u32, scale: f64) -> Image {
    rasterize_with(doc, w, h, Affine::scale(scale))
}

/// Rasterise `doc` with `m` mapping document pixels to image pixels.
pub fn rasterize_with(doc: &Doc, w: u32, h: u32, m: Affine) -> Image {
    let mut img = Image::new(w, h);
    draw_group(&doc.root, m, 1.0, &mut img);
    img
}

fn draw_group(g: &Group, m: Affine, opacity: f64, dst: &mut Image) {
    let m = m * g.transform;
    let o = opacity * g.opacity;
    if o <= 0.0 {
        return;
    }
    if g.opacity < 1.0 && g.children.len() > 1 {
        let mut tmp = Image::new(dst.width, dst.height);
        for c in &g.children {
            draw_node(c, m, 1.0, &mut tmp);
        }
        over(dst, &tmp, o as f32);
    } else {
        for c in &g.children {
            draw_node(c, m, o, dst);
        }
    }
}

fn draw_node(n: &Node, m: Affine, opacity: f64, dst: &mut Image) {
    match n {
        Node::Group(g) => draw_group(g, m, opacity, dst),
        Node::Shape(s) => {
            if s.opacity < 1.0 && s.fill.is_some() && s.stroke.is_some() {
                let mut tmp = Image::new(dst.width, dst.height);
                draw_shape(s, m, 1.0, &mut tmp);
                over(dst, &tmp, (opacity * s.opacity) as f32);
            } else {
                draw_shape(s, m, opacity * s.opacity, dst);
            }
        }
    }
}

/// `dst = src·k over dst` (premultiplied).
fn over(dst: &mut Image, src: &Image, k: f32) {
    dst.data.par_iter_mut().zip(src.data.par_iter()).for_each(|(d, s)| {
        let a = s[3] * k;
        if a > 0.0 {
            for c in 0..3 {
                d[c] = s[c] * k + d[c] * (1.0 - a);
            }
            d[3] = a + d[3] * (1.0 - a);
        }
    });
}

fn draw_shape(s: &Shape, m: Affine, opacity: f64, dst: &mut Image) {
    let m = m * s.transform;
    let path = s.geom.to_path();
    let bbox = path.bounding_box();
    let (w, h) = (dst.width, dst.height);
    if let Some(fill) = &s.fill {
        let rule = match s.fill_rule {
            FillRule::NonZero => effectcraft_path::FillRule::NonZero,
            FillRule::EvenOdd => effectcraft_path::FillRule::EvenOdd,
        };
        let cov = effectcraft_path::fill_coverage(std::slice::from_ref(&path), &mat3(m), w, h, rule);
        paint(dst, &cov, fill, (opacity * s.fill_opacity) as f32, m, bbox);
    }
    if let Some(st) = &s.stroke {
        let style = effectcraft_path::StrokeStyle {
            width: st.width,
            cap: match st.cap {
                crate::Cap::Butt => effectcraft_path::Cap::Butt,
                crate::Cap::Round => effectcraft_path::Cap::Round,
                crate::Cap::Square => effectcraft_path::Cap::Square,
            },
            join: match st.join {
                crate::Join::Miter => effectcraft_path::Join::Miter,
                crate::Join::Round => effectcraft_path::Join::Round,
                crate::Join::Bevel => effectcraft_path::Join::Bevel,
            },
            miter: st.miter,
            dash: st.dash.clone(),
        };
        let cov = effectcraft_path::stroke_coverage(std::slice::from_ref(&path), &style, &mat3(m), w, h);
        paint(dst, &cov, &st.paint, (opacity * st.opacity) as f32, m, bbox);
    }
}

/// Composite paint through coverage.
fn paint(dst: &mut Image, cov: &Mask, p: &Paint, opacity: f32, m: Affine, bbox: kurbo::Rect) {
    let w = dst.width as usize;
    if w == 0 {
        return;
    }
    let shader = match p {
        Paint::Color(c) => Shader::Solid([c[0] as f32, c[1] as f32, c[2] as f32, 1.0]),
        Paint::Gradient(g) => {
            let unit = if g.bbox_units { Affine::new([bbox.width(), 0.0, 0.0, bbox.height(), bbox.x0, bbox.y0]) } else { Affine::IDENTITY };
            let to_grad = (m * unit * g.transform).inverse();
            if !to_grad.as_coeffs().iter().all(|v| v.is_finite()) {
                return;
            }
            Shader::Gradient(g, to_grad)
        }
    };
    dst.data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, d) in row.iter_mut().enumerate() {
            let c = cov.data[y * w + x];
            if c <= 0.0 {
                continue;
            }
            let col = shader.at(x as f64 + 0.5, y as f64 + 0.5);
            let a = col[3] * c * opacity;
            if a <= 0.0 {
                continue;
            }
            for k in 0..3 {
                d[k] = col[k] * a + d[k] * (1.0 - a);
            }
            d[3] = a + d[3] * (1.0 - a);
        }
    });
}

enum Shader<'a> {
    Solid([f32; 4]),
    Gradient(&'a Gradient, Affine),
}

impl Shader<'_> {
    /// Straight RGBA at a pixel centre.
    fn at(&self, x: f64, y: f64) -> [f32; 4] {
        match self {
            Shader::Solid(c) => *c,
            Shader::Gradient(g, inv) => {
                let p = *inv * Point::new(x, y);
                let t = match g.kind {
                    GradientKind::Linear { x1, y1, x2, y2 } => {
                        let (dx, dy) = (x2 - x1, y2 - y1);
                        let l2 = dx * dx + dy * dy;
                        if l2 <= 0.0 { 1.0 } else { ((p.x - x1) * dx + (p.y - y1) * dy) / l2 }
                    }
                    GradientKind::Radial { cx, cy, r, fx, fy } => radial_t(p, cx, cy, r, fx, fy),
                };
                eval_stops(&g.stops, spread(t, g.spread))
            }
        }
    }
}

/// Radial gradient parameter with a focal point: the circle through `p` from the focus.
pub(crate) fn radial_t(p: Point, cx: f64, cy: f64, r: f64, fx: f64, fy: f64) -> f64 {
    if r <= 0.0 {
        return 1.0;
    }
    let (dx, dy) = (p.x - fx, p.y - fy);
    let (ex, ey) = (fx - cx, fy - cy);
    if ex * ex + ey * ey < 1e-12 {
        return (dx * dx + dy * dy).sqrt() / r;
    }
    // Solve |e + u·d|² = r² for u > 0; t = 1/u.
    let a = dx * dx + dy * dy;
    if a < 1e-18 {
        return 0.0;
    }
    let b = ex * dx + ey * dy;
    let c = ex * ex + ey * ey - r * r;
    let disc = b * b - a * c;
    if disc < 0.0 {
        return 1.0;
    }
    let u = (-b + disc.sqrt()) / a;
    if u <= 0.0 { 1.0 } else { 1.0 / u }
}

fn spread(t: f64, s: Spread) -> f64 {
    match s {
        Spread::Pad => t.clamp(0.0, 1.0),
        Spread::Repeat => t.rem_euclid(1.0),
        Spread::Reflect => {
            let r = t.rem_euclid(2.0);
            if r > 1.0 { 2.0 - r } else { r }
        }
    }
}

pub(crate) fn eval_stops(stops: &[(f64, [f64; 4])], t: f64) -> [f32; 4] {
    let Some(first) = stops.first() else { return [0.0; 4] };
    if t <= first.0 {
        return first.1.map(|v| v as f32);
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.0 {
            let k = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 1.0 };
            return [0, 1, 2, 3].map(|i| (a.1[i] + (b.1[i] - a.1[i]) * k) as f32);
        }
    }
    stops.last().map_or([0.0; 4], |s| s.1.map(|v| v as f32))
}
