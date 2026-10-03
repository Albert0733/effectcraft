//! Advanced 3D: the GPU rasteriser against the CPU reference on the same scenes.
//!
//! Tolerances: the GPU's colour target is half float (relative error ≤ 1e-3) and its
//! rasteriser and interpolators differ from the CPU's in the last bits, so pixels on triangle
//! edges may be covered by the other neighbour. Interior pixels agree within 2/255; at most
//! 1.5 % of pixels (silhouettes) may differ more. Skipped without an adapter.

use effectcraft_keyframe::Value;
use effectcraft_project::{Comp, ItemId, ItemKind, LayerSource, LightKind, PrimitiveKind, Project, Renderer as R3, Solid, build};
use effectcraft_render::three_d::adv::{self, Scene};
use effectcraft_render::{Accelerator, EvalCtx, NoFootage, RenderOpts, Renderer};
use effectcraft_time::{FrameRate, Tick};

use crate::Gpu;

fn gpu() -> Option<Gpu> {
    let g = Gpu::headless();
    if g.is_none() {
        eprintln!("effectcraft-gpu adv3d tests: no GPU adapter, skipping");
    }
    g
}

fn add(p: &mut Project, cid: ItemId, src: LayerSource, edit: impl FnOnce(&mut effectcraft_project::Layer)) {
    let comp = p.comp(cid).unwrap().clone();
    let mut l = build::layer(p, &comp, "L", src, (comp.width, comp.height), None);
    edit(&mut l);
    p.comp_mut(cid).unwrap().layers.insert(0, l);
}

fn set(l: &mut effectcraft_project::Layer, path: &str, v: Value) {
    l.props.prop_mut(path).unwrap_or_else(|| panic!("{path}")).value = v;
}

/// Primitives with metal/dielectric materials, a shadow-casting spot light and a point light,
/// an environment light, a semi-transparent textured card.
fn scene_project() -> (Project, ItemId) {
    let mut p = Project::default();
    let mut c = Comp::new(240, 160, FrameRate::FPS_30, Tick::from_seconds_f64(1.0));
    c.renderer = R3::Advanced3D;
    let cid = p.add_item("C", Default::default(), None, ItemKind::Comp(c.into()));
    add(&mut p, cid, LayerSource::Primitive { kind: PrimitiveKind::Plane }, |l| {
        set(l, "geometryOptions/width", Value::Scalar(900.0));
        set(l, "geometryOptions/height", Value::Scalar(900.0));
        set(l, "transform/position", Value::Vec3([120.0, 130.0, 0.0]));
        set(l, "transform/rotationX", Value::Scalar(90.0));
    });
    add(&mut p, cid, LayerSource::Primitive { kind: PrimitiveKind::Sphere }, |l| {
        set(l, "geometryOptions/radius", Value::Scalar(35.0));
        set(l, "transform/position", Value::Vec3([80.0, 95.0, 0.0]));
        set(l, "materialOptions/baseColor", Value::Color([0.9, 0.2, 0.2, 1.0]));
        set(l, "materialOptions/roughness", Value::Scalar(35.0));
        set(l, "materialOptions/castsShadows", Value::Enum(1));
    });
    add(&mut p, cid, LayerSource::Primitive { kind: PrimitiveKind::Torus }, |l| {
        set(l, "geometryOptions/radius", Value::Scalar(35.0));
        set(l, "geometryOptions/tubeRadius", Value::Scalar(12.0));
        set(l, "transform/position", Value::Vec3([165.0, 90.0, 0.0]));
        set(l, "transform/rotationX", Value::Scalar(60.0));
        set(l, "materialOptions/metallic", Value::Scalar(100.0));
        set(l, "materialOptions/roughness", Value::Scalar(20.0));
        set(l, "materialOptions/castsShadows", Value::Enum(1));
    });
    let sid = p.add_item("S", Default::default(), None, ItemKind::Solid(Solid { color: [0.2, 0.6, 0.9], width: 60, height: 40, pixel_aspect: 1.0 }));
    add(&mut p, cid, LayerSource::Solid { item: sid }, |l| {
        l.switches.three_d = true;
        set(l, "transform/opacity", Value::Scalar(60.0));
        set(l, "transform/position", Value::Vec3([120.0, 80.0, -60.0]));
        set(l, "transform/rotationY", Value::Scalar(30.0));
    });
    let env = p.add_item("E", Default::default(), None, ItemKind::Solid(Solid { color: [0.6, 0.7, 1.0], width: 32, height: 16, pixel_aspect: 1.0 }));
    add(&mut p, cid, LayerSource::Solid { item: env }, |l| {
        l.environment = true;
        l.switches.three_d = true;
    });
    add(&mut p, cid, LayerSource::Light { kind: LightKind::Environment }, |l| set(l, "lightOptions/intensity", Value::Scalar(50.0)));
    add(&mut p, cid, LayerSource::Light { kind: LightKind::Spot }, |l| {
        set(l, "transform/position", Value::Vec3([40.0, -120.0, -200.0]));
        set(l, "transform/poi", Value::Vec3([120.0, 100.0, 0.0]));
        set(l, "lightOptions/castsShadows", Value::Bool(true));
        set(l, "lightOptions/coneAngle", Value::Scalar(110.0));
        set(l, "lightOptions/shadowDiffusion", Value::Scalar(4.0));
    });
    add(&mut p, cid, LayerSource::Light { kind: LightKind::Point }, |l| {
        set(l, "transform/position", Value::Vec3([200.0, 20.0, -100.0]));
        set(l, "lightOptions/intensity", Value::Scalar(40.0));
        set(l, "lightOptions/falloff", Value::Enum(1));
    });
    (p, cid)
}

