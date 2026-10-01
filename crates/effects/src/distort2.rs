//! Distort effects, batch 2: Turbulent Displace, Displacement Map (self as map), Optics
//! Compensation, Magnify, Mesh Warp, Bezier Warp, CC Bend It, CC Lens, CC Page Turn, CC Tiler,
//! CC Griddler.
//!
//! Mesh Warp and Bezier Warp share a grid rasteriser: a (nx+1)×(ny+1) grid of destination points
//! and matching source points is split into triangles, bucketed per output row and filled
//! row-parallel with inverse barycentric sampling.
//!
//! Mesh Warp's distortion mesh is stored in the hidden string parameter `mesh`: whitespace (or
//! `;`) separated `dx,dy` pairs, row-major over the (rows+1)×(columns+1) vertices, giving each
//! vertex's offset in layer pixels. An empty string (or one with the wrong count) is the
//! undistorted mesh.

use std::f64::consts::{FRAC_PI_2, PI};

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::{Image, Px};
use rayon::prelude::*;

use crate::generate::value_noise;
use crate::util::{SRC_NAMES, layer_rect, lerp4, map_xy, pick, premul, remap, smoothstep, src_at, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Distort", params, render, gpu: false, float: true }
}

fn pt(x: f64, y: f64) -> Value {
    Value::Vec2([x, y])
}

fn pct(max: f64) -> ParamUi {
    slider(0.0, max, 0.0, max.min(100.0), 1)
}

// ---------------------------------------------------------------- Turbulent Displace

fn fbm(u: f64, v: f64, z: f64, seed: u32, octaves: f64, falloff: f32) -> f32 {
    let n = octaves.ceil().max(1.0) as usize;
    let frac = (octaves - octaves.floor()) as f32;
    let (mut sum, mut norm, mut amp, mut f) = (0.0f32, 0.0f32, 1.0f32, 1.0f32);
    for o in 0..n {
        let w = if o + 1 == n && frac > 0.0 { frac } else { 1.0 };
        sum += value_noise(u as f32 * f, v as f32 * f, z as f32 + o as f32 * 5.17, seed.wrapping_add(o as u32 * 101)) * amp * w;
        norm += amp * w;
        amp *= falloff;
        f *= 2.0;
    }
    sum / norm.max(1e-6)
}

fn turbulent_displace(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let amount = ctx.params.f("amount") * b.scale;
    if amount.abs() < 1e-9 {
        return b;
    }
    let kind = ctx.params.e("displacement");
    let size = (ctx.params.f("size") * b.scale).max(1.0);
    let off = b.to_px(ctx.params.v2("offset"));
    let oct = ctx.params.f("complexity").clamp(1.0, 10.0);
    let evo = ctx.params.f("evolution") / 360.0;
    let pin = ctx.params.e("pinning");
    let falloff = if kind == 3 { 0.3 } else { 0.5 };
    let (w, h) = (b.img.width as f64, b.img.height as f64);
    let edge = (w.min(h) * 0.1).max(1.0);
    let seed = ctx.seed ^ 0x7d15;
    let n = |u: f64, v: f64, s: u32| fbm(u, v, evo, seed.wrapping_add(s), oct, falloff) as f64 - 0.5;
    b.img = remap(&b.img, false, |x, y| {
        let (u, v) = ((x - off.0) / size, (y - off.1) / size);
        let (dx, dy) = match kind {
            1 | 2 => {
                let e = 0.05;
                let gx = (n(u + e, v, 0) - n(u - e, v, 0)) / (2.0 * e);
                let gy = (n(u, v + e, 0) - n(u, v - e, 0)) / (2.0 * e);
                let k = amount * 0.25;
                if kind == 1 { (gx * k, gy * k) } else { (-gy * k, gx * k) }
            }
            4 => (n(u, v, 0) * amount, 0.0),
            5 => (0.0, n(u, v, 17) * amount),
            6 => {
                let a = n(u, v, 0) * amount;
                (a, a)
            }
            _ => (n(u, v, 0) * amount, n(u, v, 17) * amount),
        };
        let ex = (x.min(w - x) / edge).clamp(0.0, 1.0);
        let ey = (y.min(h - y) / edge).clamp(0.0, 1.0);
        let k = match pin {
            1 => ex.min(ey),
            2 => ey,
            3 => ex,
            _ => 1.0,
        };
        Some((x + dx * k, y + dy * k))
    });
    b
}

// ---------------------------------------------------------------- Displacement Map

