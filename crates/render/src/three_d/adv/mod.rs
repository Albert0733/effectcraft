//! The Advanced 3D renderer (Composition Settings ▸ 3D Renderer ▸ Advanced 3D).
//!
//! A run of 3D layers becomes one [`Scene`] of world-space triangles (models, primitives,
//! extruded text and shapes, and textured cards for every other layer), lit physically
//! ([`shade`]) by the comp's lights, an Environment light's image and shadow maps. The scene
//! is rasterised with a depth buffer on the GPU when an [`crate::Accelerator`] implements
//! [`crate::Accelerator::raster_3d`], otherwise by the software rasteriser ([`raster`]), at 2×2
//! supersampling. The resolve (box filter), depth of field (a gather blur by each pixel's
//! circle of confusion) and the conversion back to the working encoding run on the CPU for
//! both paths, and the result is composited over the layers below the run.

pub mod raster;
pub mod scene;
pub mod shade;

use effectcraft_project::Layer;
use rayon::prelude::*;

pub use raster::Target;
pub use scene::{EnvInfo, Light, Material, Scene, ShadowMap, TexInfo, Vertex};

use super::camera::Dof;
use super::compose::camera_for;
use crate::{EvalCtx, Image, Renderer};

/// Box-filter a supersampled target down by `k`: (premultiplied colour, mean depth of the
/// covered subsamples, ∞ where none).
pub fn resolve(t: &Target, k: u32) -> (Image, Vec<f32>) {
    let (w, h) = (t.width / k, t.height / k);
    let mut img = Image::new(w, h);
    let mut depth = vec![f32::INFINITY; (w * h) as usize];
    let n = (k * k) as f32;
    img.data.par_chunks_mut(w as usize).zip(depth.par_chunks_mut(w as usize)).enumerate().for_each(|(y, (row, drow))| {
        for x in 0..w as usize {
            let mut acc = [0.0f32; 4];
            let (mut dz, mut dn) = (0.0f32, 0.0f32);
            for j in 0..k as usize {
                for i in 0..k as usize {
                    let s = (y * k as usize + j) * t.width as usize + x * k as usize + i;
                    let c = t.color[s];
                    for q in 0..4 {
                        acc[q] += c[q];
                    }
                    if t.depth[s].is_finite() {
                        dz += t.depth[s];
                        dn += 1.0;
                    }
                }
            }
            row[x] = acc.map(|v| v / n);
            if dn > 0.0 {
                drow[x] = dz / dn;
            }
        }
    });
    (img, depth)
}

/// Depth of field: a gather blur where each pixel collects the neighbours whose circle of
/// confusion (from their depth, in output pixels) reaches it; nearer in-focus surfaces are not
/// smeared by the blur of what is behind them.
pub fn depth_of_field(img: &Image, depth: &[f32], dof: &Dof, scale: f64) -> Image {
    let (w, h) = (img.width as i64, img.height as i64);
    let max_r = 48.0f32;
    let radius: Vec<f32> = depth
        .iter()
        .map(|&z| {
            let z = if z.is_finite() { z as f64 } else { 1.0e9 };
            ((dof.coc(z) * scale * 0.5) as f32).min(max_r)
        })
        .collect();
    let rmax = radius.iter().fold(0.0f32, |a, &b| a.max(b));
    if rmax < 0.5 {
        return img.clone();
    }
    // Disk samples (golden-angle spiral) over the largest radius.
    const N: usize = 64;
    let golden = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
    let taps: Vec<(f32, f32, f32)> = (1..=N)
        .map(|i| {
            let r = (i as f32 / N as f32).sqrt() * rmax;
            let a = i as f32 * golden;
            (r * a.cos(), r * a.sin(), r)
        })
        .collect();
    let mut out = Image::new(img.width, img.height);
    out.data.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
        for x in 0..w as usize {
            let i = y * w as usize + x;
            let (rp, zp) = (radius[i], depth[i]);
            let cw = 1.0 / (std::f32::consts::PI * rp * rp).max(1.0);
            let c = img.data[i];
            let mut acc = [c[0] * cw, c[1] * cw, c[2] * cw, c[3] * cw];
            let mut wsum = cw;
            for &(dx, dy, d) in &taps {
                let (sx, sy) = (x as i64 + dx.round() as i64, y as i64 + dy.round() as i64);
                if sx < 0 || sy < 0 || sx >= w || sy >= h {
                    continue;
                }
                let j = (sy * w + sx) as usize;
                // A sample behind this pixel blurs no further than this pixel's own circle.
                let rq = if depth[j] > zp { radius[j].min(rp) } else { radius[j] };
                let cov = (rq - d + 0.5).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                // Each tap stands for an annulus of area ≈ π·rmax²/N.
                let wq = cov / (std::f32::consts::PI * rq * rq).max(1.0) * (std::f32::consts::PI * rmax * rmax / N as f32);
                let q = img.data[j];
                for k in 0..4 {
                    acc[k] += q[k] * wq;
                }
                wsum += wq;
            }
            row[x] = acc.map(|v| v / wsum);
        }
    });
    out
}

/// Linear → sRGB-encoded premultiplied pixels (the working encoding of non-linear projects).
pub fn encode(img: &mut Image) {
    img.data.par_iter_mut().for_each(|p| {
        let a = p[3];
        if a > 1e-6 {
            for c in 0..3 {
                p[c] = shade::linear_to_srgb(p[c] / a) * a;
            }
        }
    });
}

/// Render a scene on the CPU, resolved: (premultiplied image at output size, depth).
pub fn render_cpu(s: &Scene) -> (Image, Vec<f32>) {
    resolve(&raster::render(s), s.ssaa.max(1))
}

/// Build the scene of a run (for tests and tools).
pub fn scene_of(r: &Renderer, ctx: &EvalCtx, run: &[&Layer], out: (u32, u32)) -> Scene {
    scene::build(r, ctx, run, out)
}

/// Draw a run of 3D layers with the Advanced 3D renderer into `canvas`.
pub(crate) fn draw_run(r: &Renderer, ctx: &EvalCtx, run: &[&Layer], canvas: &mut Image) {
    let s = scene::build(r, ctx, run, (canvas.width, canvas.height));
    if s.indices.is_empty() {
        return;
    }
    let target = r.active_accel().and_then(|a| a.raster_3d(&s)).filter(|t| t.width == s.width && t.height == s.height).unwrap_or_else(|| raster::render(&s));
    let (mut img, depth) = resolve(&target, s.ssaa.max(1));
    let cam = camera_for(r, ctx);
    if let Some(dof) = cam.dof.filter(|_| !ctx.comp.draft_3d && !r.opts.draft) {
        img = depth_of_field(&img, &depth, &dof, r.opts.scale);
    }
    if !s.linear_io {
        encode(&mut img);
    }
    canvas.data.par_iter_mut().zip(img.data.par_iter()).for_each(|(d, s)| {
        let k = 1.0 - s[3];
        *d = [s[0] + d[0] * k, s[1] + d[1] * k, s[2] + d[2] * k, s[3] + d[3] * k];
    });
}

#[cfg(test)]
mod tests;
