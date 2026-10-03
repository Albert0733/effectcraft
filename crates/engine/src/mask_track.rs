//! Mask tracking runtime (Layer ▸ Mask ▸ Track Mask, the Tracker panel in mask mode): follow
//! the pixels inside a mask from frame to frame and key its Mask Path on every analysed frame.
//!
//! Analysis runs on a background thread (progress, cancel; blocking with `wait` and on wasm32)
//! like the point tracker, over the layer's source frames; one undo step covers it. The
//! algorithm is `effectcraft_track::mask` (KLT features inside the mask + a RANSAC fit of the
//! chosen motion model), and every frame's motion is applied to the mask's vertices and
//! tangents, so the shape's Bezier structure is kept.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use effectcraft_keyframe::{Keyframe, ShapePath, Value};
use effectcraft_project::{ItemId, LayerId, Project, Uid};
use effectcraft_render::{ExprHost, FootageSource, LayerCache, RenderOpts, Renderer};
use effectcraft_time::Tick;
use effectcraft_track as trk;
use effectcraft_track::fit::Model;
use serde::{Deserialize, Serialize};

use crate::tracking::{Direction, TrackProgress, lock, source_frame};
use crate::{Event, Session};

/// Tracker panel ▸ Method (mask tracking).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskMethod {
    Position,
    PositionScale,
    #[default]
    PositionScaleRotation,
    PositionScaleRotationSkew,
    Perspective,
}

impl MaskMethod {
    pub const ALL: [MaskMethod; 5] =
        [MaskMethod::Position, MaskMethod::PositionScale, MaskMethod::PositionScaleRotation, MaskMethod::PositionScaleRotationSkew, MaskMethod::Perspective];
    pub fn label(self) -> &'static str {
        match self {
            MaskMethod::Position => "Position",
            MaskMethod::PositionScale => "Position & Scale",
            MaskMethod::PositionScaleRotation => "Position, Scale & Rotation",
            MaskMethod::PositionScaleRotationSkew => "Position, Scale, Rotation & Skew",
            MaskMethod::Perspective => "Perspective",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            MaskMethod::Position => "position",
            MaskMethod::PositionScale => "positionScale",
            MaskMethod::PositionScaleRotation => "positionScaleRotation",
            MaskMethod::PositionScaleRotationSkew => "positionScaleRotationSkew",
            MaskMethod::Perspective => "perspective",
        }
    }
    pub fn from_name(s: &str) -> Option<MaskMethod> {
        let n: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        MaskMethod::ALL
            .into_iter()
            .find(|m| m.id().to_ascii_lowercase() == n || m.label().chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase() == n)
    }
    pub fn model(self) -> Model {
        match self {
            MaskMethod::Position => Model::Translation,
            MaskMethod::PositionScale => Model::TranslationScale,
            MaskMethod::PositionScaleRotation => Model::Similarity,
            MaskMethod::PositionScaleRotationSkew => Model::Affine,
            MaskMethod::Perspective => Model::Homography,
        }
    }
}

/// A closed polyline through a Bezier path (`steps` samples per segment), in layer pixels.
pub fn flatten(p: &ShapePath, steps: usize) -> Vec<[f64; 2]> {
    let n = p.len();
    if n == 0 {
        return vec![];
    }
    let segs = if p.closed { n } else { n.saturating_sub(1) };
    let mut out = Vec::with_capacity(segs * steps + 1);
    for i in 0..segs {
        let j = (i + 1) % n;
        let (a, b) = (p.vertices[i], p.vertices[j]);
        let o = p.out_tangents.get(i).copied().unwrap_or([0.0; 2]);
        let it = p.in_tangents.get(j).copied().unwrap_or([0.0; 2]);
        let c = [a, [a[0] + o[0], a[1] + o[1]], [b[0] + it[0], b[1] + it[1]], b];
        for k in 0..steps {
            let t = k as f64 / steps as f64;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            out.push([0, 1].map(|d| w[0] * c[0][d] + w[1] * c[1][d] + w[2] * c[2][d] + w[3] * c[3][d]));
        }
    }
    if !p.closed {
        out.push(p.vertices[n - 1]);
    }
    out
}

/// A path moved by a frame-to-frame motion (vertices and tangent handles).
pub fn transform_path(p: &ShapePath, h: &trk::Homography) -> ShapePath {
    ShapePath {
        vertices: p.vertices.iter().map(|v| h.apply(*v)).collect(),
        in_tangents: p.vertices.iter().zip(&p.in_tangents).map(|(v, t)| h.apply_tangent(*v, *t)).collect(),
        out_tangents: p.vertices.iter().zip(&p.out_tangents).map(|(v, t)| h.apply_tangent(*v, *t)).collect(),
        closed: p.closed,
        feather: p.feather.clone(),
    }
}

