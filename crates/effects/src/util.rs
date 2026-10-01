//! Shared helpers for effect implementations: single-channel float planes (separable box /
//! Gaussian blur, O(1)-per-pixel min/max filters, box means for guided filtering), channel
//! pickers, a small deterministic RNG and per-pixel helpers.

use effectcraft_color::{luminance, rgb_to_hsl};
use effectcraft_raster::{Image, Px};
use rayon::prelude::*;

/// A single-channel `f32` image (row-major).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl Plane {
    pub fn new(w: usize, h: usize) -> Plane {
        Plane { w, h, data: vec![0.0; w * h] }
    }
    /// Build from an image by mapping every (premultiplied) pixel.
    pub fn from_image(img: &Image, f: impl Fn(Px) -> f32 + Sync) -> Plane {
        Plane { w: img.width as usize, h: img.height as usize, data: img.data.par_iter().map(|&p| f(p)).collect() }
    }
    pub fn alpha(img: &Image) -> Plane {
        Plane::from_image(img, |p| p[3])
    }
    /// Straight-colour Rec. 709 luminance (0 where transparent).
    pub fn luma(img: &Image) -> Plane {
        Plane::from_image(img, |p| {
            let (c, _) = unpremul(p);
            luminance(c[0], c[1], c[2])
        })
    }
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.w + x]
    }
    #[inline]
    pub fn get_clamped(&self, x: i64, y: i64) -> f32 {
        let x = x.clamp(0, self.w as i64 - 1) as usize;
        let y = y.clamp(0, self.h as i64 - 1) as usize;
        self.data[y * self.w + x]
    }
    /// Bilinear sample at continuous pixel coordinates (centres at +0.5), edges clamped.
    pub fn sample(&self, x: f64, y: f64) -> f32 {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = (fx - x0) as f32;
        let ty = (fy - y0) as f32;
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = self.get_clamped(x0, y0);
        let b = self.get_clamped(x0 + 1, y0);
        let c = self.get_clamped(x0, y0 + 1);
        let d = self.get_clamped(x0 + 1, y0 + 1);
        let top = a + (b - a) * tx;
        let bot = c + (d - c) * tx;
        top + (bot - top) * ty
    }
    pub fn transpose(&self) -> Plane {
        let (w, h) = (self.w, self.h);
        let mut out = Plane::new(h, w);
        out.data.par_chunks_mut(h.max(1)).enumerate().for_each(|(x, col)| {
            for y in 0..h {
                col[y] = self.data[y * w + x];
            }
        });
        out
    }
    /// Apply `f(src_row, dst_row)` to every row in parallel.
    pub fn map_rows(&self, f: impl Fn(&[f32], &mut [f32]) + Sync) -> Plane {
        let mut out = Plane::new(self.w, self.h);
        if self.w == 0 {
            return out;
        }
        out.data.par_chunks_mut(self.w).zip(self.data.par_chunks(self.w)).for_each(|(o, s)| f(s, o));
        out
    }
    /// Element-wise combination with another plane of the same size.
    pub fn zip_map(&self, other: &Plane, f: impl Fn(f32, f32) -> f32 + Sync) -> Plane {
        Plane { w: self.w, h: self.h, data: self.data.par_iter().zip(other.data.par_iter()).map(|(&a, &b)| f(a, b)).collect() }
    }
    pub fn map(&self, f: impl Fn(f32) -> f32 + Sync) -> Plane {
        Plane { w: self.w, h: self.h, data: self.data.par_iter().map(|&a| f(a)).collect() }
    }
}

/// Box mean of radius `r` along each row (window 2r+1, edge pixels repeated).
fn box_row(src: &[f32], dst: &mut [f32], r: usize) {
    let n = src.len();
    if n == 0 {
        return;
    }
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let get = |i: isize| src[i.clamp(0, n as isize - 1) as usize];
    let norm = 1.0 / (2 * r + 1) as f32;
    let mut acc = 0.0f64;
    for i in -(r as isize)..=(r as isize) {
        acc += get(i) as f64;
    }
    for x in 0..n {
        dst[x] = acc as f32 * norm;
        acc += get(x as isize + r as isize + 1) as f64 - get(x as isize - r as isize) as f64;
    }
}

