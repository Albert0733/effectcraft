//! Effects & Presets: search + category tree; double-click applies to the selected layers, drag
//! onto a layer (timeline, viewer, Effect Controls) applies there.

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::panels::DragPayload;
use crate::theme::Tokens;
use crate::{EffectcraftApp, widgets};

pub fn show(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    let sr = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(rect.width() - 16.0, 24.0));
    let mut q = app.ui.effects_search.clone();
    widgets::search_field(ui, sr, &mut q, "Search effects", &t);
    app.ui.effects_search = q.clone();
    app.auto.add("effects.search", sr, "Search");
    let list = Rect::from_min_max(pos2(rect.min.x, sr.max.y + 6.0), rect.max);
    let lp = p.with_clip_rect(list);
    let scroll_id = egui::Id::new("fx-scroll");
    let mut scroll: f32 = ctx.data(|d| d.get_temp(scroll_id).unwrap_or(0.0));
    if ui.rect_contains_pointer(list) {
        scroll = (scroll - ui.input(|i| i.smooth_scroll_delta.y)).max(0.0);
    }
    let query = q.to_lowercase();
    let mut y = list.min.y - scroll;
    let mut apply: Option<String> = None;
    let row_h = 21.0;
    // "* Animation Presets" (our own, original presets come later) then categories.
    let mut cats: Vec<&str> = effectcraft_engine::effects::CATEGORIES.to_vec();
    cats.retain(|c| effectcraft_engine::effects::registry().iter().any(|e| e.category == *c));
    for cat in cats {
        let items: Vec<_> = effectcraft_engine::effects::registry()
            .iter()
            .filter(|e| e.category == cat && (query.is_empty() || e.name.to_lowercase().contains(&query)))
            .collect();
        if items.is_empty() {
            continue;
        }
        let open = !query.is_empty() || app.ui.effects_open.contains(cat);
        let r = Rect::from_min_size(pos2(list.min.x, y), vec2(list.width(), row_h));
        y += row_h;
        let tw = Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(12.0, 12.0));
        let resp = ui.interact(r.intersect(list), egui::Id::new(("fxcat", cat)), Sense::click());
        if resp.hovered() {
            lp.rect_filled(r, 0.0, t.hover);
        }
        icons::paint(&lp, tw, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.text_dim);
        icons::paint(&lp, Rect::from_center_size(pos2(r.min.x + 28.0, r.center().y), vec2(13.0, 13.0)), Icon::Folder, Color32::from_rgb(0xc8, 0xb0, 0x58));
        lp.text(pos2(r.min.x + 40.0, r.center().y), Align2::LEFT_CENTER, cat, Tokens::ui(12.0), t.text);
        app.auto.add(&format!("effects.category.{cat}"), r, cat);
        if resp.clicked() && query.is_empty() {
            if open {
                app.ui.effects_open.remove(cat);
            } else {
                app.ui.effects_open.insert(cat.to_string());
            }
        }
        if !open {
            continue;
        }
        for e in items {
            let r = Rect::from_min_size(pos2(list.min.x, y), vec2(list.width(), row_h));
            y += row_h;
            if r.max.y < list.min.y || r.min.y > list.max.y {
                continue;
            }
            let resp = ui.interact(r.intersect(list), egui::Id::new(("fx", e.id)), Sense::click_and_drag());
            if resp.hovered() {
                lp.rect_filled(r, 0.0, t.hover);
            }
            icons::paint(&lp, Rect::from_center_size(pos2(r.min.x + 44.0, r.center().y), vec2(12.0, 12.0)), Icon::Fx, t.text_dim);
            lp.text(pos2(r.min.x + 56.0, r.center().y), Align2::LEFT_CENTER, e.name, Tokens::ui(12.0), t.text);
            if e.float {
                lp.text(pos2(r.max.x - 10.0, r.center().y), Align2::RIGHT_CENTER, "32", Tokens::ui(9.5), t.text_faint);
            }
            app.auto.add(&format!("effects.item.{}", e.id), r, e.name);
            if resp.double_clicked() {
                apply = Some(e.id.to_string());
            }
            if resp.drag_started() {
                egui::DragAndDrop::set_payload(&ctx, DragPayload::Effect(e.id.to_string()));
            }
        }
    }
    let content = y + scroll - list.min.y;
    scroll = scroll.min((content - list.height()).max(0.0));
    ctx.data_mut(|d| d.insert_temp(scroll_id, scroll));
    if let Some(DragPayload::Effect(id)) = egui::DragAndDrop::payload::<DragPayload>(&ctx).as_deref()
        && let Some(pos) = ctx.pointer_hover_pos()
    {
        let name = effectcraft_engine::effects::find(id).map(|e| e.name).unwrap_or("");
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("dnd-fx")));
        let g = painter.layout_no_wrap(name.to_string(), Tokens::ui(12.0), t.text);
        let r = Rect::from_min_size(pos + vec2(12.0, 8.0), g.size() + vec2(16.0, 8.0));
        painter.rect_filled(r, 4.0, Color32::from_rgba_premultiplied(40, 40, 40, 230));
        painter.rect_stroke(r, 4.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        painter.galley(r.min + vec2(8.0, 4.0), g, t.text);
    }
    if let Some(id) = apply {
        if app.session.state.selected_layers.is_empty() {
            app.ui.status = "Select a layer to apply the effect to".into();
        } else if let Err(e) = app.session.execute("effect.apply", json!({"effect": id})) {
            app.ui.status = e.to_string();
        } else {
            app.show_panel(crate::dock::PanelKind::EffectControls);
            app.ui.focused = crate::dock::PanelKind::EffectsPresets;
        }
    }
}
