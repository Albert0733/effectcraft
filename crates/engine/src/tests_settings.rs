//! Settings and keyboard shortcut presets.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use crate::Session;
use crate::config::{ConfigStore, MemoryConfig};
use crate::prefs::{PREFS_FILE, Prefs};
use crate::shortcuts::{DEFAULT_PRESET, Keymaps, normalize};

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

// ---------------------------------------------------------------- shortcuts

#[test]
fn shortcut_text_normalizes() {
    assert_eq!(normalize("shift+cmd+k").as_deref(), Some("Cmd+Shift+K"));
    assert_eq!(normalize("Opt+Ctrl+f5").as_deref(), Some("Ctrl+Alt+F5"));
    assert_eq!(normalize("⌘⇧S").as_deref(), Some("Cmd+Shift+S"));
    assert_eq!(normalize("Cmd++").as_deref(), Some("Cmd++"));
    assert_eq!(normalize("space").as_deref(), Some("Space"));
    assert_eq!(normalize("Cmd+").as_deref(), None);
}

#[test]
fn defaults_have_no_conflicts_and_match_the_menus() {
    let s = Session::default();
    let t = s.shortcuts();
    assert!(t.all_conflicts().is_empty(), "{:?}", t.all_conflicts());
    assert_eq!(t.shortcut_of("file.save", &json!(null)), Some("Cmd+S"));
    assert_eq!(t.shortcut_of("app.settings", &json!({"page": "general"})), Some("Cmd+Alt+;"));
    assert!(t.bindables.len() > 500);
}

#[test]
fn shortcut_edits_detect_conflicts_and_create_a_custom_preset() {
    let store = Arc::new(MemoryConfig::default());
    let mut s = Session { config: Some(store.clone()), ..Default::default() };
    let r = s.execute_checked("shortcuts.set", json!({"command": "layer.newNull", "keys": "cmd+s"})).unwrap();
    assert_eq!(r["keys"], json!(["Cmd+S"]));
    assert_eq!(r["created"], json!("Custom"));
    assert!(r["conflicts"].as_array().unwrap().iter().any(|c| c["key"] == "file.save"), "{r}");
    assert_eq!(s.keymaps.active, "Custom");
    assert_eq!(s.shortcuts().shortcut_of("layer.newNull", &json!(null)), Some("Cmd+S"));
    assert!(!s.execute("shortcuts.conflicts", json!({})).unwrap().as_array().unwrap().is_empty());
    // The default preset is untouched.
    s.execute("shortcuts.preset", json!({"op": "select", "name": DEFAULT_PRESET})).unwrap();
    assert_eq!(s.shortcuts().shortcut_of("layer.newNull", &json!(null)), Some("Cmd+Alt+Shift+Y"));
    // Remove a shortcut; reset one.
    s.execute("shortcuts.preset", json!({"op": "select", "name": "Custom"})).unwrap();
    s.execute("shortcuts.set", json!({"command": "file.save", "keys": null})).unwrap();
    assert_eq!(s.shortcuts().shortcut_of("file.save", &json!(null)), None);
    s.execute("shortcuts.reset", json!({"command": "file.save"})).unwrap();
    assert_eq!(s.shortcuts().shortcut_of("file.save", &json!(null)), Some("Cmd+S"));
    // Bound-parameter entries are bindable too.
    s.execute("shortcuts.set", json!({"command": "app.settings", "params": {"page": "labels"}, "keys": ["F9"]})).unwrap();
    assert_eq!(s.shortcuts().shortcut_of("app.settings", &json!({"page": "labels"})), Some("F9"));
    // Persisted.
    let k = Keymaps::from_json(&store.read(crate::shortcuts::SHORTCUTS_FILE).unwrap());
    assert_eq!(k.active, "Custom");
    assert!(s.execute("shortcuts.set", json!({"command": "no.such", "keys": "F1"})).is_err());
    assert!(s.execute("shortcuts.set", json!({"command": "file.save", "keys": "Cmd+"})).is_err());
    let list = s.execute("shortcuts.list", json!({"keys": "F9"})).unwrap();
    assert!(list["commands"].as_array().unwrap().iter().any(|c| c["command"] == "app.settings" && c["params"]["page"] == "labels"), "{list}");
}

#[test]
fn shortcut_presets_duplicate_rename_delete_export_import() {
    let d = tmp("presets");
    let mut s = Session::default();
    s.execute("shortcuts.preset", json!({"op": "new", "name": "Mine"})).unwrap();
    s.execute("shortcuts.set", json!({"command": "edit.undo", "keys": ["Cmd+Z", "F2"]})).unwrap();
    s.execute("shortcuts.preset", json!({"op": "duplicate", "name": "Mine 2"})).unwrap();
    assert_eq!(s.keymaps.names(), vec![DEFAULT_PRESET, "Mine", "Mine 2"]);
    s.execute("shortcuts.preset", json!({"op": "rename", "name": "Mine 2", "newName": "Other"})).unwrap();
    assert!(s.execute("shortcuts.preset", json!({"op": "delete", "name": DEFAULT_PRESET})).is_err());
    let path = d.join("mine.json");
    s.execute("shortcuts.export", json!({"preset": "Mine", "path": path.to_string_lossy()})).unwrap();
    s.execute("shortcuts.preset", json!({"op": "delete", "name": "Mine"})).unwrap();
    s.execute("shortcuts.preset", json!({"op": "delete", "name": "Other"})).unwrap();
    assert_eq!(s.keymaps.active, DEFAULT_PRESET);
    let r = s.execute("shortcuts.import", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["preset"], json!("Mine"));
    assert_eq!(s.shortcuts().keys["edit.undo"], vec!["Cmd+Z".to_string(), "F2".into()]);
    s.execute("shortcuts.reset", json!({})).unwrap();
    assert_eq!(s.shortcuts().keys["edit.undo"], vec!["Cmd+Z".to_string()]);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn frontend_commands_are_bindable_with_panel_scope() {
    let mut s = Session::default();
    s.set_ui_commands(vec![
        crate::shortcuts::UiCommand { id: "timeline.zoomIn".into(), label: "Zoom In Time".into(), shortcut: Some("=".into()) },
        crate::shortcuts::UiCommand { id: "tool.hand".into(), label: "Hand Tool".into(), shortcut: Some("H".into()) },
    ]);
    let b = s.shortcuts().find("timeline.zoomIn").unwrap().clone();
    assert_eq!(b.scope, "Timeline");
    assert_eq!(s.shortcuts().shortcut_of("tool.hand", &json!(null)), Some("H"));
    // A panel shortcut conflicts with an application shortcut on the same keys.
    let c = s.shortcuts().conflicts(&b, "H");
    assert!(c.iter().any(|c| c.key == "tool.hand"));
}
