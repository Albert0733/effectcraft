//! Effect Controls: the selected layer's effects with all parameters.

use effectcraft_engine::keyframe::Value;
use effectcraft_engine::project::{GroupKind, Layer, Node, ParamUi, PropGroup, Property};
use effectcraft_engine::render::EvalCtx;
use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::{EffectcraftApp, widgets};

type Actions = Vec<(String, serde_json::Value)>;

fn selected_layer(app: &EffectcraftApp) -> Option<Layer> {
    let comp = app.session.active_comp()?;
    let id = app.session.state.selected_layers.first()?;
    comp.layer(*id).cloned()
}

/// One property row: stopwatch, name, value control.
fn prop_row(
    app: &mut EffectcraftApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layer: &Layer,
    prop: &Property,
    ectx: &EvalCtx,
    r: Rect,
    indent: f32,
    actions: &mut Actions,
) {
    let t = app.tokens;
    let cy = r.center().y;
    let uid = prop.uid;
    let swr = Rect::from_center_size(pos2(r.min.x + indent, cy), vec2(14.0, 14.0));
    if !prop.static_only && !matches!(prop.ui, ParamUi::Hidden) {
        let resp = ui.interact(swr, egui::Id::new(("ec-sw", uid)), Sense::click());
        icons::paint(
            p,
            swr,
            Icon::Stopwatch,
            if prop.is_animated() {
                t.hot_text
            } else if resp.hovered() {
                t.text
            } else {
                t.text_dim
            },
        );
        app.auto.add(&format!("effectControls.prop.{uid}.stopwatch"), swr, &prop.name);
        if resp.clicked() {
            actions.push(("prop.toggleAnimation".into(), json!({"layer": layer.id.0, "prop": uid})));
        }
    }
    p.text(pos2(swr.max.x + 6.0, cy), Align2::LEFT_CENTER, &prop.name, Tokens::ui(12.0), t.text);
    let vx = (r.min.x + r.width() * 0.48).max(swr.max.x + 130.0);
    let value = ectx.value(layer, prop);
    let merge = format!("ec-{uid}");
    let set =
        |actions: &mut Actions, v: serde_json::Value| actions.push(("prop.set".into(), json!({"layer": layer.id.0, "prop": uid, "value": v, "merge": merge})));
    match &value {
        Value::Scalar(v) => {
            let (min, max, dec) = match &prop.ui {
                ParamUi::Slider { min, max, decimals, .. } => (*min, *max, *decimals as usize),
                _ => (-1e9, 1e9, 1),
            };
            let suffix = match prop.ui {
                ParamUi::Angle => "°",
                ParamUi::Percent => "%",
                _ => "",
            };
            let speed = match &prop.ui {
                ParamUi::Slider { slider_min, slider_max, .. } => ((slider_max - slider_min) / 300.0).clamp(0.001, 50.0),
                _ => 0.5,
            };
            let (vr, nv, _) = widgets::hot_number_at(
                ui,
                pos2(vx, cy - 9.0),
                egui::Id::new(("ec-v", uid)),
                *v,
                speed,
                (min, max),
                dec.max(if dec == 0 { 0 } else { 1 }),
                suffix,
                &t,
            );
            app.auto.add(&format!("effectControls.prop.{uid}.value"), vr, &prop.name);
            if let Some(nv) = nv {
                set(actions, json!(nv));
            }
            // Slider track for slider params.
            if let ParamUi::Slider { slider_min, slider_max, .. } = prop.ui {
                let tr = Rect::from_min_max(pos2(vr.max.x + 10.0, cy - 2.0), pos2(r.max.x - 12.0, cy + 2.0));
                if tr.width() > 40.0 {
                    p.rect_filled(tr, 2.0, t.field_bg);
                    let f = ((v - slider_min) / (slider_max - slider_min)).clamp(0.0, 1.0) as f32;
                    p.rect_filled(Rect::from_min_max(tr.min, pos2(tr.min.x + tr.width() * f, tr.max.y)), 2.0, t.accent.gamma_multiply(0.7));
                    let knob = pos2(tr.min.x + tr.width() * f, cy);
                    p.circle_filled(knob, 5.0, t.text);
                    let sresp = ui.interact(tr.expand2(vec2(4.0, 6.0)), egui::Id::new(("ec-slider", uid)), Sense::click_and_drag());
                    app.auto.add(&format!("effectControls.prop.{uid}.slider"), tr, &prop.name);
                    if (sresp.dragged() || sresp.clicked())
                        && let Some(pt) = sresp.interact_pointer_pos()
                    {
                        let f = ((pt.x - tr.min.x) / tr.width()).clamp(0.0, 1.0) as f64;
                        set(actions, json!(slider_min + (slider_max - slider_min) * f));
                    }
                }
            }
        }
        Value::Vec2(_) | Value::Vec3(_) => {
            let c = value.components();
            let n = if prop.shown_dims > 0 && !(layer.is_3d() && c.len() == 3) { prop.shown_dims as usize } else { c.len() };
            let mut x = vx;
            if matches!(prop.ui, ParamUi::Point | ParamUi::Point3) {
                icons::paint(p, Rect::from_center_size(pos2(x + 6.0, cy), vec2(12.0, 12.0)), Icon::PanBehind, t.text_dim);
                x += 18.0;
            }
            for d in 0..n.min(c.len()) {
                let (vr, nv, _) = widgets::hot_number_at(ui, pos2(x, cy - 9.0), egui::Id::new(("ec-v", uid, d)), c[d], 1.0, (-1e9, 1e9), 1, "", &t);
                app.auto.add(&format!("effectControls.prop.{uid}.value.{d}"), vr, &prop.name);
                if let Some(nv) = nv {
                    let mut nc = c.clone();
                    nc[d] = nv;
                    set(actions, json!(nc));
                }
                x = vr.max.x + 8.0;
            }
        }
        Value::Color(c) => {
            let sr = Rect::from_min_size(pos2(vx, cy - 8.0), vec2(30.0, 16.0));
            let pop = egui::Id::new(("ec-cpop", uid));
            if widgets::swatch(ui, sr, [c[0] as f32, c[1] as f32, c[2] as f32, 1.0], egui::Id::new(("ec-c", uid)), &t).clicked() {
                widgets::open_popup(ui, pop);
            }
            app.auto.add(&format!("effectControls.prop.{uid}.value"), sr, &prop.name);
            let mut rgb = [c[0] as f32, c[1] as f32, c[2] as f32];
            if crate::header::color_popup(ui, pop, sr.left_bottom(), &mut rgb) {
                set(actions, json!([rgb[0], rgb[1], rgb[2], 1.0]));
            }
            p.text(
                pos2(sr.max.x + 8.0, cy),
                Align2::LEFT_CENTER,
                effectcraft_engine::color::Rgba::from_array([c[0] as f32, c[1] as f32, c[2] as f32, 1.0]).to_hex(),
                Tokens::mono(11.0),
                t.text_dim,
            );
        }
        Value::Bool(b) => {
            let cr = Rect::from_min_size(pos2(vx, cy - 8.0), vec2(16.0, 16.0));
            if widgets::checkbox(ui, cr, *b, &t, egui::Id::new(("ec-b", uid))).clicked() {
                set(actions, json!(!b));
            }
            app.auto.add(&format!("effectControls.prop.{uid}.value"), cr, &prop.name);
        }
        Value::Enum(i) => {
            if let ParamUi::Popup { options } = &prop.ui {
                let dr = Rect::from_min_size(pos2(vx, cy - 9.0), vec2((r.max.x - vx - 12.0).clamp(80.0, 220.0), 18.0));
                let pop = egui::Id::new(("ec-epop", uid));
                if widgets::dropdown(ui, dr, options.get(*i as usize).map(String::as_str).unwrap_or(""), &t, egui::Id::new(("ec-e", uid))).clicked() {
                    widgets::open_popup(ui, pop);
                }
                app.auto.add(&format!("effectControls.prop.{uid}.value"), dr, &prop.name);
                if let Some(ni) = widgets::popup_menu(ui, pop, dr.left_bottom(), options, Some(*i as usize)) {
                    set(actions, json!(ni));
                }
            }
        }
        Value::Layer(l) => {
            let comp = app.session.active_comp().cloned();
            if let Some(comp) = comp {
                let dr = Rect::from_min_size(pos2(vx, cy - 9.0), vec2(160.0, 18.0));
                let name = l.and_then(|id| comp.layer(effectcraft_engine::project::LayerId(id))).map(|l| l.name.clone()).unwrap_or_else(|| "None".into());
                let pop = egui::Id::new(("ec-lpop", uid));
                if widgets::dropdown(ui, dr, &name, &t, egui::Id::new(("ec-l", uid))).clicked() {
                    widgets::open_popup(ui, pop);
                }
                let mut opts = vec!["None".to_string()];
                opts.extend(comp.layers.iter().map(|l| l.name.clone()));
                if let Some(i) = widgets::popup_menu(ui, pop, dr.left_bottom(), &opts, None) {
                    set(actions, if i == 0 { serde_json::Value::Null } else { json!(comp.layers[i - 1].id.0) });
                }
            }
        }
        _ => {}
    }
}

