//! Simulation effects, part 2: layer-shattering and surface simulations — CC Ball Action,
//! CC Pixel Polly, CC Scatterize, Card Dance, Shatter (planar pieces in perspective), Caustics,
//! Wave World (stepped 2D wave equation) and Foam (stepped bubble sim).
//!
//! Written from the public descriptions of the effects' behaviour. Pieces are textured convex
//! polygons placed in 3D and drawn through a per-piece inverse homography, far to near.

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::{Image, Px};
use rayon::prelude::*;

use crate::generate::value_noise;
use crate::sim::{Acc, Shape, Sprite, h, hs, pct, pt, raster, spec, splat};
use crate::util::{Plane, SimCache, gauss_plane, layer_or_self, params_key, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

// ------------------------------------------------------------------ 3D pieces

type M3 = [[f64; 3]; 3];

fn mat_mul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

/// Rotation from Euler angles (radians), applied X then Y then Z.
fn rot_xyz(x: f64, y: f64, z: f64) -> M3 {
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mat_mul(&rz, &mat_mul(&ry, &rx))
}

/// Rotation by `angle` about unit `axis` (Rodrigues).
fn rot_axis(axis: [f64; 3], angle: f64) -> M3 {
    let l = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if l < 1e-12 || angle == 0.0 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let (x, y, z) = (axis[0] / l, axis[1] / l, axis[2] / l);
    let (s, c) = angle.sin_cos();
    let t = 1.0 - c;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

fn inv3(m: &M3) -> Option<M3> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        [(m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d],
        [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d, (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d],
        [(m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d, (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d, (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d],
    ])
}

/// Perspective camera looking down +z at the layer plane z = 0, `d` pixels away, centred on
/// `c` (buffer pixels): screen = c + (P - c) · d / (d + z).
#[derive(Clone, Copy)]
struct Cam {
    c: [f64; 2],
    d: f64,
}

/// A planar textured piece ready to draw.
#[derive(Clone, Copy)]
struct Piece {
    /// Screen (buffer px, homogeneous) → piece-local (a, b, w).
    inv: M3,
    /// Rest centre of the piece in texture pixels.
    c0: [f32; 2],
    /// Convex polygon (texture pixels), counter-clockwise in y-down space.
    poly: [[f32; 2]; 6],
    n: u8,
    bbox: [f32; 4],
    shade: f32,
    alpha: f32,
    /// Flat straight colour instead of the texture.
    flat: Option<[f32; 3]>,
    /// Facing away from the camera (draw the back texture).
    back: bool,
    depth: f64,
}

impl Piece {
    /// Place polygon `poly` (texture px, around rest centre `c0`) with rotation `r` and its
    /// centre at 3D `pos` (buffer px; z away from the camera).
    fn new(poly: &[[f32; 2]], c0: [f32; 2], r: &M3, pos: [f64; 3], cam: Cam) -> Option<Piece> {
        let n = poly.len().min(6);
        if n < 3 {
            return None;
        }
        let (cx, cy, d) = (cam.c[0], cam.c[1], cam.d);
        let zc = pos[2];
        let hm: M3 = [
            [cx * r[2][0] + d * r[0][0], cx * r[2][1] + d * r[0][1], cx * (d + zc) + d * (pos[0] - cx)],
            [cy * r[2][0] + d * r[1][0], cy * r[2][1] + d * r[1][1], cy * (d + zc) + d * (pos[1] - cy)],
            [r[2][0], r[2][1], d + zc],
        ];
        let mut bb = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        for v in &poly[..n] {
            let (a, b) = ((v[0] - c0[0]) as f64, (v[1] - c0[1]) as f64);
            let w = hm[2][0] * a + hm[2][1] * b + hm[2][2];
            if w < d * 0.05 {
                return None;
            }
            let sx = ((hm[0][0] * a + hm[0][1] * b + hm[0][2]) / w) as f32;
            let sy = ((hm[1][0] * a + hm[1][1] * b + hm[1][2]) / w) as f32;
            bb = [bb[0].min(sx), bb[1].min(sy), bb[2].max(sx), bb[3].max(sy)];
        }
        let inv = inv3(&hm)?;
        let mut pp = [[0.0f32; 2]; 6];
        pp[..n].copy_from_slice(&poly[..n]);
        // Normalise winding so "inside" is cross >= 0 for every edge.
        let area: f32 = (0..n).map(|i| pp[i][0] * pp[(i + 1) % n][1] - pp[(i + 1) % n][0] * pp[i][1]).sum();
        if area < 0.0 {
            pp[..n].reverse();
        }
        // Front normal (0, 0, -1) rotated; facing the camera when its z is negative.
        let nz = -r[2][2];
        Some(Piece {
            inv,
            c0,
            poly: pp,
            n: n as u8,
            bbox: [bb[0] - 1.0, bb[1] - 1.0, bb[2] + 1.0, bb[3] + 1.0],
            shade: 1.0,
            alpha: 1.0,
            flat: None,
            back: nz > 0.0,
            depth: zc,
        })
    }

    /// Lambert factor of the piece normal against a light shining along +z.
    fn facing(r: &M3) -> f32 {
        r[2][2].abs() as f32
    }
}

/// Draw pieces (already sorted far → near) over `out`.
fn draw_pieces(out: &mut Image, pieces: &[Piece], front: &Image, back: Option<&Image>) {
    raster(
        out,
        pieces,
        |q| Some(q.bbox),
        |q, x, y| {
            let (xf, yf) = (x as f64, y as f64);
            let m = &q.inv;
            let w = m[2][0] * xf + m[2][1] * yf + m[2][2];
            if w.abs() < 1e-12 {
                return None;
            }
            let u = ((m[0][0] * xf + m[0][1] * yf + m[0][2]) / w) as f32 + q.c0[0];
            let v = ((m[1][0] * xf + m[1][1] * yf + m[1][2]) / w) as f32 + q.c0[1];
            let n = q.n as usize;
            let mut md = f32::INFINITY;
            for i in 0..n {
                let a = q.poly[i];
                let b = q.poly[(i + 1) % n];
                let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
                let l = (ex * ex + ey * ey).sqrt().max(1e-6);
                md = md.min((ex * (v - a[1]) - ey * (u - a[0])) / l);
            }
            // Coverage extends half a pixel past the edge so abutting pieces leave no seams.
            let cov = (md + 1.0).clamp(0.0, 1.0);
            if cov <= 0.0 {
                return None;
            }
            let (c, a) = match q.flat {
                Some(c) => (c, 1.0),
                None => {
                    let tex = if q.back { back.unwrap_or(front) } else { front };
                    unpremul(tex.sample_bilinear(u as f64, v as f64))
                }
            };
            let a = a * cov * q.alpha;
            if a <= 1e-6 {
                return None;
            }
            let k = q.shade;
            Some([c[0] * k * a, c[1] * k * a, c[2] * k * a, a])
        },
        Acc::Over,
    );
}

fn sort_far_first(pieces: &mut [Piece]) {
    pieces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
}

// ------------------------------------------------------------------ CC Ball Action

fn ball_action(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let scatter = pr.f("scatter") as f32;
    let axis = pr.e("rotationAxis");
    let rot = pr.f("rotation").to_radians();
    let twist_prop = pr.e("twistProperty");
    let twist = pr.f("twistAngle").to_radians();
    let spacing = pr.f("gridSpacing").max(1.0) as f32;
    let size = pr.f("ballSize") as f32 / 100.0;
    let instab = pr.f("instabilityState").to_radians() as f32;
    let seed = ctx.seed.wrapping_mul(0x2545f491);
    let (lw, lh) = (ctx.layer_size[0] as f32, ctx.layer_size[1] as f32);
    let s = b.scale as f32;
    let nx = (lw / spacing).ceil().max(1.0) as u32;
    let ny = (lh / spacing).ceil().max(1.0) as u32;
    let d = 2.0 * lw.max(lh) as f64;
    let (cx, cy) = (lw as f64 * 0.5, lh as f64 * 0.5);
    let src = b.img.clone();
    let ax: [f64; 3] = match axis {
        0 => [1.0, 0.0, 0.0],
        1 => [0.0, 1.0, 0.0],
        2 => [0.0, 0.0, 1.0],
        3 => [1.0, 1.0, 0.0],
        4 => [1.0, 0.0, 1.0],
        5 => [0.0, 1.0, 1.0],
        _ => [1.0, 1.0, 1.0],
    };
    let maxr = ((lw * lw + lh * lh).sqrt() * 0.5).max(1.0);
    let mut balls: Vec<(f64, Sprite)> = (0..nx * ny)
        .into_par_iter()
        .filter_map(|i| {
            let gx = (i % nx) as f32 * spacing + spacing * 0.5;
            let gy = (i / nx) as f32 * spacing + spacing * 0.5;
            let (sx, sy) = b.to_px([gx as f64, gy as f64]);
            let (c, a) = unpremul(src.sample_bilinear(sx, sy));
            if a <= 0.0 || size <= 0.0 {
                return None;
            }
            let tp = match twist_prop {
                0 => gx / lw.max(1.0),
                1 => gy / lh.max(1.0),
                2 => ((gx - cx as f32).hypot(gy - cy as f32)) / maxr,
                _ => crate::util::rgb3([c[0], c[1], c[2], 0.0]).iter().sum::<f32>() / 3.0,
            } as f64;
            let ang = rot + twist * tp;
            // Scatter: random 3D offsets whose direction drifts with the instability state.
            let ph = h(i, 1, seed) * std::f32::consts::TAU + instab;
            let sc = scatter * h(i, 2, seed);
            let off = [(sc * ph.cos()) as f64, (sc * ph.sin()) as f64, (scatter * hs(i, 3, seed)) as f64];
            let p0 = [gx as f64 - cx + off[0], gy as f64 - cy + off[1], off[2]];
            let m = rot_axis(ax, ang);
            let q: [f64; 3] = std::array::from_fn(|k| m[k][0] * p0[0] + m[k][1] * p0[1] + m[k][2] * p0[2]);
            if d + q[2] <= d * 0.05 {
                return None;
            }
            let persp = d / (d + q[2]);
            let (bx, by) = b.to_px([cx + q[0] * persp, cy + q[1] * persp]);
            let r = spacing * 0.5 * size * persp as f32 * s;
            Some((q[2], Sprite::new(bx as f32, by as f32, r, [c[0], c[1], c[2], a], Shape::Sphere)))
        })
        .collect();
    balls.sort_by(|a, b| b.0.total_cmp(&a.0));
    let sprites: Vec<Sprite> = balls.into_iter().map(|(_, s)| s).collect();
    b.img = splat(src.width, src.height, &sprites, Acc::Over);
    b
}

// ------------------------------------------------------------------ CC Pixel Polly

fn pixel_polly(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let t = ctx.time - pr.f("startTime");
    if t <= 0.0 {
        return b;
    }
    let force = pr.f("force");
    let gravity = pr.f("gravity");
    let spin = pr.f("spinning").to_radians();
    let fc = pr.v2("forceCenter");
    let dir_rand = pr.f("directionRandomness") / 100.0;
    let spd_rand = pr.f("speedRandomness") / 100.0;
    let g = pr.f("gridSpacing").max(1.0);
    let object = pr.e("object");
    let depth_sort = pr.b("enableDepthSort");
    let seed = ctx.seed.wrapping_mul(0x632be5ab);
    let [lw, lh] = ctx.layer_size;
    let s = b.scale;
    let cam = Cam { c: [b.offset[0] + lw * 0.5 * s, b.offset[1] + lh * 0.5 * s], d: 2.0 * lw.max(lh) * s };
    let nx = (lw / g).ceil().max(1.0) as u32;
    let ny = (lh / g).ceil().max(1.0) as u32;
    let tri = object <= 1;
    let textured = object == 1 || object == 3;
    let maxd = lw.max(lh);
    let src = b.img.clone();
    let per = if tri { 2 } else { 1 };
    let mut pieces: Vec<Piece> = (0..nx * ny * per)
        .into_par_iter()
        .filter_map(|i| {
            let cell = i / per;
            let (x0, y0) = ((cell % nx) as f64 * g, (cell / nx) as f64 * g);
            let poly: Vec<[f64; 2]> = if !tri {
                vec![[x0, y0], [x0 + g, y0], [x0 + g, y0 + g], [x0, y0 + g]]
            } else if i % 2 == 0 {
                vec![[x0, y0], [x0 + g, y0], [x0, y0 + g]]
            } else {
                vec![[x0 + g, y0], [x0 + g, y0 + g], [x0, y0 + g]]
            };
            let c = [poly.iter().map(|v| v[0]).sum::<f64>() / poly.len() as f64, poly.iter().map(|v| v[1]).sum::<f64>() / poly.len() as f64];
            let (dx, dy) = (c[0] - fc[0], c[1] - fc[1]);
            let dist = (dx * dx + dy * dy).sqrt().max(1e-6);
            let ang = dy.atan2(dx) + dir_rand * std::f64::consts::PI * hs(i, 1, seed) as f64;
            let speed = force / 100.0 * 0.6 * maxd / (1.0 + dist / (0.5 * maxd)) * (1.0 + spd_rand * hs(i, 2, seed) as f64);
            let vz = -speed * 0.5 * h(i, 3, seed) as f64;
            let pos = [c[0] + ang.cos() * speed * t, c[1] + ang.sin() * speed * t + 0.5 * gravity * 0.5 * lh * t * t, vz * t];
            let axis = [hs(i, 4, seed) as f64, hs(i, 5, seed) as f64, hs(i, 6, seed) as f64];
            let r = rot_axis(axis, t * (std::f64::consts::PI * h(i, 7, seed) as f64 + spin));
            let tex: Vec<[f32; 2]> = poly.iter().map(|v| [(v[0] * s + b.offset[0]) as f32, (v[1] * s + b.offset[1]) as f32]).collect();
            let c0 = [(c[0] * s + b.offset[0]) as f32, (c[1] * s + b.offset[1]) as f32];
            let posb = [pos[0] * s + b.offset[0], pos[1] * s + b.offset[1], pos[2] * s];
            let mut pc = Piece::new(&tex, c0, &r, posb, cam)?;
            pc.shade = 0.35 + 0.65 * Piece::facing(&r);
            if !textured {
                let (cc, a) = unpremul(src.sample_bilinear(c0[0] as f64, c0[1] as f64));
                if a <= 0.0 {
                    return None;
                }
                pc.flat = Some(cc);
                pc.alpha = a;
            }
            Some(pc)
        })
        .collect();
    if depth_sort {
        sort_far_first(&mut pieces);
    }
    let mut out = Image::new(src.width, src.height);
    draw_pieces(&mut out, &pieces, &src, None);
    b.img = out;
    b
}

// ------------------------------------------------------------------ CC Scatterize

fn scatterize(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let scatter = pr.f("scatter") as f32;
    let right = pr.f("rightTwist").to_radians();
    let left = pr.f("leftTwist").to_radians();
    if scatter == 0.0 && right == 0.0 && left == 0.0 {
        return b;
    }
    let add = pr.e("transferMode") == 1;
    let seed = ctx.seed.wrapping_mul(0x7feb352d);
    let s = b.scale;
    let (w, hh) = (b.img.width, b.img.height);
    let (cx, cy) = (b.offset[0] + ctx.layer_size[0] * 0.5 * s, b.offset[1] + ctx.layer_size[1] * 0.5 * s);
    let half = (ctx.layer_size[0] * 0.5 * s).max(1.0);
    let d = 2.0 * ctx.layer_size[0].max(ctx.layer_size[1]) * s;
    let src = &b.img;
    let sprites: Vec<Sprite> = (0..w * hh)
        .into_par_iter()
        .filter_map(|i| {
            let px = src.data[i as usize];
            if px[3] <= 0.0 {
                return None;
            }
            let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            // Twist about the horizontal axis through the centre, growing towards each side.
            let k = ((x - cx) / half).abs().min(1.0);
            let ang = if x < cx { left } else { right } * k;
            let (sa, ca) = ang.sin_cos();
            let (yy, zz) = ((y - cy) * ca, (y - cy) * sa);
            let persp = d / (d + zz).max(d * 0.05);
            let jx = hs(i, 1, seed) * scatter * s as f32;
            let jy = hs(i, 2, seed) * scatter * s as f32;
            let sx = cx + (x - cx) * persp + jx as f64;
            let sy = cy + yy * persp + jy as f64;
            let (c, a) = unpremul(px);
            Some(Sprite::new(sx as f32, sy as f32, 0.6 * persp as f32, [c[0], c[1], c[2], a], Shape::Disc))
        })
        .collect();
    b.img = splat(w, hh, &sprites, if add { Acc::Add } else { Acc::Over });
    b
}

// ------------------------------------------------------------------ Card Dance

const CD_SOURCES: &[&str] = &["None", "Intensity 1", "Red 1", "Green 1", "Blue 1", "Alpha 1", "Intensity 2", "Red 2", "Green 2", "Blue 2", "Alpha 2"];
const CD_PROPS: [(&str, &str); 8] = [
    ("xPos", "X Position"),
    ("yPos", "Y Position"),
    ("zPos", "Z Position"),
    ("xRot", "X Rotation"),
    ("yRot", "Y Rotation"),
    ("zRot", "Z Rotation"),
    ("xScale", "X Scale"),
    ("yScale", "Y Scale"),
];

fn card_source(src: u32, g1: [f32; 4], g2: [f32; 4]) -> f32 {
    let (c1, a1) = unpremul(g1);
    let (c2, a2) = unpremul(g2);
    let lum = |c: [f32; 3]| effectcraft_color::luminance(c[0], c[1], c[2]);
    match src {
        1 => lum(c1),
        2 => c1[0],
        3 => c1[1],
        4 => c1[2],
        5 => a1,
        6 => lum(c2),
        7 => c2[0],
        8 => c2[1],
        9 => c2[2],
        10 => a2,
        _ => 0.0,
    }
}

fn card_dance(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let rows = pr.f("rows").clamp(1.0, 1000.0) as u32;
    let cols = if pr.e("rowsColumns") == 1 { rows } else { pr.f("columns").clamp(1.0, 1000.0) as u32 };
    let g1 = layer_or_self(ctx, &b, "gradientLayer1", true, true);
    let g2 = layer_or_self(ctx, &b, "gradientLayer2", true, true);
    let back = ctx.layer_param("backLayer", true).map(|o| crate::util::fit_layer(ctx, &b, &o, true));
    let [lw, lh] = ctx.layer_size;
    let s = b.scale;
    let (cw, ch) = (lw / cols as f64, lh / rows as f64);
    let zpos = pr.f("cameraZ").max(0.05);
    let focal = pr.f("focalLength").max(1.0);
    let cam = Cam { c: [b.offset[0] + lw * 0.5 * s, b.offset[1] + lh * 0.5 * s], d: lw.max(lh) * s * zpos * focal / 140.0 };
    let ambient = pr.f("ambientLight") as f32;
    let diffuse = pr.f("diffuse") as f32;
    let intensity = pr.f("lightIntensity") as f32;
    let props: Vec<(u32, f64, f64)> =
        CD_PROPS.iter().map(|(id, _)| (pr.e(&format!("{id}Source")), pr.f(&format!("{id}Multiplier")), pr.f(&format!("{id}Offset")))).collect();
    let src = b.img.clone();
    let mut pieces: Vec<Piece> = (0..rows * cols)
        .into_par_iter()
        .filter_map(|i| {
            let (cx0, cy0) = ((i % cols) as f64 * cw, (i / cols) as f64 * ch);
            let c = [cx0 + cw * 0.5, cy0 + ch * 0.5];
            let (bx, by) = (c[0] * s + b.offset[0], c[1] * s + b.offset[1]);
            let (gp1, gp2) = (g1.sample_bilinear(bx, by), g2.sample_bilinear(bx, by));
            let val = |k: usize| {
                let (sr, m, o) = props[k];
                o + m * card_source(sr, gp1, gp2) as f64
            };
            let (sx, sy) = (val(6), val(7));
            let r = rot_xyz(val(3).to_radians(), val(4).to_radians(), val(5).to_radians());
            let rs: M3 = std::array::from_fn(|k| [r[k][0] * sx, r[k][1] * sy, r[k][2]]);
            let pos = [bx + val(0) * cw * s, by + val(1) * ch * s, val(2) * cw.max(ch) * s];
            let (tx0, ty0) = ((cx0 * s + b.offset[0]) as f32, (cy0 * s + b.offset[1]) as f32);
            let (tw, th) = ((cw * s) as f32, (ch * s) as f32);
            let poly = [[tx0, ty0], [tx0 + tw, ty0], [tx0 + tw, ty0 + th], [tx0, ty0 + th]];
            let mut pc = Piece::new(&poly, [bx as f32, by as f32], &rs, pos, cam)?;
            pc.shade = ambient + diffuse * intensity * Piece::facing(&r);
            Some(pc)
        })
        .collect();
    sort_far_first(&mut pieces);
    let mut out = Image::new(src.width, src.height);
    draw_pieces(&mut out, &pieces, &src, back.as_ref());
    b.img = out;
    b
}

// ------------------------------------------------------------------ Shatter

const SHATTER_PATTERNS: &[&str] = &["Bricks", "Glass", "Hexagons", "Squares", "Triangles"];

/// Pattern cells (polygons in pattern space, before rotation) covering `[x0, x1]×[y0, y1]`.
fn pattern_cells(kind: u32, cell: f64, x0: f64, y0: f64, x1: f64, y1: f64, seed: u32) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    let i0 = (x0 / cell).floor() as i64 - 1;
    let i1 = (x1 / cell).ceil() as i64 + 1;
    let j0 = (y0 / cell).floor() as i64 - 1;
    let j1 = (y1 / cell).ceil() as i64 + 1;
    match kind {
        0 => {
            let bh = cell * 0.5;
            let r0 = (y0 / bh).floor() as i64 - 1;
            let r1 = (y1 / bh).ceil() as i64 + 1;
            for r in r0..=r1 {
                let sh = if r.rem_euclid(2) == 1 { cell * 0.5 } else { 0.0 };
                for i in i0..=i1 {
                    let (x, y) = (i as f64 * cell + sh, r as f64 * bh);
                    out.push(vec![[x, y], [x + cell, y], [x + cell, y + bh], [x, y + bh]]);
                }
            }
        }
        1 => {
            // Jittered lattice split into triangles along a random diagonal.
            let jit = |i: i64, j: i64| -> [f64; 2] {
                let (a, b) = (i as u32, j as u32);
                [i as f64 * cell + hs(a, b.wrapping_add(17), seed) as f64 * cell * 0.35, j as f64 * cell + hs(a, b.wrapping_add(91), seed) as f64 * cell * 0.35]
            };
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (p00, p10, p11, p01) = (jit(i, j), jit(i + 1, j), jit(i + 1, j + 1), jit(i, j + 1));
                    if h(i as u32, j as u32, seed ^ 0x55) < 0.5 {
                        out.push(vec![p00, p10, p11]);
                        out.push(vec![p00, p11, p01]);
                    } else {
                        out.push(vec![p00, p10, p01]);
                        out.push(vec![p10, p11, p01]);
                    }
                }
            }
        }
        2 => {
            let r = cell / 3f64.sqrt();
            let dx = 1.5 * r;
            let dy = 3f64.sqrt() * r;
            let c0 = (x0 / dx).floor() as i64 - 1;
            let c1 = (x1 / dx).ceil() as i64 + 1;
            let rr0 = (y0 / dy).floor() as i64 - 1;
            let rr1 = (y1 / dy).ceil() as i64 + 1;
            for ci in c0..=c1 {
                for rj in rr0..=rr1 {
                    let cx = ci as f64 * dx;
                    let cy = (rj as f64 + if ci.rem_euclid(2) == 1 { 0.5 } else { 0.0 }) * dy;
                    out.push(
                        (0..6)
                            .map(|k| {
                                let a = k as f64 * std::f64::consts::PI / 3.0;
                                [cx + r * a.cos(), cy + r * a.sin()]
                            })
                            .collect(),
                    );
                }
            }
        }
        3 => {
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (x, y) = (i as f64 * cell, j as f64 * cell);
                    out.push(vec![[x, y], [x + cell, y], [x + cell, y + cell], [x, y + cell]]);
                }
            }
        }
        _ => {
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (x, y) = (i as f64 * cell, j as f64 * cell);
                    if (i + j).rem_euclid(2) == 0 {
                        out.push(vec![[x, y], [x + cell, y], [x, y + cell]]);
                        out.push(vec![[x + cell, y], [x + cell, y + cell], [x, y + cell]]);
                    } else {
                        out.push(vec![[x, y], [x + cell, y], [x + cell, y + cell]]);
                        out.push(vec![[x, y], [x + cell, y + cell], [x, y + cell]]);
                    }
                }
            }
        }
    }
    out
}

