//! Menus, keyboard shortcuts and UI-level commands. The menu bar is generated from the engine
//! registry plus the UI command table (commands that only affect the frontend: tools, playback,
//! viewer zoom, panels, reveal shortcuts). `invoke` is the single entry point used by menus,
//! shortcuts and the control channel.

use serde_json::{Value, json};

use crate::EffectcraftApp;
use crate::dock::PanelKind;
use crate::state::{Resolution, Tool};

pub struct UiCommand {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static [&'static str],
    pub shortcut: Option<&'static str>,
}

macro_rules! uic {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr) => {
        UiCommand { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc }
    };
}

pub const UI_COMMANDS: &[UiCommand] = &[
    uic!("playback.toggle", "Play/Stop", ["Composition", "Preview"], Some("Space")),
    uic!("playback.ramPreview", "Play Current Preview", ["Composition", "Preview"], Some("Num0")),
    uic!("playback.stop", "Stop", [], None),
    uic!("view.zoomIn", "Zoom In", ["View"], Some(".")),
    uic!("view.zoomOut", "Zoom Out", ["View"], Some(",")),
    uic!("view.fit", "Fit", ["View"], Some("Shift+/")),
    uic!("view.actualSize", "100%", ["View"], Some("/")),
    uic!("view.res.full", "Full", ["View", "Resolution"], Some("Cmd+J")),
    uic!("view.res.half", "Half", ["View", "Resolution"], Some("Cmd+Shift+J")),
    uic!("view.res.third", "Third", ["View", "Resolution"], None),
    uic!("view.res.quarter", "Quarter", ["View", "Resolution"], Some("Cmd+Alt+Shift+J")),
    uic!("view.res.auto", "Auto", ["View", "Resolution"], None),
    uic!("view.rulers", "Show Rulers", ["View"], Some("Cmd+R")),
    uic!("view.grid", "Show Grid", ["View"], Some("Cmd+'")),
    uic!("view.safeMargins", "Title/Action Safe", ["View"], None),
    uic!("view.transparencyGrid", "Transparency Grid", ["View"], None),
    uic!("view.layerControls", "Show Layer Controls", ["View"], Some("Cmd+Shift+H")),
    uic!("view.fastPreviews", "Fast Previews", ["View"], None),
    uic!("view.theme.dark", "Dark", ["View", "Appearance"], None),
    uic!("view.theme.darker", "Darker", ["View", "Appearance"], None),
    uic!("view.theme.light", "Light", ["View", "Appearance"], None),
    uic!("timeline.zoomIn", "Zoom In Time", [], Some("=")),
    uic!("timeline.zoomOut", "Zoom Out Time", [], Some("-")),
    uic!("timeline.zoomFit", "Zoom to Fit Comp", [], Some(";")),
    uic!("timeline.graphEditor", "Graph Editor", ["Window"], Some("Shift+F3")),
    uic!("timeline.switchesModes", "Toggle Switches / Modes", [], Some("F4")),
    uic!("timeline.workAreaBegin", "Set Work Area Begin", [], Some("B")),
    uic!("timeline.workAreaEnd", "Set Work Area End", [], Some("N")),
    uic!("timeline.moveInToTime", "Move Layer In Point to Current Time", [], Some("[")),
    uic!("timeline.moveOutToTime", "Move Layer Out Point to Current Time", [], Some("]")),
    uic!("timeline.trimInToTime", "Trim Layer In Point to Current Time", [], Some("Alt+[")),
    uic!("timeline.trimOutToTime", "Trim Layer Out Point to Current Time", [], Some("Alt+]")),
    uic!("timeline.reveal.position", "Reveal Position", [], Some("P")),
    uic!("timeline.reveal.scale", "Reveal Scale", [], Some("S")),
    uic!("timeline.reveal.rotation", "Reveal Rotation", [], Some("R")),
    uic!("timeline.reveal.opacity", "Reveal Opacity", [], Some("T")),
    uic!("timeline.reveal.anchor", "Reveal Anchor Point", [], Some("A")),
    uic!("timeline.reveal.animated", "Reveal Animated Properties", ["Animation"], Some("U")),
    uic!("timeline.reveal.effects", "Reveal Effects", [], Some("E")),
    uic!("timeline.reveal.masks", "Reveal Masks", [], Some("M")),
    uic!("timeline.reveal.feather", "Reveal Mask Feather", [], Some("F")),
    uic!("timeline.collapseAll", "Collapse All", [], Some("Cmd+`")),
    uic!("window.workspace.default", "Default", ["Window", "Workspace"], Some("Shift+F10")),
    uic!("window.workspace.standard", "Standard", ["Window", "Workspace"], Some("Shift+F11")),
    uic!("window.workspace.smallscreen", "Small Screen", ["Window", "Workspace"], Some("Shift+F12")),
    uic!("window.workspace.animation", "Animation", ["Window", "Workspace"], None),
    uic!("window.workspace.effects", "Effects", ["Window", "Workspace"], None),
    uic!("window.workspace.motiontracking", "Motion Tracking", ["Window", "Workspace"], None),
    uic!("window.workspace.paint", "Paint", ["Window", "Workspace"], None),
    uic!("window.workspace.text", "Text", ["Window", "Workspace"], None),
    uic!("window.workspace.minimal", "Minimal", ["Window", "Workspace"], None),
    uic!("window.workspace.allpanels", "All Panels", ["Window", "Workspace"], None),
    uic!("window.workspace.reset", "Reset to Saved Layout", ["Window", "Workspace"], None),
    uic!("tool.selection", "Selection Tool", [], Some("V")),
    uic!("tool.hand", "Hand Tool", [], Some("H")),
    uic!("tool.zoom", "Zoom Tool", [], Some("Z")),
    uic!("tool.orbit", "Orbit Tool", [], Some("1")),
    uic!("tool.panCamera", "Pan Camera Tool", [], Some("2")),
    uic!("tool.dolly", "Dolly Tool", [], Some("3")),
    uic!("tool.rotate", "Rotation Tool", [], Some("W")),
    uic!("tool.panBehind", "Pan Behind Tool", [], Some("Y")),
    uic!("tool.shape", "Shape Tools", [], Some("Q")),
    uic!("tool.pen", "Pen Tool", [], Some("G")),
    uic!("tool.type", "Type Tools", [], Some("Cmd+T")),
    uic!("tool.brush", "Brush Tools", [], Some("Cmd+B")),
    uic!("tool.rotoBrush", "Roto Brush Tool", [], Some("Alt+W")),
    uic!("tool.puppet", "Puppet Tools", [], Some("Cmd+P")),
    uic!("app.newComp", "New Composition…", [], None),
    uic!("app.compSettings", "Composition Settings…", [], None),
    uic!("app.solidSettings", "New Solid…", [], None),
    uic!("app.commandPalette", "Command Palette…", ["Window"], Some("Cmd+Shift+P")),
    uic!("app.about", "About EffectCraft", ["Help"], None),
    uic!("app.home", "Home", ["Window"], None),
];

