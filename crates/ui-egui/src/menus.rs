//! Menus, keyboard shortcuts and UI-level commands. The menu bar is the engine's After Effects
//! menu tree (`effectcraft_engine::menus`): every entry is an engine command, and frontend-only
//! commands (viewer zoom, panels, dialogs…) come back as `Event::Frontend` and are performed by
//! [`frontend`]. [`UI_COMMANDS`] holds the remaining UI-only shortcuts (tools, timeline
//! navigation, reveal keys). `invoke` is the single entry point used by menus, shortcuts and the
//! control channel.

use serde_json::{Value, json};

use crate::EffectcraftApp;
use crate::dock::PanelKind;
use crate::state::{Resolution, Tool};
use effectcraft_engine::menus::{MenuEntry, MenuNode};

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

/// UI-only commands (not in the AE menus; shortcuts and the command palette).
pub const UI_COMMANDS: &[UiCommand] = &[
    uic!("playback.ramPreview", "Play Current Preview", [], Some("Num0")),
    uic!("playback.stop", "Stop", [], None),
    uic!("view.fit", "Fit", [], Some("Shift+/")),
    uic!("view.actualSize", "100%", [], Some("/")),
    uic!("view.res.auto", "Resolution: Auto", [], None),
    uic!("view.safeMargins", "Title/Action Safe", [], None),
    uic!("view.transparencyGrid", "Transparency Grid", [], None),
    uic!("view.fastPreviews", "Fast Previews", [], None),
    uic!("view.theme.dark", "Theme: Dark", [], None),
    uic!("view.theme.darker", "Theme: Darker", [], None),
    uic!("view.theme.light", "Theme: Light", [], None),
    uic!("timeline.zoomIn", "Zoom In Time", [], Some("=")),
    uic!("timeline.zoomOut", "Zoom Out Time", [], Some("-")),
    uic!("timeline.zoomFit", "Zoom to Fit Comp", [], Some(";")),
    uic!("timeline.graphEditor", "Graph Editor", [], Some("Shift+F3")),
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
    uic!("timeline.reveal.effects", "Reveal Effects", [], Some("E")),
    uic!("timeline.reveal.masks", "Reveal Masks", [], Some("M")),
    uic!("timeline.reveal.feather", "Reveal Mask Feather", [], Some("F")),
    uic!("timeline.reveal.levels", "Reveal Audio Levels (press twice: Waveform)", [], Some("L")),
    uic!("timeline.reveal.waveform", "Reveal Audio Waveform", [], None),
    uic!("timeline.collapseAll", "Collapse All", [], Some("Cmd+`")),
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
    uic!("app.newComp", "New Composition...", [], None),
    uic!("app.compSettings", "Composition Settings...", [], None),
    uic!("app.solidSettings", "New Solid...", [], None),
    uic!("app.home", "Home", [], None),
];

pub fn panel_command_id(p: PanelKind) -> String {
    format!("window.panel.{}", p.id())
}

