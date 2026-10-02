//! Background frame rendering and the RAM preview cache.
//!
//! Frames render on a dedicated thread pool from immutable `Arc<Project>` snapshots, so the UI
//! never waits for the compositor. Results land in a memory-budgeted cache keyed by (project
//! revision, comp, frame, scale); the timeline draws the cached range as the green cache bar.
//!
//! wasm32 has no threads: jobs wait in the same priority queue and [`Frames::pump`] renders them
//! on the UI thread between egui frames, within a time budget.
//!
//! With Mercury GPU Acceleration (a [`Gpu`] on egui-wgpu's device and the project's renderer set
//! to the GPU) frames stay on the GPU: the compositor writes an RGBA8 texture that the viewer
//! registers with egui-wgpu and draws directly; pixels are read back only when something needs
//! them (Info panel, eyedroppers, histograms).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use effectcraft_engine::project::{ItemId, Project};
use effectcraft_engine::render::{ExprHost, FootageSource, LayerCache, RenderOpts, Renderer};
use effectcraft_engine::time::Tick;
use effectcraft_gpu::{DisplayFrame, Gpu};
use rayon::prelude::*;

/// GPU frames kept for RAM preview (bytes of video memory).
const GPU_BUDGET: usize = 1 << 30;

/// A rendered viewer frame: CPU pixels or a GPU texture.
#[derive(Clone)]
pub enum FrameImage {
    Cpu(Arc<egui::ColorImage>),
    Gpu(Arc<DisplayFrame>),
}

impl FrameImage {
    pub fn size(&self) -> [usize; 2] {
        match self {
            FrameImage::Cpu(c) => c.size,
            FrameImage::Gpu(f) => [f.width as usize, f.height as usize],
        }
    }
    fn bytes(&self) -> usize {
        self.size()[0] * self.size()[1] * 4
    }
    fn is_gpu(&self) -> bool {
        matches!(self, FrameImage::Gpu(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameKey {
    pub revision: u64,
    pub comp: u64,
    pub frame: i64,
    /// Render scale × 1000.
    pub scale: u32,
    /// Hash of the 3D view camera (0 = the comp's active camera).
    pub view: u64,
}

struct Cache {
    map: HashMap<FrameKey, FrameImage>,
    order: VecDeque<FrameKey>,
    bytes: usize,
    /// Bytes of GPU frames (also counted in `bytes`).
    gpu_bytes: usize,
    budget: usize,
}

impl Cache {
    fn insert(&mut self, k: FrameKey, img: FrameImage) {
        let (sz, gpu) = (img.bytes(), img.is_gpu());
        if let Some(old) = self.map.insert(k, img) {
            self.forget(&old);
        } else {
            self.order.push_back(k);
        }
        self.bytes += sz;
        if gpu {
            self.gpu_bytes += sz;
        }
        while self.bytes > self.budget {
            let Some(old) = self.order.pop_front() else { break };
            self.remove(&old);
        }
        // Video memory: drop the oldest GPU frames past their own budget.
        while self.gpu_bytes > GPU_BUDGET {
            let Some(i) = self.order.iter().position(|k| self.map.get(k).is_some_and(FrameImage::is_gpu)) else { break };
            if let Some(old) = self.order.remove(i) {
                self.remove(&old);
            }
        }
    }
    fn forget(&mut self, img: &FrameImage) {
        self.bytes -= img.bytes();
        if img.is_gpu() {
            self.gpu_bytes -= img.bytes();
        }
    }
    fn remove(&mut self, k: &FrameKey) {
        if let Some(i) = self.map.remove(k) {
            self.forget(&i);
        }
    }
    fn evict_to_budget(&mut self) {
        while self.bytes > self.budget {
            let Some(old) = self.order.pop_front() else { break };
            self.remove(&old);
        }
    }
}

/// What a render job needs (all cheap clones).
#[derive(Clone)]
pub struct RenderSource {
    pub project: Arc<Project>,
    pub footage: Arc<dyn FootageSource>,
    pub expr: Option<Arc<dyn ExprHost>>,
    pub layer_cache: Arc<LayerCache>,
    /// GPU compositor on the viewer's device (Mercury GPU Acceleration), if available.
    pub gpu: Option<Gpu>,
}

pub struct Frames {
    cache: Arc<Mutex<Cache>>,
    /// Keys queued or rendering.
    inflight: Arc<Mutex<HashSet<FrameKey>>>,
    /// Pending jobs, picked by priority when a pool thread frees up (the viewer's frame first).
    queue: Arc<Mutex<Queue>>,
    #[cfg(not(target_arch = "wasm32"))]
    pool: rayon::ThreadPool,
    ctx: Option<egui::Context>,
    /// Render time of the last viewer (urgent) frame in ms, for the Info panel / perf readout.
    pub last_ms: Arc<Mutex<f64>>,
}

impl Default for Frames {
    fn default() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 16);
        Frames {
            cache: Arc::new(Mutex::new(Cache { map: HashMap::new(), order: VecDeque::new(), bytes: 0, gpu_bytes: 0, budget: 3 << 30 })),
            inflight: Arc::new(Mutex::new(HashSet::new())),
            queue: Arc::new(Mutex::new(Queue::default())),
            #[cfg(not(target_arch = "wasm32"))]
            pool: rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("ec-frame-{i}")).build().expect("frame pool"),
            ctx: None,
            last_ms: Arc::new(Mutex::new(0.0)),
        }
    }
}

/// Premultiplied f32 → egui premultiplied Color32 (alpha kept, so the viewer can show the
/// transparency grid or the comp background colour underneath).
pub fn to_color_image(img: &effectcraft_engine::render::Image) -> egui::ColorImage {
    let px: Vec<egui::Color32> = img
        .data
        .par_iter()
        .map(|p| {
            let a = p[3].clamp(0.0, 1.0);
            let c = |v: f32| (v.clamp(0.0, a) * 255.0 + 0.5) as u8;
            egui::Color32::from_rgba_premultiplied(c(p[0]), c(p[1]), c(p[2]), (a * 255.0 + 0.5) as u8)
        })
        .collect();
    egui::ColorImage::new([img.width as usize, img.height as usize], px)
}

impl Frames {
    pub fn set_context(&mut self, ctx: &egui::Context) {
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
    }

