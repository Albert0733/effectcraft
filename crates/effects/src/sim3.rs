//! Simulation: Particle Playground and CC Hair.
//!
//! **Particle Playground** is a deterministic 2D particle system stepped from layer time 0 at
//! [`SPS`] steps per second through [`SimCache`] (seeking anywhere gives the same frame as
//! playing up to it). Producers: **Cannon** (a stream from a barrel with angle, radius, rate,
//! velocity and random spreads), **Grid** (particles placed on a grid at time 0) and **Layer
//! Exploder** (another layer broken into particles at time 0). Forces: **Gravity** (force,
//! random spread, direction), **Repel** (particles push each other apart within a radius),
//! **Wall** (a mask of the layer the particles bounce off) and the **Persistent Property
//! Mapper** (a map layer's red / green / blue drive particle properties at the particle's
//! position each step). **Layer Map** draws particles with the colours of another layer
//! sampled at each particle's birth position. Map layers are read at the frame being rendered
//! (their pixels are part of the cache key). Particles replace the layer's own pixels, as in
//! After Effects.
//!
//! **CC Hair** grows strands from the layer's opaque pixels (Density per 1 000 px²), each a
//! quadratic curve of Length that droops with Weight, optionally steered and scaled by a hairfall
//! map layer (luminance → length, horizontal/vertical gradient → lean), and shades them with a
//! Kajiya–Kay-style strand model (diffuse ∝ sin(tangent, light), specular from the half vector).

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::{Image, Px};
use rayon::prelude::*;

use crate::sim::{Acc, SPS, Shape, Sprite, combine, splat, steps_at};
use crate::util::{SimCache, hash1, params_key, point_in_poly, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, col, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Simulation", params, render, gpu: false, float: true }
}

// ---------------------------------------------------------------- Particle Playground

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PgParticle {
    pub p: [f32; 2],
    pub v: [f32; 2],
    /// Layer position where the particle was born (Layer Map lookups).
    pub origin: [f32; 2],
    pub c: [f32; 4],
    pub r: f32,
    pub mass: f32,
    pub friction: f32,
    pub force: [f32; 2],
    pub id: u32,
}

#[derive(Clone, Debug, Default)]
pub struct PgState {
    pub parts: Vec<PgParticle>,
    carry: f64,
    next_id: u32,
}

/// A map layer sampled in layer coordinates.
struct MapImg {
    img: Image,
    /// Layer point → pixel: (x · k₀ + o₀, y · k₁ + o₁).
    k: [f64; 2],
    o: [f64; 2],
}

impl MapImg {
    fn from(ctx: &EffectCtx, lp: crate::LayerPixels) -> MapImg {
        // Fit the map layer to this layer (stretch), like AE's layer maps.
        let (ls, os) = (ctx.layer_size, lp.size);
        let sx = if ls[0] > 0.0 { os[0] / ls[0] } else { 1.0 };
        let sy = if ls[1] > 0.0 { os[1] / ls[1] } else { 1.0 };
        MapImg { img: lp.buf.img, k: [lp.buf.scale * sx, lp.buf.scale * sy], o: lp.buf.offset }
    }
    fn at(&self, x: f32, y: f32) -> Px {
        let (c, a) = unpremul(self.img.sample_bilinear_clamped(x as f64 * self.k[0] + self.o[0], y as f64 * self.k[1] + self.o[1]));
        [c[0], c[1], c[2], a]
    }
}

pub const MAP_TARGETS: [&str; 12] =
    ["None", "Red", "Green", "Blue", "Scale", "Opacity", "X Velocity", "Y Velocity", "X Force", "Y Force", "Kinetic Friction", "Mass"];

struct Pg {
    // Cannon
    cannon: bool,
    pos: [f32; 2],
    barrel_angle: f32,
    barrel_radius: f32,
    rate: f64,
    dir_spread: f32,
    vel: f32,
    vel_spread: f32,
    color: [f32; 4],
    radius: f32,
    // Gravity
    gravity: [f32; 2],
    grav_spread: f32,
    // Repel
    repel: f32,
    repel_radius: f32,
    // Wall
    wall: Option<Vec<[f64; 2]>>,
    // Property mapper
    mapper: Option<MapImg>,
    map_to: [(u32, f32, f32); 3],
    seed: u32,
    bounds: [f32; 4],
}

