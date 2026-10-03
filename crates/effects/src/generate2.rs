//! Generate effects, batch 2: Cell Pattern (Worley), Ellipse, Lens Flare, Beam, Radio Waves,
//! Advanced Lightning, CC Light Rays, CC Light Burst 2.5, CC Light Sweep.
//!
//! All designs are original procedural renderings written from the public descriptions of what
//! each effect produces.

use std::f64::consts::PI;

use effectcraft_color::luminance;
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::Px;
use rayon::prelude::*;

use crate::util::{Plane, Rng, gauss_plane, hash1, layer_rect, lerp3, map_xy, smoothstep, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Generate", params, render, gpu: false, float: true }
}

fn pt(x: f64, y: f64) -> Value {
    Value::Vec2([x, y])
}

fn pct() -> ParamUi {
    slider(0.0, 100.0, 0.0, 100.0, 1)
}

/// Premultiplied "over": `src` on top of `dst`.
#[inline]
fn over(dst: Px, src: Px) -> Px {
    let k = 1.0 - src[3];
    [src[0] + dst[0] * k, src[1] + dst[1] * k, src[2] + dst[2] * k, src[3] + dst[3] * k]
}

/// Add light (straight rgb `l`, coverage `la`) onto a premultiplied pixel.
#[inline]
fn add_light(px: Px, l: [f32; 3], la: f32) -> Px {
    let la = la.clamp(0.0, 1.0);
    [px[0] + l[0], px[1] + l[1], px[2] + l[2], (px[3] + la * (1.0 - px[3])).clamp(0.0, 1.0)]
}

// ---------------------------------------------------------------- Cell Pattern

/// Cell Pattern options. The HQ variants share the shaping of their standard counterparts
/// (we always render at full precision).
const CELL_PATTERNS: [&str; 12] = [
    "Bubbles",
    "Crystals",
    "Plates",
    "Static Plates",
    "Crystallize",
    "Pillow",
    "Crystals HQ",
    "Plates HQ",
    "Static Plates HQ",
    "Crystallize HQ",
    "Mixed Crystals HQ",
    "Tubular",
];

/// Shaping used for Cell Pattern option `i` (see [`cell_value`]).
pub fn pattern_kind(i: u32) -> u32 {
    match i {
        0..=5 => i,
        6 => 1,
        7 => 2,
        8 => 3,
        9 => 4,
        10 => 6,
        _ => 8,
    }
}