pub fn panel_command_id(p: PanelKind) -> String {
    format!("window.panel.{}", p.id())
}

fn reveal(app: &mut EffectcraftApp, kind: &str) {
    let add = app.ui.timeline.reveal.contains(&kind.to_string());
    if add && app.ui.timeline.reveal.len() == 1 {
        app.ui.timeline.reveal.clear();
        app.ui.timeline.open_layers.clear();
        return;
    }
    app.ui.timeline.reveal = vec![kind.to_string()];
    let sel = if app.session.state.selected_layers.is_empty() {
        app.session.active_comp().map(|c| c.layers.iter().map(|l| l.id).collect()).unwrap_or_default()
    } else {
        app.session.state.selected_layers.clone()
    };
    app.ui.timeline.open_layers = sel.iter().map(|l| l.0).collect();
}

/// Execute a UI or engine command by id.
pub fn invoke(app: &mut EffectcraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {
    let now = ctx.input(|i| i.time);
    // New Camera/Light and Camera/Light Settings without parameters open their dialogs.
    if crate::panels::dialogs_3d::route(app, id, &params)? {
        return Ok(Value::Null);
    }
    if let Some(rest) = id.strip_prefix("window.panel.") {
        let p = PanelKind::from_name(rest).ok_or_else(|| format!("unknown panel `{rest}`"))?;
        app.show_panel(p);
        return Ok(Value::Null);
    }
    if let Some(ws) = id.strip_prefix("window.workspace.") {
        if ws == "reset" {
            let name = app.ui.workspace.clone();
            app.set_workspace(&name);
            return Ok(Value::Null);
        }
        let name = crate::dock::WORKSPACES.iter().find(|w| w.to_ascii_lowercase().replace(' ', "") == ws).ok_or_else(|| format!("unknown workspace `{ws}`"))?;
        app.set_workspace(name);
        return Ok(json!({"workspace": name}));
    }
    if let Some(t) = id.strip_prefix("tool.") {
        // Slot shortcuts cycle through the slot's tools.
        let slot = match t {
            "shape" => Some(8),
            "type" => Some(10),
            "brush" => Some(11),
            _ => None,
        };
        if let Some(si) = slot {
            let tools: Vec<Tool> = match si {
                11 => vec![Tool::Brush, Tool::Clone, Tool::Eraser],
                _ => crate::state::Tool::SLOTS[si].to_vec(),
            };
            let cur = tools.iter().position(|x| *x == app.ui.tool);
            let next = match cur {
                Some(i) => tools[(i + 1) % tools.len()],
                None => app.ui.slot_tools.get(si).copied().filter(|x| tools.contains(x)).unwrap_or(tools[0]),
            };
            app.ui.tool = next;
            if si < app.ui.slot_tools.len() && si != 11 {
                app.ui.slot_tools[si] = next;
            }
            return Ok(json!({"tool": next}));
        }
        let tool = Tool::from_name(t).ok_or_else(|| format!("unknown tool `{t}`"))?;
        app.ui.tool = tool;
        return Ok(json!({"tool": tool}));
    }
    if let Some(k) = id.strip_prefix("timeline.reveal.") {
        reveal(app, k);
        return Ok(Value::Null);
    }
    if let Some(r) = id.strip_prefix("view.res.") {
        app.ui.viewer.res = Resolution::ALL.into_iter().find(|x| x.label().eq_ignore_ascii_case(r)).ok_or("unknown resolution")?;
        return Ok(Value::Null);
    }
    if let Some(th) = id.strip_prefix("view.theme.") {
        let k = crate::theme::ThemeKind::from_name(th).ok_or("unknown theme")?;
        app.set_theme(ctx, k);
        return Ok(Value::Null);
    }
    let timing = |app: &mut EffectcraftApp, op: &str| app.session.execute("layer.timing", json!({"op": op})).map_err(|e| e.to_string());
    match id {
        "playback.toggle" | "playback.ramPreview" => {
            app.toggle_play(now);
            return Ok(json!({"playing": app.playback.playing}));
        }
        "playback.stop" => {
            app.stop();
            return Ok(Value::Null);
        }
        "view.zoomIn" | "view.zoomOut" => {
            let cur = app.ui.viewer.zoom.unwrap_or_else(|| crate::panels::viewer::last_fit(ctx));
            const STEPS: [f32; 16] = [0.015, 0.03, 0.0625, 0.125, 0.25, 0.333, 0.5, 0.66, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0, 32.0];
            let next = if id == "view.zoomIn" {
                STEPS.iter().copied().find(|s| *s > cur + 1e-3).unwrap_or(32.0)
            } else {
                STEPS.iter().rev().copied().find(|s| *s < cur - 1e-3).unwrap_or(0.015)
            };
            app.ui.viewer.zoom = Some(next);
            return Ok(json!({"zoom": next}));
        }
        "view.fit" => {
            app.ui.viewer.zoom = None;
            app.ui.viewer.pan = [0.0, 0.0];
            return Ok(Value::Null);
        }
        "view.actualSize" => {
            app.ui.viewer.zoom = Some(1.0 / ctx.pixels_per_point());
            app.ui.viewer.pan = [0.0, 0.0];
            return Ok(Value::Null);
        }
        "view.rulers" => app.ui.viewer.rulers = !app.ui.viewer.rulers,
        "view.grid" => app.ui.viewer.grid = !app.ui.viewer.grid,
        "view.safeMargins" => app.ui.viewer.safe_margins = !app.ui.viewer.safe_margins,
        "view.transparencyGrid" => app.ui.viewer.transparency_grid = !app.ui.viewer.transparency_grid,
        "view.layerControls" => app.ui.viewer.show_layer_controls = !app.ui.viewer.show_layer_controls,
        "view.fastPreviews" => app.ui.viewer.fast_preview = !app.ui.viewer.fast_preview,
        "timeline.zoomIn" | "timeline.zoomOut" => {
            let k = if id == "timeline.zoomIn" { 1.5 } else { 1.0 / 1.5 };
            crate::panels::timeline::zoom(app, ctx, k);
            return Ok(Value::Null);
        }
        "timeline.zoomFit" => app.ui.timeline.pps = None,
        "timeline.graphEditor" => app.ui.timeline.graph_editor = !app.ui.timeline.graph_editor,
        "timeline.switchesModes" => app.ui.timeline.show_modes = !app.ui.timeline.show_modes,
        "timeline.collapseAll" => {
            app.ui.timeline.open_layers.clear();
            app.ui.timeline.open_groups.clear();
            app.ui.timeline.reveal.clear();
        }
        "timeline.workAreaBegin" => return app.session.execute("comp.workArea", json!({"set": "begin"})).map_err(|e| e.to_string()),
        "timeline.workAreaEnd" => return app.session.execute("comp.workArea", json!({"set": "end"})).map_err(|e| e.to_string()),
        "timeline.moveInToTime" => return timing(app, "moveInToTime"),
        "timeline.moveOutToTime" => return timing(app, "moveOutToTime"),
        "timeline.trimInToTime" => return timing(app, "trimInToTime"),
        "timeline.trimOutToTime" => return timing(app, "trimOutToTime"),
        "app.about" => app.dialog = Some(crate::Dialog::About),
        "app.home" => app.ui.start_screen = !app.ui.start_screen,
        "app.commandPalette" => {
            app.dialog_state.palette_query.clear();
            app.dialog = Some(crate::Dialog::CommandPalette);
        }
        "app.newComp" | "comp.new" if params.as_object().is_none_or(|m| m.is_empty()) => {
            crate::panels::dialogs::open_new_comp(app);
        }
        "app.compSettings" | "comp.settings" if params.as_object().is_none_or(|m| m.is_empty()) => {
            crate::panels::dialogs::open_comp_settings(app)?;
        }
        "app.solidSettings" | "layer.newSolid" if params.as_object().is_none_or(|m| m.is_empty()) => {
            crate::panels::dialogs::open_new_solid(app)?;
        }
        _ => {
            if (id == "file.import" && params.get("paths").is_none() && params.get("path").is_none())
                || (id == "file.saveAs" && params.get("path").is_none())
                || (id == "file.open" && params.get("path").is_none())
                || (id == "file.save" && params.get("path").is_none() && app.session.path.is_none())
            {
                return file_dialog(app, id);
            }
            if id == "layer.rename" && params.get("name").is_none() {
                crate::panels::timeline::begin_rename(app, ctx);
                return Ok(Value::Null);
            }
            let r = app.session.execute(id, params).map_err(|e| e.to_string());
            if let Err(e) = &r {
                app.ui.status = e.clone();
            }
            return r;
        }
    }
    Ok(Value::Null)
}

fn file_dialog(app: &mut EffectcraftApp, id: &str) -> Result<Value, String> {
    match id {
        "file.import" => {
            let f = app.hooks.pick_files.as_ref().ok_or("no file dialog available (pass `paths`)")?;
            let paths = f(&[
                "mp4", "mov", "m4v", "mkv", "webm", "png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp", "exr", "wav", "aif", "aiff", "mp3", "flac",
                "ogg", "opus", "svg",
            ]);
            if paths.is_empty() {
                return Ok(Value::Null);
            }
            app.session.execute("file.import", json!({"paths": paths})).map_err(|e| e.to_string())
        }
        "file.open" => {
            let f = app.hooks.pick_open_project.as_ref().ok_or("no file dialog available (pass `path`)")?;
            match f() {
                Some(p) => app.session.execute("file.open", json!({"path": p})).map_err(|e| e.to_string()),
                None => Ok(Value::Null),
            }
        }
        _ => {
            let f = app.hooks.pick_save.as_ref().ok_or("no file dialog available (pass `path`)")?;
            match f("Untitled Project.ecproj") {
                Some(p) => app.session.execute("file.saveAs", json!({"path": p})).map_err(|e| e.to_string()),
                None => Ok(Value::Null),
            }
        }
    }
}

/// A menu entry for display / `ui.menu.list`.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub path: Vec<String>,
    pub shortcut: Option<String>,
    pub enabled: bool,
    /// Parameters bound to the entry (e.g. the effect for Effect menu items).
    #[serde(skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

pub const MENUS: [&str; 9] = ["File", "Edit", "Composition", "Layer", "Effect", "Animation", "View", "Window", "Help"];

pub fn menu_items(app: &EffectcraftApp) -> Vec<MenuItem> {
    let mut out = Vec::new();
    for c in effectcraft_engine::command_specs() {
        if c.menu.is_empty() {
            continue;
        }
        out.push(MenuItem {
            id: c.id.into(),
            label: c.label.into(),
            path: c.menu.iter().map(|s| s.to_string()).collect(),
            shortcut: c.shortcut.map(str::to_string),
            enabled: app.session.is_enabled(c.id),
            params: Value::Null,
        });
    }
    for c in UI_COMMANDS {
        if c.menu.is_empty() {
            continue;
        }
        out.push(MenuItem {
            id: c.id.into(),
            label: c.label.into(),
            path: c.menu.iter().map(|s| s.to_string()).collect(),
            shortcut: c.shortcut.map(str::to_string),
            enabled: true,
            params: Value::Null,
        });
    }
    for p in PanelKind::ALL {
        out.push(MenuItem {
            id: panel_command_id(p),
            label: p.title().into(),
            path: vec!["Window".into()],
            shortcut: p.window_shortcut().map(str::to_string),
            enabled: true,
            params: Value::Null,
        });
    }
    let has_layer = !app.session.state.selected_layers.is_empty();
    for e in effectcraft_engine::effects::registry() {
        out.push(MenuItem {
            id: "effect.apply".into(),
            label: e.name.into(),
            path: vec!["Effect".into(), e.category.into()],
            shortcut: None,
            enabled: has_layer,
            params: json!({"effect": e.id}),
        });
    }
    out
}

/// Shortcut text in this OS's notation (`⇧⌘K` on macOS, `Ctrl+Shift+K` elsewhere).
pub fn shortcut_text(s: &str) -> String {
    if cfg!(target_os = "macos") {
        let mut out = String::new();
        let parts: Vec<&str> = s.split('+').collect();
        let (mods, key) = parts.split_at(parts.len().saturating_sub(1));
        for m in mods {
            out.push_str(match *m {
                "Ctrl" => "⌃",
                "Alt" => "⌥",
                "Shift" => "⇧",
                "Cmd" => "⌘",
                x => x,
            });
        }
        out.push_str(key.first().copied().unwrap_or(""));
        out
    } else {
        s.replace("Cmd", "Ctrl")
    }
}

/// Parse "Cmd+Shift+K" into modifiers + key.
pub fn parse_shortcut(s: &str) -> Option<(egui::Modifiers, egui::Key)> {
    let mut m = egui::Modifiers::NONE;
    let mut key = None;
    let parts: Vec<&str> = if s == "+" { vec!["+"] } else { s.split('+').collect() };
    for p in parts {
        match p {
            "Cmd" => {
                m.command = true;
                m.mac_cmd = cfg!(target_os = "macos");
                if !cfg!(target_os = "macos") {
                    m.ctrl = true;
                }
            }
            "Shift" => m.shift = true,
            "Alt" => m.alt = true,
            "Ctrl" => m.ctrl = true,
            k => {
                key = match k {
                    ";" => Some(egui::Key::Semicolon),
                    "'" => Some(egui::Key::Quote),
                    "," => Some(egui::Key::Comma),
                    "." => Some(egui::Key::Period),
                    "/" => Some(egui::Key::Slash),
                    "\\" => Some(egui::Key::Backslash),
                    "=" => Some(egui::Key::Equals),
                    "-" => Some(egui::Key::Minus),
                    "`" => Some(egui::Key::Backtick),
                    "[" => Some(egui::Key::OpenBracket),
                    "]" => Some(egui::Key::CloseBracket),
                    "Num0" => Some(egui::Key::Num0),
                    "Num*" => None,
                    _ => egui::Key::from_name(k),
                }
            }
        }
    }
    key.map(|k| (m, k))
}

/// All active bindings, most modifiers first.
pub fn bindings() -> Vec<(egui::Modifiers, egui::Key, String)> {
    let mut v: Vec<(egui::Modifiers, egui::Key, String)> = Vec::new();
    for c in effectcraft_engine::command_specs() {
        if let Some((m, k)) = c.shortcut.and_then(parse_shortcut) {
            v.push((m, k, c.id.to_string()));
        }
    }
    for c in UI_COMMANDS {
        if let Some((m, k)) = c.shortcut.and_then(parse_shortcut) {
            v.push((m, k, c.id.to_string()));
        }
    }
    for p in PanelKind::ALL {
        if let Some((m, k)) = p.window_shortcut().and_then(parse_shortcut) {
            v.push((m, k, panel_command_id(p)));
        }
    }
    v.sort_by_key(|(m, ..)| std::cmp::Reverse(m.command as u8 + m.shift as u8 + m.alt as u8 + m.ctrl as u8));
    v
}

fn mods_match(want: egui::Modifiers, got: egui::Modifiers) -> bool {
    want.command == got.command && want.shift == got.shift && want.alt == got.alt && (want.ctrl == got.ctrl || got.command && cfg!(not(target_os = "macos")))
}

/// Dispatch keyboard shortcuts (skipped while typing in a text field).
pub fn handle_shortcuts(app: &mut EffectcraftApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let events: Vec<(egui::Key, egui::Modifiers)> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, repeat, .. }
                    if !*repeat || matches!(key, egui::Key::PageUp | egui::Key::PageDown | egui::Key::ArrowLeft | egui::Key::ArrowRight) =>
                {
                    Some((*key, *modifiers))
                }
                _ => None,
            })
            .collect()
    });
    if events.is_empty() {
        return;
    }
    let binds = bindings();
    for (key, mods) in events {
        // Escape closes dialogs.
        if key == egui::Key::Escape && app.dialog.is_some() {
            app.dialog = None;
            continue;
        }
        if app.dialog.is_some() {
            continue;
        }
        // Delete/Backspace clears selection.
        if matches!(key, egui::Key::Delete | egui::Key::Backspace) && !mods.any() {
            let _ = invoke(app, ctx, "edit.clear", json!({}));
            continue;
        }
        if let Some((_, _, id)) = binds.iter().find(|(m, k, _)| *k == key && mods_match(*m, mods)) {
            let id = id.clone();
            if let Err(e) = invoke(app, ctx, &id, json!({})) {
                app.ui.status = e;
            }
        }
    }
}