impl Pg {
    fn spawn(&self, id: u32) -> PgParticle {
        let s = self.seed;
        let h = |k: u32| hash1(id, k, s) * 2.0 - 1.0;
        // AE angles: 0° = up, clockwise.
        let a = (self.barrel_angle + h(1) * self.dir_spread * 0.5).to_radians();
        let d = [a.sin(), -a.cos()];
        let perp = [-d[1], d[0]];
        let off = h(2) * self.barrel_radius;
        let speed = (self.vel + h(3) * self.vel_spread).max(0.0);
        let p = [self.pos[0] + perp[0] * off, self.pos[1] + perp[1] * off];
        PgParticle { p, v: [d[0] * speed, d[1] * speed], origin: p, c: self.color, r: self.radius, mass: 1.0, friction: 0.0, force: [0.0; 2], id }
    }

    fn step(&self, st: &mut PgState) {
        let dt = (1.0 / SPS) as f32;
        let s = self.seed;
        // Property mapper (persistent): drive properties from the map at the particle position.
        if let Some(m) = &self.mapper {
            st.parts.par_iter_mut().for_each(|q| {
                let px = m.at(q.p[0], q.p[1]);
                for (ch, &(target, lo, hi)) in self.map_to.iter().enumerate() {
                    let v = lo + (hi - lo) * px[ch];
                    match target {
                        1 => q.c[0] = v,
                        2 => q.c[1] = v,
                        3 => q.c[2] = v,
                        4 => q.r = self.radius * v.max(0.0),
                        5 => q.c[3] = v.clamp(0.0, 1.0),
                        6 => q.v[0] = v,
                        7 => q.v[1] = v,
                        8 => q.force[0] = v,
                        9 => q.force[1] = v,
                        10 => q.friction = v.clamp(0.0, 1.0),
                        11 => q.mass = v.max(0.01),
                        _ => {}
                    }
                }
            });
        }
        // Repel: spatial hash, symmetric pairwise push within the radius.
        let mut push = vec![[0.0f32; 2]; st.parts.len()];
        if self.repel != 0.0 && self.repel_radius > 0.0 && st.parts.len() > 1 {
            let cell = self.repel_radius;
            let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
            for (i, q) in st.parts.iter().enumerate() {
                grid.entry(((q.p[0] / cell).floor() as i32, (q.p[1] / cell).floor() as i32)).or_default().push(i);
            }
            let parts = &st.parts;
            push.par_iter_mut().enumerate().for_each(|(i, f)| {
                let q = parts[i];
                let (cx, cy) = ((q.p[0] / cell).floor() as i32, (q.p[1] / cell).floor() as i32);
                for gy in cy - 1..=cy + 1 {
                    for gx in cx - 1..=cx + 1 {
                        let Some(list) = grid.get(&(gx, gy)) else { continue };
                        for &j in list {
                            if j == i {
                                continue;
                            }
                            let o = parts[j];
                            let (dx, dy) = (q.p[0] - o.p[0], q.p[1] - o.p[1]);
                            let d = (dx * dx + dy * dy).sqrt();
                            if d < self.repel_radius && d > 1e-4 {
                                let k = self.repel * (1.0 - d / self.repel_radius) / d;
                                f[0] += dx * k;
                                f[1] += dy * k;
                            }
                        }
                    }
                }
            });
        }
        let wall = self.wall.as_deref();
        st.parts.par_iter_mut().zip(push.par_iter()).for_each(|(q, f)| {
            let g = if self.grav_spread > 0.0 { 1.0 + (hash1(q.id, 7, s) * 2.0 - 1.0) * self.grav_spread } else { 1.0 };
            let ax = self.gravity[0] * g + (q.force[0] + f[0]) / q.mass;
            let ay = self.gravity[1] * g + (q.force[1] + f[1]) / q.mass;
            let damp = 1.0 - q.friction * 0.1;
            q.v = [(q.v[0] + ax * dt) * damp, (q.v[1] + ay * dt) * damp];
            let np = [q.p[0] + q.v[0] * dt, q.p[1] + q.v[1] * dt];
            if let Some(w) = wall {
                let was = point_in_poly(w, q.p[0] as f64, q.p[1] as f64);
                let now = point_in_poly(w, np[0] as f64, np[1] as f64);
                if was != now {
                    // Bounce: reverse the velocity and stay on the same side.
                    q.v = [-q.v[0] * 0.9, -q.v[1] * 0.9];
                    return;
                }
            }
            q.p = np;
        });
        let b = self.bounds;
        st.parts.retain(|q| q.p[0] > b[0] && q.p[0] < b[2] && q.p[1] > b[1] && q.p[1] < b[3]);
        if self.cannon {
            st.carry += self.rate / SPS;
            let n = st.carry.floor() as u32;
            st.carry -= n as f64;
            for _ in 0..n {
                if st.parts.len() >= 60_000 {
                    break;
                }
                let id = st.next_id;
                st.next_id = st.next_id.wrapping_add(1);
                let mut q = self.spawn(id);
                // Sub-step birth offset keeps streams smooth.
                let frac = hash1(id, 9, s) * dt;
                q.p = [q.p[0] + q.v[0] * frac, q.p[1] + q.v[1] * frac];
                st.parts.push(q);
            }
        }
    }
}

