//! The Timeline panel.
//!
//! Left: current-time display, search, comp switches, then per layer A/V features (video, audio,
//! solo, lock), label, number, name, switches (shy, collapse, quality, fx, frame blend, motion
//! blur, adjustment, 3D), mode, track matte and parent, with twirl-down property trees (stopwatch,
//! keyframe navigator, scrubbable values). Right: time navigator, ruler with work area and cache
//! bar, layer duration bars, keyframes, the CTI, and the value graph editor.

use effectcraft_engine::color::BlendMode;
use effectcraft_engine::keyframe::Value;
use effectcraft_engine::project::{Comp, GroupKind, Layer, LayerId, LayerSource, MatteKind, Node, ParamUi, PropGroup, Property};
use effectcraft_engine::render::EvalCtx;
use effectcraft_engine::time::Tick;
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::{EffectcraftApp, widgets};

#[derive(Clone, Debug)]
enum RowKind {
    Layer,
    Group {
        uid: u64,
        name: String,
        open: bool,
        has_children: bool,
        fx: Option<bool>,
        /// Eye switch (layer styles).
        eye: Option<bool>,
    },
    Prop {
        uid: u64,
    },
    /// Inline expression editor under a property with an expression.
    Expr {
        uid: u64,
        lines: usize,
    },
    /// Audio > Waveform of a footage layer with audio (drawn from the item's peak summary).
    Waveform {
        item: u64,
    },
}

/// Synthetic group uid for a layer's Audio > Waveform twirl (never a real property uid).
const WAVE_BIT: u64 = 1 << 60;

/// Row height (expression editors grow with their text).
fn row_height(row: &Row, rh: f32) -> f32 {
    match row.kind {
        RowKind::Expr { lines, .. } => rh * lines.clamp(1, 8) as f32 + 6.0,
        RowKind::Waveform { .. } => rh * 3.0,
        _ => rh,
    }
}

/// Insert an expression editor row after every property that has an expression.
fn with_expr_rows(rows: Vec<Row>, comp: &Comp, closed: &std::collections::BTreeSet<u64>) -> Vec<Row> {
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let extra = match r.kind {
            RowKind::Prop { uid } if !closed.contains(&uid) => comp.layer(r.layer).and_then(|l| l.props.find(uid)).and_then(|p| p.expr.as_ref()).map(|e| Row {
                layer: r.layer,
                depth: r.depth,
                kind: RowKind::Expr { uid, lines: e.text.lines().count().max(1) },
            }),
            _ => None,
        };
        out.push(r);
        out.extend(extra);
    }
    out
}

/// A pick-whip drag in progress: (parent = 0 / property = 1, layer, prop uid, start point).
type PickWhip = (u8, u64, u64, Pos2);

fn pick_whip_id() -> egui::Id {
    egui::Id::new("tl-pickwhip")
}

#[derive(Clone, Debug)]
struct Row {
    layer: LayerId,
    depth: usize,
    kind: RowKind,
}

const AV_W: f32 = 76.0;
const LABEL_W: f32 = 20.0;
const NUM_W: f32 = 26.0;
const SW: f32 = 19.0;

struct Cols {
    av: f32,
    label: f32,
    num: f32,
    name: f32,
    switches: f32,
    mode: f32,
    trkmat: f32,
    parent: f32,
    end: f32,
}

fn cols(x0: f32, width: f32, show_modes: bool) -> Cols {
    let fixed = AV_W + LABEL_W + NUM_W + SW * 8.0 + if show_modes { 92.0 + 112.0 } else { 0.0 } + 116.0;
    let name_w = (width - fixed).max(120.0);
    let av = x0;
    let label = av + AV_W;
    let num = label + LABEL_W;
    let name = num + NUM_W;
    let switches = name + name_w;
    let mode = switches + SW * 8.0 + 6.0;
    let trkmat = mode + if show_modes { 92.0 } else { 0.0 };
    let parent = trkmat + if show_modes { 112.0 } else { 0.0 };
    Cols { av, label, num, name, switches, mode, trkmat, parent, end: parent + 116.0 }
}

/// Timeline horizontal mapping.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TMap {
    pub x0: f32,
    pub start: f64,
    pub pps: f64,
}

impl TMap {
    pub fn x(&self, secs: f64) -> f32 {
        self.x0 + ((secs - self.start) * self.pps) as f32
    }
    pub fn t(&self, x: f32) -> f64 {
        self.start + (x - self.x0) as f64 / self.pps
    }
}

fn tl_map_id() -> egui::Id {
    egui::Id::new("timeline-map")
}

/// Zoom the time ruler around the CTI.
pub fn zoom(app: &mut EffectcraftApp, ctx: &egui::Context, k: f64) {
    let Some((x0, w)) = ctx.data(|d| d.get_temp::<(f32, f32)>(egui::Id::new("timeline-graph-area"))) else { return };
    let comp = app.session.active_comp().cloned();
    let Some(comp) = comp else { return };
    let fit = (w as f64 - 20.0) / comp.duration.seconds().max(0.01);
    let pps = app.ui.timeline.pps.unwrap_or(fit);
    let npps = (pps * k).clamp(fit.min(1.0), 4000.0);
    let cti = app.session.time().seconds();
    let rel = (cti - app.ui.timeline.start) * pps;
    app.ui.timeline.start = (cti - rel / npps).max(0.0);
    app.ui.timeline.pps = Some(npps);
    let _ = x0;
}

/// Screen point of a layer bar (centre, or at a comp time).
pub fn locate(app: &EffectcraftApp, layer: u64, time: Option<f64>) -> Option<(f32, f32)> {
    let e = app.auto.find(&format!("timeline.layer.{layer}.bar"))?;
    let y = e.rect[1] + e.rect[3] / 2.0;
    match time {
        Some(t) => {
            // The ruler element's label carries "start,pps".
            let tm = app.auto.find("timeline.ruler")?;
            let (start, pps) = tm.label.split_once(',').and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?)))?;
            Some((tm.rect[0] + 6.0 + ((t - start) * pps) as f32, y))
        }
        None => Some((e.rect[0] + e.rect[2] / 2.0, y)),
    }
}

/// Start renaming the first selected layer.
pub fn begin_rename(app: &mut EffectcraftApp, ctx: &egui::Context) {
    if let Some(l) = app.session.state.selected_layers.first().and_then(|id| app.session.active_comp().and_then(|c| c.layer(*id))) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("tl-rename"), (l.id.0, l.name.clone())));
    }
}

fn group_visible(g: &PropGroup, layer: &Layer) -> bool {
    match g.match_id.as_str() {
        "masks" | "effects" => !g.children.is_empty(),
        "materialOptions" | "geometryOptions" => layer.is_3d(),
        "audio" => true,
        _ => true,
    }
}

fn prop_visible(p: &Property, layer: &Layer) -> bool {
    if matches!(p.ui, ParamUi::Hidden) {
        return false;
    }
    // Path Options show only the Path popup until a mask path is chosen.
    if let Some(po) = layer.props.group("text/pathOptions")
        && p.match_id != "path"
        && po.get(&p.match_id).is_some_and(|q| q.uid == p.uid)
        && po.get("path").is_some_and(|q| q.value.as_enum() == 0 && q.keys.is_empty())
    {
        return false;
    }
    if p.three_d_only && !layer.is_3d() {
        return false;
    }
    if p.two_d_only && layer.is_3d() {
        return false;
    }
    // One-node cameras (and lights with auto-orient off) have no Point of Interest.
    if p.match_id == "poi" && (layer.is_camera() || layer.is_light()) && layer.auto_orient != effectcraft_engine::project::AutoOrient::TowardsPointOfInterest {
        return false;
    }
    // Separate Dimensions: X/Y/Z Position replace Position.
    if p.match_id == "position" && layer.transform().is_some_and(|tr| tr.get("positionX").is_some() && tr.get("position").is_some_and(|q| q.uid == p.uid)) {
        return false;
    }
    true
}

fn reveal_matches(p: &Property, path_matches: &[&str], kind: &str) -> bool {
    match kind {
        "animated" => p.is_animated() || p.has_expression(),
        _ => path_matches.contains(&p.match_id.as_str()),
    }
}

fn build_rows(app: &EffectcraftApp, comp: &Comp) -> Vec<Row> {
    let tl = &app.ui.timeline;
    let search = tl.search.to_lowercase();
    let mut rows = Vec::new();
    for l in &comp.layers {
        if comp.hide_shy && l.switches.shy {
            continue;
        }
        if !search.is_empty() && !l.name.to_lowercase().contains(&search) {
            continue;
        }
        rows.push(Row { layer: l.id, depth: 0, kind: RowKind::Layer });
        if !tl.open_layers.contains(&l.id.0) {
            continue;
        }
        if let Some(kind) = tl.reveal.first() {
            let (group, props): (&str, Vec<&str>) = match kind.as_str() {
                "position" => ("transform", vec!["position", "positionX", "positionY", "positionZ"]),
                "scale" => ("transform", vec!["scale"]),
                "rotation" => ("transform", vec!["rotation", "rotationX", "rotationY", "orientation"]),
                "opacity" => ("transform", vec!["opacity"]),
                "anchor" => ("transform", vec!["anchor"]),
                "feather" => ("masks", vec!["feather"]),
                "levels" => ("audio", vec!["levels"]),
                _ => ("", vec![]),
            };
            match kind.as_str() {
                "waveform" => {
                    if let Some(item) = super::waveform::audio_item(&app.session.project, l) {
                        rows.push(Row { layer: l.id, depth: 1, kind: RowKind::Waveform { item: item.0 } });
                    }
                }
                "effects" => {
                    if let Some(fx) = l.effects() {
                        for g in fx.groups() {
                            rows.push(Row {
                                layer: l.id,
                                depth: 1,
                                kind: RowKind::Group {
                                    uid: g.uid,
                                    name: g.name.clone(),
                                    open: tl.open_groups.contains(&g.uid),
                                    has_children: true,
                                    fx: Some(g.enabled),
                                    eye: None,
                                },
                            });
                            if tl.open_groups.contains(&g.uid) {
                                push_group(&mut rows, l, g, 2, &tl.open_groups);
                            }
                        }
                    }
                }
                "masks" => {
                    if let Some(m) = l.masks() {
                        for g in m.groups() {
                            rows.push(Row {
                                layer: l.id,
                                depth: 1,
                                kind: RowKind::Group {
                                    uid: g.uid,
                                    name: g.name.clone(),
                                    open: tl.open_groups.contains(&g.uid),
                                    has_children: true,
                                    fx: None,
                                    eye: None,
                                },
                            });
                            if tl.open_groups.contains(&g.uid) {
                                push_group(&mut rows, l, g, 2, &tl.open_groups);
                            }
                        }
                    }
                }
                "props" => {
                    // Animation ▸ Reveal Properties…: the engine picked the uids.
                    fn groups_in<'a>(g: &'a PropGroup, set: &std::collections::BTreeSet<u64>, out: &mut Vec<&'a PropGroup>) {
                        for sg in g.groups() {
                            if set.contains(&sg.uid) {
                                out.push(sg);
                            } else {
                                groups_in(sg, set, out);
                            }
                        }
                    }
                    let mut groups = vec![];
                    groups_in(&l.props, &tl.reveal_props, &mut groups);
                    for g in groups {
                        let open = tl.open_groups.contains(&g.uid);
                        rows.push(Row {
                            layer: l.id,
                            depth: 1,
                            kind: RowKind::Group { uid: g.uid, name: g.name.clone(), open, has_children: !g.children.is_empty(), fx: None, eye: None },
                        });
                        if open {
                            push_group(&mut rows, l, g, 2, &tl.open_groups);
                        }
                    }
                    let mut found = vec![];
                    collect_props(&l.props, &mut found, &|p| prop_visible(p, l) && tl.reveal_props.contains(&p.uid));
                    for uid in found {
                        rows.push(Row { layer: l.id, depth: 1, kind: RowKind::Prop { uid } });
                    }
                }
                _ => {
                    let mut found = vec![];
                    let root = if group.is_empty() {
                        Some(&l.props)
                    } else if group == "masks" {
                        l.masks()
                    } else {
                        l.props.sub(group)
                    };
                    if let Some(g) = root {
                        collect_props(g, &mut found, &|p| prop_visible(p, l) && reveal_matches(p, &props, kind));
                    }
                    for uid in found {
                        rows.push(Row { layer: l.id, depth: 1, kind: RowKind::Prop { uid } });
                    }
                }
            }
            continue;
        }
        for c in &l.props.children {
            match c {
                Node::Group(g) if group_visible(g, l) => {
                    let open = tl.open_groups.contains(&g.uid);
                    rows.push(Row {
                        layer: l.id,
                        depth: 1,
                        kind: RowKind::Group {
                            uid: g.uid,
                            name: g.name.clone(),
                            open,
                            has_children: !g.children.is_empty(),
                            fx: None,
                            eye: (g.match_id == effectcraft_engine::project::styles::GROUP).then_some(g.enabled),
                        },
                    });
                    if open {
                        push_group(&mut rows, l, g, 2, &tl.open_groups);
                        // Audio > Waveform > Waveform (footage with audio).
                        if g.match_id == "audio"
                            && let Some(item) = super::waveform::audio_item(&app.session.project, l)
                        {
                            let wuid = WAVE_BIT | g.uid;
                            let wopen = tl.open_groups.contains(&wuid);
                            rows.push(Row {
                                layer: l.id,
                                depth: 2,
                                kind: RowKind::Group { uid: wuid, name: "Waveform".into(), open: wopen, has_children: true, fx: None, eye: None },
                            });
                            if wopen {
                                rows.push(Row { layer: l.id, depth: 3, kind: RowKind::Waveform { item: item.0 } });
                            }
                        }
                    }
                }
                Node::Prop(p) if prop_visible(p, l) => rows.push(Row { layer: l.id, depth: 1, kind: RowKind::Prop { uid: p.uid } }),
                _ => {}
            }
        }
    }
    rows
}

