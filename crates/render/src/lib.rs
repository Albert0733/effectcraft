//! The EffectCraft compositor (CPU reference path).
//!
//! `Renderer::comp_frame` renders a composition at a time:
//! bottom-to-top over visible layers → **source** (solid, footage, text, shapes, precomp) →
//! **masks** → **effects** → **transform** (2D affine or 3D projective through the active camera,
//! with motion blur sub-samples) → **track matte** → **blend** with the layer's mode and opacity.
//! Adjustment layers run their effects on everything below, limited by their own bounds/masks.

pub mod audio;
pub mod cache;
pub mod color;
pub mod eval;
pub mod masks;
pub mod shapes;
pub mod styles;
pub mod text;
pub mod three_d;

use std::sync::Arc;

pub use cache::{CacheStats, LayerCache};
use effectcraft_color::BlendMode;
use effectcraft_effects::{Buf, EffectCtx, EffectEnv, EffectHost, LayerPixels, Params};
use effectcraft_geom::{Mat3, Mat4, vec2};
use effectcraft_project::{Comp, Footage, FootageKind, FrameBlend, GroupKind, ItemId, ItemKind, Layer, LayerSource, MatteKind, Project, Quality, Sampling};
pub use effectcraft_raster::Image;
use effectcraft_raster::{WarpOpts, composite_warp};
use effectcraft_time::{FrameRate, TICKS_PER_SECOND, Tick};
pub use eval::{EvalCtx, ExprHost, source_size};
use rayon::prelude::*;

/// Supplies decoded footage frames (implemented by the media layer).
pub trait FootageSource: Send + Sync {
    /// The frame of `item` at source time `t`, straight from the file (any size).
    fn frame(&self, item: ItemId, footage: &Footage, t: Tick) -> Option<Arc<Image>>;
    /// `frames` interleaved stereo sample frames of `item`'s audio from source time `t` at
    /// `rate` Hz (`None` when unavailable).
    fn audio(&self, _item: ItemId, _footage: &Footage, _t: Tick, _frames: usize, _rate: u32) -> Option<Vec<f32>> {
        None
    }
    /// Set the decoded-frame cache budget in bytes (Settings ▸ Memory & CPU); sources without
    /// a cache ignore it.
    fn set_cache_budget(&self, _bytes: usize) {}
    /// The decoded-frame cache budget in bytes, if the source has a cache.
    fn cache_budget(&self) -> Option<usize> {
        None
    }
}

/// Layer parameters and audio for effects (see [`effectcraft_effects::EffectHost`]).
struct FxHost<'r, 'a, 'c> {
    r: &'r Renderer<'a>,
    ctx: &'c EvalCtx<'a>,
    layer: &'c Layer,
    /// Index of the effect being rendered (bounds `self_at` to effects before it).
    index: std::sync::atomic::AtomicUsize,
}

/// Nesting limit for effects that render layers (other layers, other times).
const MAX_FX_DEPTH: usize = 8;

impl EffectHost for FxHost<'_, '_, '_> {
    fn layer(&self, id: u64, masks_and_effects: bool) -> Option<LayerPixels> {
        let other = self.ctx.layer(effectcraft_project::LayerId(id))?;
        if other.id == self.layer.id || self.r.depth > MAX_FX_DEPTH {
            return None;
        }
        let sub = self.r.nested();
        // The cached layer buffer is shared; effects get their own copy.
        let buf = if masks_and_effects { (*sub.content_buf(self.ctx, other)?).clone() } else { sub.source(self.ctx, other)? };
        let size = source_size(self.r.project, other);
        let size = if size.0 == 0 { [self.ctx.comp.width as f64, self.ctx.comp.height as f64] } else { [size.0 as f64, size.1 as f64] };
        Some(LayerPixels { buf, size })
    }

    fn layer_masks(&self, id: u64) -> Option<LayerPixels> {
        let other = self.ctx.layer(effectcraft_project::LayerId(id))?;
        if other.id == self.layer.id || self.r.depth > MAX_FX_DEPTH {
            return None;
        }
        let sub = self.r.nested();
        let buf = (*sub.layer_input(self.ctx, other, 0)?).clone();
        let size = source_size(self.r.project, other);
        let size = if size.0 == 0 { [self.ctx.comp.width as f64, self.ctx.comp.height as f64] } else { [size.0 as f64, size.1 as f64] };
        Some(LayerPixels { buf, size })
    }

    fn audio(&self, id: u64, start: f64, frames: usize, rate: u32) -> Option<Vec<f32>> {
        let other = self.ctx.layer(effectcraft_project::LayerId(id))?;
        let LayerSource::Footage { item } = &other.source else { return None };
        let ItemKind::Footage(f) = &self.r.project.item(*item)?.kind else { return None };
        if !f.has_audio {
            return None;
        }
        let st = other.layer_time(Tick::from_seconds_f64(start));
        self.r.footage.audio(*item, f, st, frames, rate)
    }

    fn self_at(&self, layer_time: f64, effects: usize) -> Option<Buf> {
        if self.r.depth > MAX_FX_DEPTH || self.layer.switches.adjustment {
            return None;
        }
        let n = effects.min(self.index.load(std::sync::atomic::Ordering::Relaxed));
        let t = self.layer.comp_time(Tick::from_seconds_f64(layer_time));
        let sub = self.r.nested();
        sub.layer_input(&self.ctx.at(t), self.layer, n).map(|b| (*b).clone())
    }

    fn layer_at(&self, id: u64, comp_time: f64, masks_and_effects: bool) -> Option<LayerPixels> {
        let other = self.ctx.layer(effectcraft_project::LayerId(id))?;
        if other.id == self.layer.id || self.r.depth > MAX_FX_DEPTH {
            return None;
        }
        let ctx = self.ctx.at(Tick::from_seconds_f64(comp_time));
        let sub = self.r.nested();
        let buf = if masks_and_effects { (*sub.content_buf(&ctx, other)?).clone() } else { (*sub.layer_input(&ctx, other, 0)?).clone() };
        let size = source_size(self.r.project, other);
        let size = if size.0 == 0 { [self.ctx.comp.width as f64, self.ctx.comp.height as f64] } else { [size.0 as f64, size.1 as f64] };
        Some(LayerPixels { buf, size })
    }
}

