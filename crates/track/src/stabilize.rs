//! Video stabilization (the Warp Stabilizer's analysis and stabilization steps), clean-room from
//! the published approach of 2D stabilizers:
//!
//! 1. **Analysis** ([`Analyzer`]): Shi–Tomasi features are tracked between consecutive frames
//!    with pyramidal Lucas–Kanade ([`crate::klt`]), and the dominant (background) motion between
//!    each pair of frames is fitted robustly with RANSAC ([`crate::fit`]) for three models at
//!    once: translation, similarity and homography. The result ([`WarpAnalysis`]) is small and
//!    serialisable; it is stored in the effect instance.
//! 2. **Stabilization** ([`plan`]): the camera path is smoothed and per-frame corrective
//!    transforms are derived. For *Smooth Motion* each frame is warped to the Gaussian-weighted
//!    average of its neighbours' viewpoints — the motion-smoothing scheme of Matsushita et al.,
//!    "Full-Frame Video Stabilization with Motion Inpainting" (PAMI 2006) — so the low-frequency
//!    camera move stays and the jitter goes; *No Motion* maps every frame onto one reference
//!    frame. Framing then crops to the region valid on every frame (a centred rectangle inside
//!    every warped frame outline) and optionally scales it up to fill the frame; when that would
//!    exceed Maximum Scale, the correction is relaxed towards identity on the offending frames —
//!    trading smoothness for less crop, the constraint idea of Grundmann et al., "Auto-Directed
//!    Video Stabilization with Robust L1 Optimal Camera Paths" (CVPR 2011), applied here
//!    per frame rather than through a linear program.
//!
//! Coordinates are layer pixels; transforms map *source* frame pixels to *stabilized* pixels.

use serde::{Deserialize, Serialize};

use crate::Frame;
use crate::fit::{Model, ransac};
use crate::klt::{CornerOpts, GrayPyramid, LkOpts, analysis_factor, good_features, track};
use crate::solve::Homography;

/// Inter-frame motion of one frame (from the previous frame to this one) for each model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrameMotion {
    /// Translation `[tx, ty]`.
    pub t: [f64; 2],
    /// Similarity `[a, b, tx, ty]` (`x' = a x − b y + tx`, `y' = b x + a y + ty`).
    pub s: [f64; 4],
    /// Homography, row-major without `h33 = 1`.
    pub h: [f64; 8],
    /// Correspondences that agreed with the homography / tracked in total.
    pub inliers: u32,
    pub features: u32,
    /// A subset of this frame's tracked features (layer pixels), for Show Track Points.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<[f32; 2]>,
}

impl Default for FrameMotion {
    fn default() -> Self {
        FrameMotion { t: [0.0; 2], s: [1.0, 0.0, 0.0, 0.0], h: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0], inliers: 0, features: 0, points: vec![] }
    }
}

impl FrameMotion {
    /// The motion under a stabilization method.
    pub fn motion(&self, method: Method) -> Homography {
        match method {
            Method::Position => Homography::translation(self.t),
            Method::Similarity => {
                let s = self.s;
                Homography([[s[0], -s[1], s[2]], [s[1], s[0], s[3]], [0.0, 0.0, 1.0]])
            }
            Method::Perspective | Method::SubspaceWarp => {
                let h = self.h;
                Homography([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]])
            }
        }
    }
}

/// The stored analysis of a clip.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WarpAnalysis {
    pub version: u32,
    /// Layer time (seconds) of frame 0, and the frame duration.
    pub start: f64,
    pub frame_duration: f64,
    /// Layer size (pixels).
    pub size: [f64; 2],
    /// Detailed Analysis was on.
    pub detailed: bool,
    /// Per frame; frame 0's motion is the identity.
    pub frames: Vec<FrameMotion>,
}