/// `∫0^t e^{-ks} ds` and `∫0^t ∫0^u e^{-ks} ds du` (drag-damped ballistic factors).
fn drag_factors(k: f64, t: f64) -> (f64, f64) {
    if k < 1e-6 {
        (t, 0.5 * t * t)
    } else {
        let e = (-k * t).exp();
        ((1.0 - e) / k, t / k - (1.0 - e) / (k * k))
    }
}

fn shatter(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let view = pr.e("view");
    let render = pr.e("render");
    let kind = pr.e("pattern");
    let reps = pr.f("repetitions").clamp(1.0, 500.0);
    let dir = pr.f("direction").to_radians();
    let origin = pr.v2("origin");
    let forces = [
        (pr.v2("force1Position"), pr.f("force1Depth"), pr.f("force1Radius"), pr.f("force1Strength")),
        (pr.v2("force2Position"), pr.f("force2Depth"), pr.f("force2Radius"), pr.f("force2Strength")),
    ];
    let rot_speed = pr.f("rotationSpeed");
    let tumble = pr.e("tumbleAxis");
    let randomness = pr.f("randomness");
    let k = pr.f("viscosity").max(0.0) * 5.0;
    let mass_var = pr.f("massVariance") / 100.0;
    let gravity = pr.f("gravity");
    let gdir = pr.f("gravityDirection").to_radians();
    let ginc = pr.f("gravityInclination").to_radians();
    let ambient = pr.f("ambientLight") as f32;
    let intensity = pr.f("lightIntensity") as f32;
    let seed = (pr.f("randomSeed") as u32).wrapping_mul(0x9e3779b1) ^ ctx.seed;
    let [lw, lh] = ctx.layer_size;
    let s = b.scale;
    let t = ctx.time.max(0.0);
    let unit = lw * 0.1;
    let cell = lw / reps;
    let cam = Cam { c: [b.offset[0] + lw * 0.5 * s, b.offset[1] + lh * 0.5 * s], d: 2.0 * lw.max(lh) * s };
    let (sd, cd) = dir.sin_cos();
    let to_layer = |q: [f64; 2]| [origin[0] + q[0] * cd - q[1] * sd, origin[1] + q[0] * sd + q[1] * cd];
    // Pattern-space bounds of the layer rectangle.
    let corners = [[0.0, 0.0], [lw, 0.0], [lw, lh], [0.0, lh]].map(|c: [f64; 2]| {
        let (dx, dy) = (c[0] - origin[0], c[1] - origin[1]);
        [dx * cd + dy * sd, -dx * sd + dy * cd]
    });
    let (px0, px1) = (corners.iter().map(|c| c[0]).fold(f64::INFINITY, f64::min), corners.iter().map(|c| c[0]).fold(f64::NEG_INFINITY, f64::max));
    let (py0, py1) = (corners.iter().map(|c| c[1]).fold(f64::INFINITY, f64::min), corners.iter().map(|c| c[1]).fold(f64::NEG_INFINITY, f64::max));
    let cells = pattern_cells(kind, cell, px0, py0, px1, py1, seed);
    let g3 = [gdir.sin() * ginc.cos() * gravity * unit, -gdir.cos() * ginc.cos() * gravity * unit, ginc.sin() * gravity * unit];
    let src = b.img.clone();
    let mut pieces: Vec<Piece> = cells
        .par_iter()
        .enumerate()
        .filter_map(|(i, poly)| {
            let i = i as u32;
            let lp: Vec<[f64; 2]> = poly.iter().map(|&q| to_layer(q)).collect();
            let (mnx, mxx) = (lp.iter().map(|v| v[0]).fold(f64::INFINITY, f64::min), lp.iter().map(|v| v[0]).fold(f64::NEG_INFINITY, f64::max));
            let (mny, mxy) = (lp.iter().map(|v| v[1]).fold(f64::INFINITY, f64::min), lp.iter().map(|v| v[1]).fold(f64::NEG_INFINITY, f64::max));
            if mxx < 0.0 || mxy < 0.0 || mnx > lw || mny > lh {
                return None;
            }
            let c = [lp.iter().map(|v| v[0]).sum::<f64>() / lp.len() as f64, lp.iter().map(|v| v[1]).sum::<f64>() / lp.len() as f64];
            // Broken when inside a force sphere (centred `depth` in front of the layer).
            let mut v = [0.0f64; 3];
            let mut broken = false;
            for (fp, fd, fr, fs) in forces {
                let rad = fr * lw;
                if rad <= 0.0 {
                    continue;
                }
                let dv = [c[0] - fp[0], c[1] - fp[1], fd * lw];
                let dist = (dv[0] * dv[0] + dv[1] * dv[1] + dv[2] * dv[2]).sqrt();
                if dist < rad {
                    broken = true;
                    let f = fs * (1.0 - dist / rad) * unit / dist.max(1e-6);
                    for q in 0..3 {
                        v[q] += dv[q] * f;
                    }
                }
            }
            if (broken && render == 1) || (!broken && render == 2) {
                return None;
            }
            let mut pos = [c[0], c[1], 0.0];
            let mut r = rot_axis([1.0, 0.0, 0.0], 0.0);
            if broken && t > 0.0 {
                let mass = 1.0 + mass_var * hs(i, 1, seed) as f64;
                for q in 0..3 {
                    v[q] = v[q] / mass + randomness * unit * hs(i, 2 + q as u32, seed) as f64 * 2.0;
                }
                let (f1, f2) = drag_factors(k, t);
                for q in 0..3 {
                    pos[q] += v[q] * f1 + g3[q] * f2;
                }
                let axis = match tumble {
                    1 => [0.0, 0.0, 0.0],
                    2 => [1.0, 0.0, 0.0],
                    3 => [0.0, 1.0, 0.0],
                    4 => [0.0, 0.0, 1.0],
                    5 => [hs(i, 6, seed) as f64, hs(i, 7, seed) as f64, 0.0],
                    6 => [hs(i, 6, seed) as f64, 0.0, hs(i, 8, seed) as f64],
                    7 => [0.0, hs(i, 7, seed) as f64, hs(i, 8, seed) as f64],
                    _ => [hs(i, 6, seed) as f64, hs(i, 7, seed) as f64, hs(i, 8, seed) as f64],
                };
                let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() / unit;
                let omega = rot_speed * std::f64::consts::TAU * (0.5 + h(i, 9, seed) as f64) * speed.min(10.0);
                r = rot_axis(axis, omega * f1);
            }
            let tex: Vec<[f32; 2]> = lp.iter().map(|v| [(v[0] * s + b.offset[0]) as f32, (v[1] * s + b.offset[1]) as f32]).collect();
            let c0 = [(c[0] * s + b.offset[0]) as f32, (c[1] * s + b.offset[1]) as f32];
            let posb = [pos[0] * s + b.offset[0], pos[1] * s + b.offset[1], pos[2] * s];
            let mut pc = Piece::new(&tex, c0, &r, posb, cam)?;
            pc.shade = ambient + (1.0 - ambient) * intensity * Piece::facing(&r);
            Some(pc)
        })
        .collect();
    sort_far_first(&mut pieces);
    let mut out = Image::new(src.width, src.height);
    if view == 0 {
        draw_pieces(&mut out, &pieces, &src, None);
    } else {
        // Wireframes: piece outlines (front view: at rest; otherwise animated), plus forces.
        let animated = view == 2 || view == 4;
        let mut lines = Vec::new();
        for pc in &pieces {
            let n = pc.n as usize;
            let pts: Vec<[f32; 2]> = if animated {
                let fw = inv3(&pc.inv);
                match fw {
                    Some(m) => pc.poly[..n]
                        .iter()
                        .map(|v| {
                            let (a, bb) = ((v[0] - pc.c0[0]) as f64, (v[1] - pc.c0[1]) as f64);
                            let w = m[2][0] * a + m[2][1] * bb + m[2][2];
                            [((m[0][0] * a + m[0][1] * bb + m[0][2]) / w) as f32, ((m[1][0] * a + m[1][1] * bb + m[1][2]) / w) as f32]
                        })
                        .collect(),
                    None => continue,
                }
            } else {
                pc.poly[..n].to_vec()
            };
            for e in 0..n {
                let (a, c) = (pts[e], pts[(e + 1) % n]);
                lines.push(Sprite::new(a[0], a[1], 0.5, [0.75, 0.85, 1.0, 1.0], Shape::Line { dx: c[0] - a[0], dy: c[1] - a[1] }));
            }
        }
        if view >= 3 {
            for (fp, _, fr, _) in forces {
                let (fx, fy) = b.to_px(fp);
                let rad = fr * lw * s;
                for k in 0..64 {
                    let (a0, a1) = (k as f64 / 64.0 * std::f64::consts::TAU, (k + 1) as f64 / 64.0 * std::f64::consts::TAU);
                    let (x0, y0) = (fx + rad * a0.cos(), fy + rad * a0.sin());
                    let (x1, y1) = (fx + rad * a1.cos(), fy + rad * a1.sin());
                    lines.push(Sprite::new(x0 as f32, y0 as f32, 0.5, [1.0, 0.3, 0.3, 1.0], Shape::Line { dx: (x1 - x0) as f32, dy: (y1 - y0) as f32 }));
                }
            }
        }
        out = splat(src.width, src.height, &lines, Acc::Over);
    }
    b.img = out;
    b
}

