//! Modal dialogs: About (with community links), New Composition / Composition Settings, Solid
//! Settings and the command palette (Camera/Light Settings live in `dialogs_3d`).

use egui::{Align2, Color32, Rect, Sense, pos2, vec2};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::{Dialog, EffectcraftApp};

pub use super::comp_settings::CompDraft;

#[derive(Clone, Debug, Default)]
pub struct DialogState {
    pub comp: CompDraft,
    pub editing_existing: bool,
    pub solid_name: String,
    pub solid_color: [f32; 3],
    pub solid_size: [u32; 2],
    pub palette_query: String,
    pub palette_sel: usize,
    pub camera: super::dialogs_3d::CameraDraft,
    pub light: super::dialogs_3d::LightDraft,
    pub velocity: super::key_dialogs::VelocityDraft,
    pub interp: super::key_dialogs::InterpDraft,
    pub stretch: super::key_dialogs::StretchDraft,
}

pub fn open_new_comp(app: &mut EffectcraftApp) {
    let n = app.session.project.comps().count() + 1;
    app.dialog_state.comp = CompDraft { name: format!("Comp {n}"), ..Default::default() };
    app.dialog_state.editing_existing = false;
    app.dialog = Some(Dialog::NewComp);
}

pub fn open_comp_settings(app: &mut EffectcraftApp) -> Result<(), String> {
    let cid = app.session.active_comp_id().ok_or("no composition is open")?;
    let c = app.session.project.comp(cid).ok_or("no composition")?;
    app.dialog_state.comp = CompDraft {
        name: app.session.project.item(cid).map(|i| i.name.clone()).unwrap_or_default(),
        width: c.width,
        height: c.height,
        pixel_aspect: c.pixel_aspect,
        fps: c.frame_rate.as_f64(),
        duration: c.duration.seconds(),
        start: c.display_start.seconds(),
        bg: c.background,
        shutter_angle: c.shutter_angle,
        shutter_phase: c.shutter_phase,
        samples: c.motion_blur_samples,
        advanced_3d: c.renderer == effectcraft_engine::project::Renderer::Advanced3D,
        ..Default::default()
    };
    app.dialog_state.editing_existing = true;
    app.dialog = Some(Dialog::CompSettings);
    Ok(())
}

pub fn open_new_solid(app: &mut EffectcraftApp) -> Result<(), String> {
    let c = app.session.active_comp().ok_or("no composition is open")?;
    let n = app.session.project.items.values().filter(|i| matches!(i.kind, effectcraft_engine::project::ItemKind::Solid(_))).count() + 1;
    app.dialog_state.solid_name = format!("Solid {n}");
    app.dialog_state.solid_size = [c.width, c.height];
    if app.dialog_state.solid_color == [0.0; 3] {
        app.dialog_state.solid_color = [0.2, 0.45, 0.85];
    }
    app.dialog = Some(Dialog::SolidSettings);
    Ok(())
}

pub(crate) fn modal(ctx: &egui::Context, title: &str, size: egui::Vec2, t: &Tokens, body: impl FnOnce(&mut egui::Ui)) {
    // Dim the app.
    let screen = ctx.content_rect();
    ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("modal-dim"))).rect_filled(screen, 0.0, Color32::from_black_alpha(120));
    egui::Area::new(egui::Id::new(("modal", title))).order(egui::Order::Foreground).fixed_pos(screen.center() - size / 2.0).show(ctx, |ui| {
        egui::Frame::window(ui.style()).fill(t.panel_bg).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
            ui.set_width(size.x - 36.0);
            ui.set_min_height(size.y - 36.0);
            ui.label(egui::RichText::new(title).font(Tokens::semibold(15.0)).color(t.tab_text_active));
            ui.add_space(10.0);
            body(ui);
        });
    });
}

pub fn show(app: &mut EffectcraftApp, ctx: &egui::Context) {
    let Some(d) = app.dialog else { return };
    let t = app.tokens;
    match d {
        Dialog::About => about(app, ctx, &t),
        Dialog::NewComp | Dialog::CompSettings => comp_settings(app, ctx, &t, d == Dialog::CompSettings),
        Dialog::SolidSettings => solid(app, ctx, &t),
        Dialog::CommandPalette => palette(app, ctx, &t),
        Dialog::CameraSettings => super::dialogs_3d::camera(app, ctx, &t),
        Dialog::LightSettings => super::dialogs_3d::light(app, ctx, &t),
        Dialog::KeyVelocity => super::key_dialogs::velocity(app, ctx, &t),
        Dialog::KeyInterpolation => super::key_dialogs::interpolation(app, ctx, &t),
        Dialog::TimeStretch => super::key_dialogs::time_stretch(app, ctx, &t),
    }
}