impl WarpAnalysis {
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
    /// Frame index at a layer time (held at the ends).
    pub fn frame_at(&self, layer_time: f64) -> Option<usize> {
        if self.frames.is_empty() || self.frame_duration <= 0.0 {
            return None;
        }
        let i = ((layer_time - self.start) / self.frame_duration).round();
        Some(i.clamp(0.0, (self.frames.len() - 1) as f64) as usize)
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Option<WarpAnalysis> {
        if s.trim().is_empty() {
            return None;
        }
        serde_json::from_str(s).ok()
    }
}

/// Analysis options.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnalyzeOpts {
    /// Detailed Analysis: twice the resolution and more features.
    pub detailed: bool,
}

/// Incremental analysis: push the clip's frames in order.
pub struct Analyzer {
    opts: AnalyzeOpts,
    prev: Option<GrayPyramid>,
    frames: Vec<FrameMotion>,
    size: [f64; 2],
}

fn round_pts(p: &[[f64; 2]], n: usize) -> Vec<[f32; 2]> {
    let step = (p.len() / n.max(1)).max(1);
    p.iter().step_by(step).take(n).map(|q| [((q[0] * 10.0).round() / 10.0) as f32, ((q[1] * 10.0).round() / 10.0) as f32]).collect()
}

impl Analyzer {
    pub fn new(size: [f64; 2], opts: AnalyzeOpts) -> Analyzer {
        Analyzer { opts, prev: None, frames: vec![], size }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Analyse the next frame.
    pub fn push(&mut self, frame: &Frame) {
        let (max_side, max_feat) = if self.opts.detailed { (960, 700) } else { (480, 350) };
        let factor = analysis_factor(frame.img.width, frame.img.height, max_side);
        let pyr = GrayPyramid::from_image(frame.img, frame.offset, factor, 3);
        let mut fm = FrameMotion::default();
        if let Some(prev) = &self.prev
            && prev.width() == pyr.width()
            && prev.height() == pyr.height()
            && prev.offset == pyr.offset
        {
            let md = (prev.width().max(prev.height()) as f64 / 40.0).clamp(4.0, 16.0);
            let feats = good_features(prev, &CornerOpts { max_features: max_feat, min_distance: md, quality: 0.01, window: 2, border: 4 }, None);
            let q = track(prev, &pyr, &feats, None, &LkOpts { radius: 5, fb_max: 0.75, ..Default::default() });
            let (src, dst): (Vec<[f64; 2]>, Vec<[f64; 2]>) = feats.iter().zip(&q).filter_map(|(p, q)| q.map(|q| (prev.to_layer(*p), pyr.to_layer(q)))).unzip();
            fm.features = src.len() as u32;
            let thr = 1.0 * factor as f64;
            let seed = self.frames.len() as u64 + 1;
            if let Some(f) = ransac(Model::Translation, &src, &dst, thr * 1.5, 200, seed) {
                let m = f.h.0;
                fm.t = [m[0][2], m[1][2]];
            }
            if let Some(f) = ransac(Model::Similarity, &src, &dst, thr * 1.5, 300, seed) {
                let m = f.h.0;
                fm.s = [m[0][0], m[1][0], m[0][2], m[1][2]];
            }
            if let Some(f) = ransac(Model::Homography, &src, &dst, thr, 500, seed) {
                let m = f.h.normalized().0;
                // Reject wild projective fits (few features): keep the similarity then.
                let sane = m[2][0].abs() < 0.01 && m[2][1].abs() < 0.01;
                fm.h = if sane {
                    [m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1]]
                } else {
                    [fm.s[0], -fm.s[1], fm.s[2], fm.s[1], fm.s[0], fm.s[3], 0.0, 0.0]
                };
                fm.inliers = f.inlier_count() as u32;
                let inl: Vec<[f64; 2]> = dst.iter().zip(&f.inliers).filter(|(_, b)| **b).map(|(p, _)| *p).collect();
                fm.points = round_pts(&inl, 48);
            }
        }
        self.frames.push(fm);
        self.prev = Some(pyr);
    }

    pub fn finish(self, start: f64, frame_duration: f64) -> WarpAnalysis {
        WarpAnalysis { version: 1, start, frame_duration, size: self.size, detailed: self.opts.detailed, frames: self.frames }
    }
}

