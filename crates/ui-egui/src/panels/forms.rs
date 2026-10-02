//! Menu dialogs: a generic parameter form (numeric Transform dialogs, Mask Feather/Opacity/
//! Expansion, Auto-Orient, Go to Time, Add Guide, Sequence Layers, Interpret Footage, Project
//! Settings, placeholders…), View Options and simple message boxes (Settings: `settings`;
//! Keyboard Shortcuts: `shortcut_editor`). Every form runs an engine command with the collected parameters, so whatever a
//! dialog does an agent can do with one `engine.execute`.

use egui::{Color32, vec2};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;
use crate::{Dialog, EffectcraftApp};

#[derive(Clone, Debug)]
pub enum FieldKind {
    Number {
        value: f64,
        speed: f64,
    },
    Text(String),
    Bool(bool),
    /// Options as (label, value); `sel` is the chosen index.
    Choice {
        options: Vec<(String, Value)>,
        sel: usize,
    },
}

/// One form field. `key` may index into an array param (`value[1]`).
#[derive(Clone, Debug)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
}

impl Field {
    pub fn num(key: &str, label: &str, value: f64) -> Field {
        Field { key: key.into(), label: label.into(), kind: FieldKind::Number { value, speed: 1.0 } }
    }
    pub fn text(key: &str, label: &str, value: &str) -> Field {
        Field { key: key.into(), label: label.into(), kind: FieldKind::Text(value.into()) }
    }
    pub fn bool(key: &str, label: &str, value: bool) -> Field {
        Field { key: key.into(), label: label.into(), kind: FieldKind::Bool(value) }
    }
    pub fn choice(key: &str, label: &str, options: &[(&str, Value)], sel: usize) -> Field {
        Field {
            key: key.into(),
            label: label.into(),
            kind: FieldKind::Choice { options: options.iter().map(|(l, v)| (l.to_string(), v.clone())).collect(), sel },
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Form {
    pub title: String,
    pub command: String,
    pub base: Value,
    pub fields: Vec<Field>,
}

impl Form {
    /// The command parameters: `base` plus every field.
    pub fn params(&self) -> Value {
        let mut m: Map<String, Value> = self.base.as_object().cloned().unwrap_or_default();
        for f in &self.fields {
            let v = match &f.kind {
                FieldKind::Number { value, .. } => json!(value),
                FieldKind::Text(s) => json!(s),
                FieldKind::Bool(b) => json!(b),
                FieldKind::Choice { options, sel } => options.get(*sel).map(|o| o.1.clone()).unwrap_or(Value::Null),
            };
            match f.key.split_once('[') {
                Some((name, idx)) => {
                    let i: usize = idx.trim_end_matches(']').parse().unwrap_or(0);
                    let arr = m.entry(name.to_string()).or_insert_with(|| json!([]));
                    if let Value::Array(a) = arr {
                        while a.len() <= i {
                            a.push(json!(0));
                        }
                        a[i] = v;
                    }
                }
                None => {
                    m.insert(f.key.clone(), v);
                }
            }
        }
        Value::Object(m)
    }
}

/// Open a form dialog.
pub fn form(app: &mut EffectcraftApp, title: &str, command: &str, base: Value, fields: Vec<Field>) {
    app.dialog_state.form = Form { title: title.into(), command: command.into(), base, fields };
    app.dialog = Some(Dialog::Form);
}

/// Open a message box.
pub fn info(app: &mut EffectcraftApp, title: &str, body: &str) {
    app.dialog_state.info = (title.into(), body.into());
    app.dialog = Some(Dialog::Info);
}

fn has(p: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|k| p.get(*k).is_some())
}

/// If `id` is a dialog command invoked without its parameters, open its form and return true.
pub fn open_form(app: &mut EffectcraftApp, id: &str, p: &Value) -> bool {
    let s = &app.session;
    let comp = s.active_comp();
    let layer = comp.and_then(|c| s.state.selected_layers.first().and_then(|l| c.layer(*l)));
    let t = s.time();
    let lt = layer.map(|l| l.layer_time(t)).unwrap_or(t);
    let base = p.clone();
    let (title, fields): (String, Vec<Field>) = match id {
        "layer.setTransform" if !has(p, &["value"]) => {
            let prop = p.get("prop").and_then(Value::as_str).unwrap_or("position");
            let cur = layer.and_then(|l| l.transform()).and_then(|tr| tr.get(if prop == "anchorPoint" { "anchor" } else { prop })).map(|pr| pr.value_at(lt));
            let v = cur.map(|c| c.as_vec3()).unwrap_or([0.0; 3]);
            let three = layer.is_some_and(|l| l.is_3d());
            let title = match prop {
                "anchor" => "Anchor Point",
                "position" => "Position",
                "scale" => "Scale",
                "orientation" => "Orientation",
                "rotation" => "Rotation",
                _ => "Opacity",
            };
            let fields = match prop {
                "rotation" | "opacity" => vec![Field::num("value", if prop == "rotation" { "Degrees" } else { "Opacity (%)" }, v[0])],
                "orientation" => vec![Field::num("value[0]", "X", v[0]), Field::num("value[1]", "Y", v[1]), Field::num("value[2]", "Z", v[2])],
                _ if three => vec![Field::num("value[0]", "X", v[0]), Field::num("value[1]", "Y", v[1]), Field::num("value[2]", "Z", v[2])],
                _ => vec![Field::num("value[0]", "X", v[0]), Field::num("value[1]", "Y", v[1])],
            };
            (title.into(), fields)
        }
        "layer.mask.set" if !has(p, &["value"]) => {
            let field = p.get("field").and_then(Value::as_str).unwrap_or("feather");
            let first = layer.and_then(|l| l.masks()).and_then(|m| m.groups().next()).and_then(|g| g.get(field)).map(|pr| pr.value_at(lt).as_f64());
            let (title, label, d) = match field {
                "feather" => ("Mask Feather", "Feather (pixels)", 0.0),
                "opacity" => ("Mask Opacity", "Opacity (%)", 100.0),
                _ => ("Mask Expansion", "Expansion (pixels)", 0.0),
            };
            (title.into(), vec![Field::num("value", label, first.unwrap_or(d))])
        }
        "layer.mask.shape" if !has(p, &["rect"]) => {
            let (w, h) = layer.map(|l| effectcraft_engine::render::source_size(&s.project, l)).unwrap_or((100, 100));
            (
                "Mask Shape".into(),
                vec![
                    Field::num("rect[0]", "Left", 0.0),
                    Field::num("rect[1]", "Top", 0.0),
                    Field::num("rect[2]", "Width", w as f64),
                    Field::num("rect[3]", "Height", h as f64),
                    Field::choice("shape", "Shape", &[("Rectangle", json!("rect")), ("Ellipse", json!("ellipse"))], 0),
                ],
            )
        }
        "layer.autoOrient" if !has(p, &["mode"]) => {
            let cur = layer.map(|l| l.auto_orient as usize).unwrap_or(0);
            (
                "Auto-Orientation".into(),
                vec![Field::choice(
                    "mode",
                    "Mode",
                    &[
                        ("Off", json!("off")),
                        ("Orient Along Path", json!("alongPath")),
                        ("Orient Towards Camera", json!("towardsCamera")),
                        ("Orient Towards Point of Interest", json!("towardsPointOfInterest")),
                    ],
                    cur.min(3),
                )],
            )
        }
        "time.set" if !has(p, &["time", "frame", "timecode"]) => {
            let frame = comp.map(|c| c.frame_rate.frame_at(t)).unwrap_or(0);
            ("Go to Time".into(), vec![Field::num("frame", "Frame", frame as f64)])
        }
        "view.addGuide" if !has(p, &["position"]) => {
            let (w, _) = comp.map(|c| (c.width, c.height)).unwrap_or((1920, 1080));
            (
                "Add Guide".into(),
                vec![
                    Field::choice("orientation", "Orientation", &[("Vertical", json!("vertical")), ("Horizontal", json!("horizontal"))], 0),
                    Field::num("position", "Position (pixels)", w as f64 / 2.0),
                ],
            )
        }
        "layer.sequence" if !has(p, &["overlap"]) => (
            "Sequence Layers".into(),
            vec![
                Field::bool("overlap", "Overlap", false),
                Field::num("duration", "Duration (seconds)", 1.0),
                Field::choice(
                    "transition",
                    "Transition",
                    &[
                        ("Off", json!("off")),
                        ("Dissolve Front Layer", json!("dissolveFront")),
                        ("Cross Dissolve Front and Back Layers", json!("crossDissolve")),
                    ],
                    0,
                ),
            ],
        ),
        "file.interpretFootage" if !has(p, &["frameRate", "alpha", "loop", "pixelAspect"]) => {
            let f = s.state.project_selection.first().and_then(|i| s.project.item(*i)).and_then(|it| match &it.kind {
                effectcraft_engine::project::ItemKind::Footage(f) => Some(f.clone()),
                _ => None,
            });
            let Some(f) = f else { return false };
            (
                "Interpret Footage".into(),
                vec![
                    Field::num("frameRate", "Frame rate", f.frame_rate.as_f64()),
                    Field::choice(
                        "alpha",
                        "Alpha",
                        &[("Straight", json!("straight")), ("Premultiplied", json!("premultiplied")), ("Ignore", json!("ignore"))],
                        f.alpha as usize,
                    ),
                    Field::num("loop", "Loop (times)", f.loop_count as f64),
                    Field::num("pixelAspect", "Pixel aspect ratio", f.pixel_aspect),
                ],
            )
        }
        "file.projectSettings" if p.as_object().is_none_or(|m| m.is_empty()) => {
            let st = &s.project.settings;
            let depth = match st.bit_depth.label() {
                l if l.starts_with("16") => 1,
                l if l.starts_with("32") => 2,
                _ => 0,
            };
            (
                "Project Settings".into(),
                vec![
                    Field::choice(
                        "bitDepth",
                        "Depth",
                        &[("8 bits per channel", json!(8)), ("16 bits per channel", json!(16)), ("32 bits per channel (float)", json!(32))],
                        depth,
                    ),
                    Field::choice(
                        "timeDisplay",
                        "Time display style",
                        &[("Timecode", json!("timecode")), ("Frames", json!("frames"))],
                        usize::from(matches!(st.time_display, effectcraft_engine::project::TimeDisplayStyle::Frames)),
                    ),
                    Field::bool("linearize", "Linearize working space", st.linearize),
                ],
            )
        }
        "layer.precompose" if !has(p, &["name"]) => {
            let n = s.project.comps().count() + 1;
            ("Pre-compose".into(), vec![Field::text("name", "New composition name", &format!("Pre-comp {n}"))])
        }
        "file.importPlaceholder" | "file.replaceWithPlaceholder" if p.as_object().is_none_or(|m| m.is_empty()) => (
            "New Placeholder".into(),
            vec![
                Field::text("name", "Name", "Placeholder"),
                Field::num("width", "Width", 1920.0),
                Field::num("height", "Height", 1080.0),
                Field::num("frameRate", "Frame rate", 29.97),
                Field::num("duration", "Duration (seconds)", 30.0),
            ],
        ),
        "file.importSolid" | "file.replaceWithSolid" if p.as_object().is_none_or(|m| m.is_empty()) => {
            let (w, h) = comp.map(|c| (c.width, c.height)).unwrap_or((1920, 1080));
            (
                "Solid Settings".into(),
                vec![
                    Field::text("name", "Name", "Solid"),
                    Field::text("color", "Color (#rrggbb)", "#808080"),
                    Field::num("width", "Width", w as f64),
                    Field::num("height", "Height", h as f64),
                ],
            )
        }
        _ => return false,
    };
    form(app, &title, id, base, fields);
    true
}

pub fn show_form(app: &mut EffectcraftApp, ctx: &egui::Context, t: &Tokens) {
    let mut f = app.dialog_state.form.clone();
    let (mut ok, mut close) = (false, false);
    let h = 120.0 + f.fields.len() as f32 * 30.0;
    super::dialogs::modal(ctx, &f.title.clone(), vec2(440.0, h), t, |ui| {
        egui::Grid::new("form-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
            for fl in &mut f.fields {
                ui.label(&fl.label);
                match &mut fl.kind {
                    FieldKind::Number { value, speed } => {
                        ui.add(egui::DragValue::new(value).speed(*speed).max_decimals(3));
                    }
                    FieldKind::Text(s) => {
                        ui.add(egui::TextEdit::singleline(s).desired_width(220.0));
                    }
                    FieldKind::Bool(b) => {
                        ui.checkbox(b, "");
                    }
                    FieldKind::Choice { options, sel } => {
                        let cur = options.get(*sel).map(|o| o.0.clone()).unwrap_or_default();
                        egui::ComboBox::from_id_salt(("form", fl.key.as_str())).selected_text(cur).width(240.0).show_ui(ui, |ui| {
                            for (i, (l, _)) in options.iter().enumerate() {
                                ui.selectable_value(sel, i, l);
                            }
                        });
                    }
                }
                ui.end_row();
            }
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
    let enter = !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Enter));
    app.dialog_state.form = f.clone();
    if ok || enter {
        app.dialog = None;
        let params = f.params();
        if let Err(e) = app.session.execute(&f.command, params) {
            app.ui.status = e.to_string();
        }
        return;
    }
    if close {
        app.dialog = None;
    }
}

pub fn show_info(app: &mut EffectcraftApp, ctx: &egui::Context, t: &Tokens) {
    let (title, body) = app.dialog_state.info.clone();
    let mut close = false;
    super::dialogs::modal(ctx, &title, vec2(440.0, 160.0), t, |ui| {
        ui.label(body);
        ui.add_space(16.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(egui::RichText::new("   OK   ").color(Color32::WHITE)).fill(t.accent)).clicked() {
                close = true;
            }
        });
    });
    if close {
        app.dialog = None;
    }
}

pub fn show_view_options(app: &mut EffectcraftApp, ctx: &egui::Context, t: &Tokens) {
    let mut close = false;
    super::dialogs::modal(ctx, "View Options", vec2(360.0, 330.0), t, |ui| {
        let v = &mut app.ui.viewer;
        ui.checkbox(&mut v.show_layer_controls, "Layer controls");
        ui.checkbox(&mut v.show_masks, "Masks");
        ui.checkbox(&mut v.rulers, "Rulers");
        ui.checkbox(&mut v.guides, "Guides");
        ui.checkbox(&mut v.grid, "Grid");
        ui.checkbox(&mut v.safe_margins, "Title/action safe");
        ui.checkbox(&mut v.transparency_grid, "Transparency grid");
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(egui::RichText::new("   OK   ").color(Color32::WHITE)).fill(t.accent)).clicked() {
                close = true;
            }
        });
    });
    if close {
        app.dialog = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_params_build_arrays_and_choices() {
        let f = Form {
            title: "T".into(),
            command: "layer.setTransform".into(),
            base: json!({"prop": "position"}),
            fields: vec![
                Field::num("value[0]", "X", 10.0),
                Field::num("value[1]", "Y", 20.0),
                Field::choice("mode", "M", &[("A", json!("a")), ("B", json!("b"))], 1),
            ],
        };
        assert_eq!(f.params(), json!({"prop": "position", "value": [10.0, 20.0], "mode": "b"}));
    }
}