/// Separable box mean with radii `rx`, `ry` (edges repeated).
pub fn box_plane(p: &Plane, rx: usize, ry: usize) -> Plane {
    let mut cur = if rx > 0 { p.map_rows(|s, d| box_row(s, d, rx)) } else { p.clone() };
    if ry > 0 {
        cur = cur.transpose().map_rows(|s, d| box_row(s, d, ry)).transpose();
    }
    cur
}

/// Box radii for `n` passes approximating a Gaussian of `sigma`.
pub fn box_radii(sigma: f64, n: usize) -> Vec<usize> {
    if sigma <= 0.0 {
        return vec![0; n];
    }
    let w_ideal = (12.0 * sigma * sigma / n as f64 + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - (n as i64 * wl * wl) as f64 - 4.0 * n as f64 * wl as f64 - 3.0 * n as f64) / (-4.0 * wl as f64 - 4.0);
    let m = m_ideal.round() as i64;
    (0..n as i64).map(|i| (((if i < m { wl } else { wu }) - 1) / 2).max(0) as usize).collect()
}

/// Approximate Gaussian blur of a plane (3 box passes per axis, edges repeated).
pub fn gauss_plane(p: &Plane, sigma_x: f64, sigma_y: f64) -> Plane {
    let mut cur = p.clone();
    if sigma_x > 0.05 {
        for r in box_radii(sigma_x, 3) {
            cur = cur.map_rows(|s, d| box_row(s, d, r));
        }
    }
    if sigma_y > 0.05 {
        let mut t = cur.transpose();
        for r in box_radii(sigma_y, 3) {
            t = t.map_rows(|s, d| box_row(s, d, r));
        }
        cur = t.transpose();
    }
    cur
}

/// Running min/max over a window of radius `r` (van Herk / Gil-Werman, O(1) per pixel; edges
/// repeated).
pub fn minmax_row(src: &[f32], dst: &mut [f32], r: usize, max: bool) {
    let n = src.len();
    if n == 0 {
        return;
    }
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let k = 2 * r + 1;
    let len = n + 2 * r;
    let get = |i: usize| src[(i as isize - r as isize).clamp(0, n as isize - 1) as usize];
    let op = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let mut g = vec![0.0f32; len];
    let mut hh = vec![0.0f32; len];
    for i in 0..len {
        let v = get(i);
        g[i] = if i % k == 0 { v } else { op(g[i - 1], v) };
    }
    for i in (0..len).rev() {
        let v = get(i);
        hh[i] = if i == len - 1 || (i + 1) % k == 0 { v } else { op(hh[i + 1], v) };
    }
    for x in 0..n {
        dst[x] = op(hh[x], g[x + k - 1]);
    }
}

/// Grey-scale erosion (`max = false`) or dilation (`max = true`) with a (2rx+1)×(2ry+1) box.
pub fn morph_plane(p: &Plane, rx: usize, ry: usize, max: bool) -> Plane {
    let mut cur = if rx > 0 { p.map_rows(|s, d| minmax_row(s, d, rx, max)) } else { p.clone() };
    if ry > 0 {
        cur = cur.transpose().map_rows(|s, d| minmax_row(s, d, ry, max)).transpose();
    }
    cur
}

/// Morphology with a fractional radius (linear blend between the two nearest integer radii).
pub fn morph_frac(p: &Plane, radius: f64, max: bool) -> Plane {
    if radius <= 0.0 {
        return p.clone();
    }
    let r0 = radius.floor() as usize;
    let t = (radius - r0 as f64) as f32;
    let a = morph_plane(p, r0, r0, max);
    if t < 1e-4 {
        return a;
    }
    let b = morph_plane(p, r0 + 1, r0 + 1, max);
    a.zip_map(&b, |x, y| x + (y - x) * t)
}

