//! The demo project: an animated title sequence built entirely from procedural content (original
//! work, MIT OR Apache-2.0): gradient background, orbiting shape rings with trim paths and a
//! repeater burst, an animated title with a text animator and glow, a lower-third precomp.

use effectcraft_color::BlendMode;
use effectcraft_color::Label;
use effectcraft_keyframe::{Ease, Gradient, Interp, Justify, Keyframe, TextDoc, Value};
use effectcraft_project::build::{self, Ids};
use effectcraft_project::{Comp, ItemId, ItemKind, Layer, LayerSource, Project, PropGroup, Solid};
use effectcraft_time::{FrameRate, Tick};

pub const MAIN_COMP: &str = "EffectCraft Intro";

fn t(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

fn hex(h: &str) -> [f64; 4] {
    let c = effectcraft_color::Rgba::from_hex(h).unwrap_or(effectcraft_color::Rgba::WHITE);
    [c.r as f64, c.g as f64, c.b as f64, 1.0]
}

/// Eased keyframes (Easy Ease on every key).
fn keys(list: &[(f64, Value)]) -> Vec<Keyframe> {
    list.iter().map(|(s, v)| Keyframe::new(t(*s), v.clone()).eased()).collect()
}

/// Keyframes with a strong "expo out" ease: fast start, long settle.
fn keys_out(list: &[(f64, Value)]) -> Vec<Keyframe> {
    let mut k: Vec<Keyframe> = list.iter().map(|(s, v)| Keyframe::new(t(*s), v.clone())).collect();
    let n = k.len();
    for (i, key) in k.iter_mut().enumerate() {
        let d = key.value.dims().max(1);
        if i + 1 < n {
            key.out_interp = Interp::Bezier;
            key.out_ease = vec![Ease { speed: 0.0, influence: 0.05 }; d];
        }
        if i > 0 {
            key.in_interp = Interp::Bezier;
            key.in_ease = vec![Ease { speed: 0.0, influence: 0.85 }; d];
        }
    }
    k
}

fn set(l: &mut Layer, path: &str, v: Value) {
    if let Some(p) = l.props.prop_mut(path) {
        p.value = v;
    }
}
fn anim(l: &mut Layer, path: &str, k: Vec<Keyframe>) {
    if let Some(p) = l.props.prop_mut(path) {
        p.keys = k;
    }
}

fn effect(p: &mut Project, l: &mut Layer, id: &str, size: [f64; 2], params: &[(&str, Value)]) {
    let Some(spec) = effectcraft_effects::find(id) else { return };
    let mut next = p.next_id;
    let mut g = effectcraft_effects::instantiate(spec, &mut Ids(&mut next), spec.name, size);
    p.next_id = next;
    for (k, v) in params {
        if let Some(pr) = g.get_mut(k) {
            pr.value = v.clone();
        }
    }
    if let Some(fx) = l.props.sub_mut("effects") {
        fx.children.push(g.into());
    }
}

fn contents(l: &mut Layer, items: Vec<PropGroup>) {
    if let Some(c) = l.props.sub_mut("contents") {
        for g in items {
            c.children.push(g.into());
        }
    }
}

fn text_layer(p: &mut Project, comp: &Comp, name: &str, doc: TextDoc, pos: [f64; 2]) -> Layer {
    let mut l = build::layer(p, comp, name, LayerSource::Text, (comp.width, comp.height), None);
    set(&mut l, "text/sourceText", Value::Text(Box::new(doc)));
    set(&mut l, "transform/position", Value::Vec3([pos[0], pos[1], 0.0]));
    l
}

fn solid_layer(p: &mut Project, comp: &Comp, folder: ItemId, name: &str, color: [f32; 3]) -> Layer {
    let sid = p.add_item(name, Label::Red, Some(folder), ItemKind::Solid(Solid { color, width: comp.width, height: comp.height, pixel_aspect: 1.0 }));
    build::layer(p, comp, name, LayerSource::Solid { item: sid }, (comp.width, comp.height), None)
}

fn lower_third(p: &mut Project) -> Comp {
    let mut c = Comp::new(1920, 1080, FrameRate::FPS_29_97, t(4.0));
    let mut bar = build::layer(p, &c, "Bar", LayerSource::Shape, (1920, 1080), None);
    let mut next = p.next_id;
    {
        let mut ids = Ids(&mut next);
        let rect = build::shape_rect(&mut ids, [560.0, 96.0], [0.0, 0.0], 14.0);
        let gfill = build::shape_gradient_fill(
            &mut ids,
            false,
            [-280.0, 0.0],
            [280.0, 0.0],
            Gradient { colors: vec![(0.0, [0.18, 0.55, 0.92, 1.0]), (1.0, [0.55, 0.32, 0.98, 1.0])], opacities: vec![(0.0, 1.0), (1.0, 1.0)] },
        );
        let g = build::shape_group(&mut ids, "Bar", vec![rect, gfill]);
        contents(&mut bar, vec![g]);
    }
    p.next_id = next;
    set(&mut bar, "transform/position", Value::Vec3([440.0, 900.0, 0.0]));
    anim(&mut bar, "transform/scale", keys_out(&[(0.0, Value::Vec3([0.0, 100.0, 100.0])), (0.7, Value::Vec3([100.0, 100.0, 100.0]))]));
    let mut title = text_layer(
        p,
        &c,
        "Built with EffectCraft",
        TextDoc { text: "Built with EffectCraft".into(), size: 40.0, style: "SemiBold".into(), justify: Justify::Center, ..Default::default() },
        [440.0, 914.0],
    );
    anim(&mut title, "transform/opacity", keys(&[(0.4, Value::Scalar(0.0)), (0.9, Value::Scalar(100.0))]));
    c.layers = vec![title, bar];
    c
}

/// Build the demo project.
pub fn demo_project() -> Project {
    let mut p = Project::default();
    let mut comp = Comp::new(1920, 1080, FrameRate::FPS_29_97, t(10.0));
    comp.background = [0.03, 0.035, 0.07];
    let solids = p.add_item("Solids", Label::Yellow, None, ItemKind::Folder);
    let precomps = p.add_item("Precomps", Label::Yellow, None, ItemKind::Folder);
    let (cw, ch) = (1920.0, 1080.0);
    let size = [cw, ch];

    // Lower third precomp.
    let lt = lower_third(&mut p);
    let lt_id = p.add_item("Lower Third", Label::Sandstone, Some(precomps), ItemKind::Comp(lt.into()));

    // Background: radial gradient.
    let mut bg = solid_layer(&mut p, &comp, solids, "Background", [0.0, 0.0, 0.0]);
    effect(
        &mut p,
        &mut bg,
        "ec.generate.gradientramp",
        size,
        &[
            ("start", Value::Vec2([960.0, 470.0])),
            ("end", Value::Vec2([960.0, 1500.0])),
            ("startColor", Value::Color(hex("#26306E"))),
            ("endColor", Value::Color(hex("#05060D"))),
            ("shape", Value::Enum(1)),
        ],
    );

    // Controller null drives the rings' rotation.
    let mut null = build::layer(&mut p, &comp, "Controller", LayerSource::Null, (100, 100), None);
    set(&mut null, "transform/anchor", Value::Vec3([50.0, 50.0, 0.0]));
    set(&mut null, "transform/position", Value::Vec3([960.0, 540.0, 0.0]));
    anim(&mut null, "transform/rotation", vec![Keyframe::new(t(0.0), Value::Scalar(-30.0)), Keyframe::new(t(10.0), Value::Scalar(60.0))]);
    let null_id = null.id;

    // Orbit rings with trim-path write-on.
    let mut rings = build::layer(&mut p, &comp, "Orbit Rings", LayerSource::Shape, (1920, 1080), None);
    let mut next = p.next_id;
    {
        let mut ids = Ids(&mut next);
        let e1 = build::shape_ellipse(&mut ids, [720.0, 720.0], [0.0, 0.0]);
        let s1 = build::shape_stroke(&mut ids, hex("#3D8FF5"), 4.0);
        let tr1 = build::shape_trim(&mut ids, 0.0, 0.0, 0.0);
        let g1 = build::shape_group(&mut ids, "Inner Ring", vec![e1, tr1, s1]);
        let e2 = build::shape_ellipse(&mut ids, [860.0, 860.0], [0.0, 0.0]);
        let mut s2 = build::shape_stroke(&mut ids, hex("#8E6BFF"), 2.0);
        if let Some(d) = s2.sub_mut("dashes") {
            if let Some(x) = d.get_mut("dash") {
                x.value = Value::Scalar(18.0);
            }
            if let Some(x) = d.get_mut("gap") {
                x.value = Value::Scalar(14.0);
            }
        }
        if let Some(x) = s2.get_mut("cap") {
            x.value = Value::Enum(1);
        }
        let tr2 = build::shape_trim(&mut ids, 0.0, 0.0, 0.0);
        let g2 = build::shape_group(&mut ids, "Outer Ring", vec![e2, tr2, s2]);
        contents(&mut rings, vec![g1, g2]);
    }
    p.next_id = next;
    set(&mut rings, "transform/position", Value::Vec3([50.0, 50.0, 0.0]));
    rings.parent = Some(null_id);
    anim(&mut rings, "contents/group#1/contents/trim/end", keys(&[(0.2, Value::Scalar(0.0)), (1.8, Value::Scalar(100.0))]));
    anim(&mut rings, "contents/group#2/contents/trim/end", keys(&[(0.5, Value::Scalar(0.0)), (2.3, Value::Scalar(100.0))]));
    anim(&mut rings, "contents/group#2/contents/trim/offset", vec![Keyframe::new(t(0.0), Value::Scalar(0.0)), Keyframe::new(t(10.0), Value::Scalar(-240.0))]);

    // Tick burst: a repeater of rounded bars around the centre.
    let mut burst = build::layer(&mut p, &comp, "Tick Burst", LayerSource::Shape, (1920, 1080), None);
    let mut next = p.next_id;
    {
        let mut ids = Ids(&mut next);
        let r = build::shape_rect(&mut ids, [6.0, 36.0], [0.0, -470.0], 3.0);
        let f = build::shape_fill(&mut ids, hex("#E8EEFF"));
        let mut rep = build::shape_repeater(&mut ids, 60.0, [0.0, 0.0]);
        if let Some(tr) = rep.sub_mut("transform") {
            if let Some(x) = tr.get_mut("rotation") {
                x.value = Value::Scalar(6.0);
            }
            if let Some(x) = tr.get_mut("endOpacity") {
                x.value = Value::Scalar(8.0);
            }
        }
        let g = build::shape_group(&mut ids, "Ticks", vec![r, f, rep]);
        contents(&mut burst, vec![g]);
    }
    p.next_id = next;
    anim(&mut burst, "transform/scale", keys_out(&[(0.0, Value::Vec3([60.0, 60.0, 100.0])), (1.6, Value::Vec3([100.0, 100.0, 100.0]))]));
    anim(&mut burst, "transform/rotation", vec![Keyframe::new(t(0.0), Value::Scalar(0.0)), Keyframe::new(t(10.0), Value::Scalar(-45.0))]);
    anim(&mut burst, "transform/opacity", keys(&[(0.0, Value::Scalar(0.0)), (1.0, Value::Scalar(55.0))]));
    burst.blend_mode = BlendMode::Add;

    // Accent line under the title.
    let mut line = build::layer(&mut p, &comp, "Accent Line", LayerSource::Shape, (1920, 1080), None);
    let mut next = p.next_id;
    {
        let mut ids = Ids(&mut next);
        let r = build::shape_rect(&mut ids, [640.0, 5.0], [0.0, 0.0], 2.5);
        let gf = build::shape_gradient_fill(
            &mut ids,
            false,
            [-320.0, 0.0],
            [320.0, 0.0],
            Gradient {
                colors: vec![(0.0, [0.24, 0.56, 0.96, 1.0]), (1.0, [0.56, 0.42, 1.0, 1.0])],
                opacities: vec![(0.0, 0.0), (0.2, 1.0), (0.8, 1.0), (1.0, 0.0)],
            },
        );
        let g = build::shape_group(&mut ids, "Line", vec![r, gf]);
        contents(&mut line, vec![g]);
    }
    p.next_id = next;
    set(&mut line, "transform/position", Value::Vec3([960.0, 600.0, 0.0]));
    anim(&mut line, "transform/scale", keys_out(&[(1.1, Value::Vec3([0.0, 100.0, 100.0])), (2.2, Value::Vec3([100.0, 100.0, 100.0]))]));

    // Title with a per-character rise + fade animator.
    let mut title = text_layer(
        &mut p,
        &comp,
        "EFFECTCRAFT",
        TextDoc { text: "EFFECTCRAFT".into(), size: 148.0, style: "Bold".into(), tracking: 120.0, justify: Justify::Center, ..Default::default() },
        [960.0, 560.0],
    );
    let mut next = p.next_id;
    {
        let mut ids = Ids(&mut next);
        let pos = build::text_anim_prop(&mut ids, "position").map(|mut pr| {
            pr.value = Value::Vec2([0.0, 90.0]);
            pr
        });
        let op = build::text_anim_prop(&mut ids, "opacity").map(|mut pr| {
            pr.value = Value::Scalar(0.0);
            pr
        });
        let sc = build::text_anim_prop(&mut ids, "scale").map(|mut pr| {
            pr.value = Value::Vec2([60.0, 60.0]);
            pr
        });
        let mut a = build::text_animator(&mut ids, "Animator 1", [pos, op, sc].into_iter().flatten().collect());
        if let Some(sel) = a.group_mut("selectors/#1")
            && let Some(adv) = sel.sub_mut("advanced")
            && let Some(sh) = adv.get_mut("shape")
        {
            sh.value = Value::Enum(1); // ramp up
        }
        if let Some(anims) = title.props.group_mut("text/animators") {
            anims.children.push(a.into());
        }
    }
    p.next_id = next;
    anim(&mut title, "text/animators/#1/selectors/#1/start", keys(&[(0.6, Value::Scalar(0.0)), (2.2, Value::Scalar(100.0))]));
    effect(
        &mut p,
        &mut title,
        "ec.stylize.glow",
        size,
        &[("threshold", Value::Scalar(40.0)), ("radius", Value::Scalar(36.0)), ("intensity", Value::Scalar(0.9))],
    );

    // Subtitle.
    let mut sub = text_layer(
        &mut p,
        &comp,
        "Tagline",
        TextDoc {
            text: "MOTION GRAPHICS  ·  VISUAL EFFECTS  ·  PURE RUST".into(),
            size: 30.0,
            style: "Medium".into(),
            tracking: 260.0,
            fill: [0.68, 0.75, 1.0, 1.0],
            justify: Justify::Center,
            ..Default::default()
        },
        [960.0, 666.0],
    );
    anim(&mut sub, "transform/opacity", keys(&[(1.8, Value::Scalar(0.0)), (2.6, Value::Scalar(100.0))]));
    anim(&mut sub, "transform/position", keys(&[(1.8, Value::Vec3([960.0, 690.0, 0.0])), (2.6, Value::Vec3([960.0, 666.0, 0.0]))]));

    // Lower third precomp in at 5s.
    let ltc = p.comp(lt_id).cloned();
    let mut lower = build::layer(&mut p, &comp, "Lower Third", LayerSource::Comp { item: lt_id }, (1920, 1080), ltc.map(|c| c.duration));
    lower.start_time = t(5.0);
    lower.in_point = t(5.0);
    lower.out_point = t(9.5);

    // Stack: top first.
    comp.layers = vec![lower, title, sub, line, burst, rings, null, bg];
    for l in &mut comp.layers {
        if l.name == "Controller" {
            l.switches.video = false;
        }
    }
    comp.work_area = (Tick::ZERO, t(10.0));
    p.add_item(MAIN_COMP, Label::Sandstone, None, ItemKind::Comp(comp.into()));
    p.fix_next_id();
    p
}
