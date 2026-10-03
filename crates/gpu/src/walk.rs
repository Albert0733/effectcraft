//! The per-frame compositing walk on the GPU: the same steps as `Renderer::draw_comp` (layer
//! order, 3D runs, collapsed precomps, track mattes, Preserve Transparency, layer styles, motion
//! blur, bit-depth quantisation, colour conversions), with layer pixels taken from the CPU
//! renderer and its layer cache. Steps without a GPU implementation (3D runs, adjustment and
//! wireframe layers) read the canvas back, run on the CPU and upload the result.

use std::sync::Arc;

use effectcraft_color::BlendMode;
use effectcraft_effects::Buf;
use effectcraft_project::{ItemId, Layer, Quality};
use effectcraft_raster::Image;
use effectcraft_render::{EvalCtx, Renderer, styles};
use effectcraft_time::Tick;

use crate::context::{Enc, GpuImage};
use crate::ops;

/// Render a comp frame into a GPU image (the canvas in the output space, as
/// `Renderer::comp_frame_cpu` returns it). `None` when something cannot run here (the caller
/// renders on the CPU instead).
pub(crate) fn render<'g>(e: &mut Enc<'g>, r: &Renderer, comp_id: ItemId, t: Tick) -> Option<GpuImage> {
    // Region-of-interest frames are cropped on the CPU path.
    if r.opts.roi.is_some() {
        return None;
    }
    let ctx = r.eval_ctx(comp_id, t)?;
    let s = r.opts.scale;
    let w = ((ctx.comp.width as f64 * s).round() as u32).max(1);
    let h = ((ctx.comp.height as f64 * s).round() as u32).max(1);
    if r.depth() > 16 || !e.g.fits(w, h) {
        return None;
    }
    let mut canvas = e.image(w, h);
    draw_comp(e, r, &ctx, &mut canvas)?;
    let pipe = r.pipe();
    if let Some(c) = pipe.from_blend() {
        canvas = ops::convert(e, &canvas, &c);
    }
    if let Some(l) = pipe.levels {
        canvas = ops::quantize(e, &canvas, l);
    }
    if r.depth() == 0
        && let Some(c) = pipe.output()
    {
        canvas = ops::convert(e, &canvas, &c);
        if let Some(l) = pipe.levels {
            canvas = ops::quantize(e, &canvas, l);
        }
    }
    Some(canvas)
}

/// Run a CPU drawing step on the canvas (read back, draw, upload).
fn on_cpu(e: &mut Enc, canvas: &mut GpuImage, f: impl FnOnce(&mut Image)) -> Option<()> {
    let mut img = e.download(canvas)?;
    f(&mut img);
    *canvas = e.g.upload_image(&img)?;
    Some(())
}

fn draw_comp<'a>(e: &mut Enc, r: &Renderer<'a>, ctx: &EvalCtx<'a>, canvas: &mut GpuImage) -> Option<()> {
    let visible = r.visible_layers(ctx);
    let levels = r.pipe().levels;
    let mut i = 0;
    while i < visible.len() {
        if visible[i].is_3d() {
            let mut j = i;
            while j < visible.len() && visible[j].is_3d() {
                j += 1;
            }
            draw_3d(e, r, ctx, &visible[i..j], canvas)?;
            i = j;
        } else {
            match r.collapsed(ctx, visible[i]) {
                Some(item) => draw_collapsed(e, r, ctx, visible[i], item, canvas)?,
                None => draw_layer(e, r, ctx, visible[i], canvas)?,
            }
            i += 1;
        }
        // 8/16 bpc: the comp is an integer buffer after every layer.
        if let Some(l) = levels {
            *canvas = ops::quantize(e, canvas, l);
        }
    }
    Some(())
}

