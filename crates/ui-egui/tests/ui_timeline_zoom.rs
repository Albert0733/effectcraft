//! Zooming the Timeline's time ruler with real input: Alt+wheel zooms out until the whole comp
//! shows (#158).

use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Modifiers, MouseWheelUnit, Pos2, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

/// A 10-minute comp with one solid, zoomed in on its start.
fn harness() -> (Harness<'static, EffectcraftApp>, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Long", "width": 320, "height": 180, "frameRate": 30, "duration": 600})).unwrap();
    let layer = s.execute("layer.newSolid", json!({"name": "Plate", "color": "#406080"})).unwrap()["layer"].as_u64().unwrap();
    let mut app = EffectcraftApp::new(s);
    app.ui.timeline.pps = Some(200.0);
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| app);
    h.run_steps(3);
    (h, layer)
}

/// A point over the layer's bar in the time graph.
fn over_bar(h: &Harness<'_, EffectcraftApp>, layer: u64) -> Pos2 {
    let e = h.state().auto.find(&format!("timeline.layer.{layer}.bar")).expect("layer bar").clone();
    pos2(e.rect[0] + 40.0, e.rect[1] + e.rect[3] / 2.0)
}

fn alt_wheel(h: &mut Harness<'_, EffectcraftApp>, at: Pos2, dy: f32) {
    h.event(Event::PointerMoved(at));
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.event(Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, dy), modifiers: Modifiers::ALT, phase: egui::TouchPhase::Move });
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
}

/// #158: on a long comp, Alt+wheel zooms out all the way to the whole comp (and no further).
#[test]
fn alt_wheel_zooms_out_until_the_whole_comp_shows() {
    let (mut h, layer) = harness();
    for _ in 0..80 {
        let at = over_bar(&h, layer);
        alt_wheel(&mut h, at, -120.0);
        if h.state().ui.timeline.pps.is_none() {
            break;
        }
    }
    let tl = &h.state().ui.timeline;
    assert_eq!((tl.pps, tl.start), (None, 0.0), "fits the whole comp: {:?}", tl.pps);
    // The bar spans (nearly) the whole time graph.
    let bar = h.state().auto.find(&format!("timeline.layer.{layer}.bar")).unwrap().rect;
    let ruler = h.state().auto.find("timeline.ruler").unwrap().rect;
    assert!(bar[2] > ruler[2] * 0.95, "bar {bar:?} ruler {ruler:?}");
    // Zooming in again works from there.
    let at = over_bar(&h, layer);
    alt_wheel(&mut h, at, 240.0);
    assert!(h.state().ui.timeline.pps.is_some());
}
