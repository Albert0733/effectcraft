//! File menu.

use effectcraft_color::Label;
use effectcraft_project::{FootageKind, ItemKind, Project};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, str_p};
use crate::{EngineError, Result, Session, cmd};

fn has_path(s: &Session) -> std::result::Result<(), String> {
    if s.path.is_some() { Ok(()) } else { Err("the project has not been saved yet".into()) }
}

fn new_project(s: &mut Session, _: &Value) -> Result<Value> {
    // Settings ▸ Project ▸ New Project Loads Template: open the template as an untitled project.
    let tpl = s.prefs.project.template_path.trim().to_string();
    if s.prefs.project.use_template && !tpl.is_empty() {
        let bytes = s.services.read_file(&tpl).map_err(|e| EngineError::Other(format!("cannot read the new project template {tpl}: {e}")))?;
        let text = String::from_utf8(bytes).map_err(|_| EngineError::Other("the new project template is not a project file".into()))?;
        s.replace_project(Project::from_json(&text)?, None);
        s.update_sentinel();
        return Ok(json!({"template": tpl}));
    }
    s.replace_project(Project::default(), None);
    s.update_sentinel();
    Ok(Value::Null)
}

fn demo(s: &mut Session, _: &Value) -> Result<Value> {
    let p = crate::demo::demo_project();
    s.replace_project(p, None);
    if let Some(id) = s.project.items.values().find(|i| i.name == crate::demo::MAIN_COMP).map(|i| i.id) {
        s.open_comp(id);
        s.set_time(effectcraft_time::Tick::from_seconds_f64(2.5));
    }
    Ok(json!({"comp": s.state.active_comp.map(|c| c.0)}))
}

pub(crate) fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").ok_or_else(|| bad("file.open", "missing `path`"))?;
    let bytes = s.services.read_file(path).map_err(|e| EngineError::Other(format!("cannot read {path}: {e}")))?;
    let text = String::from_utf8(bytes).map_err(|_| EngineError::Other("not a text project file".into()))?;
    let proj = Project::from_json(&text)?;
    s.replace_project(proj, Some(path.to_string()));
    s.note_project_path(path);
    Ok(json!({"path": path}))
}

fn save_to(s: &mut Session, path: &str) -> Result<Value> {
    let json = s.project.to_json();
    s.services.write_file(path, json.as_bytes()).map_err(|e| EngineError::Other(format!("cannot write {path}: {e}")))?;
    s.path = Some(path.to_string());
    s.saved_revision = s.revision;
    s.note_project_path(path);
    s.toast(format!("Saved {path}"));
    Ok(json!({"path": path, "bytes": json.len()}))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").map(str::to_string).or_else(|| s.path.clone()).ok_or_else(|| bad("file.save", "no path: use file.saveAs {path}"))?;
    save_to(s, &path)
}

fn save_as(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").ok_or_else(|| bad("file.saveAs", "missing `path`"))?;
    save_to(s, path)
}

fn increment_save(s: &mut Session, _: &Value) -> Result<Value> {
    let cur = s.path.clone().ok_or_else(|| bad("file.incrementAndSave", "save the project first"))?;
    // After Effects: `Intro.aep` → `Intro 2.aep` → `Intro 3.aep`.
    let next = crate::autosave::increment_path(&cur, |p| s.file_ops().is_file(std::path::Path::new(p)));
    save_to(s, &next)
}