fn group_rows(
    app: &mut EffectcraftApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layer: &Layer,
    g: &PropGroup,
    ectx: &EvalCtx,
    rect: Rect,
    y: &mut f32,
    depth: usize,
    actions: &mut Actions,
) {
    let t = app.tokens;
    for c in &g.children {
        let r = Rect::from_min_size(pos2(rect.min.x, *y), vec2(rect.width(), 24.0));
        match c {
            Node::Prop(pr) => {
                if matches!(pr.ui, ParamUi::Hidden) || pr.three_d_only && !layer.is_3d() {
                    continue;
                }
                *y += 24.0;
                if r.max.y < rect.min.y || r.min.y > rect.max.y {
                    continue;
                }
                prop_row(app, ui, p, layer, pr, ectx, r, 24.0 + 14.0 * depth as f32, actions);
            }
            Node::Group(sg) => {
                *y += 24.0;
                let open = !app.ui.fx_closed.contains(&sg.uid);
                let tw = Rect::from_center_size(pos2(r.min.x + 12.0 + 14.0 * depth as f32, r.center().y), vec2(12.0, 12.0));
                if r.max.y >= rect.min.y && r.min.y <= rect.max.y {
                    if widgets::twirl(ui, tw, open, egui::Id::new(("ec-g", sg.uid)), &t).clicked() {
                        if open {
                            app.ui.fx_closed.insert(sg.uid);
                        } else {
                            app.ui.fx_closed.remove(&sg.uid);
                        }
                    }
                    p.text(pos2(tw.max.x + 6.0, r.center().y), Align2::LEFT_CENTER, &sg.name, Tokens::ui(12.0), t.text);
                }
                if open {
                    group_rows(app, ui, p, layer, sg, ectx, rect, y, depth + 1, actions);
                }
            }
        }
    }
}

