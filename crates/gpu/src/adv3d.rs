//! The Advanced 3D rasteriser on wgpu: a render pipeline with a reversed-Z depth buffer
//! (`Depth32Float`, Greater), the scene's materials, textures, lights and shadow maps in
//! storage buffers, and `advanced3d.wgsl`'s physically based fragment shader. Opaque triangles
//! are drawn with depth writes, then the sorted transparent ones blended over (premultiplied).
//! Colour (`Rgba16Float`) and camera depth (`R32Float`) are read back for the CPU's resolve,
//! depth of field and encoding.

use effectcraft_render::three_d::adv::{Scene, Target};
use wgpu::util::DeviceExt;

use crate::context::{Enc, GpuContext};

/// Pipelines (built on first use).
pub(crate) struct Pipes {
    bgl: wgpu::BindGroupLayout,
    opaque: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
}

const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_OUT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const VERTEX_SIZE: u64 = 52;

fn storage(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

impl Pipes {
    fn new(device: &wgpu::Device) -> Pipes {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("effectcraft advanced 3d"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/advanced3d.wgsl").into()),
        });
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }];
        entries.extend((1..=6).map(storage));
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("advanced 3d"), entries: &entries });
        let layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("advanced 3d"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 48, shader_location: 4 },
        ];
        let make = |transparent: bool| {
            let blend = transparent.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if transparent { "advanced 3d transparent" } else { "advanced 3d opaque" }),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: VERTEX_SIZE, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(!transparent),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::ColorTargetState { format: COLOR, blend, write_mask: wgpu::ColorWrites::ALL }),
                        Some(wgpu::ColorTargetState {
                            format: DEPTH_OUT,
                            blend: None,
                            write_mask: if transparent { wgpu::ColorWrites::empty() } else { wgpu::ColorWrites::ALL },
                        }),
                    ],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Pipes { opaque: make(false), transparent: make(true), bgl }
    }
}

fn f32s(v: impl IntoIterator<Item = f32>) -> Vec<u8> {
    let mut b: Vec<u8> = v.into_iter().flat_map(f32::to_le_bytes).collect();
    while b.len() < 16 {
        b.push(0);
    }
    b
}

/// Pack the scene's arrays as the shader's storage buffers.
fn pack(s: &Scene) -> [Vec<u8>; 7] {
    let mut globals: Vec<f32> = vec![];
    // Column-major clip matrix.
    for c in 0..4 {
        for r in 0..4 {
            globals.push(s.clip[r][c]);
        }
    }
    globals.extend(s.view[2]);
    globals.extend([s.eye[0], s.eye[1], s.eye[2], s.ortho as u32 as f32]);
    globals.extend([s.cam_fwd[0], s.cam_fwd[1], s.cam_fwd[2], s.lights.len() as f32]);
    globals.extend([s.ambient[0], s.ambient[1], s.ambient[2], s.env.is_some() as u32 as f32]);
    let e = s.env.unwrap_or(effectcraft_render::three_d::adv::EnvInfo { radiance: 0, mips: 1, irradiance: 0, intensity: 0.0, rotation: 0.0 });
    globals.extend([e.radiance as f32, e.mips as f32, e.irradiance as f32, e.intensity]);
    let lit = !(s.lights.is_empty() && s.env.is_none() && s.ambient == [0.0; 3]);
    globals.extend([e.rotation, lit as u32 as f32, 0.0, 0.0]);
    let mats = s.materials.iter().flat_map(|m| {
        let flags = (m.double_sided as u32) | (m.unlit as u32) << 1 | (m.accepts_lights as u32) << 2 | (m.receives_shadows as u32) << 3 | (m.alpha_mode << 8);
        [
            m.base[0],
            m.base[1],
            m.base[2],
            m.base[3],
            m.emissive[0],
            m.emissive[1],
            m.emissive[2],
            m.metallic,
            m.roughness,
            m.normal_scale,
            m.occlusion_strength,
            m.alpha_cutoff,
            m.tex_base as f32,
            m.tex_mr as f32,
            m.tex_normal as f32,
            m.tex_occlusion as f32,
            m.tex_emissive as f32,
            flags as f32,
            m.diffuse_k,
            m.specular_k,
            m.ambient_k,
            m.opacity,
            0.0,
            0.0,
        ]
    });
    let mut tex: Vec<u8> = s.textures.iter().flat_map(|t| [t.offset, t.width, t.height, t.wrap_u | t.wrap_v << 8]).flat_map(u32::to_le_bytes).collect();
    while tex.len() < 16 {
        tex.push(0);
    }
    let texels = f32s(s.texels.iter().flatten().copied());
    let lights = s.lights.iter().flat_map(|l| {
        [
            l.pos[0],
            l.pos[1],
            l.pos[2],
            l.kind as f32,
            l.dir[0],
            l.dir[1],
            l.dir[2],
            l.cos_outer,
            l.color[0],
            l.color[1],
            l.color[2],
            l.cos_inner,
            l.falloff as f32,
            l.radius,
            l.falloff_distance,
            l.shadow as f32,
            l.shadow_darkness,
            0.0,
            0.0,
            0.0,
        ]
    });
    let shadows = s.shadows.iter().flat_map(|m| {
        let mut v: Vec<f32> = m.view.iter().flatten().copied().collect();
        v.extend([m.focal, m.ortho as u32 as f32, f32::from_bits(m.size), f32::from_bits(m.offset), m.bias, m.radius, 0.0, 0.0]);
        v
    });
    [f32s(globals), f32s(mats), tex, texels, f32s(lights), f32s(shadows), f32s(s.shadow_texels.iter().copied())]
}

