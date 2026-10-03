//! Device, pipelines, textures, uploads and readback.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use effectcraft_effects::Buf;
use effectcraft_raster::Image;
use wgpu::util::DeviceExt;

/// Compute entry points in `kernels.wgsl` (one pipeline each).
const ENTRIES: &[&str] = &[
    "warp_blend",
    "blend_full",
    "matte",
    "preserve",
    "knockout",
    "channel_mix",
    "quantize",
    "convert",
    "half",
    "box_h",
    "box_v",
    "directional",
    "glow_bright",
    "glow_combine",
    "shadow_make",
    "shadow_combine",
    "pointwise",
    "adjust_mix",
];

/// Entry points that also bind group 1 (four read-only storage buffers; see
/// [`Enc::dispatch_ext`]).
const EXT_ENTRIES: &[&str] = &["classic3d"];

/// Pixel format of every working texture: premultiplied RGBA, 32-bit float (32 bpc headroom;
/// 8/16 bpc are emulated by clamping and quantising, as on the CPU).
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// Uploaded layer buffers kept on the GPU (bytes).
const UPLOAD_BUDGET: usize = 1536 << 20;

/// A GPU image: premultiplied RGBA f32 in a texture (cheap to clone; immutable once written).
#[derive(Clone, Debug)]
pub struct GpuImage {
    pub texture: wgpu::Texture,
    pub width: u32,
    pub height: u32,
}

/// Uniform parameter block shared by every kernel (see `shaders/common.wgsl`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Params {
    pub u: [[u32; 4]; 4],
    pub f: [[f32; 4]; 12],
}

impl Params {
    fn bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(256);
        for r in &self.u {
            for x in r {
                v.extend_from_slice(&x.to_le_bytes());
            }
        }
        for r in &self.f {
            for x in r {
                v.extend_from_slice(&x.to_le_bytes());
            }
        }
        v
    }
}

struct Upload {
    buf: Weak<Buf>,
    img: GpuImage,
    bytes: usize,
    last_use: u64,
}

#[derive(Default)]
struct Uploads {
    map: HashMap<usize, Upload>,
    bytes: usize,
    clock: u64,
}

/// The device and everything compiled for it.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub(crate) name: String,
    pub(crate) max_dim: u32,
    bgl: wgpu::BindGroupLayout,
    bgl_ext: wgpu::BindGroupLayout,
    pipelines: HashMap<&'static str, wgpu::ComputePipeline>,
    display_bgl: wgpu::BindGroupLayout,
    display: wgpu::ComputePipeline,
    dummy_tex: wgpu::TextureView,
    dummy_buf: wgpu::Buffer,
    uploads: Mutex<Uploads>,
    /// Advanced 3D render pipelines (built on first use).
    pub(crate) adv3d: std::sync::OnceLock<crate::adv3d::Pipes>,
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_tex_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 },
        count: None,
    }
}

