//! Vector conversions behind Layer ▸ Create: Create Shapes from Vector Layer (SVG, PDF,
//! Illustrator and EPS footage → shape layer), Create Shapes from Text and Create Masks from
//! Text (glyph outlines), and vector files imported as compositions (one layer per file layer).

use effectcraft_keyframe::{Gradient, ShapePath, Value};
use effectcraft_project::build::{self, Ids};
use effectcraft_project::{Comp, ItemId, Layer, LayerSource, MaskMode, Project, PropGroup};
use effectcraft_svg::{Affine, Doc, Geom, GradientKind, Node, Paint};
use kurbo::{BezPath, Point};

fn shape_paths(ids: &mut Ids, path: &BezPath) -> Vec<PropGroup> {
    effectcraft_path::from_kurbo(path)
        .into_iter()
        .enumerate()
        .map(|(i, sp)| {
            let mut g = build::shape_path(ids, sp);
            g.name = format!("Path {}", i + 1);
            g
        })
        .collect()
}

fn det_scale(m: Affine) -> f64 {
    let c = m.as_coeffs();
    (c[0] * c[3] - c[1] * c[2]).abs().sqrt()
}

/// Axis-aligned scale + translation only (basic shapes stay parametric).
fn axis_aligned(m: Affine) -> Option<(f64, f64, f64, f64)> {
    let c = m.as_coeffs();
    (c[1].abs() < 1e-9 && c[2].abs() < 1e-9 && c[0] > 0.0 && c[3] > 0.0).then_some((c[0], c[3], c[4], c[5]))
}

fn gradient_value(g: &effectcraft_svg::Gradient) -> Gradient {
    Gradient {
        colors: g.stops.iter().map(|(o, c)| (*o, [c[0] as f32, c[1] as f32, c[2] as f32, 1.0])).collect(),
        opacities: g.stops.iter().map(|(o, c)| (*o, c[3] as f32)).collect(),
    }
}

/// Gradient start/end points in shape-layer space (`m`: shape user space → layer space).
fn gradient_points(g: &effectcraft_svg::Gradient, m: Affine, bbox: kurbo::Rect) -> (bool, [f64; 2], [f64; 2]) {
    let unit = if g.bbox_units { Affine::new([bbox.width(), 0.0, 0.0, bbox.height(), bbox.x0, bbox.y0]) } else { Affine::IDENTITY };
    let full = m * unit * g.transform;
    let p = |x: f64, y: f64| {
        let q = full * Point::new(x, y);
        [q.x, q.y]
    };
    match g.kind {
        GradientKind::Linear { x1, y1, x2, y2 } => (false, p(x1, y1), p(x2, y2)),
        GradientKind::Radial { cx, cy, r, .. } => (true, p(cx, cy), p(cx + r, cy)),
    }
}