fn scene(p: &Project, cid: ItemId) -> Scene {
    let r = Renderer::new(p, &NoFootage, RenderOpts::default());
    let ctx = EvalCtx { project: p, comp_id: cid, comp: p.comp(cid).unwrap(), time: Tick::ZERO, expr: None };
    let run: Vec<&effectcraft_project::Layer> = ctx.comp.layers.iter().rev().filter(|l| l.is_3d() && l.has_video()).collect();
    adv::scene_of(&r, &ctx, &run, (ctx.comp.width, ctx.comp.height))
}

fn compare(s: &Scene, g: &Gpu) {
    let cpu = adv::raster::render(s);
    let gt = g.raster_3d(s).expect("gpu raster");
    assert_eq!((gt.width, gt.height), (cpu.width, cpu.height));
    let n = cpu.color.len();
    let mut bad = 0;
    let mut sum = 0.0f64;
    for (a, b) in cpu.color.iter().zip(&gt.color) {
        let d = (0..4).map(|k| (a[k] - b[k]).abs() / a[k].abs().max(1.0)).fold(0.0f32, f32::max);
        sum += d as f64;
        if d > 2.0 / 255.0 {
            bad += 1;
        }
    }
    let frac = bad as f64 / n as f64;
    eprintln!("adv3d gpu vs cpu: {bad}/{n} pixels beyond 2/255 ({:.3} %), mean {:.5}", frac * 100.0, sum / n as f64);
    assert!(frac < 0.015, "{:.3} % of pixels differ", frac * 100.0);
    assert!(sum / (n as f64) < 0.002);
    // Depth agrees where both see a surface.
    let mut dz = 0;
    for (a, b) in cpu.depth.iter().zip(&gt.depth) {
        if a.is_finite() != b.is_finite() || (a.is_finite() && (a - b).abs() > 0.01 * a.abs().max(1.0)) {
            dz += 1;
        }
    }
    assert!((dz as f64) < 0.015 * n as f64, "{dz} depth mismatches");
}

#[test]
fn gpu_matches_cpu_lit_scene() {
    let Some(g) = gpu() else { return };
    let (p, cid) = scene_project();
    let s = scene(&p, cid);
    assert!(!s.shadows.is_empty() && s.env.is_some() && s.opaque_count < s.indices.len() as u32);
    compare(&s, &g);
}

#[test]
fn gpu_matches_cpu_unlit_and_full_frames() {
    let Some(g) = gpu() else { return };
    let (mut p, cid) = scene_project();
    // Unlit: remove the lights.
    p.comp_mut(cid).unwrap().layers.retain(|l| !l.is_light());
    compare(&scene(&p, cid), &g);
    // Whole frames through the renderer with the GPU attached.
    let (p, cid) = scene_project();
    let cpu = Renderer::new(&p, &NoFootage, RenderOpts::default()).comp_frame(cid, Tick::ZERO);
    let mut r = Renderer::new(&p, &NoFootage, RenderOpts { backend: effectcraft_render::Backend::Gpu, ..Default::default() });
    r.accel = Some(&g);
    let gimg = r.comp_frame(cid, Tick::ZERO);
    let diff = cpu.data.iter().zip(&gimg.data).filter(|(a, b)| (0..4).any(|k| (a[k] - b[k]).abs() > 3.0 / 255.0)).count();
    assert!((diff as f64) < 0.02 * cpu.data.len() as f64, "{diff} pixels differ");
}

#[test]
fn gpu_raster_perf_smoke() {
    let Some(g) = gpu() else { return };
    let (p, cid) = scene_project();
    let s = scene(&p, cid);
    let t0 = std::time::Instant::now();
    let _ = adv::raster::render(&s);
    let cpu = t0.elapsed();
    // First use compiles the pipelines.
    let _ = g.raster_3d(&s);
    let t0 = std::time::Instant::now();
    let _ = g.raster_3d(&s);
    let gpu = t0.elapsed();
    eprintln!("adv3d 480x320 (2x SSAA of 240x160), {} triangles: cpu {:?}, gpu {:?}", s.indices.len() / 3, cpu, gpu);
}
