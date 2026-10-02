//! Settings.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use crate::Session;
use crate::config::{ConfigStore, MemoryConfig};
use crate::prefs::{PREFS_FILE, Prefs};

fn tmp(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("ec-settings-{}-{}-{name}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn session_with_comp() -> Session {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width": 320, "height": 180, "duration": 10})).unwrap();
    s
}

// ---------------------------------------------------------------- prefs model

#[test]
fn prefs_round_trip_through_json() {
    let mut p = Prefs::default();
    p.set("general.undoLevels", json!(12)).unwrap();
    p.set("labels.2.name", json!("Teal")).unwrap();
    p.set("autoSave.location", json!("custom")).unwrap();
    p.set("appearance.brightness", json!(0.25)).unwrap();
    p.push_recent("/a/b.ecproj");
    let back = Prefs::from_json(&p.to_json());
    assert_eq!(back, p);
    assert_eq!(Prefs::from_json(&Prefs::default().to_json()), Prefs::default());
}

#[test]
fn prefs_migrate_old_layout_and_keep_unknown_keys() {
    let old = r#"{
        "undoLevels": 50, "autoSaveMinutes": 7, "autoSaveVersions": 3, "theme": "light",
        "labelNames": ["Hot", "Sun"],
        "futureSetting": {"x": 1},
        "general": {"showToolTips": false, "notYetKnown": true},
        "memory": {"layerCacheMb": "not a number"}
    }"#;
    let p = Prefs::from_json(old);
    assert_eq!(p.version, crate::prefs::PREFS_VERSION);
    assert_eq!(p.general.undo_levels, 50);
    assert!(!p.general.show_tool_tips);
    assert_eq!(p.auto_save.interval_minutes, 7);
    assert_eq!(p.auto_save.max_versions, 3);
    assert_eq!(p.appearance.theme, "light");
    assert_eq!(p.labels[0].name, "Hot");
    assert_eq!(p.labels[1].name, "Sun");
    assert_eq!(p.labels[2].name, "Aqua");
    assert_eq!(p.labels.len(), 16);
    // A bad value falls back to that page's defaults; other pages survive.
    assert_eq!(p.memory.layer_cache_mb, 1024);
    // Unknown keys (a newer version's) are kept and written back.
    let again: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
    assert_eq!(again["futureSetting"], json!({"x": 1}));
    assert_eq!(again["general"]["notYetKnown"], json!(true));
    assert!(again.get("undoLevels").is_none(), "migrated keys leave the top level");
    // Garbage is defaults.
    assert_eq!(Prefs::from_json("not json"), Prefs::default());
}

#[test]
fn prefs_set_validates_keys_types_and_ranges() {
    let mut p = Prefs::default();
    assert!(p.set("general.nope", json!(1)).is_err());
    assert!(p.set("general.showToolTips", json!([1])).is_err());
    p.set("general.undoLevels", json!(500)).unwrap();
    assert_eq!(p.general.undo_levels, 99);
    p.set("general.undoLevels", json!("7")).unwrap();
    assert_eq!(p.general.undo_levels, 7);
    p.set("labels.0.color", json!("zzz")).unwrap();
    assert_eq!(p.labels[0].color, Prefs::default().labels[0].color);
    p.reset(Some("general")).unwrap();
    assert_eq!(p.general.undo_levels, 32);
    assert!(p.reset(Some("bogus")).is_err());
}