/// No footage available (renders footage layers as transparent).
pub struct NoFootage;
impl FootageSource for NoFootage {
    fn frame(&self, _: ItemId, _: &Footage, _: Tick) -> Option<Arc<Image>> {
        None
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RenderOpts {
    /// Output scale relative to comp pixels (1 = Full, 0.5 = Half…).
    pub scale: f64,
    pub motion_blur: bool,
    /// Render guide layers (viewer only).
    pub guides: bool,
    /// Draft quality: fewer motion-blur samples, bilinear only.
    pub draft: bool,
    /// 3D view camera override (viewer Front/Top/Custom views); None = the comp's active camera.
    pub view: Option<three_d::CameraState>,
    /// Region of interest `[x, y, w, h]` in comp pixels (viewer only): the top-level frame covers
    /// just this rectangle, and only its pixels are composited.
    pub roi: Option<[f64; 4]>,
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts { scale: 1.0, motion_blur: true, guides: false, draft: false, view: None, roi: None }
    }
}

pub struct Renderer<'a> {
    pub project: &'a Project,
    pub footage: &'a dyn FootageSource,
    pub expr: Option<&'a dyn ExprHost>,
    pub opts: RenderOpts,
    /// Processed-layer cache shared across frames (see [`cache`]). `None` renders everything.
    pub cache: Option<&'a LayerCache>,
    /// Optional per-layer timing sink (`frame --bench` / perf readouts).
    pub profile: Option<&'a std::sync::Mutex<Vec<LayerTiming>>>,
    /// Current nesting depth (precomp recursion guard).
    depth: usize,
    /// The project's colour pipeline (bit depth, working space, linear blending).
    pub(crate) pipe: color::Pipe,
    /// Collapsed precomp being drawn into its parent: nested comp pixels → parent comp pixels
    /// (before the output scale).
    outer: Option<Mat3>,
    /// Opacity of the collapsed precomp layers this comp is drawn through.
    opacity_mul: f32,
    /// Collapsed precomp: nested 3D layers join the parent's 3D space.
    pub(crate) collapse3d: Option<Collapse3d<'a>>,
}

/// How a collapsed precomp's 3D layers are placed in the parent (see `Renderer::collapse_into`).
#[derive(Clone, Copy)]
pub(crate) struct Collapse3d<'a> {
    /// Nested comp world → parent world (the precomp layer's world transform).
    pub world: Mat4,
    /// The parent's camera.
    pub cam: three_d::CameraState,
    /// The outermost comp's context (its lights light the nested layers).
    pub parent: EvalCtx<'a>,
}

/// Time spent on one layer of one frame (milliseconds).
#[derive(Clone, Debug, Default)]
pub struct LayerTiming {
    pub depth: usize,
    pub layer: String,
    /// Source + masks + effects (cache lookup only, on a hit).
    pub process_ms: f64,
    /// Per effect: (effect id, ms). Empty on a cache hit.
    pub effects: Vec<(String, f64)>,
    /// Transform + matte + blend.
    pub composite_ms: f64,
    /// The processed buffer came from the layer cache.
    pub cached: bool,
}