fn collect_props(g: &PropGroup, out: &mut Vec<u64>, f: &dyn Fn(&Property) -> bool) {
    for c in &g.children {
        match c {
            Node::Prop(p) if f(p) => out.push(p.uid),
            Node::Group(g) => collect_props(g, out, f),
            _ => {}
        }
    }
}

fn push_group(rows: &mut Vec<Row>, l: &Layer, g: &PropGroup, depth: usize, open: &std::collections::BTreeSet<u64>) {
    for c in &g.children {
        match c {
            // Text animators list their selectors and properties directly (as in AE).
            Node::Group(sg) if g.match_id == "animator" && matches!(sg.match_id.as_str(), "selectors" | "properties") => {
                push_group(rows, l, sg, depth, open);
            }
            Node::Group(sg) => {
                let o = open.contains(&sg.uid);
                let fx = matches!(sg.kind, GroupKind::Effect { .. }).then_some(sg.enabled);
                // Layer style groups have eye switches (not Blending Options).
                let eye = (g.match_id == effectcraft_engine::project::styles::GROUP && sg.match_id != effectcraft_engine::project::styles::BLENDING)
                    .then_some(sg.enabled);
                rows.push(Row {
                    layer: l.id,
                    depth,
                    kind: RowKind::Group { uid: sg.uid, name: sg.name.clone(), open: o, has_children: !sg.children.is_empty(), fx, eye },
                });
                if o {
                    push_group(rows, l, sg, depth + 1, open);
                }
            }
            Node::Prop(p) if prop_visible(p, l) => rows.push(Row { layer: l.id, depth, kind: RowKind::Prop { uid: p.uid } }),
            _ => {}
        }
    }
}

fn layer_icon(l: &Layer, project: &effectcraft_engine::project::Project) -> Icon {
    match &l.source {
        LayerSource::Text => Icon::TextLayer,
        LayerSource::Shape => Icon::ShapeLayer,
        LayerSource::Null => Icon::Null,
        LayerSource::Camera => Icon::Camera,
        LayerSource::Light { .. } => Icon::Light,
        LayerSource::Comp { .. } => Icon::Comp,
        LayerSource::Solid { .. } => {
            if l.switches.adjustment {
                Icon::Adjustment
            } else {
                Icon::Solid
            }
        }
        LayerSource::Footage { item } => match project.item(*item).map(|i| i.type_name()) {
            Some("Image") | Some("Image Sequence") => Icon::Image,
            Some("Audio") => Icon::Audio,
            _ => Icon::Footage,
        },
    }
}