/// Draw the in-window menu bar.
pub fn menu_bar(app: &mut EffectcraftApp, ui: &mut egui::Ui) {
    let items = menu_items(app);
    let ctx = ui.ctx().clone();
    let mut clicked: Option<(String, Value)> = None;
    ui.horizontal_centered(|ui| {
        ui.add_space(6.0);
        egui::MenuBar::new().ui(ui, |ui| {
            for top in MENUS {
                let mine: Vec<&MenuItem> = items.iter().filter(|i| i.path.first().map(String::as_str) == Some(top)).collect();
                ui.menu_button(top, |ui| {
                    ui.set_min_width(260.0);
                    let mut subs: Vec<&str> = Vec::new();
                    let mut last_was_sub = false;
                    for it in &mine {
                        if it.path.len() > 1 {
                            let sub = it.path[1].as_str();
                            if !subs.contains(&sub) {
                                subs.push(sub);
                                ui.menu_button(sub, |ui| {
                                    ui.set_min_width(220.0);
                                    for s in mine.iter().filter(|x| x.path.get(1).map(String::as_str) == Some(sub)) {
                                        if menu_entry(ui, s) {
                                            clicked = Some((s.id.clone(), s.params.clone()));
                                            ui.close();
                                        }
                                    }
                                });
                                last_was_sub = true;
                            }
                        } else {
                            if last_was_sub && top != "Effect" {
                                ui.separator();
                                last_was_sub = false;
                            }
                            if menu_entry(ui, it) {
                                clicked = Some((it.id.clone(), it.params.clone()));
                                ui.close();
                            }
                        }
                    }
                });
            }
        });
    });
    if let Some((id, params)) = clicked
        && let Err(e) = invoke(app, &ctx, &id, if params.is_null() { json!({}) } else { params })
    {
        app.ui.status = e;
    }
}

fn menu_entry(ui: &mut egui::Ui, it: &MenuItem) -> bool {
    let mut b = egui::Button::new(&it.label);
    if let Some(s) = &it.shortcut {
        b = b.shortcut_text(shortcut_text(s));
    }
    ui.add_enabled(it.enabled, b).clicked()
}