fn shape_items(ids: &mut Ids, s: &effectcraft_svg::Shape, m: Affine) -> Vec<PropGroup> {
    use kurbo::Shape as _;
    let mut items = vec![];
    // Geometry: parametric when the transform keeps rectangles/ellipses axis-aligned.
    match (&s.geom, axis_aligned(m)) {
        (Geom::Rect { x, y, w, h, rx, ry }, Some((sx, sy, tx, ty))) if (rx - ry).abs() < 1e-9 && (*rx == 0.0 || (sx - sy).abs() < 1e-9) => {
            let (w2, h2) = (w * sx, h * sy);
            let pos = [tx + (x + w / 2.0) * sx, ty + (y + h / 2.0) * sy];
            let mut g = build::shape_rect(ids, [w2, h2], pos, rx * sx);
            g.name = "Rectangle Path 1".into();
            items.push(g);
        }
        (Geom::Ellipse { cx, cy, rx, ry }, Some((sx, sy, tx, ty))) => {
            items.push(build::shape_ellipse(ids, [2.0 * rx * sx, 2.0 * ry * sy], [tx + cx * sx, ty + cy * sy]));
        }
        (g, _) => items.extend(shape_paths(ids, &(m * g.to_path()))),
    }
    let bbox = s.geom.to_path().bounding_box();
    if let Some(st) = &s.stroke {
        let width = st.width * det_scale(m);
        let mut g = match &st.paint {
            Paint::Color(c) => build::shape_stroke(ids, [c[0], c[1], c[2], 1.0], width),
            Paint::Gradient(gr) => {
                let (radial, a, b) = gradient_points(gr, m, bbox);
                build::shape_gradient_stroke(ids, radial, a, b, gradient_value(gr), width)
            }
        };
        set(&mut g, "opacity", Value::Scalar(st.opacity * 100.0));
        set(&mut g, "cap", Value::Enum(st.cap as u32));
        set(&mut g, "join", Value::Enum(st.join as u32));
        set(&mut g, "miter", Value::Scalar(st.miter));
        if let Some((d, off)) = &st.dash
            && let Some(dg) = g.sub_mut("dashes")
        {
            let k = det_scale(m);
            set(dg, "dash", Value::Scalar(d.first().copied().unwrap_or(0.0) * k));
            set(dg, "gap", Value::Scalar(d.get(1).or(d.first()).copied().unwrap_or(0.0) * k));
            set(dg, "offset", Value::Scalar(off * k));
        }
        items.push(g);
    }
    if let Some(f) = &s.fill {
        let mut g = match f {
            Paint::Color(c) => build::shape_fill(ids, [c[0], c[1], c[2], 1.0]),
            Paint::Gradient(gr) => {
                let (radial, a, b) = gradient_points(gr, m, bbox);
                build::shape_gradient_fill(ids, radial, a, b, gradient_value(gr))
            }
        };
        set(&mut g, "opacity", Value::Scalar(s.fill_opacity * 100.0));
        set(&mut g, "rule", Value::Enum(u32::from(s.fill_rule == effectcraft_svg::FillRule::EvenOdd)));
        items.push(g);
    }
    items
}

fn set(g: &mut PropGroup, k: &str, v: Value) {
    if let Some(p) = g.get_mut(k) {
        p.value = v;
    }
}

fn group(ids: &mut Ids, name: &str, items: Vec<PropGroup>, opacity: f64) -> PropGroup {
    let mut g = build::shape_group(ids, name, items);
    if let Some(p) = g.prop_mut("transform/opacity") {
        p.value = Value::Scalar(opacity * 100.0);
    }
    g
}

/// Shape layer contents for an SVG document, in document pixels (transforms are baked into the
/// geometry; group and element opacity go to the group transforms). Top of the paint order first.
pub fn svg_contents(ids: &mut Ids, doc: &Doc) -> Vec<PropGroup> {
    fn walk(ids: &mut Ids, nodes: &[Node], m: Affine) -> Vec<PropGroup> {
        let mut out = vec![];
        for n in nodes.iter().rev() {
            match n {
                Node::Group(g) => {
                    let items = walk(ids, &g.children, m * g.transform);
                    if !items.is_empty() {
                        out.push(group(ids, &g.name, items, g.opacity));
                    }
                }
                Node::Shape(s) => {
                    let items = shape_items(ids, s, m * s.transform);
                    if !items.is_empty() {
                        out.push(group(ids, &s.name, items, s.opacity));
                    }
                }
            }
        }
        out
    }
    let root = &doc.root;
    let items = walk(ids, &root.children, root.transform);
    if (root.opacity - 1.0).abs() > 1e-9 { vec![group(ids, "svg", items, root.opacity)] } else { items }
}

/// Whether footage is a vector file (SVG / PDF / AI / EPS).
pub fn is_vector(f: &effectcraft_project::Footage) -> bool {
    effectcraft_render::is_vector_footage(f) || f.path.to_ascii_lowercase().ends_with(".svg")
}