pub fn show(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    let Some(cid) = app.session.active_comp_id() else {
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "(no composition)", Tokens::ui(12.0), t.text_faint);
        return;
    };
    let Some(comp) = app.session.project.comp(cid).cloned() else { return };
    let p = ui.painter().clone();
    let time = app.session.time();
    let fr = comp.frame_rate;

    // Geometry.
    let header_h = 48.0;
    let ruler_h = 34.0;
    let colhdr_h = 22.0;
    let footer_h = 24.0;
    let fixed = AV_W + LABEL_W + NUM_W + SW * 8.0 + if app.ui.timeline.show_modes { 92.0 + 112.0 } else { 0.0 } + 116.0 + 6.0;
    let left_w = (fixed + 190.0).clamp(420.0, (rect.width() * 0.62).max(420.0));
    let cw = cols(rect.min.x, left_w, app.ui.timeline.show_modes);
    let graph_x0 = rect.min.x + left_w + 1.0;
    let graph_x1 = rect.max.x - 10.0;
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("timeline-graph-area"), (graph_x0, graph_x1 - graph_x0)));
    let fit_pps = (graph_x1 - graph_x0 - 10.0) as f64 / comp.duration.seconds().max(0.01);
    let pps = app.ui.timeline.pps.unwrap_or(fit_pps);
    if app.ui.timeline.pps.is_none() {
        app.ui.timeline.start = 0.0;
    }
    let tm = TMap { x0: graph_x0 + 6.0, start: app.ui.timeline.start, pps };
    ctx.data_mut(|d| d.insert_temp(tl_map_id(), (tm.x0, tm.start, tm.pps)));
    let top = rect.min.y;
    let rows_top = top + header_h + colhdr_h;
    let rows_rect = Rect::from_min_max(pos2(rect.min.x, rows_top), pos2(rect.max.x, rect.max.y - footer_h));
    let graph_rect = Rect::from_min_max(pos2(graph_x0, top + header_h - ruler_h + colhdr_h), pos2(graph_x1 + 10.0, rect.max.y - footer_h));
    p.rect_filled(Rect::from_min_max(pos2(graph_x0, top), rect.max), 0.0, t.tl_bg);
    p.line_segment([pos2(graph_x0 - 1.0, top), pos2(graph_x0 - 1.0, rect.max.y)], Stroke::new(1.0, t.app_bg));

    // ---- header (left): time display, search, switches.
    let tc = crate::panels::timecode(&app.session, &comp, time);
    let tc_rect = Rect::from_min_size(pos2(rect.min.x + 12.0, top + 6.0), vec2(150.0, 24.0));
    let tc_resp = ui.interact(tc_rect, egui::Id::new("tl-timecode"), Sense::click_and_drag());
    p.text(tc_rect.left_center(), Align2::LEFT_CENTER, &tc, Tokens::semibold(20.0), t.timecode);
    let sub = format!("{:05} ({:.2} fps)", fr.frame_at(time) + app.session.project.settings.frame_start, fr.as_f64());
    p.text(pos2(tc_rect.min.x, tc_rect.max.y + 8.0), Align2::LEFT_CENTER, sub, Tokens::ui(10.5), t.text_faint);
    app.auto.add("timeline.timecode", tc_rect, &tc);
    if tc_resp.dragged() {
        let d = tc_resp.drag_delta().x as f64;
        let f = fr.frame_at(time) + (d * 0.5).round() as i64;
        let _ = app.session.execute("time.set", json!({"frame": f.max(0)}));
    }
    if tc_resp.double_clicked() {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("tl-tc-edit"), tc.clone()));
    }
    if let Some(mut buf) = ctx.data(|d| d.get_temp::<String>(egui::Id::new("tl-tc-edit"))) {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(tc_rect));
        let r = child.add(egui::TextEdit::singleline(&mut buf).font(Tokens::semibold(18.0)).desired_width(150.0));
        r.request_focus();
        if r.lost_focus() {
            let _ = app.session.execute("time.set", json!({"timecode": buf}));
            ctx.data_mut(|d| d.remove::<String>(egui::Id::new("tl-tc-edit")));
        } else {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("tl-tc-edit"), buf));
        }
    }
    let sr = Rect::from_min_size(pos2(tc_rect.max.x + 14.0, top + 12.0), vec2((left_w - 420.0).clamp(120.0, 220.0), 22.0));
    let mut search = app.ui.timeline.search.clone();
    widgets::search_field(ui, sr, &mut search, "Search", &t);
    app.ui.timeline.search = search;
    app.auto.add("timeline.search", sr, "Search");
    let mut bx = rect.min.x + left_w - 10.0;
    for (icon, on, id, tip) in [
        (Icon::Graph, app.ui.timeline.graph_editor, "graph", "Graph Editor"),
        (Icon::MotionBlur, comp.enable_motion_blur, "motionBlur", "Enables Motion Blur for all layers with the Motion Blur switch set"),
        (Icon::FrameBlend, comp.enable_frame_blending, "frameBlending", "Enables Frame Blending for all layers with the Frame Blend switch set"),
        (Icon::Shy, comp.hide_shy, "hideShy", "Hides all layers for which the Shy switch is set"),
        (Icon::Draft3D, comp.draft_3d, "draft3d", "Draft 3D"),
        (Icon::Flowchart, false, "flowchart", "Composition Mini-Flowchart"),
    ] {
        let r = Rect::from_min_size(pos2(bx - 24.0, top + 11.0), vec2(24.0, 24.0));
        let resp = widgets::icon_button(ui, r, icon, on, &t, egui::Id::new(("tl-sw", id))).on_hover_text(tip);
        app.auto.add(&format!("timeline.switch.{id}"), r, tip);
        if resp.clicked() {
            match id {
                "graph" => app.ui.timeline.graph_editor = !app.ui.timeline.graph_editor,
                "flowchart" => app.show_panel(crate::dock::PanelKind::Flowchart),
                _ => {
                    let _ = app.session.execute("comp.setSwitch", json!({"switch": id}));
                }
            }
        }
        bx -= 28.0;
    }

    // ---- ruler (right).
    let ruler = Rect::from_min_max(pos2(graph_x0, top + header_h - ruler_h), pos2(rect.max.x, top + header_h));
    let nav = Rect::from_min_max(pos2(graph_x0 + 6.0, top + 6.0), pos2(graph_x1, top + 12.0));
    // Time navigator.
    p.rect_filled(nav, 3.0, t.field_bg);
    let vis0 = (tm.start / comp.duration.seconds()) as f32;
    let vis1 = (tm.t(graph_x1) / comp.duration.seconds()) as f32;
    let nav_vis =
        Rect::from_min_max(pos2(nav.min.x + nav.width() * vis0.clamp(0.0, 1.0), nav.min.y), pos2(nav.min.x + nav.width() * vis1.clamp(0.0, 1.0), nav.max.y));
    p.rect_filled(nav_vis, 3.0, t.work_area);
    let nresp = ui.interact(nav, egui::Id::new("tl-nav"), Sense::drag());
    if nresp.dragged() {
        let d = nresp.drag_delta().x as f64 / nav.width() as f64 * comp.duration.seconds();
        app.ui.timeline.start = (app.ui.timeline.start + d).clamp(0.0, comp.duration.seconds());
        if app.ui.timeline.pps.is_none() {
            app.ui.timeline.pps = Some(pps);
        }
    }
    p.rect_filled(ruler, 0.0, t.tl_ruler_bg);
    app.auto.add("timeline.ruler", ruler, &format!("{},{}", tm.start, tm.pps));
    // Work area bar.
    let wa = Rect::from_min_max(pos2(tm.x(comp.work_area.0.seconds()), ruler.min.y + 2.0), pos2(tm.x(comp.work_area.1.seconds()), ruler.min.y + 10.0));
    p.rect_filled(wa, 2.0, t.work_area);
    for (hx, set) in [(wa.min.x, "begin"), (wa.max.x, "end")] {
        let hr = Rect::from_center_size(pos2(hx, wa.center().y), vec2(8.0, 12.0));
        p.rect_filled(Rect::from_center_size(pos2(hx, wa.center().y), vec2(3.0, 10.0)), 1.0, t.text_dim);
        let resp = ui.interact(hr, egui::Id::new(("wa", set)), Sense::drag());
        app.auto.add(&format!("timeline.workArea.{set}"), hr, set);
        if resp.dragged()
            && let Some(pt) = resp.interact_pointer_pos()
        {
            let secs = fr.snap_nearest(Tick::from_seconds_f64(tm.t(pt.x).max(0.0))).seconds();
            let key = if set == "begin" { "start" } else { "end" };
            let _ = app.session.execute("comp.workArea", json!({key: secs, "merge": "wa-drag"}));
        }
    }
    // Cache bar (green: cached frames at the viewer's resolution).
    let scale_key = app.viewer_shown.as_ref().map(|(_, k)| k.scale).unwrap_or(1000);
    let cached = app.frames.cached_frames(app.session.revision, cid.0, scale_key);
    let fd = comp.frame_duration().seconds();
    let cy0 = ruler.min.y + 11.0;
    let mut run: Option<(i64, i64)> = None;
    let draw_run = |a: i64, b: i64| {
        let x0 = tm.x(a as f64 * fd);
        let x1 = tm.x((b + 1) as f64 * fd);
        p.rect_filled(Rect::from_min_max(pos2(x0, cy0), pos2(x1, cy0 + 2.5)), 0.0, t.cache_green);
    };
    for f in cached {
        run = match run {
            Some((a, b)) if f == b + 1 => Some((a, f)),
            Some((a, b)) => {
                draw_run(a, b);
                Some((f, f))
            }
            None => Some((f, f)),
        };
    }
    if let Some((a, b)) = run {
        draw_run(a, b);
    }
    // Ticks + labels.
    // Label spacing in whole frames (AE: `00:15f`-style seconds:frames labels).
    let fps_i = fr.as_f64().round().max(1.0) as i64;
    let half = (fps_i / 2).max(1);
    let label_frames = [1, 2, 5, 10, half, fps_i, 2 * fps_i, 5 * fps_i, 10 * fps_i, 30 * fps_i, 60 * fps_i]
        .into_iter()
        .find(|f| *f as f64 * fd * pps >= 52.0)
        .unwrap_or(60 * fps_i);
    let secs_per_label = label_frames as f64 * fd;
    let first = (tm.start / secs_per_label).floor() * secs_per_label;
    let mut s = first;
    let pr = p.with_clip_rect(Rect::from_min_max(pos2(graph_x0, ruler.min.y), ruler.max));
    while s <= tm.t(graph_x1) + secs_per_label {
        let x = tm.x(s);
        pr.line_segment([pos2(x, ruler.max.y - 10.0), pos2(x, ruler.max.y)], Stroke::new(1.0, t.tl_ruler_tick));
        let fno = (s / fd).round() as i64;
        let label = if fno >= 60 * fps_i * 60 {
            format!("{}:{:02}:{:02}f", fno / (fps_i * 60), (fno / fps_i) % 60, fno % fps_i)
        } else {
            format!("{:02}:{:02}f", fno / fps_i, fno % fps_i)
        };
        pr.text(pos2(x + 3.0, ruler.max.y - 16.0), Align2::LEFT_CENTER, label, Tokens::ui(10.0), t.tl_ruler_text);
        for k in 1..5 {
            let xx = tm.x(s + secs_per_label * k as f64 / 5.0);
            pr.line_segment([pos2(xx, ruler.max.y - 4.0), pos2(xx, ruler.max.y)], Stroke::new(1.0, t.tl_ruler_tick.gamma_multiply(0.6)));
        }
        s += secs_per_label;
    }
    // Comp markers.
    for m in &comp.markers {
        let x = tm.x(m.time.seconds());
        pr.add(egui::Shape::convex_polygon(
            vec![pos2(x - 4.0, ruler.max.y - 10.0), pos2(x + 4.0, ruler.max.y - 10.0), pos2(x, ruler.max.y - 4.0)],
            t.label(m.label),
            Stroke::NONE,
        ));
    }
    // Scrub in the ruler.
    let rresp = ui.interact(Rect::from_min_max(pos2(graph_x0, ruler.min.y + 12.0), ruler.max), egui::Id::new("tl-ruler"), Sense::click_and_drag());
    if (rresp.dragged() || rresp.clicked())
        && let Some(pt) = rresp.interact_pointer_pos()
    {
        let tt = Tick::from_seconds_f64(tm.t(pt.x).max(0.0));
        app.session.set_time(tt);
        app.stop();
    }

    // ---- column headers.
    let ch = Rect::from_min_max(pos2(rect.min.x, top + header_h), pos2(rect.max.x, top + header_h + colhdr_h));
    p.rect_filled(Rect::from_min_max(ch.min, pos2(graph_x0 - 1.0, ch.max.y)), 0.0, t.panel_bg);
    p.line_segment([pos2(rect.min.x, ch.max.y), pos2(rect.max.x, ch.max.y)], Stroke::new(1.0, t.separator));
    let hy = ch.center().y;
    let hic = |icon: Icon, x: f32| icons::paint(&p, Rect::from_center_size(pos2(x, hy), vec2(12.0, 12.0)), icon, t.text_dim);
    hic(Icon::Eye, cw.av + 10.0);
    hic(Icon::Speaker, cw.av + 28.0);
    hic(Icon::Solo, cw.av + 46.0);
    hic(Icon::Lock, cw.av + 64.0);
    icons::paint(&p, Rect::from_center_size(pos2(cw.label + 10.0, hy), vec2(10.0, 10.0)), Icon::Keyframe, t.text_dim);
    p.text(pos2(cw.num + 6.0, hy), Align2::LEFT_CENTER, "#", Tokens::ui(11.0), t.text_dim);
    p.text(pos2(cw.name + 22.0, hy), Align2::LEFT_CENTER, "Layer Name", Tokens::ui(11.0), t.text_dim);
    for (i, icon) in
        [Icon::Shy, Icon::Collapse, Icon::Quality, Icon::Fx, Icon::FrameBlend, Icon::MotionBlur, Icon::Adjustment, Icon::Cube].into_iter().enumerate()
    {
        hic(icon, cw.switches + SW * i as f32 + SW / 2.0);
    }
    if app.ui.timeline.show_modes {
        p.text(pos2(cw.mode + 4.0, hy), Align2::LEFT_CENTER, "Mode", Tokens::ui(11.0), t.text_dim);
        p.text(pos2(cw.mode + 74.0, hy), Align2::LEFT_CENTER, "T", Tokens::ui(11.0), t.text_dim);
        p.text(pos2(cw.trkmat + 4.0, hy), Align2::LEFT_CENTER, "Track Matte", Tokens::ui(11.0), t.text_dim);
    }
    icons::paint(&p, Rect::from_center_size(pos2(cw.parent + 10.0, hy), vec2(12.0, 12.0)), Icon::PickWhip, t.text_dim);
    p.text(pos2(cw.parent + 20.0, hy), Align2::LEFT_CENTER, "Parent & Link", Tokens::ui(11.0), t.text_dim);
    let _ = cw.end;

    // ---- rows.
    let rows = with_expr_rows(build_rows(app, &comp), &comp, &app.ui.timeline.expr_closed);
    let rh = t.row_h;
    let total_h: f32 = rows.iter().map(|r| row_height(r, rh)).sum();
    let max_scroll = (total_h - rows_rect.height() + rh).max(0.0);
    if ui.rect_contains_pointer(rows_rect) {
        let (dy, dx, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.smooth_scroll_delta.x, i.modifiers.alt));
        if zoom && dy.abs() > 0.0 {
            zoom_at(app, &comp, tm, (dy as f64 / 200.0).exp(), ui.input(|i| i.pointer.hover_pos()).map(|p| p.x).unwrap_or(graph_x0));
        } else {
            app.ui.timeline.scroll_y = (app.ui.timeline.scroll_y - dy).clamp(0.0, max_scroll);
            if dx.abs() > 0.0 && app.ui.timeline.pps.is_some() {
                app.ui.timeline.start = (app.ui.timeline.start - dx as f64 / pps).max(0.0);
            }
        }
    }
    app.ui.timeline.scroll_y = app.ui.timeline.scroll_y.min(max_scroll);
    let snap_project = app.session.project.clone();
    let snap_expr = app.session.expr.clone();
    let ectx = EvalCtx { project: &snap_project, comp_id: cid, comp: &comp, time, expr: snap_expr.as_deref() };
    let lp = p.with_clip_rect(rows_rect.intersect(Rect::from_min_max(rows_rect.min, pos2(graph_x0 - 1.0, rows_rect.max.y))));
    let gp = p.with_clip_rect(rows_rect.intersect(Rect::from_min_max(pos2(graph_x0, rows_rect.min.y), rows_rect.max)));
    let mut y = rows_rect.min.y - app.ui.timeline.scroll_y;
    let selected = app.session.state.selected_layers.clone();
    let sel_keys = app.session.state.selected_keys.clone();
    let mut actions: Vec<(String, serde_json::Value)> = Vec::new();
    let mut ui_actions: Vec<UiAct> = Vec::new();
    let idx_of = |id: LayerId| comp.index_of(id).unwrap_or(0);
    let graph_on = app.ui.timeline.graph_editor;
    let full_clip = ui.clip_rect();
    let left_clip = full_clip.intersect(Rect::from_min_max(pos2(rect.min.x, rows_rect.min.y), pos2(graph_x0 - 1.0, rows_rect.max.y)));
    let right_clip = full_clip.intersect(Rect::from_min_max(pos2(graph_x0, rows_rect.min.y), rows_rect.max));
    // Registered before the rows so keys, bars and editors on top of it get the pointer first.
    let empty_rect = if graph_on { Rect::NOTHING } else { Rect::from_min_max(pos2(graph_x0, rows_rect.min.y), rows_rect.max) };
    let empty = ui.interact(empty_rect, egui::Id::new("tl-graph-bg"), Sense::click_and_drag());
    let mut hit_rows: Vec<(Rect, Row)> = vec![];
    for (ri, row) in rows.iter().enumerate() {
        // Outline widgets never spill into the time graph (narrow timelines).
        ui.set_clip_rect(left_clip);
        let hh = row_height(row, rh);
        let r = Rect::from_min_size(pos2(rect.min.x, y), vec2(rect.width(), hh));
        y += hh;
        if r.max.y < rows_rect.min.y || r.min.y > rows_rect.max.y {
            continue;
        }
        hit_rows.push((r, row.clone()));
        let Some(layer) = comp.layer(row.layer) else { continue };
        let is_sel = selected.contains(&layer.id);
        let left = Rect::from_min_max(r.min, pos2(graph_x0 - 1.0, r.max.y));
        let cy = r.min.y + rh / 2.0;
        match &row.kind {
            RowKind::Layer => {
                lp.rect_filled(
                    left,
                    0.0,
                    if is_sel {
                        t.row_selected
                    } else if ri % 2 == 0 {
                        t.row
                    } else {
                        t.row_alt
                    },
                );
                gp.rect_filled(Rect::from_min_max(pos2(graph_x0, r.min.y), r.max), 0.0, if is_sel { Color32::from_rgb(0x26, 0x26, 0x26) } else { t.tl_bg });
                lp.line_segment([pos2(rect.min.x, r.max.y), pos2(graph_x0, r.max.y)], Stroke::new(1.0, Color32::from_black_alpha(80)));
                // A/V features.
                let sw = &layer.switches;
                let av_items: [(Icon, bool, &str, bool); 4] = [
                    (Icon::Eye, sw.video, "video", layer.source.is_av()),
                    (Icon::Speaker, sw.audio, "audio", layer.props.sub("audio").is_some()),
                    (Icon::Solo, sw.solo, "solo", layer.source.is_av()),
                    (Icon::Lock, sw.locked, "lock", true),
                ];
                for (i, (icon, on, name, applicable)) in av_items.into_iter().enumerate() {
                    let br = Rect::from_center_size(pos2(cw.av + 10.0 + 18.0 * i as f32, cy), vec2(17.0, 17.0));
                    if !applicable {
                        continue;
                    }
                    lp.rect_stroke(br.shrink(1.5), 2.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
                    let resp = widgets::icon_toggle(ui, br, icon, on, &t, egui::Id::new(("av", layer.id.0, i)), None);
                    app.auto.add(&format!("timeline.layer.{}.{name}", layer.id.0), br, name);
                    if resp.clicked() {
                        actions.push(("layer.setSwitch".into(), json!({"layers": [layer.id.0], "switch": name})));
                    }
                }
                // Label swatch.
                let lr = Rect::from_center_size(pos2(cw.label + 10.0, cy), vec2(12.0, 12.0));
                lp.rect_filled(lr, 2.0, t.label(layer.label));
                let lresp = ui.interact(lr, egui::Id::new(("label", layer.id.0)), Sense::click());
                lresp.context_menu(|ui| {
                    for lab in effectcraft_engine::color::Label::ALL {
                        let name = if lab == effectcraft_engine::color::Label::None { lab.name().to_string() } else { app.session.prefs.label_name(lab) };
                        if ui.button(name).clicked() {
                            actions.push(("edit.label".into(), json!({"layers": [layer.id.0], "label": lab.name()})));
                            ui.close();
                        }
                    }
                });
                lp.text(pos2(cw.num + 13.0, cy), Align2::CENTER_CENTER, format!("{}", idx_of(layer.id)), Tokens::ui(11.5), t.text_dim);
                // Twirl + icon + name.
                let tw = Rect::from_center_size(pos2(cw.name + 8.0, cy), vec2(14.0, 14.0));
                let open = app.ui.timeline.open_layers.contains(&layer.id.0);
                if widgets::twirl(ui, tw, open, egui::Id::new(("twirl", layer.id.0)), &t).clicked() {
                    ui_actions.push(UiAct::ToggleLayer(layer.id.0, ui.input(|i| i.modifiers.alt)));
                }
                app.auto.add(&format!("timeline.layer.{}.twirl", layer.id.0), tw, "twirl");
                icons::paint(&lp, Rect::from_center_size(pos2(cw.name + 24.0, cy), vec2(13.0, 13.0)), layer_icon(layer, &app.session.project), t.text_dim);
                let name_rect = Rect::from_min_max(pos2(cw.name + 34.0, r.min.y), pos2(cw.switches - 4.0, r.max.y));
                let rename_id = egui::Id::new("tl-rename");
                let renaming: Option<(u64, String)> = ctx.data(|d| d.get_temp(rename_id));
                if let Some((rl, mut buf)) = renaming.filter(|(rl, _)| *rl == layer.id.0) {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(name_rect.shrink2(vec2(0.0, 2.0))));
                    let er = child.add(egui::TextEdit::singleline(&mut buf).font(Tokens::ui(12.0)).desired_width(name_rect.width()));
                    er.request_focus();
                    if er.lost_focus() {
                        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            actions.push(("layer.rename".into(), json!({"layer": rl, "name": buf})));
                        }
                        ctx.data_mut(|d| d.remove::<(u64, String)>(rename_id));
                    } else {
                        ctx.data_mut(|d| d.insert_temp(rename_id, (rl, buf)));
                    }
                } else {
                    let name_col = if is_sel { Color32::WHITE } else { t.text };
                    let lpn = lp.with_clip_rect(name_rect.intersect(lp.clip_rect()));
                    lpn.text(pos2(name_rect.min.x, cy), Align2::LEFT_CENTER, &layer.name, Tokens::ui(12.0), name_col);
                }
                // Row click → select; double-click → rename; drag → reorder (later).
                let row_resp =
                    ui.interact(Rect::from_min_max(pos2(cw.num, r.min.y), pos2(cw.switches, r.max.y)), egui::Id::new(("row", layer.id.0)), Sense::click());
                app.auto.add(&format!("timeline.layer.{}.row", layer.id.0), left, &layer.name);
                if row_resp.clicked() {
                    let m = ui.input(|i| i.modifiers);
                    actions.push(("layer.select".into(), json!({"layers": [layer.id.0], "toggle": m.command, "add": m.shift})));
                }
                if row_resp.double_clicked() {
                    match &layer.source {
                        LayerSource::Comp { item } => actions.push(("comp.open".into(), json!({"comp": item.0}))),
                        // Footage and solids open in the Layer panel (paint happens there).
                        LayerSource::Footage { .. } | LayerSource::Solid { .. } => {
                            actions.push(("layer.openLayer".into(), json!({"layer": layer.id.0})));
                        }
                        _ => {
                            ctx.data_mut(|d| d.insert_temp(rename_id, (layer.id.0, layer.name.clone())));
                        }
                    }
                }
                layer_context_menu(&row_resp, layer, &mut actions);
                // Switches.
                let sws: [(Icon, bool, &str, bool); 8] = [
                    (Icon::Shy, sw.shy, "shy", true),
                    (Icon::Collapse, sw.collapse, "collapse", matches!(layer.source, LayerSource::Comp { .. } | LayerSource::Shape | LayerSource::Text)),
                    (Icon::Quality, sw.quality == effectcraft_engine::project::Quality::Best, "quality", layer.source.is_av()),
                    (Icon::Fx, sw.effects, "fx", layer.effects().is_some_and(|f| !f.children.is_empty())),
                    (
                        Icon::FrameBlend,
                        sw.frame_blend != effectcraft_engine::project::FrameBlend::Off,
                        "frameBlend",
                        matches!(layer.source, LayerSource::Footage { .. } | LayerSource::Comp { .. }),
                    ),
                    (Icon::MotionBlur, sw.motion_blur, "motionBlur", layer.source.is_av()),
                    (Icon::Adjustment, sw.adjustment, "adjustment", layer.source.is_av()),
                    (Icon::Cube, layer.is_3d(), "threeD", layer.source.is_av() || matches!(layer.source, LayerSource::Null)),
                ];
                for (i, (icon, on, name, applicable)) in sws.into_iter().enumerate() {
                    if !applicable {
                        continue;
                    }
                    let br = Rect::from_center_size(pos2(cw.switches + SW * i as f32 + SW / 2.0, cy), vec2(16.0, 16.0));
                    lp.rect_stroke(br.shrink(1.5), 2.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
                    let resp = widgets::icon_toggle(ui, br, icon, on, &t, egui::Id::new(("sw", layer.id.0, i)), None);
                    app.auto.add(&format!("timeline.layer.{}.switch.{name}", layer.id.0), br, name);
                    if resp.clicked() {
                        actions.push(("layer.setSwitch".into(), json!({"layers": [layer.id.0], "switch": name})));
                    }
                }
                // Modes.
                if app.ui.timeline.show_modes && layer.source.is_av() {
                    let mr = Rect::from_min_size(pos2(cw.mode + 2.0, cy - 9.0), vec2(80.0, 18.0));
                    let mresp = widgets::dropdown(ui, mr, layer.blend_mode.label(), &t, egui::Id::new(("mode", layer.id.0)));
                    app.auto.add(&format!("timeline.layer.{}.mode", layer.id.0), mr, "Mode");
                    let pop = egui::Id::new(("mode-pop", layer.id.0));
                    if mresp.clicked() {
                        widgets::open_popup(ui, pop);
                    }
                    let opts: Vec<String> = BlendMode::ALL
                        .iter()
                        .flat_map(|m| if m.ends_group() { vec![m.label().to_string(), "-".to_string()] } else { vec![m.label().to_string()] })
                        .collect();
                    if let Some(i) = widgets::popup_menu(ui, pop, mr.left_bottom(), &opts, opts.iter().position(|o| o == layer.blend_mode.label())) {
                        actions.push(("layer.setBlendMode".into(), json!({"layers": [layer.id.0], "mode": opts[i]})));
                    }
                    let tr = Rect::from_center_size(pos2(cw.mode + 76.0 + 6.0, cy), vec2(14.0, 14.0));
                    if widgets::checkbox(ui, tr, layer.preserve_transparency, &t, egui::Id::new(("pt", layer.id.0))).clicked() {
                        actions.push(("layer.setSwitch".into(), json!({"layers": [layer.id.0], "switch": "preserveTransparency"})));
                    }
                    // Track matte: layer picker + kind.
                    let mr = Rect::from_min_size(pos2(cw.trkmat + 2.0, cy - 9.0), vec2(104.0, 18.0));
                    let label = match layer.track_matte {
                        Some(m) => comp.layer(m.layer).map(|l| format!("{} {}", idx_of(l.id), l.name)).unwrap_or("None".into()),
                        None => "No Track Matte".into(),
                    };
                    let tresp = widgets::dropdown(ui, mr, &label, &t, egui::Id::new(("trkmat", layer.id.0)));
                    app.auto.add(&format!("timeline.layer.{}.trackMatte", layer.id.0), mr, "Track Matte");
                    let pop = egui::Id::new(("trk-pop", layer.id.0));
                    if tresp.clicked() {
                        widgets::open_popup(ui, pop);
                    }
                    let mut opts = vec!["No Track Matte".to_string(), "-".to_string()];
                    let candidates: Vec<&Layer> = comp.layers.iter().filter(|l| l.id != layer.id && l.source.is_av()).collect();
                    for l in &candidates {
                        opts.push(format!("{}. {}", idx_of(l.id), l.name));
                    }
                    opts.push("-".into());
                    for k in MatteKind::ALL {
                        opts.push(k.label().into());
                    }
                    if let Some(i) = widgets::popup_menu(ui, pop, mr.left_bottom(), &opts, None) {
                        if i == 0 {
                            actions.push(("layer.setTrackMatte".into(), json!({"layer": layer.id.0, "matte": null})));
                        } else if i >= 2 && i < 2 + candidates.len() {
                            let kind = layer.track_matte.map(|m| m.kind).unwrap_or_default();
                            actions
                                .push(("layer.setTrackMatte".into(), json!({"layer": layer.id.0, "matte": candidates[i - 2].id.0, "kind": kind_name(kind)})));
                        } else if let Some(k) = MatteKind::ALL.into_iter().find(|k| k.label() == opts[i])
                            && let Some(m) = layer.track_matte
                        {
                            actions.push(("layer.setTrackMatte".into(), json!({"layer": layer.id.0, "matte": m.layer.0, "kind": kind_name(k)})));
                        }
                    }
                    if layer.track_matte.is_some() {
                        let kr = Rect::from_center_size(pos2(mr.max.x - 26.0, cy), vec2(10.0, 10.0));
                        let _ = kr;
                    }
                }
                // Parent.
                let pw_rect = Rect::from_center_size(pos2(cw.parent + 9.0, cy), vec2(15.0, 15.0));
                let pw = ui.interact(pw_rect, egui::Id::new(("parent-whip", layer.id.0)), Sense::drag()).on_hover_text("Parent pick whip: drag onto a layer");
                icons::paint(&lp, pw_rect.shrink(1.0), Icon::PickWhip, if pw.hovered() || pw.dragged() { t.text } else { t.text_dim });
                app.auto.add(&format!("timeline.layer.{}.pickWhip", layer.id.0), pw_rect, "Parent pick whip");
                if pw.drag_started() {
                    let pw_state: PickWhip = (0, layer.id.0, 0, pw_rect.center());
                    ctx.data_mut(|d| d.insert_temp(pick_whip_id(), pw_state));
                }
                let pr_rect = Rect::from_min_size(pos2(cw.parent + 20.0, cy - 9.0), vec2(94.0, 18.0));
                let plabel = layer.parent.and_then(|p| comp.layer(p)).map(|l| format!("{}. {}", idx_of(l.id), l.name)).unwrap_or_else(|| "None".into());
                let presp = widgets::dropdown(ui, pr_rect, &plabel, &t, egui::Id::new(("parent", layer.id.0)));
                app.auto.add(&format!("timeline.layer.{}.parent", layer.id.0), pr_rect, "Parent");
                let pop = egui::Id::new(("par-pop", layer.id.0));
                if presp.clicked() {
                    widgets::open_popup(ui, pop);
                }
                let cands: Vec<&Layer> = comp.layers.iter().filter(|l| l.id != layer.id).collect();
                let mut popts = vec!["None".to_string(), "-".to_string()];
                popts.extend(cands.iter().map(|l| format!("{}. {}", idx_of(l.id), l.name)));
                if let Some(i) = widgets::popup_menu(ui, pop, pr_rect.left_bottom(), &popts, None) {
                    let par = if i == 0 { serde_json::Value::Null } else { json!(cands[i - 2].id.0) };
                    actions.push(("layer.setParent".into(), json!({"layers": [layer.id.0], "parent": par})));
                }
                // Layer bar.
                ui.set_clip_rect(right_clip);
                if !graph_on {
                    let x_in = tm.x(layer.in_point.seconds());
                    let x_out = tm.x(layer.out_point.seconds());
                    let bar = Rect::from_min_max(pos2(x_in, r.min.y + 3.0), pos2(x_out, r.max.y - 3.0));
                    let lc = t.label(layer.label);
                    let fill = if is_sel { lc.gamma_multiply(0.85) } else { lc.gamma_multiply(0.55) };
                    gp.rect_filled(bar, 2.0, fill);
                    if is_sel {
                        gp.rect_stroke(bar, 2.0, Stroke::new(1.0, Color32::from_white_alpha(120)), StrokeKind::Inside);
                    }
                    // Source extent (e.g. trimmed footage) shown faint.
                    if let LayerSource::Footage { .. } | LayerSource::Comp { .. } = layer.source
                        && let Some(d) = layer.source.item().and_then(|i| app.session.project.item(i)).and_then(|i| i.duration())
                    {
                        let xs = tm.x(layer.start_time.seconds());
                        let xe = tm.x(layer.comp_time(d).seconds());
                        let ghost = Rect::from_min_max(pos2(xs.min(xe), r.min.y + 9.0), pos2(xs.max(xe), r.max.y - 9.0));
                        gp.rect_filled(ghost, 1.0, lc.gamma_multiply(0.18));
                        // Dragging the source bar outside the in/out span slips the source.
                        for (side, gr) in [
                            ("l", Rect::from_min_max(ghost.min, pos2(bar.min.x, ghost.max.y))),
                            ("r", Rect::from_min_max(pos2(bar.max.x, ghost.min.y), ghost.max)),
                        ] {
                            if gr.width() < 2.0 {
                                continue;
                            }
                            let gresp = ui.interact(gr, egui::Id::new(("ghost", side, layer.id.0)), Sense::drag()).on_hover_text("Drag to slip the source");
                            app.auto.add(&format!("timeline.layer.{}.source{}", layer.id.0, side), gr, "Slip");
                            if gresp.hovered() || gresp.dragged() {
                                ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                            }
                            let acc_id = egui::Id::new(("slip-acc", layer.id.0));
                            if gresp.dragged() {
                                let mut acc: f32 = ctx.data(|d| d.get_temp(acc_id).unwrap_or(0.0));
                                acc += gresp.drag_delta().x;
                                let frames = ((acc as f64 / pps) / fd).trunc() as i64;
                                if frames != 0 {
                                    acc -= (frames as f64 * fd * pps) as f32;
                                    actions.push((
                                        "layer.slip".into(),
                                        json!({"layers": [layer.id.0], "frames": frames, "merge": format!("slip-{}", layer.id.0)}),
                                    ));
                                }
                                ctx.data_mut(|d| d.insert_temp(acc_id, acc));
                            }
                            if gresp.drag_stopped() {
                                ctx.data_mut(|d| d.remove::<f32>(acc_id));
                                ui_actions.push(UiAct::EndMerge);
                            }
                        }
                    }
                    app.auto.add(&format!("timeline.layer.{}.bar", layer.id.0), bar, &layer.name);
                    // Interactions: move body, trim edges.
                    let edge_w = 6.0;
                    let body = ui.interact(bar.shrink2(vec2(edge_w, 0.0)), egui::Id::new(("bar", layer.id.0)), Sense::click_and_drag());
                    let lin =
                        ui.interact(Rect::from_min_max(bar.min, pos2(bar.min.x + edge_w, bar.max.y)), egui::Id::new(("bar-in", layer.id.0)), Sense::drag());
                    let lout =
                        ui.interact(Rect::from_min_max(pos2(bar.max.x - edge_w, bar.min.y), bar.max), egui::Id::new(("bar-out", layer.id.0)), Sense::drag());
                    if lin.hovered() || lout.hovered() || lin.dragged() || lout.dragged() {
                        ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    if (body.clicked() || body.drag_started()) && !is_sel {
                        actions.push(("layer.select".into(), json!({"layers": [layer.id.0], "add": ui.input(|i| i.modifiers.shift)})));
                    }
                    layer_context_menu(&body, layer, &mut actions);
                    let drag_key = format!("bar-{}", layer.id.0);
                    for (resp, kind) in [(&body, "move"), (&lin, "in"), (&lout, "out")] {
                        if resp.dragged() {
                            let acc_id = egui::Id::new(("bar-acc", layer.id.0, kind));
                            let mut acc: f32 = ctx.data(|d| d.get_temp(acc_id).unwrap_or(0.0));
                            acc += resp.drag_delta().x;
                            let frames = ((acc as f64 / pps) / fd).trunc() as i64;
                            if frames != 0 {
                                acc -= (frames as f64 * fd * pps) as f32;
                                let d = frames as f64 * fd;
                                let params = match kind {
                                    "move" => {
                                        json!({"layers": if is_sel { json!(selected.iter().map(|l| l.0).collect::<Vec<_>>()) } else { json!([layer.id.0]) }, "delta": d, "merge": drag_key})
                                    }
                                    "in" => json!({"layers": [layer.id.0], "in": (layer.in_point.seconds() + d).max(0.0), "merge": drag_key}),
                                    _ => {
                                        json!({"layers": [layer.id.0], "out": (layer.out_point.seconds() + d).min(comp.duration.seconds()), "merge": drag_key})
                                    }
                                };
                                actions.push(("layer.timing".into(), params));
                            }
                            ctx.data_mut(|d| d.insert_temp(acc_id, acc));
                        }
                        if resp.drag_stopped() {
                            ctx.data_mut(|d| d.remove::<f32>(egui::Id::new(("bar-acc", layer.id.0, kind))));
                            ui_actions.push(UiAct::EndMerge);
                        }
                    }
                    // Markers on the layer.
                    for m in &layer.markers {
                        let x = tm.x(layer.comp_time(m.time).seconds());
                        gp.add(egui::Shape::convex_polygon(
                            vec![pos2(x - 3.5, r.min.y + 3.0), pos2(x + 3.5, r.min.y + 3.0), pos2(x, r.min.y + 9.0)],
                            Color32::from_rgb(0xd8, 0xd8, 0x60),
                            Stroke::NONE,
                        ));
                    }
                }
            }
            RowKind::Group { uid, name, open, has_children, fx, eye } => {
                lp.rect_filled(left, 0.0, if ri % 2 == 0 { t.row } else { t.row_alt });
                gp.rect_filled(Rect::from_min_max(pos2(graph_x0, r.min.y), r.max), 0.0, t.tl_bg);
                let indent = cw.name + 6.0 + 14.0 * row.depth as f32;
                if *has_children {
                    let tw = Rect::from_center_size(pos2(indent, cy), vec2(13.0, 13.0));
                    if widgets::twirl(ui, tw, *open, egui::Id::new(("gtw", uid)), &t).clicked() {
                        ui_actions.push(UiAct::ToggleGroup(*uid));
                    }
                    app.auto.add(&format!("timeline.group.{uid}.twirl"), tw, name);
                }
                if let Some(en) = fx {
                    let fr_ = Rect::from_center_size(pos2(cw.switches + SW * 3.0 + SW / 2.0, cy), vec2(16.0, 16.0));
                    if widgets::icon_toggle(ui, fr_, Icon::Fx, *en, &t, egui::Id::new(("gfx", uid)), None).clicked() {
                        actions.push(("effect.toggle".into(), json!({"layer": layer.id.0, "effect": uid})));
                    }
                    app.auto.add(&format!("timeline.group.{uid}.fx"), fr_, name);
                }
                if let Some(en) = eye {
                    // Layer style eye switch in the A/V column, like AE.
                    let er = Rect::from_center_size(pos2(cw.av + 10.0, cy), vec2(17.0, 17.0));
                    lp.rect_stroke(er.shrink(1.5), 2.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
                    if widgets::icon_toggle(ui, er, Icon::Eye, *en, &t, egui::Id::new(("geye", uid)), None).clicked() {
                        actions.push(("layer.style.toggle".into(), json!({"layer": layer.id.0, "style": uid})));
                    }
                    app.auto.add(&format!("timeline.group.{uid}.eye"), er, name);
                }
                lp.text(pos2(indent + 10.0, cy), Align2::LEFT_CENTER, name, Tokens::ui(12.0), t.text);
                text_anim_popups(app, ui, &lp, layer, *uid, cw.switches, cy, &mut actions);
                // Mask mode + inverted inline.
                if let Some(g) = layer.props.find_group(*uid)
                    && let GroupKind::Mask { mode, inverted, .. } = g.kind
                {
                    let mr = Rect::from_min_size(pos2(cw.switches + 4.0, cy - 9.0), vec2(80.0, 18.0));
                    let pop = egui::Id::new(("maskmode", uid));
                    if widgets::dropdown(ui, mr, mode.label(), &t, egui::Id::new(("mm", uid))).clicked() {
                        widgets::open_popup(ui, pop);
                    }
                    let opts: Vec<String> = effectcraft_engine::project::MaskMode::ALL.iter().map(|m| m.label().to_string()).collect();
                    if let Some(i) = widgets::popup_menu(ui, pop, mr.left_bottom(), &opts, None) {
                        actions.push(("layer.setMask".into(), json!({"layer": layer.id.0, "mask": uid, "mode": opts[i]})));
                    }
                    let ir = Rect::from_min_size(pos2(mr.max.x + 8.0, cy - 8.0), vec2(16.0, 16.0));
                    if widgets::checkbox(ui, ir, inverted, &t, egui::Id::new(("minv", uid))).clicked() {
                        actions.push(("layer.setMask".into(), json!({"layer": layer.id.0, "mask": uid, "inverted": !inverted})));
                    }
                    lp.text(pos2(ir.max.x + 4.0, cy), Align2::LEFT_CENTER, "Inverted", Tokens::ui(11.0), t.text_dim);
                }
                let gr = ui.interact(Rect::from_min_max(pos2(indent + 8.0, r.min.y), pos2(cw.switches, r.max.y)), egui::Id::new(("grow", uid)), Sense::click());
                if gr.clicked() && uid & WAVE_BIT == 0 {
                    actions.push(("prop.select".into(), json!({"layer": layer.id.0, "prop": uid, "selectKeys": false})));
                }
                if gr.double_clicked() {
                    ui_actions.push(UiAct::ToggleGroup(*uid));
                }
            }
            RowKind::Waveform { item } => {
                lp.rect_filled(left, 0.0, if ri % 2 == 0 { t.row } else { t.row_alt });
                let indent = cw.name + 6.0 + 14.0 * row.depth as f32;
                lp.text(pos2(indent + 10.0, cy), Align2::LEFT_CENTER, "Waveform", Tokens::ui(12.0), t.text);
                let wr = Rect::from_min_max(pos2(graph_x0, r.min.y), r.max);
                gp.rect_filled(wr, 0.0, t.tl_bg);
                app.auto.add(&format!("timeline.layer.{}.waveform", layer.id.0), wr, "Waveform");
                match super::waveform::summary(app, &ctx, effectcraft_engine::project::ItemId(*item)) {
                    Some(s) => super::waveform::draw(&gp, wr.shrink2(vec2(0.0, 2.0)), &tm, layer, &s, Color32::from_rgb(0x5f, 0xc8, 0x8a)),
                    None => {
                        gp.text(
                            pos2(tm.x(layer.in_point.seconds()).max(wr.min.x) + 6.0, wr.center().y),
                            Align2::LEFT_CENTER,
                            "Building waveform…",
                            Tokens::ui(11.0),
                            t.text_faint,
                        );
                    }
                }
            }
            RowKind::Expr { uid, lines } => {
                let Some(prop) = layer.props.find(*uid) else { continue };
                let Some(ex) = prop.expr.clone() else { continue };
                lp.rect_filled(left, 0.0, t.row_alt);
                let indent = cw.name + 6.0 + 14.0 * row.depth as f32;
                lp.text(pos2(indent + 12.0, cy), Align2::LEFT_CENTER, "Expression:", Tokens::ui(11.5), t.text_dim);
                lp.text(pos2(indent + 82.0, cy), Align2::LEFT_CENTER, &prop.name, Tokens::ui(11.5), t.text);
                // "=" enable switch and the property pick whip (AE's expression controls).
                let en_r = Rect::from_center_size(pos2(cw.switches + 10.0, cy), vec2(16.0, 16.0));
                let en = ui.interact(en_r, egui::Id::new(("expr-en", uid)), Sense::click()).on_hover_text("Enable Expression");
                lp.rect_stroke(en_r.shrink(1.0), 2.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
                let expr_col = Color32::from_rgb(0xe8, 0x7c, 0x5c);
                lp.text(
                    en_r.center(),
                    Align2::CENTER_CENTER,
                    if ex.enabled { "=" } else { "≠" },
                    Tokens::semibold(13.0),
                    if ex.enabled { expr_col } else { t.text_dim },
                );
                app.auto.add(&format!("timeline.prop.{uid}.exprEnable"), en_r, "Enable Expression");
                if en.clicked() {
                    actions.push(("prop.setExpression".into(), json!({"layer": layer.id.0, "prop": uid, "enabled": !ex.enabled})));
                }
                let pw_rect = Rect::from_center_size(pos2(cw.switches + 30.0, cy), vec2(15.0, 15.0));
                let pw = ui.interact(pw_rect, egui::Id::new(("expr-whip", uid)), Sense::drag()).on_hover_text("Expression pick whip: drag onto a property");
                icons::paint(&lp, pw_rect.shrink(1.0), Icon::PickWhip, if pw.hovered() || pw.dragged() { t.text } else { t.text_dim });
                app.auto.add(&format!("timeline.prop.{uid}.pickWhip"), pw_rect, "Expression pick whip");
                if pw.drag_started() {
                    let pw_state: PickWhip = (1, layer.id.0, *uid, pw_rect.center());
                    ctx.data_mut(|d| d.insert_temp(pick_whip_id(), pw_state));
                }
                // Syntax errors: warning in the outline (the expression is disabled).
                if let Some(check) = app.session.expr_check
                    && let Err(e) = check(&ex.text)
                {
                    let wr = Rect::from_center_size(pos2(cw.switches + 50.0, cy), vec2(14.0, 14.0));
                    lp.text(wr.center(), Align2::CENTER_CENTER, "⚠", Tokens::ui(12.0), Color32::from_rgb(0xf0, 0xa0, 0x30));
                    let _ = ui.interact(wr, egui::Id::new(("expr-err", uid)), Sense::hover()).on_hover_text(e);
                }
                // The editor in the time-graph area.
                ui.set_clip_rect(right_clip);
                gp.rect_filled(Rect::from_min_max(pos2(graph_x0, r.min.y), r.max), 0.0, Color32::from_rgb(0x1a, 0x1a, 0x1a));
                let er = Rect::from_min_max(pos2(graph_x0 + 8.0, r.min.y + 3.0), pos2(rect.max.x - 14.0, r.max.y - 3.0));
                let buf_id = egui::Id::new(("expr-buf", uid));
                let mut buf: String = ctx.data(|d| d.get_temp(buf_id)).unwrap_or_else(|| ex.text.clone());
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(er));
                let resp = child.add(
                    egui::TextEdit::multiline(&mut buf)
                        .id(egui::Id::new(("expr-edit", uid)))
                        .font(Tokens::mono(11.5))
                        .text_color(if ex.enabled { expr_col } else { t.text_dim })
                        .desired_width(er.width())
                        .desired_rows((*lines).clamp(1, 8))
                        .frame(egui::Frame::NONE),
                );
                app.auto.add(&format!("timeline.prop.{uid}.expression"), er, &prop.name);
                let commit = resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && (i.modifiers.command || i.modifiers.ctrl));
                if commit {
                    resp.surrender_focus();
                }
                if resp.has_focus() && !commit {
                    ctx.data_mut(|d| d.insert_temp(buf_id, buf));
                } else {
                    if (resp.lost_focus() || commit) && buf.trim_end() != ex.text {
                        actions.push(("prop.setExpression".into(), json!({"layer": layer.id.0, "prop": uid, "expression": buf.trim_end()})));
                    }
                    ctx.data_mut(|d| d.remove::<String>(buf_id));
                }
            }
            RowKind::Prop { uid } => {
                let Some(prop) = layer.props.find(*uid) else { continue };
                let sel_prop = app.session.state.selected_props.contains(&(layer.id, *uid));
                lp.rect_filled(
                    left,
                    0.0,
                    if sel_prop {
                        t.row_selected
                    } else if ri % 2 == 0 {
                        t.row
                    } else {
                        t.row_alt
                    },
                );
                gp.rect_filled(Rect::from_min_max(pos2(graph_x0, r.min.y), r.max), 0.0, t.tl_bg);
                let indent = cw.name + 6.0 + 14.0 * row.depth as f32;
                // Keyframe navigator (in the A/V column, like AE).
                if prop.is_animated() {
                    let lt = layer.layer_time(time);
                    let at_key = effectcraft_engine::keyframe::key_at(&prop.keys, lt).is_some();
                    let nav_x = cw.av + 18.0;
                    let prev = Rect::from_center_size(pos2(nav_x, cy), vec2(12.0, 14.0));
                    let mid = Rect::from_center_size(pos2(nav_x + 16.0, cy), vec2(14.0, 14.0));
                    let next = Rect::from_center_size(pos2(nav_x + 32.0, cy), vec2(12.0, 14.0));
                    icons::paint(&lp, prev.shrink(2.0), Icon::ChevronLeft, t.text_dim);
                    icons::paint(&lp, next.shrink(2.0), Icon::ChevronRight, t.text_dim);
                    let kc = if at_key { t.keyframe_selected } else { Color32::TRANSPARENT };
                    lp.add(egui::Shape::convex_polygon(
                        vec![mid.center() + vec2(0.0, -5.0), mid.center() + vec2(5.0, 0.0), mid.center() + vec2(0.0, 5.0), mid.center() + vec2(-5.0, 0.0)],
                        kc,
                        Stroke::new(1.0, t.text_dim),
                    ));
                    if ui.interact(prev, egui::Id::new(("kprev", uid)), Sense::click()).clicked()
                        && let Some(k) = prop.keys.iter().rev().find(|k| layer.comp_time(k.time).0 < time.0 - fr.frame_duration().0 / 2)
                    {
                        ui_actions.push(UiAct::SetTime(layer.comp_time(k.time)));
                    }
                    if ui.interact(next, egui::Id::new(("knext", uid)), Sense::click()).clicked()
                        && let Some(k) = prop.keys.iter().find(|k| layer.comp_time(k.time).0 > time.0 + fr.frame_duration().0 / 2)
                    {
                        ui_actions.push(UiAct::SetTime(layer.comp_time(k.time)));
                    }
                    if ui.interact(mid, egui::Id::new(("kmid", uid)), Sense::click()).clicked() {
                        actions.push(("prop.toggleKey".into(), json!({"layer": layer.id.0, "prop": uid})));
                    }
                    app.auto.add(&format!("timeline.prop.{uid}.addKey"), mid, "Add or remove keyframe");
                }
                // Stopwatch.
                if !prop.static_only {
                    let swr = Rect::from_center_size(pos2(indent, cy), vec2(15.0, 15.0));
                    let resp = ui.interact(swr, egui::Id::new(("stopwatch", uid)), Sense::click());
                    let col = if prop.is_animated() {
                        t.hot_text
                    } else if resp.hovered() {
                        t.text
                    } else {
                        t.text_dim
                    };
                    icons::paint(&lp, swr, Icon::Stopwatch, col);
                    app.auto.add(&format!("timeline.prop.{uid}.stopwatch"), swr, &prop.name);
                    if resp.clicked() {
                        if ui.input(|i| i.modifiers.alt) {
                            actions.push(("prop.setExpression".into(), json!({"layer": layer.id.0, "prop": uid})));
                        } else {
                            actions.push(("prop.toggleAnimation".into(), json!({"layer": layer.id.0, "prop": uid})));
                        }
                    }
                }
                let name_x = indent + 12.0;
                // 3D layers show Rotation as "Z Rotation" next to X/Y Rotation.
                let is_tr_rot = layer.transform().and_then(|t| t.get("rotation")).is_some_and(|r| r.uid == prop.uid);
                let base = if is_tr_rot && prop.name == "Rotation" && layer.is_3d() { "Z Rotation".to_string() } else { prop.name.clone() };
                let pname = if prop.has_expression() { format!("{base}  =") } else { base };
                lp.text(
                    pos2(name_x, cy),
                    Align2::LEFT_CENTER,
                    &pname,
                    Tokens::ui(12.0),
                    if prop.has_expression() { Color32::from_rgb(0xe8, 0x7c, 0x5c) } else { t.text },
                );
                let name_resp =
                    ui.interact(Rect::from_min_max(pos2(name_x, r.min.y), pos2(cw.switches - 2.0, r.max.y)), egui::Id::new(("pname", uid)), Sense::click());
                if name_resp.clicked() {
                    actions.push(("prop.select".into(), json!({"layer": layer.id.0, "prop": uid, "add": ui.input(|i| i.modifiers.shift)})));
                }
                // Value editors.
                let value = ectx.value(layer, prop);
                let vx = (cw.switches + 4.0).max(name_x + 120.0);
                value_editor(app, ui, &lp, layer, prop, &value, pos2(vx, cy), &mut actions);
                ui.set_clip_rect(right_clip);
                // Expression text row hint.
                if let Some(e) = prop.expr.as_ref().filter(|e| e.enabled && app.ui.timeline.expr_closed.contains(uid)) {
                    gp.text(
                        pos2(graph_x0 + 8.0, cy),
                        Align2::LEFT_CENTER,
                        format!("= {}", e.text.lines().next().unwrap_or("")),
                        Tokens::mono(11.0),
                        Color32::from_rgb(0xe8, 0x7c, 0x5c),
                    );
                }
                // Keyframes.
                if !graph_on {
                    for k in &prop.keys {
                        let ct = layer.comp_time(k.time);
                        let x = tm.x(ct.seconds());
                        if x < graph_x0 - 8.0 || x > rect.max.x + 8.0 {
                            continue;
                        }
                        let kref = effectcraft_engine::KeyRef { layer: layer.id, prop: *uid, time: k.time };
                        let ks = sel_keys.contains(&kref);
                        let icon = k.icon();
                        let c = pos2(x, cy);
                        icons::keyframe(&gp, c, 11.0, icon.left, icon.right, if ks { t.keyframe_selected } else { t.keyframe }, Color32::from_black_alpha(200));
                        let kr = Rect::from_center_size(c, vec2(12.0, 14.0));
                        let kresp = ui.interact(kr, egui::Id::new(("key", uid, k.time.0)), Sense::click_and_drag());
                        app.auto.add(&format!("timeline.key.{uid}.{}", fr.frame_at(ct)), kr, &format!("{} key", prop.name));
                        if kresp.clicked() || kresp.drag_started() {
                            let shift = ui.input(|i| i.modifiers.shift);
                            if !ks || shift {
                                actions.push((
                                    "keys.select".into(),
                                    json!({"keys": [{"layer": layer.id.0, "prop": uid, "time": k.time.seconds()}], "add": shift}),
                                ));
                            }
                        }
                        if kresp.double_clicked() {
                            ui_actions.push(UiAct::SetTime(ct));
                        }
                        kresp.context_menu(|ui| {
                            for (lbl, cmd, params) in [
                                ("Copy", "keys.copy", json!({})),
                                ("Paste", "keys.paste", json!({})),
                                ("-", "", json!({})),
                                ("Keyframe Interpolation…", "keys.interpolation", json!({})),
                                ("Keyframe Velocity…", "keys.velocity", json!({})),
                                ("Toggle Hold Keyframe", "keys.toggleHold", json!({})),
                                ("Rove Across Time", "keys.interpolation", json!({"roving": !k.roving})),
                                ("-", "", json!({})),
                                ("Easy Ease", "keys.easyEase", json!({})),
                                ("Easy Ease In", "keys.easyEaseIn", json!({})),
                                ("Easy Ease Out", "keys.easyEaseOut", json!({})),
                                ("Time-Reverse Keyframes", "keys.timeReverse", json!({})),
                                ("-", "", json!({})),
                                ("Linear", "keys.interpolation", json!({"interpolation": "linear"})),
                                ("Bezier", "keys.interpolation", json!({"interpolation": "bezier"})),
                                ("Auto Bezier", "keys.interpolation", json!({"interpolation": "autoBezier"})),
                                ("Select All Keyframes", "keys.selectAll", json!({})),
                                ("Delete", "keys.delete", json!({})),
                            ] {
                                if lbl == "-" {
                                    ui.separator();
                                    continue;
                                }
                                if lbl == "Rove Across Time" && !prop.spatial {
                                    continue;
                                }
                                if ui.button(lbl).clicked() {
                                    if !ks {
                                        actions.push(("keys.select".into(), json!({"keys": [{"layer": layer.id.0, "prop": uid, "time": k.time.seconds()}]})));
                                    }
                                    actions.push((cmd.into(), params));
                                    ui.close();
                                }
                            }
                        });
                        if kresp.dragged() {
                            let acc_id = egui::Id::new("key-drag-acc");
                            let mut acc: f32 = ctx.data(|d| d.get_temp(acc_id).unwrap_or(0.0));
                            acc += kresp.drag_delta().x;
                            let frames = ((acc as f64 / pps) / fd).trunc() as i64;
                            if frames != 0 {
                                acc -= (frames as f64 * fd * pps) as f32;
                                actions.push(("keys.move".into(), json!({"delta": frames as f64 * fd, "merge": "key-drag"})));
                            }
                            ctx.data_mut(|d| d.insert_temp(acc_id, acc));
                        }
                        if kresp.drag_stopped() {
                            ctx.data_mut(|d| d.remove::<f32>(egui::Id::new("key-drag-acc")));
                            ui_actions.push(UiAct::EndMerge);
                        }
                    }
                }
            }
        }
    }
    ui.set_clip_rect(full_clip);
    // Pick whips: a line follows the pointer; releasing over a row links to it.
    if let Some((kind, src_layer, src_prop, start)) = ctx.data(|d| d.get_temp::<PickWhip>(pick_whip_id())) {
        let ptr = ctx.input(|i| i.pointer.latest_pos()).unwrap_or(start);
        let fg = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("tl-pickwhip-line")));
        fg.line_segment([start, ptr], Stroke::new(1.5, t.accent));
        fg.circle_stroke(ptr, 4.0, Stroke::new(1.5, t.accent));
        let target = hit_rows.iter().find(|(r, _)| r.contains(ptr)).map(|(r, row)| (*r, row.clone()));
        if let Some((tr, _)) = &target {
            fg.rect_stroke(Rect::from_min_max(tr.min, pos2(graph_x0 - 1.0, tr.max.y)), 0.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
        }
        if ctx.input(|i| !i.pointer.any_down()) {
            ctx.data_mut(|d| d.remove::<PickWhip>(pick_whip_id()));
            match (kind, target) {
                (0, Some((_, row))) if row.layer.0 != src_layer => {
                    actions.push(("layer.setParent".into(), json!({"layers": [src_layer], "parent": row.layer.0})));
                }
                (1, Some((_, Row { layer, kind: RowKind::Prop { uid }, .. }))) if uid != src_prop => {
                    actions.push(("prop.pickWhip".into(), json!({"layer": src_layer, "prop": src_prop, "target": {"layer": layer.0, "prop": uid}})));
                }
                _ => {}
            }
        }
    }
    // Graph editor.
    if graph_on {
        super::graph::show(app, ui, &gp, &comp, &ectx, tm, Rect::from_min_max(pos2(graph_x0, rows_rect.min.y), rows_rect.max), &mut actions);
    }
    // Empty-area click in the graph: deselect keys; drag: box-select keys.
    if empty.clicked() {
        actions.push(("keys.select".into(), json!({"keys": []})));
    }
    if let (true, Some(origin), Some(cur)) =
        (empty.dragged(), empty.interact_pointer_pos().and_then(|_| ctx.input(|i| i.pointer.press_origin())), empty.interact_pointer_pos())
    {
        let br = Rect::from_two_pos(origin, cur);
        gp.rect_filled(br, 0.0, t.accent.gamma_multiply(0.12));
        gp.rect_stroke(br, 0.0, Stroke::new(1.0, t.accent), StrokeKind::Middle);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("tl-box"), br));
    }
    if empty.drag_stopped()
        && let Some(br) = ctx.data(|d| d.get_temp::<Rect>(egui::Id::new("tl-box")))
    {
        ctx.data_mut(|d| d.remove::<Rect>(egui::Id::new("tl-box")));
        let mut keys = vec![];
        let mut yy = rows_rect.min.y - app.ui.timeline.scroll_y;
        for row in &rows {
            let cy = yy + rh / 2.0;
            yy += row_height(row, rh);
            if let RowKind::Prop { uid } = row.kind
                && cy >= br.min.y
                && cy <= br.max.y
                && let Some(l) = comp.layer(row.layer)
                && let Some(pr) = l.props.find(uid)
            {
                for k in &pr.keys {
                    let x = tm.x(l.comp_time(k.time).seconds());
                    if x >= br.min.x && x <= br.max.x {
                        keys.push(json!({"layer": l.id.0, "prop": uid, "time": k.time.seconds()}));
                    }
                }
            }
        }
        actions.push(("keys.select".into(), json!({"keys": keys, "add": ui.input(|i| i.modifiers.shift)})));
    }

    // CTI.
    let cx = tm.x(time.seconds());
    if cx >= graph_x0 && cx <= rect.max.x {
        p.line_segment([pos2(cx, ruler.min.y + 12.0), pos2(cx, rows_rect.max.y)], Stroke::new(1.0, t.cti));
        let head = vec![
            pos2(cx - 6.0, ruler.min.y + 12.0),
            pos2(cx + 6.0, ruler.min.y + 12.0),
            pos2(cx + 6.0, ruler.max.y - 8.0),
            pos2(cx, ruler.max.y - 2.0),
            pos2(cx - 6.0, ruler.max.y - 8.0),
        ];
        p.add(egui::Shape::convex_polygon(head, t.cti, Stroke::NONE));
        app.auto.add("timeline.cti", Rect::from_center_size(pos2(cx, ruler.center().y), vec2(12.0, 20.0)), "Current time indicator");
    }
    let _ = graph_rect;

    // Footer.
    let footer = Rect::from_min_max(pos2(rect.min.x, rect.max.y - footer_h), rect.max);
    p.rect_filled(footer, 0.0, t.panel_bg);
    p.line_segment([footer.left_top(), footer.right_top()], Stroke::new(1.0, t.separator));
    let tgl = Rect::from_min_size(pos2(footer.min.x + 10.0, footer.min.y + 3.0), vec2(150.0, 18.0));
    let tresp = ui.interact(tgl, egui::Id::new("tl-toggle-modes"), Sense::click());
    p.text(tgl.left_center(), Align2::LEFT_CENTER, "Toggle Switches / Modes", Tokens::ui(11.0), if tresp.hovered() { t.text } else { t.text_dim });
    app.auto.add("timeline.toggleSwitchesModes", tgl, "Toggle Switches / Modes");
    if tresp.clicked() {
        app.ui.timeline.show_modes = !app.ui.timeline.show_modes;
    }
    let ms = app.frames.last_ms.lock().map(|v| *v).unwrap_or(0.0);
    p.text(pos2(graph_x0 - 12.0, footer.center().y), Align2::RIGHT_CENTER, format!("Frame Render Time  {ms:.0}ms"), Tokens::ui(11.0), t.text_faint);
    // Zoom slider.
    let zs = Rect::from_min_max(pos2(graph_x0 + 30.0, footer.center().y - 2.0), pos2((graph_x0 + 230.0).min(rect.max.x - 30.0), footer.center().y + 2.0));
    icons::paint(&p, Rect::from_center_size(pos2(zs.min.x - 14.0, footer.center().y), vec2(14.0, 14.0)), Icon::MountainSmall, t.text_dim);
    icons::paint(&p, Rect::from_center_size(pos2(zs.max.x + 14.0, footer.center().y), vec2(16.0, 16.0)), Icon::MountainLarge, t.text_dim);
    p.rect_filled(zs, 2.0, t.field_bg);
    let maxpps = 4000.0f64;
    let frac = ((pps / fit_pps).ln() / (maxpps / fit_pps).ln()).clamp(0.0, 1.0) as f32;
    let knob = pos2(zs.min.x + zs.width() * frac, zs.center().y);
    p.circle_filled(knob, 6.0, t.text);
    let zresp = ui.interact(zs.expand2(vec2(6.0, 8.0)), egui::Id::new("tl-zoom"), Sense::drag());
    app.auto.add("timeline.zoomSlider", zs, "Zoom");
    if zresp.dragged()
        && let Some(pt) = zresp.interact_pointer_pos()
    {
        let f = ((pt.x - zs.min.x) / zs.width()).clamp(0.0, 1.0) as f64;
        let npps = fit_pps * (maxpps / fit_pps).powf(f);
        let cti = time.seconds();
        let rel = (cti - tm.start) * pps;
        app.ui.timeline.start = (cti - rel / npps).max(0.0);
        app.ui.timeline.pps = Some(npps);
    }

    // Drop targets: footage/comps from the Project panel, effects from Effects & Presets.
    if let Some(payload) = egui::DragAndDrop::payload::<crate::panels::DragPayload>(&ctx)
        && ui.rect_contains_pointer(rows_rect)
    {
        p.rect_stroke(rows_rect, 0.0, Stroke::new(2.0, t.accent), StrokeKind::Inside);
        if ctx.input(|i| i.pointer.any_released()) {
            let hover_y = ctx.input(|i| i.pointer.hover_pos()).map(|p| p.y).unwrap_or(0.0);
            let target = hit_rows.iter().find(|(r, _)| hover_y >= r.min.y && hover_y < r.max.y).map(|(_, row)| row.layer);
            match payload.as_ref() {
                crate::panels::DragPayload::Item(id) => actions.push(("layer.addItem".into(), json!({"item": id}))),
                crate::panels::DragPayload::Effect(e) => {
                    if let Some(l) = target {
                        actions.push(("effect.apply".into(), json!({"effect": e, "layers": [l.0]})));
                    }
                }
            }
            egui::DragAndDrop::clear_payload(&ctx);
        }
    }

    // Apply.
    for a in ui_actions {
        match a {
            UiAct::ToggleLayer(id, all) => {
                let tl = &mut app.ui.timeline;
                tl.reveal.clear();
                let open = !tl.open_layers.contains(&id);
                if open {
                    tl.open_layers.insert(id);
                } else {
                    tl.open_layers.remove(&id);
                }
                if all && let Some(l) = comp.layer(LayerId(id)) {
                    let mut uids = vec![];
                    walk_groups(&l.props, &mut uids);
                    for u in uids {
                        if open {
                            tl.open_groups.insert(u);
                        } else {
                            tl.open_groups.remove(&u);
                        }
                    }
                }
            }
            UiAct::ToggleGroup(uid) => {
                if !app.ui.timeline.open_groups.remove(&uid) {
                    app.ui.timeline.open_groups.insert(uid);
                }
            }
            UiAct::SetTime(tt) => app.session.set_time(tt),
            UiAct::EndMerge => app.session.history.merge_key = None,
        }
    }
    for (id, params) in actions {
        if id == "__endMerge" {
            app.session.history.merge_key = None;
            continue;
        }
        if let Err(e) = crate::menus::invoke(app, &ctx, &id, params) {
            app.ui.status = e;
        }
    }
}

