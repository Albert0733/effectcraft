//! RAM preview frames are keyed by content: editing one comp keeps the cached frames of another,
//! and undo finds the frames of the state it returns to.

use effectcraft_engine::Session;
use effectcraft_engine::project::ItemId;
use effectcraft_ui_egui::EffectcraftApp;
use egui_kittest::Harness;
use serde_json::json;

fn settle(h: &mut Harness<'_, EffectcraftApp>) {
    for _ in 0..600 {
        h.step();
        if h.state().frames.inflight() == 0 && h.state().frames.last_ms.lock().map(|v| *v > 0.0).unwrap_or(false) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run_steps(2);
}

fn shown_cached(h: &Harness<'_, EffectcraftApp>, comp: u64) -> bool {
    let app = h.state();
    let key = effectcraft_ui_egui::frames::FrameKey { frame: 0, ..app.shown_series(ItemId(comp)) };
    app.frames.is_cached(&key)
}

#[test]
fn an_edit_in_another_comp_keeps_the_cached_frames_and_undo_finds_them_again() {
    let mut s = Session::default();
    let b = s.execute("comp.new", json!({"name": "B", "width": 64, "height": 36, "duration": 1})).unwrap()["comp"].as_u64().unwrap();
    let a = s.execute("comp.new", json!({"name": "A", "width": 64, "height": 36, "duration": 1})).unwrap()["comp"].as_u64().unwrap();
    s.execute("layer.newSolid", json!({"color": "#3080ff"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| EffectcraftApp::new(s));
    settle(&mut h);
    assert!(shown_cached(&h, a), "A's frame rendered");
    // Edit B (open it, add a layer), then come back to A: its frame is still in RAM.
    h.state_mut().session.execute("comp.open", json!({"comp": b})).unwrap();
    h.state_mut().session.execute("layer.newSolid", json!({"color": "#ff8030"})).unwrap();
    settle(&mut h);
    let b_frame = h.state().shown_series(ItemId(b));
    h.state_mut().session.execute("comp.open", json!({"comp": a})).unwrap();
    assert!(shown_cached(&h, a), "A's frame survived the edit in B");
    // Edit A, then undo: the frame of the state undo returns to is found again.
    let before = h.state().shown_series(ItemId(a));
    h.state_mut().session.execute("layer.newSolid", json!({"color": "#20c040"})).unwrap();
    settle(&mut h);
    assert_ne!(h.state().shown_series(ItemId(a)), before, "A's content changed");
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().shown_series(ItemId(a)), before);
    assert!(shown_cached(&h, a), "undo finds the earlier frame");
    // B's frame is still there too.
    assert!(h.state().frames.is_cached(&effectcraft_ui_egui::frames::FrameKey { frame: 0, ..b_frame }));
}
