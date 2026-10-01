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
    s.replace_project(Project::default(), None);
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

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").ok_or_else(|| bad("file.open", "missing `path`"))?;
    let bytes = s.services.read_file(path).map_err(|e| EngineError::Other(format!("cannot read {path}: {e}")))?;
    let text = String::from_utf8(bytes).map_err(|_| EngineError::Other("not a text project file".into()))?;
    let proj = Project::from_json(&text)?;
    s.replace_project(proj, Some(path.to_string()));
    Ok(json!({"path": path}))
}

fn save_to(s: &mut Session, path: &str) -> Result<Value> {
    let json = s.project.to_json();
    s.services.write_file(path, json.as_bytes()).map_err(|e| EngineError::Other(format!("cannot write {path}: {e}")))?;
    s.path = Some(path.to_string());
    s.saved_revision = s.revision;
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
    let stem = cur.trim_end_matches(".ecproj");
    let (base, n) = match stem.rsplit_once('_') {
        Some((b, n)) if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => (b.to_string(), n.parse::<u32>().unwrap_or(0) + 1),
        _ => (stem.to_string(), 2),
    };
    save_to(s, &format!("{base}_{n}.ecproj"))
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
    let importer = s.importer.clone().ok_or_else(|| EngineError::Other("media import is not available in this build".into()))?;
    let mut ids = vec![];
    let mut errors = vec![];
    let mut probed = vec![];
    for path in &paths {
        match importer.probe(path) {
            Ok(f) => probed.push((path.clone(), f)),
            Err(e) => errors.push(format!("{path}: {e}")),
        }
    }
    s.edit("Import", None, |proj, st| {
        for (path, f) in probed {
            let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(path.clone());
            let label = match f.kind {
                FootageKind::Still | FootageKind::Sequence => Label::Lavender,
                FootageKind::Audio => Label::SeaFoam,
                FootageKind::Video => Label::Aqua,
            };
            let id = proj.add_item(&name, label, None, ItemKind::Footage(f));
            ids.push(id.0);
            st.project_selection = vec![id];
        }
        Ok(())
    })?;
    Ok(json!({"items": ids, "errors": errors}))
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
        if let Some(t) = str_p(p, "timeDisplay") {
            proj.settings.time_display =
                if t.eq_ignore_ascii_case("frames") { effectcraft_project::TimeDisplayStyle::Frames } else { effectcraft_project::TimeDisplayStyle::Timecode };
        }
        Ok(())
    })?;
    Ok(serde_json::to_value(&s.project.settings).unwrap_or_default())
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
        cmd!("file.openDemoProject", "Open Demo Project", ["File"], None, "{}", always, demo),
        cmd!("file.open", "Open Project…", ["File"], Some("Cmd+O"), "{path}", always, open),
        cmd!("file.save", "Save", ["File"], Some("Cmd+S"), "{path?}", always, save),
        cmd!("file.saveAs", "Save As…", ["File", "Save As"], Some("Cmd+Shift+S"), "{path}", always, save_as),
        cmd!("file.incrementAndSave", "Increment and Save", ["File"], Some("Cmd+Alt+Shift+S"), "{}", has_path, increment_save),
        cmd!("file.revert", "Revert", ["File"], None, "{}", has_path, revert),
        cmd!("file.import", "File…", ["File", "Import"], Some("Cmd+I"), "{paths: [string]}", always, import),
        cmd!("file.projectSettings", "Project Settings…", ["File"], Some("Cmd+Alt+Shift+K"), "{bitDepth?: 8|16|32, linearize?, timeDisplay?: timecode|frames}", always, project_settings),
        cmd!("file.cycleBitDepth", "Cycle Project Bit Depth", [], None, "{}", always, cycle_depth),
    ]
}