/// One tracked frame: (layer time, comp time, mask path).
type MaskFrame = (Tick, Tick, ShapePath);

#[derive(Default)]
pub struct MaskTrackShared {
    pub cancel: AtomicBool,
    pub state: Mutex<TrackProgress>,
    frames: Mutex<Vec<MaskFrame>>,
}

/// A running (or finished, not yet polled) mask track.
pub struct MaskTrackJob {
    pub shared: Arc<MaskTrackShared>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub comp: ItemId,
    pub layer: LayerId,
    pub mask: Uid,
    pub direction: Direction,
}

impl MaskTrackJob {
    pub fn is_finished(&self) -> bool {
        lock(&self.shared.state).finished
    }
    pub fn progress(&self) -> TrackProgress {
        lock(&self.shared.state).clone()
    }
}

/// What the worker needs, detached from the session.
pub(crate) struct MaskWork {
    pub project: Arc<Project>,
    pub footage: Arc<dyn FootageSource>,
    pub expr: Option<Arc<dyn ExprHost>>,
    pub cache: Arc<LayerCache>,
    pub comp: ItemId,
    pub layer: LayerId,
    /// The mask path on the first frame.
    pub path: ShapePath,
    pub method: MaskMethod,
    /// Comp times; the first is the start frame.
    pub times: Vec<Tick>,
}

fn run_work(w: MaskWork, shared: &MaskTrackShared) {
    let t0 = web_time::Instant::now();
    let fail = |msg: &str| {
        let mut s = lock(&shared.state);
        s.error = Some(msg.to_string());
        s.finished = true;
    };
    let Some(comp) = w.project.comp(w.comp) else { return fail("composition missing") };
    let Some(layer) = comp.layer(w.layer) else { return fail("layer missing") };
    let mut r = Renderer::new(&w.project, w.footage.as_ref(), RenderOpts { scale: 1.0, ..Default::default() });
    r.expr = w.expr.as_deref();
    r.cache = Some(&w.cache);
    let r = &r;
    let frame_at = |t: Tick| source_frame(r, &w.project, w.comp, comp, layer, t, w.expr.as_deref());
    lock(&shared.state).total = w.times.len().saturating_sub(1) as u64;
    let Some(first) = frame_at(w.times[0]) else { return fail("the layer has no pixels to track") };
    let mut tracker = trk::mask::MaskTracker::new(w.method.model(), flatten(&w.path, 8), &trk::Frame { img: &first.0, offset: first.1 });
    let mut path = w.path.clone();
    lock(&shared.frames).push((layer.layer_time(w.times[0]), w.times[0], path.clone()));
    let mut next = if w.times.len() > 1 { frame_at(w.times[1]) } else { None };
    for k in 1..w.times.len() {
        if shared.cancel.load(Ordering::Relaxed) {
            lock(&shared.state).cancelled = true;
            break;
        }
        let Some(cur) = next.take() else {
            lock(&shared.state).error = Some(format!("no frame at {:.3} s", w.times[k].seconds()));
            break;
        };
        let (res, nf) =
            rayon::join(|| tracker.step(&trk::Frame { img: &cur.0, offset: cur.1 }), || if k + 1 < w.times.len() { frame_at(w.times[k + 1]) } else { None });
        next = nf;
        let Some(step) = res else {
            lock(&shared.state).error = Some(format!("lost the mask's pixels at {:.3} s (too little texture inside the mask)", w.times[k].seconds()));
            break;
        };
        path = transform_path(&path, &step.motion);
        lock(&shared.frames).push((layer.layer_time(w.times[k]), w.times[k], path.clone()));
        let mut s = lock(&shared.state);
        s.done = k as u64;
        s.elapsed = t0.elapsed().as_secs_f64();
        s.fps = if s.elapsed > 0.0 { k as f64 / s.elapsed } else { 0.0 };
        s.time = w.times[k].seconds();
        crate::offload::report(s.done, s.total);
    }
    let mut s = lock(&shared.state);
    s.elapsed = t0.elapsed().as_secs_f64();
    s.finished = true;
}

fn write_frames(p: &mut Project, job: &MaskTrackJob, frames: &[MaskFrame]) {
    let Some(layer) = p.comp_mut(job.comp).and_then(|c| c.layer_mut(job.layer)) else { return };
    let Some(g) = layer.props.find_group_mut(job.mask) else { return };
    let Some(pr) = g.get_mut("path") else { return };
    for (lt, _, path) in frames {
        effectcraft_keyframe::set_key(&mut pr.keys, Keyframe::new(*lt, Value::Path(path.clone())));
    }
}