// ------------------------------------------------------------------ Caustics

/// Sample with repeat mode: 0 Once (transparent outside), 1 Tiled, 2 Reflected.
fn sample_repeat(img: &Image, x: f64, y: f64, mode: u32) -> Px {
    let (w, hh) = (img.width as f64, img.height as f64);
    match mode {
        1 => img.sample_bilinear_clamped(x.rem_euclid(w), y.rem_euclid(hh)),
        2 => {
            let m = |v: f64, n: f64| {
                let r = v.rem_euclid(2.0 * n);
                if r > n { 2.0 * n - r } else { r }
            };
            img.sample_bilinear_clamped(m(x, w), m(y, hh))
        }
        _ => img.sample_bilinear(x, y),
    }
}

fn caustics(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let s = b.scale;
    let mut bottom = layer_or_self(ctx, &b, "bottom", true, pr.e("bottomSizeDiffers") == 1);
    let blur = pr.f("blur") * s;
    if blur > 0.05 {
        bottom = effectcraft_raster::gaussian_blur(&bottom, blur * 0.5, blur * 0.5, false);
    }
    let scaling = pr.f("scaling").max(0.01);
    let repeat = pr.e("repeatMode");
    let wave_h = pr.f("waveHeight");
    let smoothing = pr.f("smoothing") * s;
    let depth = pr.f("waterDepth");
    let ior = pr.f("refractiveIndex").max(1.0);
    let surf = pr.color("surfaceColor");
    let surf_op = pr.f("surfaceOpacity") as f32;
    let cstr = pr.f("causticsStrength") as f32;
    let li = pr.f("lightIntensity") as f32;
    let lc = pr.color("lightColor");
    let lpos = pr.v2("lightPosition");
    let lheight = pr.f("lightHeight").max(0.01);
    let ambient = pr.f("ambientLight") as f32;
    let diffuse = pr.f("diffuse") as f32;
    let specular = pr.f("specular") as f32;
    let sharp = pr.f("highlightSharpness").max(1.0) as f32;
    let (w, hh) = (b.img.width as usize, b.img.height as usize);
    // Height field of the water surface (luminance of the chosen layer; none = flat).
    let height = ctx.layer_param("waterSurface", true).map(|o| {
        let img = crate::util::fit_layer(ctx, &b, &o, true);
        let pl = Plane::luma(&img);
        if smoothing > 0.05 { gauss_plane(&pl, smoothing * 0.5, smoothing * 0.5) } else { pl }
    });
    let lw = ctx.layer_size[0] * s;
    let k = wave_h * depth * (1.0 - 1.0 / ior) * lw * 2.0;
    let (cx, cy) = (b.offset[0] + ctx.layer_size[0] * 0.5 * s, b.offset[1] + ctx.layer_size[1] * 0.5 * s);
    let (lx, ly) = b.to_px(lpos);
    let lv = {
        let v = [(lx - cx) / lw.max(1.0), (ly - cy) / lw.max(1.0), lheight];
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [(v[0] / n) as f32, (v[1] / n) as f32, (v[2] / n) as f32]
    };
    let grad = |x: usize, y: usize| -> (f64, f64) {
        match &height {
            Some(pl) => {
                let gx = (pl.get_clamped(x as i64 + 1, y as i64) - pl.get_clamped(x as i64 - 1, y as i64)) as f64 * 0.5;
                let gy = (pl.get_clamped(x as i64, y as i64 + 1) - pl.get_clamped(x as i64, y as i64 - 1)) as f64 * 0.5;
                (gx, gy)
            }
            None => (0.0, 0.0),
        }
    };
    // Displacement and its Jacobian (caustic focusing) per pixel.
    let disp: Vec<(f64, f64)> = (0..w * hh)
        .into_par_iter()
        .map(|i| {
            let (gx, gy) = grad(i % w, i / w);
            (-gx * k, -gy * k)
        })
        .collect();
    b.img.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let (dx, dy) = disp[i];
            let d = |xx: usize, yy: usize| disp[yy.min(hh - 1) * w + xx.min(w - 1)];
            let (dxr, _) = d(x + 1, y);
            let (_, dyd) = d(x, y + 1);
            let (dxl, _) = d(x.saturating_sub(1), y);
            let (_, dyu) = d(x, y.saturating_sub(1));
            let jac = (1.0 + (dxr - dxl) * 0.5) * (1.0 + (dyd - dyu) * 0.5);
            let focus = ((1.0 / jac.abs().max(0.2)) as f32 - 1.0).clamp(-1.0, 4.0);
            let sx = cx + (x as f64 + 0.5 + dx - cx) / scaling;
            let sy = cy + (y as f64 + 0.5 + dy - cy) / scaling;
            let p = sample_repeat(&bottom, sx, sy, repeat);
            let (c, a) = unpremul(p);
            let (gx, gy) = grad(x, y);
            let n = {
                let v = [-gx * wave_h * 20.0, -gy * wave_h * 20.0, 1.0];
                let m = (v[0] * v[0] + v[1] * v[1] + 1.0).sqrt();
                [(v[0] / m) as f32, (v[1] / m) as f32, (v[2] / m) as f32]
            };
            let nl = (n[0] * lv[0] + n[1] * lv[1] + n[2] * lv[2]).max(0.0);
            let hv = {
                let v = [lv[0], lv[1], lv[2] + 1.0];
                let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                [v[0] / m, v[1] / m, v[2] / m]
            };
            let spec = (n[0] * hv[0] + n[1] * hv[1] + n[2] * hv[2]).max(0.0).powf(sharp) * specular * li;
            let light = ambient + diffuse * li * nl + cstr * focus * li;
            let cc: [f32; 3] = std::array::from_fn(|q| {
                let base = c[q] * light * lc[q];
                base * (1.0 - surf_op) + surf[q] * surf_op * (ambient + diffuse * li * nl) + spec * lc[q]
            });
            let a = (a + surf_op * (1.0 - a)).min(1.0);
            *px = [cc[0] * a, cc[1] * a, cc[2] * a, a];
        }
    });
    b
}

