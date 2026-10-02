//! Region of interest: a ROI render equals the matching crop of the full render.

use effectcraft_color::{BlendMode, Label};
use effectcraft_keyframe::Value;
use effectcraft_project::build;
use effectcraft_project::{Comp, ItemKind, LayerSource, Project, Solid};
use effectcraft_time::{FrameRate, Tick};

use crate::{Image, NoFootage, RenderOpts, Renderer};

fn project(three_d: bool) -> (Project, effectcraft_project::ItemId) {
    let mut p = Project::default();
    let comp = Comp::new(160, 120, FrameRate::FPS_30, Tick::from_seconds_f64(1.0));
    let cid = p.add_item("Comp", Label::Sandstone, None, ItemKind::Comp(comp.clone().into()));
    let mut layers = vec![];
    for (i, (c, w, h, pos, rot)) in
        [([1.0, 0.2, 0.1], 90, 50, [60.0, 50.0], 17.0), ([0.1, 0.4, 1.0], 70, 70, [100.0, 75.0], -30.0), ([0.9, 0.9, 0.2], 40, 30, [30.0, 95.0], 0.0)]
            .into_iter()
            .enumerate()
    {
        let sid = p.add_item("Solid", Label::Red, None, ItemKind::Solid(Solid { color: c, width: w, height: h, pixel_aspect: 1.0 }));
        let mut l = build::layer(&mut p, &comp, "Solid", LayerSource::Solid { item: sid }, (w, h), None);
        l.props.prop_mut("transform/position").unwrap().value = Value::Vec3([pos[0], pos[1], 0.0]);
        l.props.prop_mut("transform/rotation").unwrap().value = Value::Scalar(rot);
        l.props.prop_mut("transform/opacity").unwrap().value = Value::Scalar(80.0);
        if i == 1 {
            l.blend_mode = BlendMode::Screen;
            l.switches.three_d = three_d;
        }
        layers.push(l);
    }
    p.comp_mut(cid).unwrap().layers = layers;
    (p, cid)
}

fn crop(img: &Image, x: u32, y: u32, w: u32, h: u32) -> Image {
    let mut out = Image::new(w, h);
    for yy in 0..h {
        for xx in 0..w {
            out.data[(yy * w + xx) as usize] = img.data[((yy + y) * img.width + xx + x) as usize];
        }
    }
    out
}

fn assert_same(a: &Image, b: &Image) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    for (i, (p, q)) in a.data.iter().zip(&b.data).enumerate() {
        for c in 0..4 {
            assert!((p[c] - q[c]).abs() < 1e-4, "pixel {i} channel {c}: {} vs {}", p[c], q[c]);
        }
    }
}

#[test]
fn roi_render_equals_cropped_full_render() {
    for three_d in [false, true] {
        let (p, cid) = project(three_d);
        let full = Renderer::new(&p, &NoFootage, RenderOpts::default()).comp_frame(cid, Tick::ZERO);
        let roi = Renderer::new(&p, &NoFootage, RenderOpts { roi: Some([30.0, 20.0, 80.0, 60.0]), ..Default::default() }).comp_frame(cid, Tick::ZERO);
        assert_eq!((roi.width, roi.height), (80, 60));
        assert_same(&roi, &crop(&full, 30, 20, 80, 60));
    }
}

#[test]
fn roi_at_half_resolution() {
    let (p, cid) = project(false);
    let full = Renderer::new(&p, &NoFootage, RenderOpts { scale: 0.5, ..Default::default() }).comp_frame(cid, Tick::ZERO);
    let roi = Renderer::new(&p, &NoFootage, RenderOpts { scale: 0.5, roi: Some([40.0, 20.0, 60.0, 40.0]), ..Default::default() }).comp_frame(cid, Tick::ZERO);
    assert_eq!((roi.width, roi.height), (30, 20));
    assert_same(&roi, &crop(&full, 20, 10, 30, 20));
}
