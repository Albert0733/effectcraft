//! Character, Paragraph and Align panels. While a text layer is edited in the viewer the
//! Character and Paragraph panels show and change the selected text (creating style runs);
//! otherwise they apply to the whole selected text layer.

use effectcraft_engine::keyframe::{BaselineOption, Composer, Direction, Justify, Kerning, TextDoc};
use effectcraft_engine::render::EvalCtx;
use egui::{Align2, Rect, Sense, pos2, vec2};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::{EffectcraftApp, widgets};

/// What the text panels act on: a text layer, a document whose base style and first paragraph
/// show the selection's formatting, and the selected character range while editing.
#[derive(Clone, Debug)]
pub struct TextTarget {
    pub layer: u64,
    pub doc: TextDoc,
    pub range: Option<[usize; 2]>,
}

impl TextTarget {
    /// `layer.setText` params for this target.
    pub fn params(&self, mut v: serde_json::Value) -> serde_json::Value {
        v["layer"] = json!(self.layer);
        if let Some(r) = self.range {
            v["range"] = json!(r);
        }
        v
    }
}

/// The edited text layer's selection, else the selected text layer's Source Text at the CTI.
pub fn text_target(app: &EffectcraftApp) -> Option<TextTarget> {
    let comp = app.session.active_comp()?;
    let cid = app.session.active_comp_id()?;
    if let Some(e) = app.session.state.text_edit.clone()
        && let Some(full) = effectcraft_engine::commands::text_edit::layer_doc(&app.session, e.layer)
    {
        let r = e.range();
        let style = e.pending.clone().unwrap_or_else(|| if r.is_empty() { full.insertion_style(r.start) } else { full.style_at(r.start) });
        let para = full.para(full.para_of(r.start));
        let mut view = full.clone();
        view.runs.clear();
        view.paragraphs.clear();
        view.apply_style_all(|s| *s = style.clone());
        let n = view.para_count();
        view.set_paras(vec![para; n]);
        return Some(TextTarget { layer: e.layer.0, doc: view, range: Some([r.start, r.end]) });
    }
    let layer = app
        .session
        .state
        .selected_layers
        .iter()
        .filter_map(|id| comp.layer(*id))
        .find(|l| matches!(l.source, effectcraft_engine::project::LayerSource::Text))?;
    let ectx = EvalCtx { project: &app.session.project, comp_id: cid, comp, time: app.session.time(), expr: app.session.expr.as_deref() };
    effectcraft_engine::render::text::source_text(&ectx, layer).map(|d| TextTarget { layer: layer.id.0, doc: d, range: None })
}

fn toggle(app: &mut EffectcraftApp, ui: &mut egui::Ui, p: &egui::Painter, r: Rect, id: &str, label: &str, on: bool, bold: bool) -> bool {
    let t = app.tokens;
    let resp = ui.interact(r, egui::Id::new(("text-toggle", id)), Sense::click());
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
    let font = if bold { Tokens::semibold(13.0) } else { Tokens::ui(12.0) };
    p.text(r.center(), Align2::CENTER_CENTER, label, font, if on { egui::Color32::WHITE } else { t.text });
    app.auto.add(id, r, label);
    resp.clicked()
}