pub fn show(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let ctx = ui.ctx().clone();
    let Some(layer) = selected_layer(app) else {
        p.text(rect.center(), Align2::CENTER_CENTER, "Select a layer to see its effects", Tokens::ui(12.0), t.text_faint);
        return;
    };
    let Some(cid) = app.session.active_comp_id() else { return };
    let comp = app.session.project.comp(cid).cloned().unwrap_or_else(|| effectcraft_engine::project::Comp::new(1, 1, Default::default(), Default::default()));
    let comp_name = app.session.project.item(cid).map(|i| i.name.clone()).unwrap_or_default();
    let snap_project = app.session.project.clone();
    let snap_expr = app.session.expr.clone();
    let ectx = EvalCtx { project: &snap_project, comp_id: cid, comp: &comp, time: app.session.time(), expr: snap_expr.as_deref() };
    let hdr = Rect::from_min_size(rect.min, vec2(rect.width(), 24.0));
    p.text(pos2(hdr.min.x + 10.0, hdr.center().y), Align2::LEFT_CENTER, format!("{} • {}", comp_name, layer.name), Tokens::ui(11.5), t.text_dim);
    p.line_segment([hdr.left_bottom(), hdr.right_bottom()], Stroke::new(1.0, t.separator));
    let body = Rect::from_min_max(pos2(rect.min.x, hdr.max.y), rect.max);
    let scroll_id = egui::Id::new("ec-scroll");
    let mut scroll: f32 = ctx.data(|d| d.get_temp(scroll_id).unwrap_or(0.0));
    if ui.rect_contains_pointer(body) {
        scroll = (scroll - ui.input(|i| i.smooth_scroll_delta.y)).max(0.0);
    }
    let mut y = body.min.y + 4.0 - scroll;
    let mut actions: Actions = vec![];
    let bp = p.with_clip_rect(body);
    let fx: Vec<PropGroup> = layer.effects().map(|f| f.groups().cloned().collect()).unwrap_or_default();
    if fx.is_empty() {
        bp.text(
            pos2(body.center().x, body.min.y + 40.0),
            Align2::CENTER_CENTER,
            "No effects. Drag one from Effects & Presets.",
            Tokens::ui(12.0),
            t.text_faint,
        );
    }
    for g in &fx {
        let GroupKind::Effect { .. } = g.kind else { continue };
        let r = Rect::from_min_size(pos2(body.min.x, y), vec2(body.width(), 26.0));
        y += 26.0;
        let open = !app.ui.fx_closed.contains(&g.uid);
        bp.rect_filled(r, 0.0, Color32::from_rgb(0x2a, 0x2a, 0x2a));
        let fxr = Rect::from_center_size(pos2(r.min.x + 14.0, r.center().y), vec2(16.0, 16.0));
        if widgets::icon_toggle(ui, fxr, Icon::Fx, g.enabled, &t, egui::Id::new(("ec-fx", g.uid)), None).clicked() {
            actions.push(("effect.toggle".into(), json!({"layer": layer.id.0, "effect": g.uid})));
        }
        app.auto.add(&format!("effectControls.effect.{}.fx", g.uid), fxr, &g.name);
        let tw = Rect::from_center_size(pos2(r.min.x + 32.0, r.center().y), vec2(12.0, 12.0));
        if widgets::twirl(ui, tw, open, egui::Id::new(("ec-tw", g.uid)), &t).clicked() {
            if open {
                app.ui.fx_closed.insert(g.uid);
            } else {
                app.ui.fx_closed.remove(&g.uid);
            }
        }
        bp.text(pos2(tw.max.x + 6.0, r.center().y), Align2::LEFT_CENTER, &g.name, Tokens::semibold(12.0), t.text);
        let reset = Rect::from_min_size(pos2(r.max.x - 92.0, r.min.y + 4.0), vec2(40.0, 18.0));
        let rresp = ui.interact(reset, egui::Id::new(("ec-reset", g.uid)), Sense::click());
        bp.text(reset.center(), Align2::CENTER_CENTER, "Reset", Tokens::ui(11.5), if rresp.hovered() { t.hot_text } else { t.text_dim });
        app.auto.add(&format!("effectControls.effect.{}.reset", g.uid), reset, "Reset");
        if rresp.clicked()
            && let Some(spec) = g.kind_effect().and_then(effectcraft_engine::effects::find)
        {
            for ps in &spec.params {
                if let Some(pr) = g.get(ps.id) {
                    let size = effectcraft_engine::render::source_size(&app.session.project, &layer);
                    let def = match (&ps.ui, &ps.default) {
                        (ParamUi::Point, Value::Vec2(f)) => Value::Vec2([f[0] * size.0.max(comp.width) as f64, f[1] * size.1.max(comp.height) as f64]),
                        _ => ps.default.clone(),
                    };
                    actions.push(("prop.reset".into(), json!({"layer": layer.id.0, "prop": pr.uid, "default": def.to_json()})));
                }
            }
        }
        let rm = Rect::from_min_size(pos2(r.max.x - 28.0, r.min.y + 4.0), vec2(18.0, 18.0));
        if widgets::icon_button(ui, rm, Icon::Close, false, &t, egui::Id::new(("ec-rm", g.uid))).on_hover_text("Remove effect").clicked() {
            actions.push(("effect.remove".into(), json!({"layer": layer.id.0, "effect": g.uid})));
        }
        app.auto.add(&format!("effectControls.effect.{}.remove", g.uid), rm, "Remove");
        let hresp =
            ui.interact(Rect::from_min_max(pos2(tw.max.x, r.min.y), pos2(reset.min.x - 4.0, r.max.y)), egui::Id::new(("ec-hdr", g.uid)), Sense::click());
        app.auto.add(&format!("effectControls.effect.{}", g.uid), r, &g.name);
        if hresp.clicked() {
            app.session.state.selected_props = vec![(layer.id, g.uid)];
        }
        hresp.context_menu(|ui| {
            for (label, cmd) in [("Duplicate", "effect.duplicate"), ("Remove", "effect.remove")] {
                if ui.button(label).clicked() {
                    actions.push((cmd.into(), json!({"layer": layer.id.0, "effect": g.uid})));
                    ui.close();
                }
            }
            if ui.button("Move Up").clicked() {
                let idx = fx.iter().position(|x| x.uid == g.uid).unwrap_or(0);
                actions.push(("effect.reorder".into(), json!({"layer": layer.id.0, "effect": g.uid, "index": idx.max(1)})));
                ui.close();
            }
            if ui.button("Move Down").clicked() {
                let idx = fx.iter().position(|x| x.uid == g.uid).unwrap_or(0);
                actions.push(("effect.reorder".into(), json!({"layer": layer.id.0, "effect": g.uid, "index": idx + 2})));
                ui.close();
            }
        });
        if open {
            group_rows(app, ui, &bp, &layer, g, &ectx, body, &mut y, 0, &mut actions);
        }
        y += 4.0;
    }
    let content_h = y + scroll - body.min.y;
    scroll = scroll.min((content_h - body.height()).max(0.0));
    ctx.data_mut(|d| d.insert_temp(scroll_id, scroll));
    // Drop effects here.
    if let Some(payload) = egui::DragAndDrop::payload::<crate::panels::DragPayload>(&ctx)
        && ui.rect_contains_pointer(rect)
        && let crate::panels::DragPayload::Effect(e) = payload.as_ref()
    {
        p.rect_stroke(rect, 0.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        if ctx.input(|i| i.pointer.any_released()) {
            actions.push(("effect.apply".into(), json!({"effect": e, "layers": [layer.id.0]})));
            egui::DragAndDrop::clear_payload(&ctx);
        }
    }
    for (id, params) in actions {
        if let Err(e) = crate::menus::invoke(app, &ctx, &id, params) {
            app.ui.status = e;
        }
    }
}

trait EffectKind {
    fn kind_effect(&self) -> Option<&str>;
}
impl EffectKind for PropGroup {
    fn kind_effect(&self) -> Option<&str> {
        match &self.kind {
            GroupKind::Effect { effect } => Some(effect.as_str()),
            _ => None,
        }
    }
}
