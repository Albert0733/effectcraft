//! The EffectCraft GPU compositor (Mercury GPU Acceleration's counterpart), on wgpu compute
//! shaders: Metal, Vulkan, Direct3D 12 and WebGPU.
//!
//! The CPU [`Renderer`] stays the reference and keeps rendering layer *content* (sources, masks,
//! CPU effects, layer styles) into its layer cache. [`Gpu`] composites: it uploads the cached
//! layer buffers (once per buffer), applies the layer transforms with the CPU's sampling
//! (nearest / bilinear / bicubic, minification pre-filter), motion-blur sub-samples, track
//! mattes, Preserve Transparency, layer styles' passes, all 38 blend modes and the 8/16 bpc
//! clamping and quantisation, and converts colour spaces. 3D runs, adjustment layers and
//! wireframes run on the CPU between GPU steps (read back, draw, upload).
//!
//! Advanced 3D comps rasterise on a render pipeline (`advanced3d.wgsl`: depth buffer, PBR,
//! image-based light, shadow maps), see [`Accelerator::raster_3d`].
//!
//! GPU effects ([`effectcraft_effects::GPU_EFFECTS`]) run as compute kernels with the CPU
//! effect's exact steps (padding, box-blur radii, parameter conversions); chains of them are
//! uploaded and read back once.
//!
//! Plug a [`Gpu`] into [`Renderer::accel`] (it implements [`Accelerator`]); renders then use it
//! when [`RenderOpts::backend`](effectcraft_render::RenderOpts) asks for it. The viewer can
//! skip readback entirely with [`Gpu::render_display`], which leaves an RGBA8 texture for
//! egui-wgpu to draw.

mod adv3d;
mod classic3d;
mod context;
mod effects;
mod fx_color;
mod fx_distort;
mod fx_generate;
mod ops;
mod walk;

use std::sync::Arc;

pub use context::{GpuContext, GpuImage};
use effectcraft_effects::Buf;
use effectcraft_project::ItemId;
use effectcraft_raster::Image;
use effectcraft_render::{Accelerator, FxStep, Renderer};
use effectcraft_time::Tick;
pub use wgpu;

use crate::context::Enc;

/// The GPU compositor (cheap to clone; one device shared by all clones).
#[derive(Clone)]
pub struct Gpu {
    ctx: Arc<GpuContext>,
}

/// A viewer frame left on the GPU: premultiplied RGBA8 (`wgpu::TextureFormat::Rgba8Unorm`),
/// ready for `egui_wgpu::Renderer::register_native_texture`.
#[derive(Clone, Debug)]
pub struct DisplayFrame {
    pub texture: wgpu::Texture,
    pub width: u32,
    pub height: u32,
}

impl Gpu {
    /// On a device of its own (CLI, tests, benchmarks); `None` without a usable adapter.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn headless() -> Option<Gpu> {
        GpuContext::headless().map(Gpu::from_context)
    }

    /// On an existing device (the desktop app shares egui-wgpu's).
    pub fn new(adapter: &wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue) -> Result<Gpu, String> {
        GpuContext::new(adapter, device, queue).map(Gpu::from_context)
    }

    pub fn from_context(ctx: GpuContext) -> Gpu {
        Gpu { ctx: Arc::new(ctx) }
    }

    pub fn context(&self) -> &GpuContext {
        &self.ctx
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.ctx.device
    }

    /// Render a top-level comp frame and leave it on the GPU as a display texture (no
    /// readback unless a CPU fallback step needs one). `None` = render on the CPU.
    pub fn render_display(&self, r: &Renderer, comp: ItemId, t: Tick) -> Option<DisplayFrame> {
        let mut e = Enc::new(&self.ctx);
        let img = walk::render(&mut e, r, comp, t)?;
        let texture = e.display(&img);
        e.submit();
        Some(DisplayFrame { texture, width: img.width, height: img.height })
    }

    /// Read a display frame back as premultiplied RGBA8 (Info panel sampling, eyedroppers).
    pub fn read_display(&self, f: &DisplayFrame) -> Option<Vec<u8>> {
        Enc::new(&self.ctx).read_texture(&f.texture, f.width, f.height, 4)
    }

    /// [`Gpu::read_display`] without blocking (the browser's main thread): `done` gets the bytes
    /// once the GPU has them.
    pub fn read_display_async(&self, f: &DisplayFrame, done: impl FnOnce(Option<Vec<u8>>) + wgpu::WasmNotSend + 'static) {
        Enc::new(&self.ctx).read_texture_async(&f.texture, f.width, f.height, 4, done);
    }

    /// Render a frame on the GPU and read it back (`None` = not handled).
    pub fn render(&self, r: &Renderer, comp: ItemId, t: Tick) -> Option<Image> {
        if !self.ctx.can_readback() {
            return None;
        }
        let mut e = Enc::new(&self.ctx);
        let img = walk::render(&mut e, r, comp, t)?;
        e.download(&img)
    }

    /// Wait for all submitted GPU work (benchmarks).
    pub fn wait(&self) {
        let _ = self.ctx.device.poll(wgpu::PollType::wait_indefinitely());
    }
}

impl Accelerator for Gpu {
    fn name(&self) -> String {
        self.ctx.name.clone()
    }

    fn comp_frame(&self, r: &Renderer, comp: ItemId, t: Tick) -> Option<Image> {
        self.render(r, comp, t)
    }

    fn supports_effect(&self, id: &str) -> bool {
        self.ctx.can_readback() && effects::supports(id)
    }

    fn effects(&self, chain: &[FxStep], buf: &Buf, levels: Option<f32>) -> Option<Buf> {
        effects::run_chain(&mut Enc::new(&self.ctx), chain, buf, levels)
    }

    fn raster_3d(&self, scene: &effectcraft_render::three_d::adv::Scene) -> Option<effectcraft_render::three_d::adv::Target> {
        adv3d::render(&self.ctx, scene)
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_3d;
#[cfg(test)]
mod tests_adjust;
#[cfg(test)]
mod tests_adv3d;
#[cfg(test)]
mod tests_fx_color;