fn displacement_map(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let hs = src_at(ctx.params.e("useForHorizontal"));
    let vs = src_at(ctx.params.e("useForVertical"));
    let mh = ctx.params.f("maxHorizontal") * b.scale;
    let mv = ctx.params.f("maxVertical") * b.scale;
    let wrap = ctx.params.b("wrapPixelsAround");
    let src = b.img.clone();
    let (w, h) = (src.width as f64, src.height as f64);
    map_xy(&mut b.img, |x, y, px| {
        let (c, a) = unpremul(px);
        let dx = (pick(hs, c, a) as f64 - 0.5) * 2.0 * mh;
        let dy = (pick(vs, c, a) as f64 - 0.5) * 2.0 * mv;
        if dx == 0.0 && dy == 0.0 {
            return px;
        }
        let (mut sx, mut sy) = (x as f64 + 0.5 + dx, y as f64 + 0.5 + dy);
        if wrap {
            sx = sx.rem_euclid(w);
            sy = sy.rem_euclid(h);
            src.sample_bilinear_clamped(sx, sy)
        } else {
            src.sample_bilinear(sx, sy)
        }
    });
    b
}

// ---------------------------------------------------------------- Optics Compensation

fn optics_compensation(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let fov = ctx.params.f("fieldOfView").clamp(0.0, 179.0).to_radians();
    if fov < 1e-6 {
        return b;
    }
    let reverse = ctx.params.b("reverseLensDistortion");
    let (_, _, lw, lh) = layer_rect(ctx, &b);
    let r_ref = match ctx.params.e("fovOrientation") {
        1 => lh * 0.5,
        2 => (lw * lw + lh * lh).sqrt() * 0.5,
        _ => lw * 0.5,
    }
    .max(1.0);
    let c = b.to_px(ctx.params.v2("viewCenter"));
    let half = fov * 0.5;
    let f = r_ref / half.tan();
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let r = (dx * dx + dy * dy).sqrt();
        if r < 1e-9 {
            return Some((x, y));
        }
        let rs = if reverse {
            (r / f).atan() / half * r_ref
        } else {
            let th = r / r_ref * half;
            if th >= FRAC_PI_2 - 1e-4 {
                return None;
            }
            f * th.tan()
        };
        let k = rs / r;
        Some((c.0 + dx * k, c.1 + dy * k))
    });
    b
}

// ---------------------------------------------------------------- Magnify

fn magnify(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let square = ctx.params.e("shape") == 1;
    let c = b.to_px(ctx.params.v2("center"));
    let mag = (ctx.params.f("magnification") / 100.0).max(0.01);
    let link = ctx.params.e("link");
    let mut size = ctx.params.f("size") * b.scale;
    let mut feather = ctx.params.f("feather") * b.scale;
    if link >= 1 {
        size *= mag;
    }
    if link == 2 {
        feather *= mag;
    }
    let op = (ctx.params.f("opacity") / 100.0) as f32;
    let src = b.img.clone();
    map_xy(&mut b.img, |x, y, px| {
        let (dx, dy) = (x as f64 + 0.5 - c.0, y as f64 + 0.5 - c.1);
        let d = if square { dx.abs().max(dy.abs()) } else { (dx * dx + dy * dy).sqrt() };
        let w = if feather > 0.01 { 1.0 - smoothstep((size - feather) as f32, size as f32, d as f32) } else { (size - d + 0.5).clamp(0.0, 1.0) as f32 } * op;
        if w <= 0.0 {
            return px;
        }
        let m = src.sample_bilinear(c.0 + dx / mag, c.1 + dy / mag);
        lerp4(px, m, w)
    });
    b
}

// ---------------------------------------------------------------- grid rasteriser

type Pt = (f64, f64);

