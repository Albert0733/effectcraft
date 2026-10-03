use effectcraft_keyframe::Value;
use effectcraft_project::{Comp, ItemId, ItemKind, LayerSource, LightKind, PrimitiveKind, Project, Renderer as R3, build};
use effectcraft_time::{FrameRate, Tick};

use super::scene::{Material, Scene, TexInfo, Vertex, irradiance_map};
use super::shade::{self, PI};
use super::*;
use crate::{NoFootage, RenderOpts, Renderer};

// ---------------------------------------------------------------- shading

#[test]
fn brdf_matches_cook_torrance_formula() {
    // Reference: f = kd·c/π + D·G·F/(4·NV·NL), returned × NL × π.
    let n = [0.0, 0.0, 1.0];
    let v = shade::norm([0.3, 0.1, 1.0]);
    let l = shade::norm([-0.4, 0.2, 1.0]);
    let (base, metal, rough) = ([0.8, 0.4, 0.2], 0.3f32, 0.45f32);
    let got = shade::brdf(base, metal, rough, 1.0, 1.0, n, v, l);
    let h = shade::norm([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
    let (nv, nl, nh, vh) = (shade::dot(n, v), shade::dot(n, l), shade::dot(n, h), shade::dot(v, h));
    let a2 = (rough * rough).powi(2);
    let d = a2 / (PI * ((nh * nh) * (a2 - 1.0) + 1.0).powi(2));
    let k = (rough + 1.0).powi(2) / 8.0;
    let g = nv / (nv * (1.0 - k) + k) * nl / (nl * (1.0 - k) + k);
    for c in 0..3 {
        let f0 = 0.04 + (base[c] - 0.04) * metal;
        let f = f0 + (1.0 - f0) * (1.0 - vh).powi(5);
        let expect = ((1.0 - f) * (1.0 - metal) * base[c] / PI + d * g * f / (4.0 * nv * nl)) * nl * PI;
        assert!((got[c] - expect).abs() < 1e-5, "channel {c}: {} vs {expect}", got[c]);
    }
    // No light from behind the surface.
    assert_eq!(shade::brdf(base, metal, rough, 1.0, 1.0, n, v, [0.0, 0.0, -1.0]), [0.0; 3]);
    // A white rough dielectric lit head-on reflects about its base colour (π folded in).
    let w = shade::brdf([1.0; 3], 0.0, 1.0, 1.0, 1.0, n, n, n);
    assert!(w[0] > 0.95 && w[0] < 1.05, "{w:?}");
}

#[test]
fn ggx_distribution_is_normalised() {
    // ∫ D(h)·(n·h) dω = 1 over the hemisphere.
    for &rough in &[0.2f32, 0.5, 0.9] {
        let a = rough * rough;
        let n = 4000;
        let mut sum = 0.0f64;
        for i in 0..n {
            let th = (i as f64 + 0.5) / n as f64 * std::f64::consts::FRAC_PI_2;
            let c = th.cos() as f32;
            sum += shade::d_ggx(c, a) as f64 * c as f64 * th.sin() * std::f64::consts::FRAC_PI_2 / n as f64 * 2.0 * std::f64::consts::PI;
        }
        assert!((sum - 1.0).abs() < 0.02, "roughness {rough}: {sum}");
    }
}

#[test]
fn constant_environment_has_constant_irradiance() {
    let src = vec![[0.25, 0.5, 0.75, 1.0]; 64 * 32];
    let irr = irradiance_map(64, 32, &src, 32, 16);
    for p in &irr {
        for c in 0..3 {
            assert!((p[c] - src[0][c]).abs() < 1e-4, "{p:?}");
        }
    }
    // And a sky brighter than the ground lights upward-facing normals more.
    let sky: Vec<[f32; 4]> = (0..32).flat_map(|y| (0..64).map(move |_| if y < 16 { [1.0, 1.0, 1.0, 1.0] } else { [0.0, 0.0, 0.0, 1.0] })).collect();
    let irr = irradiance_map(64, 32, &sky, 32, 16);
    assert!(irr[16][0] > 0.9 && irr[15 * 32 + 16][0] < 0.1, "{:?} {:?}", irr[16], irr[15 * 32 + 16]);
}

#[test]
fn equirect_round_trip_and_env_brdf() {
    for (u, v) in [(0.1f32, 0.2f32), (0.5, 0.5), (0.9, 0.7)] {
        let d = shade::equirect_dir(u, v, 0.3);
        let (u2, v2) = shade::equirect_uv(d, 0.3);
        assert!((u - u2).abs() < 1e-4 && (v - v2).abs() < 1e-4);
    }
    // Up (−Y in AE space) is the top row.
    assert!(shade::equirect_uv([0.0, -1.0, 0.0], 0.0).1 < 1e-3);
    let (a, b) = shade::env_brdf(1.0, 0.0);
    assert!(a + b > 0.9 && a + b <= 1.05);
    let (a, b) = shade::env_brdf(0.1, 1.0);
    assert!(a >= 0.0 && b > -0.01 && a + b < 1.0);
}

// ---------------------------------------------------------------- rasteriser

/// A scene of axis-aligned quads (z = camera depth) seen by an orthographic identity camera
/// mapping world x/y straight to raster pixels.
fn quad_scene(w: u32, h: u32, quads: &[([f32; 4], f32, [f32; 4])]) -> Scene {
    let mut s = Scene { width: w, height: h, ssaa: 1, linear_io: true, ..Scene::default() };
    // clip: x → 2x/w − 1, y → 1 − 2y/h, z: depth 0..1000 → 1..0 (nearer = larger).
    s.clip = [[2.0 / w as f32, 0.0, 0.0, -1.0], [0.0, -2.0 / h as f32, 0.0, 1.0], [0.0, 0.0, -0.001, 1.0], [0.0, 0.0, 0.0, 1.0]];
    s.view = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    s.ortho = true;
    s.cam_fwd = [0.0, 0.0, 1.0];
    for (rect, z, color) in quads {
        let m = s.materials.len() as u32;
        s.materials.push(Material { base: *color, unlit: true, double_sided: true, alpha_mode: if color[3] < 1.0 { 2 } else { 0 }, ..Material::default() });
        let b = s.vertices.len() as u32;
        for (x, y) in [(rect[0], rect[1]), (rect[2], rect[1]), (rect[2], rect[3]), (rect[0], rect[3])] {
            s.vertices.push(Vertex { pos: [x, y, *z], normal: [0.0, 0.0, -1.0], uv: [0.0; 2], tangent: [1.0, 0.0, 0.0, 1.0], material: m });
        }
        s.indices.extend([b, b + 2, b + 1, b, b + 3, b + 2]);
    }
    // Opaque first, transparent (in given order) after.
    let tris: Vec<[u32; 3]> = s.indices.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
    let (o, t): (Vec<_>, Vec<_>) = tris.into_iter().partition(|t| !s.materials[s.vertices[t[0] as usize].material as usize].transparent());
    s.opaque_count = 3 * o.len() as u32;
    s.indices = o.into_iter().chain(t).flatten().collect();
    s
}

#[test]
fn depth_buffer_keeps_the_nearest_surface() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    // Blue is listed first but is farther away.
    let s = quad_scene(40, 30, &[([0.0, 0.0, 30.0, 20.0], 500.0, blue), ([10.0, 10.0, 40.0, 30.0], 100.0, red)]);
    let t = raster::render(&s);
    let px = |x: usize, y: usize| t.color[y * 40 + x];
    assert_eq!(px(5, 5), blue);
    assert_eq!(px(15, 15), red, "overlap: the nearer quad wins");
    assert_eq!(px(35, 25), red);
    assert_eq!(px(35, 5), [0.0; 4]);
    assert_eq!(t.depth[15 * 40 + 15], 100.0);
    assert!(t.depth[5 * 40 + 35].is_infinite());
}

#[test]
fn coverage_is_exact_for_pixel_aligned_quads() {
    // Shared diagonal edges are filled once (top-left rule): exact pixel counts.
    let s = quad_scene(32, 32, &[([4.0, 4.0, 20.0, 12.0], 10.0, [1.0, 1.0, 1.0, 0.5])]);
    let t = raster::render(&s);
    let covered = t.color.iter().filter(|c| c[3] > 0.0).count();
    assert_eq!(covered, 16 * 8);
    // Blended once (no double coverage along the diagonal).
    assert!(t.color.iter().all(|c| c[3] == 0.0 || (c[3] - 0.5).abs() < 1e-6));
    // Two transparent layers composite in order.
    let s = quad_scene(8, 8, &[([0.0, 0.0, 8.0, 8.0], 100.0, [1.0, 0.0, 0.0, 0.5]), ([0.0, 0.0, 8.0, 8.0], 50.0, [0.0, 0.0, 1.0, 0.5])]);
    let c = raster::render(&s).color[0];
    assert!((c[0] - 0.25).abs() < 1e-6 && (c[2] - 0.5).abs() < 1e-6 && (c[3] - 0.75).abs() < 1e-6, "{c:?}");
}

#[test]
fn textures_sample_bilinear_with_wrap_modes() {
    let t = TexInfo { offset: 0, width: 2, height: 1, wrap_u: 0, wrap_v: 1 };
    let texels = vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
    assert_eq!(shade::sample(&t, &texels, 0.25, 0.5)[0], 0.0);
    assert_eq!(shade::sample(&t, &texels, 0.5, 0.5)[0], 0.5);
    // Repeat wraps the right edge back to texel 0.
    assert!((shade::sample(&t, &texels, 1.0, 0.5)[0] - 0.5).abs() < 1e-6);
    let c = TexInfo { wrap_u: 1, ..t };
    assert_eq!(shade::sample(&c, &texels, 1.0, 0.5)[0], 1.0);
}

// ---------------------------------------------------------------- full renderer

fn project(w: u32, h: u32) -> (Project, ItemId) {
    let mut p = Project::default();
    let mut c = Comp::new(w, h, FrameRate::FPS_30, Tick::from_seconds_f64(1.0));
    c.renderer = R3::Advanced3D;
    let id = p.add_item("C", Default::default(), None, ItemKind::Comp(c.into()));
    (p, id)
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

fn render(p: &Project, cid: ItemId) -> crate::Image {
    Renderer::new(p, &NoFootage, RenderOpts::default()).comp_frame(cid, Tick::ZERO)
}

#[test]
fn lit_cube_renders_with_depth_and_shading() {
    let (mut p, cid) = project(160, 120);
    add(&mut p, cid, LayerSource::Primitive { kind: PrimitiveKind::Cube }, |l| {
        set(l, "geometryOptions/width", Value::Scalar(60.0));
        set(l, "geometryOptions/height", Value::Scalar(60.0));
        set(l, "geometryOptions/depth", Value::Scalar(60.0));
        set(l, "transform/rotationY", Value::Scalar(35.0));
        set(l, "transform/rotationX", Value::Scalar(-25.0));
        set(l, "materialOptions/baseColor", Value::Color([1.0, 1.0, 1.0, 1.0]));
    });
    let img = render(&p, cid);
    let centre = img.data[60 * 160 + 80];
    assert!(centre[3] > 0.99, "cube covers the centre: {centre:?}");
    assert_eq!(img.data[2 * 160 + 2][3], 0.0, "corner is empty");
    // No lights: unlit (base colour) — add a light and faces get different shades.
    add(&mut p, cid, LayerSource::Light { kind: LightKind::Parallel }, |_| {});
    let lit = render(&p, cid);
    let mut shades: Vec<i32> = lit.data.iter().filter(|q| q[3] > 0.99).map(|q| (q[0] * 20.0) as i32).collect();
    shades.sort();
    shades.dedup();
    assert!(shades.len() >= 3, "faces are shaded differently: {shades:?}");
    // Classic 3D draws no meshes.
    p.comp_mut(cid).unwrap().renderer = R3::Classic3D;
    assert!(render(&p, cid).data.iter().all(|q| q[3] == 0.0));
}

#[test]
fn shadows_darken_the_receiver() {
    let (mut p, cid) = project(200, 150);
    // A floor plane facing the camera, a sphere in front of it, a spot light behind the camera.
    add(&mut p, cid, LayerSource::Primitive { kind: PrimitiveKind::Plane }, |l| {
        set(l, "geometryOptions/width", Value::Scalar(400.0));
        set(l, "geometryOptions/height", Value::Scalar(400.0));
        set(l, "transform/position", Value::Vec3([100.0, 75.0, 200.0]));
        set(l, "materialOptions/baseColor", Value::Color([1.0, 1.0, 1.0, 1.0]));
    });
    add(&mut p, cid, LayerSource::Primitive { kind: PrimitiveKind::Sphere }, |l| {
        set(l, "geometryOptions/radius", Value::Scalar(30.0));
        set(l, "transform/position", Value::Vec3([100.0, 75.0, 50.0]));
        set(l, "materialOptions/castsShadows", Value::Enum(2));
    });
    add(&mut p, cid, LayerSource::Light { kind: LightKind::Point }, |l| {
        set(l, "transform/position", Value::Vec3([100.0, 75.0, -300.0]));
        set(l, "lightOptions/castsShadows", Value::Bool(true));
    });
    let img = render(&p, cid);
    // The sphere casts only (invisible); its shadow lands on the plane behind it.
    let mid = img.data[75 * 200 + 100];
    let side = img.data[75 * 200 + 20];
    assert!(mid[3] > 0.99 && side[3] > 0.99);
    assert!(mid[0] < side[0] * 0.5, "shadowed centre {mid:?} vs lit side {side:?}");
    // Without shadow casting the centre is lit.
    let l = &mut p.comp_mut(cid).unwrap().layers[0];
    set(l, "lightOptions/castsShadows", Value::Bool(false));
    let img = render(&p, cid);
    assert!(img.data[75 * 200 + 100][0] > side[0] * 0.8);
}

#[test]
fn cards_and_environment_light() {
    let (mut p, cid) = project(120, 90);
    let sid = p.add_item(
        "S",
        Default::default(),
        None,
        ItemKind::Solid(effectcraft_project::Solid { color: [0.5, 0.5, 0.5], width: 60, height: 40, pixel_aspect: 1.0 }),
    );
    add(&mut p, cid, LayerSource::Solid { item: sid }, |l| l.switches.three_d = true);
    let unlit = render(&p, cid);
    let c = unlit.data[20 * 120 + 30];
    assert!((c[0] - 0.5).abs() < 0.01 && c[3] > 0.99, "unlit card keeps its colour: {c:?}");
    assert_eq!(unlit.data[80 * 120 + 110][3], 0.0);
    // An Environment light sourcing a white environment layer brightens the card.
    let env = p.add_item(
        "E",
        Default::default(),
        None,
        ItemKind::Solid(effectcraft_project::Solid { color: [1.0, 1.0, 1.0], width: 64, height: 32, pixel_aspect: 1.0 }),
    );
    add(&mut p, cid, LayerSource::Solid { item: env }, |l| {
        l.environment = true;
        l.switches.three_d = true;
    });
    add(&mut p, cid, LayerSource::Light { kind: LightKind::Environment }, |_| {});
    let lit = render(&p, cid);
    let c2 = lit.data[20 * 120 + 30];
    assert!(c2[0] > 0.3, "{c2:?}");
    // The environment layer itself is not drawn.
    assert_eq!(lit.data[80 * 120 + 110][3], 0.0);
}

#[test]
fn depth_of_field_blurs_out_of_focus_surfaces() {
    let mut img = Image::new(64, 64);
    for y in 0..64 {
        for x in 0..64 {
            let v = if (x / 4 + y / 4) % 2 == 0 { 1.0 } else { 0.0 };
            img.data[y * 64 + x] = [v, v, v, 1.0];
        }
    }
    let var = |im: &Image| {
        let m: f32 = im.data.iter().map(|p| p[0]).sum::<f32>() / im.data.len() as f32;
        im.data.iter().map(|p| (p[0] - m).powi(2)).sum::<f32>() / im.data.len() as f32
    };
    let dof = crate::three_d::camera::Dof { focus: 1000.0, aperture: 40.0, blur_level: 1.0, iris: Default::default(), highlight: Default::default() };
    let sharp = depth_of_field(&img, &vec![1000.0; 64 * 64], &dof, 1.0);
    assert!((var(&sharp) - var(&img)).abs() < 1e-6, "in focus: unchanged");
    let blurred = depth_of_field(&img, &vec![3000.0; 64 * 64], &dof, 1.0);
    assert!(var(&blurred) < var(&img) * 0.3, "out of focus: blurred");
}

#[test]
fn extruded_text_renders_as_a_mesh() {
    let (mut p, cid) = project(200, 100);
    add(&mut p, cid, LayerSource::Text, |l| {
        l.switches.three_d = true;
        let doc = effectcraft_keyframe::TextDoc { text: "IO".into(), size: 60.0, fill: [1.0, 1.0, 1.0, 1.0], apply_fill: true, ..Default::default() };
        set(l, "text/sourceText", Value::Text(Box::new(doc)));
        let mut next = 10_000u64;
        l.props.children.push(build::extrusion_geometry_options(&mut build::Ids(&mut next)).into());
        set(l, "geometryOptions/extrusionDepth", Value::Scalar(30.0));
        set(l, "geometryOptions/bevelStyle", Value::Enum(1));
        set(l, "transform/rotationY", Value::Scalar(40.0));
    });
    let ctx = crate::EvalCtx { project: &p, comp_id: cid, comp: p.comp(cid).unwrap(), time: Tick::ZERO, expr: None, footage: None };
    let l = &ctx.comp.layers[0];
    let params = scene::extrusion_params(&ctx, l).unwrap();
    let meshes = scene::extruded_meshes(&ctx, l, &params);
    assert!(!meshes.is_empty());
    assert!(meshes.iter().all(|(m, _, _)| m.positions.iter().any(|q| (q[2] - 30.0).abs() < 1e-3)));
    let img = render(&p, cid);
    assert!(img.data.iter().filter(|q| q[3] > 0.5).count() > 200);
}