enum UiAct {
    ToggleLayer(u64, bool),
    ToggleGroup(u64),
    SetTime(Tick),
    EndMerge,
}

fn walk_groups(g: &PropGroup, out: &mut Vec<u64>) {
    for sg in g.groups() {
        out.push(sg.uid);
        walk_groups(sg, out);
    }
}

/// The Text group's "Animate:" and an animator's "Add:" pop-up menus (AE's twirl-down menus).
#[allow(clippy::too_many_arguments)]
fn text_anim_popups(
    app: &mut EffectcraftApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layer: &Layer,
    uid: u64,
    x: f32,
    cy: f32,
    actions: &mut Vec<(String, serde_json::Value)>,
) {
    use effectcraft_engine::project::build::TEXT_ANIMATOR_KINDS;
    if !matches!(layer.source, LayerSource::Text) {
        return;
    }
    let Some(g) = layer.props.find_group(uid) else { return };
    let t = app.tokens;
    let (label, key) = match g.match_id.as_str() {
        "text" if layer.props.sub("text").is_some_and(|tg| tg.uid == uid) => ("Animate:", "animate"),
        "animator" => ("Add:", "add"),
        _ => return,
    };
    p.text(pos2(x + 6.0, cy), Align2::LEFT_CENTER, label, Tokens::ui(11.5), t.text_dim);
    let br = Rect::from_center_size(pos2(x + 6.0 + if key == "add" { 34.0 } else { 62.0 }, cy), vec2(16.0, 16.0));
    let resp = ui.interact(br, egui::Id::new(("tl-textanim", uid)), Sense::click()).on_hover_text(if key == "add" {
        "Add property or selector"
    } else {
        "Animate text"
    });
    icons::paint(p, br.shrink(3.0), Icon::ChevronRight, if resp.hovered() { t.text } else { t.text_dim });
    app.auto.add(&format!("timeline.group.{uid}.{key}"), br, label);
    let pop = egui::Id::new(("tl-textanim-pop", uid));
    if resp.clicked() {
        widgets::open_popup(ui, pop);
    }
    let per_char = layer.props.prop("text/perChar3d").is_some_and(|q| q.value.as_bool());
    let mut opts: Vec<String> = Vec::new();
    let mut acts: Vec<(String, serde_json::Value)> = Vec::new();
    if key == "animate" {
        opts.push(if per_char { "Disable Per-character 3D" } else { "Enable Per-character 3D" }.into());
        acts.push(("layer.enablePerChar3D".into(), json!({"layer": layer.id.0, "enabled": !per_char})));
        opts.push("-".into());
        acts.push((String::new(), json!(null)));
        for (k, l) in TEXT_ANIMATOR_KINDS {
            opts.push(l.to_string());
            acts.push(if *k == "-" { (String::new(), json!(null)) } else { ("layer.addTextAnimator".into(), json!({"layer": layer.id.0, "property": k})) });
        }
    } else {
        for (kind, l) in [("range", "Selector: Range"), ("wiggly", "Selector: Wiggly"), ("expression", "Selector: Expression")] {
            opts.push(l.into());
            acts.push(("layer.addTextSelector".into(), json!({"layer": layer.id.0, "animator": uid, "kind": kind})));
        }
        opts.push("-".into());
        acts.push((String::new(), json!(null)));
        for (k, l) in TEXT_ANIMATOR_KINDS {
            opts.push(if *k == "-" { "-".into() } else { format!("Property: {l}") });
            acts.push(if *k == "-" {
                (String::new(), json!(null))
            } else {
                ("layer.addTextAnimatorProperty".into(), json!({"layer": layer.id.0, "animator": uid, "property": k}))
            });
        }
    }
    if let Some(i) = widgets::popup_menu(ui, pop, br.left_bottom(), &opts, None)
        && let Some((id, params)) = acts.get(i).filter(|a| !a.0.is_empty())
    {
        actions.push((id.clone(), params.clone()));
    }
}