/// He et al. guided filter: smooth `p` while following edges of guide `g` (radius `r`, regulariser `eps`).
pub fn guided_filter(g: &Plane, p: &Plane, r: usize, eps: f32) -> Plane {
    let mean_g = box_plane(g, r, r);
    let mean_p = box_plane(p, r, r);
    let gp = g.zip_map(p, |a, b| a * b);
    let gg = g.map(|a| a * a);
    let corr_gp = box_plane(&gp, r, r);
    let corr_gg = box_plane(&gg, r, r);
    let n = g.data.len();
    let mut a = Plane::new(g.w, g.h);
    let mut b = Plane::new(g.w, g.h);
    a.data.par_iter_mut().zip(b.data.par_iter_mut()).enumerate().for_each(|(i, (av, bv))| {
        let var = corr_gg.data[i] - mean_g.data[i] * mean_g.data[i];
        let cov = corr_gp.data[i] - mean_g.data[i] * mean_p.data[i];
        *av = cov / (var + eps);
        *bv = mean_p.data[i] - *av * mean_g.data[i];
    });
    debug_assert_eq!(n, a.data.len());
    let ma = box_plane(&a, r, r);
    let mb = box_plane(&b, r, r);
    Plane { w: g.w, h: g.h, data: (0..n).into_par_iter().map(|i| ma.data[i] * g.data[i] + mb.data[i]).collect() }
}

/// Split an image into its four premultiplied channel planes.
pub fn split(img: &Image) -> [Plane; 4] {
    [0, 1, 2, 3].map(|c| Plane::from_image(img, |p| p[c]))
}

/// Join four planes into an image.
pub fn join(ch: &[Plane; 4]) -> Image {
    let (w, h) = (ch[0].w, ch[0].h);
    let mut img = Image::new(w as u32, h as u32);
    img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        *px = [ch[0].data[i], ch[1].data[i], ch[2].data[i], ch[3].data[i]];
    });
    img
}

/// Premultiplied → (straight colour, alpha).
#[inline]
pub fn unpremul(p: Px) -> ([f32; 3], f32) {
    let a = p[3];
    if a > 1e-6 { ([p[0] / a, p[1] / a, p[2] / a], a) } else { ([0.0; 3], a.max(0.0)) }
}

#[inline]
pub fn premul(c: [f32; 3], a: f32) -> Px {
    [c[0] * a, c[1] * a, c[2] * a, a]
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}

#[inline]
pub fn lerp4(a: Px, b: Px, t: f32) -> Px {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t), lerp(a[3], b[3], t)]
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if (e1 - e0).abs() < 1e-9 {
        return if x < e0 { 0.0 } else { 1.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn rgb3(c: [f32; 4]) -> [f32; 3] {
    [c[0], c[1], c[2]]
}

/// Channel sources used by Set Channels, Shift Channels, Set Matte, Displacement Map, …
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
    Red,
    Green,
    Blue,
    Alpha,
    Luminance,
    Hue,
    Lightness,
    Saturation,
    Full,
    Half,
    Off,
}

/// Value of a channel source for a straight colour `c` with alpha `a`.
#[inline]
pub fn pick(src: Src, c: [f32; 3], a: f32) -> f32 {
    match src {
        Src::Red => c[0],
        Src::Green => c[1],
        Src::Blue => c[2],
        Src::Alpha => a,
        Src::Luminance => luminance(c[0], c[1], c[2]),
        Src::Hue => rgb_to_hsl(c[0], c[1], c[2]).0,
        Src::Lightness => rgb_to_hsl(c[0], c[1], c[2]).2,
        Src::Saturation => rgb_to_hsl(c[0], c[1], c[2]).1,
        Src::Full => 1.0,
        Src::Half => 0.5,
        Src::Off => 0.0,
    }
}

/// The common source list in AE order: Red, Green, Blue, Alpha, Luminance, Hue, Lightness,
/// Saturation, Full, Half, Off.
pub const SRC_ORDER: [Src; 11] =
    [Src::Red, Src::Green, Src::Blue, Src::Alpha, Src::Luminance, Src::Hue, Src::Lightness, Src::Saturation, Src::Full, Src::Half, Src::Off];
pub const SRC_NAMES: [&str; 11] = ["Red", "Green", "Blue", "Alpha", "Luminance", "Hue", "Lightness", "Saturation", "Full", "Half", "Off"];

pub fn src_at(i: u32) -> Src {
    SRC_ORDER[(i as usize).min(SRC_ORDER.len() - 1)]
}

/// Tiny deterministic RNG (xorshift*), for procedural generators.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03 | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [0, 1).
    pub fn f(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Uniform in [-1, 1).
    pub fn s(&mut self) -> f64 {
        self.f() * 2.0 - 1.0
    }
}

/// Integer hash → [0, 1).
#[inline]
pub fn hash1(a: u32, b: u32, seed: u32) -> f32 {
    effectcraft_raster::hash_noise(a, b, seed)
}

/// Map every pixel with its coordinates (row-parallel).
pub fn map_xy(img: &mut Image, f: impl Fn(usize, usize, Px) -> Px + Sync) {
    img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            *px = f(x, y, *px);
        }
    });
}

