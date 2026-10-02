//! The Settings dialog (After Effects' Preferences): the page list on the left, the page on the
//! right, OK / Cancel / Previous / Next. Pages are built from the engine's schema
//! (`effectcraft_engine::prefs::pages`), so the dialog, `prefs.pages` and the docs agree. Edits
//! apply live; Cancel restores the settings from when the dialog opened, OK saves them.
//!
//! Automation ids: `settings.page.<id>`, `settings.<key>` (e.g. `settings.general.undoLevels`),
//! `settings.labels.<n>.name` / `.color`, `settings.button.<label>`, `settings.ok`,
//! `settings.cancel`, `settings.previous`, `settings.next`.

use effectcraft_engine::prefs::{Item, Kind, Page, pages};
use egui::{Color32, RichText, vec2};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{Dialog, EffectcraftApp};

/// Open the dialog on a page, remembering the settings for Cancel.
pub fn open(app: &mut EffectcraftApp, page: &str) {
    if app.dialog != Some(Dialog::Settings) {
        app.dialog_state.prefs_snapshot = Some(app.session.prefs.clone());
    }
    app.dialog_state.settings_page = page.to_string();
    app.dialog = Some(Dialog::Settings);
}

enum Act {
    Set(String, Value),
    Run(String, Value),
    PickFolder(String),
    PickProject(String),
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn reg(app: &mut EffectcraftApp, id: &str, r: &egui::Response, label: &str) {
    app.auto.add(id, r.rect, label);
}

/// One page's rows.
fn page_ui(app: &mut EffectcraftApp, ui: &mut egui::Ui, page: &Page, cur: &Value, t: &Tokens, acts: &mut Vec<Act>) {
    let get = |key: &str| cur.pointer(&format!("/{}", key.replace('.', "/"))).cloned().unwrap_or(Value::Null);
    let label_w = 250.0;
    for item in &page.items {
        match *item {
            Item::Section(title) => {
                ui.add_space(6.0);
                ui.label(RichText::new(title).font(Tokens::semibold(12.5)).color(t.tab_text_active));
                ui.separator();
            }
            Item::Note(text) => {
                ui.label(RichText::new(text).color(t.text_dim));
            }
            Item::Button { label, command, params } => {
                let r = ui.button(label);
                reg(app, &format!("settings.button.{}", label.trim_end_matches('.')), &r, label);
                if r.clicked() {
                    acts.push(Act::Run(command.into(), serde_json::from_str(params).unwrap_or(json!({}))));
                }
            }
            Item::Labels => labels_ui(app, ui, cur, acts),
            Item::AudioDevices { key } => {
                let devices: Vec<String> = app.hooks.audio_devices.as_ref().map(|f| f()).unwrap_or_default();
                let sel = get(key).as_str().unwrap_or("").to_string();
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(label_w, 18.0), egui::Label::new("Default Output:"));
                    let shown = if sel.is_empty() { "System Default".to_string() } else { sel.clone() };
                    let r = egui::ComboBox::from_id_salt(("settings", key)).selected_text(shown).width(260.0).show_ui(ui, |ui| {
                        if ui.selectable_label(sel.is_empty(), "System Default").clicked() {
                            acts.push(Act::Set(key.into(), json!("")));
                        }
                        for d in &devices {
                            if ui.selectable_label(*d == sel, d).clicked() {
                                acts.push(Act::Set(key.into(), json!(d)));
                            }
                        }
                    });
                    reg(app, &format!("settings.{key}"), &r.response, "Default Output");
                });
                if devices.is_empty() {
                    ui.label(RichText::new("No device list from this host; the system default output is used.").color(t.text_dim));
                }
            }
            Item::Setting { key, label, kind, live } => {
                let v = get(key);
                let id = format!("settings.{key}");
                let tip = (!live).then_some("Stored, but not used by EffectCraft yet");
                let mut resp: Option<egui::Response> = None;
                match kind {
                    Kind::Bool => {
                        let mut b = v.as_bool().unwrap_or(false);
                        let r = ui.checkbox(&mut b, label);
                        if r.changed() {
                            acts.push(Act::Set(key.into(), json!(b)));
                        }
                        resp = Some(r);
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.add_sized(vec2(label_w, 18.0), egui::Label::new(format!("{label}:")).truncate());
                            let r = match kind {
                                Kind::Int(lo, hi, unit) => {
                                    let mut x = v.as_i64().unwrap_or(0);
                                    let r = ui.add(egui::DragValue::new(&mut x).range(lo..=hi).speed(0.25));
                                    if !unit.is_empty() {
                                        ui.label(RichText::new(unit).color(t.text_dim));
                                    }
                                    if r.changed() {
                                        acts.push(Act::Set(key.into(), json!(x)));
                                    }
                                    r
                                }
                                Kind::Float(lo, hi, unit) => {
                                    let mut x = v.as_f64().unwrap_or(0.0);
                                    let r = ui.add(egui::DragValue::new(&mut x).range(lo..=hi).speed(0.1).max_decimals(3));
                                    if !unit.is_empty() {
                                        ui.label(RichText::new(unit).color(t.text_dim));
                                    }
                                    if r.changed() {
                                        acts.push(Act::Set(key.into(), json!(x)));
                                    }
                                    r
                                }
                                Kind::Slider(lo, hi) => {
                                    let mut x = v.as_f64().unwrap_or(0.0);
                                    ui.label(RichText::new("Darker").color(t.text_dim));
                                    let r = ui.add(egui::Slider::new(&mut x, lo..=hi).show_value(false));
                                    ui.label(RichText::new("Lighter").color(t.text_dim));
                                    if ui.small_button("Default").clicked() {
                                        acts.push(Act::Set(key.into(), json!(0.0)));
                                    }
                                    if r.changed() {
                                        acts.push(Act::Set(key.into(), json!(x)));
                                    }
                                    r
                                }
                                Kind::Choice(opts) => {
                                    let curv = v.as_str().unwrap_or("").to_string();
                                    let shown = opts.iter().find(|o| o.1 == curv).map(|o| o.0).unwrap_or(curv.as_str()).to_string();
                                    egui::ComboBox::from_id_salt(("settings", key))
                                        .selected_text(shown)
                                        .width(220.0)
                                        .show_ui(ui, |ui| {
                                            for (l, val) in opts {
                                                if ui.selectable_label(*val == curv, *l).clicked() {
                                                    acts.push(Act::Set(key.into(), json!(val)));
                                                }
                                            }
                                        })
                                        .response
                                }
                                Kind::Text | Kind::Path => {
                                    let mut s = v.as_str().unwrap_or("").to_string();
                                    let r = ui.add(egui::TextEdit::singleline(&mut s).desired_width(if kind == Kind::Path { 230.0 } else { 260.0 }));
                                    if r.changed() {
                                        acts.push(Act::Set(key.into(), json!(s)));
                                    }
                                    if kind == Kind::Path {
                                        let b = ui.button("Choose...");
                                        reg(app, &format!("{id}.choose"), &b, "Choose...");
                                        if b.clicked() {
                                            acts.push(if key.ends_with("Path") { Act::PickProject(key.into()) } else { Act::PickFolder(key.into()) });
                                        }
                                    }
                                    r
                                }
                                Kind::Color => {
                                    let c = crate::panels::viewer::hex_rgb(v.as_str().unwrap_or("")).unwrap_or([128, 128, 128]);
                                    let mut rgb = c;
                                    let r = ui.color_edit_button_srgb(&mut rgb);
                                    ui.label(RichText::new(hex(rgb)).monospace().color(t.text_dim));
                                    if rgb != c {
                                        acts.push(Act::Set(key.into(), json!(hex(rgb))));
                                    }
                                    r
                                }
                                Kind::Bool => unreachable!(),
                            };
                            resp = Some(r);
                        });
                    }
                }
                if let Some(r) = resp {
                    let r = match tip {
                        Some(tip) => r.on_hover_text(tip),
                        None => r,
                    };
                    reg(app, &id, &r, label);
                }
            }
        }
    }
}