static PG_CACHE: SimCache<PgState> = SimCache::new(6);

fn image_key(img: &Image) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    img.width.hash(&mut h);
    img.height.hash(&mut h);
    for p in img.data.iter().step_by(7) {
        for c in p {
            c.to_bits().hash(&mut h);
        }
    }
    h.finish()
}

fn particle_playground(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let (lw, lh) = (ctx.layer_size[0] as f32, ctx.layer_size[1] as f32);
    let grav_dir = (pr.f("gravityDirection") as f32).to_radians();
    let gf = pr.f("gravityForce") as f32;
    let wall = (pr.f("wallBoundary").round() as usize).checked_sub(1).and_then(|i| ctx.env.masks.get(i)).map(|m| m.points.clone());
    let mapper = if pr.b("mapperEnabled") { ctx.layer_param("useLayerAsMap", true).map(|lp| MapImg::from(ctx, lp)) } else { None };
    let pg = Pg {
        cannon: pr.b("cannonEnabled"),
        pos: { pr.v2("cannonPosition").map(|v| v as f32) },
        barrel_angle: pr.f("barrelAngle") as f32,
        barrel_radius: pr.f("barrelRadius") as f32,
        rate: pr.f("particlesPerSecond").max(0.0),
        dir_spread: pr.f("directionRandomSpread") as f32,
        vel: pr.f("velocity") as f32,
        vel_spread: pr.f("velocityRandomSpread") as f32,
        color: pr.color("cannonColor"),
        radius: pr.f("cannonParticleRadius").max(0.0) as f32,
        gravity: [grav_dir.sin() * gf, -grav_dir.cos() * gf],
        grav_spread: (pr.f("gravityForceRandomSpread") / 100.0).max(0.0) as f32,
        repel: pr.f("repelForce") as f32 * 100.0,
        repel_radius: pr.f("repelForceRadius").max(0.0) as f32,
        wall,
        mapper,
        map_to: [
            (pr.e("mapRedTo"), pr.f("redMin") as f32, pr.f("redMax") as f32),
            (pr.e("mapGreenTo"), pr.f("greenMin") as f32, pr.f("greenMax") as f32),
            (pr.e("mapBlueTo"), pr.f("blueMin") as f32, pr.f("blueMax") as f32),
        ],
        seed: (pr.f("randomSeed") as u32).wrapping_mul(0x9e37_79b9) ^ ctx.seed,
        bounds: [-lw * 2.0, -lh * 2.0, lw * 3.0, lh * 3.0],
    };
    // Initial particles: Grid and Layer Exploder.
    let grid_on = pr.b("gridEnabled");
    let across = pr.f("particlesAcross").max(0.0).round() as u32;
    let down = pr.f("particlesDown").max(0.0).round() as u32;
    let gpos = pr.v2("gridPosition");
    let (gw, gh) = (pr.f("gridWidth"), pr.f("gridHeight"));
    let gcol = pr.color("gridColor");
    let grad = pr.f("gridParticleRadius").max(0.0) as f32;
    let exploder = if pr.b("exploderEnabled") {
        let host = ctx.env.host;
        let lid = pr.get("explodeLayer").and_then(Value::as_layer);
        match (host, lid) {
            (Some(h), Some(id)) => h.layer_at(id, ctx.env.comp_time - ctx.time, true).or_else(|| h.layer(id, true)),
            _ => None,
        }
    } else {
        None
    };
    let ex_r = pr.f("radiusOfNewParticles").max(0.5) as f32;
    let ex_disp = pr.f("velocityDispersion") as f32;
    let layer_map = pr.b("layerMapEnabled").then(|| ctx.layer_param("layerMapLayer", true).map(|lp| MapImg::from(ctx, lp))).flatten();

    let mut key = params_key(ctx, &Buf { img: Image::new(0, 0), offset: [0.0; 2], scale: 1.0 }, 0x7067);
    if let Some(m) = &pg.mapper {
        key ^= image_key(&m.img).rotate_left(7);
    }
    if let Some(e) = &exploder {
        key ^= image_key(&e.buf.img).rotate_left(13);
    }
    if let Some(w) = &pg.wall {
        key ^= w.iter().fold(0u64, |a, p| a.rotate_left(5) ^ p[0].to_bits() ^ p[1].to_bits().rotate_left(17));
    }
    let seed = pg.seed;
    let init = || {
        let mut st = PgState::default();
        if grid_on && across > 0 && down > 0 {
            for j in 0..down {
                for i in 0..across {
                    let fx = if across > 1 { i as f64 / (across - 1) as f64 - 0.5 } else { 0.0 };
                    let fy = if down > 1 { j as f64 / (down - 1) as f64 - 0.5 } else { 0.0 };
                    let p = [(gpos[0] + fx * gw) as f32, (gpos[1] + fy * gh) as f32];
                    let id = st.next_id;
                    st.next_id += 1;
                    st.parts.push(PgParticle { p, v: [0.0; 2], origin: p, c: gcol, r: grad, mass: 1.0, friction: 0.0, force: [0.0; 2], id });
                }
            }
        }
        if let Some(e) = &exploder {
            let m = MapImg::from(ctx, e.clone());
            let step = (ex_r * 2.0).max(1.0);
            let mut y = step * 0.5;
            while y < lh {
                let mut x = step * 0.5;
                while x < lw {
                    let c = m.at(x, y);
                    if c[3] > 0.05 {
                        let id = st.next_id;
                        st.next_id += 1;
                        let h = |k: u32| hash1(id, k, seed) * 2.0 - 1.0;
                        let (dx, dy) = (x - lw * 0.5, y - lh * 0.5);
                        let l = (dx * dx + dy * dy).sqrt().max(1.0);
                        let v = [dx / l * ex_disp + h(1) * ex_disp, dy / l * ex_disp + h(2) * ex_disp];
                        st.parts.push(PgParticle { p: [x, y], v, origin: [x, y], c, r: ex_r, mass: 1.0, friction: 0.0, force: [0.0; 2], id });
                    }
                    x += step;
                }
                y += step;
            }
        }
        st
    };
    let st = PG_CACHE.run(key, steps_at(ctx.time), init, |st, _| pg.step(st));
    let s = b.scale as f32;
    let sprites: Vec<Sprite> = st
        .parts
        .iter()
        .map(|q| {
            let (x, y) = b.to_px([q.p[0] as f64, q.p[1] as f64]);
            let c = match &layer_map {
                Some(m) => {
                    let mc = m.at(q.origin[0], q.origin[1]);
                    [mc[0], mc[1], mc[2], mc[3] * q.c[3]]
                }
                None => q.c,
            };
            Sprite::new(x as f32, y as f32, q.r * s, c, Shape::Disc)
        })
        .collect();
    let fx = splat(b.img.width, b.img.height, &sprites, Acc::Over);
    // Particles replace the layer's own pixels.
    b.img = combine(&b.img, &fx, 5);
    b
}