fn revert(s: &mut Session, _: &Value) -> Result<Value> {
    let path = s.path.clone().ok_or_else(|| bad("file.revert", "not saved"))?;
    open(s, &json!({"path": path}))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let paths: Vec<String> = match p.get("paths").or(p.get("path")) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        Some(Value::String(x)) => vec![x.clone()],
        _ => return Err(bad("file.import", "missing `paths`")),
    };
    // Photoshop documents as compositions (Import As: Composition / – Retain Layer Sizes).
    let import_as = str_p(p, "importAs").unwrap_or("footage");
    let retain = match import_as {
        "footage" => None,
        "composition" | "comp" => Some(false),
        "compositionLayerSizes" | "compositionRetainLayerSizes" | "layerSizes" => Some(true),
        other => return Err(bad("file.import", format!("importAs: footage|composition|compositionLayerSizes, not `{other}`"))),
    };
    let mut paths = paths;
    let mut out_comps = vec![];
    let mut ids = vec![];
    let mut errors = vec![];
    if let Some(retain) = retain {
        let mut rest = vec![];
        for path in paths {
            let bytes = match s.services.read_file(&path) {
                Ok(b) if effectcraft_psd::is_psd(&b) => b,
                Ok(b) if effectcraft_pdf::sniff(&b).is_some() && !path.to_ascii_lowercase().ends_with(".svg") => {
                    // PDF / Illustrator / EPS: one layer per file layer.
                    match import_vector_comp(s, &path, &b) {
                        Ok((comp, items)) => {
                            out_comps.push(comp);
                            ids.extend(items);
                        }
                        Err(e) => errors.push(e.to_string()),
                    }
                    continue;
                }
                _ => {
                    rest.push(path);
                    continue;
                }
            };
            match import_psd_comp(s, &path, bytes, retain) {
                Ok((comp, items, warnings)) => {
                    out_comps.push(comp);
                    ids.extend(items);
                    errors.extend(warnings);
                }
                Err(e) => errors.push(format!("{path}: {e}")),
            }
        }
        paths = rest;
        if paths.is_empty() {
            if let Some(c) = out_comps.first() {
                s.open_comp(effectcraft_project::ItemId(*c));
            }
            return Ok(json!({"items": ids, "comps": out_comps, "errors": errors}));
        }
    }
    let mut probed = vec![];
    let psd_layer = p.get("layer").cloned();
    let seq_rate = effectcraft_time::FrameRate::from_f64(s.prefs.import.sequence_fps);
    for path in &paths {
        // Data files (JSON, CSV, TSV) for data-driven animation: kept as text in the project.
        if let Some(f) = data_footage(s, path) {
            match f {
                Ok(f) => probed.push((path.clone(), f)),
                Err(e) => errors.push(format!("{path}: {e}")),
            }
            continue;
        }
        let Some(importer) = s.importer.clone() else {
            errors.push(format!("{path}: media import is not available in this build"));
            continue;
        };
        match importer.probe(path) {
            Ok(mut f) => {
                // Settings ▸ Import ▸ Sequence Footage frames per second.
                if f.kind == FootageKind::Sequence {
                    let r = seq_rate;
                    let frames = f.frame_rate.frame_at(f.duration);
                    f.frame_rate = r;
                    f.duration = r.tick_of(frames.max(1));
                }
                // Choose Layer: one layer of a Photoshop document (document-sized).
                if let Some(sel) = &psd_layer
                    && f.codec == "PSD"
                {
                    match s.services.read_file(path).ok().and_then(|b| effectcraft_psd::Psd::parse(b).ok()).and_then(|d| find_psd_layer(&d, sel)) {
                        Some((index, name)) => {
                            f.layer = Some(effectcraft_project::SourceLayer { index: index as u32, name, layer_size: false, embedded: None })
                        }
                        None => {
                            errors.push(format!("{path}: no layer {sel}"));
                            continue;
                        }
                    }
                }
                probed.push((path.clone(), f))
            }
            Err(e) => errors.push(format!("{path}: {e}")),
        }
    }
    s.edit("Import", None, |proj, st| {
        for (path, f) in probed {
            let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(path.clone());
            let name = match &f.layer {
                Some(l) => format!("{}/{name}", l.name),
                None => name,
            };
            let label = match f.kind {
                FootageKind::Still | FootageKind::Sequence => Label::Lavender,
                FootageKind::Audio => Label::SeaFoam,
                FootageKind::Video | FootageKind::Model => Label::Aqua,
                FootageKind::Data => Label::Sandstone,
            };
            let id = proj.add_item(&name, label, None, ItemKind::Footage(f));
            ids.push(id.0);
            st.project_selection = vec![id];
        }
        Ok(())
    })?;
    if let Some(c) = out_comps.first() {
        s.open_comp(effectcraft_project::ItemId(*c));
    }
    Ok(json!({"items": ids, "comps": out_comps, "errors": errors}))
}