    pub fn get(&self, k: &FrameKey) -> Option<FrameImage> {
        self.cache.lock().ok()?.map.get(k).cloned()
    }

    pub fn is_cached(&self, k: &FrameKey) -> bool {
        self.cache.lock().map(|c| c.map.contains_key(k)).unwrap_or(false)
    }

    pub fn inflight(&self) -> usize {
        self.inflight.lock().map(|s| s.len()).unwrap_or(0)
    }

    /// Cached frame numbers for (revision, comp, scale) — for the cache bar.
    pub fn cached_frames(&self, revision: u64, comp: u64, scale: u32) -> Vec<i64> {
        let Ok(c) = self.cache.lock() else { return vec![] };
        let mut v: Vec<i64> = c.map.keys().filter(|k| k.revision == revision && k.comp == comp && k.scale == scale).map(|k| k.frame).collect();
        v.sort_unstable();
        v
    }

    /// Change the RAM preview cache budget (Settings ▸ Memory & CPU), evicting the oldest
    /// frames when it shrinks.
    pub fn set_budget(&self, bytes: usize) {
        if let Ok(mut c) = self.cache.lock() {
            c.budget = bytes;
            c.evict_to_budget();
        }
    }

    pub fn budget(&self) -> usize {
        self.cache.lock().map(|c| c.budget).unwrap_or(0)
    }

    pub fn clear(&self) {
        if let Ok(mut c) = self.cache.lock() {
            c.map.clear();
            c.order.clear();
            c.bytes = 0;
            c.gpu_bytes = 0;
        }
    }

    /// Queue a prefetch frame (no-op if cached or already queued/rendering).
    pub fn request(&self, src: &RenderSource, key: FrameKey, comp: ItemId, t: Tick, opts: RenderOpts) {
        self.request_with(src, key, comp, t, opts, false);
    }

    /// Queue the frame the viewer is showing: it jumps ahead of every prefetch job.
    pub fn request_urgent(&self, src: &RenderSource, key: FrameKey, comp: ItemId, t: Tick, opts: RenderOpts) {
        self.request_with(src, key, comp, t, opts, true);
    }

    /// An urgent (viewer) frame is queued or rendering.
    pub fn urgent_pending(&self) -> bool {
        self.queue.lock().map(|q| q.urgent_running > 0 || q.jobs.iter().any(|j| j.urgent)).unwrap_or(false)
    }