// ---------------------------------------------------------------- CC Hair

fn cc_hair(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let len = pr.f("length").max(0.0) as f32;
    let density = pr.f("density").max(0.0) as f32 / 100.0;
    if len <= 0.0 || density <= 0.0 {
        return b;
    }
    let thick = pr.f("thickness").max(0.05) as f32;
    let weight = pr.f("weight") as f32;
    let const_mass = pr.b("constantMass");
    let map = ctx.layer_param("mapLayer", true).map(|lp| MapImg::from(ctx, lp));
    let map_strength = (pr.f("mapStrength") / 100.0) as f32;
    let map_soft = pr.f("mapSoftness").max(0.0) as f32;
    let noise = (pr.f("addNoise") / 100.0) as f32;
    let hc = pr.color("hairColor");
    let bright = (pr.f("brightness") / 100.0) as f32;
    let opacity = (pr.f("opacity") / 100.0) as f32;
    let inherit = (pr.f("colorInheritance") / 100.0) as f32;
    let light_i = (pr.f("lightIntensity") / 100.0) as f32;
    let ld = (pr.f("lightDirection") as f32).to_radians();
    let light = [ld.sin(), -ld.cos()];
    let (amb, dif, spe) = ((pr.f("ambient") / 100.0) as f32, (pr.f("diffuse") / 100.0) as f32, (pr.f("specular") / 100.0) as f32);
    let rough = (pr.f("roughness") / 100.0).clamp(0.01, 1.0) as f32;
    let seed = ctx.seed ^ (pr.f("randomSeed") as u32).wrapping_mul(0x85eb_ca6b);
    let (lw, lh) = (ctx.layer_size[0] as f32, ctx.layer_size[1] as f32);
    // Roots: a jittered grid over the layer, density per 1 000 px².
    let count = ((lw * lh / 1000.0) * density * 10.0).min(200_000.0) as u32;
    let src = b.img.clone();
    let segs = 6u32;
    let strands: Vec<Vec<Sprite>> = (0..count)
        .into_par_iter()
        .filter_map(|i| {
            let rx = hash1(i, 1, seed) * lw;
            let ry = hash1(i, 2, seed) * lh;
            let (bx, by) = b.to_px([rx as f64, ry as f64]);
            let root = src.sample_bilinear(bx, by);
            if root[3] < 0.5 {
                return None;
            }
            let (rc, _) = unpremul(root);
            let mut l = len * (0.75 + 0.5 * hash1(i, 3, seed));
            // Lean: random, plus the map's local gradient (hairfall map).
            let mut lean = [(hash1(i, 4, seed) * 2.0 - 1.0) * 0.6, -1.0 + hash1(i, 5, seed) * 0.4];
            if let Some(m) = &map {
                let lum = |x: f32, y: f32| {
                    let c = m.at(x, y);
                    (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]) * c[3]
                };
                let d = map_soft.max(1.0);
                let (gx, gy) = (lum(rx + d, ry) - lum(rx - d, ry), lum(rx, ry + d) - lum(rx, ry - d));
                l *= 1.0 - map_strength + map_strength * lum(rx, ry);
                lean = [lean[0] + gx * map_strength * 4.0, lean[1] + gy * map_strength * 4.0];
                if noise > 0.0 {
                    lean[0] += (hash1(i, 6, seed) * 2.0 - 1.0) * noise;
                }
            }
            let ll = (lean[0] * lean[0] + lean[1] * lean[1]).sqrt().max(1e-4);
            let dir = [lean[0] / ll, lean[1] / ll];
            // Droop: heavier (or thinner with Constant Mass off) hair bends more.
            let droop = weight * if const_mass { 1.0 } else { 1.0 / thick.max(0.2) } * l;
            let base = [rc[0] + (hc[0] - rc[0]) * (1.0 - inherit), rc[1] + (hc[1] - rc[1]) * (1.0 - inherit), rc[2] + (hc[2] - rc[2]) * (1.0 - inherit)];
            let pt = |s: f32| [rx + dir[0] * l * s, ry + dir[1] * l * s + droop * s * s];
            let mut out = Vec::with_capacity(segs as usize);
            for k in 0..segs {
                let (s0, s1) = (k as f32 / segs as f32, (k + 1) as f32 / segs as f32);
                let (a, c) = (pt(s0), pt(s1));
                let t = {
                    let (tx, ty) = (c[0] - a[0], c[1] - a[1]);
                    let tl = (tx * tx + ty * ty).sqrt().max(1e-6);
                    [tx / tl, ty / tl]
                };
                // Strand shading (Kajiya–Kay): diffuse ∝ sin(T, L), specular from T·L.
                let tl = t[0] * light[0] + t[1] * light[1];
                let diff = (1.0 - tl * tl).max(0.0).sqrt();
                let spec = (1.0 - tl.abs()).powf(1.0 / rough) * spe;
                let shade = (amb + dif * diff * light_i) * bright;
                let tip = 1.0 - 0.6 * s1;
                let c3 = [base[0] * shade + spec * light_i, base[1] * shade + spec * light_i, base[2] * shade + spec * light_i];
                let (pa, pc) = (b.to_px([a[0] as f64, a[1] as f64]), b.to_px([c[0] as f64, c[1] as f64]));
                out.push(Sprite {
                    x: pa.0 as f32,
                    y: pa.1 as f32,
                    r: thick * 0.5 * b.scale as f32 * tip,
                    c: [c3[0], c3[1], c3[2], opacity],
                    shape: Shape::Line { dx: (pc.0 - pa.0) as f32, dy: (pc.1 - pa.1) as f32 },
                    rot: 0.0,
                });
            }
            Some(out)
        })
        .collect();
    let sprites: Vec<Sprite> = strands.into_iter().flatten().collect();
    let fx = splat(b.img.width, b.img.height, &sprites, Acc::Over);
    b.img = combine(&b.img, &fx, 0);
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let pt = |x: f64, y: f64| Value::Vec2([x, y]);
    let px = |d: f64, max: f64| (num(d), slider(0.0, max * 10.0, 0.0, max, 1));
    let sp = |id: &'static str, name: &'static str, v: (Value, ParamUi)| p(id, name, v.0, v.1);
    let mut pg = vec![
        // Cannon
        p("cannonEnabled", "Cannon", Value::Bool(true), ParamUi::Checkbox),
        p("cannonPosition", "Position", pt(0.5, 0.9), ParamUi::Point),
        p("barrelAngle", "Barrel Angle", num(0.0), ParamUi::Angle),
        sp("barrelRadius", "Barrel Radius", px(0.0, 100.0)),
        sp("particlesPerSecond", "Particles Per Second", px(60.0, 500.0)),
        sp("directionRandomSpread", "Direction Random Spread", px(20.0, 360.0)),
        sp("velocity", "Velocity", px(130.0, 1000.0)),
        sp("velocityRandomSpread", "Velocity Random Spread", px(20.0, 500.0)),
        p("cannonColor", "Color", col(1.0, 0.0, 0.0), ParamUi::Color),
        sp("cannonParticleRadius", "Particle Radius", px(2.0, 50.0)),
        // Grid
        p("gridEnabled", "Grid", Value::Bool(true), ParamUi::Checkbox),
        p("gridPosition", "Grid Position", pt(0.5, 0.5), ParamUi::Point),
        sp("gridWidth", "Width", px(100.0, 2000.0)),
        sp("gridHeight", "Height", px(100.0, 2000.0)),
        sp("particlesAcross", "Particles Across", px(0.0, 100.0)),
        sp("particlesDown", "Particles Down", px(0.0, 100.0)),
        p("gridColor", "Grid Color", col(1.0, 1.0, 1.0), ParamUi::Color),
        sp("gridParticleRadius", "Grid Particle Radius", px(2.0, 50.0)),
        // Layer Exploder
        p("exploderEnabled", "Layer Exploder", Value::Bool(true), ParamUi::Checkbox),
        p("explodeLayer", "Explode Layer", Value::Layer(None), ParamUi::Layer),
        sp("radiusOfNewParticles", "Radius of New Particles", px(2.0, 50.0)),
        sp("velocityDispersion", "Velocity Dispersion", px(20.0, 500.0)),
        // Layer Map
        p("layerMapEnabled", "Layer Map", Value::Bool(true), ParamUi::Checkbox),
        p("layerMapLayer", "Use Layer", Value::Layer(None), ParamUi::Layer),
        // Gravity
        sp("gravityForce", "Force", px(108.0, 1000.0)),
        sp("gravityForceRandomSpread", "Force Random Spread", px(0.0, 100.0)),
        p("gravityDirection", "Direction", num(180.0), ParamUi::Angle),
        // Repel
        p("repelForce", "Repel Force", num(0.0), slider(-100.0, 100.0, -10.0, 10.0, 2)),
        sp("repelForceRadius", "Force Radius", px(0.0, 100.0)),
        // Wall
        p("wallBoundary", "Boundary", num(0.0), ParamUi::Mask),
        // Persistent Property Mapper
        p("mapperEnabled", "Persistent Property Mapper", Value::Bool(true), ParamUi::Checkbox),
        p("useLayerAsMap", "Use Layer As Map", Value::Layer(None), ParamUi::Layer),
    ];
    for (to, mn, mx, n_to, n_min, n_max) in [
        ("mapRedTo", "redMin", "redMax", "Map Red To", "Red Min", "Red Max"),
        ("mapGreenTo", "greenMin", "greenMax", "Map Green To", "Green Min", "Green Max"),
        ("mapBlueTo", "blueMin", "blueMax", "Map Blue To", "Blue Min", "Blue Max"),
    ] {
        pg.push(p(to, n_to, Value::Enum(0), popup(&MAP_TARGETS)));
        pg.push(p(mn, n_min, num(0.0), slider(-10000.0, 10000.0, -100.0, 100.0, 2)));
        pg.push(p(mx, n_max, num(1.0), slider(-10000.0, 10000.0, -100.0, 100.0, 2)));
    }
    pg.push(p("randomSeed", "Random Seed", num(0.0), slider(0.0, 10000.0, 0.0, 1000.0, 0)));
    let pc = |d: f64| (num(d), slider(0.0, 100.0, 0.0, 100.0, 1));
    vec![
        spec("ec.sim.particleplayground", "Particle Playground", pg, particle_playground),
        spec(
            "ec.sim.cchair",
            "CC Hair",
            vec![
                sp("length", "Length", px(30.0, 200.0)),
                p("thickness", "Thickness", num(1.0), slider(0.05, 20.0, 0.1, 5.0, 2)),
                p("weight", "Weight", num(0.2), slider(-10.0, 10.0, -2.0, 2.0, 2)),
                p("constantMass", "Constant Mass", Value::Bool(false), ParamUi::Checkbox),
                sp("density", "Density", px(100.0, 1000.0)),
                // Hairfall Map
                sp("mapStrength", "Map Strength", pc(0.0)),
                p("mapLayer", "Map Layer", Value::Layer(None), ParamUi::Layer),
                sp("mapSoftness", "Map Softness", px(0.0, 100.0)),
                sp("addNoise", "Add Noise", pc(0.0)),
                // Hair Color
                p("hairColor", "Color", col(0.35, 0.25, 0.15), ParamUi::Color),
                sp("brightness", "Brightness", px(100.0, 400.0)),
                sp("opacity", "Opacity", pc(100.0)),
                sp("colorInheritance", "Color Inheritance", pc(0.0)),
                // Light
                sp("lightIntensity", "Light Intensity", px(100.0, 400.0)),
                p("lightDirection", "Light Direction", num(-45.0), ParamUi::Angle),
                // Shading
                sp("ambient", "Ambient", pc(40.0)),
                sp("diffuse", "Diffuse", pc(60.0)),
                sp("specular", "Specular", pc(30.0)),
                sp("roughness", "Roughness", pc(20.0)),
                p("randomSeed", "Random Seed", num(0.0), slider(0.0, 10000.0, 0.0, 1000.0, 0)),
            ],
            cc_hair,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectEnv, MaskShape, run_fx};

    fn run(vals: &[(&str, Value)], t: f64) -> Image {
        run_fx("ec.sim.particleplayground", vals, Image::filled(80, 60, [0.2, 0.2, 0.2, 1.0]), t, EffectEnv::default()).img
    }

    fn cannon(t: f64) -> Image {
        run(&[("cannonPosition", Value::Vec2([40.0, 50.0])), ("velocity", num(60.0)), ("gravityForce", num(30.0))], t)
    }

    #[test]
    fn cannon_stream_is_seek_consistent() {
        let a = cannon(1.0);
        // Particles replace the layer: some coverage, mostly transparent, red.
        let cov: f32 = a.data.iter().map(|p| p[3]).sum();
        assert!(cov > 5.0, "{cov}");
        assert!(a.data.iter().any(|p| p[3] > 0.5 && p[0] > 0.5 && p[1] < 0.1));
        // Seek back and forth: the same frame every time.
        let _ = cannon(2.0);
        let b = cannon(1.0);
        assert_eq!(a.data, b.data);
        let c = cannon(0.5);
        assert_ne!(a.data, c.data);
        // Nothing has been fired at time 0.
        let z = cannon(0.0);
        assert!(z.data.iter().all(|p| p[3] == 0.0));
    }

    #[test]
    fn grid_falls_with_gravity_and_bounces_off_a_wall() {
        let grid = |t: f64, env: EffectEnv| {
            run_fx(
                "ec.sim.particleplayground",
                &[
                    ("cannonEnabled", Value::Bool(false)),
                    ("gridPosition", Value::Vec2([40.0, 10.0])),
                    ("gridWidth", num(40.0)),
                    ("gridHeight", num(0.0)),
                    ("particlesAcross", num(5.0)),
                    ("particlesDown", num(1.0)),
                    ("gravityForce", num(100.0)),
                    ("wallBoundary", num(1.0)),
                ],
                Image::new(80, 60),
                t,
                env,
            )
            .img
        };
        // Alpha-weighted mean height of the particles.
        let row_of = |img: &Image| {
            let (mut s, mut n) = (0.0f64, 0.0f64);
            for y in 0..60 {
                for x in 0..80 {
                    let a = img.get(x, y)[3] as f64;
                    s += (y as f64 + 0.5) * a;
                    n += a;
                }
            }
            (s / n.max(1e-9)).round() as i64
        };
        let t0 = grid(0.0, EffectEnv::default());
        let t1 = grid(0.5, EffectEnv::default());
        assert_eq!(row_of(&t0), 10);
        // ½ g t² = 12.5 px.
        assert!((row_of(&t1) - 22).abs() <= 2, "{}", row_of(&t1));
        // A wall (mask 1) whose floor is at y = 15 keeps them above it.
        let masks = [MaskShape { name: "w".into(), points: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 15.0], [0.0, 15.0]], closed: true, inverted: false }];
        let env = EffectEnv { masks: &masks, ..Default::default() };
        let w = grid(1.5, env);
        assert!(row_of(&w) <= 16, "{}", row_of(&w));
    }

    #[test]
    fn repel_spreads_particles() {
        let spread = |force: f64| {
            let img = run(
                &[
                    ("cannonEnabled", Value::Bool(false)),
                    ("gridPosition", Value::Vec2([40.0, 30.0])),
                    ("gridWidth", num(6.0)),
                    ("gridHeight", num(6.0)),
                    ("particlesAcross", num(3.0)),
                    ("particlesDown", num(3.0)),
                    ("gravityForce", num(0.0)),
                    ("repelForce", num(force)),
                    ("repelForceRadius", num(20.0)),
                ],
                0.5,
            );
            let (mut x0, mut x1) = (80, 0);
            for y in 0..60 {
                for x in 0..80 {
                    if img.get(x, y)[3] > 0.3 {
                        x0 = x0.min(x);
                        x1 = x1.max(x);
                    }
                }
            }
            x1 - x0
        };
        assert!(spread(0.2) > spread(0.0) + 4, "{} vs {}", spread(0.2), spread(0.0));
    }

    #[test]
    fn property_mapper_and_layer_map_read_layers() {
        // Without a host, the layer-based features are inert (deterministic output).
        let a = run(&[("mapRedTo", Value::Enum(6)), ("redMin", num(-50.0)), ("redMax", num(50.0))], 0.7);
        let b = run(&[("mapRedTo", Value::Enum(6)), ("redMin", num(-50.0)), ("redMax", num(50.0))], 0.7);
        assert_eq!(a.data, b.data);
        let st = Pg {
            cannon: false,
            pos: [0.0; 2],
            barrel_angle: 0.0,
            barrel_radius: 0.0,
            rate: 0.0,
            dir_spread: 0.0,
            vel: 0.0,
            vel_spread: 0.0,
            color: [1.0; 4],
            radius: 2.0,
            gravity: [0.0; 2],
            grav_spread: 0.0,
            repel: 0.0,
            repel_radius: 0.0,
            wall: None,
            mapper: Some(MapImg { img: Image::filled(4, 4, [1.0, 0.0, 0.5, 1.0]), k: [1.0; 2], o: [0.0; 2] }),
            map_to: [(6, 0.0, 120.0), (0, 0.0, 1.0), (4, 0.0, 2.0)],
            seed: 1,
            bounds: [-1e6, -1e6, 1e6, 1e6],
        };
        let mut s = PgState::default();
        s.parts.push(PgParticle { p: [1.0, 1.0], v: [0.0; 2], origin: [1.0, 1.0], c: [1.0; 4], r: 2.0, mass: 1.0, friction: 0.0, force: [0.0; 2], id: 0 });
        st.step(&mut s);
        // Red 1 → X velocity 120 px/s; blue 0.5 → scale 1 (radius 2).
        assert!((s.parts[0].v[0] - 120.0).abs() < 1e-3);
        assert!((s.parts[0].r - 2.0).abs() < 1e-3);
    }

    #[test]
    fn cc_hair_grows_from_opaque_pixels() {
        let mut img = Image::new(60, 60);
        for y in 20..40 {
            for x in 20..40 {
                img.set(x, y, [0.8, 0.6, 0.4, 1.0]);
            }
        }
        let out = run_fx("ec.sim.cchair", &[("length", num(12.0)), ("density", num(300.0))], img.clone(), 0.0, EffectEnv::default());
        // Hair reaches beyond the square (above it: hair leans up, then droops).
        let outside: f32 = (0..60).flat_map(|x| (0..20).map(move |y| (x, y))).map(|(x, y)| out.img.get(x, y)[3]).sum();
        assert!(outside > 1.0, "{outside}");
        // Nothing grows from transparent areas far away.
        assert_eq!(out.img.get(2, 58)[3], 0.0);
        let again = run_fx("ec.sim.cchair", &[("length", num(12.0)), ("density", num(300.0))], img.clone(), 3.0, EffectEnv::default());
        assert_eq!(out.img.data, again.img.data);
        let none = run_fx("ec.sim.cchair", &[("length", num(0.0))], img.clone(), 0.0, EffectEnv::default());
        assert_eq!(none.img.data, img.data);
    }
}