fn labels_ui(app: &mut EffectcraftApp, ui: &mut egui::Ui, cur: &Value, acts: &mut Vec<Act>) {
    ui.label("Label Colors and Names:");
    ui.add_space(4.0);
    let labels = cur.get("labels").and_then(Value::as_array).cloned().unwrap_or_default();
    egui::Grid::new("settings-labels").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
        for (i, l) in labels.iter().enumerate() {
            let c = crate::panels::viewer::hex_rgb(l["color"].as_str().unwrap_or("")).unwrap_or([128, 128, 128]);
            let mut rgb = c;
            let r = ui.color_edit_button_srgb(&mut rgb);
            app.auto.add(&format!("settings.labels.{i}.color"), r.rect, "label colour");
            if rgb != c {
                acts.push(Act::Set(format!("labels.{i}.color"), json!(hex(rgb))));
            }
            let mut name = l["name"].as_str().unwrap_or("").to_string();
            let r = ui.add_sized(vec2(160.0, 20.0), egui::TextEdit::singleline(&mut name));
            app.auto.add(&format!("settings.labels.{i}.name"), r.rect, "label name");
            if r.changed() {
                acts.push(Act::Set(format!("labels.{i}.name"), json!(name)));
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
    ui.add_space(8.0);
    let r = ui.button("Reset Labels");
    app.auto.add("settings.button.Reset Labels", r.rect, "Reset Labels");
    if r.clicked() {
        acts.push(Act::Run("prefs.reset".into(), json!({"page": "labels"})));
    }
}

pub fn show(app: &mut EffectcraftApp, ctx: &egui::Context, t: &Tokens) {
    let all = pages();
    let mut page = app.dialog_state.settings_page.clone();
    if !all.iter().any(|p| p.id == page) {
        page = "general".into();
    }
    let idx = all.iter().position(|p| p.id == page).unwrap_or(0);
    let cur = serde_json::to_value(&app.session.prefs).unwrap_or_default();
    let mut acts: Vec<Act> = vec![];
    let (mut ok, mut cancel) = (false, false);
    super::dialogs::modal(ctx, "Settings", vec2(820.0, 600.0), t, |ui| {
        let body_h = 470.0;
        ui.allocate_ui_with_layout(vec2(ui.available_width(), body_h), egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.set_height(body_h);
            // Page list (left, like After Effects).
            ui.vertical(|ui| {
                ui.set_width(180.0);
                for p in &all {
                    let (r, resp) = ui.allocate_exact_size(vec2(176.0, 24.0), egui::Sense::click());
                    let sel = page == p.id;
                    if sel {
                        ui.painter().rect_filled(r, 3.0, t.accent);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 3.0, t.hover);
                    }
                    ui.painter().text(
                        r.left_center() + vec2(10.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        p.title,
                        Tokens::ui(12.5),
                        if sel { Color32::WHITE } else { t.text },
                    );
                    app.auto.add(&format!("settings.page.{}", p.id), r, p.title);
                    if resp.clicked() {
                        page = p.id.to_string();
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(590.0);
                let p = &all[idx];
                ui.label(RichText::new(p.title).font(Tokens::semibold(14.0)).color(t.tab_text_active));
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(body_h - 30.0).auto_shrink([false, false]).show(ui, |ui| {
                    page_ui(app, ui, p, &cur, t, &mut acts);
                });
            });
        });
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            let r = ui.add_enabled(idx > 0, egui::Button::new("Previous"));
            app.auto.add("settings.previous", r.rect, "Previous");
            if r.clicked() {
                page = all[idx - 1].id.to_string();
            }
            let r = ui.add_enabled(idx + 1 < all.len(), egui::Button::new("Next"));
            app.auto.add("settings.next", r.rect, "Next");
            if r.clicked() {
                page = all[idx + 1].id.to_string();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = ui.add(egui::Button::new(RichText::new("   OK   ").color(Color32::WHITE)).fill(t.accent));
                app.auto.add("settings.ok", r.rect, "OK");
                ok |= r.clicked();
                let r = ui.button("Cancel");
                app.auto.add("settings.cancel", r.rect, "Cancel");
                cancel |= r.clicked();
            });
        });
    });
    app.dialog_state.settings_page = page;
    for a in acts {
        match a {
            Act::Set(k, v) => {
                if let Err(e) = app.session.prefs.set(&k, v) {
                    app.ui.status = e;
                }
                app.session.prefs_changed();
            }
            Act::Run(cmd, params) => {
                if let Err(e) = crate::menus::invoke(app, ctx, &cmd, params) {
                    app.ui.status = e;
                }
                // Buttons such as Reset Settings change the baseline Cancel returns to.
                if cmd.starts_with("prefs.") {
                    app.dialog_state.prefs_snapshot = Some(app.session.prefs.clone());
                }
                if app.dialog != Some(Dialog::Settings) && app.dialog != Some(Dialog::Info) {
                    app.dialog = Some(Dialog::Settings);
                }
            }
            Act::PickFolder(k) => {
                if let Some(f) = app.hooks.pick_folder.as_ref().and_then(|f| f()) {
                    let _ = app.session.prefs.set(&k, json!(f));
                    app.session.prefs_changed();
                }
            }
            Act::PickProject(k) => {
                if let Some(f) = app.hooks.pick_open_project.as_ref().and_then(|f| f()) {
                    let _ = app.session.prefs.set(&k, json!(f));
                    app.session.prefs_changed();
                }
            }
        }
    }
    if cancel {
        if let Some(p) = app.dialog_state.prefs_snapshot.take() {
            app.session.prefs = p;
            app.session.prefs_changed();
        }
        app.dialog = None;
    } else if ok {
        app.dialog_state.prefs_snapshot = None;
        app.session.save_prefs();
        app.dialog = None;
    }
}