// ------------------------------------------------------------------ Wave World

#[derive(Clone, Debug)]
struct Waves {
    nx: usize,
    ny: usize,
    u: Vec<f32>,
    v: Vec<f32>,
}

struct Producer {
    ring: bool,
    pos: [f32; 2],
    len: f32,
    width: f32,
    angle: f32,
    amp: f32,
    freq: f32,
    phase: f32,
}

impl Producer {
    /// Weight of a grid point (layer-fraction coordinates, x in widths, y in widths).
    fn weight(&self, x: f32, y: f32) -> f32 {
        let (dx, dy) = (x - self.pos[0], y - self.pos[1]);
        let (sa, ca) = self.angle.sin_cos();
        let (u, v) = (dx * ca + dy * sa, -dx * sa + dy * ca);
        let d = if self.ring {
            let (a, bb) = ((self.len * 0.5).max(1e-3), (self.width * 0.5).max(1e-3));
            ((u / a).powi(2) + (v / bb).powi(2)).sqrt()
        } else {
            let l = (self.len * 0.5).max(1e-3);
            let wd = (self.width * 0.5).max(1e-3);
            (u.abs() / l).max(v.abs() / wd)
        };
        (1.0 - d).clamp(0.0, 1.0).min(1.0)
    }
}

static WAVE_CACHE: SimCache<Waves> = SimCache::new(4);