/// Warp `src` into a `w`×`h` image: grid point `i` (row-major, (nx+1)×(ny+1)) at `dest[i]` shows
/// the source at `srcp[i]`. Uncovered pixels are transparent.
fn grid_warp(src: &Image, w: u32, h: u32, nx: usize, ny: usize, dest: &[Pt], srcp: &[Pt]) -> Image {
    let idx = |i: usize, j: usize| j * (nx + 1) + i;
    let mut tris: Vec<[usize; 3]> = Vec::with_capacity(nx * ny * 2);
    for j in 0..ny {
        for i in 0..nx {
            let (a, b2, c, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
            tris.push([a, b2, c]);
            tris.push([a, c, d]);
        }
    }
    let hh = h as usize;
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); hh];
    for (t, tri) in tris.iter().enumerate() {
        let ys = tri.map(|k| dest[k].1);
        let y0 = (ys[0].min(ys[1]).min(ys[2]) - 0.5).floor().max(0.0) as usize;
        let y1 = (ys[0].max(ys[1]).max(ys[2]) + 0.5).ceil().max(0.0) as usize;
        for y in y0..y1.min(hh) {
            buckets[y].push(t);
        }
    }
    let mut out = Image::new(w, h);
    out.rows_mut().for_each(|(y, row)| {
        let py = y as f64 + 0.5;
        for &t in &buckets[y] {
            let [ia, ib, ic] = tris[t];
            let (a, bb, c) = (dest[ia], dest[ib], dest[ic]);
            let den = (bb.1 - c.1) * (a.0 - c.0) + (c.0 - bb.0) * (a.1 - c.1);
            if den.abs() < 1e-12 {
                continue;
            }
            let x0 = (a.0.min(bb.0).min(c.0) - 0.5).floor().max(0.0) as usize;
            let x1 = ((a.0.max(bb.0).max(c.0) + 0.5).ceil().max(0.0) as usize).min(row.len());
            for x in x0..x1 {
                if row[x][3] > 0.0 {
                    continue;
                }
                let px = x as f64 + 0.5;
                let l1 = ((bb.1 - c.1) * (px - c.0) + (c.0 - bb.0) * (py - c.1)) / den;
                let l2 = ((c.1 - a.1) * (px - c.0) + (a.0 - c.0) * (py - c.1)) / den;
                let l3 = 1.0 - l1 - l2;
                let e = -1e-9;
                if l1 < e || l2 < e || l3 < e {
                    continue;
                }
                let (sa, sb, sc) = (srcp[ia], srcp[ib], srcp[ic]);
                let sx = sa.0 * l1 + sb.0 * l2 + sc.0 * l3;
                let sy = sa.1 * l1 + sb.1 * l2 + sc.1 * l3;
                row[x] = src.sample_bilinear(sx, sy);
            }
        }
    });
    out
}

fn parse_mesh(s: &str) -> Vec<Pt> {
    s.split(|c: char| c.is_whitespace() || c == ';')
        .filter(|t| !t.is_empty())
        .filter_map(|t| {
            let mut it = t.split(',');
            let x = it.next()?.trim().parse::<f64>().ok()?;
            let y = it.next()?.trim().parse::<f64>().ok()?;
            Some((x, y))
        })
        .collect()
}

fn mesh_warp(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let rows = ctx.params.f("rows").round().clamp(1.0, 31.0) as usize;
    let cols = ctx.params.f("columns").round().clamp(1.0, 31.0) as usize;
    let q = ctx.params.f("quality").round().clamp(1.0, 10.0) as usize;
    let offs = parse_mesh(ctx.params.s("mesh"));
    if offs.len() != (rows + 1) * (cols + 1) || offs.iter().all(|o| o.0 == 0.0 && o.1 == 0.0) {
        return b;
    }
    let (lw, lh) = (ctx.layer_size[0], ctx.layer_size[1]);
    let vert = |i: usize, j: usize| {
        let o = offs[j * (cols + 1) + i];
        b.to_px([i as f64 / cols as f64 * lw + o.0, j as f64 / rows as f64 * lh + o.1])
    };
    let (nx, ny) = (cols * q, rows * q);
    let mut dest = Vec::with_capacity((nx + 1) * (ny + 1));
    let mut srcp = Vec::with_capacity((nx + 1) * (ny + 1));
    for gj in 0..=ny {
        for gi in 0..=nx {
            let (ci, cj) = ((gi / q).min(cols - 1), (gj / q).min(rows - 1));
            let (u, v) = ((gi - ci * q) as f64 / q as f64, (gj - cj * q) as f64 / q as f64);
            let (p00, p10, p01, p11) = (vert(ci, cj), vert(ci + 1, cj), vert(ci, cj + 1), vert(ci + 1, cj + 1));
            let x = (p00.0 * (1.0 - u) + p10.0 * u) * (1.0 - v) + (p01.0 * (1.0 - u) + p11.0 * u) * v;
            let y = (p00.1 * (1.0 - u) + p10.1 * u) * (1.0 - v) + (p01.1 * (1.0 - u) + p11.1 * u) * v;
            dest.push((x, y));
            srcp.push(b.to_px([gi as f64 / nx as f64 * lw, gj as f64 / ny as f64 * lh]));
        }
    }
    b.img = grid_warp(&b.img, b.img.width, b.img.height, nx, ny, &dest, &srcp);
    b
}

// ---------------------------------------------------------------- Bezier Warp

