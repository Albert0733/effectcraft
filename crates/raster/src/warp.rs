//! Projective warps composited straight into a destination (the layer transform step).

use effectcraft_color::{BlendMode, blend_pixel};
use effectcraft_geom::{Mat3, Rect, vec2};
use rayon::prelude::*;

use crate::{Image, hash_noise};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sampling {
    Nearest,
    #[default]
    Bilinear,
    Bicubic,
}

#[derive(Clone, Copy, Debug)]
pub struct WarpOpts {
    pub sampling: Sampling,
    pub opacity: f32,
    pub mode: BlendMode,
    pub seed: u32,
    /// Only touch destination pixels inside this rect (dst pixel coords).
    pub clip: Option<Rect>,
}

impl Default for WarpOpts {
    fn default() -> Self {
        WarpOpts { sampling: Sampling::Bilinear, opacity: 1.0, mode: BlendMode::Normal, seed: 0, clip: None }
    }
}

/// Box-filter halving (for minification).
fn half(src: &Image) -> Image {
    let w = src.width.div_ceil(2).max(1);
    let h = src.height.div_ceil(2).max(1);
    let mut out = Image::new(w, h);
    out.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (sx, sy) = (2 * x as i64, 2 * y as i64);
            let mut acc = [0.0f32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = src.get_clamped(sx + dx, sy + dy);
                for c in 0..4 {
                    acc[c] += p[c];
                }
            }
            *px = acc.map(|v| v * 0.25);
        }
    });
    out
}

/// Draw `src` transformed by `m` (src pixel space → dst pixel space) onto `dst` with blending.
pub fn composite_warp(dst: &mut Image, src: &Image, m: &Mat3, opts: &WarpOpts) {
    if src.is_empty() || dst.is_empty() || opts.opacity <= 0.0 {
        return;
    }
    let Some(inv) = m.inverse() else { return };
    // Minification: pick a pre-filtered level so sampling doesn't alias.
    let scale = m.mean_scale();
    let mut level_img;
    let mut img = src;
    let mut inv = inv;
    if scale < 0.5 && m.is_affine() {
        let mut s = scale;
        let mut f = 1.0;
        level_img = half(src);
        f *= 0.5;
        s *= 2.0;
        while s < 0.5 && level_img.width > 1 && level_img.height > 1 {
            level_img = half(&level_img);
            f *= 0.5;
            s *= 2.0;
        }
        img = &level_img;
        inv = Mat3::scale(vec2(f, f)) * inv;
    }
    let bounds = m.map_rect(&Rect::from_size(src.width as f64, src.height as f64)).inflate(1.0).round_out();
    let full = Rect::from_size(dst.width as f64, dst.height as f64);
    let mut area = bounds.intersect(&full);
    if let Some(c) = opts.clip {
        area = area.intersect(&c);
    }
    if area.is_empty() {
        return;
    }
    let (x0, x1) = (area.x0.max(0.0) as usize, area.x1.min(dst.width as f64) as usize);
    let (y0, y1) = (area.y0.max(0.0) as usize, area.y1.min(dst.height as f64) as usize);
    let w = dst.width as usize;
    let affine = inv.is_affine();
    let mode = opts.mode;
    let op = opts.opacity;
    dst.data.par_chunks_mut(w).enumerate().skip(y0).take(y1 - y0).for_each(|(y, row)| {
        let fy = y as f64 + 0.5;
        let mut sp = inv.apply(vec2(x0 as f64 + 0.5, fy));
        let step = inv.apply_vec(vec2(1.0, 0.0));
        for x in x0..x1 {
            let p = if affine {
                let p = sp;
                sp += step;
                p
            } else {
                inv.apply(vec2(x as f64 + 0.5, fy))
            };
            if p.x < -1.0 || p.y < -1.0 || p.x > img.width as f64 + 1.0 || p.y > img.height as f64 + 1.0 {
                if mode.is_stencil() {
                    row[x] = blend_pixel(mode, row[x], [0.0; 4], 0.5);
                }
                continue;
            }
            let mut s = match opts.sampling {
                Sampling::Nearest => img.get(p.x.floor() as i64, p.y.floor() as i64),
                Sampling::Bilinear => img.sample_bilinear(p.x, p.y),
                Sampling::Bicubic => img.sample_bicubic(p.x, p.y),
            };
            if s[3] <= 0.0 && !mode.is_stencil() {
                continue;
            }
            if op < 1.0 {
                for c in s.iter_mut() {
                    *c *= op;
                }
            }
            let n = if matches!(mode, BlendMode::Dissolve | BlendMode::DancingDissolve) { hash_noise(x as u32, y as u32, opts.seed) } else { 0.5 };
            row[x] = blend_pixel(mode, row[x], s, n);
        }
    });
    // Stencil modes also clear everything outside the layer bounds.
    if mode.is_stencil() && matches!(mode, BlendMode::StencilAlpha | BlendMode::StencilLuma) {
        for (y, row) in dst.data.chunks_mut(w).enumerate() {
            for (x, px) in row.iter_mut().enumerate() {
                if x < x0 || x >= x1 || y < y0 || y >= y1 {
                    *px = [0.0; 4];
                }
            }
        }
    }
}

/// Resize with bilinear filtering (box pre-filter when shrinking).
pub fn resample(src: &Image, w: u32, h: u32) -> Image {
    let mut out = Image::new(w, h);
    if src.is_empty() || w == 0 || h == 0 {
        return out;
    }
    let m = Mat3::scale(vec2(w as f64 / src.width as f64, h as f64 / src.height as f64));
    composite_warp(&mut out, src, &m, &WarpOpts::default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_warp_copies() {
        let mut src = Image::new(4, 4);
        src.set(1, 2, [0.5, 0.5, 0.5, 1.0]);
        let mut dst = Image::new(4, 4);
        composite_warp(&mut dst, &src, &Mat3::IDENTITY, &WarpOpts::default());
        assert_eq!(dst.get(1, 2), [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(dst.get(0, 0), [0.0; 4]);
    }

    #[test]
    fn translate_by_integer() {
        let mut src = Image::new(4, 4);
        src.set(0, 0, [1.0; 4]);
        let mut dst = Image::new(8, 8);
        composite_warp(&mut dst, &src, &Mat3::translate(vec2(3.0, 2.0)), &WarpOpts::default());
        assert_eq!(dst.get(3, 2), [1.0; 4]);
    }

    #[test]
    fn downscale_preserves_average() {
        let src = Image::filled(64, 64, [0.25, 0.5, 0.75, 1.0]);
        let out = resample(&src, 8, 8);
        let p = out.get(4, 4);
        assert!((p[1] - 0.5).abs() < 1e-4 && (p[3] - 1.0).abs() < 1e-4, "{p:?}");
    }
}
