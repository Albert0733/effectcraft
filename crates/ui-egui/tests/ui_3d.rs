//! Headless UI checks for the 3D viewer (egui_kittest). The `snapshot` test renders the window
//! with wgpu and writes PNGs when `EC_SNAPSHOT_DIR` is set (`cargo test -p effectcraft-ui-egui
//! --test ui_3d -- --ignored`); the other test only runs the UI logic.

use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use egui_kittest::Harness;
use serde_json::json;

fn app() -> EffectcraftApp {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("comp.open", json!({"comp": "3D Showcase"})).unwrap();
    s.execute("time.set", json!({"time": 5.0})).unwrap();
    EffectcraftApp::new(s)
}

/// Step the UI until background frame renders have landed.
fn settle(h: &mut Harness<'_, EffectcraftApp>) {
    for _ in 0..600 {
        h.step();
        if h.state().frames.inflight() == 0 && h.state().frames.last_ms.lock().map(|v| *v > 0.0).unwrap_or(false) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    for _ in 0..4 {
        h.step();
    }
}

#[test]
fn viewer_registers_3d_controls() {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| app());
    settle(&mut h);
    h.step();
    let ids: Vec<String> = h.state().auto.previous.iter().chain(h.state().auto.elements.iter()).map(|e| e.id.clone()).collect();
    for want in ["viewer.view3d", "viewer.renderer3d"] {
        assert!(ids.iter().any(|i| i == want), "missing {want}");
    }
    assert!(ids.iter().any(|i| i.starts_with("viewer.light.")), "light wireframe registered");
    // In a custom view the active camera is drawn as a wireframe too.
    h.state_mut().session.execute("view.3d.custom1", json!({})).unwrap();
    settle(&mut h);
    h.step();
    let ids: Vec<String> = h.state().auto.previous.iter().chain(h.state().auto.elements.iter()).map(|e| e.id.clone()).collect();
    assert!(ids.iter().any(|i| i.starts_with("viewer.camera.")), "camera wireframe registered");
}

#[test]
fn camera_and_light_dialogs_create_layers() {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| app());
    h.step();
    let ctx = h.ctx.clone();
    let n0 = h.state().session.active_comp().unwrap().layers.len();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.newCamera", json!({})).unwrap();
    assert_eq!(h.state().dialog, Some(effectcraft_ui_egui::Dialog::CameraSettings));
    h.step();
    let ok = h.state().auto.elements.iter().chain(h.state().auto.previous.iter()).find(|e| e.id == "dialog.camera.ok").map(|e| e.rect);
    assert!(ok.is_some(), "camera dialog OK registered");
    // OK through the dialog state (Enter).
    h.key_press(egui::Key::Enter);
    h.step();
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), n0 + 1);
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.newLight", json!({})).unwrap();
    assert_eq!(h.state().dialog, Some(effectcraft_ui_egui::Dialog::LightSettings));
    h.step();
    h.key_press(egui::Key::Enter);
    h.step();
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), n0 + 2);
    // Layer Settings on the new light reopens Light Settings for it.
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.settings", json!({})).unwrap();
    assert_eq!(h.state().dialog, Some(effectcraft_ui_egui::Dialog::LightSettings));
}

#[test]
#[ignore = "renders with wgpu; run with --ignored (set EC_SNAPSHOT_DIR to keep PNGs)"]
fn snapshot() {
    let dir = std::env::var("EC_SNAPSHOT_DIR").ok();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|_| app());
    let shots: [(&str, Option<&str>); 4] =
        [("active", None), ("custom1", Some("view.3d.custom1")), ("top", Some("view.3d.top")), ("left", Some("view.3d.left"))];
    for (name, cmd) in shots {
        if let Some(c) = cmd {
            h.state_mut().session.execute(c, json!({})).unwrap();
        }
        settle(&mut h);
        let img = h.render().expect("render");
        if let Some(d) = &dir {
            img.save(format!("{d}/ui3d_{name}.png")).unwrap();
        }
    }
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.newCamera", json!({})).unwrap();
    h.run_steps(3);
    let img = h.render().expect("render");
    if let Some(d) = &dir {
        img.save(format!("{d}/ui3d_camera_dialog.png")).unwrap();
    }
    h.state_mut().dialog = None;
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.newLight", json!({})).unwrap();
    h.run_steps(3);
    let img = h.render().expect("render");
    if let Some(d) = &dir {
        img.save(format!("{d}/ui3d_light_dialog.png")).unwrap();
    }
    // A 3D layer's rotation properties (R) and the selected layer's gizmo in the active camera.
    h.state_mut().dialog = None;
    h.state_mut().session.execute("view.3d.activeCamera", json!({})).unwrap();
    h.state_mut().session.execute("layer.select", json!({"layers": ["Card Blue"]})).unwrap();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.reveal.rotation", json!({})).unwrap();
    settle(&mut h);
    let img = h.render().expect("render");
    if let Some(d) = &dir {
        img.save(format!("{d}/ui3d_reveal_rotation.png")).unwrap();
    }
}
