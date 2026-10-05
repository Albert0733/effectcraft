//! Comp tabs and tab drags (egui_kittest): double-clicking a comp in the Project panel opens it
//! as its own Timeline tab (not a rename, and not in place of the comp shown), the tabs switch and
//! close comps, and a dragged tab lands where its insertion mark shows.

use effectcraft_engine::Session;
use effectcraft_engine::project::ItemId;
use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::dock::{DockNode, PanelKind};
use egui::{Event, Pos2, pos2};
use egui_kittest::Harness;
use serde_json::json;

/// "Pre" and "Main" (Main holds Pre); only Main is open.
fn harness() -> (Harness<'static, EffectcraftApp>, u64, u64) {
    let mut s = Session::default();
    let pre = s.execute("comp.new", json!({"name": "Pre", "width": 320, "height": 180, "duration": 4})).unwrap()["comp"].as_u64().unwrap();
    let main = s.execute("comp.new", json!({"name": "Main", "width": 320, "height": 180, "duration": 4})).unwrap()["comp"].as_u64().unwrap();
    s.execute("layer.addItem", json!({"item": pre})).unwrap();
    s.state.open_comps = vec![ItemId(main)];
    s.state.active_comp = Some(ItemId(main));
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    (h, pre, main)
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> egui::Rect {
    let e = h.state().auto.previous.iter().chain(h.state().auto.elements.iter()).find(|e| e.id == id).cloned().unwrap_or_else(|| {
        let have: Vec<&String> = h.state().auto.elements.iter().map(|e| &e.id).filter(|i| i.starts_with("panel.") || i.starts_with("project.")).collect();
        panic!("no element {id}; have {have:?}")
    });
    egui::Rect::from_min_size(pos2(e.rect[0], e.rect[1]), egui::vec2(e.rect[2], e.rect[3]))
}

fn click_n(h: &mut Harness<'_, EffectcraftApp>, p: Pos2, n: u32) {
    h.event(Event::PointerMoved(p));
    h.step();
    // All in one frame, straight into the input (the harness' event queue spreads presses over
    // frames, longer apart than a double-click).
    for _ in 0..n {
        h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    }
    h.step();
    h.run_steps(2);
}

fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for k in 1..=6 {
        let f = k as f32 / 6.0;
        h.event(Event::PointerMoved(from + (to - from) * f));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.step();
    h.run_steps(2);
}

/// The tabs of the docked group holding `p`, in order.
fn group_of(n: &DockNode, p: PanelKind) -> Option<Vec<PanelKind>> {
    match n {
        DockNode::Split { a, b, .. } => group_of(a, p).or_else(|| group_of(b, p)),
        DockNode::Tabs { panels, .. } => panels.contains(&p).then(|| panels.clone()),
        DockNode::Stack { entries } => entries.iter().any(|e| e.panel == p).then(|| entries.iter().map(|e| e.panel).collect()),
    }
}

fn open_comps(h: &Harness<'_, EffectcraftApp>) -> Vec<u64> {
    h.state().session.state.open_comps.iter().map(|c| c.0).collect()
}

#[test]
fn project_double_click_opens_a_comp_as_its_own_timeline_tab() {
    let (mut h, pre, main) = harness();
    // Double-click the precomp's name in the Project panel: it opens (no rename) next to Main.
    let name = rect(&h, &format!("project.item.{pre}.name"));
    click_n(&mut h, name.center(), 2);
    assert_eq!(open_comps(&h), vec![main, pre]);
    assert_eq!(h.state().session.state.active_comp, Some(ItemId(pre)));
    assert_eq!(h.state().session.project.item(ItemId(pre)).map(|i| i.name.clone()), Some("Pre".into()));
    // One Timeline tab per comp, the newest last; the shown one answers to the panel's id.
    let (tm, tp) = (rect(&h, &format!("panel.tab.Timeline.{main}")), rect(&h, &format!("panel.tab.Timeline.{pre}")));
    assert!(tp.min.x > tm.max.x, "{tm:?} {tp:?}");
    assert_eq!(rect(&h, "panel.tab.Timeline"), tp);
    // Clicking Main's tab shows Main again; both stay open.
    click_n(&mut h, tm.center(), 1);
    assert_eq!(h.state().session.state.active_comp, Some(ItemId(main)));
    assert_eq!(open_comps(&h), vec![main, pre]);
    // × closes the shown comp's Timeline only: the panel stays with Pre.
    let close = rect(&h, "panel.tab.Timeline.close");
    click_n(&mut h, close.center(), 1);
    assert_eq!(open_comps(&h), vec![pre]);
    assert_eq!(h.state().session.state.active_comp, Some(ItemId(pre)));
    assert!(h.state().ui.dock.contains(PanelKind::Timeline));
    // Double-clicking an open comp again adds no second tab.
    let name = rect(&h, &format!("project.item.{pre}.name"));
    click_n(&mut h, name.center(), 2);
    assert_eq!(open_comps(&h), vec![pre]);
}

#[test]
fn dragged_tabs_land_where_the_insertion_mark_is() {
    let (mut h, _, _) = harness();
    let dock = |h: &Harness<'_, EffectcraftApp>| h.state().ui.dock.clone();
    assert_eq!(group_of(&dock(&h), PanelKind::Timeline), Some(vec![PanelKind::Timeline, PanelKind::RenderQueue]));
    // Reorder within the group: Render Queue dropped on the left half of the Timeline tab goes
    // before it.
    let tl = rect(&h, "panel.tab.Timeline");
    let rq = rect(&h, "panel.tab.RenderQueue");
    drag(&mut h, rq.center(), pos2(tl.min.x + tl.width() * 0.2, tl.center().y));
    assert_eq!(group_of(&dock(&h), PanelKind::Timeline), Some(vec![PanelKind::RenderQueue, PanelKind::Timeline]));
    // Another group's tab dropped between them lands between them.
    let (rq, tl) = (rect(&h, "panel.tab.RenderQueue"), rect(&h, "panel.tab.Timeline"));
    let fx = rect(&h, "panel.tab.EffectControls");
    drag(&mut h, fx.center(), pos2((rq.max.x + tl.min.x) / 2.0 + 2.0, tl.center().y));
    assert_eq!(group_of(&dock(&h), PanelKind::Timeline), Some(vec![PanelKind::RenderQueue, PanelKind::EffectControls, PanelKind::Timeline]));
    // Past the last tab: last.
    let (ec, tl) = (rect(&h, "panel.tab.EffectControls"), rect(&h, "panel.tab.Timeline"));
    drag(&mut h, ec.center(), pos2(tl.max.x + 30.0, tl.center().y));
    assert_eq!(group_of(&dock(&h), PanelKind::Timeline), Some(vec![PanelKind::RenderQueue, PanelKind::Timeline, PanelKind::EffectControls]));
}

/// `cargo test -p effectcraft-ui-egui --test ui_tabs -- --ignored`: the insertion mark mid-drag
/// (`target/test-out/tab-drag.png`).
#[test]
#[ignore]
fn tab_drag_snapshot() {
    let (mut h, _, _) = harness();
    let (tl, rq) = (rect(&h, "panel.tab.Timeline"), rect(&h, "panel.tab.RenderQueue"));
    let fx = rect(&h, "panel.tab.EffectControls");
    let to = pos2((tl.max.x + rq.min.x) / 2.0, tl.center().y);
    h.event(Event::PointerMoved(fx.center()));
    h.step();
    h.event(Event::PointerButton { pos: fx.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for k in 1..=6 {
        h.event(Event::PointerMoved(fx.center() + (to - fx.center()) * (k as f32 / 6.0)));
        h.step();
    }
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/test-out");
    std::fs::create_dir_all(dir).unwrap();
    h.render().expect("render").save(format!("{dir}/tab-drag.png")).unwrap();
}

/// A precomp layer's bar shows its comp's markers; a double-click opens that comp at the marker.
#[test]
fn nested_comp_markers_show_on_the_precomp_bar() {
    let (mut h, pre, main) = harness();
    {
        let s = &mut h.state_mut().session;
        s.execute("comp.open", json!({"comp": pre})).unwrap();
        s.execute("markers.set", json!({"new": true, "time": 1.0, "comment": "beat"})).unwrap();
        s.execute("comp.open", json!({"comp": main})).unwrap();
    }
    h.run_steps(4);
    let l = h.state().session.project.comp(ItemId(main)).unwrap().layers[0].id.0;
    let id = format!("timeline.layer.{l}.nestedMarker.0");
    let m = rect(&h, &id);
    let e = h.state().auto.find(&id).unwrap().clone();
    assert_eq!(e.label, "beat (marker in Pre)");
    click_n(&mut h, m.center(), 2);
    assert_eq!(h.state().session.active_comp_id(), Some(ItemId(pre)));
    // (29.97 fps: the marker's frame is at 1.001 s)
    assert!((h.state().session.time().seconds() - 1.0).abs() < 0.034, "{}", h.state().session.time().seconds());
}