impl<'a> Renderer<'a> {
    pub fn new(project: &'a Project, footage: &'a dyn FootageSource, opts: RenderOpts) -> Renderer<'a> {
        Renderer {
            project,
            footage,
            expr: None,
            opts,
            cache: None,
            profile: None,
            depth: 0,
            pipe: color::Pipe::of(&project.settings),
            outer: None,
            opacity_mul: 1.0,
            collapse3d: None,
        }
    }

    /// A renderer for nested work (precomps, layers read by effects): one level deeper.
    fn nested(&self) -> Renderer<'a> {
        Renderer { depth: self.depth + 1, outer: None, opacity_mul: 1.0, collapse3d: None, ..*self }
    }

    /// The nested comp of a precomp layer whose transformations collapse into this comp: the
    /// Collapse Transformations switch is on and the layer has no masks, effects or layer
    /// styles (those force a flattened render of the precomp, as in After Effects).
    pub(crate) fn collapsed(&self, ctx: &EvalCtx, layer: &Layer) -> Option<ItemId> {
        if !layer.switches.collapse || layer.switches.adjustment || self.depth > 12 {
            return None;
        }
        let LayerSource::Comp { item } = &layer.source else { return None };
        if self.project.comp_contains(*item, ctx.comp_id) || self.project.comp(*item).is_none() {
            return None;
        }
        if layer.masks().is_some_and(|m| !m.children.is_empty()) {
            return None;
        }
        if layer.switches.effects && layer.effects().is_some_and(|fx| fx.groups().any(|g| g.enabled)) {
            return None;
        }
        if styles::active(ctx, layer) {
            return None;
        }
        Some(*item)
    }

    /// A renderer and context that draw a collapsed precomp's layers straight into this comp:
    /// their transforms are concatenated with the precomp layer's (one resample, no
    /// rasterisation at the precomp's bounds), their opacity multiplied by `opacity`, and their
    /// 3D layers use this comp's camera and lights.
    pub(crate) fn collapse_into(&self, ctx: &EvalCtx<'a>, layer: &Layer, item: ItemId, opacity: f32) -> Option<(Renderer<'a>, EvalCtx<'a>)> {
        let nc = self.project.comp(item)?;
        let nctx = EvalCtx { comp_id: item, comp: nc, time: ctx.source_time(layer), ..*ctx };
        let (l2c, _) = ctx.layer_to_comp(layer);
        let outer = Some(self.outer.map_or(l2c, |o| o * l2c));
        let world = ctx.world_matrix(layer);
        let c3 = Collapse3d {
            world: self.collapse3d.map_or(world, |c| c.world * world),
            cam: three_d::compose::camera_for(self, ctx),
            parent: self.collapse3d.map_or(*ctx, |c| c.parent),
        };
        Some((Renderer { depth: self.depth + 1, outer, opacity_mul: opacity, collapse3d: Some(c3), ..*self }, nctx))
    }

    pub(crate) fn opacity_mul(&self) -> f32 {
        self.opacity_mul
    }

    /// Draw a collapsed 2D precomp layer: nested layers blend straight into `canvas` with their
    /// own modes (Normal precomp, no matte); otherwise they are drawn into an isolated buffer
    /// that then takes the precomp layer's track matte, Preserve Transparency and blend mode.
    fn draw_collapsed(&self, ctx: &EvalCtx<'a>, layer: &Layer, item: ItemId, canvas: &mut Image) {
        let opacity = ctx.opacity(layer) as f32 * self.opacity_mul;
        if opacity <= 0.0 {
            return;
        }
        let matte = layer.track_matte.is_some_and(|tm| tm.layer != layer.id && ctx.comp.layer(tm.layer).is_some());
        if layer.blend_mode == BlendMode::Normal && !matte && !layer.preserve_transparency {
            if let Some((sub, nctx)) = self.collapse_into(ctx, layer, item, opacity) {
                sub.draw_comp(&nctx, canvas);
            }
            return;
        }
        let Some((sub, nctx)) = self.collapse_into(ctx, layer, item, 1.0) else { return };
        let mut iso = Image::new(canvas.width, canvas.height);
        sub.draw_comp(&nctx, &mut iso);
        self.composite_iso(ctx, layer, iso, canvas, opacity);
    }

    /// Rasterisation scale of a layer's source: the output scale, or — for Continuously
    /// Rasterize text and shape layers — the output scale times the layer's on-screen scale
    /// (quarter-octave steps, rounded up, so vector content is drawn at its final size and
    /// stays sharp when scaled up).
    pub fn raster_scale(&self, ctx: &EvalCtx, layer: &Layer) -> f64 {
        let s = self.opts.scale;
        if !layer.switches.collapse || !matches!(layer.source, LayerSource::Text | LayerSource::Shape) {
            return s;
        }
        let (l2c, _) = ctx.layer_to_comp(layer);
        let m = self.outer.map_or(l2c, |o| o * l2c);
        let a = layer.transform().map(|tr| ctx.v3(layer, tr, "anchor", [0.0; 3])).unwrap_or([0.0; 3]);
        let p0 = m.apply(vec2(a[0], a[1]));
        let px = m.apply(vec2(a[0] + 1.0, a[1]));
        let py = m.apply(vec2(a[0], a[1] + 1.0));
        let k = ((px.x - p0.x).hypot(px.y - p0.y)).max((py.x - p0.x).hypot(py.y - p0.y));
        if !k.is_finite() || k <= 1e-6 {
            return s;
        }
        let mut k = 2f64.powf((k.log2() * 4.0).ceil() / 4.0).clamp(1.0 / 16.0, 64.0);
        // Keep the buffer within about 16 million pixels.
        if let Some(b) = content_bounds(ctx, layer) {
            let area = ((b[2] - b[0]) * (b[3] - b[1])).max(1.0) * s * s;
            k = k.min((16.0e6 / area).sqrt().max(1.0));
        }
        s * k
    }

    /// Draw a Wireframe-quality layer: its bounds as a one-pixel outline.
    fn draw_wireframe(&self, ctx: &EvalCtx, layer: &Layer, canvas: &mut Image) {
        let Some(b) = content_bounds(ctx, layer) else { return };
        let s = self.opts.scale;
        let (l2c, _) = ctx.layer_to_comp(layer);
        let m = Mat3::scale(vec2(s, s)) * self.outer.map_or(l2c, |o| o * l2c);
        let c = [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])].map(|(x, y)| m.apply(vec2(x, y)));
        let (w, h) = (canvas.width as i64, canvas.height as i64);
        for i in 0..4 {
            let (p, q) = (c[i], c[(i + 1) % 4]);
            let n = ((q.x - p.x).abs().max((q.y - p.y).abs()) * 2.0).ceil().clamp(1.0, 1.0e5) as usize;
            for j in 0..=n {
                let t = j as f64 / n as f64;
                let (x, y) = ((p.x + (q.x - p.x) * t).floor() as i64, (p.y + (q.y - p.y) * t).floor() as i64);
                if x >= 0 && y >= 0 && x < w && y < h {
                    canvas.set(x as u32, y as u32, [1.0, 1.0, 1.0, 1.0]);
                }
            }
        }
    }

    fn ctx(&self, comp_id: ItemId, comp: &'a Comp, t: Tick) -> EvalCtx<'a> {
        EvalCtx { project: self.project, comp_id, comp, time: t, expr: self.expr }
    }

    /// Render a composition at comp time `t` (transparent background, comp size × scale).
    pub fn comp_frame(&self, comp_id: ItemId, t: Tick) -> Image {
        let Some(comp) = self.project.comp(comp_id) else { return Image::new(1, 1) };
        let s = self.opts.scale;
        let w = ((comp.width as f64 * s).round() as u32).max(1);
        let h = ((comp.height as f64 * s).round() as u32).max(1);
        // Region of interest on a comp with 3D layers: render the frame and crop it.
        if self.depth == 0
            && let Some(r) = self.opts.roi
            && comp.has_3d()
        {
            let full = Renderer { opts: RenderOpts { roi: None, ..self.opts }, ..*self }.comp_frame(comp_id, t);
            let (x0, y0) = ((r[0] * s).round().max(0.0) as u32, (r[1] * s).round().max(0.0) as u32);
            let (rw, rh) = (((r[2] * s).round() as u32).max(1), ((r[3] * s).round() as u32).max(1));
            let mut out = Image::new(rw, rh);
            for y in 0..rh.min(full.height.saturating_sub(y0)) {
                for x in 0..rw.min(full.width.saturating_sub(x0)) {
                    out.data[(y * rw + x) as usize] = full.data[((y + y0) * full.width + x + x0) as usize];
                }
            }
            return out;
        }
        let (w, h) = match self.roi_offset() {
            Some((_, _, rw, rh)) => (rw, rh),
            None => (w, h),
        };
        let mut canvas = Image::new(w, h);
        if self.depth > 16 {
            return canvas;
        }
        let ctx = self.ctx(comp_id, comp, t);
        self.draw_comp(&ctx, &mut canvas);
        if let Some(c) = self.pipe.from_blend() {
            color::convert(&mut canvas, &c);
        }
        self.pipe.quantize(&mut canvas);
        // The top-level comp leaves the working space for the (sRGB) display / output.
        if self.depth == 0
            && let Some(c) = self.pipe.output()
        {
            color::convert(&mut canvas, &c);
            self.pipe.quantize(&mut canvas);
        }
        canvas
    }

    /// Draw a comp's layers (bottom to top) into `canvas` (blending space).
    fn draw_comp(&self, ctx: &EvalCtx<'a>, canvas: &mut Image) {
        let (comp, t) = (ctx.comp, ctx.time);
        let any_solo = comp.layers.iter().any(|l| l.switches.solo && l.source.is_av() && l.is_active_at(t));
        // Bottom-to-top, with runs of consecutive 3D layers depth-sorted (farthest first).
        let visible: Vec<&Layer> = comp
            .layers
            .iter()
            .rev()
            .filter(|l| l.is_active_at(t) && l.has_video() && (!any_solo || l.switches.solo) && (self.opts.guides || !l.switches.guide))
            .collect();
        let mut i = 0;
        while i < visible.len() {
            if visible[i].is_3d() {
                let mut j = i;
                while j < visible.len() && visible[j].is_3d() {
                    j += 1;
                }
                three_d::compose::draw_run(self, ctx, &visible[i..j], canvas);
                i = j;
            } else {
                match self.collapsed(ctx, visible[i]) {
                    Some(item) => self.draw_collapsed(ctx, visible[i], item, canvas),
                    None => self.draw_layer(ctx, visible[i], canvas),
                }
                i += 1;
            }
            // 8/16 bpc: the comp is an integer buffer after every layer.
            self.pipe.quantize(canvas);
        }
    }

    /// Evaluate an effect instance's parameters.
    fn effect_params(&self, ctx: &EvalCtx, layer: &Layer, g: &effectcraft_project::PropGroup) -> Params {
        // Nested groups (Paint strokes, Puppet meshes and pins) are flattened too.
        effectcraft_effects::flatten_params(g, &mut |pr| ctx.value(layer, pr))
    }

    fn apply_effects(&self, ctx: &EvalCtx, layer: &Layer, buf: Buf, adjustment: bool) -> Buf {
        self.apply_effects_timed(ctx, layer, buf, adjustment, usize::MAX, None)
    }

    /// Run the first `limit` effects of the layer's stack (by position; disabled ones count).
    fn apply_effects_timed(
        &self,
        ctx: &EvalCtx,
        layer: &Layer,
        mut buf: Buf,
        adjustment: bool,
        limit: usize,
        mut timing: Option<&mut Vec<(String, f64)>>,
    ) -> Buf {
        if !layer.switches.effects {
            return buf;
        }
        let Some(fx) = layer.effects() else { return buf };
        let lt = layer.layer_time(ctx.time);
        let size = source_size(self.project, layer);
        let layer_size = if size.0 == 0 { [ctx.comp.width as f64, ctx.comp.height as f64] } else { [size.0 as f64, size.1 as f64] };
        let mask_shapes = masks::shapes(ctx, layer);
        let host = FxHost { r: self, ctx, layer, index: Default::default() };
        let env =
            EffectEnv { masks: &mask_shapes, host: Some(&host), comp_time: ctx.time.seconds(), frame_rate: ctx.comp.frame_rate.as_f64(), effect_index: 0 };
        for (i, g) in fx.groups().enumerate() {
            if i >= limit {
                break;
            }
            if !g.enabled {
                continue;
            }
            let GroupKind::Effect { effect } = &g.kind else { continue };
            let Some(spec) = effectcraft_effects::find(effect) else { continue };
            if effectcraft_effects::audio_fx::is_audio_effect(spec.id) {
                continue;
            }
            let params = self.effect_params(ctx, layer, g);
            host.index.store(i, std::sync::atomic::Ordering::Relaxed);
            let env = EffectEnv { effect_index: i, ..env };
            let ectx = EffectCtx { params: &params, time: lt.seconds(), layer_size, seed: g.uid as u32, adjustment, env };
            let t0 = web_time::Instant::now();
            buf = effectcraft_effects::apply(spec, &ectx, buf);
            // 8/16 bpc effects write integer pixels.
            self.pipe.quantize(&mut buf.img);
            if let Some(v) = timing.as_deref_mut() {
                v.push((spec.id.to_string(), t0.elapsed().as_secs_f64() * 1e3));
            }
        }
        buf
    }

    /// Source pixels of a layer in layer space (before masks/effects).
    fn source(&self, ctx: &EvalCtx, layer: &Layer) -> Option<Buf> {
        let s = self.opts.scale;
        match &layer.source {
            LayerSource::Solid { item } => {
                let ItemKind::Solid(sol) = &self.project.item(*item)?.kind else { return None };
                let w = ((sol.width as f64 * s).ceil() as u32).max(1);
                let h = ((sol.height as f64 * s).ceil() as u32).max(1);
                if layer.switches.adjustment {
                    return Some(Buf { img: Image::filled(w, h, [1.0; 4]), offset: [0.0; 2], scale: s });
                }
                let mut c = sol.color;
                if let Some(conv) = self.pipe.authored_in() {
                    c = conv.apply(c);
                }
                Some(Buf { img: Image::filled(w, h, [c[0], c[1], c[2], 1.0]), offset: [0.0; 2], scale: s })
            }
            LayerSource::Comp { item } => {
                if self.project.comp_contains(*item, ctx.comp_id) {
                    return None;
                }
                let sub = self.nested();
                let lt = ctx.source_time(layer);
                // Frame blending between the nested comp's frames (time-stretched/remapped).
                let mode = frame_blend_mode(ctx, layer);
                if mode != FrameBlend::Off
                    && let Some(nc) = self.project.comp(*item)
                {
                    let (i, w) = frame_position(lt, nc.frame_rate);
                    if w > 1e-6 {
                        let a = sub.comp_frame(*item, nc.frame_rate.tick_of(i));
                        let b = sub.comp_frame(*item, nc.frame_rate.tick_of(i + 1));
                        return Some(Buf { img: blend_frames(mode, &a, &b, w as f32), offset: [0.0; 2], scale: s });
                    }
                }
                Some(Buf { img: sub.comp_frame(*item, lt), offset: [0.0; 2], scale: s })
            }
            LayerSource::Footage { item } => {
                let it = self.project.item(*item)?;
                let ItemKind::Footage(f) = &it.kind else { return None };
                if !f.has_video {
                    return None;
                }
                let lt = ctx.source_time(layer);
                let img = self.footage_frame(ctx, layer, *item, f, lt)?;
                let k = if img.width > 0 { f.width.max(1) as f64 / img.width as f64 } else { 1.0 };
                // Keep native pixels when downsampling is small; resample otherwise.
                let mut buf = if s < 0.75 {
                    let w = ((f.width as f64 * s).round() as u32).max(1);
                    let h = ((f.height as f64 * s).round() as u32).max(1);
                    Buf { img: effectcraft_raster::resample(&img, w, h), offset: [0.0; 2], scale: s }
                } else {
                    Buf { img: (*img).clone(), offset: [0.0; 2], scale: 1.0 / k }
                };
                if let Some(c) = self.pipe.media_in(f.color_profile) {
                    color::convert(&mut buf.img, &c);
                }
                Some(buf)
            }
            LayerSource::Text => Some(self.authored(text::render(ctx, layer, self.raster_scale(ctx, layer)))),
            LayerSource::Shape => layer.props.sub("contents").map(|c| self.authored(shapes::render(ctx, layer, c, self.raster_scale(ctx, layer)))),
            _ => None,
        }
    }

    /// Authored colours into the working space (linear working spaces).
    fn authored(&self, mut buf: Buf) -> Buf {
        if let Some(c) = self.pipe.authored_in() {
            color::convert(&mut buf.img, &c);
        }
        buf
    }

    /// A footage frame at source time `t`, frame-blended between the two nearest source frames
    /// when the layer's Frame Blending switch (and the comp's Enable Frame Blending) is on and
    /// `t` falls between frames (footage rate ≠ comp rate, time stretch, time remapping).
    fn footage_frame(&self, ctx: &EvalCtx, layer: &Layer, item: ItemId, f: &Footage, t: Tick) -> Option<Arc<Image>> {
        let mode = frame_blend_mode(ctx, layer);
        if mode != FrameBlend::Off && matches!(f.kind, FootageKind::Video | FootageKind::Sequence) {
            let (i, w) = frame_position(t, f.frame_rate);
            if w > 1e-6 {
                // Blended frames are cached (Pixel Motion is expensive).
                let key = self.cache.map(|_| {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    (0x00f8_a3e5_u64, item, i, (w as f32).to_bits(), mode as u8, f.frame_rate.num, f.frame_rate.den, &f.path).hash(&mut h);
                    h.finish()
                });
                if let (Some(c), Some(k)) = (self.cache, key)
                    && let Some(b) = c.get(k)
                {
                    return Some(Arc::new(b.img.clone()));
                }
                let a = self.footage.frame(item, f, f.frame_rate.tick_of(i));
                let b = self.footage.frame(item, f, f.frame_rate.tick_of(i + 1));
                if let (Some(a), Some(b)) = (a, b) {
                    let img = blend_frames(mode, &a, &b, w as f32);
                    if let (Some(c), Some(k)) = (self.cache, key) {
                        c.insert(k, Arc::new(Buf { img: img.clone(), offset: [0.0; 2], scale: 1.0 }));
                    }
                    return Some(Arc::new(img));
                }
            }
        }
        self.footage.frame(item, f, t)
    }

    /// Source → masks, clamped/quantised to the project depth.
    fn masked_source(&self, ctx: &EvalCtx, layer: &Layer) -> Option<Buf> {
        let mut buf = self.source(ctx, layer)?;
        masks::apply(ctx, layer, &mut buf);
        self.pipe.quantize(&mut buf.img);
        Some(buf)
    }

    /// Fully processed layer buffer (source → masks → effects → layer styles, flattened).
    /// Served from the layer cache when the layer's content key matches.
    pub fn layer_buf(&self, ctx: &EvalCtx, layer: &Layer) -> Option<Arc<Buf>> {
        self.layer_buf_keyed(ctx, layer).map(|(b, _)| b)
    }

    /// [`Self::layer_buf`] in the blending space (linear with Blend Colors Using 1.0 Gamma).
    pub(crate) fn blend_layer_buf(&self, ctx: &EvalCtx, layer: &Layer) -> Option<Arc<Buf>> {
        let (b, k) = self.layer_buf_keyed(ctx, layer)?;
        Some(self.to_blend(b, k))
    }

    /// A buffer converted to the blending space (cached under a key derived from `key`).
    pub(crate) fn to_blend(&self, b: Arc<Buf>, key: Option<u64>) -> Arc<Buf> {
        let Some(c) = self.pipe.to_blend() else { return b };
        let key = key.map(|k| cache::derive(k, 0x1ea7));
        if let (Some(cache), Some(k)) = (self.cache, key)
            && let Some(x) = cache.get(k)
        {
            return x;
        }
        let mut nb = (*b).clone();
        color::convert(&mut nb.img, &c);
        let nb = Arc::new(nb);
        if let (Some(cache), Some(k)) = (self.cache, key) {
            cache.insert(k, nb.clone());
        }
        nb
    }

    fn layer_buf_keyed(&self, ctx: &EvalCtx, layer: &Layer) -> Option<(Arc<Buf>, Option<u64>)> {
        let st = self.styled(ctx, layer, None)?;
        if st.passes.is_empty() {
            return Some((st.body, st.key));
        }
        let key = st.key.map(|k| cache::derive(k, 0xf1a7));
        if let (Some(c), Some(k)) = (self.cache, key)
            && let Some(b) = c.get(k)
        {
            return Some((b, key));
        }
        let flat = Arc::new(styles::Styled { passes: st.passes.iter().map(|(b, m)| ((**b).clone(), *m)).collect(), body: (*st.body).clone() }.flatten());
        if let (Some(c), Some(k)) = (self.cache, key) {
            c.insert(k, flat.clone());
        }
        Some((flat, key))
    }

    /// A layer's source pixels at the context time, before masks, effects and the transform
    /// (what the motion tracker analyses, like After Effects' Layer panel). `None` for layers
    /// without pixels.
    pub fn layer_source(&self, ctx: &EvalCtx, layer: &Layer) -> Option<Buf> {
        self.source(ctx, layer)
    }

    /// Layer pixels after source → masks → effects (no layer styles).
    pub fn content_buf(&self, ctx: &EvalCtx, layer: &Layer) -> Option<Arc<Buf>> {
        self.content_buf_timed(ctx, layer, None).map(|(b, _)| b)
    }

    fn content_buf_timed(&self, ctx: &EvalCtx, layer: &Layer, mut timing: Option<&mut LayerTiming>) -> Option<(Arc<Buf>, Option<u64>)> {
        let key = self.cache.and_then(|_| cache::layer_key(ctx, layer, self.raster_scale(ctx, layer), self.opts.draft));
        if let (Some(c), Some(k)) = (self.cache, key)
            && let Some(b) = c.get(k)
        {
            if let Some(t) = timing.as_deref_mut() {
                t.cached = true;
            }
            return Some((b, key));
        }
        let buf = self.masked_source(ctx, layer)?;
        let buf = Arc::new(self.apply_effects_timed(ctx, layer, buf, false, usize::MAX, timing.map(|t| &mut t.effects)));
        if let (Some(c), Some(k)) = (self.cache, key) {
            c.insert(k, buf.clone());
        }
        Some((buf, key))
    }

    /// A layer's input at the context time: source → masks → its first `effects` effects
    /// (what Time effects read at neighbouring times). Served from / stored in the layer cache
    /// under its own key, so scrubbing reuses frames rendered for earlier output frames.
    pub fn layer_input(&self, ctx: &EvalCtx, layer: &Layer, effects: usize) -> Option<Arc<Buf>> {
        let key = self.cache.and_then(|_| cache::input_key(ctx, layer, self.raster_scale(ctx, layer), self.opts.draft, effects));
        if let (Some(c), Some(k)) = (self.cache, key)
            && let Some(b) = c.get(k)
        {
            return Some(b);
        }
        let mut buf = self.masked_source(ctx, layer)?;
        if effects > 0 {
            buf = self.apply_effects_timed(ctx, layer, buf, false, effects, None);
        }
        let buf = Arc::new(buf);
        if let (Some(c), Some(k)) = (self.cache, key) {
            c.insert(k, buf.clone());
        }
        Some(buf)
    }

    /// The layer with its styles: exterior passes and body (cached per pass).
    fn styled(&self, ctx: &EvalCtx, layer: &Layer, mut timing: Option<&mut LayerTiming>) -> Option<StyledLayer> {
        let (content, ckey) = self.content_buf_timed(ctx, layer, timing.as_deref_mut())?;
        if !styles::active(ctx, layer) {
            return Some(StyledLayer { passes: vec![], body: content.clone(), content, key: ckey, ckey, plain: true });
        }
        let modes = styles::pass_modes(ctx, layer);
        let key = ckey.map(|k| cache::styles_key(ctx, layer, k));
        if let (Some(c), Some(k)) = (self.cache, key)
            && let Some(body) = c.get(k)
        {
            let passes: Option<Vec<_>> = modes.iter().enumerate().map(|(i, m)| c.get(cache::derive(k, i as u64 + 1)).map(|b| (b, *m))).collect();
            if let Some(passes) = passes {
                return Some(StyledLayer { passes, body, content, key, ckey, plain: false });
            }
        }
        let t0 = web_time::Instant::now();
        let size = source_size(self.project, layer);
        let layer_size = if size.0 == 0 { [ctx.comp.width as f64, ctx.comp.height as f64] } else { [size.0 as f64, size.1 as f64] };
        let st = styles::render(ctx, layer, &content, layer_size);
        if let Some(t) = timing {
            t.cached = false;
            t.effects.push(("layerStyles".into(), t0.elapsed().as_secs_f64() * 1e3));
        }
        let body = Arc::new(st.body);
        let passes: Vec<_> = st.passes.into_iter().map(|(b, m)| (Arc::new(b), m)).collect();
        if let (Some(c), Some(k)) = (self.cache, key) {
            c.insert(k, body.clone());
            for (i, (b, _)) in passes.iter().enumerate() {
                c.insert(cache::derive(k, i as u64 + 1), b.clone());
            }
        }
        Some(StyledLayer { passes, body, content, key, ckey, plain: false })
    }

    /// The styled layer's buffers in the blending space.
    fn styled_to_blend(&self, st: StyledLayer) -> StyledLayer {
        if self.pipe.to_blend().is_none() {
            return st;
        }
        let body = self.to_blend(st.body, st.key);
        let content = if st.plain { body.clone() } else { self.to_blend(st.content, st.ckey) };
        let passes = st.passes.into_iter().enumerate().map(|(i, (b, m))| (self.to_blend(b, st.key.map(|k| cache::derive(k, i as u64 + 1))), m)).collect();
        StyledLayer { passes, body, content, ..st }
    }

    fn sampling(&self, layer: &Layer) -> effectcraft_raster::Sampling {
        if layer.switches.quality == Quality::Draft {
            // Draft quality: no interpolation (nearest neighbour), as After Effects' Draft.
            effectcraft_raster::Sampling::Nearest
        } else if self.opts.draft {
            effectcraft_raster::Sampling::Bilinear
        } else if layer.switches.sampling == Sampling::Bicubic {
            effectcraft_raster::Sampling::Bicubic
        } else {
            effectcraft_raster::Sampling::Bilinear
        }
    }

    /// The region of interest of a top-level 2D frame: (x, y) offset of the output in output
    /// pixels and its size. `None` renders the whole comp (no ROI, or a nested comp).
    fn roi_offset(&self) -> Option<(f64, f64, u32, u32)> {
        let r = self.opts.roi.filter(|_| self.depth == 0)?;
        let s = self.opts.scale;
        if r[2] <= 0.0 || r[3] <= 0.0 {
            return None;
        }
        Some(((r[0] * s).round(), (r[1] * s).round(), ((r[2] * s).round() as u32).max(1), ((r[3] * s).round() as u32).max(1)))
    }

    /// Scaled comp pixel → output pixel: the region of interest's offset.
    fn out_matrix(&self) -> Mat3 {
        match self.roi_offset() {
            Some((x, y, _, _)) => Mat3::translate(vec2(-x, -y)),
            None => Mat3::IDENTITY,
        }
    }

    /// Buffer pixel → output pixel matrix for the layer at the context time.
    fn buf_matrix(&self, ctx: &EvalCtx, layer: &Layer, buf: &Buf) -> Mat3 {
        let s = self.opts.scale;
        let (l2c, _) = ctx.layer_to_comp(layer);
        let l2c = self.outer.map_or(l2c, |o| o * l2c);
        self.out_matrix()
            * Mat3::scale(vec2(s, s))
            * l2c
            * Mat3::scale(vec2(1.0 / buf.scale, 1.0 / buf.scale))
            * Mat3::translate(vec2(-buf.offset[0], -buf.offset[1]))
    }

    /// Draw a processed layer into `target` (transform, motion blur) with `mode`/`opacity`.
    fn place(&self, ctx: &EvalCtx, layer: &Layer, buf: &Buf, target: &mut Image, mode: BlendMode, opacity: f32) {
        let samples = if self.opts.motion_blur && ctx.comp.enable_motion_blur && layer.switches.motion_blur {
            if self.opts.draft { 4 } else { ctx.comp.motion_blur_samples.clamp(2, 64) as usize }
        } else {
            1
        };
        let opts = WarpOpts { sampling: self.sampling(layer), opacity, mode, seed: layer.id.0 as u32, clip: None };
        if samples <= 1 {
            let m = self.buf_matrix(ctx, layer, buf);
            composite_warp(target, &buf.img, &m, &opts);
            return;
        }
        let fd = ctx.comp.frame_duration().seconds();
        let angle = ctx.comp.shutter_angle / 360.0;
        let phase = ctx.comp.shutter_phase / 360.0;
        // Sub-samples sum straight into one buffer (each warp is row-parallel).
        let mut acc = Image::new(target.width, target.height);
        let k = 1.0 / samples as f32;
        for i in 0..samples {
            let f = phase + angle * i as f64 / (samples - 1) as f64;
            let sub = ctx.at(ctx.time + Tick::from_seconds_f64(f * fd));
            let m = self.buf_matrix(&sub, layer, buf);
            effectcraft_raster::accumulate_warp(&mut acc, &buf.img, &m, opts.sampling, k);
        }
        target.blend_from(&acc, mode, opacity, layer.id.0 as u32);
    }

    fn draw_layer(&self, ctx: &EvalCtx, layer: &Layer, canvas: &mut Image) {
        let opacity = ctx.opacity(layer) as f32 * self.opacity_mul;
        if layer.switches.quality == Quality::Wireframe && !layer.switches.adjustment {
            self.draw_wireframe(ctx, layer, canvas);
            return;
        }
        if opacity <= 0.0 && !layer.switches.adjustment {
            return;
        }
        if layer.switches.adjustment {
            self.draw_adjustment(ctx, layer, canvas, opacity);
            return;
        }
        let mut timing = self.profile.map(|_| LayerTiming { depth: self.depth, layer: layer.name.clone(), ..Default::default() });
        let t0 = web_time::Instant::now();
        let Some(st) = self.styled(ctx, layer, timing.as_mut()) else { return };
        let st = self.styled_to_blend(st);
        let t1 = web_time::Instant::now();
        if st.plain {
            self.composite_layer(ctx, layer, &st.body, canvas, opacity);
        } else {
            self.composite_styled(ctx, layer, &st, canvas, opacity);
        }
        if let (Some(prof), Some(mut timing)) = (self.profile, timing) {
            timing.process_ms = (t1 - t0).as_secs_f64() * 1e3;
            timing.composite_ms = t1.elapsed().as_secs_f64() * 1e3;
            if let Ok(mut v) = prof.lock() {
                v.push(timing);
            }
        }
    }

    /// Composite a styled layer: exterior passes with their own modes, then the body with the
    /// layer's mode; the layer's opacity fades the whole stack. Knockout clears what is below the
    /// layer's content; switched-off R/G/B channels keep the values below.
    fn composite_styled(&self, ctx: &EvalCtx, layer: &Layer, st: &StyledLayer, canvas: &mut Image, opacity: f32) {
        let bl = styles::blending(ctx, layer);
        let matte = layer.track_matte.is_some_and(|tm| tm.layer != layer.id && ctx.comp.layer(tm.layer).is_some());
        if matte || layer.preserve_transparency {
            let mut iso = Image::new(canvas.width, canvas.height);
            for (b, m) in &st.passes {
                self.place(ctx, layer, b, &mut iso, *m, 1.0);
            }
            self.place(ctx, layer, &st.body, &mut iso, BlendMode::Normal, 1.0);
            self.composite_iso(ctx, layer, iso, canvas, opacity);
            return;
        }
        let mut tmp = canvas.clone();
        if bl.knockout > 0 {
            let mut k = Image::new(canvas.width, canvas.height);
            self.place(ctx, layer, &st.content, &mut k, BlendMode::Normal, 1.0);
            tmp.data.par_iter_mut().zip(k.data.par_iter()).for_each(|(p, q)| {
                let keep = 1.0 - q[3].clamp(0.0, 1.0);
                for c in p.iter_mut() {
                    *c *= keep;
                }
            });
        }
        for (b, m) in &st.passes {
            self.place(ctx, layer, b, &mut tmp, *m, 1.0);
        }
        self.place(ctx, layer, &st.body, &mut tmp, layer.blend_mode, 1.0);
        let ch = bl.channels;
        canvas.data.par_iter_mut().zip(tmp.data.par_iter()).for_each(|(c, t)| {
            for i in 0..4 {
                let v = if i < 3 && !ch[i] { c[i] } else { t[i] };
                c[i] += (v - c[i]) * opacity;
            }
        });
    }

    fn composite_layer(&self, ctx: &EvalCtx, layer: &Layer, buf: &Buf, canvas: &mut Image, opacity: f32) {
        let matte = layer.track_matte.is_some_and(|tm| tm.layer != layer.id && ctx.comp.layer(tm.layer).is_some());
        if !matte && !layer.preserve_transparency {
            self.place(ctx, layer, buf, canvas, layer.blend_mode, opacity);
            return;
        }
        // Render in isolation, then matte / preserve transparency, then blend.
        let mut iso = Image::new(canvas.width, canvas.height);
        self.place(ctx, layer, buf, &mut iso, BlendMode::Normal, 1.0);
        self.composite_iso(ctx, layer, iso, canvas, opacity);
    }

    /// Apply the track matte / preserve transparency to an isolated layer render, then blend.
    fn composite_iso(&self, ctx: &EvalCtx, layer: &Layer, mut iso: Image, canvas: &mut Image, opacity: f32) {
        let matte = layer.track_matte.and_then(|tm| ctx.comp.layer(tm.layer).filter(|m| m.id != layer.id).map(|m| (m, tm.kind)));
        let preserve = layer.preserve_transparency;
        if let Some((m, kind)) = matte {
            let mut mimg = Image::new(canvas.width, canvas.height);
            if m.is_active_at(ctx.time)
                && let Some(mb) = self.layer_buf(ctx, m)
            {
                let mo = ctx.opacity(m) as f32;
                self.place(ctx, m, &mb, &mut mimg, BlendMode::Normal, mo);
            }
            iso.data.par_iter_mut().zip(mimg.data.par_iter()).for_each(|(p, q)| {
                let k = match kind {
                    MatteKind::Alpha => q[3],
                    MatteKind::AlphaInverted => 1.0 - q[3],
                    MatteKind::Luma => effectcraft_color::luminance(q[0], q[1], q[2]),
                    MatteKind::LumaInverted => 1.0 - effectcraft_color::luminance(q[0], q[1], q[2]),
                }
                .clamp(0.0, 1.0);
                for c in p.iter_mut() {
                    *c *= k;
                }
            });
        }
        if preserve {
            iso.data.par_iter_mut().zip(canvas.data.par_iter()).for_each(|(p, q)| {
                for c in p.iter_mut() {
                    *c *= q[3];
                }
            });
        }
        canvas.blend_from(&iso, layer.blend_mode, opacity, layer.id.0 as u32);
    }

    /// Adjustment layer: effects applied to the comp below, limited to the layer's footprint.
    fn draw_adjustment(&self, ctx: &EvalCtx, layer: &Layer, canvas: &mut Image, opacity: f32) {
        let Some(mut foot) = self.source(ctx, layer) else { return };
        masks::apply(ctx, layer, &mut foot);
        let mut matte = Image::new(canvas.width, canvas.height);
        self.place(ctx, layer, &foot, &mut matte, BlendMode::Normal, 1.0);
        let o = self.roi_offset().map(|(x, y, _, _)| [-x, -y]).unwrap_or([0.0; 2]);
        let mut below = Buf { img: canvas.clone(), offset: o, scale: self.opts.scale };
        // Effects run in the working space, not the linear blending space.
        if let Some(c) = self.pipe.from_blend() {
            color::convert(&mut below.img, &c);
        }
        let mut adjusted = self.apply_effects(ctx, layer, below, true);
        if let Some(c) = self.pipe.to_blend() {
            color::convert(&mut adjusted.img, &c);
        }
        if adjusted.img.width != canvas.width || adjusted.img.height != canvas.height {
            return;
        }
        canvas.data.par_iter_mut().zip(adjusted.img.data.par_iter()).zip(matte.data.par_iter()).for_each(|((c, a), m)| {
            let k = m[3] * opacity;
            for i in 0..4 {
                c[i] += (a[i] - c[i]) * k;
            }
        });
    }
}