/// Build a new image of size `w`×`h` from `f(x, y)` (row-parallel).
pub fn gen_image(w: u32, h: u32, f: impl Fn(usize, usize) -> Px + Sync) -> Image {
    let mut img = Image::new(w, h);
    map_xy(&mut img, |x, y, _| f(x, y));
    img
}

/// Replace the alpha of `img` with `new_a`, keeping straight colour. Pixels that were (nearly)
/// transparent take their colour from a small blur of the surroundings.
pub fn set_alpha(img: &mut Image, new_a: &Plane) {
    let needs_fill = img.data.par_iter().zip(new_a.data.par_iter()).any(|(p, &a)| p[3] <= 1e-4 && a > 1e-4);
    let fill = if needs_fill {
        let blurred = effectcraft_raster::gaussian_blur(img, 3.0, 3.0, true);
        Some(blurred)
    } else {
        None
    };
    img.data.par_iter_mut().zip(new_a.data.par_iter()).enumerate().for_each(|(i, (p, &na))| {
        let na = na.clamp(0.0, 1.0);
        let (c, a) = unpremul(*p);
        let c = if a > 1e-4 {
            c
        } else if let Some(f) = &fill {
            unpremul(f.data[i]).0
        } else {
            c
        };
        *p = premul(c, na);
    });
}

/// Normalised kernel-free resampling helper: sample an image with bilinear filtering, either
/// transparent or clamped outside.
#[inline]
pub fn sample(img: &Image, x: f64, y: f64, clamp: bool) -> Px {
    if clamp { img.sample_bilinear_clamped(x, y) } else { img.sample_bilinear(x, y) }
}

/// Inverse-mapped warp: each output pixel samples the source at `f(x, y)` (pixel centres +0.5).
pub fn remap(src: &Image, clamp: bool, f: impl Fn(f64, f64) -> Option<(f64, f64)> + Sync) -> Image {
    let mut out = Image::new(src.width, src.height);
    out.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            *px = match f(x as f64 + 0.5, y as f64 + 0.5) {
                Some((sx, sy)) => sample(src, sx, sy, clamp),
                None => [0.0; 4],
            };
        }
    });
    out
}

/// Layer rectangle in buffer pixels: (x0, y0, width, height).
pub fn layer_rect(ctx: &crate::EffectCtx, b: &crate::Buf) -> (f64, f64, f64, f64) {
    (b.offset[0], b.offset[1], ctx.layer_size[0] * b.scale, ctx.layer_size[1] * b.scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minmax_matches_brute_force() {
        let src: Vec<f32> = (0..37).map(|i| ((i * 7919) % 31) as f32).collect();
        for r in 0..5 {
            let mut d = vec![0.0; src.len()];
            minmax_row(&src, &mut d, r, false);
            for x in 0..src.len() {
                let lo = x.saturating_sub(r);
                let hi = (x + r).min(src.len() - 1);
                let m = src[lo..=hi].iter().cloned().fold(f32::INFINITY, f32::min);
                assert_eq!(d[x], m, "r={r} x={x}");
            }
        }
    }

    #[test]
    fn gauss_plane_preserves_constant() {
        let p = Plane { w: 20, h: 10, data: vec![0.25; 200] };
        let g = gauss_plane(&p, 3.0, 2.0);
        assert!(g.data.iter().all(|v| (v - 0.25).abs() < 1e-5));
    }

    #[test]
    fn guided_filter_keeps_constant() {
        let g = Plane { w: 16, h: 16, data: (0..256).map(|i| (i % 16) as f32 / 16.0).collect() };
        let p = Plane { w: 16, h: 16, data: vec![0.7; 256] };
        let o = guided_filter(&g, &p, 2, 1e-3);
        assert!(o.data.iter().all(|v| (v - 0.7).abs() < 1e-3));
    }
}