fn wave_world(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let [lw, lh] = ctx.layer_size;
    let res = pr.f("gridResolution").clamp(1.0, 400.0) as usize;
    let res = if pr.b("gridResDownsamples") { ((res as f64) * b.scale).round().max(1.0) as usize } else { res };
    let nx = res.max(2) + 1;
    let ny = ((res as f64 * lh / lw.max(1.0)).round() as usize).max(2) + 1;
    let speed = pr.f("waveSpeed").max(0.0) as f32;
    let damping = pr.f("damping").max(0.0) as f32;
    let reflect = pr.e("reflectEdges");
    let preroll = pr.f("preRoll").max(0.0);
    let prods: Vec<Producer> = [1, 2]
        .iter()
        .map(|k| {
            let pos = pr.v2(&format!("producer{k}Position"));
            Producer {
                ring: pr.e(&format!("producer{k}Type")) == 0,
                pos: [(pos[0] / lw.max(1.0)) as f32, (pos[1] / lw.max(1.0)) as f32],
                len: pr.f(&format!("producer{k}Length")) as f32,
                width: pr.f(&format!("producer{k}Width")) as f32,
                angle: (pr.f(&format!("producer{k}Angle")) as f32).to_radians(),
                amp: pr.f(&format!("producer{k}Amplitude")) as f32,
                freq: pr.f(&format!("producer{k}Frequency")) as f32,
                phase: (pr.f(&format!("producer{k}Phase")) as f32).to_radians(),
            }
        })
        .collect();
    let sps = crate::sim::SPS as f32;
    // Wave speed in layer widths per second → cells per second; sub-steps keep it stable.
    let cps = speed * (nx - 1) as f32;
    let sub = ((cps * 2.0 / sps).ceil() as usize).clamp(1, 64);
    let dt = 1.0 / (sps * sub as f32);
    let cell = 1.0 / (nx - 1) as f32;
    let weights: Vec<Vec<f32>> = prods.iter().map(|p| (0..nx * ny).map(|i| p.weight((i % nx) as f32 * cell, (i / nx) as f32 * cell)).collect()).collect();
    let key = params_key(ctx, &Buf { img: Image::new(0, 0), offset: [0.0; 2], scale: if pr.b("gridResDownsamples") { b.scale } else { 1.0 } }, 4);
    let steps = crate::sim::steps_at(ctx.time + preroll);
    let st = WAVE_CACHE.run(
        key,
        steps,
        || Waves { nx, ny, u: vec![0.0; nx * ny], v: vec![0.0; nx * ny] },
        |w, step| {
            for k in 0..sub {
                let t = (step as f32 + k as f32 / sub as f32) / sps - preroll as f32;
                let c2 = cps * cps;
                let mut acc = vec![0.0f32; nx * ny];
                for y in 0..ny {
                    for x in 0..nx {
                        let i = y * nx + x;
                        let g = |xx: usize, yy: usize| w.u[yy * nx + xx];
                        let l = g(x.saturating_sub(1), y) + g((x + 1).min(nx - 1), y) + g(x, y.saturating_sub(1)) + g(x, (y + 1).min(ny - 1)) - 4.0 * w.u[i];
                        acc[i] = c2 * l - damping * 4.0 * w.v[i];
                    }
                }
                for i in 0..nx * ny {
                    w.v[i] += acc[i] * dt;
                    w.u[i] += w.v[i] * dt;
                }
                // Producers drive the surface.
                for (p, wt) in prods.iter().zip(&weights) {
                    if p.amp == 0.0 {
                        continue;
                    }
                    let target = p.amp * (std::f32::consts::TAU * p.freq * t + p.phase).sin();
                    for i in 0..nx * ny {
                        if wt[i] > 0.0 {
                            w.u[i] += (target - w.u[i]) * wt[i];
                            w.v[i] *= 1.0 - wt[i];
                        }
                    }
                }
                // Edges: reflective ones mirror; the others absorb in a thin sponge layer.
                let refl = |side: u32| reflect == 5 || reflect == side;
                let sponge = 4usize.min(nx / 4).max(1);
                for y in 0..ny {
                    for x in 0..nx {
                        let i = y * nx + x;
                        let mut f = 1.0f32;
                        if !refl(1) && x < sponge {
                            f = f.min(x as f32 / sponge as f32);
                        }
                        if !refl(3) && nx - 1 - x < sponge {
                            f = f.min((nx - 1 - x) as f32 / sponge as f32);
                        }
                        if !refl(2) && y < sponge {
                            f = f.min(y as f32 / sponge as f32);
                        }
                        if !refl(4) && ny - 1 - y < sponge {
                            f = f.min((ny - 1 - y) as f32 / sponge as f32);
                        }
                        if f < 1.0 {
                            let k = 0.85 + 0.15 * f;
                            w.u[i] *= k;
                            w.v[i] *= k;
                        }
                    }
                }
            }
        },
    );
    let s = b.scale;
    let (bw, bh) = (b.img.width, b.img.height);
    let sample = |x: f64, y: f64| -> f32 {
        let gx = (x / lw.max(1.0) * (st.nx - 1) as f64).clamp(0.0, (st.nx - 1) as f64);
        let gy = (y / lw.max(1.0) * (st.nx - 1) as f64).clamp(0.0, (st.ny - 1) as f64);
        let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(st.nx - 1), (y0 + 1).min(st.ny - 1));
        let (tx, ty) = ((gx - x0 as f64) as f32, (gy - y0 as f64) as f32);
        let g = |xx: usize, yy: usize| st.u[yy * st.nx + xx];
        let a = g(x0, y0) + (g(x1, y0) - g(x0, y0)) * tx;
        let c = g(x0, y1) + (g(x1, y1) - g(x0, y1)) * tx;
        a + (c - a) * ty
    };
    if pr.e("view") == 1 {
        let bright = pr.f("brightness") as f32;
        let contrast = pr.f("contrast") as f32;
        let gamma = pr.f("gamma").max(0.01) as f32;
        let alpha = 1.0 - pr.f("transparency") as f32;
        b.img.rows_mut().for_each(|(y, row)| {
            for (x, px) in row.iter_mut().enumerate() {
                let lx = (x as f64 + 0.5 - b.offset[0]) / s;
                let ly = (y as f64 + 0.5 - b.offset[1]) / s;
                let v = (bright + contrast * sample(lx, ly)).clamp(0.0, 1.0).powf(1.0 / gamma);
                *px = [v * alpha, v * alpha, v * alpha, alpha];
            }
        });
    } else {
        // Wireframe preview: the grid seen at an angle, heights lifting the lines.
        let lift = lh as f32 * 0.25;
        let to = |x: usize, y: usize| -> [f32; 2] {
            let lx = x as f64 / (st.nx - 1) as f64 * lw;
            let ly = y as f64 / (st.nx - 1) as f64 * lw;
            let (px, py) = b.to_px([lx, lh * 0.25 + ly * 0.6]);
            [px as f32, py as f32 - st.u[y * st.nx + x] * lift * s as f32]
        };
        let mut lines = Vec::new();
        for y in 0..st.ny {
            for x in 0..st.nx {
                let a = to(x, y);
                if x + 1 < st.nx {
                    let c = to(x + 1, y);
                    lines.push(Sprite::new(a[0], a[1], 0.4, [0.4, 0.9, 0.4, 1.0], Shape::Line { dx: c[0] - a[0], dy: c[1] - a[1] }));
                }
                if y + 1 < st.ny {
                    let c = to(x, y + 1);
                    lines.push(Sprite::new(a[0], a[1], 0.4, [0.4, 0.9, 0.4, 1.0], Shape::Line { dx: c[0] - a[0], dy: c[1] - a[1] }));
                }
            }
        }
        let wire = splat(bw, bh, &lines, Acc::Over);
        b.img = crate::sim::combine(&Image::filled(bw, bh, [0.0, 0.0, 0.0, 1.0]), &wire, 0);
    }
    b
}

// ------------------------------------------------------------------ Foam

#[derive(Clone, Debug)]
struct FoamBubble {
    p: [f32; 2],
    v: [f32; 2],
    age: f32,
    size: f32,
    id: u32,
}

#[derive(Clone, Debug, Default)]
struct FoamState {
    b: Vec<FoamBubble>,
    carry: f32,
    next: u32,
}

static FOAM_CACHE: SimCache<FoamState> = SimCache::new(4);

/// Foam steps once per frame at 30 frames per second.
const FOAM_FPS: f64 = 30.0;