/// A layer ready to composite (see [`Renderer::styled`]).
struct StyledLayer {
    passes: Vec<(Arc<Buf>, BlendMode)>,
    body: Arc<Buf>,
    /// Pre-style pixels (knockout shape).
    content: Arc<Buf>,
    /// Styled cache key (the content key when plain).
    key: Option<u64>,
    /// Content cache key.
    ckey: Option<u64>,
    /// No layer styles: composite `body` the usual way.
    plain: bool,
}

/// The layer's frame blending mode, if the comp's Enable Frame Blending switch is on.
fn frame_blend_mode(ctx: &EvalCtx, layer: &Layer) -> FrameBlend {
    if ctx.comp.enable_frame_blending { layer.switches.frame_blend } else { FrameBlend::Off }
}

/// Position of source time `t` among frames at `rate`: the frame at or before `t` and the
/// fraction (0..1) of the way to the next frame. Exact (integer tick arithmetic), so frames that
/// line up exactly report a fraction of 0.
pub fn frame_position(t: Tick, rate: FrameRate) -> (i64, f64) {
    let num = t.0 as i128 * rate.num as i128;
    let den = TICKS_PER_SECOND as i128 * rate.den as i128;
    (num.div_euclid(den) as i64, num.rem_euclid(den) as f64 / den as f64)
}