/// Worley distances for point (u, v) in cell units: (F1, F2, hash of the nearest cell).
/// `tile` repeats the cell layout every (columns, rows) cells.
fn worley(u: f64, v: f64, disperse: f64, evo: f64, seed: u32, tile: Option<(i64, i64)>) -> (f64, f64, f32) {
    let (cx, cy) = (u.floor() as i64, v.floor() as i64);
    let mut f1 = f64::INFINITY;
    let mut f2 = f64::INFINITY;
    let mut id = 0.0f32;
    for j in -1..=1 {
        for i in -1..=1 {
            let (gx, gy) = (cx + i, cy + j);
            let (tx, ty) = match tile {
                Some((nx, ny)) => (gx.rem_euclid(nx), gy.rem_euclid(ny)),
                None => (gx, gy),
            };
            let (hx, hy) = (tx as i32 as u32, ty as i32 as u32);
            let h1 = hash1(hx, hy, seed) as f64;
            let h2 = hash1(hx, hy, seed.wrapping_add(1)) as f64;
            let h3 = hash1(hx, hy, seed.wrapping_add(2)) as f64;
            let ang = 2.0 * PI * (h3 + evo);
            let px = gx as f64 + 0.5 + (h1 - 0.5) * disperse + 0.15 * disperse * ang.cos();
            let py = gy as f64 + 0.5 + (h2 - 0.5) * disperse + 0.15 * disperse * ang.sin();
            let d = ((px - u).powi(2) + (py - v).powi(2)).sqrt();
            if d < f1 {
                f2 = f1;
                f1 = d;
                id = hash1(hx, hy, seed.wrapping_add(3));
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1, f2, id)
}

fn cell_value(pattern: u32, f1: f64, f2: f64, id: f32) -> f32 {
    let (f1, f2) = (f1 as f32, f2 as f32);
    let e = f2 - f1;
    match pattern {
        0 => (1.0 - f1 * 1.3).clamp(0.0, 1.0),               // Bubbles
        1 => id * 0.8 + 0.2 * (1.0 - f1).clamp(0.0, 1.0),    // Crystals
        2 => (e * 2.0).clamp(0.0, 1.0),                      // Plates
        3 => id * smoothstep(0.0, 0.06, e),                  // Static Plates
        4 => id,                                             // Crystallize
        5 => (1.0 - f1 * f1 * 2.0).clamp(0.0, 1.0),          // Pillow
        6 => id * (e * 3.0).clamp(0.0, 1.0),                 // Mixed Crystals
        _ => 1.0 - ((e - 0.15).abs() * 5.0).clamp(0.0, 1.0), // Tubular
    }
}

fn overflow(v: f32, mode: u32) -> f32 {
    match mode {
        1 => 0.5 + 0.5 * ((v - 0.5) * 2.0).tanh(),
        2 => {
            let t = v.rem_euclid(2.0);
            if t > 1.0 { 2.0 - t } else { t }
        }
        _ => v.clamp(0.0, 1.0),
    }
}

fn cell_pattern(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pattern = pattern_kind(ctx.params.e("cellPattern"));
    let invert = ctx.params.b("invert");
    let contrast = ctx.params.f("contrast") as f32 / 100.0;
    let ov = ctx.params.e("overflow");
    let disperse = ctx.params.f("disperse").clamp(0.0, 1.5);
    let size = (ctx.params.f("size") * b.scale).max(1.0);
    let off = b.to_px(ctx.params.v2("offset"));
    // Static Plates keep their shades still while the cells move.
    let mut evo = ctx.params.f("evolution") / 360.0;
    // Cycle Evolution loops the evolution every `cycle` revolutions.
    if ctx.params.b("evolutionOptions/cycleEvolution") {
        evo = evo.rem_euclid(ctx.params.f("evolutionOptions/cycle").round().max(1.0));
    }
    let seed = (ctx.params.f("evolutionOptions/randomSeed") as i64 as u32).wrapping_mul(7919) ^ 0xce11;
    let tile = ctx
        .params
        .b("tilingOptions/enableTiling")
        .then(|| (ctx.params.f("tilingOptions/cellsHorizontal").round().max(1.0) as i64, ctx.params.f("tilingOptions/cellsVertical").round().max(1.0) as i64));
    map_xy(&mut b.img, |x, y, _| {
        let u = (x as f64 + 0.5 - off.0) / size;
        let v = (y as f64 + 0.5 - off.1) / size;
        let (f1, f2, id) = worley(u, v, disperse, evo, seed, tile);
        let mut val = cell_value(pattern, f1, f2, id);
        val = overflow((val - 0.5) * contrast + 0.5, ov);
        if invert {
            val = 1.0 - val;
        }
        [val, val, val, 1.0]
    });
    b
}

// ---------------------------------------------------------------- Ellipse

fn ellipse(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = b.to_px(ctx.params.v2("center"));
    let rx = (ctx.params.f("width") * b.scale * 0.5).max(0.5);
    let ry = (ctx.params.f("height") * b.scale * 0.5).max(0.5);
    let half = (ctx.params.f("thickness") * b.scale * 0.5).max(0.25);
    let soft = (ctx.params.f("softness") / 100.0).clamp(0.0, 1.0);
    let inside = ctx.params.color("insideColor");
    let outside = ctx.params.color("outsideColor");
    let composite = ctx.params.b("compositeOnOriginal");
    map_xy(&mut b.img, |x, y, px| {
        let (dx, dy) = (x as f64 + 0.5 - c.0, y as f64 + 0.5 - c.1);
        let q = ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt();
        let dist = if q < 1e-9 {
            rx.min(ry)
        } else {
            let gx = dx / (rx * rx) / q;
            let gy = dy / (ry * ry) / q;
            ((q - 1.0) / (gx * gx + gy * gy).sqrt().max(1e-9)).abs()
        };
        let t = (dist / half) as f32;
        let a = if soft > 1e-3 { 1.0 - smoothstep(1.0 - soft as f32, 1.0, t) } else { (half - dist + 0.5).clamp(0.0, 1.0) as f32 };
        let cc = lerp3([inside[0], inside[1], inside[2]], [outside[0], outside[1], outside[2]], smoothstep(0.0, 1.0, t));
        let g = [cc[0] * a, cc[1] * a, cc[2] * a, a];
        if composite { over(px, g) } else { g }
    });
    b
}

// ---------------------------------------------------------------- Lens Flare

struct Ghost {
    t: f64,
    r: f64,
    c: [f32; 3],
    k: f32,
    ring: bool,
}

fn ghosts(lens: u32) -> Vec<Ghost> {
    let g = |t, r, c: [f32; 3], k, ring| Ghost { t, r, c, k, ring };
    match lens {
        1 => vec![
            g(0.45, 0.03, [0.4, 0.7, 1.0], 0.20, false),
            g(0.9, 0.06, [0.5, 1.0, 0.6], 0.12, false),
            g(1.3, 0.12, [1.0, 0.6, 0.3], 0.10, true),
            g(1.6, 0.04, [0.8, 0.5, 1.0], 0.15, false),
        ],
        2 => vec![g(0.6, 0.02, [1.0, 0.8, 0.5], 0.25, false), g(1.1, 0.05, [0.6, 0.6, 1.0], 0.12, false), g(1.4, 0.09, [0.4, 0.9, 1.0], 0.08, true)],
        _ => vec![
            g(0.25, 0.02, [1.0, 0.9, 0.6], 0.25, false),
            g(0.5, 0.04, [0.3, 1.0, 0.5], 0.15, false),
            g(0.75, 0.025, [0.5, 0.6, 1.0], 0.2, false),
            g(1.15, 0.07, [1.0, 0.5, 0.3], 0.10, false),
            g(1.45, 0.1, [0.6, 0.4, 1.0], 0.08, true),
            g(1.8, 0.05, [0.4, 0.9, 1.0], 0.15, false),
            g(2.1, 0.14, [1.0, 0.8, 0.4], 0.06, true),
        ],
    }
}

fn lens_flare(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let f = b.to_px(ctx.params.v2("flareCenter"));
    let bright = (ctx.params.f("flareBrightness") / 100.0) as f32;
    let lens = ctx.params.e("lensType");
    let blend = (ctx.params.f("blendWithOriginal") / 100.0) as f32;
    let (x0, y0, w, h) = layer_rect(ctx, &b);
    let c = (x0 + w * 0.5, y0 + h * 0.5);
    let diag = (w * w + h * h).sqrt().max(1.0);
    let gs = ghosts(lens);
    let (core_k, rays, halo_r) = match lens {
        1 => (0.035, 6.0, 0.18),
        2 => (0.02, 12.0, 0.3),
        _ => (0.03, 8.0, 0.25),
    };
    map_xy(&mut b.img, |x, y, px| {
        let (dx, dy) = (x as f64 + 0.5 - f.0, y as f64 + 0.5 - f.1);
        let r = (dx * dx + dy * dy).sqrt() / diag;
        let th = dy.atan2(dx);
        let core = 1.5 * (-(r / core_k).powi(2)).exp() + 0.25 * (-r / 0.12).exp();
        let streak = (rays * 0.5 * th).cos().abs().powi(40) * (-r / 0.3).exp() * 0.35;
        let halo = (-((r - halo_r) / 0.012).powi(2)).exp() * 0.12;
        let base = (core + streak) as f32;
        let mut l = [base, base * 0.95, base * 0.85];
        let hal = halo as f32;
        l[0] += hal * 0.6;
        l[1] += hal * 0.8;
        l[2] += hal;
        for g in &gs {
            let gx = f.0 + (c.0 - f.0) * g.t;
            let gy = f.1 + (c.1 - f.1) * g.t;
            let d = ((x as f64 + 0.5 - gx).powi(2) + (y as f64 + 0.5 - gy).powi(2)).sqrt() / diag;
            let v = if g.ring { (-((d - g.r) / (g.r * 0.12)).powi(2)).exp() as f32 } else { smoothstep(g.r as f32, g.r as f32 * 0.8, d as f32) };
            for i in 0..3 {
                l[i] += g.c[i] * g.k * v;
            }
        }
        let l = l.map(|v| v * bright);
        let la = l[0].max(l[1]).max(l[2]).min(1.0);
        let o = add_light(px, l, la);
        crate::util::lerp4(o, px, blend)
    });
    b
}

// ---------------------------------------------------------------- Beam

fn beam(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let s = b.to_px(ctx.params.v2("startPoint"));
    let e = b.to_px(ctx.params.v2("endPoint"));
    let len = (ctx.params.f("length") / 100.0).clamp(0.0, 1.0);
    let time = (ctx.params.f("time") / 100.0).clamp(0.0, 1.0);
    let st = ctx.params.f("startThickness") * b.scale;
    let et = ctx.params.f("endThickness") * b.scale;
    let soft = (ctx.params.f("softness") / 100.0).clamp(0.0, 1.0) as f32;
    let inside = ctx.params.color("insideColor");
    let outside = ctx.params.color("outsideColor");
    let composite = ctx.params.b("compositeOnOriginal");
    // 3D Perspective: thickness follows the beam's position between the two points (it
    // grows / shrinks as it travels); off: the beam's own tail and head take the starting and
    // ending thickness.
    let persp = ctx.params.b("perspective3d");
    let t0 = time * (1.0 - len);
    let t1 = t0 + len;
    let (dx, dy) = (e.0 - s.0, e.1 - s.1);
    let l2 = (dx * dx + dy * dy).max(1e-9);
    map_xy(&mut b.img, |x, y, px| {
        let (vx, vy) = (x as f64 + 0.5 - s.0, y as f64 + 0.5 - s.1);
        let tc = ((vx * dx + vy * dy) / l2).clamp(t0, t1);
        let (qx, qy) = (s.0 + dx * tc, s.1 + dy * tc);
        let dist = ((x as f64 + 0.5 - qx).powi(2) + (y as f64 + 0.5 - qy).powi(2)).sqrt();
        let tk = if persp || t1 - t0 < 1e-9 { tc } else { (tc - t0) / (t1 - t0) };
        let half = ((st + (et - st) * tk) * 0.5).max(0.25);
        let tt = (dist / half) as f32;
        let a = if len <= 0.0 {
            0.0
        } else if soft > 1e-3 {
            1.0 - smoothstep(1.0 - soft, 1.0, tt)
        } else {
            (half - dist + 0.5).clamp(0.0, 1.0) as f32
        };
        let cc = lerp3([inside[0], inside[1], inside[2]], [outside[0], outside[1], outside[2]], smoothstep(0.0, 1.0, tt));
        let g = [cc[0] * a, cc[1] * a, cc[2] * a, a];
        if composite { over(px, g) } else { g }
    });
    b
}

// ---------------------------------------------------------------- Radio Waves

/// Distance from `p` (relative to the wave centre) to a closed outline `pts`, measured along
/// the ray from the centre and corrected to the edge normal.
fn outline_dist(pts: &[(f64, f64)], dx: f64, dy: f64) -> f64 {
    let r = (dx * dx + dy * dy).sqrt();
    if r < 1e-9 {
        return pts.iter().map(|p| (p.0 * p.0 + p.1 * p.1).sqrt()).fold(f64::INFINITY, f64::min);
    }
    let dir = (dx / r, dy / r);
    let n = pts.len();
    let mut best = f64::INFINITY;
    for k in 0..n {
        let p0 = pts[k];
        let p1 = pts[(k + 1) % n];
        let e = (p1.0 - p0.0, p1.1 - p0.1);
        let den = dir.0 * e.1 - dir.1 * e.0;
        if den.abs() < 1e-12 {
            continue;
        }
        let t = (p0.0 * e.1 - p0.1 * e.0) / den;
        // The ray must cross this edge between its endpoints.
        let q = (dir.0 * t - p0.0, dir.1 * t - p0.1);
        let el2 = (e.0 * e.0 + e.1 * e.1).max(1e-12);
        let s = (q.0 * e.0 + q.1 * e.1) / el2;
        if t <= 0.0 || !(-1e-9..=1.0 + 1e-9).contains(&s) {
            continue;
        }
        let d = (r - t).abs() * den.abs() / el2.sqrt();
        best = best.min(d);
    }
    best
}

fn radio_waves(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = b.to_px(ctx.params.v2("producerPoint"));
    let sides = ctx.params.f("polygon/sides").round().clamp(3.0, 64.0);
    let star = ctx.params.b("polygon/star");
    let depth = ctx.params.f("polygon/starDepth").clamp(-1.0, 1.0);
    let freq = ctx.params.f("waveMotion/frequency").max(0.01);
    let expansion = ctx.params.f("waveMotion/expansion") * b.scale;
    let orient = ctx.params.f("waveMotion/orientation").to_radians();
    // Direction: 0° = up, clockwise.
    let dirn = ctx.params.f("waveMotion/direction").to_radians();
    let vel = ctx.params.f("waveMotion/velocity") * b.scale;
    let spin = ctx.params.f("waveMotion/spin").to_radians();
    let life = ctx.params.f("waveMotion/lifespan").max(0.01);
    let color = ctx.params.color("waveStroke/color");
    let op = (ctx.params.f("waveStroke/opacity") / 100.0) as f32;
    let fin = ctx.params.f("waveStroke/fadeInTime").max(0.0);
    let fout = ctx.params.f("waveStroke/fadeOutTime").max(0.0);
    let sw = ctx.params.f("waveStroke/startWidth") * b.scale;
    let ew = ctx.params.f("waveStroke/endWidth") * b.scale;
    let t = ctx.time.max(0.0);
    let k_hi = (t * freq).floor() as i64;
    let k_lo = (((t - life) * freq).floor() as i64).max(0).max(k_hi - 256);
    // (centre, radius, half width, rotation, fade) per live wave.
    let waves: Vec<((f64, f64), f64, f64, f64, f32)> = (k_lo..=k_hi)
        .filter_map(|k| {
            let age = t - k as f64 / freq;
            if !(0.0..life).contains(&age) {
                return None;
            }
            let fi = if fin > 0.0 { (age / fin).min(1.0) } else { 1.0 };
            let fo = if fout > 0.0 { ((life - age) / fout).min(1.0) } else { 1.0 };
            let centre = (c.0 + dirn.sin() * vel * age, c.1 - dirn.cos() * vel * age);
            Some((centre, expansion * age, ((sw + (ew - sw) * age / life) * 0.5).max(0.25), orient + spin * age, (fi * fo) as f32))
        })
        .collect();
    let circle = sides >= 64.0 && !star;
    // Unit outline (radius 1) of the polygon or star, rotated per wave below.
    let n = sides as usize;
    let unit: Vec<(f64, f64)> = if star {
        (0..2 * n).map(|i| (PI * i as f64 / n as f64 - PI / 2.0, if i % 2 == 0 { 1.0 } else { (1.0 + depth).max(0.0) })).collect()
    } else {
        (0..n).map(|i| (2.0 * PI * i as f64 / n as f64 - PI / 2.0, 1.0)).collect()
    };
    let outlines: Vec<Vec<(f64, f64)>> =
        waves.iter().map(|&(_, rad, _, rot, _)| unit.iter().map(|&(a, k)| ((a + rot).cos() * rad * k, (a + rot).sin() * rad * k)).collect()).collect();
    map_xy(&mut b.img, |x, y, px| {
        let mut a = 0.0f32;
        for (wi, &(cc, rad, hw, _, fade)) in waves.iter().enumerate() {
            let (dx, dy) = (x as f64 + 0.5 - cc.0, y as f64 + 0.5 - cc.1);
            let d = if circle { ((dx * dx + dy * dy).sqrt() - rad).abs() } else { outline_dist(&outlines[wi], dx, dy) };
            a = a.max((hw - d + 0.5).clamp(0.0, 1.0) as f32 * fade);
        }
        let a = a * op * color[3];
        over(px, [color[0] * a, color[1] * a, color[2] * a, a])
    });
    b
}

// ---------------------------------------------------------------- Advanced Lightning

type Seg = ((f64, f64), (f64, f64), f32);

/// Advanced Lightning's Lightning Type options.
const LIGHTNING_TYPES: [&str; 8] = ["Direction", "Strike", "Breaking", "Bouncy", "Omni", "Anywhere", "Vertical", "Two-Way Strike"];

fn bolt(a: (f64, f64), z: (f64, f64), depth: u32, inten: f32, turb: f64, fork: f64, decay: f32, rng: &mut Rng, out: &mut Vec<Seg>) {
    if out.len() > 20_000 {
        return;
    }
    let (dx, dy) = (z.0 - a.0, z.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if depth == 0 || len < 1.0 {
        out.push((a, z, inten));
        return;
    }
    let (nx, ny) = (-dy / len.max(1e-9), dx / len.max(1e-9));
    let off = rng.s() * len * 0.22 * turb;
    let m = ((a.0 + z.0) * 0.5 + nx * off, (a.1 + z.1) * 0.5 + ny * off);
    bolt(a, m, depth - 1, inten, turb, fork, decay, rng, out);
    bolt(m, z, depth - 1, inten, turb, fork, decay, rng, out);
    if rng.f() < fork && inten > 0.05 {
        let ang = rng.s() * 0.7;
        let (s, c) = ang.sin_cos();
        let (fx, fy) = ((m.0 - a.0) * c - (m.1 - a.1) * s, (m.0 - a.0) * s + (m.1 - a.1) * c);
        let k = 1.2;
        bolt(m, (m.0 + fx * k, m.1 + fy * k), depth.saturating_sub(1), inten * (1.0 - decay), turb, fork, decay, rng, out);
    }
}

fn lightning_segments(ctx: &EffectCtx, b: &Buf) -> Vec<Seg> {
    let kind = ctx.params.e("lightningType");
    let decay_main = ctx.params.b("decayMainCore");
    let o = b.to_px(ctx.params.v2("origin"));
    let d = b.to_px(ctx.params.v2("direction"));
    let turb = ctx.params.f("turbulence").max(0.0);
    let fork = (ctx.params.f("forking") / 100.0).clamp(0.0, 1.0);
    let decay = ctx.params.f("decay").clamp(0.0, 1.0) as f32;
    let depth = ctx.params.f("expertSettings/complexity").round().clamp(1.0, 12.0) as u32;
    let state = ctx.params.f("conductivityState").floor() as i64 as u64;
    let mut rng = Rng::new(state ^ ((ctx.seed as u64) << 32));
    let mut segs = Vec::new();
    let (h, w) = (b.img.height as f64, b.img.width as f64);
    // LIGHTNING_TYPES order.
    match kind {
        // Direction: travels toward the Direction point and beyond.
        0 => {
            let z = (o.0 + (d.0 - o.0) * 3.0, o.1 + (d.1 - o.1) * 3.0);
            bolt(o, z, depth, 1.0, turb, fork, decay, &mut rng, &mut segs);
        }
        // Breaking: more branching as the points move apart.
        2 => {
            let dist = ((d.0 - o.0).powi(2) + (d.1 - o.1).powi(2)).sqrt();
            let f = (fork * (0.5 + dist / w.max(h).max(1.0))).min(1.0);
            bolt(o, d, depth, 1.0, turb, f, decay, &mut rng, &mut segs);
        }
        // Bouncy: a strike with a stronger zig-zag.
        3 => bolt(o, d, depth, 1.0, turb * 1.6, fork, decay, &mut rng, &mut segs),
        // Omni / Anywhere: bolts in all directions out to the Outer Radius.
        4 | 5 => {
            let len = ((d.0 - o.0).powi(2) + (d.1 - o.1).powi(2)).sqrt().max(w.min(h) * 0.25);
            for _ in 0..4 {
                let a = rng.f() * 2.0 * PI;
                let l = if kind == 5 { len * (0.3 + 0.7 * rng.f()) } else { len };
                let z = (o.0 + a.cos() * l, o.1 + a.sin() * l);
                bolt(o, z, depth, 1.0, turb, fork, decay, &mut rng, &mut segs);
            }
        }
        6 => bolt(o, (o.0, h), depth, 1.0, turb, fork, decay, &mut rng, &mut segs),
        // Two-Way Strike: from both ends toward the middle.
        7 => {
            let m = ((o.0 + d.0) * 0.5, (o.1 + d.1) * 0.5);
            bolt(o, m, depth.saturating_sub(1).max(1), 1.0, turb, fork, decay, &mut rng, &mut segs);
            bolt(d, m, depth.saturating_sub(1).max(1), 1.0, turb, fork, decay, &mut rng, &mut segs);
        }
        // Strike.
        _ => bolt(o, d, depth, 1.0, turb, fork, decay, &mut rng, &mut segs),
    }
    // Decay Main Core: the whole bolt fades with distance from the origin.
    if decay_main {
        let far = segs.iter().map(|(_, z, _)| ((z.0 - o.0).powi(2) + (z.1 - o.1).powi(2)).sqrt()).fold(1e-9, f64::max);
        for (a, _, i) in &mut segs {
            let t = (((a.0 - o.0).powi(2) + (a.1 - o.1).powi(2)).sqrt() / far) as f32;
            *i *= (1.0 - decay * t).max(0.0);
        }
    }
    segs
}

fn advanced_lightning(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let core_r = (ctx.params.f("coreSettings/coreRadius") * b.scale).max(0.3);
    let core_op = (ctx.params.f("coreSettings/coreOpacity") / 100.0) as f32;
    let core_c = ctx.params.color("coreSettings/coreColor");
    let glow_r = ctx.params.f("glowSettings/glowRadius") * b.scale;
    let glow_op = (ctx.params.f("glowSettings/glowOpacity") / 100.0) as f32;
    let glow_c = ctx.params.color("glowSettings/glowColor");
    let composite = ctx.params.b("compositeOnOriginal");
    let segs = lightning_segments(ctx, &b);
    let (w, h) = (b.img.width as usize, b.img.height as usize);
    // Bucket segments per row (by bbox) so rows rasterise in parallel.
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); h];
    for (i, (a, z, _)) in segs.iter().enumerate() {
        let y0 = ((a.1.min(z.1) - core_r - 1.0).floor().max(0.0)) as usize;
        let y1 = ((a.1.max(z.1) + core_r + 1.0).ceil()).min(h as f64) as i64;
        for y in y0..(y1.max(0) as usize).min(h) {
            buckets[y].push(i);
        }
    }
    let mut core = Plane::new(w, h);
    core.data.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
        let py = y as f64 + 0.5;
        for &i in &buckets[y] {
            let (a, z, inten) = segs[i];
            let x0 = ((a.0.min(z.0) - core_r - 1.0).floor().max(0.0)) as usize;
            let x1 = ((a.0.max(z.0) + core_r + 1.0).ceil().max(0.0) as usize).min(w);
            let (dx, dy) = (z.0 - a.0, z.1 - a.1);
            let l2 = (dx * dx + dy * dy).max(1e-12);
            for x in x0..x1 {
                let px = x as f64 + 0.5;
                let t = (((px - a.0) * dx + (py - a.1) * dy) / l2).clamp(0.0, 1.0);
                let d = ((px - a.0 - dx * t).powi(2) + (py - a.1 - dy * t).powi(2)).sqrt();
                let v = (core_r - d + 0.5).clamp(0.0, 1.0) as f32 * inten;
                if v > row[x] {
                    row[x] = v;
                }
            }
        }
    });
    let glow = if glow_r > 0.5 { gauss_plane(&core, glow_r / 3.0, glow_r / 3.0).map(|v| (v * 3.0).min(1.0)) } else { Plane::new(w, h) };
    b.img.data.par_iter_mut().enumerate().for_each(|(i, px)| {
        let cv = core.data[i] * core_op;
        let gv = glow.data[i] * glow_op;
        let l = [core_c[0] * cv + glow_c[0] * gv, core_c[1] * cv + glow_c[1] * gv, core_c[2] * cv + glow_c[2] * gv];
        let la = cv.max(gv).min(1.0);
        *px = if composite { add_light(*px, l, la) } else { [l[0], l[1], l[2], la] };
    });
    b
}