fn foam(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let (lw, lh) = (ctx.layer_size[0] as f32, ctx.layer_size[1] as f32);
    let unit = lw * 0.01;
    let prod = pr.v2("producerPoint");
    let (psx, psy) = (pr.f("producerXSize") as f32 * lw, pr.f("producerYSize") as f32 * lh);
    let porient = (pr.f("producerOrientation") as f32).to_radians();
    let rate = pr.f("productionRate").max(0.0) as f32;
    let size = pr.f("size").max(0.0) as f32;
    let size_var = pr.f("sizeVariance") as f32;
    let lifespan = pr.f("lifespan").max(1.0) as f32;
    let growth = pr.f("growthSpeed").max(0.0001) as f32;
    let init_speed = pr.f("initialSpeed") as f32;
    let init_dir = (pr.f("initialDirection") as f32).to_radians();
    let wind = pr.f("windSpeed") as f32;
    let wind_dir = (pr.f("windDirection") as f32).to_radians();
    let turb = pr.f("turbulence") as f32;
    let repulsion = pr.f("repulsion") as f32;
    let viscosity = pr.f("viscosity").clamp(0.0, 1.0) as f32;
    let sticky = pr.f("stickiness").clamp(0.0, 1.0) as f32;
    let universe = pr.f("universeSize").max(0.01) as f32;
    let seed = (pr.f("randomSeed") as u32).wrapping_mul(0x68e31da4) ^ ctx.seed;
    let wvec = [wind_dir.sin() * wind * unit, -wind_dir.cos() * wind * unit];
    let key = params_key(ctx, &Buf { img: Image::new(0, 0), offset: [0.0; 2], scale: 1.0 }, 5);
    let steps = if ctx.time <= 0.0 { 0 } else { (ctx.time * FOAM_FPS + 1e-6).floor() as u64 };
    let st = FOAM_CACHE.run(key, steps, FoamState::default, |st, step| {
        let t = step as f32 / FOAM_FPS as f32;
        // Births.
        st.carry += rate;
        let n = st.carry.floor() as u32;
        st.carry -= n as f32;
        for _ in 0..n {
            if st.b.len() >= 3000 {
                break;
            }
            let id = st.next;
            st.next = st.next.wrapping_add(1);
            let (ux, uy) = (hs(id, 1, seed) * 0.5 * psx, hs(id, 2, seed) * 0.5 * psy);
            let (so, co) = porient.sin_cos();
            let p = [prod[0] as f32 + ux * co - uy * so, prod[1] as f32 + ux * so + uy * co];
            let v = [init_dir.sin() * init_speed * unit, -init_dir.cos() * init_speed * unit];
            let sz = (size * (1.0 + size_var * hs(id, 3, seed))).max(0.01);
            st.b.push(FoamBubble { p, v, age: 0.0, size: sz, id });
        }
        // Forces: drift towards the wind, turbulence, viscosity.
        for q in st.b.iter_mut() {
            let n1 = value_noise(q.p[0] / (lw * 0.15), q.p[1] / (lw * 0.15), t * 0.5, seed) - 0.5;
            let n2 = value_noise(q.p[0] / (lw * 0.15) + 19.0, q.p[1] / (lw * 0.15), t * 0.5, seed) - 0.5;
            for k in 0..2 {
                q.v[k] += (wvec[k] - q.v[k]) * 0.1;
                q.v[k] *= 1.0 - viscosity * 0.5;
            }
            q.v[0] += n1 * turb * unit;
            q.v[1] += n2 * turb * unit;
        }
        // Repulsion between overlapping bubbles (grid hashed, deterministic order).
        let rad = |q: &FoamBubble| q.size * unit * 3.0 * (q.age * growth).min(1.0);
        if repulsion > 0.0 && st.b.len() > 1 {
            let maxr = st.b.iter().map(rad).fold(0.0f32, f32::max).max(1.0);
            let cs = maxr * 2.0;
            let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
            for (i, q) in st.b.iter().enumerate() {
                grid.entry(((q.p[0] / cs).floor() as i32, (q.p[1] / cs).floor() as i32)).or_default().push(i);
            }
            let mut push = vec![[0.0f32; 2]; st.b.len()];
            for (i, q) in st.b.iter().enumerate() {
                let (gx, gy) = ((q.p[0] / cs).floor() as i32, (q.p[1] / cs).floor() as i32);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let Some(list) = grid.get(&(gx + dx, gy + dy)) else { continue };
                        for &j in list {
                            if j <= i {
                                continue;
                            }
                            let o = &st.b[j];
                            let (ddx, ddy) = (o.p[0] - q.p[0], o.p[1] - q.p[1]);
                            let d = (ddx * ddx + ddy * ddy).sqrt().max(1e-4);
                            let overlap = rad(q) + rad(o) - d;
                            if overlap > 0.0 {
                                let f = overlap * 0.5 * repulsion.min(4.0) * (1.0 - sticky * 0.5) / d;
                                push[i][0] -= ddx * f;
                                push[i][1] -= ddy * f;
                                push[j][0] += ddx * f;
                                push[j][1] += ddy * f;
                            }
                        }
                    }
                }
            }
            for (q, d) in st.b.iter_mut().zip(&push) {
                q.p[0] += d[0].clamp(-maxr, maxr);
                q.p[1] += d[1].clamp(-maxr, maxr);
            }
        }
        let (ux0, uy0) = (-(universe - 1.0) * 0.5 * lw, -(universe - 1.0) * 0.5 * lh);
        let (ux1, uy1) = (lw - ux0, lh - uy0);
        for q in st.b.iter_mut() {
            q.p[0] += q.v[0];
            q.p[1] += q.v[1];
            q.age += 1.0;
        }
        st.b.retain(|q| q.age < lifespan && q.p[0] > ux0 && q.p[0] < ux1 && q.p[1] > uy0 && q.p[1] < uy1);
    });
    let view = pr.e("view");
    let zoom = pr.f("zoom").max(0.01) as f32;
    let texture = pr.e("bubbleTexture");
    let blend = pr.e("blendMode");
    let s = b.scale as f32;
    let (cx, cy) = (lw * 0.5, lh * 0.5);
    let mut order: Vec<&FoamBubble> = st.b.iter().collect();
    if blend == 2 {
        // Solid New on Top: oldest first.
        order.sort_by(|a, c| c.age.total_cmp(&a.age).then(a.id.cmp(&c.id)));
    } else if blend == 1 {
        order.sort_by(|a, c| a.age.total_cmp(&c.age).then(a.id.cmp(&c.id)));
    }
    let sprites: Vec<Sprite> = order
        .iter()
        .map(|q| {
            let r = q.size * unit * 3.0 * (q.age * growth).min(1.0) * zoom * s;
            let (bx, by) = b.to_px([(cx + (q.p[0] - cx) * zoom) as f64, (cy + (q.p[1] - cy) * zoom) as f64]);
            let fade = ((lifespan - q.age) / 5.0).clamp(0.0, 1.0);
            if view == 0 {
                Sprite::new(bx as f32, by as f32, r, [0.55, 0.75, 1.0, fade], Shape::Bubble)
            } else {
                let (c, shape) = match texture {
                    1 => ([0.95, 0.95, 0.92], Shape::Faded),
                    2 => ([1.0, 0.6, 0.8], Shape::Bubble),
                    3 => ([0.8, 0.85, 0.75], Shape::Bubble),
                    4 => ([0.97, 0.97, 0.95], Shape::Sphere),
                    _ => ([1.0, 1.0, 1.0], Shape::Bubble),
                };
                Sprite::new(bx as f32, by as f32, r, [c[0], c[1], c[2], fade], shape)
            }
        })
        .collect();
    b.img = splat(b.img.width, b.img.height, &sprites, if blend == 0 && view != 0 { Acc::Add } else { Acc::Over });
    b
}

// ------------------------------------------------------------------ specs

fn producer_params(k: u32, amp: f64, pos: (f64, f64)) -> Vec<crate::ParamSpec> {
    let ids: [&'static str; 8] = match k {
        1 => [
            "producer1Type",
            "producer1Position",
            "producer1Length",
            "producer1Width",
            "producer1Angle",
            "producer1Amplitude",
            "producer1Frequency",
            "producer1Phase",
        ],
        _ => [
            "producer2Type",
            "producer2Position",
            "producer2Length",
            "producer2Width",
            "producer2Angle",
            "producer2Amplitude",
            "producer2Frequency",
            "producer2Phase",
        ],
    };
    vec![
        p(ids[0], "Type", Value::Enum(0), popup(&["Ring", "Line"])),
        p(ids[1], "Position", pt(pos.0, pos.1), ParamUi::Point),
        p(ids[2], "Height/Length", num(0.1), slider(0.0, 2.0, 0.0, 1.0, 3)),
        p(ids[3], "Width", num(0.1), slider(0.0, 2.0, 0.0, 1.0, 3)),
        p(ids[4], "Angle", num(0.0), ParamUi::Angle),
        p(ids[5], "Amplitude", num(amp), slider(-5.0, 5.0, -1.0, 1.0, 3)),
        p(ids[6], "Frequency", num(1.0), slider(0.0, 30.0, 0.0, 5.0, 3)),
        p(ids[7], "Phase", num(0.0), ParamUi::Angle),
    ]
}