/// Blend two neighbouring frames `w` of the way from `a` to `b`: a cross-fade (Frame Mix) or a
/// motion-compensated interpolation (Pixel Motion).
pub fn blend_frames(mode: FrameBlend, a: &Image, b: &Image, w: f32) -> Image {
    match mode {
        FrameBlend::PixelMotion => effectcraft_raster::flow::interpolate(a, b, w),
        _ => effectcraft_raster::flow::mix(a, b, w),
    }
}

/// Convenience: render one frame of a comp with no footage source.
pub fn render_frame(project: &Project, comp: ItemId, t: Tick, scale: f64) -> Image {
    Renderer::new(project, &NoFootage, RenderOpts { scale, ..Default::default() }).comp_frame(comp, t)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_remap;
#[cfg(test)]
mod tests_roi;
#[cfg(test)]
mod tests_styles;

/// Layer-space bounds `[x0, y0, x1, y1]` of a layer's content at the context time (source size
/// for solids/footage/precomps, glyph bounds for text, painted bounds for shapes). Used for viewer
/// handles and hit testing.
pub fn content_bounds(ctx: &EvalCtx, layer: &effectcraft_project::Layer) -> Option<[f64; 4]> {
    match &layer.source {
        LayerSource::Text => {
            let glyphs = text::glyph_paths(ctx, layer);
            let paths: Vec<_> = glyphs.into_iter().map(|g| g.0).collect();
            effectcraft_path::bounds(&paths).map(|r| [r.x0, r.y0, r.x1, r.y1])
        }
        LayerSource::Shape => layer.props.sub("contents").and_then(|c| shapes::content_bounds(ctx, layer, c)).map(|r| [r.x0, r.y0, r.x1, r.y1]),
        LayerSource::Camera | LayerSource::Light { .. } => None,
        _ => {
            let (w, h) = source_size(ctx.project, layer);
            (w > 0 && h > 0).then_some([0.0, 0.0, w as f64, h as f64])
        }
    }
}

/// A mask/shape path as a kurbo path (for drawing overlays).
pub fn kurbo_path(sp: &effectcraft_keyframe::ShapePath) -> kurbo::BezPath {
    effectcraft_path::to_kurbo(sp)
}
#[cfg(test)]
mod tests_audio_fx;
#[cfg(test)]
mod tests_collapse;
#[cfg(test)]
mod tests_color;
#[cfg(test)]
mod tests_frame_blend;
#[cfg(test)]
mod tests_paint;
#[cfg(test)]
mod tests_time;