// ---------------------------------------------------------------- CC Light Rays / Burst

const RAY_STEPS: usize = 32;

fn light_rays(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = b.to_px(ctx.params.v2("center"));
    let k = (ctx.params.f("intensity") / 100.0 * 0.5).clamp(0.0, 0.95);
    let gain = (ctx.params.f("intensity") / 100.0 * 2.0) as f32;
    let rad = (ctx.params.f("radius") * b.scale).max(0.5);
    let ws = (ctx.params.f("warpSoftness") / 50.0).max(0.0);
    let square = ctx.params.e("shape") == 1;
    let from_src = ctx.params.b("colorFromSource");
    let color = ctx.params.color("color");
    let mode = ctx.params.e("transferMode");
    let src = b.img.clone();
    let mask = |x: f64, y: f64| {
        let (dx, dy) = ((x - c.0).abs(), (y - c.1).abs());
        let d = if square { dx.max(dy) } else { (dx * dx + dy * dy).sqrt() };
        1.0 - smoothstep(rad as f32, (rad * (1.0 + ws)) as f32 + 1e-3, d as f32)
    };
    map_xy(&mut b.img, |x, y, px| {
        let (pxx, pyy) = (x as f64 + 0.5, y as f64 + 0.5);
        let mut acc = [0.0f32; 4];
        for i in 0..RAY_STEPS {
            let s = 1.0 - k * i as f64 / RAY_STEPS as f64;
            let (qx, qy) = (c.0 + (pxx - c.0) * s, c.1 + (pyy - c.1) * s);
            let m = mask(qx, qy);
            if m <= 0.0 {
                continue;
            }
            let v = src.sample_bilinear(qx, qy);
            for j in 0..4 {
                acc[j] += v[j] * m;
            }
        }
        let mut ray = acc.map(|v| v / RAY_STEPS as f32 * gain);
        if !from_src {
            let (sc, _) = unpremul(ray);
            let l = luminance(sc[0], sc[1], sc[2]) * ray[3];
            ray = [color[0] * l, color[1] * l, color[2] * l, ray[3]];
        }
        let la = ray[3].min(1.0);
        match mode {
            1 => [px[0].max(ray[0]), px[1].max(ray[1]), px[2].max(ray[2]), px[3].max(la)],
            2 => {
                let s = |a: f32, b: f32| a + b - a * b;
                [s(px[0], ray[0]), s(px[1], ray[1]), s(px[2], ray[2]), s(px[3], la)]
            }
            3 => [ray[0], ray[1], ray[2], la],
            _ => add_light(px, [ray[0], ray[1], ray[2]], la),
        }
    });
    b
}