/// The vector document of an SVG / PDF / AI / EPS file (`None` for other formats), restricted
/// to one layer when `layer` names it.
pub fn vector_doc(path: &str, bytes: &[u8], layer: Option<&effectcraft_project::SourceLayer>) -> Option<Result<Doc, String>> {
    if path.to_ascii_lowercase().ends_with(".svg") || effectcraft_svg::looks_like_svg(bytes) {
        return Some(effectcraft_svg::parse(bytes).map_err(|e| format!("{path}: {e}")));
    }
    effectcraft_pdf::sniff(bytes)?;
    Some(
        effectcraft_pdf::parse(bytes)
            .map(|d| match layer {
                Some(l) => effectcraft_pdf::layer_doc(&d, l.index as usize),
                None => d,
            })
            .map_err(|e| format!("{path}: {e}")),
    )
}

/// Import a PDF / Illustrator / EPS document as a composition the size of its page: one footage
/// item per file layer (each showing only that layer, continuously rasterisable), top layer
/// first. Returns (comp, folder, items).
pub fn import_vector_comp(
    proj: &mut Project,
    path: &str,
    bytes: &[u8],
    name: &str,
    rate: effectcraft_time::FrameRate,
    duration: effectcraft_time::Tick,
) -> Result<(ItemId, ItemId, Vec<ItemId>), String> {
    use effectcraft_color::Label;
    use effectcraft_project::{AlphaMode, Footage, FootageKind, ItemKind, SourceLayer};
    let doc = effectcraft_pdf::parse(bytes).map_err(|e| format!("{path}: {e}"))?;
    let codec = effectcraft_pdf::codec(path, bytes).unwrap_or("PDF");
    let (w, h) = doc.pixel_size();
    let file = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let folder = proj.add_item(&format!("{name} Layers"), Label::Yellow, None, ItemKind::Folder);
    let mut comp = Comp::new(w, h, rate, duration);
    let mut items = vec![];
    let names = effectcraft_pdf::layer_names(&doc);
    for (i, lname) in names.iter().enumerate() {
        let f = Footage {
            path: path.to_string(),
            kind: FootageKind::Still,
            width: w,
            height: h,
            pixel_aspect: 1.0,
            frame_rate: rate,
            has_video: true,
            alpha: AlphaMode::Straight,
            loop_count: 1,
            codec: codec.into(),
            layer: Some(SourceLayer { index: i as u32, name: lname.clone(), layer_size: false, ..Default::default() }),
            ..Default::default()
        };
        let id = proj.add_item(&format!("{lname}/{file}"), Label::Lavender, Some(folder), ItemKind::Footage(f));
        items.push(id);
        let mut l = build::layer(proj, &comp, lname, LayerSource::Footage { item: id }, (w, h), None);
        l.name = comp.unique_layer_name(lname);
        comp.layers.insert(0, l);
    }
    let cid = proj.add_item(name, Label::Sandstone, None, ItemKind::Comp(std::sync::Arc::new(comp)));
    Ok((cid, folder, items))
}

/// A copy of `src`'s transform group with fresh uids.
fn copy_transform(proj: &mut Project, src: &Layer, dst: &mut Layer) {
    let Some(tr) = src.props.sub("transform") else { return };
    let mut tr = tr.clone();
    let mut n = proj.next_id;
    tr.reassign_uids(&mut n);
    proj.next_id = n + 1;
    if let Some(slot) = dst.props.children.iter_mut().find(|c| c.match_id() == "transform") {
        *slot = tr.into();
    }
}

fn like_source(dst: &mut Layer, src: &Layer) {
    dst.start_time = src.start_time;
    dst.in_point = src.in_point;
    dst.out_point = src.out_point;
    dst.stretch = src.stretch;
    dst.parent = src.parent;
    dst.switches.three_d = src.switches.three_d;
}

/// Layer ▸ Create ▸ Create Shapes from Vector Layer: `doc` is the SVG shown by `src`.
pub fn shapes_from_vector(proj: &mut Project, comp: &Comp, src: &Layer, doc: &Doc) -> Layer {
    let mut l = build::layer(proj, comp, &format!("{} Outlines", src.name), LayerSource::Shape, (0, 0), None);
    let items = svg_contents(&mut Ids(&mut proj.next_id), doc);
    if let Some(c) = l.props.sub_mut("contents") {
        c.children.extend(items.into_iter().map(Into::into));
    }
    copy_transform(proj, src, &mut l);
    like_source(&mut l, src);
    l.name = comp.unique_layer_name(&format!("{} Outlines", src.name));
    l
}