fn bez(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let (a, bb, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [p0[0] * a + p1[0] * bb + p2[0] * c + p3[0] * d, p0[1] * a + p1[1] * bb + p2[1] * c + p3[1] * d]
}

fn bezier_warp(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let g = |id: &str| ctx.params.v2(id);
    let (tl, tlt, trt, tr) = (g("topLeftVertex"), g("topLeftTangent"), g("topRightTangent"), g("rightTopVertex"));
    let (rtt, rbt, br) = (g("rightTopTangent"), g("rightBottomTangent"), g("bottomRightVertex"));
    let (brt, blt, bl) = (g("bottomRightTangent"), g("bottomLeftTangent"), g("leftBottomVertex"));
    let (lbt, ltt) = (g("leftBottomTangent"), g("leftTopTangent"));
    let n = (ctx.params.f("quality").round().clamp(1.0, 10.0) as usize) * 4;
    let (lw, lh) = (ctx.layer_size[0], ctx.layer_size[1]);
    let mut dest = Vec::with_capacity((n + 1) * (n + 1));
    let mut srcp = Vec::with_capacity((n + 1) * (n + 1));
    for j in 0..=n {
        let v = j as f64 / n as f64;
        let d0 = bez(tl, ltt, lbt, bl, v);
        let d1 = bez(tr, rtt, rbt, br, v);
        for i in 0..=n {
            let u = i as f64 / n as f64;
            let c0 = bez(tl, tlt, trt, tr, u);
            let c1 = bez(bl, blt, brt, br, u);
            let mut s = [0.0; 2];
            for k in 0..2 {
                let bil = (1.0 - u) * (1.0 - v) * tl[k] + u * (1.0 - v) * tr[k] + (1.0 - u) * v * bl[k] + u * v * br[k];
                s[k] = (1.0 - v) * c0[k] + v * c1[k] + (1.0 - u) * d0[k] + u * d1[k] - bil;
            }
            dest.push(b.to_px(s));
            srcp.push(b.to_px([u * lw, v * lh]));
        }
    }
    b.img = grid_warp(&b.img, b.img.width, b.img.height, n, n, &dest, &srcp);
    b
}

// ---------------------------------------------------------------- CC Bend It

fn bend_it(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let mut beta = ctx.params.f("bend").to_radians();
    if beta.abs() < 1e-6 {
        return b;
    }
    let s = b.to_px(ctx.params.v2("start"));
    let e = b.to_px(ctx.params.v2("end"));
    let static_pre = ctx.params.e("renderPrestart") == 1;
    let l = ((e.0 - s.0).powi(2) + (e.1 - s.1).powi(2)).sqrt();
    if l < 1e-6 {
        return b;
    }
    let a = ((e.0 - s.0) / l, (e.1 - s.1) / l);
    let mut n = (-a.1, a.0);
    if beta < 0.0 {
        beta = -beta;
        n = (-n.0, -n.1);
    }
    let rho = l / beta;
    let pe = (rho * beta.sin(), rho - rho * beta.cos());
    let (t, nn) = ((beta.cos(), beta.sin()), (-beta.sin(), beta.cos()));
    let src = b.img.clone();
    let to_src = |u: f64, w: f64| (s.0 + a.0 * u + n.0 * w, s.1 + a.1 * u + n.1 * w);
    map_xy(&mut b.img, |x, y, _| {
        let (vx, vy) = (x as f64 + 0.5 - s.0, y as f64 + 0.5 - s.1);
        let (lx, ly) = (vx * a.0 + vy * a.1, vx * n.0 + vy * n.1);
        let mut cands: [Option<(f64, f64)>; 3] = [None; 3];
        // Arc.
        let (dx, dy) = (lx, ly - rho);
        let th = dx.atan2(-dy);
        if (0.0..=beta).contains(&th) {
            cands[0] = Some((rho * th, rho - (dx * dx + dy * dy).sqrt()));
        }
        // Rigid continuation beyond the end.
        let (ex, ey) = (lx - pe.0, ly - pe.1);
        let u2 = ex * t.0 + ey * t.1;
        if u2 >= 0.0 {
            cands[1] = Some((l + u2, ex * nn.0 + ey * nn.1));
        }
        if lx < 0.0 && static_pre {
            cands[2] = Some((lx, ly));
        }
        for (u, w) in cands.into_iter().flatten() {
            let (sx, sy) = to_src(u, w);
            let v = src.sample_bilinear(sx, sy);
            if v[3] > 1e-4 {
                return v;
            }
        }
        [0.0; 4]
    });
    b
}

// ---------------------------------------------------------------- CC Lens

fn cc_lens(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = b.to_px(ctx.params.v2("center"));
    let (_, _, lw, lh) = layer_rect(ctx, &b);
    let r = (ctx.params.f("size") / 100.0 * lw.max(lh) * 0.5).max(0.5);
    let k = 2f64.powf(ctx.params.f("convergence") / 50.0);
    b.img = remap(&b.img, false, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = (dx * dx + dy * dy).sqrt();
        if d >= r {
            return None;
        }
        if d < 1e-9 {
            return Some((x, y));
        }
        let g = ((d / r).asin() / FRAC_PI_2).powf(k);
        let s = r * g / d;
        Some((c.0 + dx * s, c.1 + dy * s))
    });
    b
}

