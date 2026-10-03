//! Headless checks for the M13.5 long-tail UI: horizontal scrolling of the Timeline outline and
//! Project panel columns (egui_kittest, UI logic only).

use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn harness(w: f32) -> Harness<'static, EffectcraftApp> {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Tail", "width": 640, "height": 360, "duration": 4})).unwrap();
    s.execute("layer.newSolid", json!({"name": "Plate", "color": "#406080", "width": 640, "height": 360})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(w, 900.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    h
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}"));
    Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]))
}

fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(from));
    h.input_mut().events.push(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for i in 1..=8 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

#[test]
fn timeline_outline_columns_scroll_horizontally() {
    let mut h = harness(1100.0);
    // Every column on: wider than the outline pane.
    let ctx = h.ctx.clone();
    for c in ["keys", "comment", "modes", "parent", "in", "out", "duration", "stretch"] {
        effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.column", json!({"column": c, "visible": true})).unwrap();
    }
    h.run_steps(3);
    let bar = rect(&h, "timeline.outlineScroll");
    let name0 = rect(&h, "timeline.header.name").min.x;
    assert_eq!(h.state().ui.timeline.outline_scroll, 0.0);
    drag(&mut h, pos2(bar.min.x + 8.0, bar.center().y), pos2(bar.max.x + 50.0, bar.center().y));
    h.run_steps(2);
    let s = h.state().ui.timeline.outline_scroll;
    assert!(s > 50.0, "{s}");
    let name1 = rect(&h, "timeline.header.name").min.x;
    assert!((name0 - name1 - s).abs() < 1.0, "the columns moved by the scroll: {name0} → {name1} ({s})");
    // The scroll is clamped to the overflow and resets when the columns fit again.
    for c in ["keys", "comment", "modes", "parent", "in", "out", "duration", "stretch"] {
        effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.column", json!({"column": c, "visible": false})).unwrap();
    }
    h.run_steps(3);
    assert_eq!(h.state().ui.timeline.outline_scroll, 0.0);
    assert!(h.state().auto.find("timeline.outlineScroll").is_none());
}

#[test]
fn project_panel_columns_scroll_horizontally() {
    let mut h = harness(1400.0);
    for c in ["type", "size", "duration", "fps", "path", "comment"] {
        effectcraft_ui_egui::panels::project::set_column(&mut h.state_mut().ui.project_columns, c, true);
    }
    h.run_steps(3);
    let bar = rect(&h, "project.hscroll");
    let x0 = rect(&h, "project.sort.comment").min.x;
    let name0 = rect(&h, "project.sort.name").min.x;
    drag(&mut h, pos2(bar.min.x + 4.0, bar.center().y), pos2(bar.max.x + 40.0, bar.center().y));
    h.run_steps(2);
    let s = h.state().ui.project_hscroll;
    assert!(s > 20.0, "{s}");
    assert!(rect(&h, "project.sort.comment").min.x < x0 - 20.0);
    assert_eq!(rect(&h, "project.sort.name").min.x, name0, "Name stays frozen");
}