pub fn character(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let ctx = ui.ctx().clone();
    let target = text_target(app);
    let enabled = target.is_some();
    let doc = target.as_ref().map(|t| t.doc.clone()).unwrap_or_default();
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
    // Numeric fields in AE's two-column grid (Kerning is a popup, at row 1 right).
    let fields: [(&str, &str, f64, (f64, f64), &str); 10] = [
        ("size", "T", doc.size, (1.0, 2000.0), " px"),
        ("leading", "A", doc.leading.unwrap_or(doc.size * 1.2), (0.0, 5000.0), " px"),
        ("", "", 0.0, (0.0, 0.0), ""),
        ("tracking", "VA", doc.tracking, (-1000.0, 10000.0), ""),
        ("strokeWidth", "W", doc.stroke_width, (0.0, 500.0), " px"),
        ("", "", 0.0, (0.0, 0.0), ""),
        ("vScale", "↕T", doc.v_scale, (1.0, 1000.0), " %"),
        ("hScale", "↔T", doc.h_scale, (1.0, 1000.0), " %"),
        ("baselineShift", "A↑", doc.baseline_shift, (-1000.0, 1000.0), " px"),
        ("tsume", "Ts", doc.tsume, (0.0, 100.0), " %"),
    ];
    for (i, (key, glyph, v, range, suffix)) in fields.into_iter().enumerate() {
        if key.is_empty() {
            continue;
        }
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
            actions.push(json!({key: nv, "merge": format!("char-{key}")}));
        }
    }
    // Kerning: Metrics / Optical / a manual value (row 1, left).
    {
        let fy = y + 28.0;
        p.text(pos2(x0, fy + 9.0), Align2::LEFT_CENTER, "V/A", Tokens::semibold(11.0), t.text_dim);
        let label = match doc.kerning {
            Kerning::Metrics => "Metrics".to_string(),
            Kerning::Optical => "Optical".to_string(),
            Kerning::Manual(v) => format!("{v:.0}"),
        };
        let kr = Rect::from_min_size(pos2(x0 + 26.0, fy), vec2((w / 2.0 - 32.0).max(50.0), 20.0));
        let kpop = egui::Id::new("char-kerning-pop");
        if widgets::dropdown(ui, kr, &label, &t, egui::Id::new("char-kerning")).clicked() && enabled {
            widgets::open_popup(ui, kpop);
        }
        app.auto.add("character.kerning", kr, "Kerning");
        let opts: Vec<String> = ["Metrics", "Optical", "0", "-50", "-25", "-10", "10", "25", "50", "100"].iter().map(|s| s.to_string()).collect();
        if let Some(i) = widgets::popup_menu(ui, kpop, kr.left_bottom(), &opts, None) {
            let v = match i {
                0 => json!("metrics"),
                1 => json!("optical"),
                _ => json!(opts[i].parse::<f64>().unwrap_or(0.0)),
            };
            actions.push(json!({"kerning": v}));
        }
    }
    // Stroke Over Fill / Fill Over Stroke (row 2, right).
    {
        let fx = x0 + w / 2.0;
        let fy = y + 2.0 * 28.0;
        let dr = Rect::from_min_size(pos2(fx, fy), vec2(w / 2.0, 20.0));
        let labels = vec!["Fill Over Stroke".to_string(), "Stroke Over Fill".to_string()];
        let pop = egui::Id::new("char-stroke-order-pop");
        if widgets::dropdown(ui, dr, &labels[doc.stroke_over_fill as usize], &t, egui::Id::new("char-stroke-order")).clicked() && enabled {
            widgets::open_popup(ui, pop);
        }
        app.auto.add("character.strokeOverFill", dr, "Stroke order");
        if let Some(i) = widgets::popup_menu(ui, pop, dr.left_bottom(), &labels, Some(doc.stroke_over_fill as usize)) {
            actions.push(json!({"strokeOverFill": i == 1}));
        }
    }
    y += 5.0 * 28.0 + 8.0;
    // Fill / stroke swatches with their enable boxes.
    for (i, (key, label, on, c)) in [("fill", "Fill", doc.apply_fill, doc.fill), ("stroke", "Stroke", doc.apply_stroke, doc.stroke)].into_iter().enumerate() {
        let fx = x0 + i as f32 * (w / 2.0);
        let cr = Rect::from_min_size(pos2(fx, y + 3.0), vec2(14.0, 14.0));
        if widgets::checkbox(ui, cr, on, &t, egui::Id::new(("char-apply", key))).clicked() && enabled {
            actions.push(json!({if key == "fill" { "applyFill" } else { "applyStroke" }: !on}));
        }
        app.auto.add(&format!("character.{key}.enabled"), cr, label);
        let fs = Rect::from_min_size(pos2(fx + 20.0, y), vec2(26.0, 20.0));
        let pop = egui::Id::new(("char-color-pop", key));
        if widgets::swatch(ui, fs, c, egui::Id::new(("char-color", key)), &t).clicked() && enabled {
            widgets::open_popup(ui, pop);
        }
        app.auto.add(&format!("character.{key}"), fs, label);
        let mut rgb = [c[0], c[1], c[2]];
        if crate::header::color_popup(ui, pop, fs.left_bottom(), &mut rgb) {
            actions.push(json!({key: [rgb[0], rgb[1], rgb[2]], "merge": format!("char-{key}")}));
        }
        p.text(pos2(fs.max.x + 6.0, y + 10.0), Align2::LEFT_CENTER, label, Tokens::ui(11.5), t.text_dim);
    }
    y += 32.0;
    // Style toggles: Faux Bold, Faux Italic, All Caps, Small Caps, Superscript, Subscript.
    let sup = doc.baseline == BaselineOption::Superscript;
    let sub = doc.baseline == BaselineOption::Subscript;
    let toggles = [
        ("fauxBold", "T", doc.faux_bold, true),
        ("fauxItalic", "T", doc.faux_italic, false),
        ("allCaps", "TT", doc.all_caps, false),
        ("smallCaps", "Tт", doc.small_caps, false),
        ("superscript", "T¹", sup, false),
        ("subscript", "T₁", sub, false),
    ];
    let bw = ((w - 5.0 * 4.0) / 6.0).clamp(22.0, 30.0);
    for (i, (key, label, on, bold)) in toggles.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x0 + i as f32 * (bw + 4.0), y), vec2(bw, 24.0));
        if toggle(app, ui, &p, r, &format!("character.{key}"), label, on, bold) && enabled {
            actions.push(json!({key: !on}));
        }
    }
    y += 32.0;
    // Ligatures.
    let lr = Rect::from_min_size(pos2(x0, y), vec2(14.0, 14.0));
    if widgets::checkbox(ui, lr, doc.ligatures, &t, egui::Id::new("char-ligatures")).clicked() && enabled {
        actions.push(json!({"ligatures": !doc.ligatures}));
    }
    app.auto.add("character.ligatures", lr, "Ligatures");
    p.text(pos2(x0 + 20.0, y + 7.0), Align2::LEFT_CENTER, "Ligatures", Tokens::ui(11.5), t.text_dim);
    match &target {
        None => {
            p.text(pos2(rect.center().x, rect.max.y - 20.0), Align2::CENTER_CENTER, "Select a text layer", Tokens::ui(11.0), t.text_faint);
        }
        Some(tt) if tt.range.is_some() => {
            let [a, b] = tt.range.unwrap_or_default();
            let msg = if a == b { "Editing: caret (applies to typed text)".to_string() } else { format!("Editing: {} characters selected", b - a) };
            p.text(pos2(x0, rect.max.y - 14.0), Align2::LEFT_CENTER, msg, Tokens::ui(10.5), t.text_faint);
        }
        _ => {}
    }
    let Some(tt) = target else { return };
    for a in actions {
        if let Err(e) = crate::menus::invoke(app, &ctx, "layer.setText", tt.params(a)) {
            app.ui.status = e;
        }
    }
}