fn light_burst(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = b.to_px(ctx.params.v2("center"));
    let inten = (ctx.params.f("intensity") / 100.0) as f32;
    let k = (ctx.params.f("rayLength") / 100.0).clamp(0.0, 1.0);
    let mode = ctx.params.e("burst");
    let set_color = ctx.params.b("setColor");
    let color = ctx.params.color("color");
    if k <= 0.0 && (inten - 1.0).abs() < 1e-6 && !set_color {
        return b;
    }
    let src = b.img.clone();
    map_xy(&mut b.img, |x, y, _| {
        let (pxx, pyy) = (x as f64 + 0.5, y as f64 + 0.5);
        let mut acc = [0.0f32; 4];
        let mut wsum = 0.0f32;
        let mut best = [0.0f32; 4];
        let mut best_l = -1.0f32;
        for i in 0..RAY_STEPS {
            let f = i as f64 / RAY_STEPS as f64;
            let s = 1.0 - k * f;
            let v = src.sample_bilinear(c.0 + (pxx - c.0) * s, c.1 + (pyy - c.1) * s);
            match mode {
                0 => {
                    let l = luminance(v[0], v[1], v[2]) + v[3] * 1e-3;
                    if l > best_l {
                        best_l = l;
                        best = v;
                    }
                }
                1 => {
                    let w = 1.0 - f as f32;
                    for j in 0..4 {
                        acc[j] += v[j] * w;
                    }
                    wsum += w;
                }
                _ => {
                    for j in 0..4 {
                        acc[j] += v[j];
                    }
                    wsum += 1.0;
                }
            }
        }
        let mut o = if mode == 0 { best } else { acc.map(|v| v / wsum.max(1e-6)) };
        if set_color {
            let (sc, a) = unpremul(o);
            let l = luminance(sc[0], sc[1], sc[2]) * a;
            o = [color[0] * l, color[1] * l, color[2] * l, a];
        }
        [o[0] * inten, o[1] * inten, o[2] * inten, (o[3] * inten).clamp(0.0, 1.0)]
    });
    b
}