/// Crash recovery: the previous session didn't exit cleanly; offer its latest auto-save.
pub fn recovery(app: &mut EffectcraftApp, ctx: &egui::Context, t: &Tokens) {
    let Some(r) = app.recovery.clone() else {
        app.dialog = None;
        return;
    };
    let mut choice: Option<&str> = None;
    super::dialogs::modal(ctx, "Recover Project", vec2(560.0, 230.0), t, |ui| {
        ui.label("EffectCraft didn't quit normally last time.");
        ui.add_space(6.0);
        if let Some(p) = &r.project {
            ui.label(format!("Project: {p}"));
        }
        if let Some(a) = &r.autosave {
            let when = std::fs::metadata(a).and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok()).map(|d| {
                let m = d.as_secs() / 60;
                if m < 1 {
                    "less than a minute ago".to_string()
                } else if m < 120 {
                    format!("{m} minutes ago")
                } else {
                    format!("{} hours ago", m / 60)
                }
            });
            ui.label(format!("Latest auto-save: {a}"));
            if let Some(w) = when {
                ui.label(RichText::new(format!("Saved {w}")).color(t.text_dim));
            }
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let b = ui.add(egui::Button::new(RichText::new(" Open Auto-Save ").color(Color32::WHITE)).fill(t.accent));
            app.auto.add("recovery.openAutoSave", b.rect, "Open Auto-Save");
            if b.clicked() {
                choice = Some("autosave");
            }
            if r.project.is_some() {
                let b = ui.button("Open Last Saved Project");
                app.auto.add("recovery.openProject", b.rect, "Open Last Saved Project");
                if b.clicked() {
                    choice = Some("project");
                }
            }
            let b = ui.button("Don't Recover");
            app.auto.add("recovery.dismiss", b.rect, "Don't Recover");
            if b.clicked() {
                choice = Some("none");
            }
        });
    });
    let Some(c) = choice else { return };
    app.dialog = None;
    app.recovery = None;
    let path = match c {
        "autosave" => r.autosave,
        "project" => r.project,
        _ => None,
    };
    if let Some(p) = path
        && let Err(e) = crate::menus::invoke(app, ctx, "file.open", json!({"path": p}))
    {
        app.ui.status = e;
    }
}
