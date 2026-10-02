//! Background frame rendering and the RAM preview cache.
//!
//! Frames render on a dedicated thread pool from immutable `Arc<Project>` snapshots, so the UI
//! never waits for the compositor. Results land in a memory-budgeted cache keyed by (project
//! revision, comp, frame, scale); the timeline draws the cached range as the green cache bar.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use effectcraft_engine::project::{ItemId, Project};
use effectcraft_engine::render::{ExprHost, FootageSource, RenderOpts, Renderer};
use effectcraft_engine::time::Tick;

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
    map: HashMap<FrameKey, Arc<egui::ColorImage>>,
    order: VecDeque<FrameKey>,
    bytes: usize,
    budget: usize,
}

impl Cache {
    fn insert(&mut self, k: FrameKey, img: Arc<egui::ColorImage>) {
        let sz = img.pixels.len() * 4;
        if self.map.insert(k, img).is_none() {
            self.order.push_back(k);
            self.bytes += sz;
        }
        while self.bytes > self.budget {
            let Some(old) = self.order.pop_front() else { break };
            if let Some(i) = self.map.remove(&old) {
                self.bytes -= i.pixels.len() * 4;
            }
        }
    }
}

/// What a render job needs (all cheap clones).
#[derive(Clone)]
pub struct RenderSource {
    pub project: Arc<Project>,
    pub footage: Arc<dyn FootageSource>,
    pub expr: Option<Arc<dyn ExprHost>>,
}

pub struct Frames {
    cache: Arc<Mutex<Cache>>,
    inflight: Arc<Mutex<HashSet<FrameKey>>>,
    pool: rayon::ThreadPool,
    ctx: Option<egui::Context>,
    /// Last render time per frame (ms), for the Info panel / perf readout.
    pub last_ms: Arc<Mutex<f64>>,
}

impl Default for Frames {
    fn default() -> Self {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 16);
        Frames {
            cache: Arc::new(Mutex::new(Cache { map: HashMap::new(), order: VecDeque::new(), bytes: 0, budget: 3 << 30 })),
            inflight: Arc::new(Mutex::new(HashSet::new())),
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
        .iter()
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

    pub fn get(&self, k: &FrameKey) -> Option<Arc<egui::ColorImage>> {
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

    pub fn clear(&self) {
        if let Ok(mut c) = self.cache.lock() {
            c.map.clear();
            c.order.clear();
            c.bytes = 0;
        }
    }

    /// Queue a frame (no-op if cached or already rendering).
    pub fn request(&self, src: &RenderSource, key: FrameKey, comp: ItemId, t: Tick, opts: RenderOpts) {
        if self.is_cached(&key) {
            return;
        }
        {
            let Ok(mut inf) = self.inflight.lock() else { return };
            if !inf.insert(key) {
                return;
            }
        }
        let src = src.clone();
        let cache = self.cache.clone();
        let inflight = self.inflight.clone();
        let ctx = self.ctx.clone();
        let last = self.last_ms.clone();
        self.pool.spawn(move || {
            let t0 = std::time::Instant::now();
            let mut r = Renderer::new(&src.project, src.footage.as_ref(), opts);
            r.expr = src.expr.as_deref();
            let img = r.comp_frame(comp, t);
            let ci = Arc::new(to_color_image(&img));
            if let Ok(mut l) = last.lock() {
                *l = t0.elapsed().as_secs_f64() * 1000.0;
            }
            if let Ok(mut c) = cache.lock() {
                c.insert(key, ci);
            }
            if let Ok(mut inf) = inflight.lock() {
                inf.remove(&key);
            }
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }
}
