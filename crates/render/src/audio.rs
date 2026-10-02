//! Audio mixdown of a composition: every audible footage layer (and, recursively, precomp layer)
//! summed with its Audio Levels (dB, per channel), respecting in/out points, the Audio switch and
//! solo. Levels are evaluated once per call (callers mix in short blocks, e.g. one video frame).
//! Time-stretched layers are resampled nearest-sample; reversed (negative stretch) layers are
//! silent for now.

use effectcraft_project::{Comp, ItemId, ItemKind, Layer, LayerSource, Project};
use effectcraft_time::{TICKS_PER_SECOND, Tick};

use crate::{EvalCtx, ExprHost, FootageSource};

const MAX_DEPTH: usize = 16;

/// `frames` stereo sample frames of `comp`'s mix from comp time `start` at `rate` Hz,
/// interleaved (L R L R …).
pub fn mix_comp(project: &Project, footage: &dyn FootageSource, expr: Option<&dyn ExprHost>, comp: ItemId, start: Tick, frames: usize, rate: u32) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    mix_into(project, footage, expr, comp, start, rate, &mut out, 1.0, 1.0, 0);
    out
}

/// Whether the comp has anything audible (footage with audio, or a precomp that has).
pub fn comp_has_audio(project: &Project, comp: ItemId) -> bool {
    fn walk(project: &Project, comp: ItemId, depth: usize) -> bool {
        let Some(c) = project.comp(comp) else { return false };
        depth < MAX_DEPTH
            && c.layers.iter().any(|l| {
                l.switches.audio
                    && match &l.source {
                        LayerSource::Footage { item } => {
                            matches!(project.item(*item).map(|i| &i.kind), Some(ItemKind::Footage(f)) if f.has_audio && !f.missing)
                        }
                        LayerSource::Comp { item } => walk(project, *item, depth + 1),
                        _ => false,
                    }
            })
    }
    walk(project, comp, 0)
}

fn audible(project: &Project, l: &Layer) -> bool {
    l.switches.audio
        && match &l.source {
            LayerSource::Footage { item } => matches!(project.item(*item).map(|i| &i.kind), Some(ItemKind::Footage(f)) if f.has_audio),
            LayerSource::Comp { .. } => true,
            _ => false,
        }
}

fn db_to_gain(db: f64) -> f32 {
    if db <= -96.0 { 0.0 } else { 10f64.powf(db / 20.0) as f32 }
}

fn units_ceil(t: Tick, rate: u32) -> i64 {
    let n = t.0 as i128 * rate as i128;
    let d = TICKS_PER_SECOND as i128;
    (n.div_euclid(d) + i128::from(n.rem_euclid(d) != 0)) as i64
}

fn sample_tick(i: i64, rate: u32) -> Tick {
    Tick(((i as i128 * TICKS_PER_SECOND as i128) / rate as i128) as i64)
}

#[allow(clippy::too_many_arguments)]
fn mix_into(
    project: &Project,
    footage: &dyn FootageSource,
    expr: Option<&dyn ExprHost>,
    comp_id: ItemId,
    start: Tick,
    rate: u32,
    out: &mut [f32],
    gl: f32,
    gr: f32,
    depth: usize,
) {
    let Some(comp) = project.comp(comp_id) else { return };
    if depth >= MAX_DEPTH || rate == 0 {
        return;
    }
    let frames = out.len() / 2;
    let any_solo = comp.layers.iter().any(|l| l.switches.solo && audible(project, l));
    let s0 = start.to_units_floor(rate as i64);
    for l in &comp.layers {
        if !audible(project, l) || (any_solo && !l.switches.solo) || l.stretch <= 0.0 {
            continue;
        }
        // Sample range of the block that falls inside the layer's [in, out).
        let a = (units_ceil(l.in_point, rate) - s0).clamp(0, frames as i64) as usize;
        let b = (units_ceil(l.out_point, rate) - s0).clamp(0, frames as i64) as usize;
        if a >= b {
            continue;
        }
        let t_a = sample_tick(s0 + a as i64, rate);
        let (ll, lr) = levels(project, comp_id, comp, l, t_a, expr);
        let (ll, lr) = (ll * gl, lr * gr);
        if ll == 0.0 && lr == 0.0 {
            continue;
        }
        let n = b - a;
        let speed = 100.0 / l.stretch;
        let src_n = ((n as f64 * speed).ceil() as usize).max(1);
        let lt = l.layer_time(t_a);
        let buf = match &l.source {
            LayerSource::Footage { item } => match project.item(*item).map(|i| &i.kind) {
                Some(ItemKind::Footage(f)) => footage.audio(*item, f, lt, src_n, rate).unwrap_or_else(|| vec![0.0; src_n * 2]),
                _ => continue,
            },
            LayerSource::Comp { item } => {
                if project.comp_contains(*item, comp_id) {
                    continue;
                }
                let mut sub = vec![0.0f32; src_n * 2];
                mix_into(project, footage, expr, *item, lt, rate, &mut sub, 1.0, 1.0, depth + 1);
                sub
            }
            _ => continue,
        };
        let same = (speed - 1.0).abs() < 1e-9;
        for i in 0..n {
            let j = if same { i } else { ((i as f64 * speed) as usize).min(src_n - 1) };
            if j * 2 + 1 >= buf.len() {
                break;
            }
            out[(a + i) * 2] += buf[j * 2] * ll;
            out[(a + i) * 2 + 1] += buf[j * 2 + 1] * lr;
        }
    }
}

