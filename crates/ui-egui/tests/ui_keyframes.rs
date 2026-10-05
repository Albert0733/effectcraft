//! Headless checks for Timeline keyframes as a person uses them: dragging keys over many frames
//! in one gesture (the drag used to end after the first frame), Shift-snapping, and Ctrl+C /
//! Ctrl+V, which the windowing layer delivers as clipboard events rather than key presses.

use effectcraft_engine::Session;
use effectcraft_engine::project::LayerId;
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Pos2, pos2};
use egui_kittest::Harness;
use serde_json::json;

/// A 4 s, 30 fps comp with a Box layer whose Opacity has keys at 0 s and 1 s, revealed in the
/// Timeline; returns the harness, the layer and the property uid.
fn harness() -> (Harness<'static, EffectcraftApp>, LayerId, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Keys", "width": 320, "height": 180, "frameRate": 30, "duration": 4})).unwrap();
    s.execute("layer.newSolid", json!({"name": "Box", "color": "#e04020", "width": 80, "height": 80})).unwrap();
    let id = s.active_comp().unwrap().layers[0].id;
    for (t, v) in [(0.0, 0.0), (1.0, 100.0)] {
        s.execute("prop.addKey", json!({"layer": id.0, "path": "transform/opacity", "time": t, "value": v})).unwrap();
    }
    let uid = s.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().uid;
    s.execute("layer.select", json!({"layers": [id.0]})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.reveal.opacity", json!({})).unwrap();
    h.run_steps(4);
    (h, id, uid)
}

fn key_times(h: &Harness<'_, EffectcraftApp>, id: LayerId) -> Vec<f64> {
    let l = h.state().session.active_comp().unwrap().layer(id).unwrap().clone();
    l.props.prop("transform/opacity").unwrap().keys.iter().map(|k| (k.time.seconds() * 30.0).round() / 30.0).collect()
}

/// Screen centres of the property's keys, left to right.
fn keys(h: &Harness<'_, EffectcraftApp>, uid: u64) -> Vec<Pos2> {
    let mut ks: Vec<Pos2> =
        h.state().auto.query(&format!("timeline.key.{uid}.")).iter().map(|e| pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)).collect();
    ks.sort_by(|a, b| a.x.total_cmp(&b.x));
    ks
}

/// Press at `from`, move to `to` in small steps (a real drag), release.
fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2, modifiers: egui::Modifiers) {
    h.input_mut().events.push(Event::PointerMoved(from));
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(modifiers));
    h.input_mut().events.push(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.step();
    for i in 1..=24 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 24.0)));
        h.step();
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(Default::default()));
    h.run_steps(2);
}

#[test]
fn a_key_drag_follows_the_pointer_for_its_whole_length() {
    let (mut h, id, uid) = harness();
    let ks = keys(&h, uid);
    assert_eq!(ks.len(), 2);
    let px_per_s = ks[1].x - ks[0].x;
    // Drag the 1 s key to where 2.5 s is: many frames, one gesture.
    let to = pos2(ks[1].x + px_per_s * 1.5, ks[1].y);
    drag(&mut h, ks[1], to, Default::default());
    let ts = key_times(&h, id);
    assert!((ts[1] - 2.5).abs() <= 2.0 / 30.0, "the key follows the whole drag: {ts:?}");
    let steps = h.state().session.history.undo.iter().filter(|(l, _)| l == "Move Keyframes").count();
    assert_eq!(steps, 1, "one undo step per drag");
    // Shift snaps it to the current time indicator (at 0.5 s), from a few pixels away.
    h.state_mut().session.set_time(effectcraft_engine::time::Tick::from_seconds_f64(0.5));
    h.run_steps(2);
    let ks = keys(&h, uid);
    let near = pos2(ks[0].x + px_per_s * 0.5 + 4.0, ks[1].y);
    drag(&mut h, ks[1], near, egui::Modifiers { shift: true, ..Default::default() });
    assert_eq!(key_times(&h, id)[1], 0.5, "snapped to the current time");
}

#[test]
fn ctrl_c_and_ctrl_v_copy_and_paste_keys_at_the_current_time() {
    let (mut h, id, uid) = harness();
    let ks = keys(&h, uid);
    // Click the 1 s key to select it, Ctrl+C (a clipboard event, not a key press).
    h.input_mut().events.push(Event::PointerMoved(ks[1]));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: ks[1], button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: ks[1], button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    assert_eq!(h.state().session.state.selected_keys.len(), 1);
    h.input_mut().events.push(Event::Copy);
    h.step();
    let copied: Vec<String> = h
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(copied, ["EffectCraft: 1 keyframe"], "the system clipboard is filled, so Ctrl+V sends a paste event");
    assert!(h.state().session.state.clip_is_keys);
    // Move to 3 s, Ctrl+V: the key lands there.
    h.state_mut().session.set_time(effectcraft_engine::time::Tick::from_seconds_f64(3.0));
    h.input_mut().events.push(Event::Paste("EffectCraft: 1 keyframe".into()));
    h.run_steps(2);
    assert_eq!(key_times(&h, id), vec![0.0, 1.0, 3.0]);
}

fn click_with(h: &mut Harness<'_, EffectcraftApp>, p: Pos2, modifiers: egui::Modifiers) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(modifiers));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.step();
    h.input_mut().events.push(Event::ModifiersChanged(Default::default()));
    h.run_steps(2);
}

#[test]
fn shift_click_toggles_and_ctrl_click_switches_interpolation() {
    let (mut h, id, uid) = harness();
    let ks = keys(&h, uid);
    let shift = egui::Modifiers { shift: true, ..Default::default() };
    click_with(&mut h, ks[0], Default::default());
    click_with(&mut h, ks[1], shift);
    assert_eq!(h.state().session.state.selected_keys.len(), 2, "Shift+click adds");
    click_with(&mut h, ks[1], shift);
    assert_eq!(h.state().session.state.selected_keys.len(), 1, "and takes out again");
    let key =
        |h: &Harness<'_, EffectcraftApp>| h.state().session.active_comp().unwrap().layer(id).unwrap().props.prop("transform/opacity").unwrap().keys[0].clone();
    // Ctrl+click: Linear → Auto Bezier → Linear.
    click_with(&mut h, ks[0], egui::Modifiers::COMMAND);
    assert!(key(&h).auto_bezier, "Auto Bezier");
    click_with(&mut h, ks[0], egui::Modifiers::COMMAND);
    assert_eq!(key(&h).out_interp, effectcraft_engine::keyframe::Interp::Linear);
    // Ctrl+Alt+click: Hold.
    click_with(&mut h, ks[0], egui::Modifiers { alt: true, ..egui::Modifiers::COMMAND });
    assert_eq!(key(&h).out_interp, effectcraft_engine::keyframe::Interp::Hold);
}
