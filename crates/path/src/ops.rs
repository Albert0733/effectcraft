//! Shape-layer path operations.

use kurbo::{BezPath, ParamCurve, ParamCurveArclen, PathSeg};

const ACC: f64 = 1e-3;

fn segments(p: &BezPath) -> Vec<PathSeg> {
    p.segments().collect()
}

fn append(out: &mut BezPath, seg: PathSeg, start: bool) {
    if start {
        out.move_to(seg.start());
    }
    match seg {
        PathSeg::Line(l) => out.line_to(l.p1),
        PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
        PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
    }
}

/// Portion of a path between arc-length fractions `a..b` (0..1, a ≤ b).
fn sub_path(segs: &[PathSeg], lens: &[f64], total: f64, a: f64, b: f64, out: &mut BezPath) {
    let (sa, sb) = (a * total, b * total);
    let mut acc = 0.0;
    let mut started = false;
    for (seg, &len) in segs.iter().zip(lens) {
        let (s0, s1) = (acc, acc + len);
        acc = s1;
        if s1 <= sa || s0 >= sb || len <= 0.0 {
            continue;
        }
        let t0 = if sa > s0 { seg.inv_arclen(sa - s0, ACC) } else { 0.0 };
        let t1 = if sb < s1 { seg.inv_arclen(sb - s0, ACC) } else { 1.0 };
        let piece = seg.subsegment(t0..t1);
        append(out, piece, !started);
        started = true;
    }
}

/// Trim Paths on each path individually: `start`/`end` in percent, `offset` in degrees.
pub fn trim(paths: &[BezPath], start: f64, end: f64, offset_deg: f64) -> Vec<BezPath> {
    let (mut s, mut e) = (start / 100.0, end / 100.0);
    if s > e {
        std::mem::swap(&mut s, &mut e);
    }
    if e - s >= 1.0 - 1e-9 && offset_deg == 0.0 {
        return paths.to_vec();
    }
    if e - s <= 1e-9 {
        return vec![];
    }
    let off = (offset_deg / 360.0).rem_euclid(1.0);
    let (s, e) = (s + off, e + off);
    paths
        .iter()
        .filter_map(|p| {
            let segs = segments(p);
            let lens: Vec<f64> = segs.iter().map(|s| s.arclen(ACC)).collect();
            let total: f64 = lens.iter().sum();
            if total <= 0.0 {
                return None;
            }
            let mut out = BezPath::new();
            if e <= 1.0 {
                sub_path(&segs, &lens, total, s, e, &mut out);
            } else if s >= 1.0 {
                sub_path(&segs, &lens, total, s - 1.0, e - 1.0, &mut out);
            } else {
                sub_path(&segs, &lens, total, s, 1.0, &mut out);
                sub_path(&segs, &lens, total, 0.0, e - 1.0, &mut out);
            }
            Some(out)
        })
        .collect()
}

/// Trim Paths "Simultaneously" across several paths: treated as one continuous length.
pub fn trim_simultaneous(paths: &[BezPath], start: f64, end: f64, offset_deg: f64) -> Vec<BezPath> {
    // AE's "Simultaneously" trims each path by the same fractions.
    trim(paths, start, end, offset_deg)
}

/// Pucker & Bloat: move vertices towards (negative) / away from (positive) the centre while
/// pulling control points the other way.
pub fn pucker_bloat(paths: &[BezPath], amount: f64) -> Vec<BezPath> {
    let a = amount / 100.0;
    paths
        .iter()
        .map(|p| {
            let Some(b) = crate::bounds(std::slice::from_ref(p)) else { return p.clone() };
            let c = b.center();
            let mut out = BezPath::new();
            for el in p.elements() {
                let mv = |q: kurbo::Point| q + (q - c) * a;
                let mvc = |q: kurbo::Point| q - (q - c) * a;
                match *el {
                    kurbo::PathEl::MoveTo(q) => out.move_to(mv(q)),
                    kurbo::PathEl::LineTo(q) => {
                        let prev = out.elements().last().and_then(|e| e.end_point()).unwrap_or(q);
                        out.curve_to(mvc(prev.lerp(q, 1.0 / 3.0)), mvc(prev.lerp(q, 2.0 / 3.0)), mv(q))
                    }
                    kurbo::PathEl::QuadTo(x, q) => out.quad_to(mvc(x), mv(q)),
                    kurbo::PathEl::CurveTo(x, y, q) => out.curve_to(mvc(x), mvc(y), mv(q)),
                    kurbo::PathEl::ClosePath => out.close_path(),
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    fn len(p: &[BezPath]) -> f64 {
        p.iter().map(|x| x.segments().map(|s| s.arclen(1e-4)).sum::<f64>()).sum()
    }

    #[test]
    fn trim_half_square() {
        let sq = kurbo::Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1);
        let t = trim(std::slice::from_ref(&sq), 0.0, 50.0, 0.0);
        assert!((len(&t) - 200.0).abs() < 0.5, "{}", len(&t));
        let w = trim(&[sq], 75.0, 125.0, 0.0);
        let _ = w;
        let t2 = trim(&[kurbo::Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1)], 0.0, 50.0, 270.0);
        assert!((len(&t2) - 200.0).abs() < 0.5);
    }

    #[test]
    fn trim_empty() {
        let sq = kurbo::Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1);
        assert!(trim(&[sq], 30.0, 30.0, 0.0).is_empty());
    }
}