fn about(app: &mut EffectcraftApp, ctx: &egui::Context, t: &Tokens) {
    let mut close = false;
    let mut cmd: Option<&str> = None;
    modal(ctx, "About EffectCraft", vec2(520.0, 400.0), t, |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, 10.0, Color32::from_rgb(0x1b, 0x22, 0x3c));
        crate::header::paint_logo(p, Rect::from_min_size(r.min + vec2(18.0, 20.0), vec2(56.0, 56.0)));
        p.text(r.min + vec2(90.0, 34.0), Align2::LEFT_CENTER, "EffectCraft", Tokens::semibold(24.0), Color32::WHITE);
        p.text(
            r.min + vec2(90.0, 62.0),
            Align2::LEFT_CENTER,
            format!("Version {}  •  Motion graphics & VFX in pure Rust", env!("CARGO_PKG_VERSION")),
            Tokens::ui(12.0),
            t.text_dim,
        );
        ui.add_space(12.0);
        ui.label("A clean-room, open-source compositor for motion graphics and visual effects: native on macOS, Windows and Linux, and in the browser. Part of the ArtCraft family of creative apps.");
        ui.add_space(14.0);
        let links: [(Icon, &str, &str, &str); 4] = [
            (Icon::Chat, "Join the ArtCraft Discord", effectcraft_engine::links::DISCORD, "help.discord"),
            (Icon::Globe, "ArtCraft website", effectcraft_engine::links::WEBSITE, "help.website"),
            (Icon::Sparkle, "EffectCraft home page", effectcraft_engine::links::APP_PAGE, "help.appPage"),
            (Icon::Code, "Source code on GitHub", effectcraft_engine::links::GITHUB, "help.github"),
        ];
        for (icon, label, url, c) in links {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            let p = ui.painter();
            let discord = c == "help.discord";
            p.rect_filled(
                r,
                6.0,
                if discord {
                    Color32::from_rgb(0x58, 0x65, 0xf2)
                } else if resp.hovered() {
                    t.hover
                } else {
                    Color32::from_rgb(0x2b, 0x2b, 0x2b)
                },
            );
            icons::paint(p, Rect::from_center_size(pos2(r.min.x + 18.0, r.center().y), vec2(15.0, 15.0)), icon, Color32::WHITE);
            p.text(pos2(r.min.x + 36.0, r.center().y), Align2::LEFT_CENTER, label, Tokens::medium(12.5), Color32::WHITE);
            p.text(
                pos2(r.max.x - 12.0, r.center().y),
                Align2::RIGHT_CENTER,
                url,
                Tokens::ui(11.0),
                if discord { Color32::from_white_alpha(200) } else { t.text_dim },
            );
            app.auto.add(&format!("about.{c}"), r, label);
            if resp.clicked() {
                cmd = Some(c);
            }
            ui.add_space(4.0);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("MIT OR Apache-2.0 • Fonts: Inter, JetBrains Mono (OFL)").small().color(t.text_faint));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
    });
    if let Some(c) = cmd {
        let _ = app.session.execute(c, json!({}));
    }
    if close {
        app.dialog = None;
    }
}

fn comp_settings(app: &mut EffectcraftApp, ctx: &egui::Context, t: &Tokens, existing: bool) {
    let mut d = app.dialog_state.comp.clone();
    let mut result = None;
    modal(ctx, "Composition Settings", vec2(600.0, 470.0), t, |ui| {
        result = super::comp_settings::show(ui, &mut d, t);
    });
    if result.is_none() && ui_enter(ctx) {
        result = Some(super::comp_settings::params(&d));
    }
    app.dialog_state.comp = d;
    match result {
        Some(serde_json::Value::Null) => app.dialog = None,
        Some(params) => {
            let r = if existing { app.session.execute("comp.settings", params) } else { app.session.execute("comp.new", params) };
            if let Err(e) = r {
                app.ui.status = e.to_string();
            }
            app.dialog = None;
        }
        None => {}
    }
}

pub(crate) fn ui_enter(ctx: &egui::Context) -> bool {
    !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Enter))
}