    fn request_with(&self, src: &RenderSource, key: FrameKey, comp: ItemId, t: Tick, opts: RenderOpts, urgent: bool) {
        if self.is_cached(&key) {
            return;
        }
        let Ok(mut q) = self.queue.lock() else { return };
        // Jobs for an older project revision can never be shown: drop them.
        let before = q.jobs.len();
        q.jobs.retain(|j| j.key.revision >= key.revision);
        let dropped = before - q.jobs.len();
        if dropped > 0
            && let Ok(mut inf) = self.inflight.lock()
        {
            inf.retain(|k| k.revision >= key.revision || q.running.contains(k));
        }
        if urgent {
            // Only the newest viewer frame is urgent; earlier ones become prefetch.
            for j in q.jobs.iter_mut() {
                j.urgent = j.key == key;
            }
        }
        {
            let Ok(mut inf) = self.inflight.lock() else { return };
            if !inf.insert(key) {
                return;
            }
        }
        q.seq += 1;
        let seq = q.seq;
        q.jobs.push(Job { key, src: src.clone(), comp, t, opts, urgent, seq });
        drop(q);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let w = self.worker();
            // One spawn per job; each spawn renders whichever queued job matters most right now.
            self.pool.spawn(move || {
                w.run_next();
            });
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(ctx) = &self.ctx {
            // rendered by `pump` after this UI frame
            ctx.request_repaint();
        }
    }

    fn worker(&self) -> Worker {
        Worker { queue: self.queue.clone(), cache: self.cache.clone(), inflight: self.inflight.clone(), ctx: self.ctx.clone(), last: self.last_ms.clone() }
    }

    /// Render queued frames on this thread, most important first, until the queue is empty or
    /// `budget` has passed (at least one frame). Returns whether frames are still queued. Only
    /// wasm32 needs this (native frames render on the pool); elsewhere it does nothing.
    pub fn pump(&self, budget: std::time::Duration) -> bool {
        if cfg!(not(target_arch = "wasm32")) {
            return false;
        }
        let t0 = web_time::Instant::now();
        let w = self.worker();
        while w.run_next() {
            if t0.elapsed() >= budget {
                break;
            }
        }
        self.queue.lock().map(|q| !q.jobs.is_empty()).unwrap_or(false)
    }
}

/// Renders queued jobs (on a pool thread, or the UI thread on wasm32).
struct Worker {
    queue: Arc<Mutex<Queue>>,
    cache: Arc<Mutex<Cache>>,
    inflight: Arc<Mutex<HashSet<FrameKey>>>,
    ctx: Option<egui::Context>,
    last: Arc<Mutex<f64>>,
}

impl Worker {
    /// Render the queued job that matters most right now. `false` when the queue was empty.
    fn run_next(&self) -> bool {
        let Worker { queue, cache, inflight, ctx, last } = self;
        let job = {
            let Ok(mut q) = queue.lock() else { return false };
            // Urgent first (newest), then prefetch in request order.
            let best = q.jobs.iter().enumerate().max_by_key(|(_, j)| (j.urgent, if j.urgent { j.seq as i64 } else { -(j.seq as i64) })).map(|(i, _)| i);
            let Some(i) = best else { return false };
            let job = q.jobs.swap_remove(i);
            q.running.insert(job.key);
            if job.urgent {
                q.urgent_running += 1;
            }
            job
        };
        let t0 = web_time::Instant::now();
        let mut r = Renderer::new(&job.src.project, job.src.footage.as_ref(), job.opts);
        r.expr = job.src.expr.as_deref();
        r.cache = Some(&job.src.layer_cache);
        r.accel = job.src.gpu.as_ref().map(|g| g as &dyn effectcraft_engine::render::Accelerator);
        // The GPU leaves the frame in a texture for the viewer; otherwise (Software Only, no
        // adapter, or a frame the GPU cannot finish here) the CPU renders it.
        let gpu_frame = match (&job.src.gpu, r.active_accel()) {
            (Some(g), Some(_)) => g.render_display(&r, job.comp, job.t),
            _ => None,
        };
        let ci = match gpu_frame {
            Some(f) => FrameImage::Gpu(Arc::new(f)),
            None => FrameImage::Cpu(Arc::new(to_color_image(&r.comp_frame(job.comp, job.t)))),
        };
        if job.urgent
            && let Ok(mut l) = last.lock()
        {
            *l = t0.elapsed().as_secs_f64() * 1000.0;
        }
        if let Ok(mut c) = cache.lock() {
            c.insert(job.key, ci);
        }
        if let Ok(mut inf) = inflight.lock() {
            inf.remove(&job.key);
        }
        if let Ok(mut q) = queue.lock() {
            q.running.remove(&job.key);
            if job.urgent {
                q.urgent_running -= 1;
            }
        }
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
        true
    }
}

struct Job {
    key: FrameKey,
    src: RenderSource,
    comp: ItemId,
    t: Tick,
    opts: RenderOpts,
    urgent: bool,
    seq: u64,
}

#[derive(Default)]
struct Queue {
    jobs: Vec<Job>,
    running: HashSet<FrameKey>,
    urgent_running: usize,
    seq: u64,
}
