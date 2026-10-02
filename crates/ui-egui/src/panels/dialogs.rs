//! Modal dialogs: About (with community links), New Composition / Composition Settings, Solid
//! Settings and the command palette (Camera/Light Settings live in `dialogs_3d`).

use egui::{Align2, Color32, Rect, Sense, pos2, vec2};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::{Dialog, EffectcraftApp};

#[derive(Clone, Debug)]
pub struct CompDraft {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub bg: [f32; 3],
    pub lock_aspect: bool,
}

impl Default for CompDraft {
    fn default() -> Self {
        CompDraft { name: "Comp 1".into(), width: 1920, height: 1080, fps: 29.97, duration: 10.0, bg: [0.0, 0.0, 0.0], lock_aspect: true }
    }
}

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
        fps: c.frame_rate.as_f64(),
        duration: c.duration.seconds(),
        bg: c.background,
        lock_aspect: true,
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
    let mut close = false;
    let mut ok = false;
    let mut d = app.dialog_state.comp.clone();
    modal(ctx, if existing { "Composition Settings" } else { "Composition Settings — New" }, vec2(520.0, 420.0), t, |ui| {
        egui::Grid::new("comp-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
            ui.label("Composition Name");
            ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(260.0));
            ui.end_row();
            ui.label("Preset");
            egui::ComboBox::from_id_salt("comp-preset").selected_text(format!("{}x{} {:.2}", d.width, d.height, d.fps)).show_ui(ui, |ui| {
                for (label, w, h, f) in [
                    ("HD · 1920x1080 · 29.97 fps", 1920, 1080, 29.97),
                    ("HD · 1920x1080 · 25 fps", 1920, 1080, 25.0),
                    ("HD · 1920x1080 · 23.976 fps", 1920, 1080, 23.976),
                    ("HD · 1280x720 · 29.97 fps", 1280, 720, 29.97),
                    ("UHD 4K · 3840x2160 · 29.97 fps", 3840, 2160, 29.97),
                    ("Social 9:16 · 1080x1920 · 30 fps", 1080, 1920, 30.0),
                    ("Social 1:1 · 1080x1080 · 30 fps", 1080, 1080, 30.0),
                    ("Social 4:5 · 1080x1350 · 30 fps", 1080, 1350, 30.0),
                    ("Cinema 4K · 4096x2160 · 24 fps", 4096, 2160, 24.0),
                ] {
                    if ui.selectable_label(false, label).clicked() {
                        d.width = w;
                        d.height = h;
                        d.fps = f;
                    }
                }
            });
            ui.end_row();
            ui.label("Width / Height");
            ui.horizontal(|ui| {
                let ow = d.width;
                ui.add(egui::DragValue::new(&mut d.width).range(4..=30000).suffix(" px"));
                ui.label("×");
                let oh = d.height;
                ui.add(egui::DragValue::new(&mut d.height).range(4..=30000).suffix(" px"));
                ui.checkbox(&mut d.lock_aspect, "Lock aspect");
                if d.lock_aspect && ow != d.width && ow > 0 {
                    d.height = ((d.width as f64) * oh as f64 / ow as f64).round().max(4.0) as u32;
                }
            });
            ui.end_row();
            ui.label("Frame Rate");
            egui::ComboBox::from_id_salt("comp-fps").selected_text(format!("{:.3}", d.fps).trim_end_matches('0').trim_end_matches('.').to_string()).show_ui(
                ui,
                |ui| {
                    for f in [23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 59.94, 60.0, 120.0] {
                        if ui.selectable_label((d.fps - f).abs() < 1e-3, format!("{f}")).clicked() {
                            d.fps = f;
                        }
                    }
                },
            );
            ui.end_row();
            ui.label("Duration");
            ui.add(egui::DragValue::new(&mut d.duration).range(0.04..=86400.0).speed(0.1).suffix(" s"));
            ui.end_row();
            ui.label("Background Color");
            ui.color_edit_button_rgb(&mut d.bg);
            ui.end_row();
        });
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(egui::RichText::new("   OK   ").color(Color32::WHITE)).fill(t.accent)).clicked() {
                    ok = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    });
    if ui_enter(ctx) {
        ok = true;
    }
    app.dialog_state.comp = d.clone();
    if ok {
        let params = json!({"name": d.name, "width": d.width, "height": d.height, "frameRate": d.fps, "duration": d.duration, "background": [d.bg[0], d.bg[1], d.bg[2]]});
        let r = if existing { app.session.execute("comp.settings", params) } else { app.session.execute("comp.new", params) };
        if let Err(e) = r {
            app.ui.status = e.to_string();
        }
        close = true;
    }
    if close {
        app.dialog = None;
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
    modal(ctx, "Solid Settings", vec2(420.0, 300.0), t, |ui| {
        egui::Grid::new("solid-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut name).desired_width(220.0));
            ui.end_row();
            ui.label("Size");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut size[0]).range(1..=30000));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut size[1]).range(1..=30000));
            });
            ui.end_row();
            ui.label("Color");
            ui.color_edit_button_rgb(&mut col);
            ui.end_row();
        });
        ui.add_space(18.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(egui::RichText::new("   OK   ").color(Color32::WHITE)).fill(t.accent)).clicked() {
                ok = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
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