/// The seven Paragraph panel alignment buttons: (justify, `layer.setText` key).
const ALIGNS: [(Justify, &str); 7] = [
    (Justify::Left, "left"),
    (Justify::Center, "center"),
    (Justify::Right, "right"),
    (Justify::JustifyLastLeft, "justifyLeft"),
    (Justify::JustifyLastCenter, "justifyCenter"),
    (Justify::JustifyLastRight, "justifyRight"),
    (Justify::JustifyAll, "justifyAll"),
];

/// Lines glyph of an alignment button.
pub fn paint_justify_glyph(p: &egui::Painter, r: Rect, j: Justify, c: egui::Color32) {
    for k in 0..4 {
        let ly = r.min.y + 6.0 + k as f32 * 4.0;
        let full = r.width() - 10.0;
        let last = k == 3;
        let justified = !matches!(j, Justify::Left | Justify::Center | Justify::Right);
        let lw = if (justified && !last) || j == Justify::JustifyAll {
            full
        } else if k % 2 == 1 || last {
            full * 0.6
        } else {
            full
        };
        let anchor = match j {
            Justify::Center | Justify::JustifyLastCenter => 1,
            Justify::Right | Justify::JustifyLastRight => 2,
            _ => 0,
        };
        let lx = match anchor {
            1 => r.center().x - lw / 2.0,
            2 => r.max.x - 5.0 - lw,
            _ => r.min.x + 5.0,
        };
        p.line_segment([pos2(lx, ly), pos2(lx + lw, ly)], egui::Stroke::new(1.4, c));
    }
}