/// A Photoshop layer by index or name (pixel layers only).
fn find_psd_layer(d: &effectcraft_psd::Psd, sel: &Value) -> Option<(usize, String)> {
    let l = match sel {
        Value::Number(n) => d.layers.get(n.as_u64()? as usize)?,
        Value::String(name) => d.layers.iter().find(|l| &l.name == name)?,
        _ => return None,
    };
    Some((l.index, l.name.clone()))
}

/// Import a Photoshop document as a composition (one undo step). Returns (comp, items, warnings).
fn import_psd_comp(s: &mut Session, path: &str, bytes: Vec<u8>, retain: bool) -> Result<(u64, Vec<u64>, Vec<String>)> {
    let psd = effectcraft_psd::Psd::parse(bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    let name = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Photoshop".into());
    let rate = effectcraft_time::FrameRate::FPS_29_97;
    let secs = if s.prefs.import.still_footage == "seconds" { s.prefs.import.still_seconds } else { 10.0 };
    let duration = rate.snap_nearest(effectcraft_time::Tick::from_seconds_f64(secs));
    let r = s.edit("Import", None, |proj, st| {
        let r = crate::psd_import::import(proj, &psd, path, &name, retain, rate, duration);
        st.project_selection = vec![r.comp];
        Ok(r)
    })?;
    let mut items = vec![r.folder.0];
    items.extend(r.items.iter().map(|i| i.0));
    Ok((r.comp.0, items, r.warnings))
}

/// Import a PDF / Illustrator / EPS file as a composition (one undo step). Returns (comp, items).
fn import_vector_comp(s: &mut Session, path: &str, bytes: &[u8]) -> Result<(u64, Vec<u64>)> {
    let name = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Vector".into());
    let rate = effectcraft_time::FrameRate::FPS_29_97;
    let secs = if s.prefs.import.still_footage == "seconds" { s.prefs.import.still_seconds } else { 10.0 };
    let duration = rate.snap_nearest(effectcraft_time::Tick::from_seconds_f64(secs));
    let (comp, folder, items) = s.edit("Import", None, |proj, st| {
        let r = crate::vector::import_vector_comp(proj, path, bytes, &name, rate, duration).map_err(EngineError::Other)?;
        st.project_selection = vec![r.0];
        Ok(r)
    })?;
    let mut out = vec![folder.0];
    out.extend(items.iter().map(|i| i.0));
    Ok((comp.0, out))
}

/// Extensions imported as data footage.
pub const DATA_EXTENSIONS: &[&str] = &["json", "csv", "tsv"];

/// A data footage item for `path` (JSON / CSV / TSV), `None` for other files.
pub(crate) fn data_footage(s: &Session, path: &str) -> Option<std::result::Result<effectcraft_project::Footage, String>> {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !DATA_EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }
    Some((|| {
        let bytes = s.services.read_file(path).map_err(|e| e.to_string())?;
        let text = String::from_utf8(bytes).map_err(|_| "data files must be UTF-8 text".to_string())?;
        if ext == "json" {
            serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).map_err(|e| format!("invalid JSON: {e}"))?;
        }
        Ok(effectcraft_project::Footage { path: path.to_string(), kind: FootageKind::Data, codec: ext.to_uppercase(), data: Some(text), ..Default::default() })
    })())
}