fn half_to_f32(h: u16) -> f32 {
    let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as f32;
    match e {
        0 => s * m * 2f32.powi(-24),
        31 => {
            if m == 0.0 {
                s * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => s * (1.0 + m / 1024.0) * 2f32.powi(e - 15),
    }
}

/// Rasterise a scene. `None` when the device can't (no readback, size or buffer limits).
pub(crate) fn render(g: &GpuContext, s: &Scene) -> Option<Target> {
    if !g.can_readback() || !g.fits(s.width, s.height) || s.indices.is_empty() {
        return None;
    }
    let limits = g.device.limits();
    let bufs = pack(s);
    if bufs.iter().any(|b| b.len() as u64 > limits.max_storage_buffer_binding_size) {
        return None;
    }
    let p = g.adv3d.get_or_init(|| Pipes::new(&g.device));
    let dev = &g.device;
    let vb: Vec<u8> = s
        .vertices
        .iter()
        .flat_map(|v| {
            let mut b = Vec::with_capacity(VERTEX_SIZE as usize);
            for x in v.pos.iter().chain(&v.normal).chain(&v.uv).chain(&v.tangent) {
                b.extend_from_slice(&x.to_le_bytes());
            }
            b.extend_from_slice(&v.material.to_le_bytes());
            b
        })
        .collect();
    let vbuf = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("adv3d vertices"), contents: &vb, usage: wgpu::BufferUsages::VERTEX });
    let ib: Vec<u8> = s.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
    let ibuf = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("adv3d indices"), contents: &ib, usage: wgpu::BufferUsages::INDEX });
    let ub = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("adv3d globals"), contents: &bufs[0], usage: wgpu::BufferUsages::UNIFORM });
    let sbs: Vec<wgpu::Buffer> = bufs[1..]
        .iter()
        .map(|b| dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("adv3d data"), contents: b, usage: wgpu::BufferUsages::STORAGE }))
        .collect();
    let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() }];
    for (i, b) in sbs.iter().enumerate() {
        entries.push(wgpu::BindGroupEntry { binding: i as u32 + 1, resource: b.as_entire_binding() });
    }
    let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("adv3d"), layout: &p.bgl, entries: &entries });
    let tex = |format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
        dev.create_texture(&wgpu::TextureDescriptor {
            label: Some("adv3d target"),
            size: wgpu::Extent3d { width: s.width, height: s.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
    let color = tex(COLOR, rt);
    let zout = tex(DEPTH_OUT, rt);
    let depth = tex(DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let (cv, zv, dv) = (color.create_view(&Default::default()), zout.create_view(&Default::default()), depth.create_view(&Default::default()));
    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("adv3d") });
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("adv3d"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &cv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &zv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: -1.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &dv,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &bg, &[]);
        pass.set_vertex_buffer(0, vbuf.slice(..));
        pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
        let n = s.indices.len() as u32;
        if s.opaque_count > 0 {
            pass.set_pipeline(&p.opaque);
            pass.draw_indexed(0..s.opaque_count, 0, 0..1);
        }
        if n > s.opaque_count {
            pass.set_pipeline(&p.transparent);
            pass.draw_indexed(s.opaque_count..n, 0, 0..1);
        }
    }
    g.queue.submit([enc.finish()]);
    let mut e = Enc::new(g);
    let cb = e.read_texture(&color, s.width, s.height, 8)?;
    let zb = e.read_texture(&zout, s.width, s.height, 4)?;
    let color = cb.chunks_exact(8).map(|c| [0, 1, 2, 3].map(|k| half_to_f32(u16::from_le_bytes([c[2 * k], c[2 * k + 1]])))).collect();
    let depth = zb
        .chunks_exact(4)
        .map(|c| {
            let z = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            if z < 0.0 { f32::INFINITY } else { z }
        })
        .collect();
    Some(Target { width: s.width, height: s.height, color, depth })
}
