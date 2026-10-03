//! M13.1 UI: ScriptUI dialogs, palettes and dockable panels drawn from the session's script
//! windows (clicks reach the script's handlers), the branching History panel, and the Window /
//! File ▸ Scripts menus listing scripts (egui_kittest, UI logic only).

use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::dock::PanelKind;
use egui::{Event, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn harness() -> Harness<'static, EffectcraftApp> {
    let mut s = effectcraft_host::session();
    s.execute("comp.new", json!({"name": "Main", "width": 320, "height": 180, "duration": 4})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    h
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}"));
    Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]))
}

fn click(h: &mut Harness<'_, EffectcraftApp>, id: &str) {
    let c = rect(h, id).center();
    h.input_mut().events.push(Event::PointerMoved(c));
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: c, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }
    h.run_steps(3);
}

fn layer_names(h: &Harness<'_, EffectcraftApp>) -> Vec<String> {
    h.state().session.active_comp().map(|c| c.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default()
}

#[test]
fn script_windows_are_drawn_and_clickable() {
    let mut h = harness();
    let code = r#"
      var w = new Window("palette", "Quick Solid");
      var n = w.add("edittext", undefined, "Card");
      n.characters = 10;
      var go = w.add("button", undefined, "Add", { name: "add" });
      go.onClick = function () { app.project.activeItem.layers.addSolid([0, 1, 0], n.text, 50, 50, 1); };
      w.show();
    "#;
    let r = h.state_mut().session.execute("script.run", json!({"code": code, "name": "quick.jsx"})).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    h.run_steps(3);
    let win = h.state().session.script_ui.windows[0].id;
    // Every control has an automation id (by id and by name).
    for id in [format!("scriptui.{win}"), format!("scriptui.{win}.1"), format!("scriptui.{win}.2"), format!("scriptui.{win}.add")] {
        assert!(h.state().auto.find(&id).is_some(), "missing {id}");
    }
    click(&mut h, &format!("scriptui.{win}.add"));
    assert_eq!(layer_names(&h), ["Card"]);
    // A modal dialog: the script waits in show() until OK is clicked.
    let code = r#"
      var d = new Window("dialog", "Confirm");
      d.add("statictext", undefined, "Make a null?");
      var g = d.add("group");
      g.add("button", undefined, "Cancel", { name: "cancel" });
      g.add("button", undefined, "OK", { name: "ok" });
      if (d.show() === 1) app.project.activeItem.layers.addNull().name = "Yes";
    "#;
    let r = h.state_mut().session.execute("script.run", json!({"code": code, "name": "confirm.jsx"})).unwrap();
    assert_eq!(r["waiting"], true, "{r}");
    h.run_steps(3);
    let d = h.state().session.script_ui.windows.iter().find(|w| w.modal).unwrap().id;
    click(&mut h, &format!("scriptui.{d}.ok"));
    assert_eq!(layer_names(&h), ["Yes", "Card"]);
    assert!(h.state().session.script_ui.windows.iter().all(|w| !w.modal));
    // Window ▸ <panel>: the sample ScriptUI panel docks as its own tab.
    h.state_mut().session.execute("window.scriptPanel", json!({"name": "Layer Tools.jsx"})).unwrap();
    h.run_steps(3);
    let panel = h.state().session.script_ui.windows.iter().find(|w| w.script == "Layer Tools.jsx").unwrap().id;
    assert!(h.state().ui.dock.contains(PanelKind::ScriptPanel(panel)), "the panel docks");
    assert!(h.state().auto.find(&format!("scriptui.{panel}")).is_some());
    // The menus list scripts and panels.
    let window_menu = effectcraft_ui_egui::menus::dynamic_entries(h.state(), "Window");
    assert!(window_menu.iter().any(|(l, c, _)| l == "Layer Tools.jsx" && c == "window.scriptPanel"));
    let scripts = effectcraft_ui_egui::menus::dynamic_entries(h.state(), "Scripts");
    assert!(scripts.iter().any(|(l, c, _)| l == "Rename Layers.jsx" && c == "file.runScript"));
    // Closing the panel's tab closes its script window.
    h.state_mut().close_panel(PanelKind::ScriptPanel(panel));
    h.run_steps(2);
    assert!(h.state().session.script_ui.window(panel).is_none());
    h.state_mut().session.execute("scriptui.close", json!({})).unwrap();
}

#[test]
fn history_panel_jumps_between_branches() {
    let mut h = harness();
    for n in ["A", "B"] {
        h.state_mut().session.execute("layer.newSolid", json!({"name": n, "color": "#ffffff", "width": 10, "height": 10})).unwrap();
    }
    h.state_mut().session.undo();
    h.state_mut().session.execute("layer.newSolid", json!({"name": "C", "color": "#ffffff", "width": 10, "height": 10})).unwrap();
    assert_eq!(layer_names(&h), ["C", "A"]);
    h.state_mut().show_panel(PanelKind::History);
    h.run_steps(3);
    // Original, New Composition, A, C, and B on its branch.
    let states = h.state().session.history_tree();
    assert_eq!(states.len(), 5);
    let b = states.iter().find(|n| n.depth == 1).unwrap().index;
    click(&mut h, &format!("history.state.{b}"));
    assert_eq!(layer_names(&h), ["B", "A"]);
    let c = h.state().session.history_tree().iter().find(|n| n.depth == 1).unwrap().index;
    click(&mut h, &format!("history.state.{c}"));
    assert_eq!(layer_names(&h), ["C", "A"]);
}
