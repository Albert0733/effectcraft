//! The EffectCraft compositor (CPU reference path).
//!
//! `Renderer::comp_frame` renders a composition at a time:
//! bottom-to-top over visible layers → **source** (solid, footage, text, shapes, precomp) →
//! **masks** → **effects** → **transform** (2D affine or 3D projective through the active camera,
//! with motion blur sub-samples) → **track matte** → **blend** with the layer's mode and opacity.
//! Adjustment layers run their effects on everything below, limited by their own bounds/masks.

pub mod cache;
pub mod eval;
pub mod masks;
pub mod shapes;
pub mod text;

use std::sync::Arc;

pub use cache::{CacheStats, LayerCache};
use effectcraft_color::BlendMode;
use effectcraft_effects::{Buf, EffectCtx, Params};
use effectcraft_geom::{Mat3, vec2};
use effectcraft_project::{Comp, Footage, GroupKind, ItemId, ItemKind, Layer, LayerSource, MatteKind, Node, Project, Quality, Sampling};
pub use effectcraft_raster::Image;
use effectcraft_raster::{WarpOpts, composite_warp};
use effectcraft_time::Tick;
pub use eval::{EvalCtx, ExprHost, source_size};
use rayon::prelude::*;

/// Supplies decoded footage frames (implemented by the media layer).
pub trait FootageSource: Send + Sync {
    /// The frame of `item` at source time `t`, straight from the file (any size).
    fn frame(&self, item: ItemId, footage: &Footage, t: Tick) -> Option<Arc<Image>>;
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
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts { scale: 1.0, motion_blur: true, guides: false, draft: false }
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
        Renderer { project, footage, expr: None, opts, cache: None, profile: None, depth: 0 }
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
        let mut canvas = Image::new(w, h);
        if self.depth > 16 {
            return canvas;
        }
        let ctx = self.ctx(comp_id, comp, t);
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
                let mut run: Vec<(&Layer, f64)> = visible[i..j].iter().map(|l| (*l, ctx.layer_to_comp(l).1)).collect();
                run.sort_by(|a, b| b.1.total_cmp(&a.1));
                for (l, _) in run {
                    self.draw_layer(&ctx, l, &mut canvas);
                }
                i = j;
            } else {
                self.draw_layer(&ctx, visible[i], &mut canvas);
                i += 1;
            }
        }
        if !self.project.settings.bit_depth.is_float() {
            canvas.clamp01();
        }
        canvas
    }

    /// Evaluate an effect instance's parameters.
    fn effect_params(&self, ctx: &EvalCtx, layer: &Layer, g: &effectcraft_project::PropGroup) -> Params {
        let mut p = Params::default();
        for c in &g.children {
            if let Node::Prop(pr) = c {
                p.values.insert(pr.match_id.clone(), ctx.value(layer, pr));
            }
        }
        p
    }

    fn apply_effects(&self, ctx: &EvalCtx, layer: &Layer, buf: Buf, adjustment: bool) -> Buf {
        self.apply_effects_timed(ctx, layer, buf, adjustment, None)
    }

    fn apply_effects_timed(&self, ctx: &EvalCtx, layer: &Layer, mut buf: Buf, adjustment: bool, mut timing: Option<&mut Vec<(String, f64)>>) -> Buf {
        if !layer.switches.effects {
            return buf;
        }
        let Some(fx) = layer.effects() else { return buf };
        let lt = layer.layer_time(ctx.time);
        let size = source_size(self.project, layer);
        let layer_size = if size.0 == 0 { [ctx.comp.width as f64, ctx.comp.height as f64] } else { [size.0 as f64, size.1 as f64] };
        for g in fx.groups() {
            if !g.enabled {
                continue;
            }
            let GroupKind::Effect { effect } = &g.kind else { continue };
            let Some(spec) = effectcraft_effects::find(effect) else { continue };
            let params = self.effect_params(ctx, layer, g);
            let ectx = EffectCtx { params: &params, time: lt.seconds(), layer_size, seed: g.uid as u32, adjustment };
            let t0 = std::time::Instant::now();
            buf = effectcraft_effects::apply(spec, &ectx, buf);
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
                let c = sol.color;
                Some(Buf { img: Image::filled(w, h, [c[0], c[1], c[2], 1.0]), offset: [0.0; 2], scale: s })
            }
            LayerSource::Comp { item } => {
                if self.project.comp_contains(*item, ctx.comp_id) {
                    return None;
                }
                let sub = Renderer { depth: self.depth + 1, ..*self };
                let lt = layer.layer_time(ctx.time);
                Some(Buf { img: sub.comp_frame(*item, lt), offset: [0.0; 2], scale: s })
            }
            LayerSource::Footage { item } => {
                let it = self.project.item(*item)?;
                let ItemKind::Footage(f) = &it.kind else { return None };
                if !f.has_video {
                    return None;
                }
                let lt = layer.layer_time(ctx.time);
                let img = self.footage.frame(*item, f, lt)?;
                let k = if img.width > 0 { f.width.max(1) as f64 / img.width as f64 } else { 1.0 };
                // Keep native pixels when downsampling is small; resample otherwise.
                if s < 0.75 {
                    let w = ((f.width as f64 * s).round() as u32).max(1);
                    let h = ((f.height as f64 * s).round() as u32).max(1);
                    return Some(Buf { img: effectcraft_raster::resample(&img, w, h), offset: [0.0; 2], scale: s });
                }
                Some(Buf { img: (*img).clone(), offset: [0.0; 2], scale: 1.0 / k })
            }
            LayerSource::Text => Some(text::render(ctx, layer, s)),
            LayerSource::Shape => layer.props.sub("contents").map(|c| shapes::render(ctx, layer, c, s)),
            _ => None,
        }
    }

    /// Fully processed layer buffer (source → masks → effects).
    /// Served from the layer cache when the layer's content key matches.
    pub fn layer_buf(&self, ctx: &EvalCtx, layer: &Layer) -> Option<Arc<Buf>> {
        self.layer_buf_timed(ctx, layer, None)
    }

    fn layer_buf_timed(&self, ctx: &EvalCtx, layer: &Layer, mut timing: Option<&mut LayerTiming>) -> Option<Arc<Buf>> {
        let key = self.cache.and_then(|_| cache::layer_key(ctx, layer, self.opts.scale, self.opts.draft));
        if let (Some(c), Some(k)) = (self.cache, key)
            && let Some(b) = c.get(k)
        {
            if let Some(t) = timing.as_deref_mut() {
                t.cached = true;
            }
            return Some(b);
        }
        let mut buf = self.source(ctx, layer)?;
        masks::apply(ctx, layer, &mut buf);
        let buf = Arc::new(self.apply_effects_timed(ctx, layer, buf, false, timing.map(|t| &mut t.effects)));
        if let (Some(c), Some(k)) = (self.cache, key) {
            c.insert(k, buf.clone());
        }
        Some(buf)
    }

    fn sampling(&self, layer: &Layer) -> effectcraft_raster::Sampling {
        if self.opts.draft || layer.switches.quality == Quality::Draft {
            effectcraft_raster::Sampling::Bilinear
        } else if layer.switches.sampling == Sampling::Bicubic {
            effectcraft_raster::Sampling::Bicubic
        } else {
            effectcraft_raster::Sampling::Bilinear
        }
    }

    /// Buffer pixel → output pixel matrix for the layer at the context time.
    fn buf_matrix(&self, ctx: &EvalCtx, layer: &Layer, buf: &Buf) -> Mat3 {
        let s = self.opts.scale;
        let (l2c, _) = ctx.layer_to_comp(layer);
        Mat3::scale(vec2(s, s)) * l2c * Mat3::scale(vec2(1.0 / buf.scale, 1.0 / buf.scale)) * Mat3::translate(vec2(-buf.offset[0], -buf.offset[1]))
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
        let opacity = ctx.opacity(layer) as f32;
        if opacity <= 0.0 && !layer.switches.adjustment {
            return;
        }
        if layer.switches.adjustment {
            self.draw_adjustment(ctx, layer, canvas, opacity);
            return;
        }
        let Some(prof) = self.profile else {
            if let Some(buf) = self.layer_buf(ctx, layer) {
                self.composite_layer(ctx, layer, &buf, canvas, opacity);
            }
            return;
        };
        let mut timing = LayerTiming { depth: self.depth, layer: layer.name.clone(), ..Default::default() };
        let t0 = std::time::Instant::now();
        let Some(buf) = self.layer_buf_timed(ctx, layer, Some(&mut timing)) else { return };
        let t1 = std::time::Instant::now();
        self.composite_layer(ctx, layer, &buf, canvas, opacity);
        timing.process_ms = (t1 - t0).as_secs_f64() * 1e3;
        timing.composite_ms = t1.elapsed().as_secs_f64() * 1e3;
        if let Ok(mut v) = prof.lock() {
            v.push(timing);
        }
    }

    fn composite_layer(&self, ctx: &EvalCtx, layer: &Layer, buf: &Buf, canvas: &mut Image, opacity: f32) {
        let matte = layer.track_matte.and_then(|tm| ctx.comp.layer(tm.layer).filter(|m| m.id != layer.id).map(|m| (m, tm.kind)));
        let preserve = layer.preserve_transparency;
        if matte.is_none() && !preserve {
            self.place(ctx, layer, buf, canvas, layer.blend_mode, opacity);
            return;
        }
        // Render in isolation, then matte / preserve transparency, then blend.
        let mut iso = Image::new(canvas.width, canvas.height);
        self.place(ctx, layer, buf, &mut iso, BlendMode::Normal, 1.0);
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
        let below = Buf { img: canvas.clone(), offset: [0.0; 2], scale: self.opts.scale };
        let adjusted = self.apply_effects(ctx, layer, below, true);
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

/// Convenience: render one frame of a comp with no footage source.
pub fn render_frame(project: &Project, comp: ItemId, t: Tick, scale: f64) -> Image {
    Renderer::new(project, &NoFootage, RenderOpts { scale, ..Default::default() }).comp_frame(comp, t)
}

#[cfg(test)]
mod tests;

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