// ---------------------------------------------------------------- CC Page Turn

fn page_turn(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let f = b.to_px(ctx.params.v2("foldPosition"));
    let phi = ctx.params.f("foldDirection").to_radians();
    let r = (ctx.params.f("foldRadius") * b.scale).max(0.5);
    let lam = ctx.params.f("lightDirection").to_radians();
    let render = ctx.params.e("render");
    let bo = (ctx.params.f("backPageOpacity") / 100.0) as f32;
    let paper = ctx.params.color("paperColor");
    // The fold travels along `fd`; the lifted part lies on the other side (toward `n`).
    let n = (-phi.sin(), phi.cos());
    let ldot = (n.0 * lam.sin() - n.1 * lam.cos()) as f32;
    let (show_front, show_back) = (render != 1, render != 2);
    let src = b.img.clone();
    map_xy(&mut b.img, |x, y, _| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let d = (px - f.0) * n.0 + (py - f.1) * n.1;
        if d > r {
            return [0.0; 4];
        }
        // (u, back side?, shade) from the top layer down.
        let mut cands: [Option<(f64, bool, f32)>; 4] = [None; 4];
        if d <= 0.0 {
            cands[0] = Some((PI * r - d, true, 0.9));
            cands[3] = Some((d, false, 1.0));
        } else {
            let a = (d / r).clamp(-1.0, 1.0).asin();
            cands[1] = Some((PI * r - r * a, true, 0.6 + 0.4 * a.cos() as f32));
            let psi = (a / FRAC_PI_2) as f32;
            cands[2] = Some((r * a, false, 1.0 - 0.4 * psi * (0.5 + 0.5 * ldot)));
        }
        for (u, back, shade) in cands.into_iter().flatten() {
            if (back && !show_back) || (!back && !show_front) {
                continue;
            }
            let k = u - d;
            let v = src.sample_bilinear(px + k * n.0, py + k * n.1);
            if v[3] <= 1e-4 {
                continue;
            }
            let (c, a) = unpremul(v);
            let c = if back { [0, 1, 2].map(|i| paper[i] * (1.0 - bo) + c[i] * bo) } else { c };
            return premul(c.map(|v| v * shade), a);
        }
        [0.0; 4]
    });
    b
}

// ---------------------------------------------------------------- CC Tiler / CC Griddler

fn cc_tiler(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let s = (ctx.params.f("scale") / 100.0).max(0.001);
    let c = b.to_px(ctx.params.v2("center"));
    let blend = (ctx.params.f("blendWithOriginal") / 100.0) as f32;
    let (x0, y0, w, h) = layer_rect(ctx, &b);
    let (w, h) = (w.max(1.0), h.max(1.0));
    let src = b.img.clone();
    map_xy(&mut b.img, |x, y, px| {
        let qx = c.0 + (x as f64 + 0.5 - c.0) / s;
        let qy = c.1 + (y as f64 + 0.5 - c.1) / s;
        let sx = x0 + (qx - x0).rem_euclid(w);
        let sy = y0 + (qy - y0).rem_euclid(h);
        let t: Px = src.sample_bilinear_clamped(sx, sy);
        lerp4(t, px, blend)
    });
    b
}