impl GpuContext {
    /// Build on an existing device (the desktop app shares egui-wgpu's). `Err` when the
    /// adapter cannot run the compositor (no compute shaders, e.g. WebGL2; no float storage
    /// textures).
    pub fn new(adapter: &wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue) -> Result<GpuContext, String> {
        let info = adapter.get_info();
        if !adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS) {
            return Err(format!("{} ({:?}): no compute shaders", info.name, info.backend));
        }
        for f in [FORMAT, wgpu::TextureFormat::Rgba8Unorm] {
            if !adapter.get_texture_format_features(f).allowed_usages.contains(wgpu::TextureUsages::STORAGE_BINDING) {
                return Err(format!("{} ({:?}): {f:?} storage textures unsupported", info.name, info.backend));
            }
        }
        let name = format!("{} ({:?})", info.name, info.backend);
        let src = [include_str!("shaders/common.wgsl"), include_str!("shaders/kernels.wgsl"), include_str!("shaders/classic3d.wgsl")].concat();
        let module =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("effectcraft kernels"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effectcraft kernels"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                storage_tex_entry(3, FORMAT),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effectcraft kernels"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let bgl_ext = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effectcraft kernels (ext)"),
            entries: &[storage(0), storage(1), storage(2), storage(3)],
        });
        let layout_ext = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effectcraft kernels (ext)"),
            bind_group_layouts: &[Some(&bgl), Some(&bgl_ext)],
            immediate_size: 0,
        });
        let pipelines = ENTRIES
            .iter()
            .map(|e| (e, &layout))
            .chain(EXT_ENTRIES.iter().map(|e| (e, &layout_ext)))
            .map(|(e, layout)| {
                let p = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(e),
                    layout: Some(layout),
                    module: &module,
                    entry_point: Some(e),
                    compilation_options: Default::default(),
                    cache: None,
                });
                (*e, p)
            })
            .collect();
        let dmodule = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("effectcraft display"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/display.wgsl").into()),
        });
        let display_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effectcraft display"),
            entries: &[tex_entry(1), storage_tex_entry(3, wgpu::TextureFormat::Rgba8Unorm)],
        });
        let dlayout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effectcraft display"),
            bind_group_layouts: &[Some(&display_bgl)],
            immediate_size: 0,
        });
        let display = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("display"),
            layout: Some(&dlayout),
            module: &dmodule,
            entry_point: Some("display"),
            compilation_options: Default::default(),
            cache: None,
        });
        let dummy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy_buf =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("dummy"), contents: &[0u8; 16], usage: wgpu::BufferUsages::STORAGE });
        let max_dim = device.limits().max_texture_dimension_2d;
        Ok(GpuContext {
            device,
            queue,
            name,
            max_dim,
            bgl,
            bgl_ext,
            pipelines,
            display_bgl,
            display,
            dummy_tex: dummy.create_view(&Default::default()),
            dummy_buf,
            uploads: Mutex::new(Uploads::default()),
            adv3d: std::sync::OnceLock::new(),
        })
    }

    /// A device of its own on the best adapter (CLI, tests, benchmarks). `None` when no
    /// adapter qualifies.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn headless() -> Option<GpuContext> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
        )
        .ok()?;
        let limits = adapter.limits();
        let required_limits = wgpu::Limits {
            max_texture_dimension_2d: limits.max_texture_dimension_2d.min(16384),
            max_buffer_size: limits.max_buffer_size,
            ..wgpu::Limits::default()
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("effectcraft gpu"), required_limits, ..Default::default() }))
                .ok()?;
        device.on_uncaptured_error(Arc::new(|e| eprintln!("wgpu: {e}")));
        GpuContext::new(&adapter, device, queue).map_err(|e| log::info!("gpu: {e}")).ok()
    }

    /// Readback (GPU → CPU) waits for the device; impossible on the browser's main thread.
    pub fn can_readback(&self) -> bool {
        cfg!(not(target_arch = "wasm32"))
    }

    pub(crate) fn fits(&self, w: u32, h: u32) -> bool {
        w > 0 && h > 0 && w <= self.max_dim && h <= self.max_dim
    }

    /// A new (zeroed) working texture.
    pub(crate) fn image(&self, w: u32, h: u32) -> GpuImage {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        GpuImage { texture, width: w, height: h }
    }

    /// Upload a CPU image (`None` when it does not fit the device).
    pub fn upload_image(&self, img: &Image) -> Option<GpuImage> {
        if !self.fits(img.width, img.height) {
            return None;
        }
        let g = self.image(img.width, img.height);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &g.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&img.data),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(img.width * 16), rows_per_image: Some(img.height) },
            wgpu::Extent3d { width: img.width, height: img.height, depth_or_array_layers: 1 },
        );
        Some(g)
    }

    /// Upload a (layer cache) buffer, reusing the texture while the same `Arc` is alive: static
    /// layers upload once, not every frame.
    pub(crate) fn upload_buf(&self, buf: &Arc<Buf>) -> Option<GpuImage> {
        let key = Arc::as_ptr(buf) as usize;
        if let Ok(mut u) = self.uploads.lock() {
            u.clock += 1;
            let now = u.clock;
            if let Some(e) = u.map.get_mut(&key)
                && e.buf.upgrade().is_some_and(|b| Arc::ptr_eq(&b, buf))
            {
                e.last_use = now;
                return Some(e.img.clone());
            }
        }
        let img = self.upload_image(&buf.img)?;
        if let Ok(mut u) = self.uploads.lock() {
            let bytes = buf.img.data.len() * 16;
            let now = u.clock;
            if let Some(old) = u.map.insert(key, Upload { buf: Arc::downgrade(buf), img: img.clone(), bytes, last_use: now }) {
                u.bytes -= old.bytes;
            }
            u.bytes += bytes;
            // Drop textures of freed buffers, then the least recently used over budget.
            if u.bytes > UPLOAD_BUDGET {
                let dead: Vec<usize> = u.map.iter().filter(|(_, e)| e.buf.strong_count() == 0).map(|(k, _)| *k).collect();
                for k in dead {
                    if let Some(e) = u.map.remove(&k) {
                        u.bytes -= e.bytes;
                    }
                }
            }
            while u.bytes > UPLOAD_BUDGET {
                let Some(k) = u.map.iter().min_by_key(|(_, e)| e.last_use).map(|(k, _)| *k) else { break };
                if let Some(e) = u.map.remove(&k) {
                    u.bytes -= e.bytes;
                }
            }
        }
        Some(img)
    }

    /// Forget uploaded layer textures.
    pub fn clear_uploads(&self) {
        if let Ok(mut u) = self.uploads.lock() {
            u.map.clear();
            u.bytes = 0;
        }
    }
}