fn levels(project: &Project, comp_id: ItemId, comp: &Comp, l: &Layer, t: Tick, expr: Option<&dyn ExprHost>) -> (f32, f32) {
    let Some(g) = l.props.sub("audio") else { return (1.0, 1.0) };
    let mut ctx = EvalCtx::new(project, comp_id, comp, t);
    ctx.expr = expr;
    let [a, b] = ctx.v2(l, g, "levels", [0.0, 0.0]);
    (db_to_gain(a), db_to_gain(b))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use effectcraft_project::{Footage, FootageKind, build};
    use effectcraft_time::FrameRate;

    use super::*;
    use crate::Image;

    /// A 1 kHz-ish constant "tone": every sample is 0.25 (left) / -0.25 (right).
    struct Dc;
    impl FootageSource for Dc {
        fn frame(&self, _: ItemId, _: &Footage, _: Tick) -> Option<Arc<Image>> {
            None
        }
        fn audio(&self, _: ItemId, _: &Footage, start: Tick, frames: usize, _: u32) -> Option<Vec<f32>> {
            Some((0..frames).flat_map(|_| if start >= Tick::ZERO { [0.25, -0.25] } else { [0.0, 0.0] }).collect())
        }
    }

    #[test]
    fn mixes_layers_with_levels_and_in_out() {
        let mut p = Project::default();
        let f = Footage {
            path: "x.wav".into(),
            kind: FootageKind::Audio,
            width: 0,
            height: 0,
            pixel_aspect: 1.0,
            frame_rate: FrameRate::new(25, 1),
            native_rate: None,
            duration: Tick::from_seconds_f64(10.0),
            has_video: false,
            has_audio: true,
            alpha: Default::default(),
            premul_color: [0.0; 3],
            loop_count: 1,
            codec: String::new(),
            missing: false,
            sequence: vec![],
        };
        let fid = p.add_item("x.wav", effectcraft_color::Label::SeaFoam, None, ItemKind::Footage(f));
        let comp = Comp::new(64, 64, FrameRate::new(25, 1), Tick::from_seconds_f64(2.0));
        let mut l1 = build::layer(&mut p, &comp, "a", LayerSource::Footage { item: fid }, (0, 0), None);
        l1.in_point = Tick::from_seconds_f64(1.0);
        let mut l2 = build::layer(&mut p, &comp, "b", LayerSource::Footage { item: fid }, (0, 0), None);
        if let Some(g) = l2.props.sub_mut("audio")
            && let Some(pr) = g.get_mut("levels")
        {
            pr.value = effectcraft_project::Value::Vec2([-6.0206, -96.0]);
        }
        let mut comp = comp;
        comp.layers = vec![l1, l2];
        let cid = p.add_item("C", effectcraft_color::Label::Sandstone, None, ItemKind::Comp(comp.into()));
        assert!(comp_has_audio(&p, cid));
        let rate = 1000;
        let m = mix_comp(&p, &Dc, None, cid, Tick::from_seconds_f64(0.5), 1000, rate);
        // first half: only layer b (half gain left, muted right)
        assert!((m[0] - 0.125).abs() < 1e-3 && m[1].abs() < 1e-6, "{} {}", m[0], m[1]);
        // second half (≥ 1 s): a + b
        let i = 700 * 2;
        assert!((m[i] - 0.375).abs() < 1e-3 && (m[i + 1] + 0.25).abs() < 1e-3, "{} {}", m[i], m[i + 1]);
    }
}