#[test]
fn every_schema_key_exists_and_docs_list_the_todo_settings() {
    let p = Prefs::default();
    let mut ids = vec![];
    for page in crate::prefs::pages() {
        assert!(crate::prefs::page_id(page.id).is_some(), "page {}", page.id);
        ids.push(page.id);
        for item in page.items {
            if let crate::prefs::Item::Setting { key, .. } | crate::prefs::Item::AudioDevices { key } = item {
                assert!(p.get(key).is_some(), "schema key `{key}` is not a setting");
            }
        }
    }
    // The Settings menu and the dialog list the same pages.
    for (path, e) in crate::menus::entries() {
        if e.command == "app.settings" {
            let page = e.params["page"].as_str().unwrap();
            assert!(ids.contains(&page), "{path:?} opens unknown page {page}");
        }
    }
    let docs = include_str!("../../../docs/preferences.md");
    for k in crate::prefs::todo_keys() {
        assert!(docs.contains(&format!("`{k}`")), "docs/preferences.md must list the not-yet-wired setting `{k}`");
    }
}

// ---------------------------------------------------------------- prefs effects

#[test]
fn prefs_commands_get_set_reset_and_persist() {
    let store = Arc::new(MemoryConfig::default());
    let mut s = Session { config: Some(store.clone()), ..Default::default() };
    s.execute_checked("prefs.set", json!({"key": "general.recentItems", "value": 4})).unwrap();
    assert_eq!(s.execute("prefs.get", json!({"key": "general.recentItems"})).unwrap(), json!(4));
    s.execute("prefs.set", json!({"values": {"autoSave.maxVersions": 9, "grids.gridSpacing": 50}})).unwrap();
    assert!(s.execute("prefs.set", json!({"key": "nope.nope", "value": 1})).is_err());
    let saved = store.read(PREFS_FILE).unwrap();
    let mut s2 = Session { config: Some(store.clone()), ..Default::default() };
    s2.load_settings();
    assert_eq!(s2.prefs.general.recent_items, 4);
    assert_eq!(s2.prefs.auto_save.max_versions, 9);
    assert!(saved.contains("\"maxVersions\": 9"));
    s2.execute("prefs.reset", json!({"page": "project"})).unwrap();
    assert_eq!(s2.prefs.auto_save.max_versions, 5);
    assert!(s2.execute("prefs.pages", json!({})).unwrap().as_array().unwrap().len() >= 17);
    s2.execute("prefs.open", json!({"page": "Auto-Save"})).unwrap();
    assert!(
        s2.drain_events().iter().any(|e| matches!(e, crate::Event::Frontend { command, params } if command == "app.settings" && params["page"] == "project"))
    );
}

#[test]
fn undo_levels_are_honoured() {
    let mut s = session_with_comp();
    s.execute("prefs.set", json!({"key": "general.undoLevels", "value": 3})).unwrap();
    for i in 0..6 {
        s.execute("layer.newNull", json!({"name": format!("N{i}")})).unwrap();
    }
    assert_eq!(s.history.undo.len(), 3);
    for _ in 0..3 {
        assert!(s.undo());
    }
    assert!(!s.undo());
    assert_eq!(s.active_comp().unwrap().layers.len(), 3);
    // Lowering the setting trims the history right away.
    s.execute("prefs.set", json!({"key": "general.undoLevels", "value": 1})).unwrap();
    assert!(s.history.undo.len() <= 1);
}

#[test]
fn renamed_labels_show_in_the_label_menu_and_apply_by_name() {
    let mut s = session_with_comp();
    s.execute("layer.newNull", json!({})).unwrap();
    s.execute("prefs.set", json!({"key": "labels.0.name", "value": "Hot Sauce"})).unwrap();
    s.execute("prefs.set", json!({"key": "labels.0.color", "value": "#102030"})).unwrap();
    let red = crate::menus::entries().into_iter().find(|(_, e)| e.command == "edit.label" && e.params["label"] == "Red").unwrap().1;
    assert_eq!(crate::menus::entry_label(&s, red), "Hot Sauce");
    assert_eq!(s.prefs.label_rgb(effectcraft_color::Label::Red), [0x10, 0x20, 0x30]);
    s.execute("edit.label", json!({"label": "Yellow"})).unwrap();
    s.execute("edit.label", json!({"label": "hot sauce"})).unwrap();
    let l = &s.active_comp().unwrap().layers[0];
    assert_eq!(l.label, effectcraft_color::Label::Red);
}