impl Session {
    /// Whether a mask track is running.
    pub fn is_mask_tracking(&self) -> bool {
        self.mask_job.as_ref().is_some_and(|j| !j.is_finished()) || self.offloaded(crate::offload::JobKind::MaskTrack).is_some()
    }

    /// Live progress of the running (or just finished) mask track.
    pub fn mask_track_progress(&self) -> Option<TrackProgress> {
        self.mask_job.as_ref().map(|j| j.progress()).or_else(|| self.offloaded(crate::offload::JobKind::MaskTrack).map(|j| j.progress.track()))
    }

    /// Cancel the running mask track (frames tracked so far are kept).
    pub fn stop_mask_track(&mut self) -> bool {
        if self.cancel_offloaded(crate::offload::JobKind::MaskTrack) {
            return true;
        }
        match &self.mask_job {
            Some(j) if !j.is_finished() => {
                j.shared.cancel.store(true, Ordering::Relaxed);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn start_mask_track(&mut self, work: MaskWork, mask: Uid, direction: Direction, wait: bool) -> Result<(), String> {
        if self.is_mask_tracking() || self.is_tracking() {
            return Err("a track analysis is already running".into());
        }
        self.poll_mask_track();
        if !wait && self.offloads() {
            let MaskWork { comp, layer, path, method, times, .. } = work;
            return self.offload_analysis(crate::offload::WorkerJob::MaskTrack { comp, layer, mask, direction, path, method, times });
        }
        let wait = wait || cfg!(target_arch = "wasm32");
        self.history.undo.push(("Track Mask".into(), self.project.clone()));
        self.history.redo.clear();
        self.history.merge_key = None;
        let shared = Arc::new(MaskTrackShared::default());
        let (comp, layer) = (work.comp, work.layer);
        let thread = if wait {
            run_work(work, &shared);
            None
        } else {
            let sh = shared.clone();
            Some(std::thread::Builder::new().name("mask-track".into()).spawn(move || run_work(work, &sh)).map_err(|e| e.to_string())?)
        };
        self.mask_job = Some(MaskTrackJob { shared, thread, comp, layer, mask, direction });
        self.poll_mask_track();
        Ok(())
    }

    /// Key newly tracked frames and move the CTI with them; drop the job once it finished.
    /// Frontends call this every frame while tracking.
    pub fn poll_mask_track(&mut self) -> bool {
        let Some(job) = &self.mask_job else { return false };
        let frames = std::mem::take(&mut *lock(&job.shared.frames));
        let finished = job.is_finished();
        let changed = !frames.is_empty();
        if changed {
            write_frames(Arc::make_mut(&mut self.project), job, &frames);
            if self.state.active_comp == Some(job.comp)
                && let Some(last) = frames.last()
            {
                self.state.times.insert(job.comp, last.1);
            }
            self.bump();
        }
        if finished {
            let mut job = self.mask_job.take().expect("job");
            if let Some(t) = job.thread.take() {
                let _ = t.join();
            }
            let st = job.progress();
            let msg = match (&st.error, st.cancelled) {
                (Some(e), _) => format!("Mask tracking stopped: {e}"),
                (None, true) => format!("Mask tracking stopped after {} frame(s)", st.done),
                _ => format!("Tracked the mask over {} frame(s) in {:.1} s", st.done, st.elapsed),
            };
            self.events.push(Event::Toast { message: msg, error: st.error.is_some() });
        }
        changed || finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_parse_and_paths_transform() {
        assert_eq!(MaskMethod::from_name("positionScaleRotationSkew"), Some(MaskMethod::PositionScaleRotationSkew));
        assert_eq!(MaskMethod::from_name("Position & Scale"), Some(MaskMethod::PositionScale));
        assert_eq!(MaskMethod::from_name("perspective"), Some(MaskMethod::Perspective));
        let p = ShapePath::ellipse([50.0, 40.0], 40.0, 20.0);
        let poly = flatten(&p, 8);
        assert_eq!(poly.len(), p.len() * 8);
        assert!(poly.iter().all(|q| ((q[0] - 50.0) / 20.0).powi(2) + ((q[1] - 40.0) / 10.0).powi(2) < 1.01));
        let h = trk::Homography([[0.0, -2.0, 10.0], [2.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
        let q = transform_path(&p, &h);
        for i in 0..p.len() {
            let v = h.apply(p.vertices[i]);
            assert!((q.vertices[i][0] - v[0]).abs() < 1e-9);
            let t = p.out_tangents[i];
            assert!((q.out_tangents[i][0] + 2.0 * t[1]).abs() < 1e-9 && (q.out_tangents[i][1] - 2.0 * t[0]).abs() < 1e-9);
        }
    }
}