// ---------------------------------------------------------------- stabilization

/// Stabilization ▸ Result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StabResult {
    #[default]
    SmoothMotion,
    NoMotion,
}

/// Stabilization ▸ Method.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Method {
    Position,
    /// Position, Scale, Rotation.
    Similarity,
    Perspective,
    /// Approximated by Perspective (see the module docs of the effect).
    #[default]
    SubspaceWarp,
}

/// Borders ▸ Framing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Framing {
    StabilizeOnly,
    StabilizeCrop,
    #[default]
    StabilizeCropAutoScale,
    SynthesizeEdges,
}

/// Effect parameters that shape the stabilization.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StabSettings {
    pub result: StabResult,
    /// Smoothness, percent (50 = default).
    pub smoothness: f64,
    pub method: Method,
    pub preserve_scale: bool,
    pub framing: Framing,
    /// Maximum Scale, percent.
    pub max_scale: f64,
    /// Action Safe Margin, percent of the frame on each side.
    pub action_safe: f64,
    /// Additional Scale, percent.
    pub additional_scale: f64,
    /// Crop Less ↔ Smooth More (−100…100).
    pub crop_less_smooth_more: f64,
    /// Frames per second (for Smoothness).
    pub fps: f64,
}

impl Default for StabSettings {
    fn default() -> Self {
        StabSettings {
            result: StabResult::SmoothMotion,
            smoothness: 50.0,
            method: Method::SubspaceWarp,
            preserve_scale: false,
            framing: Framing::StabilizeCropAutoScale,
            max_scale: 150.0,
            action_safe: 0.0,
            additional_scale: 100.0,
            crop_less_smooth_more: 0.0,
            fps: 30.0,
        }
    }
}

impl StabSettings {
    /// Gaussian smoothing sigma in frames.
    pub fn sigma(&self) -> f64 {
        let base = self.smoothness.max(0.0) / 100.0 * self.fps.max(1.0) * 0.6;
        base * 2f64.powf(self.crop_less_smooth_more.clamp(-100.0, 100.0) / 100.0)
    }
}

/// The per-frame stabilization of a clip.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// Final source → output transform per frame (layer pixels), framing scale included.
    pub warps: Vec<Homography>,
    /// The visible region in output pixels `[x0, y0, x1, y1]` (Stabilize, Crop / Auto-scale);
    /// `None` = everything the warp produces.
    pub crop: Option<[f64; 4]>,
    /// Auto-scale factor (1 = none).
    pub auto_scale: f64,
    /// Fraction of the frame (per side length) valid on every frame before scaling.
    pub valid_fraction: f64,
}

