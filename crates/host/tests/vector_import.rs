//! Photoshop and SVG import through the fully wired session (media decoding included): pixels
//! of imported compositions, SVG footage at any scale, and Create Shapes from Vector Layer.

use effectcraft_engine::render::RenderOpts;
use effectcraft_project::ItemId;
use effectcraft_psd::Rect;
use effectcraft_psd::write::*;
use effectcraft_raster::Image;
use serde_json::json;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("effectcraft-host-vector-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name).to_string_lossy().to_string()
}

fn mean_diff(a: &Image, b: &Image) -> f32 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let mut s = 0.0;
    for (p, q) in a.data.iter().zip(&b.data) {
        for c in 0..4 {
            s += (p[c] - q[c]).abs();
        }
    }
    s / (a.data.len() * 4) as f32
}

fn psd_doc() -> WDoc {
    let mut d = WDoc::new(64, 48);
    let mut masked = WLayer::solid("Masked", Rect::new(30, 8, 24, 24), [0.1, 0.9, 0.3, 1.0]);
    masked.mask = Some(WMask {
        rect: Rect::new(30, 8, 24, 24),
        data: (0..576).map(|i| if (i / 24) < 12 { 1.0 } else { 0.25 }).collect(),
        default_color: 0,
        disabled: false,
    });
    d.layers = vec![
        WLayer::solid("Background", Rect::new(0, 0, 64, 48), [0.2, 0.25, 0.3, 1.0]),
        WLayer::solid("Half Red", Rect::new(4, 4, 30, 20), [1.0, 0.0, 0.0, 1.0]).with_opacity(128),
        masked,
        WLayer::group_end(),
        WLayer::solid("In Group", Rect::new(10, 30, 20, 12), [0.9, 0.9, 0.1, 1.0]),
        WLayer::group("Group", true, *b"norm"),
    ];
    d
}

/// Straight-alpha over of the layers as Photoshop composites them (normal blending), as the
/// merged image to compare the imported composition with.
fn reference(bytes: &[u8]) -> Vec<[f32; 4]> {
    let p = effectcraft_psd::Psd::parse(bytes.to_vec()).unwrap();
    let mut acc = vec![[0.0f32; 4]; (p.width * p.height) as usize];
    for l in p.layers.iter().filter(|l| l.has_pixels() && !l.hidden) {
        let px = p.layer_pixels(l.index, true).unwrap();
        let o = l.opacity as f32 / 255.0;
        for (d, s) in acc.iter_mut().zip(&px.data) {
            let a = s[3] * o;
            let oa = a + d[3] * (1.0 - a);
            for c in 0..3 {
                d[c] = if oa > 0.0 { (s[c] * a + d[c] * d[3] * (1.0 - a)) / oa } else { 0.0 };
            }
            d[3] = oa;
        }
    }
    acc
}

#[test]
fn psd_composition_renders_like_the_document() {
    let mut doc = psd_doc();
    let bytes = write(&doc);
    let merged = reference(&bytes);
    doc.composite = Some(merged.clone());
    let bytes = write(&doc);
    let path = tmp("layers.psd");
    std::fs::write(&path, &bytes).unwrap();
    let mut s = effectcraft_host::session();
    // Footage (merged image).
    let r = s.execute_checked("file.import", json!({"paths": [path]})).unwrap();
    let merged_item = r["items"][0].as_u64().unwrap();
    // Composition and Composition – Retain Layer Sizes.
    let mut renders = vec![];
    for kind in ["composition", "compositionLayerSizes"] {
        let r = s.execute_checked("file.import", json!({"paths": [path], "importAs": kind})).unwrap();
        let cid = ItemId(r["comps"][0].as_u64().unwrap());
        renders.push(s.render(cid, effectcraft_time::Tick::ZERO, RenderOpts::default()));
    }
    // The merged footage in a comp of the same size.
    s.execute("comp.new", json!({"name": "Merged", "width": 64, "height": 48, "frameRate": 30, "duration": 1})).unwrap();
    s.execute("layer.addItem", json!({"item": merged_item})).unwrap();
    let mcid = s.active_comp_id().unwrap();
    let merged_img = s.render(mcid, effectcraft_time::Tick::ZERO, RenderOpts::default());
    let expected = Image { width: 64, height: 48, data: merged.iter().map(|p| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]).collect() };
    assert!(mean_diff(&merged_img, &expected) < 2.0 / 255.0, "merged footage");
    for (i, img) in renders.iter().enumerate() {
        let d = mean_diff(img, &expected);
        assert!(d < 2.0 / 255.0, "import kind {i}: mean diff {d}");
    }
    // A single layer as footage (Choose Layer).
    let r = s.execute_checked("file.import", json!({"paths": [path], "layer": "Masked"})).unwrap();
    let it = s.project.item(ItemId(r["items"][0].as_u64().unwrap())).unwrap();
    assert_eq!(it.name, "Masked/layers.psd");
}

