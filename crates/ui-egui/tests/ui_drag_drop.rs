//! Dragging Project items into the Timeline (egui_kittest, real pointer drags): they land where
//! they are dropped, between layers and, over the time graph, starting there (#89).

use effectcraft_engine::Session;
use effectcraft_engine::time::Tick;
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Modifiers, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

/// Comp "Main" (4 s at 30 fps, current time 1 s) with solids Top, Middle and Bottom, and comp
/// "Clip" in the Project panel to drag in.
fn harness() -> (Harness<'static, EffectcraftApp>, u64) {
    let mut s = Session::default();
    let clip = s.execute("comp.new", json!({"name": "Clip", "width": 160, "height": 90, "frameRate": 30, "duration": 1})).unwrap()["comp"].as_u64().unwrap();
    s.execute("comp.new", json!({"name": "Main", "width": 320, "height": 180, "frameRate": 30, "duration": 4})).unwrap();
    for name in ["Bottom", "Middle", "Top"] {
        s.execute("layer.newSolid", json!({"name": name, "color": "#406080"})).unwrap();
    }
    s.execute("time.set", json!({"time": 1.0})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    (h, clip)
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}")).clone();
    Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]))
}

/// Press on `from`, move to `to` in steps, release there (with `modifiers` held).
fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2, modifiers: Modifiers) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::ModifiersChanged(modifiers));
    for k in 1..=10 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 10.0)));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(3);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
}

/// Layer names top to bottom, and the In point of `name`.
fn stack(h: &Harness<'_, EffectcraftApp>) -> Vec<String> {
    h.state().session.active_comp().unwrap().layers.iter().map(|l| l.name.clone()).collect()
}

fn in_point(h: &Harness<'_, EffectcraftApp>, index: usize) -> f64 {
    h.state().session.active_comp().unwrap().layers[index].in_point.seconds()
}

fn layer_id(h: &Harness<'_, EffectcraftApp>, name: &str) -> u64 {
    h.state().session.active_comp().unwrap().layers.iter().find(|l| l.name == name).unwrap().id.0
}

/// #89: dropped on the layer outline, an item goes in between the layers there; over the time
/// graph it also starts where it was dropped, or at the current time with Shift.
#[test]
fn project_items_land_where_they_are_dropped_in_the_timeline() {
    let (mut h, clip) = harness();
    let item = rect(&h, &format!("project.item.{clip}.name")).center();
    let frame = 1.0 / 30.0;

    // Between Top and Middle in the outline: the lower half of Top's row.
    let top = rect(&h, &format!("timeline.layer.{}.row", layer_id(&h, "Top")));
    drag(&mut h, item, pos2(top.center().x, top.max.y - 2.0), Modifiers::NONE);
    assert_eq!(stack(&h), ["Top", "Clip", "Middle", "Bottom"]);
    // (Settings ▸ General ▸ Create Layers at Composition Start Time, on by default.)
    assert_eq!(in_point(&h, 1), 0.0, "starts where new layers start");
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    h.run_steps(2);

    // Over the time graph at 2 s, on the upper half of Bottom's row: above Bottom, starting at 2 s.
    let bottom = layer_id(&h, "Bottom");
    let row = rect(&h, &format!("timeline.layer.{bottom}.row"));
    let bar = rect(&h, &format!("timeline.layer.{bottom}.bar"));
    let at_2s = bar.min.x + bar.width() * 0.5;
    drag(&mut h, item, pos2(at_2s, row.min.y + 2.0), Modifiers::NONE);
    assert_eq!(stack(&h), ["Top", "Middle", "Clip", "Bottom"]);
    assert!((in_point(&h, 2) - 2.0).abs() <= frame + 1e-9, "starts where it was dropped: {}", in_point(&h, 2));
    assert_eq!(
        Tick::from_seconds_f64(in_point(&h, 2)),
        h.state().session.active_comp().unwrap().frame_rate.snap_nearest(Tick::from_seconds_f64(in_point(&h, 2))),
        "on a frame"
    );
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    h.run_steps(2);

    // Shift: at the current time; below the last layer: at the bottom of the stack.
    let row = rect(&h, &format!("timeline.layer.{bottom}.row"));
    drag(&mut h, item, pos2(at_2s, row.max.y + 30.0), Modifiers::SHIFT);
    assert_eq!(stack(&h), ["Top", "Middle", "Bottom", "Clip"]);
    assert!((in_point(&h, 3) - 1.0).abs() < 1e-9, "Shift starts it at the current time");
}