fn kind_name(k: MatteKind) -> &'static str {
    match k {
        MatteKind::Alpha => "alpha",
        MatteKind::AlphaInverted => "alphaInverted",
        MatteKind::Luma => "luma",
        MatteKind::LumaInverted => "lumaInverted",
    }
}

fn zoom_at(app: &mut EffectcraftApp, comp: &Comp, tm: TMap, k: f64, at_x: f32) {
    let fit = tm.pps / app.ui.timeline.pps.map(|p| p / tm.pps).unwrap_or(1.0);
    let npps = (tm.pps * k).clamp(fit.min(5.0), 4000.0);
    let at_t = tm.t(at_x);
    app.ui.timeline.start = (at_t - (at_x - tm.x0) as f64 / npps).clamp(0.0, comp.duration.seconds());
    app.ui.timeline.pps = Some(npps);
}

fn layer_context_menu(resp: &egui::Response, layer: &Layer, actions: &mut Vec<(String, serde_json::Value)>) {
    resp.context_menu(|ui| {
        let id = layer.id.0;
        let mut item = |ui: &mut egui::Ui, label: &str, cmd: &str, params: serde_json::Value| {
            if ui.button(label).clicked() {
                actions.push(("layer.select".into(), json!({"layers": [id]})));
                actions.push((cmd.into(), params));
                ui.close();
            }
        };
        ui.menu_button("New", |ui| {
            item(ui, "Text", "layer.newText", json!({"text": "Text"}));
            item(ui, "Solid…", "app.solidSettings", json!({}));
            item(ui, "Light…", "layer.newLight", json!({}));
            item(ui, "Camera…", "layer.newCamera", json!({}));
            item(ui, "Null Object", "layer.newNull", json!({}));
            item(ui, "Shape Layer", "layer.newShape", json!({"kind": "none"}));
            item(ui, "Adjustment Layer", "layer.newAdjustment", json!({}));
        });
        item(ui, "Layer Settings…", "layer.settings", json!({}));
        ui.separator();
        ui.menu_button("Masks", |ui| {
            item(ui, "New Mask", "layer.addMask", json!({"layer": id}));
        });
        ui.menu_button("Transform", |ui| {
            item(ui, "Reset", "layer.transform", json!({"layers": [id], "op": "reset"}));
            item(ui, "Center In View", "layer.transform", json!({"layers": [id], "op": "center"}));
            item(ui, "Fit to Comp", "layer.transform", json!({"layers": [id], "op": "fit"}));
            item(ui, "Flip Horizontal", "layer.transform", json!({"layers": [id], "op": "flipH"}));
            item(ui, "Flip Vertical", "layer.transform", json!({"layers": [id], "op": "flipV"}));
        });
        ui.menu_button("Time", |ui| {
            item(ui, "Enable Time Remapping", "layer.enableTimeRemap", json!({"layers": [id]}));
            item(ui, "Time-Reverse Layer", "layer.timeReverse", json!({"layers": [id]}));
            item(ui, "Time Stretch…", "layer.timeStretch", json!({}));
            item(ui, "Freeze Frame", "layer.freezeFrame", json!({"layers": [id]}));
            item(ui, "Freeze on Last Frame", "layer.freezeOnLastFrame", json!({"layers": [id]}));
        });
        ui.menu_button("Blending Mode", |ui| {
            for m in BlendMode::ALL {
                item(ui, m.label(), "layer.setBlendMode", json!({"layers": [id], "mode": m.label()}));
                if m.ends_group() {
                    ui.separator();
                }
            }
        });
        ui.menu_button("Arrange", |ui| {
            item(ui, "Bring Layer to Front", "layer.arrange", json!({"layers": [id], "to": "front"}));
            item(ui, "Bring Layer Forward", "layer.arrange", json!({"layers": [id], "to": "forward"}));
            item(ui, "Send Layer Backward", "layer.arrange", json!({"layers": [id], "to": "backward"}));
            item(ui, "Send Layer to Back", "layer.arrange", json!({"layers": [id], "to": "back"}));
        });
        ui.separator();
        item(ui, "Pre-compose…", "layer.precompose", json!({"layers": [id], "name": format!("{} Comp 1", layer.name)}));
        item(ui, "Duplicate", "edit.duplicate", json!({"layers": [id]}));
        item(ui, "Split Layer", "edit.splitLayer", json!({"layers": [id]}));
        item(ui, "Delete", "edit.clear", json!({"layers": [id]}));
    });
}

