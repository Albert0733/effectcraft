//! Character, Paragraph and Align panels.

use effectcraft_engine::keyframe::{Justify, TextDoc};
use effectcraft_engine::render::EvalCtx;
use egui::{Align2, Rect, Sense, pos2, vec2};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::{EffectcraftApp, widgets};

/// The selected text layer's Source Text at the CTI.
fn text_doc(app: &EffectcraftApp) -> Option<(u64, TextDoc)> {
    let comp = app.session.active_comp()?;
    let cid = app.session.active_comp_id()?;
    let layer = app
        .session
        .state
        .selected_layers
        .iter()
        .filter_map(|id| comp.layer(*id))
        .find(|l| matches!(l.source, effectcraft_engine::project::LayerSource::Text))?;
    let ectx = EvalCtx { project: &app.session.project, comp_id: cid, comp, time: app.session.time(), expr: app.session.expr.as_deref() };
    effectcraft_engine::render::text::source_text(&ectx, layer).map(|d| (layer.id.0, d))
}

pub fn character(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let ctx = ui.ctx().clone();
    let (lid, doc) = text_doc(app).unwrap_or((0, TextDoc::default()));
    let enabled = lid != 0;
    let mut y = rect.min.y + 10.0;
    let x0 = rect.min.x + 10.0;
    let w = rect.width() - 20.0;
    let mut actions: Vec<serde_json::Value> = vec![];
    // Font family + style.
    let fr = Rect::from_min_size(pos2(x0, y), vec2(w, 22.0));
    let pop = egui::Id::new("char-font-pop");
    if widgets::dropdown(ui, fr, &doc.font, &t, egui::Id::new("char-font")).clicked() && enabled {
        widgets::open_popup(ui, pop);
    }
    app.auto.add("character.font", fr, "Font family");
    let fams: Vec<String> = effectcraft_engine::text_families();
    if let Some(i) = widgets::popup_menu(ui, pop, fr.left_bottom(), &fams, fams.iter().position(|f| *f == doc.font)) {
        actions.push(json!({"font": fams[i]}));
    }
    y += 28.0;
    let sr = Rect::from_min_size(pos2(x0, y), vec2(w, 22.0));
    let spop = egui::Id::new("char-style-pop");
    if widgets::dropdown(ui, sr, &doc.style, &t, egui::Id::new("char-style")).clicked() && enabled {
        widgets::open_popup(ui, spop);
    }
    app.auto.add("character.style", sr, "Font style");
    let styles: Vec<String> = ["Regular", "Medium", "SemiBold", "Bold", "Italic"].iter().map(|s| s.to_string()).collect();
    if let Some(i) = widgets::popup_menu(ui, spop, sr.left_bottom(), &styles, styles.iter().position(|s| *s == doc.style)) {
        actions.push(json!({"style": styles[i]}));
    }
    y += 34.0;
    // Numeric fields in a 2-column grid.
    let fields: [(&str, &str, f64, (f64, f64), &str); 6] = [
        ("size", "T", doc.size, (1.0, 2000.0), " px"),
        ("leading", "A", doc.leading.unwrap_or(doc.size * 1.2), (0.0, 5000.0), " px"),
        ("tracking", "VA", doc.tracking, (-1000.0, 10000.0), ""),
        ("strokeWidth", "≡", doc.stroke_width, (0.0, 500.0), " px"),
        ("baselineShift", "A↑", doc.baseline_shift, (-1000.0, 1000.0), " px"),
        ("hScale", "↔", doc.h_scale, (1.0, 1000.0), "%"),
    ];
    for (i, (key, glyph, v, range, suffix)) in fields.into_iter().enumerate() {
        let col = i % 2;
        let row = i / 2;
        let fx = x0 + col as f32 * (w / 2.0);
        let fy = y + row as f32 * 28.0;
        p.text(pos2(fx, fy + 9.0), Align2::LEFT_CENTER, glyph, Tokens::semibold(11.0), t.text_dim);
        let (r, nv, _) = widgets::hot_number_at(ui, pos2(fx + 26.0, fy), egui::Id::new(("char", key)), v, 0.5, range, 0, suffix, &t);
        app.auto.add(&format!("character.{key}"), r, key);
        if let Some(nv) = nv
            && enabled
        {
            let k = if key == "baselineShift" || key == "hScale" { "tracking" } else { key };
            if k == key {
                actions.push(json!({key: nv, "merge": format!("char-{key}")}));
            }
        }
    }
    y += 3.0 * 28.0 + 8.0;
    // Fill / stroke swatches.
    p.text(pos2(x0, y + 10.0), Align2::LEFT_CENTER, "Fill", Tokens::ui(11.5), t.text_dim);
    let fs = Rect::from_min_size(pos2(x0 + 34.0, y), vec2(26.0, 20.0));
    if widgets::swatch(ui, fs, doc.fill, egui::Id::new("char-fill"), &t).clicked() && enabled {
        widgets::open_popup(ui, egui::Id::new("char-fill-pop"));
    }
    app.auto.add("character.fill", fs, "Fill color");
    let mut fc = [doc.fill[0], doc.fill[1], doc.fill[2]];
    if crate::header::color_popup(ui, egui::Id::new("char-fill-pop"), fs.left_bottom(), &mut fc) {
        actions.push(json!({"fill": [fc[0], fc[1], fc[2]], "merge": "char-fill"}));
    }
    p.text(pos2(x0 + 80.0, y + 10.0), Align2::LEFT_CENTER, "Stroke", Tokens::ui(11.5), t.text_dim);
    let ss = Rect::from_min_size(pos2(x0 + 126.0, y), vec2(26.0, 20.0));
    if widgets::swatch(ui, ss, doc.stroke, egui::Id::new("char-stroke"), &t).clicked() && enabled {
        widgets::open_popup(ui, egui::Id::new("char-stroke-pop"));
    }
    app.auto.add("character.stroke", ss, "Stroke color");
    let mut sc = [doc.stroke[0], doc.stroke[1], doc.stroke[2]];
    if crate::header::color_popup(ui, egui::Id::new("char-stroke-pop"), ss.left_bottom(), &mut sc) {
        actions.push(json!({"stroke": [sc[0], sc[1], sc[2]], "merge": "char-stroke"}));
    }
    y += 32.0;
    // Style toggles.
    let toggles = [("fauxBold", "T", doc.faux_bold), ("fauxItalic", "T", doc.faux_italic), ("allCaps", "TT", doc.all_caps)];
    for (i, (key, label, on)) in toggles.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x0 + i as f32 * 34.0, y), vec2(30.0, 24.0));
        let resp = ui.interact(r, egui::Id::new(("char-t", key)), Sense::click());
        p.rect_filled(
            r,
            3.0,
            if on {
                t.accent
            } else if resp.hovered() {
                t.hover
            } else {
                t.field_bg
            },
        );
        let font = if key == "fauxBold" { Tokens::semibold(13.0) } else { Tokens::ui(12.0) };
        p.text(r.center(), Align2::CENTER_CENTER, label, font, if on { egui::Color32::WHITE } else { t.text });
        app.auto.add(&format!("character.{key}"), r, key);
        if resp.clicked() && enabled {
            actions.push(json!({key: !on}));
        }
    }
    if !enabled {
        p.text(pos2(rect.center().x, rect.max.y - 20.0), Align2::CENTER_CENTER, "Select a text layer", Tokens::ui(11.0), t.text_faint);
    }
    for mut a in actions {
        a["layer"] = json!(lid);
        if let Err(e) = crate::menus::invoke(app, &ctx, "layer.setText", a) {
            app.ui.status = e;
        }
    }
}