fn project_settings(s: &mut Session, p: &Value) -> Result<Value> {
    s.edit("Project Settings", None, |proj, _| {
        if let Some(b) = str_p(p, "bitDepth") {
            proj.settings.bit_depth = match b {
                "8" | "8bpc" | "8 bpc" => effectcraft_project::BitDepth::Bpc8,
                "16" | "16bpc" | "16 bpc" => effectcraft_project::BitDepth::Bpc16,
                "32" | "32bpc" | "32 bpc" => effectcraft_project::BitDepth::Bpc32,
                _ => return Err(bad("file.projectSettings", "bitDepth must be 8, 16 or 32")),
            };
        } else if let Some(n) = p.get("bitDepth").and_then(Value::as_u64) {
            proj.settings.bit_depth = match n {
                16 => effectcraft_project::BitDepth::Bpc16,
                32 => effectcraft_project::BitDepth::Bpc32,
                _ => effectcraft_project::BitDepth::Bpc8,
            };
        }
        if let Some(l) = p.get("linearize").and_then(Value::as_bool) {
            proj.settings.linearize = l;
        }
        if let Some(l) = p.get("blendLinear").or_else(|| p.get("blendColorsUsing1Gamma")).and_then(Value::as_bool) {
            proj.settings.blend_linear = l;
        }
        use effectcraft_project::{ColorEngine, ColorSpace, HdrMode};
        let cmd = "file.projectSettings";
        if let Some(e) = str_p(p, "colorEngine") {
            let engine = match e.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
                "adobe" | "adobemanaged" | "builtin" => ColorEngine::Adobe,
                "ocio" | "ociomanaged" => ColorEngine::Ocio,
                _ => return Err(bad(cmd, format!("colorEngine: adobe|ocio, not `{e}`"))),
            };
            if engine == ColorEngine::Ocio && proj.settings.color_engine != ColorEngine::Ocio {
                // The OCIO built-in config works in ACES and renders the display through its
                // tone-mapped view.
                if !proj.settings.working_space.is_some_and(|w| w.is_linear()) {
                    proj.settings.working_space = Some(ColorSpace::AcesCg);
                }
                if proj.settings.hdr == HdrMode::Clip {
                    proj.settings.hdr = HdrMode::ToneMap;
                }
            }
            proj.settings.color_engine = engine;
        }
        if let Some(w) = p.get("workingSpace") {
            let ids = ColorSpace::WORKING.iter().map(|c| c.id()).collect::<Vec<_>>().join("|");
            proj.settings.working_space = match w.as_str() {
                None | Some("none" | "None" | "") => None,
                Some(n) => Some(
                    ColorSpace::parse(n).filter(|c| ColorSpace::WORKING.contains(c)).ok_or_else(|| bad(cmd, format!("workingSpace: none|{ids}, not `{n}`")))?,
                ),
            };
        }
        if proj.settings.color_engine == ColorEngine::Ocio && !proj.settings.working_space.is_some_and(|w| w.is_linear()) {
            return Err(bad(cmd, "the OCIO built-in config's working spaces are acescg and aces2065"));
        }
        if let Some(h) = str_p(p, "hdr") {
            proj.settings.hdr = match h.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
                "clip" | "off" | "none" => HdrMode::Clip,
                "compand" => HdrMode::Compand,
                "tonemap" | "tonemapped" => HdrMode::ToneMap,
                _ => return Err(bad(cmd, format!("hdr: clip|compand|toneMap, not `{h}`"))),
            };
        }
        if let Some(o) = p.get("outputSpace") {
            proj.settings.output_space = match o.as_str() {
                None | Some("none" | "None" | "" | "default") => None,
                Some(n) => Some(ColorSpace::parse(n).ok_or_else(|| bad(cmd, format!("outputSpace: srgb|rec709|rec2020|p3|rec2100pq|rec2100hlg, not `{n}`")))?),
            };
        }
        if let Some(r) = p.get("renderer").or_else(|| p.get("gpuAcceleration")) {
            proj.settings.gpu_acceleration = match r {
                Value::Bool(b) => *b,
                Value::String(s) => match effectcraft_render::Backend::parse(s) {
                    Some(effectcraft_render::Backend::Cpu) => false,
                    Some(_) => true,
                    None => return Err(bad("file.projectSettings", format!("renderer: gpu|software, not `{s}`"))),
                },
                _ => return Err(bad("file.projectSettings", "renderer: gpu|software")),
            };
        }
        if let Some(t) = str_p(p, "timeDisplay") {
            proj.settings.time_display = effectcraft_project::TimeDisplayStyle::parse(t)
                .ok_or_else(|| bad("file.projectSettings", format!("timeDisplay: timecode|frames|feet35|feet16, not `{t}`")))?;
        }
        Ok(())
    })?;
    Ok(serde_json::to_value(&s.project.settings).unwrap_or_default())
}