/// Layer ▸ Create ▸ Create Shapes from Text: one group per character with its outline, fill and
/// stroke (as laid out at the comp time `ctx` was made for).
pub fn shapes_from_text(proj: &mut Project, comp: &Comp, ctx_project: &Project, cid: ItemId, src: &Layer, time: effectcraft_time::Tick) -> Option<Layer> {
    let ctx = effectcraft_render::EvalCtx::new(ctx_project, cid, comp, time);
    let geom = effectcraft_render::text::text_geom(&ctx, src)?;
    let chars: Vec<char> = geom.doc.text.chars().filter(|c| !c.is_whitespace()).collect();
    let mut l = build::layer(proj, comp, &format!("{} Outlines", src.name), LayerSource::Shape, (0, 0), None);
    let mut groups = vec![];
    {
        let mut ids = Ids(&mut proj.next_id);
        for (i, g) in geom.glyphs.iter().enumerate() {
            let path = g.path();
            if path.elements().is_empty() {
                continue;
            }
            let mut items = shape_paths(&mut ids, &path);
            if g.stroke_width > 0.0 && g.stroke[3] > 0.0 {
                let c = g.stroke;
                items.push(build::shape_stroke(&mut ids, [c[0] as f64, c[1] as f64, c[2] as f64, 1.0], g.stroke_width));
            }
            if g.apply_fill {
                let c = g.fill;
                let mut f = build::shape_fill(&mut ids, [c[0] as f64, c[1] as f64, c[2] as f64, 1.0]);
                set(&mut f, "opacity", Value::Scalar(c[3] as f64 * 100.0));
                items.push(f);
            }
            let name = chars.get(i).map(|c| c.to_string()).unwrap_or_else(|| format!("Glyph {}", i + 1));
            groups.push(group(&mut ids, &name, items, 1.0));
        }
    }
    // First character on top, as in After Effects.
    if let Some(c) = l.props.sub_mut("contents") {
        c.children.extend(groups.into_iter().map(Into::into));
    }
    copy_transform(proj, src, &mut l);
    like_source(&mut l, src);
    l.name = comp.unique_layer_name(&format!("{} Outlines", src.name));
    Some(l)
}

/// Glyph outlines of a text layer in comp space (for Create Masks from Text).
pub fn text_outlines_in_comp(project: &Project, cid: ItemId, comp: &Comp, src: &Layer, time: effectcraft_time::Tick) -> Vec<ShapePath> {
    let ctx = effectcraft_render::EvalCtx::new(project, cid, comp, time);
    let Some(geom) = effectcraft_render::text::text_geom(&ctx, src) else { return vec![] };
    let (m, _) = ctx.layer_to_comp(src);
    let mut out = vec![];
    for g in &geom.glyphs {
        let p = g.path();
        let p = effectcraft_path::transform(std::slice::from_ref(&p), &m).remove(0);
        out.extend(effectcraft_path::from_kurbo(&p));
    }
    out
}

/// A comp-sized white solid with one mask per glyph contour (Difference, so counters stay open).
pub fn masks_layer(proj: &mut Project, comp: &Comp, name: &str, outlines: Vec<ShapePath>, solid: ItemId) -> Layer {
    let mut l = build::layer(proj, comp, name, LayerSource::Solid { item: solid }, (comp.width, comp.height), None);
    let mut ids = Ids(&mut proj.next_id);
    if let Some(masks) = l.props.sub_mut("masks") {
        for (i, sp) in outlines.into_iter().enumerate() {
            masks.children.push(build::mask(&mut ids, &format!("Mask {}", i + 1), sp, MaskMode::Difference, [255, 255, 0]).into());
        }
    }
    l
}