fn cc_griddler(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let hs = (ctx.params.f("horizontalScale") / 100.0).max(0.001);
    let vs = (ctx.params.f("verticalScale") / 100.0).max(0.001);
    let rot = ctx.params.f("rotation").to_radians();
    let cut = ctx.params.b("cutTiles");
    let (x0, y0, lw, _) = layer_rect(ctx, &b);
    let t = (ctx.params.f("tileSize") / 100.0 * lw).max(1.0);
    if (hs - 1.0).abs() < 1e-9 && (vs - 1.0).abs() < 1e-9 && rot.abs() < 1e-9 && !cut {
        return b;
    }
    let (sr, cr) = rot.sin_cos();
    let src = b.img.clone();
    map_xy(&mut b.img, |x, y, _| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let tcx = x0 + (((px - x0) / t).floor() + 0.5) * t;
        let tcy = y0 + (((py - y0) / t).floor() + 0.5) * t;
        let (lx, ly) = (px - tcx, py - tcy);
        if cut && lx.abs().max(ly.abs()) > 0.45 * t {
            return [0.0; 4];
        }
        let (rx, ry) = (lx * cr + ly * sr, -lx * sr + ly * cr);
        src.sample_bilinear(tcx + rx / hs, tcy + ry / vs)
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let srcs = || popup(&SRC_NAMES);
    vec![
        spec(
            "ec.distort.turbulentdisplace",
            "Turbulent Displace",
            vec![
                p(
                    "displacement",
                    "Displacement",
                    Value::Enum(0),
                    popup(&["Turbulent", "Bulge", "Twist", "Turbulent Smoother", "Horizontal Displacement", "Vertical Displacement", "Cross Displacement"]),
                ),
                p("amount", "Amount", num(50.0), slider(-1000.0, 1000.0, -200.0, 200.0, 1)),
                p("size", "Size", num(100.0), slider(2.0, 4000.0, 2.0, 400.0, 1)),
                p("offset", "Offset (Turbulence)", pt(0.5, 0.5), ParamUi::Point),
                p("complexity", "Complexity", num(1.0), slider(1.0, 10.0, 1.0, 10.0, 1)),
                p("evolution", "Evolution", num(0.0), ParamUi::Angle),
                p("pinning", "Pinning", Value::Enum(0), popup(&["None", "Pin All Edges", "Pin Horizontal Edges", "Pin Vertical Edges"])),
            ],
            turbulent_displace,
        ),
        spec(
            "ec.distort.displacementmap",
            "Displacement Map",
            vec![
                p("useForHorizontal", "Use For Horizontal Displacement", Value::Enum(0), srcs()),
                p("maxHorizontal", "Max Horizontal Displacement", num(5.0), slider(-32000.0, 32000.0, -100.0, 100.0, 1)),
                p("useForVertical", "Use For Vertical Displacement", Value::Enum(1), srcs()),
                p("maxVertical", "Max Vertical Displacement", num(5.0), slider(-32000.0, 32000.0, -100.0, 100.0, 1)),
                p("wrapPixelsAround", "Wrap Pixels Around", Value::Bool(false), ParamUi::Checkbox),
            ],
            displacement_map,
        ),
        spec(
            "ec.distort.opticscompensation",
            "Optics Compensation",
            vec![
                p("fieldOfView", "Field Of View (FOV)", num(0.0), slider(0.0, 180.0, 0.0, 180.0, 1)),
                p("reverseLensDistortion", "Reverse Lens Distortion", Value::Bool(false), ParamUi::Checkbox),
                p("fovOrientation", "FOV Orientation", Value::Enum(0), popup(&["Horizontal", "Vertical", "Diagonal"])),
                p("viewCenter", "View Center", pt(0.5, 0.5), ParamUi::Point),
            ],
            optics_compensation,
        ),
        spec(
            "ec.distort.magnify",
            "Magnify",
            vec![
                p("shape", "Shape", Value::Enum(0), popup(&["Circle", "Square"])),
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("magnification", "Magnification", num(200.0), slider(1.0, 10000.0, 100.0, 600.0, 1)),
                p("link", "Link", Value::Enum(0), popup(&["None", "Size To Magnification", "Size & Feather To Magnification"])),
                p("size", "Size", num(100.0), slider(0.0, 4000.0, 0.0, 1000.0, 1)),
                p("feather", "Feather", num(0.0), slider(0.0, 4000.0, 0.0, 200.0, 1)),
                p("opacity", "Opacity", num(100.0), pct(100.0)),
            ],
            magnify,
        ),
        spec(
            "ec.distort.meshwarp",
            "Mesh Warp",
            vec![
                p("rows", "Rows", num(7.0), slider(1.0, 31.0, 1.0, 31.0, 0)),
                p("columns", "Columns", num(7.0), slider(1.0, 31.0, 1.0, 31.0, 0)),
                p("quality", "Quality", num(8.0), slider(1.0, 10.0, 1.0, 10.0, 0)),
                p("mesh", "Distortion Mesh", Value::Str(String::new()), ParamUi::Hidden),
            ],
            mesh_warp,
        ),
        spec(
            "ec.distort.bezierwarp",
            "Bezier Warp",
            vec![
                p("topLeftVertex", "Top Left Vertex", pt(0.0, 0.0), ParamUi::Point),
                p("topLeftTangent", "Top Left Tangent", pt(1.0 / 3.0, 0.0), ParamUi::Point),
                p("topRightTangent", "Top Right Tangent", pt(2.0 / 3.0, 0.0), ParamUi::Point),
                p("rightTopVertex", "Right Top Vertex", pt(1.0, 0.0), ParamUi::Point),
                p("rightTopTangent", "Right Top Tangent", pt(1.0, 1.0 / 3.0), ParamUi::Point),
                p("rightBottomTangent", "Right Bottom Tangent", pt(1.0, 2.0 / 3.0), ParamUi::Point),
                p("bottomRightVertex", "Bottom Right Vertex", pt(1.0, 1.0), ParamUi::Point),
                p("bottomRightTangent", "Bottom Right Tangent", pt(2.0 / 3.0, 1.0), ParamUi::Point),
                p("bottomLeftTangent", "Bottom Left Tangent", pt(1.0 / 3.0, 1.0), ParamUi::Point),
                p("leftBottomVertex", "Left Bottom Vertex", pt(0.0, 1.0), ParamUi::Point),
                p("leftBottomTangent", "Left Bottom Tangent", pt(0.0, 2.0 / 3.0), ParamUi::Point),
                p("leftTopTangent", "Left Top Tangent", pt(0.0, 1.0 / 3.0), ParamUi::Point),
                p("quality", "Quality", num(8.0), slider(1.0, 10.0, 1.0, 10.0, 0)),
            ],
            bezier_warp,
        ),
        spec(
            "ec.distort.ccbendit",
            "CC Bend It",
            vec![
                p("bend", "Bend", num(0.0), ParamUi::Angle),
                p("start", "Start", pt(0.5, 0.2), ParamUi::Point),
                p("end", "End", pt(0.5, 0.8), ParamUi::Point),
                p("renderPrestart", "Render Prestart", Value::Enum(1), popup(&["None", "Static"])),
            ],
            bend_it,
        ),
        spec(
            "ec.distort.cclens",
            "CC Lens",
            vec![
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("size", "Size", num(50.0), slider(0.0, 400.0, 0.0, 200.0, 1)),
                p("convergence", "Convergence", num(0.0), slider(-100.0, 100.0, -100.0, 100.0, 1)),
            ],
            cc_lens,
        ),
        spec(
            "ec.distort.ccpageturn",
            "CC Page Turn",
            vec![
                p("foldPosition", "Fold Position", pt(0.7, 0.7), ParamUi::Point),
                p("foldDirection", "Fold Direction", num(315.0), ParamUi::Angle),
                p("foldRadius", "Fold Radius", num(20.0), slider(0.0, 1000.0, 0.0, 200.0, 1)),
                p("lightDirection", "Light Direction", num(-45.0), ParamUi::Angle),
                p("render", "Render", Value::Enum(0), popup(&["Front & Back Page", "Back Page", "Front Page"])),
                p("backPageOpacity", "Back Opacity", num(75.0), pct(100.0)),
                p("paperColor", "Paper Color", col(1.0, 1.0, 1.0), ParamUi::Color),
            ],
            page_turn,
        ),
        spec(
            "ec.distort.cctiler",
            "CC Tiler",
            vec![
                p("scale", "Scale", num(50.0), slider(1.0, 10000.0, 1.0, 200.0, 1)),
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("blendWithOriginal", "Blend w. Original", num(0.0), pct(100.0)),
            ],
            cc_tiler,
        ),
        spec(
            "ec.distort.ccgriddler",
            "CC Griddler",
            vec![
                p("horizontalScale", "Horizontal Scale", num(100.0), slider(1.0, 1000.0, 1.0, 400.0, 1)),
                p("verticalScale", "Vertical Scale", num(100.0), slider(1.0, 1000.0, 1.0, 400.0, 1)),
                p("tileSize", "Tile Size", num(10.0), slider(0.5, 100.0, 0.5, 100.0, 1)),
                p("rotation", "Rotation", num(0.0), ParamUi::Angle),
                p("cutTiles", "Cut Tiles", Value::Bool(false), ParamUi::Checkbox),
            ],
            cc_griddler,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params;

    fn test_img(w: u32, h: u32) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let a = 1.0;
                img.set(x, y, [x as f32 / w as f32 * a, y as f32 / h as f32 * a, ((x * 7 + y * 3) % 11) as f32 / 11.0 * a, a]);
            }
        }
        img
    }

    fn run(id: &str, set: &[(&str, Value)], img: Image) -> Buf {
        let s = crate::find(id).unwrap();
        let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
        let ls = [img.width as f64, img.height as f64];
        for p in &s.params {
            if let (ParamUi::Point, Value::Vec2(v)) = (&p.ui, &p.default) {
                params.values.insert(p.id.to_string(), Value::Vec2([v[0] * ls[0], v[1] * ls[1]]));
            }
        }
        for (k, v) in set {
            params.values.insert(k.to_string(), v.clone());
        }
        let ctx = EffectCtx { params: &params, time: 0.5, layer_size: ls, seed: 3, adjustment: false };
        crate::apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 })
    }

    fn max_diff(a: &Image, b: &Image) -> f32 {
        a.data.iter().zip(&b.data).map(|(p, q)| (0..4).map(|i| (p[i] - q[i]).abs()).fold(0.0, f32::max)).fold(0.0, f32::max)
    }

    #[test]
    fn displacement_map_half_is_identity() {
        let img = test_img(32, 24);
        let o = run("ec.distort.displacementmap", &[("useForHorizontal", Value::Enum(9)), ("useForVertical", Value::Enum(9))], img.clone());
        assert_eq!(o.img, img);
        let o = run(
            "ec.distort.displacementmap",
            &[("useForHorizontal", Value::Enum(8)), ("useForVertical", Value::Enum(10)), ("maxHorizontal", num(2.0))],
            img.clone(),
        );
        // Full → +2 px horizontally: pixel x shows source x + 2.
        assert!((o.img.get(5, 5)[0] - img.get(7, 5)[0]).abs() < 1e-5);
    }

    #[test]
    fn mesh_warp_empty_and_zero_are_identity() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.meshwarp", &[], img.clone()).img, img);
        let zeros = vec!["0,0"; 64].join(" ");
        assert_eq!(run("ec.distort.meshwarp", &[("mesh", Value::Str(zeros))], img.clone()).img, img);
    }

    #[test]
    fn mesh_warp_uniform_offset_translates() {
        let img = test_img(32, 24);
        let shift = vec!["4,0"; 64].join(" ");
        let o = run("ec.distort.meshwarp", &[("mesh", Value::Str(shift))], img.clone());
        for y in 2..22 {
            for x in 6..30 {
                let (a, b2) = (o.img.get(x, y), img.get(x - 4, y));
                assert!((0..4).all(|i| (a[i] - b2[i]).abs() < 1e-4), "({x},{y}) {a:?} vs {b2:?}");
            }
        }
        assert!(o.img.get(1, 10)[3] < 1e-6);
    }

    #[test]
    fn bezier_warp_default_is_identity() {
        let img = test_img(32, 24);
        let o = run("ec.distort.bezierwarp", &[], img.clone());
        assert!(max_diff(&o.img, &img) < 1e-3, "{}", max_diff(&o.img, &img));
    }

    #[test]
    fn optics_fov_zero_is_identity_and_nonzero_warps() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.opticscompensation", &[], img.clone()).img, img);
        assert!(max_diff(&run("ec.distort.opticscompensation", &[("fieldOfView", num(90.0))], img.clone()).img, &img) > 1e-3);
    }

    #[test]
    fn bend_it_zero_is_identity() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.ccbendit", &[], img.clone()).img, img);
        assert!(max_diff(&run("ec.distort.ccbendit", &[("bend", num(45.0))], img.clone()).img, &img) > 1e-3);
    }

    #[test]
    fn magnify_100_is_identity() {
        let img = test_img(32, 24);
        assert!(max_diff(&run("ec.distort.magnify", &[("magnification", num(100.0))], img.clone()).img, &img) < 1e-6);
        let o = run("ec.distort.magnify", &[("size", num(8.0))], img.clone());
        // Centre pixel unchanged; slightly off-centre samples move toward the centre.
        assert!(max_diff(&o.img, &img) > 1e-3);
        assert_eq!(o.img.get(0, 0), img.get(0, 0));
    }

    #[test]
    fn tiler_100_is_identity_and_50_tiles() {
        let img = test_img(32, 24);
        assert!(max_diff(&run("ec.distort.cctiler", &[("scale", num(100.0))], img.clone()).img, &img) < 1e-6);
        let o = run("ec.distort.cctiler", &[], img.clone());
        assert!(o.img.data.iter().all(|p| p[3] > 0.99));
    }

    #[test]
    fn turbulent_displace_zero_amount_is_identity() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.turbulentdisplace", &[("amount", num(0.0))], img.clone()).img, img);
        assert!(max_diff(&run("ec.distort.turbulentdisplace", &[], img.clone()).img, &img) > 1e-3);
    }

    #[test]
    fn griddler_default_is_identity() {
        let img = test_img(32, 24);
        assert_eq!(run("ec.distort.ccgriddler", &[], img.clone()).img, img);
    }

    #[test]
    fn page_turn_removes_corner_and_keeps_far_side() {
        let img = test_img(64, 64);
        let o = run("ec.distort.ccpageturn", &[("foldRadius", num(4.0))], img.clone());
        // Bottom-right corner is lifted away; top-left untouched.
        assert!(o.img.get(63, 63)[3] < 1e-6);
        assert_eq!(o.img.get(2, 2), img.get(2, 2));
    }

    #[test]
    fn cc_lens_clears_outside() {
        let img = test_img(32, 32);
        let o = run("ec.distort.cclens", &[], img);
        assert!(o.img.get(0, 0)[3] < 1e-6);
        assert!(o.img.get(16, 16)[3] > 0.99);
    }
}