fn reveal(app: &mut EffectcraftApp, kind: &str, now: f64) {
    // AE's double-press shortcuts: L then L again quickly reveals the Waveform (LL).
    let kind = match app.last_reveal.take() {
        Some((k, t)) if k == "levels" && kind == "levels" && now - t < 0.6 => "waveform",
        _ => kind,
    };
    app.last_reveal = Some((kind.to_string(), now));
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

fn no_params(p: &Value) -> bool {
    p.as_object().is_none_or(|m| m.is_empty())
}

/// Execute a UI or engine command by id.
pub fn invoke(app: &mut EffectcraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {
    let now = ctx.input(|i| i.time);
    // New Camera/Light and Camera/Light Settings without parameters open their dialogs.
    if crate::panels::dialogs_3d::route(app, id, &params)? {
        return Ok(Value::Null);
    }
    // Legacy per-panel / per-workspace ids (`window.panel.Project`, `window.workspace.default`).
    if let Some(rest) = id.strip_prefix("window.panel.") {
        return frontend(app, ctx, "window.panel", json!({"panel": rest}));
    }
    if let Some(ws) = id.strip_prefix("window.workspace.") {
        if ws == "reset" {
            return frontend(app, ctx, "window.resetWorkspace", json!({}));
        }
        return frontend(app, ctx, "window.workspace", json!({"name": ws}));
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
        if k == "animated" {
            return run_engine(app, ctx, "anim.reveal", json!({"kind": "keyframes"}));
        }
        reveal(app, k, now);
        return Ok(Value::Null);
    }
    if id == "view.res.auto" {
        app.ui.viewer.res = Resolution::Auto;
        return Ok(Value::Null);
    }
    if let Some(th) = id.strip_prefix("view.theme.") {
        let k = crate::theme::ThemeKind::from_name(th).ok_or("unknown theme")?;
        app.set_theme(ctx, k);
        return Ok(Value::Null);
    }
    let timing = |app: &mut EffectcraftApp, op: &str| app.session.execute("layer.timing", json!({"op": op})).map_err(|e| e.to_string());
    match id {
        "playback.ramPreview" => {
            app.toggle_play(now);
            return Ok(json!({"playing": app.playback.playing}));
        }
        "playback.stop" => {
            app.stop();
            return Ok(Value::Null);
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
        "view.safeMargins" => app.ui.viewer.safe_margins = !app.ui.viewer.safe_margins,
        "view.transparencyGrid" => app.ui.viewer.transparency_grid = !app.ui.viewer.transparency_grid,
        "view.fastPreviews" => {
            let on = !app.ui.viewer.fast_preview;
            app.set_pref("previews.fastPreviews", json!(on))?;
            app.ui.viewer.fast_preview = on;
        }
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
        "app.home" => app.ui.start_screen = !app.ui.start_screen,
        "app.newComp" | "comp.new" if no_params(&params) => crate::panels::dialogs::open_new_comp(app),
        "app.compSettings" | "comp.settings" if no_params(&params) => crate::panels::dialogs::open_comp_settings(app)?,
        "app.solidSettings" | "layer.newSolid" if no_params(&params) => crate::panels::dialogs::open_new_solid(app)?,
        "keys.velocity" if no_params(&params) => crate::panels::key_dialogs::open_velocity(app)?,
        "keys.interpolation" if no_params(&params) => crate::panels::key_dialogs::open_interpolation(app)?,
        "layer.timeStretch" if no_params(&params) => crate::panels::key_dialogs::open_time_stretch(app)?,
        _ => {
            if id == "renderQueue.render" && params.get("wait").is_none() {
                // The UI renders in the background; the panel shows progress.
                let mut p = params.clone();
                if let Some(m) = p.as_object_mut() {
                    m.insert("wait".into(), json!(false));
                } else {
                    p = json!({"wait": false});
                }
                app.show_panel(PanelKind::RenderQueue);
                return app.session.execute(id, p).map_err(|e| e.to_string());
            }
            if id == "renderQueue.add" {
                let r = app.session.execute(id, params).map_err(|e| e.to_string());
                match &r {
                    Ok(_) => app.show_panel(PanelKind::RenderQueue),
                    Err(e) => app.ui.status = e.clone(),
                }
                return r;
            }
            if let Some(r) = file_dialog(app, id, &params) {
                return r;
            }
            if crate::panels::dialogs::open_form(app, id, &params) {
                return Ok(json!({"dialog": id}));
            }
            if id == "renderQueue.render" && params.get("wait").is_none() {
                // The UI renders in the background; the panel shows progress.
                let mut p = params.clone();
                if let Some(m) = p.as_object_mut() {
                    m.insert("wait".into(), json!(false));
                } else {
                    p = json!({"wait": false});
                }
                app.show_panel(PanelKind::RenderQueue);
                return app.session.execute(id, p).map_err(|e| e.to_string());
            }
            if id == "renderQueue.add" {
                let r = app.session.execute(id, params).map_err(|e| e.to_string());
                match &r {
                    Ok(_) => app.show_panel(PanelKind::RenderQueue),
                    Err(e) => app.ui.status = e.clone(),
                }
                return r;
            }
            if id == "layer.rename" && params.get("name").is_none() {
                crate::panels::timeline::begin_rename(app, ctx);
                return Ok(Value::Null);
            }
            return run_engine(app, ctx, id, params);
        }
    }
    Ok(Value::Null)
}

/// Run an engine command and perform the frontend events it emitted right away.
fn run_engine(app: &mut EffectcraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {
    let r = app.session.execute(id, params).map_err(|e| e.to_string());
    if let Err(e) = &r {
        app.ui.status = e.clone();
    }
    let events = app.session.drain_events();
    for ev in events {
        match ev {
            effectcraft_engine::Event::Frontend { command, params } => {
                if let Err(e) = frontend(app, ctx, &command, params) {
                    app.ui.status = e;
                }
            }
            other => app.session.events.push(other),
        }
    }
    r
}

fn toggle(slot: &mut bool, p: &Value) -> Value {
    *slot = p.get("value").and_then(Value::as_bool).unwrap_or(!*slot);
    json!(*slot)
}

/// Perform a frontend command (from `Event::Frontend`).
pub fn frontend(app: &mut EffectcraftApp, ctx: &egui::Context, id: &str, p: Value) -> Result<Value, String> {
    let now = ctx.input(|i| i.time);
    let v = &mut app.ui.viewer;
    Ok(match id {
        "app.about" => {
            app.dialog = Some(crate::Dialog::About);
            Value::Null
        }
        "app.settings" => {
            let page = p.get("page").and_then(Value::as_str).unwrap_or("general");
            crate::panels::settings::open(app, effectcraft_engine::prefs::page_id(page).unwrap_or("general"));
            Value::Null
        }
        "app.gpuInfo" => {
            let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
            crate::panels::dialogs::info(
                app,
                "GPU Information",
                &format!(
                    "Compositing: CPU, {threads} threads (pure Rust, rayon).\nDisplay: egui on wgpu.\nLayer cache: {} MB  •  Preview cache: {} MB.",
                    app.session.layer_cache.budget() >> 20,
                    app.frames.budget() >> 20
                ),
            );
            Value::Null
        }
        "app.hide" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            Value::Null
        }
        "app.quit" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            Value::Null
        }
        "app.commandPalette" => {
            app.dialog_state.palette_query = p.get("query").and_then(Value::as_str).unwrap_or_default().to_string();
            app.dialog_state.palette_sel = 0;
            app.dialog = Some(crate::Dialog::CommandPalette);
            Value::Null
        }
        "app.keyboardShortcuts" => {
            app.dialog = Some(crate::Dialog::Shortcuts);
            Value::Null
        }
        "app.templates" => {
            let kind = p.get("kind").and_then(Value::as_str).unwrap_or("renderSettings");
            let title = if kind == "outputModule" { "Output Module Templates" } else { "Render Settings Templates" };
            crate::panels::dialogs::info(app, title, "Render and output templates are managed from the Render Queue panel (Window ▸ Render Queue).");
            Value::Null
        }
        "app.find" => {
            if let Some(q) = p.get("query").and_then(Value::as_str) {
                app.ui.project_search = q.to_string();
            }
            app.show_panel(PanelKind::Project);
            Value::Null
        }
        "playback.toggle" => {
            app.toggle_play(now);
            json!({"playing": app.playback.playing})
        }
        "playback.cacheWhenIdle" => {
            let r = toggle(&mut app.ui.cache_when_idle, &p);
            app.set_pref("previews.cacheFramesWhenIdle", r.clone())?;
            r
        }
        "playback.audio" => {
            let r = toggle(&mut app.ui.preview_audio, &p);
            if !app.ui.preview_audio {
                app.audio = None;
            }
            r
        }
        "view.zoomIn" | "view.zoomOut" => {
            let cur = v.zoom.unwrap_or_else(|| crate::panels::viewer::last_fit(ctx));
            const STEPS: [f32; 16] = [0.015, 0.03, 0.0625, 0.125, 0.25, 0.333, 0.5, 0.66, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0, 32.0];
            let next = if id == "view.zoomIn" {
                STEPS.iter().copied().find(|s| *s > cur + 1e-3).unwrap_or(32.0)
            } else {
                STEPS.iter().rev().copied().find(|s| *s < cur - 1e-3).unwrap_or(0.015)
            };
            v.zoom = Some(next);
            json!({"zoom": next})
        }
        "view.res.full" | "view.res.half" | "view.res.third" | "view.res.quarter" => {
            let r = id.trim_start_matches("view.res.");
            v.res = Resolution::ALL.into_iter().find(|x| x.label().eq_ignore_ascii_case(r)).ok_or("unknown resolution")?;
            Value::Null
        }
        "view.rulers" => toggle(&mut v.rulers, &p),
        "view.guides" => toggle(&mut v.guides, &p),
        "view.snapToGuides" => toggle(&mut v.snap_guides, &p),
        "view.lockGuides" => toggle(&mut v.lock_guides, &p),
        "view.grid" => toggle(&mut v.grid, &p),
        "view.snapToGrid" => toggle(&mut v.snap_grid, &p),
        "view.layerControls" => toggle(&mut v.show_layer_controls, &p),
        "view.options" => {
            app.dialog = Some(crate::Dialog::ViewOptions);
            Value::Null
        }
        "view.panelBackground" => {
            if p.get("pick").and_then(Value::as_bool) == Some(true) {
                let [r, g, b] = v.custom_pasteboard;
                crate::panels::dialogs::form(
                    app,
                    "Panel Background Color",
                    "view.panelBackground",
                    json!({}),
                    vec![crate::panels::dialogs::Field::text("color", "Color (#rrggbb)", &format!("#{r:02x}{g:02x}{b:02x}"))],
                );
                return Ok(Value::Null);
            }
            let c = p.get("color").and_then(Value::as_str).unwrap_or("mediumGray");
            v.pasteboard = match c {
                "black" => Some([0, 0, 0]),
                "darkGray" => Some([0x2a, 0x2a, 0x2a]),
                "mediumGray" => None,
                "lightGray" => Some([0xb4, 0xb4, 0xb4]),
                "white" => Some([0xff, 0xff, 0xff]),
                "custom" => Some(v.custom_pasteboard),
                hex => {
                    let c = effectcraft_engine::color::Rgba::from_hex(hex).ok_or_else(|| format!("unknown colour `{hex}`"))?;
                    let rgb = [(c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8];
                    v.custom_pasteboard = rgb;
                    Some(rgb)
                }
            };
            Value::Null
        }
        "view.fullScreen" => {
            let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
            json!(!full)
        }
        "window.panel" => {
            let name = p.get("panel").and_then(Value::as_str).unwrap_or_default();
            if name.eq_ignore_ascii_case("tools") {
                app.ui.status = "The Tools panel is the toolbar in the header".into();
                return Ok(Value::Null);
            }
            let panel = PanelKind::from_name(name).ok_or_else(|| format!("unknown panel `{name}`"))?;
            app.show_panel(panel);
            Value::Null
        }
        "window.workspace" => {
            let want = p.get("name").and_then(Value::as_str).unwrap_or("Default").to_ascii_lowercase().replace(' ', "");
            let name = crate::dock::WORKSPACES
                .iter()
                .find(|w| w.to_ascii_lowercase().replace(' ', "") == want)
                .ok_or_else(|| format!("unknown workspace `{want}`"))?;
            app.set_workspace(name);
            json!({"workspace": name})
        }
        "window.resetWorkspace" => {
            let name = app.ui.workspace.clone();
            app.set_workspace(&name);
            Value::Null
        }
        "comp.flowchart" | "comp.miniFlowchart" => {
            app.show_panel(PanelKind::Flowchart);
            Value::Null
        }
        "layer.openLayer" => {
            app.show_panel(PanelKind::Layer);
            Value::Null
        }
        "effect.manage" | "anim.browsePresets" => {
            app.show_panel(PanelKind::EffectsPresets);
            Value::Null
        }
        "timeline.revealProps" => {
            let props = p.get("props").and_then(Value::as_array).cloned().unwrap_or_default();
            let tl = &mut app.ui.timeline;
            tl.reveal_props = props.iter().filter_map(|x| x.get("prop").and_then(Value::as_u64)).collect();
            tl.open_layers = props.iter().filter_map(|x| x.get("layer").and_then(Value::as_u64)).collect();
            tl.reveal = if tl.reveal_props.is_empty() { vec![] } else { vec!["props".into()] };
            json!({"revealed": tl.reveal_props.len()})
        }
        _ => return Err(format!("`{id}` is not a frontend command")),
    })
}

/// Commands that need a file or folder path: ask the host's file dialog when `params` lacks one.
fn file_dialog(app: &mut EffectcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    enum Ask {
        Import,
        OpenProject,
        Save(&'static str),
        Open(&'static [&'static str]),
    }
    let (key, ask) = match id {
        "file.import" | "file.importMultiple" => ("paths", Ask::Import),
        "file.open" => ("path", Ask::OpenProject),
        "file.save" if app.session.path.is_none() => ("path", Ask::Save("Untitled Project.ecproj")),
        "file.saveAs" | "file.saveCopy" => ("path", Ask::Save("Untitled Project.ecproj")),
        "comp.saveFrameAs" => ("path", Ask::Save("Frame.png")),
        "anim.savePreset" => ("path", Ask::Save("Preset.ecpreset")),
        "anim.applyPreset" => ("path", Ask::Open(&["ecpreset", "json"])),
        "view.exportGuides" => ("path", Ask::Save("Guides.json")),
        "view.importGuides" => ("path", Ask::Open(&["json"])),
        "file.runScript" => ("path", Ask::Open(&["jsonl", "json", "txt"])),
        "file.replaceFootage" => ("path", Ask::Import),
        "file.collectFiles" => ("folder", Ask::Save("Collected Files")),
        _ => return None,
    };
    if params.get(key).is_some()
        || params.get("path").is_some()
        || params.get("paths").is_some()
        || params.get("steps").is_some()
        || params.get("preset").is_some()
    {
        return None;
    }
    let mut p = params.as_object().cloned().unwrap_or_default();
    let picked: Option<Value> = match ask {
        Ask::Import => {
            let Some(f) = app.hooks.pick_files.as_ref() else { return Some(Err("no file dialog available (pass `paths`)".into())) };
            let paths = f(&[
                "mp4", "mov", "m4v", "mkv", "webm", "png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp", "exr", "wav", "aif", "aiff", "mp3", "flac",
                "ogg", "opus", "svg",
            ]);
            match (paths.is_empty(), key) {
                (true, _) => None,
                (false, "paths") => Some(json!(paths)),
                (false, _) => paths.first().map(|s| json!(s)),
            }
        }
        Ask::OpenProject => {
            let Some(f) = app.hooks.pick_open_project.as_ref() else { return Some(Err("no file dialog available (pass `path`)".into())) };
            f().map(|s| json!(s))
        }
        Ask::Open(exts) => {
            let Some(f) = app.hooks.pick_files.as_ref() else { return Some(Err("no file dialog available (pass `path`)".into())) };
            f(exts).first().map(|s| json!(s))
        }
        Ask::Save(default) => {
            let Some(f) = app.hooks.pick_save.as_ref() else { return Some(Err("no file dialog available (pass `path`)".into())) };
            f(default).map(|s| json!(s))
        }
    };
    let Some(v) = picked else { return Some(Ok(Value::Null)) };
    p.insert(key.to_string(), v);
    // Save (untitled) becomes Save As.
    let id = if id == "file.save" { "file.saveAs" } else { id };
    let r = app.session.execute(id, Value::Object(p)).map_err(|e| e.to_string());
    if let Err(e) = &r {
        app.ui.status = e.clone();
    }
    Some(r)
}

/// A menu entry for display / `ui.menu.list`.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub path: Vec<String>,
    pub shortcut: Option<String>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// Parameters bound to the entry (e.g. the effect for Effect menu items).
    #[serde(skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

/// Top-level menus in order.
pub fn menus() -> Vec<&'static str> {
    effectcraft_engine::menus::top_level()
}

fn entry_label(app: &EffectcraftApp, e: &MenuEntry) -> String {
    match e.command.as_str() {
        "edit.undo" => app.session.history.undo.last().map(|u| format!("Undo {}", u.0)).unwrap_or_else(|| "Can't Undo".into()),
        "edit.redo" => app.session.history.redo.last().map(|u| format!("Redo {}", u.0)).unwrap_or_else(|| "Can't Redo".into()),
        _ => effectcraft_engine::menus::entry_label(&app.session, e),
    }
}

/// Check-mark state of frontend toggles (engine state is answered by the engine).
fn entry_checked(app: &EffectcraftApp, e: &MenuEntry) -> Option<bool> {
    let v = &app.ui.viewer;
    let pstr = |k: &str| e.params.get(k).and_then(Value::as_str);
    match e.command.as_str() {
        "view.rulers" => Some(v.rulers),
        "view.guides" => Some(v.guides),
        "view.snapToGuides" => Some(v.snap_guides),
        "view.lockGuides" => Some(v.lock_guides),
        "view.grid" => Some(v.grid),
        "view.snapToGrid" => Some(v.snap_grid),
        "view.layerControls" => Some(v.show_layer_controls),
        "playback.cacheWhenIdle" => Some(app.ui.cache_when_idle),
        "playback.audio" => Some(app.ui.preview_audio),
        "view.res.full" | "view.res.half" | "view.res.third" | "view.res.quarter" => {
            Some(v.res.label().eq_ignore_ascii_case(e.command.trim_start_matches("view.res.")))
        }
        "window.workspace" => Some(pstr("name").is_some_and(|n| n == app.ui.workspace)),
        "view.panelBackground" if e.params.get("pick").is_none() => {
            let cur = match v.pasteboard {
                None => "mediumGray",
                Some([0, 0, 0]) => "black",
                Some([0x2a, 0x2a, 0x2a]) => "darkGray",
                Some([0xb4, 0xb4, 0xb4]) => "lightGray",
                Some([0xff, 0xff, 0xff]) => "white",
                Some(_) => "custom",
            };
            Some(pstr("color") == Some(cur))
        }
        _ => effectcraft_engine::menus::checked(&app.session, &e.command, &e.params),
    }
}

fn entry_enabled(app: &EffectcraftApp, e: &MenuEntry) -> bool {
    if e.command == "window.panel" {
        return true;
    }
    app.session.is_enabled(&e.command)
}

/// Every menu entry, flattened with its submenu path.
pub fn menu_items(app: &EffectcraftApp) -> Vec<MenuItem> {
    effectcraft_engine::menus::entries()
        .into_iter()
        .map(|(path, e)| MenuItem {
            id: e.command.clone(),
            label: entry_label(app, e),
            path,
            shortcut: e.shortcut.clone(),
            enabled: entry_enabled(app, e),
            checked: entry_checked(app, e),
            params: e.params.clone(),
        })
        .collect()
}

/// Shortcut text in this OS's notation (`⇧⌘K` on macOS, `Ctrl+Shift+K` elsewhere).
pub fn shortcut_text(s: &str) -> String {
    if cfg!(target_os = "macos") {
        let mut out = String::new();
        let parts: Vec<&str> = match s.strip_suffix("++") {
            Some(head) => vec![head, "+"],
            None => s.split('+').collect(),
        };
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
        out.push_str(match key.first().copied().unwrap_or("") {
            "Space" => "Space",
            "Escape" => "Esc",
            "Home" => "↖",
            k => k,
        });
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

/// The key some keyboards report for a shifted punctuation key (`Shift+=` arrives as `+`).
fn shifted_alias(k: egui::Key) -> Option<egui::Key> {
    use egui::Key::*;
    Some(match k {
        Equals => Plus,
        Semicolon => Colon,
        OpenBracket => OpenCurlyBracket,
        CloseBracket => CloseCurlyBracket,
        Slash => Questionmark,
        Backslash => Pipe,
        _ => return None,
    })
}

/// A key binding: modifiers, key, command id and bound params.
pub type Binding = (egui::Modifiers, egui::Key, String, Value);

/// All active bindings, most modifiers first.
pub fn bindings() -> Vec<Binding> {
    let mut v: Vec<Binding> = Vec::new();
    let mut push = |sc: &str, id: &str, params: Value| {
        if let Some((m, k)) = parse_shortcut(sc) {
            if m.shift
                && let Some(alias) = shifted_alias(k)
            {
                v.push((m, alias, id.to_string(), params.clone()));
            }
            v.push((m, k, id.to_string(), params));
        }
    };
    // Menu entries (their shortcuts may bind params), then command defaults not in the menus.
    let entries = effectcraft_engine::menus::entries();
    for (_, e) in &entries {
        if let Some(sc) = &e.shortcut {
            push(sc, &e.command, e.params.clone());
        }
    }
    for c in effectcraft_engine::command_specs() {
        if let Some(sc) = c.shortcut
            && !entries.iter().any(|(_, e)| e.command == c.id)
        {
            push(sc, c.id, Value::Null);
        }
    }
    for c in UI_COMMANDS {
        if let Some(sc) = c.shortcut {
            push(sc, c.id, Value::Null);
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
            // Escape in Settings is Cancel: restore the settings from when it opened.
            if app.dialog == Some(crate::Dialog::Settings)
                && let Some(p) = app.dialog_state.prefs_snapshot.take()
            {
                app.session.prefs = p;
                app.session.prefs_changed();
            }
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
        if let Some((_, _, id, params)) = binds.iter().find(|(m, k, ..)| *k == key && mods_match(*m, mods)) {
            let (id, params) = (id.clone(), if params.is_null() { json!({}) } else { params.clone() });
            if let Err(e) = invoke(app, ctx, &id, params) {
                app.ui.status = e;
            }
        }
    }
}

/// Draw the in-window menu bar (the engine's AE menu tree).
pub fn menu_bar(app: &mut EffectcraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let mut clicked: Option<(String, Value)> = None;
    ui.horizontal_centered(|ui| {
        ui.add_space(6.0);
        egui::MenuBar::new().ui(ui, |ui| {
            for node in effectcraft_engine::menus::menu_bar() {
                if let MenuNode::Submenu { label, children } = node {
                    let r = ui.menu_button(label, |ui| {
                        ui.set_min_width(if label == "Effect" { 200.0 } else { 280.0 });
                        menu_nodes(app, ui, children, &mut clicked);
                    });
                    app.auto.add(&format!("menu.{label}"), r.response.rect, label);
                }
            }
        });
    });
    if let Some((id, params)) = clicked
        && let Err(e) = invoke(app, &ctx, &id, if params.is_null() { json!({}) } else { params })
    {
        app.ui.status = e;
    }
}

fn menu_nodes(app: &mut EffectcraftApp, ui: &mut egui::Ui, nodes: &[MenuNode], clicked: &mut Option<(String, Value)>) {
    for n in nodes {
        match n {
            MenuNode::Separator => {
                ui.separator();
            }
            MenuNode::Submenu { label, children } => {
                ui.menu_button((gutter(false), label.as_str()), |ui| {
                    ui.set_min_width(if children.len() > 30 { 200.0 } else { 240.0 });
                    // Long submenus (Blending Mode, effect categories) scroll instead of running
                    // off the screen.
                    let max_h = ui.ctx().content_rect().height() - 40.0;
                    egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| menu_nodes(app, ui, children, clicked));
                });
            }
            MenuNode::Item(e) => {
                if menu_entry(app, ui, e) {
                    *clicked = Some((e.command.clone(), e.params.clone()));
                    ui.close();
                }
            }
        }
    }
}

/// The check-mark column every menu row reserves (like macOS / After Effects menus).
fn gutter(checked: bool) -> egui::Atom<'static> {
    use egui::AtomExt;
    (if checked { "✔" } else { "" }).atom_size(egui::vec2(14.0, 14.0))
}

fn menu_entry(app: &EffectcraftApp, ui: &mut egui::Ui, e: &MenuEntry) -> bool {
    let label = entry_label(app, e);
    let mut b = egui::Button::new((gutter(entry_checked(app, e) == Some(true)), label));
    if let Some(s) = &e.shortcut {
        b = b.shortcut_text(shortcut_text(s));
    }
    ui.add_enabled(entry_enabled(app, e), b).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ui_shortcut_parses() {
        for c in UI_COMMANDS {
            if let Some(sc) = c.shortcut {
                assert!(parse_shortcut(sc).is_some() || sc == "Num*", "{sc}");
            }
        }
        for (_, e) in effectcraft_engine::menus::entries() {
            if let Some(sc) = &e.shortcut {
                assert!(parse_shortcut(sc).is_some() || sc == "Num*", "{} ({sc})", e.label);
            }
        }
    }

    #[test]
    fn ui_and_menu_shortcuts_do_not_collide() {
        let mut seen: std::collections::BTreeMap<(String, String), (String, String)> = Default::default();
        for (m, k, id, params) in bindings() {
            let key = (format!("{m:?}"), format!("{k:?}"));
            let target = (id.clone(), params.to_string());
            if let Some(prev) = seen.get(&key) {
                assert_eq!(prev, &target, "{key:?} bound to {prev:?} and {target:?}");
            } else {
                seen.insert(key, target);
            }
        }
    }

    #[test]
    fn renamed_label_shows_in_the_label_menu() {
        let mut app = EffectcraftApp::new(effectcraft_engine::Session::default());
        app.set_pref("labels.1.name", json!("Sunflower")).unwrap();
        let labels: Vec<String> = menu_items(&app).into_iter().filter(|i| i.id == "edit.label").map(|i| i.label).collect();
        assert!(labels.contains(&"Sunflower".to_string()), "{labels:?}");
        assert!(!labels.contains(&"Yellow".to_string()));
    }

    #[test]
    fn menu_items_cover_the_tree() {
        assert_eq!(menus().last(), Some(&"Help"));
        assert!(effectcraft_engine::menus::entries().len() > 500);
    }
}