/// Records GPU work for one render (a command encoder submitted at readback or the end).
pub(crate) struct Enc<'g> {
    pub g: &'g GpuContext,
    enc: Option<wgpu::CommandEncoder>,
    pending: usize,
}

impl<'g> Enc<'g> {
    pub fn new(g: &'g GpuContext) -> Enc<'g> {
        Enc { g, enc: None, pending: 0 }
    }

    fn encoder(&mut self) -> &mut wgpu::CommandEncoder {
        let g = self.g;
        self.enc.get_or_insert_with(|| g.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("effectcraft") }))
    }

    /// A new zeroed image.
    pub fn image(&self, w: u32, h: u32) -> GpuImage {
        self.g.image(w, h)
    }

    /// Run kernel `entry` writing `out` (dispatched over `groups` workgroups).
    pub fn dispatch(
        &mut self,
        entry: &str,
        p: &Params,
        src: &GpuImage,
        aux: Option<&GpuImage>,
        out: &GpuImage,
        data: Option<&wgpu::Buffer>,
        groups: (u32, u32),
    ) {
        self.dispatch_ext(entry, p, src, aux, out, data, groups, None);
    }

    /// [`Enc::dispatch`] for the kernels in `EXT_ENTRIES`, with their group 1 storage buffers.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_ext(
        &mut self,
        entry: &str,
        p: &Params,
        src: &GpuImage,
        aux: Option<&GpuImage>,
        out: &GpuImage,
        data: Option<&wgpu::Buffer>,
        groups: (u32, u32),
        ext: Option<[&wgpu::Buffer; 4]>,
    ) {
        let g = self.g;
        let Some(pipe) = g.pipelines.get(entry) else {
            log::error!("gpu: no kernel {entry}");
            return;
        };
        let ub = g.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &p.bytes(), usage: wgpu::BufferUsages::UNIFORM });
        let sv = src.texture.create_view(&Default::default());
        let av = aux.map(|a| a.texture.create_view(&Default::default()));
        let ov = out.texture.create_view(&Default::default());
        let bg = g.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &g.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&sv) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(av.as_ref().unwrap_or(&g.dummy_tex)) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&ov) },
                wgpu::BindGroupEntry { binding: 4, resource: data.unwrap_or(&g.dummy_buf).as_entire_binding() },
            ],
        });
        let bg_ext = ext.map(|b| {
            g.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &g.bgl_ext,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: b[0].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: b[1].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: b[2].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: b[3].as_entire_binding() },
                ],
            })
        });
        let enc = self.encoder();
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(entry), timestamp_writes: None });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &bg, &[]);
            if let Some(b) = &bg_ext {
                pass.set_bind_group(1, b, &[]);
            }
            pass.dispatch_workgroups(groups.0.max(1), groups.1.max(1), 1);
        }
        self.pending += 1;
        // Keep command buffers (and the transient textures they hold) bounded.
        if self.pending >= 256 {
            self.submit();
        }
    }

    /// Per-pixel kernel over `out`'s size (16×16 workgroups).
    pub fn pixels(&mut self, entry: &str, p: &Params, src: &GpuImage, aux: Option<&GpuImage>, out: &GpuImage, data: Option<&wgpu::Buffer>) {
        let groups = (out.width.div_ceil(16), out.height.div_ceil(16));
        self.dispatch(entry, p, src, aux, out, data, groups);
    }

    /// Copy `src` into a larger zeroed image at (`x`, `y`) (Buf::pad).
    pub fn copy_into(&mut self, src: &GpuImage, dst: &GpuImage, x: u32, y: u32) {
        let enc = self.encoder();
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &src.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &dst.texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: src.width, height: src.height, depth_or_array_layers: 1 },
        );
    }

    /// Submit recorded work.
    pub fn submit(&mut self) {
        if let Some(enc) = self.enc.take() {
            self.g.queue.submit([enc.finish()]);
        }
        self.pending = 0;
    }

    /// Read an image back to the CPU (submits and waits). `None` where readback is impossible.
    pub fn download(&mut self, img: &GpuImage) -> Option<Image> {
        let bytes = self.read_texture(&img.texture, img.width, img.height, 16)?;
        let mut out = Image::new(img.width, img.height);
        for (p, c) in out.data.iter_mut().zip(bytes.chunks_exact(16)) {
            for (k, v) in p.iter_mut().enumerate() {
                *v = f32::from_le_bytes([c[4 * k], c[4 * k + 1], c[4 * k + 2], c[4 * k + 3]]);
            }
        }
        Some(out)
    }

    /// Tightly packed texel bytes of a texture (`bpp` bytes per pixel).
    pub fn read_texture(&mut self, texture: &wgpu::Texture, w: u32, h: u32, bpp: u32) -> Option<Vec<u8>> {
        if !self.g.can_readback() {
            return None;
        }
        let row = w * bpp;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let size = padded as u64 * h as u64;
        let buf = self.g.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.encoder().copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.submit();
        let (tx, rx) = std::sync::mpsc::channel();
        buf.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        if let Err(e) = self.g.device.poll(wgpu::PollType::wait_indefinitely()) {
            log::error!("gpu: poll: {e}");
            return None;
        }
        rx.recv().ok()?.ok()?;
        let view = buf.get_mapped_range(..).ok()?;
        let mut out = Vec::with_capacity((row * h) as usize);
        for y in 0..h as usize {
            out.extend_from_slice(&view[y * padded as usize..y * padded as usize + row as usize]);
        }
        drop(view);
        buf.unmap();
        Some(out)
    }

    /// [`Enc::read_texture`] without waiting for the GPU (the browser's main thread can't): `done`
    /// gets the bytes once the copy is mapped (from the browser's event loop on the web; from
    /// a later device poll natively).
    pub fn read_texture_async(&mut self, texture: &wgpu::Texture, w: u32, h: u32, bpp: u32, done: impl FnOnce(Option<Vec<u8>>) + wgpu::WasmNotSend + 'static) {
        let row = w * bpp;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buf = self.g.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback-async"),
            size: padded as u64 * h as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.encoder().copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.submit();
        let b = buf.clone();
        buf.map_async(wgpu::MapMode::Read, .., move |r| {
            let out = r.ok().and_then(|_| {
                let view = b.get_mapped_range(..).ok()?;
                let mut out = Vec::with_capacity((row * h) as usize);
                for y in 0..h as usize {
                    out.extend_from_slice(&view[y * padded as usize..y * padded as usize + row as usize]);
                }
                Some(out)
            });
            b.unmap();
            done(out);
        });
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.g.device.poll(wgpu::PollType::Poll);
    }

    /// A data buffer for kernels that read tables (curves).
    pub fn data(&self, v: &[f32]) -> wgpu::Buffer {
        let mut bytes = Vec::with_capacity(v.len().max(4) * 4);
        for x in v {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        self.bytes(bytes)
    }

    /// Copy an image into a storage buffer (RGBA f32 rows) for kernels that read a third image
    /// through `data`: (buffer, row length in pixels).
    pub fn to_buffer(&mut self, img: &GpuImage) -> (wgpu::Buffer, u32) {
        let row = (img.width * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buf = self.g.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("image rows"),
            size: row as u64 * img.height as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.encoder().copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &img.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(img.height) },
            },
            wgpu::Extent3d { width: img.width, height: img.height, depth_or_array_layers: 1 },
        );
        (buf, row / 16)
    }

    /// A read-only storage buffer holding `bytes` (padded to 16 bytes).
    pub fn bytes(&self, mut bytes: Vec<u8>) -> wgpu::Buffer {
        while bytes.len() < 16 || bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        self.g.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("data"), contents: &bytes, usage: wgpu::BufferUsages::STORAGE })
    }

    /// Convert to the viewer's premultiplied RGBA8 texture (egui-wgpu's native texture format).
    pub fn display(&mut self, img: &GpuImage) -> wgpu::Texture {
        let g = self.g;
        let tex = g.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("viewer frame"),
            size: wgpu::Extent3d { width: img.width, height: img.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let sv = img.texture.create_view(&Default::default());
        let ov = tex.create_view(&Default::default());
        let bg = g.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &g.display_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&sv) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&ov) },
            ],
        });
        let enc = self.encoder();
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("display"), timestamp_writes: None });
            pass.set_pipeline(&g.display);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(img.width.div_ceil(16), img.height.div_ceil(16), 1);
        }
        tex
    }
}