fn card_dance_params() -> Vec<crate::ParamSpec> {
    let mut v = vec![
        p("rowsColumns", "Rows & Columns", Value::Enum(0), popup(&["Independent", "Columns Follow Rows"])),
        p("rows", "Rows", num(10.0), slider(1.0, 1000.0, 1.0, 100.0, 0)),
        p("columns", "Columns", num(10.0), slider(1.0, 1000.0, 1.0, 100.0, 0)),
        p("backLayer", "Back Layer", Value::Layer(None), ParamUi::Layer),
        p("gradientLayer1", "Gradient Layer 1", Value::Layer(None), ParamUi::Layer),
        p("gradientLayer2", "Gradient Layer 2", Value::Layer(None), ParamUi::Layer),
    ];
    let names: [[&'static str; 4]; 8] = [
        ["xPosSource", "xPosMultiplier", "xPosOffset", "X Position"],
        ["yPosSource", "yPosMultiplier", "yPosOffset", "Y Position"],
        ["zPosSource", "zPosMultiplier", "zPosOffset", "Z Position"],
        ["xRotSource", "xRotMultiplier", "xRotOffset", "X Rotation"],
        ["yRotSource", "yRotMultiplier", "yRotOffset", "Y Rotation"],
        ["zRotSource", "zRotMultiplier", "zRotOffset", "Z Rotation"],
        ["xScaleSource", "xScaleMultiplier", "xScaleOffset", "X Scale"],
        ["yScaleSource", "yScaleMultiplier", "yScaleOffset", "Y Scale"],
    ];
    debug_assert_eq!(names.len(), CD_PROPS.len());
    for (i, n) in names.iter().enumerate() {
        let rot = (3..6).contains(&i);
        let scale = i >= 6;
        let (lo, hi) = if rot { (-3600.0, 3600.0) } else { (-100.0, 100.0) };
        v.push(p(n[0], "Source", Value::Enum(0), popup(CD_SOURCES)));
        v.push(p(n[1], "Multiplier", num(0.0), slider(lo, hi, if rot { -360.0 } else { -10.0 }, if rot { 360.0 } else { 10.0 }, 2)));
        v.push(p(n[2], "Offset", num(if scale { 1.0 } else { 0.0 }), slider(lo, hi, if rot { -360.0 } else { -10.0 }, if rot { 360.0 } else { 10.0 }, 2)));
    }
    v.extend([
        p("cameraZ", "Z Position", num(2.0), slider(0.05, 10.0, 0.5, 5.0, 2)),
        p("focalLength", "Focal Length", num(70.0), slider(1.0, 500.0, 10.0, 200.0, 1)),
        p("lightIntensity", "Light Intensity", num(1.0), slider(0.0, 4.0, 0.0, 2.0, 2)),
        p("ambientLight", "Ambient Light", num(0.25), slider(0.0, 2.0, 0.0, 1.0, 2)),
        p("diffuse", "Diffuse Reflection", num(0.75), slider(0.0, 2.0, 0.0, 1.0, 2)),
    ]);
    v
}

pub fn specs() -> Vec<EffectSpec> {
    vec![
        spec(
            "ec.sim.ccballaction",
            "CC Ball Action",
            vec![
                p("scatter", "Scatter", num(0.0), slider(0.0, 2000.0, 0.0, 300.0, 1)),
                p("rotationAxis", "Rotation Axis", Value::Enum(0), popup(&["X Axis", "Y Axis", "Z Axis", "XY Axis", "XZ Axis", "YZ Axis", "XYZ Axis"])),
                p("rotation", "Rotation", num(0.0), ParamUi::Angle),
                p("twistProperty", "Twist Property", Value::Enum(0), popup(&["X Axis", "Y Axis", "Radius", "Brightness"])),
                p("twistAngle", "Twist Angle", num(0.0), ParamUi::Angle),
                p("gridSpacing", "Grid Spacing", num(4.0), slider(1.0, 200.0, 1.0, 50.0, 0)),
                p("ballSize", "Ball Size", num(40.0), slider(0.0, 400.0, 0.0, 200.0, 1)),
                p("instabilityState", "Instability State", num(0.0), ParamUi::Angle),
            ],
            ball_action,
        ),
        spec(
            "ec.sim.ccpixelpolly",
            "CC Pixel Polly",
            vec![
                p("force", "Force", num(100.0), slider(-1000.0, 1000.0, -300.0, 300.0, 1)),
                p("gravity", "Gravity", num(1.0), slider(-20.0, 20.0, -5.0, 5.0, 2)),
                p("spinning", "Spinning", num(0.0), ParamUi::Angle),
                p("forceCenter", "Force Center", pt(0.5, 0.5), ParamUi::Point),
                p("directionRandomness", "Direction Randomness", num(10.0), pct()),
                p("speedRandomness", "Speed Randomness", num(10.0), pct()),
                p("gridSpacing", "Grid Spacing", num(10.0), slider(1.0, 500.0, 2.0, 100.0, 0)),
                p("object", "Object", Value::Enum(0), popup(&["Polygon", "Textured Polygon", "Square", "Textured Square"])),
                p("enableDepthSort", "Enable Depth Sort", Value::Bool(true), ParamUi::Checkbox),
                p("startTime", "Start Time (sec)", num(0.0), slider(-1000.0, 1000.0, 0.0, 10.0, 2)),
            ],
            pixel_polly,
        ),
        spec(
            "ec.sim.ccscatterize",
            "CC Scatterize",
            vec![
                p("scatter", "Scatter", num(0.0), slider(0.0, 1000.0, 0.0, 200.0, 1)),
                p("rightTwist", "Right Twist", num(0.0), ParamUi::Angle),
                p("leftTwist", "Left Twist", num(0.0), ParamUi::Angle),
                p("transferMode", "Transfer Mode", Value::Enum(0), popup(&["Composite", "Add"])),
            ],
            scatterize,
        ),
        spec("ec.sim.carddance", "Card Dance", card_dance_params(), card_dance),
        spec(
            "ec.sim.caustics",
            "Caustics",
            vec![
                p("bottom", "Bottom", Value::Layer(None), ParamUi::Layer),
                p("scaling", "Scaling", num(1.0), slider(0.01, 10.0, 0.1, 3.0, 3)),
                p("repeatMode", "Repeat Mode", Value::Enum(2), popup(&["Once", "Tiled", "Reflected"])),
                p("bottomSizeDiffers", "If Layer Size Differs", Value::Enum(1), popup(&["Center", "Stretch to Fit"])),
                p("blur", "Blur", num(0.0), slider(0.0, 100.0, 0.0, 20.0, 1)),
                p("waterSurface", "Water Surface", Value::Layer(None), ParamUi::Layer),
                p("waveHeight", "Wave Height", num(0.2), slider(-1.0, 1.0, -1.0, 1.0, 3)),
                p("smoothing", "Smoothing", num(5.0), slider(0.0, 100.0, 0.0, 50.0, 1)),
                p("waterDepth", "Water Depth", num(0.1), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("refractiveIndex", "Refractive Index", num(1.2), slider(1.0, 3.0, 1.0, 2.0, 3)),
                p("surfaceColor", "Surface Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("surfaceOpacity", "Surface Opacity", num(0.3), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("causticsStrength", "Caustics Strength", num(0.0), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("lightIntensity", "Light Intensity", num(1.0), slider(0.0, 4.0, 0.0, 2.0, 2)),
                p("lightColor", "Light Color", col(1.0, 1.0, 1.0), ParamUi::Color),
                p("lightPosition", "Light Position", pt(0.0, 0.0), ParamUi::Point),
                p("lightHeight", "Light Height", num(1.0), slider(0.0, 10.0, 0.0, 4.0, 2)),
                p("ambientLight", "Ambient Light", num(0.35), slider(0.0, 2.0, 0.0, 1.0, 2)),
                p("diffuse", "Diffuse Reflection", num(0.75), slider(0.0, 2.0, 0.0, 1.0, 2)),
                p("specular", "Specular Reflection", num(0.2), slider(0.0, 2.0, 0.0, 1.0, 2)),
                p("highlightSharpness", "Highlight Sharpness", num(15.0), slider(1.0, 100.0, 1.0, 100.0, 1)),
            ],
            caustics,
        ),
        spec(
            "ec.sim.waveworld",
            "Wave World",
            {
                let mut v = vec![
                    p("view", "View", Value::Enum(0), popup(&["Wireframe Preview", "Height Map"])),
                    p("brightness", "Brightness", num(0.5), slider(-1.0, 2.0, 0.0, 1.0, 3)),
                    p("contrast", "Contrast", num(0.75), slider(0.0, 4.0, 0.0, 2.0, 3)),
                    p("gamma", "Gamma Adjustment", num(1.0), slider(0.01, 10.0, 0.2, 5.0, 3)),
                    p("transparency", "Transparency", num(0.0), slider(0.0, 1.0, 0.0, 1.0, 3)),
                    p("gridResolution", "Grid Resolution", num(60.0), slider(1.0, 400.0, 1.0, 200.0, 0)),
                    p("gridResDownsamples", "Grid Res Downsamples", Value::Bool(false), ParamUi::Checkbox),
                    p("waveSpeed", "Wave Speed", num(0.2), slider(0.0, 5.0, 0.0, 1.0, 3)),
                    p("damping", "Damping", num(0.05), slider(0.0, 5.0, 0.0, 1.0, 3)),
                    p("reflectEdges", "Reflect Edges", Value::Enum(0), popup(&["None", "Left", "Top", "Right", "Bottom", "All"])),
                    p("preRoll", "Pre-roll (seconds)", num(0.0), slider(0.0, 30.0, 0.0, 10.0, 2)),
                ];
                v.extend(producer_params(1, 0.5, (0.5, 0.5)));
                v.extend(producer_params(2, 0.0, (0.25, 0.25)));
                v
            },
            wave_world,
        ),
        spec(
            "ec.sim.shatter",
            "Shatter",
            vec![
                p(
                    "view",
                    "View",
                    Value::Enum(1),
                    popup(&["Rendered", "Wireframe Front View", "Wireframe", "Wireframe Front View + Forces", "Wireframe + Forces"]),
                ),
                p("render", "Render", Value::Enum(0), popup(&["All", "Layer", "Pieces"])),
                p("pattern", "Pattern", Value::Enum(0), popup(SHATTER_PATTERNS)),
                p("repetitions", "Repetitions", num(10.0), slider(1.0, 500.0, 1.0, 100.0, 2)),
                p("direction", "Direction", num(0.0), ParamUi::Angle),
                p("origin", "Origin", pt(0.5, 0.5), ParamUi::Point),
                p("extrusionDepth", "Extrusion Depth", num(0.05), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("force1Position", "Force 1 Position", pt(0.5, 0.5), ParamUi::Point),
                p("force1Depth", "Force 1 Depth", num(0.1), slider(-1.0, 1.0, -1.0, 1.0, 3)),
                p("force1Radius", "Force 1 Radius", num(0.4), slider(0.0, 4.0, 0.0, 2.0, 3)),
                p("force1Strength", "Force 1 Strength", num(5.0), slider(-20.0, 20.0, -10.0, 10.0, 2)),
                p("force2Position", "Force 2 Position", pt(0.25, 0.25), ParamUi::Point),
                p("force2Depth", "Force 2 Depth", num(0.1), slider(-1.0, 1.0, -1.0, 1.0, 3)),
                p("force2Radius", "Force 2 Radius", num(0.0), slider(0.0, 4.0, 0.0, 2.0, 3)),
                p("force2Strength", "Force 2 Strength", num(5.0), slider(-20.0, 20.0, -10.0, 10.0, 2)),
                p("rotationSpeed", "Rotation Speed", num(0.2), slider(0.0, 5.0, 0.0, 1.0, 3)),
                p("tumbleAxis", "Tumble Axis", Value::Enum(0), popup(&["Free", "None", "X", "Y", "Z", "XY", "XZ", "YZ"])),
                p("randomness", "Randomness", num(0.1), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("viscosity", "Viscosity", num(0.1), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("massVariance", "Mass Variance", num(30.0), pct()),
                p("gravity", "Gravity", num(3.0), slider(0.0, 100.0, 0.0, 10.0, 2)),
                p("gravityDirection", "Gravity Direction", num(180.0), ParamUi::Angle),
                p("gravityInclination", "Gravity Inclination", num(0.0), slider(-90.0, 90.0, -90.0, 90.0, 1)),
                p("lightIntensity", "Light Intensity", num(1.0), slider(0.0, 4.0, 0.0, 2.0, 2)),
                p("ambientLight", "Ambient Light", num(0.25), slider(0.0, 2.0, 0.0, 1.0, 2)),
                p("randomSeed", "Random Seed", num(0.0), slider(0.0, 10_000.0, 0.0, 1000.0, 0)),
            ],
            shatter,
        ),
        spec(
            "ec.sim.foam",
            "Foam",
            vec![
                p("view", "View", Value::Enum(0), popup(&["Draft", "Rendered"])),
                p("producerPoint", "Producer Point", pt(0.5, 0.5), ParamUi::Point),
                p("producerXSize", "Producer X Size", num(0.05), slider(0.0, 2.0, 0.0, 1.0, 3)),
                p("producerYSize", "Producer Y Size", num(0.05), slider(0.0, 2.0, 0.0, 1.0, 3)),
                p("producerOrientation", "Producer Orientation", num(0.0), ParamUi::Angle),
                p("productionRate", "Production Rate", num(1.0), slider(0.0, 50.0, 0.0, 10.0, 3)),
                p("size", "Size", num(0.5), slider(0.0, 10.0, 0.0, 2.0, 3)),
                p("sizeVariance", "Size Variance", num(0.5), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("lifespan", "Lifespan", num(300.0), slider(1.0, 30_000.0, 1.0, 1000.0, 1)),
                p("growthSpeed", "Bubble Growth Speed", num(0.1), slider(0.0001, 10.0, 0.0, 1.0, 3)),
                p("strength", "Strength", num(10.0), slider(0.0, 100.0, 0.0, 20.0, 2)),
                p("initialSpeed", "Initial Speed", num(0.0), slider(-100.0, 100.0, -10.0, 10.0, 3)),
                p("initialDirection", "Initial Direction", num(0.0), ParamUi::Angle),
                p("windSpeed", "Wind Speed", num(0.5), slider(-100.0, 100.0, -10.0, 10.0, 3)),
                p("windDirection", "Wind Direction", num(90.0), ParamUi::Angle),
                p("turbulence", "Turbulence", num(0.5), slider(0.0, 10.0, 0.0, 2.0, 3)),
                p("repulsion", "Repulsion", num(1.0), slider(0.0, 10.0, 0.0, 2.0, 3)),
                p("viscosity", "Viscosity", num(0.1), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("stickiness", "Stickiness", num(0.75), slider(0.0, 1.0, 0.0, 1.0, 3)),
                p("zoom", "Zoom", num(1.0), slider(0.01, 50.0, 0.1, 5.0, 3)),
                p("universeSize", "Universe Size", num(1.0), slider(0.01, 10.0, 0.5, 3.0, 3)),
                p("blendMode", "Blend Mode", Value::Enum(0), popup(&["Transparent", "Solid Old on Top", "Solid New on Top"])),
                p("bubbleTexture", "Bubble Texture", Value::Enum(0), popup(&["Default Bubble", "Spit", "Bubblegum", "Dishwater", "Milky"])),
                p("randomSeed", "Random Seed", num(1.0), slider(0.0, 10_000.0, 0.0, 1000.0, 0)),
            ],
            foam,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, run_fx};

    fn ramp(w: u32, hh: u32) -> Image {
        let mut img = Image::new(w, hh);
        for y in 0..hh {
            for x in 0..w {
                img.set(x, y, [x as f32 / w as f32, y as f32 / hh as f32, 0.4, 1.0]);
            }
        }
        img
    }

    fn run(id: &str, vals: &[(&str, Value)], img: Image, t: f64) -> Image {
        run_fx(id, vals, img, t, EffectEnv::default()).img
    }

    fn max_diff(a: &Image, b: &Image) -> f32 {
        a.data.iter().zip(&b.data).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0.0, f32::max)).fold(0.0, f32::max)
    }

    fn sum_diff(a: &Image, b: &Image) -> f32 {
        a.data.iter().zip(&b.data).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).sum::<f32>()).sum()
    }

    #[test]
    fn piece_at_rest_is_identity() {
        let src = ramp(16, 16);
        let cam = Cam { c: [8.0, 8.0], d: 64.0 };
        let id = rot_axis([1.0, 0.0, 0.0], 0.0);
        let pc = Piece::new(&[[0.0, 0.0], [16.0, 0.0], [16.0, 16.0], [0.0, 16.0]], [8.0, 8.0], &id, [8.0, 8.0, 0.0], cam).unwrap();
        let mut out = Image::new(16, 16);
        draw_pieces(&mut out, &[pc], &src, None);
        assert!(max_diff(&out, &src) < 1e-4);
    }

    #[test]
    fn ball_action_draws_balls() {
        let v = [("ballSize", num(80.0))];
        let a = run("ec.sim.ccballaction", &v, ramp(32, 32), 0.0);
        assert_eq!(a, run("ec.sim.ccballaction", &v, ramp(32, 32), 0.0));
        assert!(a.data.iter().any(|p| p[3] > 0.9) && a.data.iter().any(|p| p[3] == 0.0), "balls with gaps");
        let none = run("ec.sim.ccballaction", &[("ballSize", num(0.0))], ramp(32, 32), 0.0);
        assert!(none.data.iter().all(|p| p[3] == 0.0));
        let sc = run("ec.sim.ccballaction", &[("ballSize", num(80.0)), ("scatter", num(20.0))], ramp(32, 32), 0.0);
        assert!(sum_diff(&a, &sc) > 1.0);
    }

    #[test]
    fn pixel_polly_breaks_after_start() {
        let v = [("forceCenter", pt(16.0, 16.0)), ("gridSpacing", num(4.0))];
        let before = run("ec.sim.ccpixelpolly", &v, ramp(32, 32), 0.0);
        assert_eq!(before, ramp(32, 32), "intact before the start time");
        let a = run("ec.sim.ccpixelpolly", &v, ramp(32, 32), 0.5);
        assert_eq!(a, run("ec.sim.ccpixelpolly", &v, ramp(32, 32), 0.5));
        assert!(sum_diff(&a, &ramp(32, 32)) > 10.0);
        let tex = run("ec.sim.ccpixelpolly", &[("forceCenter", pt(16.0, 16.0)), ("gridSpacing", num(4.0)), ("object", Value::Enum(3))], ramp(32, 32), 0.5);
        assert!(sum_diff(&a, &tex) > 0.1);
    }

    #[test]
    fn scatterize_identity_and_scatter() {
        assert_eq!(run("ec.sim.ccscatterize", &[], ramp(24, 24), 0.0), ramp(24, 24));
        let a = run("ec.sim.ccscatterize", &[("scatter", num(5.0))], ramp(24, 24), 0.0);
        assert_eq!(a, run("ec.sim.ccscatterize", &[("scatter", num(5.0))], ramp(24, 24), 0.0));
        assert!(sum_diff(&a, &ramp(24, 24)) > 1.0);
        let tw = run("ec.sim.ccscatterize", &[("leftTwist", num(80.0)), ("rightTwist", num(80.0))], ramp(24, 24), 0.0);
        assert!(sum_diff(&tw, &ramp(24, 24)) > 1.0);
    }

    #[test]
    fn card_dance_identity_and_rotation() {
        let src = ramp(30, 20);
        let a = run("ec.sim.carddance", &[], src.clone(), 0.0);
        assert!(max_diff(&a, &src) < 0.02, "flat cards reproduce the layer: {}", max_diff(&a, &src));
        let r = run("ec.sim.carddance", &[("yRotSource", Value::Enum(1)), ("yRotMultiplier", num(90.0))], src.clone(), 0.0);
        assert_eq!(r, run("ec.sim.carddance", &[("yRotSource", Value::Enum(1)), ("yRotMultiplier", num(90.0))], src.clone(), 0.0));
        assert!(sum_diff(&r, &src) > 5.0);
        let z = run("ec.sim.carddance", &[("zPosOffset", num(-1.0))], src.clone(), 0.0);
        assert!(sum_diff(&z, &src) > 5.0, "cards pulled towards the camera grow");
    }

    #[test]
    fn shatter_at_rest_then_breaks() {
        let v = [("view", Value::Enum(0)), ("force1Position", pt(16.0, 16.0)), ("origin", pt(16.0, 16.0))];
        let src = ramp(32, 32);
        let t0 = run("ec.sim.shatter", &v, src.clone(), 0.0);
        assert!(max_diff(&t0, &src) < 0.03, "{}", max_diff(&t0, &src));
        let t1 = run("ec.sim.shatter", &v, src.clone(), 0.5);
        assert_eq!(t1, run("ec.sim.shatter", &v, src.clone(), 0.5));
        assert!(sum_diff(&t1, &src) > 10.0);
        for pat in 0..SHATTER_PATTERNS.len() as u32 {
            let vv = [("view", Value::Enum(0)), ("pattern", Value::Enum(pat)), ("force1Position", pt(16.0, 16.0)), ("origin", pt(16.0, 16.0))];
            let r = run("ec.sim.shatter", &vv, src.clone(), 0.0);
            assert!(max_diff(&r, &src) < 0.05, "pattern {pat} tiles the layer: {}", max_diff(&r, &src));
        }
        let wire = run("ec.sim.shatter", &[("repetitions", num(3.0))], src.clone(), 0.0);
        assert!(wire.data.iter().any(|p| p[3] > 0.5) && wire.data.iter().any(|p| p[3] == 0.0));
        let layer_only = run("ec.sim.shatter", &[("view", Value::Enum(0)), ("render", Value::Enum(1)), ("force1Position", pt(16.0, 16.0))], src.clone(), 0.5);
        assert!(layer_only.get(16, 16)[3] == 0.0, "broken centre removed");
    }

    #[test]
    fn caustics_flat_water() {
        let src = ramp(24, 24);
        let a = run(
            "ec.sim.caustics",
            &[("surfaceOpacity", num(0.0)), ("ambientLight", num(0.0)), ("diffuse", num(1.0)), ("lightHeight", num(1000.0)), ("specular", num(0.0))],
            src.clone(),
            0.0,
        );
        assert!(max_diff(&a, &src) < 0.01, "flat water, light overhead = bottom: {}", max_diff(&a, &src));
        let b = run("ec.sim.caustics", &[], src.clone(), 0.0);
        assert_eq!(b, run("ec.sim.caustics", &[], src.clone(), 0.0));
        assert!(sum_diff(&b, &src) > 0.1);
    }

    #[test]
    fn wave_world_seek_and_waves() {
        let v = vec![("view", Value::Enum(1)), ("producer1Position", pt(16.0, 16.0)), ("gridResolution", num(24.0))];
        let _ = run("ec.sim.waveworld", &v, Image::new(32, 32), 0.5);
        let a = run("ec.sim.waveworld", &v, Image::new(32, 32), 1.0);
        let mut v2 = v.clone();
        v2.push(("transparency", num(0.0)));
        assert_eq!(a, run("ec.sim.waveworld", &v2, Image::new(32, 32), 1.0));
        // Waves spread: some pixels differ from the flat-water grey.
        assert!(a.data.iter().any(|p| (p[0] - 0.5).abs() > 0.05), "waves");
        let flat = run("ec.sim.waveworld", &[("view", Value::Enum(1)), ("producer1Amplitude", num(0.0))], Image::new(32, 32), 1.0);
        assert!(flat.data.iter().all(|p| (p[0] - 0.5).abs() < 1e-5));
        let wire = run("ec.sim.waveworld", &[("producer1Position", pt(16.0, 16.0)), ("gridResolution", num(10.0))], Image::new(32, 32), 0.3);
        assert!(wire.data.iter().any(|p| p[1] > 0.3));
    }

    #[test]
    fn wave_world_steps_match_direct() {
        // Fresh caches: resumed simulation equals simulation from scratch.
        let v = vec![("view", Value::Enum(1)), ("producer1Position", pt(16.0, 16.0)), ("gridResolution", num(20.0)), ("damping", num(0.07))];
        let a = run("ec.sim.waveworld", &v, Image::new(32, 32), 1.3);
        let mut v2 = v.clone();
        v2.push(("damping", num(0.07000001)));
        // Different key: computed from scratch at 1.3 directly vs via 0.4 (both fresh keys).
        let _ = run("ec.sim.waveworld", &v2, Image::new(32, 32), 0.4);
        let b = run("ec.sim.waveworld", &v2, Image::new(32, 32), 1.3);
        assert!(max_diff(&a, &b) < 1e-3, "{}", max_diff(&a, &b));
    }

    #[test]
    fn foam_bubbles_drift_and_seek() {
        let v = vec![("producerPoint", pt(16.0, 16.0)), ("view", Value::Enum(1)), ("productionRate", num(2.0))];
        let none = run("ec.sim.foam", &v, Image::new(32, 32), 0.0);
        assert!(none.data.iter().all(|p| p[3] == 0.0));
        let _ = run("ec.sim.foam", &v, Image::new(32, 32), 0.6);
        let a = run("ec.sim.foam", &v, Image::new(32, 32), 1.2);
        let mut v2 = v.clone();
        v2.push(("randomSeed", num(1.0)));
        assert_eq!(a, run("ec.sim.foam", &v2, Image::new(32, 32), 1.2));
        assert!(a.data.iter().any(|p| p[3] > 0.05), "bubbles");
        let draft = run("ec.sim.foam", &[("producerPoint", pt(16.0, 16.0)), ("productionRate", num(2.0))], Image::new(32, 32), 1.0);
        assert!(draft.data.iter().any(|p| p[3] > 0.05));
    }
}