#[test]
fn cache_budgets_are_applied() {
    let mut s = Session::default();
    s.execute("prefs.set", json!({"key": "memory.layerCacheMb", "value": 128})).unwrap();
    assert_eq!(s.layer_cache.budget(), 128 << 20);
    struct Src(std::sync::Mutex<usize>);
    impl effectcraft_render::FootageSource for Src {
        fn frame(&self, _: effectcraft_project::ItemId, _: &effectcraft_project::Footage, _: effectcraft_time::Tick) -> Option<Arc<effectcraft_raster::Image>> {
            None
        }
        fn set_cache_budget(&self, b: usize) {
            *self.0.lock().unwrap() = b;
        }
    }
    let src = Arc::new(Src(Default::default()));
    s.footage = src.clone();
    s.execute("prefs.set", json!({"key": "memory.mediaCacheMb", "value": 256})).unwrap();
    assert_eq!(*src.0.lock().unwrap(), 256 << 20);
}

#[test]
fn new_layers_start_at_the_current_time_when_asked() {
    let mut s = session_with_comp();
    let nested = s.execute("comp.new", json!({"name": "Nested", "open": false})).unwrap()["comp"].as_u64().unwrap();
    s.set_time(effectcraft_time::Tick::from_seconds_f64(2.0));
    s.execute("layer.addItem", json!({"item": nested})).unwrap();
    assert_eq!(s.active_comp().unwrap().layers[0].start_time, effectcraft_time::Tick::ZERO);
    s.execute("prefs.set", json!({"key": "general.createLayersAtCompStart", "value": false})).unwrap();
    s.execute("layer.addItem", json!({"item": nested})).unwrap();
    assert_eq!(s.active_comp().unwrap().layers[0].start_time.seconds().round(), 2.0);
}

#[test]
fn default_spatial_interpolation_linear() {
    let mut s = session_with_comp();
    s.execute("layer.newNull", json!({})).unwrap();
    s.execute("prop.set", json!({"path": "transform/position", "value": [0, 0], "time": 0})).unwrap();
    s.execute("prop.toggleAnimation", json!({"path": "transform/position", "value": true})).ok();
    s.execute("prop.addKey", json!({"path": "transform/position", "time": 0})).unwrap();
    let auto =
        |s: &Session| s.active_comp().unwrap().layers[0].props.prop("transform/position").unwrap().keys.iter().map(|k| k.spatial_auto).collect::<Vec<_>>();
    assert!(auto(&s).iter().all(|a| *a));
    s.execute("prefs.set", json!({"key": "general.defaultSpatialLinear", "value": true})).unwrap();
    s.execute("prop.addKey", json!({"path": "transform/position", "time": 1, "value": [100, 50]})).unwrap();
    assert_eq!(auto(&s), vec![true, false]);
}

#[test]
fn new_project_template_and_default_renderer() {
    let d = tmp("template");
    let tpl = d.join("tpl.ecproj");
    let mut a = session_with_comp();
    a.execute("file.saveAs", json!({"path": tpl.to_string_lossy()})).unwrap();
    let mut s = Session::default();
    s.execute("prefs.set", json!({"values": {"project.useTemplate": true, "project.templatePath": tpl.to_string_lossy()}})).unwrap();
    s.execute("file.newProject", json!({})).unwrap();
    assert_eq!(s.project.comps().count(), 1);
    assert!(s.path.is_none(), "a template opens untitled");
    s.execute("prefs.set", json!({"key": "threeD.defaultRenderer", "value": "advanced"})).unwrap();
    s.execute("comp.new", json!({})).unwrap();
    assert_eq!(s.active_comp().unwrap().renderer, effectcraft_project::Renderer::Advanced3D);
    let _ = std::fs::remove_dir_all(d);
}