pub fn paragraph(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let ctx = ui.ctx().clone();
    let (lid, doc) = text_doc(app).unwrap_or((0, TextDoc::default()));
    let x0 = rect.min.x + 10.0;
    let y = rect.min.y + 10.0;
    let opts = [
        (Justify::Left, "left"),
        (Justify::Center, "center"),
        (Justify::Right, "right"),
        (Justify::JustifyLastLeft, "justify"),
        (Justify::JustifyAll, "justifyAll"),
    ];
    for (i, (j, key)) in opts.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x0 + i as f32 * 32.0, y), vec2(28.0, 26.0));
        let on = doc.justify == j;
        let resp = ui.interact(r, egui::Id::new(("para", key)), Sense::click());
        p.rect_filled(
            r,
            3.0,
            if on {
                t.accent
            } else if resp.hovered() {
                t.hover
            } else {
                t.field_bg
            },
        );
        let c = if on { egui::Color32::WHITE } else { t.text };
        for k in 0..4 {
            let ly = r.min.y + 7.0 + k as f32 * 4.0;
            let full = r.width() - 12.0;
            let lw = if k % 2 == 1 && !matches!(j, Justify::JustifyAll) { full * 0.65 } else { full };
            let lx = match j {
                Justify::Center => r.center().x - lw / 2.0,
                Justify::Right => r.max.x - 6.0 - lw,
                _ => r.min.x + 6.0,
            };
            p.line_segment([pos2(lx, ly), pos2(lx + lw, ly)], egui::Stroke::new(1.4, c));
        }
        app.auto.add(&format!("paragraph.{key}"), r, key);
        if resp.clicked() && lid != 0 {
            let _ = crate::menus::invoke(app, &ctx, "layer.setText", json!({"layer": lid, "justify": key}));
        }
    }
    if lid == 0 {
        p.text(pos2(rect.center().x, rect.max.y - 20.0), Align2::CENTER_CENTER, "Select a text layer", Tokens::ui(11.0), t.text_faint);
    }
}