pub fn paragraph(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let ctx = ui.ctx().clone();
    let target = text_target(app);
    let enabled = target.is_some();
    let doc = target.as_ref().map(|t| t.doc.clone()).unwrap_or_default();
    let x0 = rect.min.x + 10.0;
    let w = rect.width() - 20.0;
    let mut y = rect.min.y + 10.0;
    let mut actions: Vec<serde_json::Value> = vec![];
    let bw = ((w - 6.0 * 3.0) / 7.0).clamp(20.0, 28.0);
    for (i, (j, key)) in ALIGNS.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x0 + i as f32 * (bw + 3.0), y), vec2(bw, 24.0));
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
        paint_justify_glyph(&p, r, j, if on { egui::Color32::WHITE } else { t.text });
        app.auto.add(&format!("paragraph.{key}"), r, key);
        if resp.clicked() && enabled {
            actions.push(json!({"justify": key}));
        }
    }
    y += 34.0;
    // Indents and spacing (AE's two-column grid).
    let fields: [(&str, &str, f64); 6] = [
        ("indentLeft", "→|", doc.indent_left),
        ("indentRight", "|←", doc.indent_right),
        ("indentFirst", "→¶", doc.indent_first),
        ("", "", 0.0),
        ("spaceBefore", "↑¶", doc.space_before),
        ("spaceAfter", "¶↓", doc.space_after),
    ];
    for (i, (key, glyph, v)) in fields.into_iter().enumerate() {
        if key.is_empty() {
            continue;
        }
        let fx = x0 + (i % 2) as f32 * (w / 2.0);
        let fy = y + (i / 2) as f32 * 28.0;
        p.text(pos2(fx, fy + 9.0), Align2::LEFT_CENTER, glyph, Tokens::semibold(11.0), t.text_dim);
        let (r, nv, _) = widgets::hot_number_at(ui, pos2(fx + 26.0, fy), egui::Id::new(("para-f", key)), v, 0.5, (-5000.0, 5000.0), 0, " px", &t);
        app.auto.add(&format!("paragraph.{key}"), r, key);
        if let Some(nv) = nv
            && enabled
        {
            actions.push(json!({key: nv, "merge": format!("para-{key}")}));
        }
    }
    y += 3.0 * 28.0 + 6.0;
    // Direction and composer popups.
    let dr = Rect::from_min_size(pos2(x0, y), vec2(w / 2.0 - 4.0, 22.0));
    let dirs = vec!["Left-to-Right Text".to_string(), "Right-to-Left Text".to_string()];
    let dpop = egui::Id::new("para-direction-pop");
    let rtl = doc.direction == Direction::Rtl;
    if widgets::dropdown(ui, dr, &dirs[rtl as usize], &t, egui::Id::new("para-direction")).clicked() && enabled {
        widgets::open_popup(ui, dpop);
    }
    app.auto.add("paragraph.direction", dr, "Text direction");
    if let Some(i) = widgets::popup_menu(ui, dpop, dr.left_bottom(), &dirs, Some(rtl as usize)) {
        actions.push(json!({"direction": if i == 1 { "rtl" } else { "ltr" }}));
    }
    let cr = Rect::from_min_size(pos2(x0 + w / 2.0, y), vec2(w / 2.0, 22.0));
    let comps = vec!["Every-line Composer".to_string(), "Single-line Composer".to_string()];
    let cpop = egui::Id::new("para-composer-pop");
    let single = doc.composer == Composer::SingleLine;
    if widgets::dropdown(ui, cr, &comps[single as usize], &t, egui::Id::new("para-composer")).clicked() && enabled {
        widgets::open_popup(ui, cpop);
    }
    app.auto.add("paragraph.composer", cr, "Composer");
    if let Some(i) = widgets::popup_menu(ui, cpop, cr.left_bottom(), &comps, Some(single as usize)) {
        actions.push(json!({"composer": if i == 1 { "singleLine" } else { "everyLine" }}));
    }
    y += 30.0;
    let hr = Rect::from_min_size(pos2(x0, y), vec2(14.0, 14.0));
    if widgets::checkbox(ui, hr, doc.hanging_punctuation, &t, egui::Id::new("para-hanging")).clicked() && enabled {
        actions.push(json!({"hangingPunctuation": !doc.hanging_punctuation}));
    }
    app.auto.add("paragraph.hangingPunctuation", hr, "Roman Hanging Punctuation");
    p.text(pos2(x0 + 20.0, y + 7.0), Align2::LEFT_CENTER, "Roman Hanging Punctuation", Tokens::ui(11.5), t.text_dim);
    if !enabled {
        p.text(pos2(rect.center().x, rect.max.y - 20.0), Align2::CENTER_CENTER, "Select a text layer", Tokens::ui(11.0), t.text_faint);
    }
    let Some(tt) = target else { return };
    for a in actions {
        if let Err(e) = crate::menus::invoke(app, &ctx, "layer.setText", tt.params(a)) {
            app.ui.status = e;
        }
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