/// A run of consecutive 3D layers: Classic 3D on the GPU (`classic3d`), with 3D adjustment
/// layers splitting the run as on the CPU; Advanced 3D, wireframes and runs the kernel cannot
/// hold draw on the CPU.
fn draw_3d<'a>(e: &mut Enc, r: &Renderer<'a>, ctx: &EvalCtx<'a>, run: &[&'a Layer], canvas: &mut GpuImage) -> Option<()> {
    if let Some(k) = run.iter().position(|l| l.switches.adjustment) {
        draw_3d(e, r, ctx, &run[..k], canvas)?;
        draw_layer(e, r, ctx, run[k], canvas)?;
        return draw_3d(e, r, ctx, &run[k + 1..], canvas);
    }
    if run.is_empty() {
        return Some(());
    }
    match r.prepare_3d_run(ctx, run, (canvas.width, canvas.height)).and_then(|prep| crate::classic3d::draw_run(e, &prep, canvas)) {
        Some(img) => *canvas = img,
        None => on_cpu(e, canvas, |img| r.draw_3d_run(ctx, run, img))?,
    }
    Some(())
}

fn draw_collapsed<'a>(e: &mut Enc, r: &Renderer<'a>, ctx: &EvalCtx<'a>, layer: &Layer, item: ItemId, canvas: &mut GpuImage) -> Option<()> {
    let opacity = r.layer_opacity(ctx, layer);
    if opacity <= 0.0 {
        return Some(());
    }
    let matte = r.track_matte(ctx, layer).is_some();
    if layer.blend_mode == BlendMode::Normal && !matte && !layer.preserve_transparency {
        if let Some((sub, nctx)) = r.collapse_into(ctx, layer, item, opacity) {
            draw_comp(e, &sub, &nctx, canvas)?;
        }
        return Some(());
    }
    let Some((sub, nctx)) = r.collapse_into(ctx, layer, item, 1.0) else { return Some(()) };
    let mut iso = e.image(canvas.width, canvas.height);
    draw_comp(e, &sub, &nctx, &mut iso)?;
    composite_iso(e, r, ctx, layer, iso, canvas, opacity)
}

fn draw_layer(e: &mut Enc, r: &Renderer, ctx: &EvalCtx, layer: &Layer, canvas: &mut GpuImage) -> Option<()> {
    if layer.switches.adjustment {
        if draw_adjustment(e, r, ctx, layer, canvas).is_some() {
            return Some(());
        }
        return on_cpu(e, canvas, |img| r.draw_layer_cpu(ctx, layer, img));
    }
    if layer.switches.quality == Quality::Wireframe {
        // Wireframes draw outlines on the CPU.
        return on_cpu(e, canvas, |img| r.draw_layer_cpu(ctx, layer, img));
    }
    let opacity = r.layer_opacity(ctx, layer);
    if opacity <= 0.0 {
        return Some(());
    }
    let Some(st) = r.styled_layer(ctx, layer) else { return Some(()) };
    if st.plain {
        return composite_layer(e, r, ctx, layer, &st.body, canvas, opacity);
    }
    // Layer styles: exterior passes with their own modes, then the body; the layer's opacity
    // fades the whole stack; knockout and channel switches as on the CPU.
    let bl = styles::blending(ctx, layer);
    let (w, h) = (canvas.width, canvas.height);
    if r.track_matte(ctx, layer).is_some() || layer.preserve_transparency {
        let mut iso = e.image(w, h);
        for (b, m) in &st.passes {
            iso = place(e, r, ctx, layer, b, &iso, *m, 1.0)?;
        }
        iso = place(e, r, ctx, layer, &st.body, &iso, BlendMode::Normal, 1.0)?;
        return composite_iso(e, r, ctx, layer, iso, canvas, opacity);
    }
    let mut tmp = canvas.clone();
    if bl.knockout > 0 {
        let k = place(e, r, ctx, layer, &st.content, &e.image(w, h), BlendMode::Normal, 1.0)?;
        tmp = ops::knockout(e, &tmp, &k);
    }
    for (b, m) in &st.passes {
        tmp = place(e, r, ctx, layer, b, &tmp, *m, 1.0)?;
    }
    tmp = place(e, r, ctx, layer, &st.body, &tmp, layer.blend_mode, 1.0)?;
    *canvas = ops::channel_mix(e, canvas, &tmp, bl.channels, opacity);
    Some(())
}