/// Align panel: align selected layers to the composition (or to the selection).
pub fn align(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let x0 = rect.min.x + 10.0;
    let y = rect.min.y + 10.0;
    p.text(pos2(x0, y + 8.0), Align2::LEFT_CENTER, "Align Layers to: Composition", Tokens::ui(11.5), t.text_dim);
    let ops = ["left", "hcenter", "right", "top", "vcenter", "bottom"];
    for (i, op) in ops.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x0 + i as f32 * 34.0 + if i >= 3 { 12.0 } else { 0.0 }, y + 24.0), vec2(30.0, 28.0));
        let resp = ui.interact(r, egui::Id::new(("align", *op)), Sense::click());
        p.rect_filled(r, 3.0, if resp.hovered() { t.hover } else { t.field_bg });
        // Glyph: a bar (alignment edge) and two boxes.
        let c = r.center();
        let st = egui::Stroke::new(1.3, t.text);
        match *op {
            "left" | "hcenter" | "right" => {
                let ex = match *op {
                    "left" => r.min.x + 7.0,
                    "right" => r.max.x - 7.0,
                    _ => c.x,
                };
                p.line_segment([pos2(ex, r.min.y + 5.0), pos2(ex, r.max.y - 5.0)], st);
                let off = |w: f32| match *op {
                    "left" => ex,
                    "right" => ex - w,
                    _ => ex - w / 2.0,
                };
                p.rect_filled(Rect::from_min_size(pos2(off(14.0), c.y - 7.0), vec2(14.0, 5.0)), 1.0, t.text_dim);
                p.rect_filled(Rect::from_min_size(pos2(off(9.0), c.y + 2.0), vec2(9.0, 5.0)), 1.0, t.text_dim);
            }
            _ => {
                let ey = match *op {
                    "top" => r.min.y + 6.0,
                    "bottom" => r.max.y - 6.0,
                    _ => c.y,
                };
                p.line_segment([pos2(r.min.x + 5.0, ey), pos2(r.max.x - 5.0, ey)], st);
                let off = |h: f32| match *op {
                    "top" => ey,
                    "bottom" => ey - h,
                    _ => ey - h / 2.0,
                };
                p.rect_filled(Rect::from_min_size(pos2(c.x - 8.0, off(14.0)), vec2(5.0, 14.0)), 1.0, t.text_dim);
                p.rect_filled(Rect::from_min_size(pos2(c.x + 3.0, off(9.0)), vec2(5.0, 9.0)), 1.0, t.text_dim);
            }
        }
        app.auto.add(&format!("align.{op}"), r, op);
        if resp.clicked() {
            align_layers(app, op);
        }
    }
    let _ = icons::paint;
    let _ = Icon::Grid;
}

/// Move selected layers so their content bounds align to the comp.
pub fn align_layers(app: &mut EffectcraftApp, op: &str) {
    let Some(cid) = app.session.active_comp_id() else { return };
    let Some(comp) = app.session.project.comp(cid).cloned() else { return };
    let time = app.session.time();
    let mut moves = vec![];
    {
        let ectx = EvalCtx { project: &app.session.project, comp_id: cid, comp: &comp, time, expr: app.session.expr.as_deref() };
        for l in comp.layers.iter().filter(|l| app.session.state.selected_layers.contains(&l.id)) {
            let Some(b) = effectcraft_engine::render::content_bounds(&ectx, l) else { continue };
            let (m, _) = ectx.layer_to_comp(l);
            let pts = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|p| m.apply(effectcraft_engine::geom::vec2(p[0], p[1])));
            let (x0, x1) = (pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min), pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max));
            let (y0, y1) = (pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min), pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max));
            let (cw, ch) = (comp.width as f64, comp.height as f64);
            let (dx, dy) = match op {
                "left" => (-x0, 0.0),
                "right" => (cw - x1, 0.0),
                "hcenter" => (cw / 2.0 - (x0 + x1) / 2.0, 0.0),
                "top" => (0.0, -y0),
                "bottom" => (0.0, ch - y1),
                _ => (0.0, ch / 2.0 - (y0 + y1) / 2.0),
            };
            let pos = l.transform().map(|tr| ectx.v3(l, tr, "position", [0.0; 3])).unwrap_or([0.0; 3]);
            moves.push((l.id.0, [pos[0] + dx, pos[1] + dy, pos[2]]));
        }
    }
    for (id, v) in moves {
        let _ = app.session.execute("prop.set", json!({"layer": id, "path": "transform/position", "value": v}));
    }
}