fn fmt_num(v: f64, d: usize) -> String {
    format!("{v:.d$}")
}

/// Inline value editor for a property row; pushes `prop.set` actions.
fn value_editor(
    app: &mut EffectcraftApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layer: &Layer,
    prop: &Property,
    value: &Value,
    at: Pos2,
    actions: &mut Vec<(String, serde_json::Value)>,
) {
    let t = app.tokens;
    let uid = prop.uid;
    let merge = format!("scrub-{uid}");
    let set = |actions: &mut Vec<(String, serde_json::Value)>, v: serde_json::Value| {
        actions.push(("prop.set".into(), json!({"layer": layer.id.0, "prop": uid, "value": v, "merge": merge})))
    };
    let is_3d = layer.is_3d();
    let mut x = at.x;
    let y = at.y - 9.0;
    match value {
        Value::Scalar(v) => {
            let (min, max, dec, speed) = match &prop.ui {
                ParamUi::Slider { min, max, decimals, .. } => (*min, *max, *decimals as usize, ((max - min) / 400.0).clamp(0.01, 10.0)),
                ParamUi::Angle => (-1e9, 1e9, 1, 0.5),
                ParamUi::Percent => (-1e6, 1e6, 1, 0.5),
                _ => (-1e9, 1e9, 1, 1.0),
            };
            if matches!(prop.ui, ParamUi::Angle) {
                let rev = (v / 360.0).trunc();
                let deg = v - rev * 360.0;
                p.text(pos2(x, at.y), Align2::LEFT_CENTER, format!("{}x", rev as i64), Tokens::ui(12.0), t.hot_text);
                x += 22.0;
                let (r, nv, _) = widgets::hot_number_at(ui, pos2(x, y), egui::Id::new(("v", uid, 0)), deg, speed, (-1e9, 1e9), 1, "°", &t);
                app.auto.add(&format!("timeline.prop.{uid}.value"), r, &prop.name);
                if let Some(nv) = nv {
                    set(actions, json!(rev * 360.0 + nv));
                }
            } else {
                let suffix = if matches!(prop.ui, ParamUi::Percent) || prop.match_id == "opacity" { "%" } else { "" };
                let (r, nv, _) = widgets::hot_number_at(ui, pos2(x, y), egui::Id::new(("v", uid, 0)), *v, speed, (min, max), dec.max(1), suffix, &t);
                app.auto.add(&format!("timeline.prop.{uid}.value"), r, &prop.name);
                if let Some(nv) = nv {
                    set(actions, json!(nv));
                }
            }
        }
        Value::Vec2(_) | Value::Vec3(_) => {
            let c = value.components();
            let n = if prop.shown_dims > 0 && !(is_3d && c.len() == 3) { prop.shown_dims as usize } else { c.len() };
            let pct = matches!(prop.ui, ParamUi::Percent);
            for d in 0..n.min(c.len()) {
                let suffix = if pct && d + 1 == n { "%" } else { "" };
                let (r, nv, _) =
                    widgets::hot_number_at(ui, pos2(x, y), egui::Id::new(("v", uid, d)), c[d], if pct { 0.5 } else { 1.0 }, (-1e9, 1e9), 1, suffix, &t);
                app.auto.add(&format!("timeline.prop.{uid}.value.{d}"), r, &prop.name);
                if let Some(nv) = nv {
                    let mut nc = c.clone();
                    if pct && prop.match_id == "scale" && !ui.input(|i| i.modifiers.alt) {
                        // Constrain proportions (AE's chain link, on by default).
                        let k = if c[d].abs() > 1e-9 { nv / c[d] } else { 1.0 };
                        for e in 0..n {
                            nc[e] = if e == d { nv } else { c[e] * k };
                        }
                    } else {
                        nc[d] = nv;
                    }
                    set(actions, json!(nc));
                }
                x = r.max.x + if d + 1 < n { 2.0 } else { 0.0 };
                if d + 1 < n {
                    p.text(pos2(x, at.y), Align2::LEFT_CENTER, ",", Tokens::ui(12.0), t.hot_text);
                    x += 6.0;
                }
            }
        }
        Value::Color(c) => {
            let r = Rect::from_min_size(pos2(x, at.y - 7.0), vec2(26.0, 14.0));
            let resp = widgets::swatch(ui, r, [c[0] as f32, c[1] as f32, c[2] as f32, 1.0], egui::Id::new(("sw", uid)), &t);
            app.auto.add(&format!("timeline.prop.{uid}.value"), r, &prop.name);
            let pop = egui::Id::new(("cpop", uid));
            if resp.clicked() {
                widgets::open_popup(ui, pop);
            }
            let mut rgb = [c[0] as f32, c[1] as f32, c[2] as f32];
            if crate::header::color_popup(ui, pop, r.left_bottom(), &mut rgb) {
                set(actions, json!([rgb[0], rgb[1], rgb[2], 1.0]));
            }
        }
        Value::Bool(b) => {
            let r = Rect::from_min_size(pos2(x, at.y - 8.0), vec2(16.0, 16.0));
            if widgets::checkbox(ui, r, *b, &t, egui::Id::new(("cb", uid))).clicked() {
                set(actions, json!(!b));
            }
            app.auto.add(&format!("timeline.prop.{uid}.value"), r, &prop.name);
        }
        Value::Enum(i) => {
            // Text Path Options ▸ Path lists the layer's masks.
            let mask_opts: Option<Vec<String>> = layer
                .props
                .prop("text/pathOptions/path")
                .filter(|q| q.uid == uid)
                .map(|_| std::iter::once("None".to_string()).chain(layer.masks().into_iter().flat_map(|m| m.groups().map(|g| g.name.clone()))).collect());
            let ui_opts = mask_opts.map(|options| ParamUi::Popup { options });
            if let ParamUi::Popup { options } = ui_opts.as_ref().unwrap_or(&prop.ui) {
                let r = Rect::from_min_size(pos2(x, at.y - 9.0), vec2(150.0, 18.0));
                let label = options.get(*i as usize).cloned().unwrap_or_default();
                let pop = egui::Id::new(("epop", uid));
                if widgets::dropdown(ui, r, &label, &t, egui::Id::new(("en", uid))).clicked() {
                    widgets::open_popup(ui, pop);
                }
                app.auto.add(&format!("timeline.prop.{uid}.value"), r, &prop.name);
                if let Some(ni) = widgets::popup_menu(ui, pop, r.left_bottom(), options, Some(*i as usize)) {
                    set(actions, json!(ni));
                }
            } else {
                p.text(pos2(x, at.y), Align2::LEFT_CENTER, fmt_num(*i as f64, 0), Tokens::ui(12.0), t.hot_text);
            }
        }
        Value::Path(_) => {
            p.text(pos2(x, at.y), Align2::LEFT_CENTER, "Shape…", Tokens::ui(12.0), t.hot_text);
        }
        Value::Text(_) => {}
        Value::Layer(l) => {
            p.text(pos2(x, at.y), Align2::LEFT_CENTER, l.map(|l| format!("Layer {l}")).unwrap_or("None".into()), Tokens::ui(12.0), t.hot_text);
        }
        Value::Gradient(_) => {
            p.text(pos2(x, at.y), Align2::LEFT_CENTER, "Edit Gradient…", Tokens::ui(12.0), t.hot_text);
        }
        Value::Str(s) => {
            p.text(pos2(x, at.y), Align2::LEFT_CENTER, s.chars().take(30).collect::<String>(), Tokens::ui(12.0), t.text_dim);
        }
    }
}
