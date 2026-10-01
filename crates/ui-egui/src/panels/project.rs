//! The Project panel: item preview header, search, item list with columns, folders, and the
//! bottom bar (new folder, new comp, bit depth, delete).

use effectcraft_engine::project::{Item, ItemId, ItemKind};
use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::panels::DragPayload;
use crate::theme::Tokens;
use crate::{EffectcraftApp, widgets};

fn item_icon(it: &Item) -> Icon {
    match &it.kind {
        ItemKind::Folder => Icon::Folder,
        ItemKind::Comp(_) => Icon::Comp,
        ItemKind::Solid(_) => Icon::Solid,
        ItemKind::Footage(f) => match f.kind {
            effectcraft_engine::project::FootageKind::Still | effectcraft_engine::project::FootageKind::Sequence => Icon::Image,
            effectcraft_engine::project::FootageKind::Audio => Icon::Audio,
            _ => Icon::Footage,
        },
    }
}

fn fmt_dur(secs: f64, fps: f64) -> String {
    let f = (secs * fps).round() as i64;
    let fpsi = fps.round().max(1.0) as i64;
    format!("0;{:02};{:02};{:02}", f / fpsi / 60, (f / fpsi) % 60, f % fpsi)
}

pub fn show(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().clone();
    let ctx = ui.ctx().clone();
    // Preview header.
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), 74.0));
    let sel = app.session.state.project_selection.first().copied();
    if let Some(it) = sel.and_then(|i| app.session.project.item(i)).cloned() {
        let thumb = Rect::from_min_size(head.min + vec2(10.0, 10.0), vec2(96.0, 54.0));
        p.rect_filled(thumb, 3.0, Color32::from_rgb(0x15, 0x15, 0x15));
        match &it.kind {
            ItemKind::Solid(s) => {
                p.rect_filled(thumb.shrink(4.0), 2.0, Color32::from_rgb((s.color[0] * 255.0) as u8, (s.color[1] * 255.0) as u8, (s.color[2] * 255.0) as u8));
            }
            ItemKind::Comp(_) => {
                if let Some((tex, k)) = &app.viewer_tex
                    && k.comp == it.id.0
                {
                    p.image(tex.id(), thumb, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                } else {
                    icons::paint(&p, Rect::from_center_size(thumb.center(), vec2(24.0, 24.0)), Icon::Comp, t.text_dim);
                }
            }
            _ => icons::paint(&p, Rect::from_center_size(thumb.center(), vec2(24.0, 24.0)), item_icon(&it), t.text_dim),
        }
        let tx = thumb.max.x + 10.0;
        p.text(pos2(tx, thumb.min.y + 6.0), Align2::LEFT_CENTER, &it.name, Tokens::semibold(12.0), t.text);
        let mut lines = vec![];
        if let Some((w, h)) = it.dimensions() {
            lines.push(format!("{w} x {h} (1.00)"));
        }
        if let Some(d) = it.duration() {
            let fps = it.frame_rate().map(|r| r.as_f64()).unwrap_or(30.0);
            lines.push(format!("Δ {}, {:.2} fps", fmt_dur(d.seconds(), fps), fps));
        }
        if let ItemKind::Footage(f) = &it.kind {
            lines.push(f.codec.clone());
        }
        for (i, l) in lines.iter().enumerate() {
            p.text(pos2(tx, thumb.min.y + 22.0 + 13.0 * i as f32), Align2::LEFT_CENTER, l, Tokens::ui(11.0), t.text_dim);
        }
    } else {
        p.text(head.center(), Align2::CENTER_CENTER, "Select an item to see its details", Tokens::ui(11.0), t.text_faint);
    }
    // Search.
    let sr = Rect::from_min_size(pos2(rect.min.x + 8.0, head.max.y + 2.0), vec2(rect.width() - 16.0, 22.0));
    let mut q = app.ui.project_search.clone();
    widgets::search_field(ui, sr, &mut q, "Search project", &t);
    app.ui.project_search = q;
    app.auto.add("project.search", sr, "Search");
    // Column header.
    let hdr = Rect::from_min_size(pos2(rect.min.x, sr.max.y + 6.0), vec2(rect.width(), 20.0));
    p.line_segment([hdr.left_bottom(), hdr.right_bottom()], Stroke::new(1.0, t.separator));
    let name_w = (rect.width() * 0.48).max(140.0);
    let col_type = rect.min.x + name_w + 30.0;
    let col_size = col_type + 76.0;
    p.text(pos2(rect.min.x + 26.0, hdr.center().y), Align2::LEFT_CENTER, "Name", Tokens::ui(11.0), t.text_dim);
    icons::paint(&p, Rect::from_center_size(pos2(rect.min.x + name_w + 14.0, hdr.center().y), vec2(10.0, 10.0)), Icon::Keyframe, t.text_dim);
    p.text(pos2(col_type, hdr.center().y), Align2::LEFT_CENTER, "Type", Tokens::ui(11.0), t.text_dim);
    p.text(pos2(col_size, hdr.center().y), Align2::LEFT_CENTER, "Size", Tokens::ui(11.0), t.text_dim);
    // Rows.
    let footer_h = 28.0;
    let list = Rect::from_min_max(pos2(rect.min.x, hdr.max.y), pos2(rect.max.x, rect.max.y - footer_h));
    let lp = p.with_clip_rect(list);
    let mut y = list.min.y;
    let query = app.ui.project_search.to_lowercase();
    let mut rows: Vec<(ItemId, usize)> = vec![];
    fn walk(app: &EffectcraftApp, folder: Option<ItemId>, depth: usize, q: &str, out: &mut Vec<(ItemId, usize)>) {
        for it in app.session.project.children(folder) {
            if !q.is_empty() && !it.is_folder() && !it.name.to_lowercase().contains(q) {
                continue;
            }
            out.push((it.id, depth));
            if it.is_folder() && (app.ui.project_open_folders.contains(&it.id.0) || !q.is_empty()) {
                walk(app, Some(it.id), depth + 1, q, out);
            }
        }
    }
    walk(app, None, 0, &query, &mut rows);
    let mut actions: Vec<(String, serde_json::Value)> = vec![];
    for (i, (id, depth)) in rows.iter().enumerate() {
        let Some(it) = app.session.project.item(*id).cloned() else { continue };
        let r = Rect::from_min_size(pos2(list.min.x, y), vec2(list.width(), 22.0));
        y += 22.0;
        if r.min.y > list.max.y {
            break;
        }
        let selected = app.session.state.project_selection.contains(id);
        lp.rect_filled(
            r,
            0.0,
            if selected {
                t.row_selected
            } else if i % 2 == 0 {
                t.row
            } else {
                t.row_alt
            },
        );
        let x0 = r.min.x + 8.0 + 14.0 * *depth as f32;
        if it.is_folder() {
            let open = app.ui.project_open_folders.contains(&id.0);
            let tw = Rect::from_center_size(pos2(x0 + 4.0, r.center().y), vec2(12.0, 12.0));
            if widgets::twirl(ui, tw, open, egui::Id::new(("pf", id.0)), &t).clicked() {
                if open {
                    app.ui.project_open_folders.remove(&id.0);
                } else {
                    app.ui.project_open_folders.insert(id.0);
                }
            }
        }
        icons::paint(
            &lp,
            Rect::from_center_size(pos2(x0 + 18.0, r.center().y), vec2(14.0, 14.0)),
            item_icon(&it),
            if it.is_folder() { Color32::from_rgb(0xd8, 0xc0, 0x60) } else { t.text_dim },
        );
        lp.text(pos2(x0 + 30.0, r.center().y), Align2::LEFT_CENTER, &it.name, Tokens::ui(12.0), if selected { Color32::WHITE } else { t.text });
        lp.rect_filled(Rect::from_center_size(pos2(r.min.x + name_w + 14.0, r.center().y), vec2(10.0, 10.0)), 2.0, Tokens::label(it.label));
        lp.text(pos2(col_type, r.center().y), Align2::LEFT_CENTER, it.type_name(), Tokens::ui(11.5), t.text_dim);
        if let Some((w, h)) = it.dimensions() {
            lp.text(pos2(col_size, r.center().y), Align2::LEFT_CENTER, format!("{w} x {h}"), Tokens::ui(11.5), t.text_dim);
        }
        let resp = ui.interact(r.intersect(list), egui::Id::new(("pitem", id.0)), Sense::click_and_drag());
        app.auto.add(&format!("project.item.{}", id.0), r, &it.name);
        if resp.clicked() {
            let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
            if add {
                if !app.session.state.project_selection.contains(id) {
                    app.session.state.project_selection.push(*id);
                }
            } else {
                app.session.state.project_selection = vec![*id];
            }
        }
        if resp.double_clicked() {
            match &it.kind {
                ItemKind::Comp(_) => actions.push(("comp.open".into(), json!({"comp": id.0}))),
                ItemKind::Folder => {
                    if !app.ui.project_open_folders.remove(&id.0) {
                        app.ui.project_open_folders.insert(id.0);
                    }
                }
                _ => {}
            }
        }
        if resp.drag_started() && !it.is_folder() {
            egui::DragAndDrop::set_payload(&ctx, DragPayload::Item(id.0));
        }
        resp.context_menu(|ui| {
            if matches!(it.kind, ItemKind::Comp(_)) && ui.button("Open Composition").clicked() {
                actions.push(("comp.open".into(), json!({"comp": id.0})));
                ui.close();
            }
            if !it.is_folder() && ui.button("Add to Composition").clicked() {
                actions.push(("layer.addItem".into(), json!({"item": id.0})));
                ui.close();
            }
            if ui.button("New Comp from Selection").clicked() {
                if let Some((w, h)) = it.dimensions() {
                    let d = it.duration().map(|d| d.seconds()).unwrap_or(10.0);
                    actions.push(("comp.new".into(), json!({"name": format!("{} Comp", it.name), "width": w, "height": h, "duration": d})));
                    actions.push(("layer.addItem".into(), json!({"item": id.0})));
                }
                ui.close();
            }
        });
    }
    // Drag ghost.
    if let Some(DragPayload::Item(iid)) = egui::DragAndDrop::payload::<DragPayload>(&ctx).as_deref()
        && let Some(pos) = ctx.pointer_hover_pos()
        && let Some(it) = app.session.project.item(ItemId(*iid))
    {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("dnd-ghost")));
        let g = painter.layout_no_wrap(it.name.clone(), Tokens::ui(12.0), t.text);
        let r = Rect::from_min_size(pos + vec2(12.0, 8.0), g.size() + vec2(16.0, 8.0));
        painter.rect_filled(r, 4.0, Color32::from_rgba_premultiplied(40, 40, 40, 230));
        painter.galley(r.min + vec2(8.0, 4.0), g, t.text);
    }
    // Footer.
    let foot = Rect::from_min_max(pos2(rect.min.x, rect.max.y - footer_h), rect.max);
    p.line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0, t.separator));
    let mut x = foot.min.x + 8.0;
    for (icon, id, tip) in [
        (Icon::Gear, "interpret", "Interpret Footage"),
        (Icon::NewFolder, "newFolder", "Create a new Folder"),
        (Icon::NewComp, "newComp", "Create a new Composition"),
    ] {
        let r = Rect::from_min_size(pos2(x, foot.min.y + 3.0), vec2(22.0, 22.0));
        let resp = widgets::icon_button(ui, r, icon, false, &t, egui::Id::new(("pfoot", id))).on_hover_text(tip);
        app.auto.add(&format!("project.{id}"), r, tip);
        if resp.clicked() {
            match id {
                "newComp" => crate::panels::dialogs::open_new_comp(app),
                "newFolder" => {
                    let _ = app.session.edit("New Folder", None, |proj, st| {
                        let id = proj.add_item("Untitled Folder", effectcraft_engine::color::Label::Yellow, None, ItemKind::Folder);
                        st.project_selection = vec![id];
                        Ok(())
                    });
                }
                _ => app.ui.status = "Interpret Footage: select a footage item".into(),
            }
        }
        x += 26.0;
    }
    let depth = app.session.project.settings.bit_depth.label();
    let dr = Rect::from_min_size(pos2(x + 4.0, foot.min.y + 5.0), vec2(48.0, 18.0));
    let dresp = ui.interact(dr, egui::Id::new("bpc"), Sense::click());
    p.text(dr.center(), Align2::CENTER_CENTER, depth, Tokens::ui(11.5), if dresp.hovered() { t.text } else { t.text_dim });
    app.auto.add("project.bitDepth", dr, depth);
    if dresp.clicked() {
        actions.push(("file.cycleBitDepth".into(), json!({})));
    }
    let tr = Rect::from_min_size(pos2(foot.max.x - 30.0, foot.min.y + 3.0), vec2(22.0, 22.0));
    if widgets::icon_button(ui, tr, Icon::Trash, false, &t, egui::Id::new("ptrash")).on_hover_text("Delete selected project items").clicked() {
        let sel = app.session.state.project_selection.clone();
        let _ = app.session.edit("Delete Items", None, |proj, st| {
            for id in &sel {
                proj.items.remove(id);
            }
            st.project_selection.clear();
            Ok(())
        });
        app.session.sanitize_state();
    }
    app.auto.add("project.delete", tr, "Delete");
    for (id, params) in actions {
        if let Err(e) = crate::menus::invoke(app, &ctx, &id, params) {
            app.ui.status = e;
        }
    }
}