#[test]
fn psd_16bit_cmyk_composition_imports() {
    let mut doc = psd_doc();
    doc.depth = 16;
    doc.mode = effectcraft_psd::ColorMode::Cmyk;
    doc.rle = false;
    let bytes = write(&doc);
    let merged = reference(&bytes);
    let path = tmp("layers16.psd");
    std::fs::write(&path, &bytes).unwrap();
    let mut s = effectcraft_host::session();
    let r = s.execute_checked("file.import", json!({"paths": [path], "importAs": "composition"})).unwrap();
    let cid = ItemId(r["comps"][0].as_u64().unwrap());
    let img = s.render(cid, effectcraft_time::Tick::ZERO, RenderOpts::default());
    let expected = Image { width: 64, height: 48, data: merged.iter().map(|p| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]).collect() };
    let d = mean_diff(&img, &expected);
    assert!(d < 2.0 / 255.0, "mean diff {d}");
}

const SVG: &[u8] = include_bytes!("../../svg/tests/fixtures/basic-shapes.svg");
const SVG2: &[u8] = include_bytes!("../../svg/tests/fixtures/paths-gradients.svg");

#[test]
fn svg_footage_and_shapes_from_vector_layer() {
    for (name, bytes) in [("basic-shapes.svg", SVG), ("paths-gradients.svg", SVG2)] {
        let path = tmp(name);
        std::fs::write(&path, bytes).unwrap();
        let mut s = effectcraft_host::session();
        let r = s.execute_checked("file.import", json!({"paths": [path]})).unwrap();
        assert_eq!(r["errors"], json!([]), "{r}");
        let item = r["items"][0].as_u64().unwrap();
        let doc = effectcraft_svg::parse(bytes).unwrap();
        let (w, h) = doc.pixel_size();
        s.execute("comp.new", json!({"name": "V", "width": w, "height": h, "frameRate": 30, "duration": 1})).unwrap();
        let lid = s.execute_checked("layer.addItem", json!({"item": item})).unwrap()["layer"].as_u64().unwrap();
        let cid = s.active_comp_id().unwrap();
        let t = effectcraft_time::Tick::ZERO;
        let foot = s.render(cid, t, RenderOpts::default());
        let direct = effectcraft_svg::rasterize(&doc, w, h, 1.0);
        assert!(mean_diff(&foot, &direct) < 1.0 / 255.0, "{name}: footage = rasterised SVG");
        s.execute("layer.select", json!({"layers": [lid]})).unwrap();
        let r = s.execute_checked("layer.create", json!({"op": "shapesFromVector"})).unwrap();
        let sid = r["layers"][0].as_u64().unwrap();
        let c = s.active_comp().unwrap();
        assert_eq!(c.layers[0].id.0, sid);
        assert!(!c.layers[1].switches.video);
        let shapes = s.render(cid, t, RenderOpts::default());
        let d = mean_diff(&shapes, &direct);
        assert!(d < 0.02, "{name}: shapes vs SVG mean diff {d}");
        // Scaled up 3×, the shape layer stays sharp and matches the SVG rasterised at 3×.
        s.execute("prop.set", json!({"layer": sid, "path": "transform/scale", "value": [300, 300]})).unwrap();
        s.execute("prop.set", json!({"layer": sid, "path": "transform/position", "value": [0, 0]})).unwrap();
        s.execute("prop.set", json!({"layer": sid, "path": "transform/anchor", "value": [0, 0]})).unwrap();
        let big = s.render(cid, t, RenderOpts::default());
        let direct3 = effectcraft_svg::rasterize(&doc, w, h, 3.0);
        let d3 = mean_diff(&big, &direct3);
        assert!(d3 < 0.03, "{name}: ×3 mean diff {d3}");
    }
}