/// Adjustment layer (`Renderer::draw_adjustment`): the effect stack runs on the comp below,
/// resident on the GPU (non-GPU effects read back, run on the CPU and upload), and the result
/// replaces the canvas inside the layer's footprint × opacity. `None` = draw it on the CPU.
fn draw_adjustment(e: &mut Enc, r: &Renderer, ctx: &EvalCtx, layer: &Layer, canvas: &mut GpuImage) -> Option<()> {
    let opacity = r.layer_opacity(ctx, layer);
    let Some(foot) = r.adjustment_footprint(ctx, layer) else { return Some(()) };
    let (w, h) = (canvas.width, canvas.height);
    let matte = place(e, r, ctx, layer, &Arc::new(foot), &e.image(w, h), BlendMode::Normal, 1.0)?;
    let pipe = r.pipe();
    // Effects run in the working space, not the linear blending space.
    let below = match pipe.from_blend() {
        Some(c) => ops::convert(e, canvas, &c),
        None => canvas.clone(),
    };
    let mut fx = crate::effects::GpuFx::new(e, below, [0.0; 2], r.opts.scale, pipe.levels);
    r.run_effects_on(ctx, layer, true, &mut fx);
    let (mut adjusted, _, _) = fx.finish()?;
    if adjusted.width != w || adjusted.height != h {
        return Some(());
    }
    if let Some(c) = pipe.to_blend() {
        adjusted = ops::convert(e, &adjusted, &c);
    }
    *canvas = ops::adjust_mix(e, canvas, &adjusted, &matte, opacity);
    Some(())
}

fn composite_layer(e: &mut Enc, r: &Renderer, ctx: &EvalCtx, layer: &Layer, buf: &Arc<Buf>, canvas: &mut GpuImage, opacity: f32) -> Option<()> {
    let matte = r.track_matte(ctx, layer).is_some();
    if !matte && !layer.preserve_transparency {
        *canvas = place(e, r, ctx, layer, buf, canvas, layer.blend_mode, opacity)?;
        return Some(());
    }
    // Render in isolation, then matte / preserve transparency, then blend.
    let iso = place(e, r, ctx, layer, buf, &e.image(canvas.width, canvas.height), BlendMode::Normal, 1.0)?;
    composite_iso(e, r, ctx, layer, iso, canvas, opacity)
}

/// Track matte / Preserve Transparency on an isolated layer render, then blend.
fn composite_iso(e: &mut Enc, r: &Renderer, ctx: &EvalCtx, layer: &Layer, mut iso: GpuImage, canvas: &mut GpuImage, opacity: f32) -> Option<()> {
    if let Some((m, kind)) = r.track_matte(ctx, layer) {
        let mut mimg = e.image(canvas.width, canvas.height);
        if m.is_active_at(ctx.time)
            && let Some(mb) = r.layer_buf(ctx, m)
        {
            let mo = ctx.opacity(m) as f32;
            mimg = place(e, r, ctx, m, &mb, &mimg, BlendMode::Normal, mo)?;
        }
        iso = ops::matte(e, &iso, &mimg, kind);
    }
    if layer.preserve_transparency {
        iso = ops::preserve(e, &iso, canvas);
    }
    *canvas = ops::blend_full(e, canvas, &iso, layer.blend_mode, opacity, layer.id.0 as u32);
    Some(())
}

/// Draw a processed layer buffer onto `target` (transform, motion blur sub-samples).
fn place(e: &mut Enc, r: &Renderer, ctx: &EvalCtx, layer: &Layer, buf: &Arc<Buf>, target: &GpuImage, mode: BlendMode, opacity: f32) -> Option<GpuImage> {
    if buf.img.is_empty() || opacity <= 0.0 {
        return Some(target.clone());
    }
    let src = e.g.upload_buf(buf)?;
    let pl = r.placement(ctx, layer, buf);
    if let [m] = pl.matrices.as_slice() {
        return Some(ops::warp(e, target, &src, m, pl.sampling, mode, opacity, pl.seed, None));
    }
    let mut acc = e.image(target.width, target.height);
    let k = 1.0 / pl.matrices.len() as f32;
    for m in &pl.matrices {
        acc = ops::warp(e, &acc, &src, m, pl.sampling, BlendMode::Normal, 1.0, 0, Some(k));
    }
    Some(ops::blend_full(e, target, &acc, mode, opacity, pl.seed))
}