// ---------------------------------------------------------------- CC Light Sweep

fn light_sweep(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = b.to_px(ctx.params.v2("center"));
    let dir = ctx.params.f("direction").to_radians();
    let shape = ctx.params.e("shape");
    let half = (ctx.params.f("width") * b.scale * 0.5).max(0.5);
    let sweep = (ctx.params.f("sweepIntensity") / 100.0) as f32;
    let edge_i = (ctx.params.f("edgeIntensity") / 100.0) as f32;
    let edge_t = ctx.params.f("edgeThickness") * b.scale;
    let lc = ctx.params.color("lightColor");
    let mode = ctx.params.e("lightReceptionMode");
    let (nx, ny) = (dir.cos(), dir.sin());
    let alpha = Plane::alpha(&b.img);
    let soft_a = if edge_t > 0.05 { gauss_plane(&alpha, edge_t * 0.5, edge_t * 0.5) } else { alpha };
    map_xy(&mut b.img, |x, y, px| {
        let d = ((x as f64 + 0.5 - c.0) * nx + (y as f64 + 0.5 - c.1) * ny).abs();
        let band = match shape {
            0 => (1.0 - d / half).max(0.0) as f32,
            2 => (half - d + 0.5).clamp(0.0, 1.0) as f32,
            _ => smoothstep(half as f32, 0.0, d as f32),
        };
        if band <= 0.0 {
            return if mode == 2 { [0.0; 4] } else { px };
        }
        let (xi, yi) = (x as i64, y as i64);
        let gx = soft_a.get_clamped(xi + 1, yi) - soft_a.get_clamped(xi - 1, yi);
        let gy = soft_a.get_clamped(xi, yi + 1) - soft_a.get_clamped(xi, yi - 1);
        let edge = ((gx * gx + gy * gy).sqrt() * 2.0).min(1.0);
        let light = band * sweep + band * edge * edge_i;
        let a = px[3];
        match mode {
            1 => {
                let la = (band * sweep).min(1.0);
                [px[0] + lc[0] * light, px[1] + lc[1] * light, px[2] + lc[2] * light, (a + la * (1.0 - a)).min(1.0)]
            }
            2 => {
                let la = (light * a).min(1.0);
                [lc[0] * la, lc[1] * la, lc[2] * la, la]
            }
            _ => [px[0] + lc[0] * light * a, px[1] + lc[1] * light * a, px[2] + lc[2] * light * a, a],
        }
    });
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let ang = || ParamUi::Angle;
    vec![
        spec(
            "ec.generate.cellpattern",
            "Cell Pattern",
            vec![
                p("cellPattern", "Cell Pattern", Value::Enum(0), popup(&CELL_PATTERNS)),
                p("invert", "Invert", Value::Bool(false), ParamUi::Checkbox),
                p("contrast", "Contrast", num(100.0), slider(0.0, 10000.0, 0.0, 400.0, 1)),
                p("overflow", "Overflow", Value::Enum(0), popup(&["Clip", "Soft Clamp", "Wrap Back"])),
                p("disperse", "Disperse", num(1.0), slider(0.0, 1.5, 0.0, 1.5, 2)),
                p("size", "Size", num(60.0), slider(2.0, 4000.0, 2.0, 400.0, 1)),
                p("offset", "Offset", pt(0.5, 0.5), ParamUi::Point),
                p("tilingOptions/enableTiling", "Enable Tiling", Value::Bool(false), ParamUi::Checkbox),
                p("tilingOptions/cellsHorizontal", "Cells Horizontal", num(16.0), slider(1.0, 1000.0, 1.0, 100.0, 0)),
                p("tilingOptions/cellsVertical", "Cells Vertical", num(16.0), slider(1.0, 1000.0, 1.0, 100.0, 0)),
                p("evolution", "Evolution", num(0.0), ang()),
                p("evolutionOptions/cycleEvolution", "Cycle Evolution", Value::Bool(false), ParamUi::Checkbox),
                p("evolutionOptions/cycle", "Cycle (in Revolutions)", num(1.0), slider(1.0, 1000.0, 1.0, 20.0, 0)),
                p("evolutionOptions/randomSeed", "Random Seed", num(0.0), slider(0.0, 100000.0, 0.0, 1000.0, 0)),
            ],
            cell_pattern,
        ),
        spec(
            "ec.generate.ellipse",
            "Ellipse",
            vec![
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("width", "Width", num(100.0), slider(0.0, 30000.0, 0.0, 1000.0, 1)),
                p("height", "Height", num(100.0), slider(0.0, 30000.0, 0.0, 1000.0, 1)),
                p("thickness", "Thickness", num(10.0), slider(0.0, 3000.0, 0.0, 100.0, 1)),
                p("softness", "Softness", num(0.0), pct()),
                p("insideColor", "Inside Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("outsideColor", "Outside Color", col(0.0, 0.6, 1.0), ParamUi::Color),
                p("compositeOnOriginal", "Composite On Original", Value::Bool(false), ParamUi::Checkbox),
            ],
            ellipse,
        ),
        spec(
            "ec.generate.lensflare",
            "Lens Flare",
            vec![
                p("flareCenter", "Flare Center", pt(0.25, 0.25), ParamUi::Point),
                p("flareBrightness", "Flare Brightness", num(100.0), slider(0.0, 300.0, 10.0, 300.0, 0)),
                p("lensType", "Lens Type", Value::Enum(0), popup(&["50-300mm Zoom", "35mm Prime", "105mm Prime"])),
                p("blendWithOriginal", "Blend With Original", num(0.0), pct()),
            ],
            lens_flare,
        ),
        spec(
            "ec.generate.beam",
            "Beam",
            vec![
                p("startPoint", "Starting Point", pt(0.25, 0.5), ParamUi::Point),
                p("endPoint", "Ending Point", pt(0.75, 0.5), ParamUi::Point),
                p("length", "Length", num(25.0), pct()),
                p("time", "Time", num(0.0), pct()),
                p("startThickness", "Starting Thickness", num(8.0), slider(0.0, 1000.0, 0.0, 100.0, 1)),
                p("endThickness", "Ending Thickness", num(8.0), slider(0.0, 1000.0, 0.0, 100.0, 1)),
                p("softness", "Softness", num(20.0), pct()),
                p("insideColor", "Inside Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("outsideColor", "Outside Color", col(0.4, 0.4, 1.0), ParamUi::Color),
                p("perspective3d", "3D Perspective", Value::Bool(true), ParamUi::Checkbox),
                p("compositeOnOriginal", "Composite On Original", Value::Bool(true), ParamUi::Checkbox),
            ],
            beam,
        ),
        spec(
            "ec.generate.radiowaves",
            "Radio Waves",
            vec![
                p("producerPoint", "Producer Point", pt(0.5, 0.5), ParamUi::Point),
                p("polygon/sides", "Sides", num(64.0), slider(3.0, 64.0, 3.0, 64.0, 0)),
                p("polygon/star", "Star", Value::Bool(false), ParamUi::Checkbox),
                p("polygon/starDepth", "Star Depth", num(-0.3), slider(-1.0, 1.0, -1.0, 1.0, 2)),
                p("waveMotion/frequency", "Frequency", num(1.0), slider(0.01, 100.0, 0.01, 10.0, 2)),
                p("waveMotion/expansion", "Expansion", num(100.0), slider(0.0, 10000.0, 0.0, 1000.0, 1)),
                p("waveMotion/orientation", "Orientation", num(0.0), ang()),
                p("waveMotion/direction", "Direction", num(90.0), ang()),
                p("waveMotion/velocity", "Velocity", num(0.0), slider(0.0, 10000.0, 0.0, 1000.0, 1)),
                p("waveMotion/spin", "Spin", num(0.0), slider(-3600.0, 3600.0, -360.0, 360.0, 1)),
                p("waveMotion/lifespan", "Lifespan (sec)", num(2.0), slider(0.01, 100.0, 0.01, 10.0, 2)),
                p("waveStroke/color", "Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("waveStroke/opacity", "Opacity", num(100.0), pct()),
                p("waveStroke/fadeInTime", "Fade-in Time", num(0.0), slider(0.0, 100.0, 0.0, 5.0, 2)),
                p("waveStroke/fadeOutTime", "Fade-out Time", num(0.0), slider(0.0, 100.0, 0.0, 5.0, 2)),
                p("waveStroke/startWidth", "Start Width", num(5.0), slider(0.0, 1000.0, 0.0, 100.0, 1)),
                p("waveStroke/endWidth", "End Width", num(5.0), slider(0.0, 1000.0, 0.0, 100.0, 1)),
            ],
            radio_waves,
        ),
        spec(
            "ec.generate.advancedlightning",
            "Advanced Lightning",
            vec![
                p("lightningType", "Lightning Type", Value::Enum(1), popup(&LIGHTNING_TYPES)),
                p("origin", "Origin", pt(0.25, 0.2), ParamUi::Point),
                p("direction", "Direction", pt(0.75, 0.8), ParamUi::Point),
                p("conductivityState", "Conductivity State", num(10.0), slider(0.0, 100000.0, 0.0, 100.0, 1)),
                p("coreSettings/coreRadius", "Core Radius", num(3.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
                p("coreSettings/coreOpacity", "Core Opacity", num(75.0), pct()),
                p("coreSettings/coreColor", "Core Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("glowSettings/glowRadius", "Glow Radius", num(50.0), slider(0.0, 1000.0, 0.0, 200.0, 1)),
                p("glowSettings/glowOpacity", "Glow Opacity", num(50.0), pct()),
                p("glowSettings/glowColor", "Glow Color", col(0.25, 0.35, 1.0), ParamUi::Color),
                p("turbulence", "Turbulence", num(1.0), slider(0.0, 10.0, 0.0, 4.0, 2)),
                p("forking", "Forking", num(25.0), pct()),
                p("decay", "Decay", num(0.3), slider(0.0, 1.0, 0.0, 1.0, 2)),
                p("decayMainCore", "Decay Main Core", Value::Bool(false), ParamUi::Checkbox),
                p("compositeOnOriginal", "Composite On Original", Value::Bool(true), ParamUi::Checkbox),
                p("expertSettings/complexity", "Complexity", num(6.0), slider(1.0, 12.0, 1.0, 12.0, 0)),
            ],
            advanced_lightning,
        ),
        spec(
            "ec.generate.cclightrays",
            "CC Light Rays",
            vec![
                p("intensity", "Intensity", num(100.0), slider(0.0, 600.0, 0.0, 200.0, 1)),
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("radius", "Radius", num(40.0), slider(0.0, 4000.0, 0.0, 400.0, 1)),
                p("warpSoftness", "Warp Softness", num(50.0), slider(0.0, 400.0, 0.0, 100.0, 1)),
                p("shape", "Shape", Value::Enum(0), popup(&["Round", "Square"])),
                p("colorFromSource", "Color from Source", Value::Bool(true), ParamUi::Checkbox),
                p("color", "Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("transferMode", "Transfer Mode", Value::Enum(0), popup(&["Add", "Lighten", "Screen", "None"])),
            ],
            light_rays,
        ),
        spec(
            "ec.generate.cclightburst",
            "CC Light Burst 2.5",
            vec![
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("intensity", "Intensity", num(100.0), slider(0.0, 600.0, 0.0, 200.0, 1)),
                p("rayLength", "Ray Length", num(50.0), slider(0.0, 100.0, 0.0, 100.0, 1)),
                p("burst", "Burst", Value::Enum(0), popup(&["Straight", "Fade", "Center"])),
                p("setColor", "Set Color", Value::Bool(false), ParamUi::Checkbox),
                p("color", "Color", col(1.0, 1.0, 1.0), ParamUi::Color),
            ],
            light_burst,
        ),
        spec(
            "ec.generate.cclightsweep",
            "CC Light Sweep",
            vec![
                p("center", "Center", pt(0.5, 0.5), ParamUi::Point),
                p("direction", "Direction", num(-30.0), ang()),
                p("shape", "Shape", Value::Enum(1), popup(&["Linear", "Smooth", "Sharp"])),
                p("width", "Width", num(50.0), slider(0.0, 4000.0, 0.0, 400.0, 1)),
                p("sweepIntensity", "Sweep Intensity", num(50.0), slider(0.0, 500.0, 0.0, 200.0, 1)),
                p("edgeIntensity", "Edge Intensity", num(100.0), slider(0.0, 500.0, 0.0, 200.0, 1)),
                p("edgeThickness", "Edge Thickness", num(1.0), slider(0.0, 100.0, 0.0, 10.0, 1)),
                p("lightColor", "Light Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("lightReceptionMode", "Light Reception", Value::Enum(0), popup(&["Add", "Composite", "Cutout"])),
            ],
            light_sweep,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params;
    use effectcraft_raster::Image;

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
        let ctx = EffectCtx { params: &params, time: 0.5, layer_size: ls, seed: 3, adjustment: false, env: Default::default() };
        crate::apply(s, &ctx, Buf { img, offset: [0.0, 0.0], scale: 1.0 })
    }

    #[test]
    fn cell_pattern_is_deterministic_and_seeded() {
        let a = run("ec.generate.cellpattern", &[("size", num(10.0))], Image::new(40, 30));
        let b2 = run("ec.generate.cellpattern", &[("size", num(10.0))], Image::new(40, 30));
        assert_eq!(a.img, b2.img);
        let c = run("ec.generate.cellpattern", &[("size", num(10.0)), ("evolutionOptions/randomSeed", num(5.0))], Image::new(40, 30));
        assert_ne!(a.img, c.img);
        assert!(a.img.data.iter().all(|p| p[3] == 1.0 && (0.0..=1.0).contains(&p[0])));
    }

    #[test]
    fn ellipse_ring_coverage() {
        let o = run("ec.generate.ellipse", &[("width", num(40.0)), ("height", num(40.0)), ("thickness", num(6.0))], Image::new(64, 64));
        // Ring passes through (32 + 20, 32); centre is empty.
        assert!(o.img.get(51, 32)[3] > 0.9, "{:?}", o.img.get(51, 32));
        assert!(o.img.get(32, 32)[3] < 1e-3);
        assert!(o.img.get(2, 2)[3] < 1e-3);
    }

    #[test]
    fn lightning_is_deterministic_per_conductivity() {
        let a = run("ec.generate.advancedlightning", &[], Image::new(64, 48));
        let b2 = run("ec.generate.advancedlightning", &[], Image::new(64, 48));
        assert_eq!(a.img, b2.img);
        assert!(a.img.data.iter().any(|p| p[3] > 0.5));
        let c = run("ec.generate.advancedlightning", &[("conductivityState", num(11.0))], Image::new(64, 48));
        assert_ne!(a.img, c.img);
    }

    #[test]
    fn beam_draws_between_points() {
        let o = run("ec.generate.beam", &[("length", num(100.0)), ("softness", num(0.0))], Image::new(64, 32));
        assert!(o.img.get(32, 16)[3] > 0.99);
        assert!(o.img.get(32, 2)[3] < 1e-3);
        assert!(o.img.get(2, 16)[3] < 1e-3);
    }

    #[test]
    fn cell_pattern_tiles() {
        // 4×4-cell tiles of 8 px: the pattern repeats every 32 px.
        let set = [
            ("size", num(8.0)),
            ("tilingOptions/enableTiling", Value::Bool(true)),
            ("tilingOptions/cellsHorizontal", num(4.0)),
            ("tilingOptions/cellsVertical", num(4.0)),
        ];
        let o = run("ec.generate.cellpattern", &set, Image::new(64, 64));
        for (x, y) in [(3, 5), (10, 20), (17, 2)] {
            let (a, b2) = (o.img.get(x, y), o.img.get(x + 32, y + 32));
            assert!((a[0] - b2[0]).abs() < 1e-4, "{x},{y}: {a:?} vs {b2:?}");
        }
    }

    #[test]
    fn radio_waves_expansion_velocity_and_star() {
        let r = |set: &[(&str, Value)], t: f64| {
            let s = crate::find("ec.generate.radiowaves").unwrap();
            let mut params = Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
            params.values.insert("producerPoint".into(), Value::Vec2([32.0, 32.0]));
            for (k, v) in set {
                params.values.insert(k.to_string(), v.clone());
            }
            let ctx = EffectCtx { params: &params, time: t, layer_size: [64.0, 64.0], seed: 3, adjustment: false, env: Default::default() };
            crate::apply(s, &ctx, Buf { img: Image::new(64, 64), offset: [0.0, 0.0], scale: 1.0 })
        };
        // One wave born at t = 0, 0.2 s old: radius = Expansion × age = 20 px.
        let base = [("waveMotion/expansion", num(100.0)), ("waveMotion/frequency", num(0.5))];
        let o = r(&base, 0.2);
        assert!(o.img.get(52, 32)[3] > 0.9, "{:?}", o.img.get(52, 32));
        assert!(o.img.get(32, 32)[3] < 0.01);
        // Velocity moves the wave toward Direction (90° = right).
        let mut moving = base.to_vec();
        moving.push(("waveMotion/velocity", num(50.0)));
        let o = r(&moving, 0.2);
        assert!(o.img.get(62, 32)[3] > 0.9 && o.img.get(52, 32)[3] < 0.5, "{:?}", o.img.get(62, 32));
        // A 4-point star: the outline dips toward the centre between the points.
        let mut star = base.to_vec();
        star.extend([("polygon/sides", num(4.0)), ("polygon/star", Value::Bool(true)), ("polygon/starDepth", num(-0.5))]);
        let o = r(&star, 0.2);
        assert!(o.img.get(32, 12)[3] > 0.9, "point straight up");
        // Between points (45°) the outline sits near half the radius.
        assert!(o.img.get(39, 24)[3] > 0.5, "{:?}", o.img.get(39, 24));
        assert!(o.img.get(46, 18)[3] < 0.01, "no circle there");
    }

    #[test]
    fn lightning_types_and_decay_main_core() {
        let a = run("ec.generate.advancedlightning", &[], Image::new(64, 48));
        let strike = run("ec.generate.advancedlightning", &[("lightningType", Value::Enum(1))], Image::new(64, 48));
        assert_eq!(a.img, strike.img, "Strike is the default");
        for t in 0..8 {
            let o = run("ec.generate.advancedlightning", &[("lightningType", Value::Enum(t))], Image::new(64, 48));
            assert!(o.img.data.iter().any(|p| p[3] > 0.3), "type {t} draws");
        }
        let sum = |i: &Image| i.data.iter().map(|p| p[3]).sum::<f32>();
        let d = run("ec.generate.advancedlightning", &[("decayMainCore", Value::Bool(true)), ("decay", num(0.9))], Image::new(64, 48));
        let n = run("ec.generate.advancedlightning", &[("decay", num(0.9))], Image::new(64, 48));
        assert!(sum(&d.img) < sum(&n.img));
    }

    #[test]
    fn beam_3d_perspective_controls_thickness() {
        // Short beam near the start: with 3D Perspective its thickness comes from its position
        // (thin, near the start); without, its head takes the full ending thickness.
        let set = |p: bool| {
            vec![
                ("length", num(20.0)),
                ("time", num(0.0)),
                ("softness", num(0.0)),
                ("startThickness", num(2.0)),
                ("endThickness", num(20.0)),
                ("perspective3d", Value::Bool(p)),
            ]
        };
        let on = run("ec.generate.beam", &set(true), Image::new(64, 32));
        let off = run("ec.generate.beam", &set(false), Image::new(64, 32));
        let sum = |i: &Image| i.data.iter().map(|p| p[3]).sum::<f32>();
        assert!(sum(&off.img) > sum(&on.img) * 1.5, "{} {}", sum(&off.img), sum(&on.img));
    }

    #[test]
    fn lens_flare_brightest_at_center() {
        let o = run("ec.generate.lensflare", &[], Image::new(64, 64));
        let c = o.img.get(16, 16)[0];
        assert!(c > o.img.get(60, 4)[0]);
        assert!(c > 0.5);
    }
}