fn solid(app: &mut EffectcraftApp, ctx: &egui::Context, t: &Tokens) {
    let mut close = false;
    let mut ok = false;
    let mut name = app.dialog_state.solid_name.clone();
    let mut col = app.dialog_state.solid_color;
    let mut size = app.dialog_state.solid_size;
    let comp_size = app.session.active_comp().map(|c| (c.width, c.height)).unwrap_or((1920, 1080));
    let lock_id = egui::Id::new("solid-lock-aspect");
    let mut lock = ctx.data(|d| d.get_temp::<bool>(lock_id)).unwrap_or(true);
    modal(ctx, "Solid Settings", vec2(460.0, 360.0), t, |ui| {
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.add(egui::TextEdit::singleline(&mut name).desired_width(320.0));
        });
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Size").strong());
        egui::Grid::new("solid-grid").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
            let (ow, oh) = (size[0], size[1]);
            ui.label("Width:");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut size[0]).range(1..=30000).suffix(" px"));
                ui.checkbox(&mut lock, format!("Lock Aspect Ratio to {}", super::comp_settings::aspect_label(ow as f64, oh as f64)));
            });
            ui.end_row();
            ui.label("Height:");
            ui.add(egui::DragValue::new(&mut size[1]).range(1..=30000).suffix(" px"));
            ui.end_row();
            if lock && ow > 0 && oh > 0 {
                if size[0] != ow {
                    size[1] = ((size[0] as f64) * oh as f64 / ow as f64).round().max(1.0) as u32;
                } else if size[1] != oh {
                    size[0] = ((size[1] as f64) * ow as f64 / oh as f64).round().max(1.0) as u32;
                }
            }
        });
        let pct = |a: u32, b: u32| 100.0 * a as f64 / b.max(1) as f64;
        ui.label(
            egui::RichText::new(format!(
                "Width: {:.1}% of comp\nHeight: {:.1}% of comp\nFrame Aspect Ratio: {}",
                pct(size[0], comp_size.0),
                pct(size[1], comp_size.1),
                super::comp_settings::aspect_label(size[0] as f64, size[1] as f64)
            ))
            .color(Color32::GRAY),
        );
        ui.add_space(4.0);
        if ui.button("Make Comp Size").clicked() {
            size = [comp_size.0, comp_size.1];
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Color:");
            ui.color_edit_button_rgb(&mut col);
        });
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(egui::RichText::new("   OK   ").color(Color32::WHITE)).fill(t.accent)).clicked() {
                ok = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    ctx.data_mut(|d| d.insert_temp(lock_id, lock));
    app.dialog_state.solid_name = name.clone();
    app.dialog_state.solid_color = col;
    app.dialog_state.solid_size = size;
    if ok || ui_enter(ctx) {
        let _ = app.session.execute("layer.newSolid", json!({"name": name, "color": [col[0], col[1], col[2]], "width": size[0], "height": size[1]}));
        close = true;
    }
    if close {
        app.dialog = None;
    }
}

/// ⌘⇧P: fuzzy search over every command (engine + UI) and effect.
fn palette(app: &mut EffectcraftApp, ctx: &egui::Context, t: &Tokens) {
    let mut q = app.dialog_state.palette_query.clone();
    let mut run: Option<(String, serde_json::Value)> = None;
    let mut close = false;
    let items: Vec<(String, String, serde_json::Value, String)> = {
        let mut v: Vec<(String, String, serde_json::Value, String)> = crate::menus::menu_items(app)
            .into_iter()
            .filter(|m| m.enabled)
            .map(|m| (m.path.join(" ▸ ") + " ▸ " + &m.label, m.id, if m.params.is_null() { json!({}) } else { m.params }, m.shortcut.unwrap_or_default()))
            .collect();
        for c in effectcraft_engine::command_specs().iter().filter(|c| c.menu.is_empty() && c.journal && c.params == "{}") {
            if app.session.is_enabled(c.id) {
                v.push((c.label.to_string(), c.id.to_string(), json!({}), c.shortcut.unwrap_or("").to_string()));
            }
        }
        v
    };
    let ql = q.to_lowercase();
    let matches: Vec<&(String, String, serde_json::Value, String)> = items
        .iter()
        .filter(|(label, id, ..)| {
            let hay = format!("{} {}", label.to_lowercase(), id.to_lowercase());
            ql.split_whitespace().all(|w| hay.contains(w))
        })
        .take(14)
        .collect();
    modal(ctx, "Command Palette", vec2(560.0, 470.0), t, |ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("Type a command or effect…").desired_width(f32::INFINITY).font(Tokens::ui(14.0)));
        r.request_focus();
        ui.add_space(8.0);
        let sel = app.dialog_state.palette_sel.min(matches.len().saturating_sub(1));
        for (i, (label, id, params, sc)) in matches.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
            let p = ui.painter();
            if i == sel || resp.hovered() {
                p.rect_filled(r, 4.0, if i == sel { t.row_selected } else { t.hover });
            }
            p.text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, label, Tokens::ui(12.5), t.text);
            if !sc.is_empty() {
                p.text(pos2(r.max.x - 8.0, r.center().y), Align2::RIGHT_CENTER, crate::menus::shortcut_text(sc), Tokens::ui(11.0), t.text_dim);
            }
            if resp.clicked() {
                run = Some((id.clone(), (*params).clone()));
            }
        }
    });
    let (down, up, enter) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::Enter)));
    if down {
        app.dialog_state.palette_sel = (app.dialog_state.palette_sel + 1).min(matches.len().saturating_sub(1));
    }
    if up {
        app.dialog_state.palette_sel = app.dialog_state.palette_sel.saturating_sub(1);
    }
    if enter && let Some((_, id, params, _)) = matches.get(app.dialog_state.palette_sel) {
        run = Some((id.clone(), (*params).clone()));
    }
    if q != app.dialog_state.palette_query {
        app.dialog_state.palette_sel = 0;
    }
    app.dialog_state.palette_query = q;
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if let Some((id, params)) = run {
        app.dialog = None;
        if let Err(e) = crate::menus::invoke(app, ctx, &id, params) {
            app.ui.status = e;
        }
        return;
    }
    if close {
        app.dialog = None;
    }
}