fn convex_contains(q: &[[f64; 2]; 4], p: [f64; 2]) -> bool {
    let mut sign = 0.0;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let c = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if c.abs() < 1e-12 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

/// Largest `s ∈ [0, 1]` such that the centred `s·W × s·H` rectangle lies inside the frame
/// outline mapped by `b`.
pub fn valid_fraction(b: &Homography, size: [f64; 2]) -> f64 {
    let [w, h] = size;
    let quad = [b.apply([0.0, 0.0]), b.apply([w, 0.0]), b.apply([w, h]), b.apply([0.0, h])];
    // A fold or a point at infinity: nothing is valid.
    let m = b.normalized().0;
    for p in [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]] {
        if m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] <= 1e-9 {
            return 0.0;
        }
    }
    let c = [w / 2.0, h / 2.0];
    let fits = |s: f64| {
        let (hw, hh) = (s * w / 2.0, s * h / 2.0);
        [[c[0] - hw, c[1] - hh], [c[0] + hw, c[1] - hh], [c[0] + hw, c[1] + hh], [c[0] - hw, c[1] + hh]].iter().all(|p| convex_contains(&quad, *p))
    };
    if fits(1.0) {
        return 1.0;
    }
    if !fits(0.0) {
        return 0.0;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..30 {
        let mid = 0.5 * (lo + hi);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

fn remove_scale(b: &Homography, c: [f64; 2]) -> Homography {
    // Local scale at the frame centre from the Jacobian of the map.
    let e = 1.0;
    let p0 = b.apply(c);
    let px = b.apply([c[0] + e, c[1]]);
    let py = b.apply([c[0], c[1] + e]);
    let det = ((px[0] - p0[0]) * (py[1] - p0[1]) - (px[1] - p0[1]) * (py[0] - p0[0])).abs();
    let s = det.sqrt();
    if s < 1e-9 {
        return *b;
    }
    Homography::scale_about(1.0 / s, p0).then_after(b)
}

/// Corrective transforms (source → stabilized, before framing) for every frame.
pub fn corrections(a: &WarpAnalysis, s: &StabSettings) -> Vec<Homography> {
    let n = a.frames.len();
    if n == 0 {
        return vec![];
    }
    let method = s.method;
    let inter: Vec<Homography> = a.frames.iter().map(|f| f.motion(method)).collect();
    let inv: Vec<Homography> = inter.iter().map(|h| h.inverse().unwrap_or(Homography::IDENTITY)).collect();
    let c = [a.size[0] / 2.0, a.size[1] / 2.0];
    let mut out = Vec::with_capacity(n);
    match s.result {
        StabResult::NoMotion => {
            // Cumulative path C_t (frame 0 → frame t); every frame maps onto the middle frame.
            let mut cum = Vec::with_capacity(n);
            let mut cur = Homography::IDENTITY;
            for (k, h) in inter.iter().enumerate() {
                if k > 0 {
                    cur = h.then_after(&cur);
                }
                cum.push(cur);
            }
            let r = n / 2;
            for c_t in &cum {
                let ci = c_t.inverse().unwrap_or(Homography::IDENTITY);
                out.push(cum[r].then_after(&ci));
            }
        }
        StabResult::SmoothMotion => {
            let sigma = s.sigma();
            let rad = ((3.0 * sigma).ceil() as usize).min(n.saturating_sub(1));
            for t in 0..n {
                if sigma < 0.3 || rad == 0 {
                    out.push(Homography::IDENTITY);
                    continue;
                }
                let mut acc = [[0.0; 3]; 3];
                let mut wsum = 0.0;
                let mut add = |h: &Homography, d: f64| {
                    let w = (-(d * d) / (2.0 * sigma * sigma)).exp();
                    let m = h.normalized().0;
                    for i in 0..3 {
                        for j in 0..3 {
                            acc[i][j] += w * m[i][j];
                        }
                    }
                    wsum += w;
                };
                add(&Homography::IDENTITY, 0.0);
                // Forward: T_t^{i+1} = inter[i+1] ∘ T_t^i.
                let mut tf = Homography::IDENTITY;
                for i in t + 1..=(t + rad).min(n - 1) {
                    tf = inter[i].then_after(&tf);
                    add(&tf, (i - t) as f64);
                }
                // Backward: T_t^{i-1} = inter[i]⁻¹ ∘ T_t^i.
                let mut tb = Homography::IDENTITY;
                for i in (t.saturating_sub(rad)..t).rev() {
                    tb = inv[i + 1].then_after(&tb);
                    add(&tb, (t - i) as f64);
                }
                let m = acc.map(|r| r.map(|v| v / wsum));
                out.push(Homography(m).normalized());
            }
        }
    }
    if s.preserve_scale && method != Method::Position {
        out = out.iter().map(|b| remove_scale(b, c)).collect();
    }
    out
}

/// Stabilization plan: corrections plus framing.
pub fn plan(a: &WarpAnalysis, s: &StabSettings) -> Plan {
    let mut b = corrections(a, s);
    let size = a.size;
    let c = [size[0] / 2.0, size[1] / 2.0];
    let add = (s.additional_scale / 100.0).max(0.01);
    let z_add = Homography::scale_about(add, c);
    let fracs = |b: &[Homography]| b.iter().map(|h| valid_fraction(h, size)).collect::<Vec<f64>>();
    let mut fr = fracs(&b);
    let mut vmin = fr.iter().copied().fold(1.0, f64::min);
    let margin = (s.action_safe / 100.0).clamp(0.0, 0.45);
    let need_cover = 1.0 - 2.0 * margin;
    let max_scale = (s.max_scale / 100.0).max(1.0);
    if s.framing == Framing::StabilizeCropAutoScale && vmin * max_scale < need_cover && !b.is_empty() {
        // Relax the correction on frames that need more than Maximum Scale.
        let target = need_cover / max_scale;
        let n = b.len();
        let mut lam = vec![1.0f64; n];
        for t in 0..n {
            if fr[t] >= target {
                continue;
            }
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..24 {
                let mid = 0.5 * (lo + hi);
                if valid_fraction(&Homography::IDENTITY.lerp(&b[t], mid), size) >= target {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lam[t] = lo;
        }
        // Spread the relaxation smoothly (min filter, then a Gaussian of the same radius) so
        // the camera doesn't jump.
        let r = ((s.sigma() * 0.5).ceil() as usize).max(1);
        let minf: Vec<f64> = (0..n).map(|t| lam[t.saturating_sub(r)..=(t + r).min(n - 1)].iter().copied().fold(1.0, f64::min)).collect();
        let sig = r as f64 / 2.0;
        let smooth: Vec<f64> = (0..n)
            .map(|t| {
                let (mut acc, mut ws) = (0.0, 0.0);
                for i in t.saturating_sub(r)..=(t + r).min(n - 1) {
                    let d = i as f64 - t as f64;
                    let w = (-(d * d) / (2.0 * sig * sig)).exp();
                    acc += w * minf[i];
                    ws += w;
                }
                (acc / ws).min(lam[t])
            })
            .collect();
        b = b.iter().zip(&smooth).map(|(h, l)| if *l >= 1.0 { *h } else { Homography::IDENTITY.lerp(h, *l) }).collect();
        fr = fracs(&b);
        vmin = fr.iter().copied().fold(1.0, f64::min);
    }
    let rect = |k: f64| [c[0] - k * size[0] / 2.0, c[1] - k * size[1] / 2.0, c[0] + k * size[0] / 2.0, c[1] + k * size[1] / 2.0];
    let (auto, crop) = match s.framing {
        Framing::StabilizeOnly | Framing::SynthesizeEdges => (1.0, None),
        Framing::StabilizeCrop => (1.0, Some(rect(vmin * add))),
        Framing::StabilizeCropAutoScale => {
            let k = if vmin > 1e-6 { (need_cover / vmin).clamp(1.0, max_scale) } else { max_scale };
            let vis = vmin * k * add;
            (k, if vis >= 1.0 - 1e-9 { None } else { Some(rect(vis)) })
        }
    };
    let z = Homography::scale_about(auto, c);
    let warps = b.iter().map(|h| z_add.then_after(&z.then_after(h))).collect();
    Plan { warps, crop, auto_scale: auto, valid_fraction: vmin }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic analysis: a slow pan plus per-frame jitter.
    fn shaky(n: usize) -> (WarpAnalysis, Vec<[f64; 2]>) {
        let pos = |k: usize| {
            let j = [((k * 7919) % 13) as f64 - 6.0, ((k * 104729) % 11) as f64 - 5.0];
            [2.0 * k as f64 + j[0], 0.5 * k as f64 + j[1]]
        };
        let mut a = WarpAnalysis { version: 1, start: 0.0, frame_duration: 1.0 / 30.0, size: [640.0, 360.0], detailed: false, frames: vec![] };
        let path: Vec<[f64; 2]> = (0..n).map(pos).collect();
        for k in 0..n {
            let mut f = FrameMotion::default();
            if k > 0 {
                // Scene points move opposite to the camera.
                let d = [path[k - 1][0] - path[k][0], path[k - 1][1] - path[k][1]];
                f.t = d;
                f.s = [1.0, 0.0, d[0], d[1]];
                f.h = [1.0, 0.0, d[0], 0.0, 1.0, d[1], 0.0, 0.0];
            }
            a.frames.push(f);
        }
        (a, path)
    }

    fn jitter(p: &[[f64; 2]]) -> f64 {
        // Mean magnitude of the second difference (acceleration).
        let mut s = 0.0;
        for k in 1..p.len() - 1 {
            s += (p[k + 1][0] - 2.0 * p[k][0] + p[k - 1][0]).hypot(p[k + 1][1] - 2.0 * p[k][1] + p[k - 1][1]);
        }
        s / (p.len() - 2) as f64
    }

    #[test]
    fn smoothing_removes_jitter_and_no_motion_locks() {
        let (a, path) = shaky(90);
        // A scene point at (320, 180) on frame 0 appears at (320, 180) - path[k] on frame k.
        let scene = |k: usize| [320.0 - path[k][0] + path[0][0], 180.0 - path[k][1] + path[0][1]];
        let before: Vec<[f64; 2]> = (0..90).map(scene).collect();
        for method in [Method::Position, Method::Similarity, Method::Perspective] {
            let s = StabSettings { method, framing: Framing::StabilizeOnly, ..Default::default() };
            let p = plan(&a, &s);
            let after: Vec<[f64; 2]> = (0..90).map(|k| p.warps[k].apply(scene(k))).collect();
            let (j0, j1) = (jitter(&before), jitter(&after));
            assert!(j1 < 0.2 * j0, "{method:?}: jitter {j0} → {j1}");
        }
        let s = StabSettings { result: StabResult::NoMotion, framing: Framing::StabilizeOnly, ..Default::default() };
        let p = plan(&a, &s);
        let fixed: Vec<[f64; 2]> = (0..90).map(|k| p.warps[k].apply(scene(k))).collect();
        for q in &fixed {
            assert!((q[0] - fixed[45][0]).abs() < 1e-6 && (q[1] - fixed[45][1]).abs() < 1e-6);
        }
    }

    #[test]
    fn framing_crops_and_scales() {
        let (a, _) = shaky(60);
        let p = plan(&a, &StabSettings { framing: Framing::StabilizeCrop, ..Default::default() });
        let crop = p.crop.unwrap();
        assert!(p.auto_scale == 1.0 && crop[0] > 0.0 && crop[2] < 640.0);
        // Every frame covers the crop rectangle.
        for w in &p.warps {
            let inv = w.inverse().unwrap();
            for q in [[crop[0], crop[1]], [crop[2], crop[1]], [crop[2], crop[3]], [crop[0], crop[3]]] {
                let s = inv.apply(q);
                assert!(s[0] >= -1e-6 && s[1] >= -1e-6 && s[0] <= 640.0 + 1e-6 && s[1] <= 360.0 + 1e-6, "{s:?}");
            }
        }
        let p = plan(&a, &StabSettings::default());
        assert!(p.auto_scale > 1.0 && p.crop.is_none(), "{} {:?}", p.auto_scale, p.crop);
        for w in &p.warps {
            let inv = w.inverse().unwrap();
            for q in [[0.0, 0.0], [640.0, 0.0], [640.0, 360.0], [0.0, 360.0]] {
                let s = inv.apply(q);
                assert!(s[0] >= -1e-3 && s[1] >= -1e-3 && s[0] <= 640.001 && s[1] <= 360.001, "{s:?}");
            }
        }
        // A tight Maximum Scale relaxes the correction instead of showing borders.
        let p = plan(&a, &StabSettings { max_scale: 101.0, ..Default::default() });
        assert!(p.auto_scale <= 1.01 + 1e-9);
        assert!(p.valid_fraction * p.auto_scale >= 0.999 || p.crop.is_some());
        let j = WarpAnalysis::from_json(&a.to_json()).unwrap();
        assert_eq!(j, a);
    }
}