/// Video Rendering and Effects: report or set the renderer (Mercury GPU Acceleration when a GPU
/// adapter exists, else Mercury Software Only).
fn render_backend(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(b) = p.get("backend").or_else(|| p.get("renderer")) {
        let gpu = match b {
            Value::Bool(b) => *b,
            Value::String(v) => match effectcraft_render::Backend::parse(v) {
                Some(effectcraft_render::Backend::Cpu) => false,
                Some(_) => true,
                None => return Err(bad("render.backend", format!("backend: gpu|cpu, not `{v}`"))),
            },
            _ => return Err(bad("render.backend", "backend: gpu|cpu")),
        };
        if gpu != s.project.settings.gpu_acceleration {
            s.edit("Project Settings", None, |proj, _| {
                proj.settings.gpu_acceleration = gpu;
                Ok(())
            })?;
        }
    }
    Ok(backend_status(s))
}

/// The renderer setting and what renders with it.
pub fn backend_status(s: &Session) -> Value {
    let adapter = s.accel.as_ref().map(|a| a.name());
    let gpu = s.project.settings.gpu_acceleration;
    json!({
        "renderer": if gpu { "Mercury GPU Acceleration" } else { "Mercury Software Only" },
        "gpuAcceleration": gpu,
        "adapter": adapter,
        "active": if gpu && adapter.is_some() { "gpu" } else { "cpu" },
    })
}

fn cycle_depth(s: &mut Session, _: &Value) -> Result<Value> {
    s.edit("Project Bit Depth", None, |proj, _| {
        proj.settings.bit_depth = proj.settings.bit_depth.next();
        Ok(())
    })?;
    Ok(json!(s.project.settings.bit_depth.label()))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("file.newProject", "New Project", ["File", "New"], Some("Cmd+Alt+N"), "{}", always, new_project),
        cmd!("file.openDemoProject", "Open Demo Project", ["Help"], None, "{}", always, demo),
        cmd!("file.open", "Open Project...", ["File"], Some("Cmd+O"), "{path}", always, open),
        cmd!("file.save", "Save", ["File"], Some("Cmd+S"), "{path?}", always, save),
        cmd!("file.saveAs", "Save As...", ["File", "Save As"], Some("Cmd+Shift+S"), "{path}", always, save_as),
        cmd!("file.incrementAndSave", "Increment and Save", ["File"], Some("Cmd+Alt+Shift+S"), "{}", has_path, increment_save),
        cmd!("file.revert", "Revert", ["File"], None, "{}", has_path, revert),
        cmd!(
            "file.import",
            "File...",
            ["File", "Import"],
            Some("Cmd+I"),
            "{paths: [string], importAs?: footage|composition|compositionLayerSizes (Photoshop, PDF, Illustrator and EPS files), layer?: name|index (footage of one Photoshop layer)}",
            always,
            import
        ),
        cmd!(
            "file.projectSettings",
            "Project Settings...",
            ["File"],
            Some("Cmd+Alt+Shift+K"),
            "{bitDepth?: 8|16|32, colorEngine?: adobe|ocio, workingSpace?: none|srgb|rec709|rec2020|p3|acescg|aces2065, linearize?, blendLinear?, hdr?: clip|compand|toneMap, outputSpace?: srgb|rec709|rec2020|p3|rec2100pq|rec2100hlg, renderer?: gpu|software, timeDisplay?: timecode|frames|feet35|feet16}",
            always,
            project_settings
        ),
        cmd!("file.cycleBitDepth", "Cycle Project Bit Depth", [], None, "{}", always, cycle_depth),
        cmd!("render.backend", "Video Rendering and Effects", [], None, "{backend?: gpu|cpu}", always, render_backend),
    ]
}
