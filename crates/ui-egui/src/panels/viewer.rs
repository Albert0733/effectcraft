//! The Composition viewer: pasteboard + comp frame, magnification and resolution, transparency
//! grid, guides, layer bounding boxes with handles, anchor points, motion paths and masks, and
//! direct manipulation (move, scale, rotate, pan behind, shape/type creation, hand and zoom),
//! 3D views (Active Camera / Front / … / Custom View 3), camera tools (orbit, pan, dolly) and
//! camera/light wireframes.

use effectcraft_engine::geom::{Mat3, vec2 as gv2};
use effectcraft_engine::project::{Layer, LayerId};
use effectcraft_engine::render::EvalCtx;
use effectcraft_engine::render::three_d::{self, CameraState, View3D};
use effectcraft_engine::time::Tick;
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::state::Tool;
use crate::theme::Tokens;
use crate::{EffectcraftApp, widgets};

/// Viewer mapping published each frame (for the control channel and other panels).
#[derive(Clone, Copy, Debug)]
pub struct ViewerMap {
    pub origin: Pos2,
    /// Points per comp pixel.
    pub zoom: f32,
    pub comp: [f32; 2],
    pub area: Rect,
}

impl ViewerMap {
    pub fn to_screen(&self, p: [f64; 2]) -> Pos2 {
        pos2(self.origin.x + p[0] as f32 * self.zoom, self.origin.y + p[1] as f32 * self.zoom)
    }
    pub fn to_comp(&self, p: Pos2) -> [f64; 2] {
        [((p.x - self.origin.x) / self.zoom) as f64, ((p.y - self.origin.y) / self.zoom) as f64]
    }
}

fn map_id() -> egui::Id {
    egui::Id::new("viewer-map")
}

pub fn comp_to_screen(ctx: &egui::Context, p: [f32; 2]) -> Option<Pos2> {
    let m: ViewerMap = ctx.data(|d| d.get_temp(map_id()))?;
    Some(m.to_screen([p[0] as f64, p[1] as f64]))
}

pub fn screen_to_comp(ctx: &egui::Context, p: Pos2) -> Option<[f64; 2]> {
    let m: ViewerMap = ctx.data(|d| d.get_temp(map_id()))?;
    Some(m.to_comp(p))
}

/// The fit magnification from the last frame.
pub fn last_fit(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(egui::Id::new("viewer-fit"))).unwrap_or(0.5)
}

/// What a drag in the viewer is doing.
#[derive(Clone, Debug)]
enum Gesture {
    /// Per layer: start position and the parent-space change per comp pixel of drag in x and y.
    Move {
        layers: Vec<(LayerId, [f64; 3], [f64; 3], [f64; 3])>,
        start: [f64; 2],
        /// The grabbed layer's feature nearest the pointer (comp px at the start): it snaps.
        snap_src: Option<[f64; 2]>,
    },
    Camera {
        tool: Tool,
    },
    Scale {
        layer: LayerId,
        start_scale: [f64; 3],
        start_local: [f64; 2],
        inv: Mat3,
    },
    Rotate {
        layer: LayerId,
        center: Pos2,
        start_angle: f64,
        start_rot: f64,
    },
    Anchor {
        layer: LayerId,
        start_anchor: [f64; 3],
        start_pos: [f64; 3],
        start: [f64; 2],
        inv: Mat3,
        l2p: Mat3,
    },
    Pan {
        start_pan: [f32; 2],
    },
    Create {
        tool: Tool,
        start: [f64; 2],
    },
    Marquee {
        start: Pos2,
    },
    /// Dragging selected mask / shape path vertices: press point, the grabbed vertex (comp px,
    /// it snaps), the comp-space move applied so far and comp → path space.
    Vertices {
        start: [f64; 2],
        src: [f64; 2],
        applied: [f64; 2],
        inv: Mat3,
        layer: LayerId,
    },
    /// Dragging a vertex's Bezier tangent (both sides symmetric unless Alt breaks them).
    Tangent {
        layer: LayerId,
        mask: u64,
        index: usize,
        out: bool,
        vertex: [f64; 2],
        inv: Mat3,
    },
    /// Dragging a Puppet pin (comp → layer matrix inverse).
    PuppetPin {
        layer: LayerId,
        pin: u64,
        inv: Mat3,
    },
    /// Pen tool: pulling the tangents of the vertex just placed.
    Pen {
        layer: LayerId,
        mask: u64,
        index: usize,
        vertex: [f64; 2],
        inv: Mat3,
        press: Pos2,
    },
    /// Dragging a track point's feature/search region, a corner or its attach point.
    Track {
        layer: LayerId,
        tracker: u64,
        drag: super::tracker::Drag,
    },
    /// Motion paths, free transform, Mask Feather, guides and the region of interest
    /// (`viewer_overlays`).
    Overlay(ov::Drag),
}

/// Pen tool path in progress: (layer, mask uid).
fn pen_id() -> egui::Id {
    egui::Id::new("viewer-pen")
}

use super::viewer_overlays::{self as ov, VertexHit};
use super::viewer_tools as vt;

const HANDLE: f32 = 7.0;

thread_local! {
    /// The 3D view camera of the viewer being drawn (None = the comp's active camera).
    static VIEW_CAM: std::cell::Cell<Option<CameraState>> = const { std::cell::Cell::new(None) };
}

/// The camera the viewer shows (view camera or the comp's active camera).
fn cam_state(ctx: &EvalCtx) -> CameraState {
    VIEW_CAM.with(|c| c.get()).unwrap_or_else(|| three_d::active_camera(ctx))
}

/// Layer → comp (viewer) matrix through the current 3D view.
pub(crate) fn l2c(ctx: &EvalCtx, layer: &Layer) -> (Mat3, f64) {
    match VIEW_CAM.with(|c| c.get()) {
        Some(cam) if layer.is_3d() => (three_d::layer_to_view(ctx, layer, &cam), 0.0),
        _ => ctx.layer_to_comp(layer),
    }
}

/// Layer → comp matrix and its bounds quad (in comp pixels).
fn layer_quad(ctx: &EvalCtx, layer: &Layer) -> Option<(Mat3, [[f64; 2]; 4], [f64; 4])> {
    let b = effectcraft_engine::render::content_bounds(ctx, layer)?;
    let (m, _) = l2c(ctx, layer);
    let pts = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|p| {
        let q = m.apply(gv2(p[0], p[1]));
        [q.x, q.y]
    });
    Some((m, pts, b))
}

fn point_in_quad(p: [f64; 2], q: &[[f64; 2]; 4]) -> bool {
    let mut sign = 0.0;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 1) % 4];
        let c = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if c.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

/// Parent-space linear inverse (to convert a comp-space drag delta into a position delta).
fn parent_inverse(ctx: &EvalCtx, layer: &Layer) -> Mat3 {
    match layer.parent.and_then(|p| ctx.comp.layer(p)) {
        Some(p) => {
            let w = ctx.world_matrix(p);
            let m = Mat3([[w.0[0][0], w.0[0][1], 0.0], [w.0[1][0], w.0[1][1], 0.0], [0.0, 0.0, 1.0]]);
            m.inverse().unwrap_or(Mat3::IDENTITY)
        }
        None => Mat3::IDENTITY,
    }
}

/// Draw the extra views of a 2- or 4-view layout and return the main view's rectangle.
fn aux_views(
    app: &mut EffectcraftApp,
    ui: &mut egui::Ui,
    comp: &effectcraft_engine::project::Comp,
    cid: effectcraft_engine::project::ItemId,
    full: Rect,
    bg: Color32,
) -> Rect {
    let n = app.session.state.view_layout;
    if n < 2 {
        return full;
    }
    let gap = 2.0;
    let (main, aux): (Rect, Vec<(Rect, View3D)>) = if n == 2 {
        let mid = full.center().x;
        (Rect::from_min_max(pos2(mid + gap / 2.0, full.min.y), full.max), vec![(Rect::from_min_max(full.min, pos2(mid - gap / 2.0, full.max.y)), View3D::Top)])
    } else {
        let c = full.center();
        let q = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        (
            q(c.x + gap / 2.0, c.y + gap / 2.0, full.max.x, full.max.y),
            vec![
                (q(full.min.x, full.min.y, c.x - gap / 2.0, c.y - gap / 2.0), View3D::Top),
                (q(c.x + gap / 2.0, full.min.y, full.max.x, c.y - gap / 2.0), View3D::Front),
                (q(full.min.x, c.y + gap / 2.0, c.x - gap / 2.0, full.max.y), View3D::Right),
            ],
        )
    };
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, Color32::from_black_alpha(200));
    let t = app.session.time();
    let (cw, ch) = (comp.width as f32, comp.height as f32);
    let views = app.session.state.views3d.get(&cid).cloned().unwrap_or_default();
    let share = app.session.state.share_view_options;
    for (i, (r, v)) in aux.into_iter().enumerate() {
        p.rect_filled(r, 0.0, bg);
        let fit = ((r.width() - 20.0) / cw).min((r.height() - 20.0) / ch).max(0.01);
        let cr = Rect::from_center_size(r.center(), vec2(cw * fit, ch * fit));
        let scale = (fit * ctx.pixels_per_point()).min(1.0) as f64;
        let cam = views.cam(v, comp.width as f64, comp.height as f64).state();
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (app.session.revision, cid.0, t.0, i, scale.to_bits(), format!("{cam:?}")).hash(&mut h);
            h.finish()
        };
        let id = egui::Id::new(("viewer-aux", i));
        let cached: Option<(u64, egui::TextureHandle)> = ctx.data(|d| d.get_temp(id));
        let tex = match cached {
            Some((k, tex)) if k == key => tex,
            _ => {
                let opts = effectcraft_engine::render::RenderOpts { scale, view: comp.has_3d().then_some(cam), draft: true, ..Default::default() };
                let img = app.session.render(cid, t, opts);
                let tex = ctx.load_texture(format!("viewer-aux-{i}"), crate::frames::to_color_image(&img), egui::TextureOptions::LINEAR);
                ctx.data_mut(|d| d.insert_temp(id, (key, tex.clone())));
                tex
            }
        };
        let b = comp.background;
        p.rect_filled(cr, 0.0, Color32::from_rgb((b[0] * 255.0) as u8, (b[1] * 255.0) as u8, (b[2] * 255.0) as u8));
        p.image(tex.id(), cr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        p.rect_stroke(cr, 0.0, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
        if share && app.ui.viewer.safe_margins {
            for k in [0.9, 0.8] {
                p.rect_stroke(Rect::from_center_size(cr.center(), cr.size() * k), 0.0, Stroke::new(1.0, Color32::from_white_alpha(120)), StrokeKind::Middle);
            }
        }
        let label = if comp.has_3d() { v.label() } else { "Active Camera" };
        p.text(r.left_bottom() + vec2(8.0, -8.0), Align2::LEFT_BOTTOM, label, Tokens::ui(11.0), Color32::from_white_alpha(200));
        app.auto.add(&format!("viewer.view.{}", v.id()), r, label);
    }
    main
}

pub(crate) fn checker(p: &egui::Painter, r: Rect) {
    let s = 10.0;
    p.rect_filled(r, 0.0, Color32::from_gray(0xcc));
    let mut y = r.min.y;
    let mut row = 0;
    while y < r.max.y {
        let mut x = r.min.x + if row % 2 == 0 { 0.0 } else { s };
        while x < r.max.x {
            p.rect_filled(Rect::from_min_max(pos2(x, y), pos2((x + s).min(r.max.x), (y + s).min(r.max.y))), 0.0, Color32::from_gray(0x99));
            x += 2.0 * s;
        }
        y += s;
        row += 1;
    }
}

pub fn show(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    let Some(cid) = app.session.active_comp_id() else {
        empty_state(app, ui, rect);
        return;
    };
    let comp = app.session.project.comp(cid).cloned().unwrap_or_else(|| effectcraft_engine::project::Comp::new(1920, 1080, Default::default(), Tick::ZERO));
    let comp_name = app.session.project.item(cid).map(|i| i.name.clone()).unwrap_or_default();
    let p = ui.painter().clone();
    VIEW_CAM.with(|c| c.set(app.session.view_camera(cid)));

    // Navigator bar (open comps as breadcrumbs).
    let nav = Rect::from_min_size(rect.min, vec2(rect.width(), 24.0));
    p.rect_filled(nav, 0.0, t.panel_bg);
    let mut x = nav.min.x + 10.0;
    let open = app.session.state.open_comps.clone();
    for oc in open {
        let name = app.session.project.item(oc).map(|i| i.name.clone()).unwrap_or_default();
        let g = p.layout_no_wrap(name.clone(), Tokens::ui(11.5), t.text);
        let r = Rect::from_min_size(pos2(x, nav.min.y + 3.0), vec2(g.size().x + 16.0, 18.0));
        let resp = ui.interact(r, egui::Id::new(("nav", oc.0)), Sense::click());
        let active = oc == cid;
        p.rect_filled(
            r,
            9.0,
            if active {
                Color32::from_rgb(0x34, 0x3c, 0x4e)
            } else if resp.hovered() {
                t.hover
            } else {
                Color32::TRANSPARENT
            },
        );
        p.galley_with_override_text_color(pos2(r.min.x + 8.0, r.center().y - g.size().y / 2.0), g, if active { t.tab_text_active } else { t.text_dim });
        app.auto.add(&format!("viewer.nav.{}", oc.0), r, &name);
        if resp.clicked() {
            app.session.open_comp(oc);
        }
        x = r.max.x + 4.0;
    }

    // Bottom control bar.
    let bar_h = 30.0;
    let bar = Rect::from_min_max(pos2(rect.min.x, rect.max.y - bar_h), rect.max);
    // The expression error bar sits above the control bar while expressions fail.
    let err_h = super::expr_bar::height(app, &ctx, cid);
    let err_bar = Rect::from_min_max(pos2(rect.min.x, bar.min.y - err_h), pos2(rect.max.x, bar.min.y));
    let full = Rect::from_min_max(pos2(rect.min.x, nav.max.y), pos2(rect.max.x, err_bar.min.y));
    let pasteboard = app.ui.viewer.pasteboard.map(|[r, g, b]| Color32::from_rgb(r, g, b)).unwrap_or(t.pasteboard);
    // View ▸ Switch View Layout: extra views (Top / Front / Right) beside the main view, which
    // keeps the overlays and the interaction.
    let area = aux_views(app, ui, &comp, cid, full, pasteboard);
    p.rect_filled(area, 0.0, pasteboard);
    // View ▸ Show Rulers takes a strip on the top and left.
    let outer = area;
    let area = vt::inset(app, area);

    let (cw, ch) = (comp.width as f32, comp.height as f32);
    let fit = ((area.width() - 40.0) / cw).min((area.height() - 40.0) / ch).max(0.01);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("viewer-fit"), fit));
    let zoom = app.ui.viewer.zoom.unwrap_or(fit);
    let center = area.center() + vec2(app.ui.viewer.pan[0], app.ui.viewer.pan[1]);
    let origin = center - vec2(cw * zoom / 2.0, ch * zoom / 2.0);
    let map = ViewerMap { origin, zoom, comp: [cw, ch], area };
    ctx.data_mut(|d| d.insert_temp(map_id(), map));
    let comp_rect = Rect::from_min_size(origin, vec2(cw * zoom, ch * zoom));
    let painter = p.with_clip_rect(area);

    // Frame.
    let ppp = ctx.pixels_per_point();
    let scale = app.viewer_scale(zoom, ppp);
    if !app.playback.playing {
        // Queue the frame on screen before any prefetch.
        app.request_frame_urgent(cid, comp.frame_rate.frame_at(app.session.time()), scale);
    }
    crate::tick_playback(app, &ctx, scale);
    let time = app.session.time();
    let frame = comp.frame_rate.frame_at(time);
    let key = app.frame_key(cid, frame, scale);
    app.request_frame_urgent(cid, frame, scale);
    if let Some(img) = app.frames.get(&key) {
        let stale = app.viewer_shown.as_ref().is_none_or(|(_, k)| *k != key);
        if stale {
            show_frame(app, &ctx, key, img);
        }
    }
    if app.ui.viewer.transparency_grid {
        checker(&painter, comp_rect);
    } else {
        let bg = comp.background;
        painter.rect_filled(comp_rect, 0.0, Color32::from_rgb((bg[0] * 255.0) as u8, (bg[1] * 255.0) as u8, (bg[2] * 255.0) as u8));
    }
    let snap_project = app.session.project.clone();
    let snap_expr = app.session.expr.clone();
    let ectx = EvalCtx { project: &snap_project, comp_id: cid, comp: &comp, time, expr: snap_expr.as_deref(), footage: None };
    // The frame (or snapshot) through Show Channel and exposure; ROI frames cover the region.
    vt::draw_frame(app, &ctx, &painter, comp_rect, cid, &ectx);
    painter.rect_stroke(comp_rect, 0.0, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
    app.auto.add("viewer.comp", comp_rect, &comp_name);
    app.auto.add("viewer.area", area, "Composition viewer");

    // Guides.
    if app.ui.viewer.safe_margins {
        let g = &app.session.prefs.grids;
        let (act, title) = (1.0 - g.action_safe as f32 / 100.0, 1.0 - g.title_safe as f32 / 100.0);
        for (k, a) in [(act, 120u8), (title, 160)] {
            let r = Rect::from_center_size(comp_rect.center(), comp_rect.size() * k);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_white_alpha(a)), StrokeKind::Middle);
        }
        let c = comp_rect.center();
        painter.line_segment([c - vec2(10.0, 0.0), c + vec2(10.0, 0.0)], Stroke::new(1.0, Color32::from_white_alpha(140)));
        painter.line_segment([c - vec2(0.0, 10.0), c + vec2(0.0, 10.0)], Stroke::new(1.0, Color32::from_white_alpha(140)));
    }
    if app.ui.viewer.grid {
        // Settings ▸ Grids & Guides: gridline spacing, subdivisions and colour.
        let g = &app.session.prefs.grids;
        let [r, gg, b] = hex_rgb(&g.grid_color).unwrap_or([120, 160, 255]);
        let major = Stroke::new(1.0, Color32::from_rgba_unmultiplied(r, gg, b, 90));
        let minor = Stroke::new(1.0, Color32::from_rgba_unmultiplied(r, gg, b, 35));
        let step = (g.grid_spacing as f32 * zoom).max(2.0);
        let sub = g.grid_subdivisions.max(1) as f32;
        let sstep = step / sub;
        let draw_minor = sstep >= 4.0;
        let mut i = 0u32;
        let mut gx = comp_rect.min.x;
        while gx <= comp_rect.max.x {
            let is_major = i.is_multiple_of(g.grid_subdivisions.max(1));
            if is_major || draw_minor {
                painter.line_segment([pos2(gx, comp_rect.min.y), pos2(gx, comp_rect.max.y)], if is_major { major } else { minor });
            }
            gx += sstep;
            i += 1;
        }
        let mut i = 0u32;
        let mut gy = comp_rect.min.y;
        while gy <= comp_rect.max.y {
            let is_major = i.is_multiple_of(g.grid_subdivisions.max(1));
            if is_major || draw_minor {
                painter.line_segment([pos2(comp_rect.min.x, gy), pos2(comp_rect.max.x, gy)], if is_major { major } else { minor });
            }
            gy += sstep;
            i += 1;
        }
    }

    vt::proportional_grid(app, &painter, comp_rect);

    // Comp guides (View ▸ Show Guides) and the region of interest.
    if app.ui.viewer.guides {
        let [r, gg, b] = hex_rgb(&app.session.prefs.grids.guide_color).unwrap_or([0x3c, 0xc8, 0xf0]);
        let stroke = Stroke::new(1.0, Color32::from_rgb(r, gg, b));
        for g in &comp.guides {
            if g.vertical {
                let x = comp_rect.min.x + g.position as f32 * zoom;
                painter.line_segment([pos2(x, area.min.y), pos2(x, area.max.y)], stroke);
            } else {
                let y = comp_rect.min.y + g.position as f32 * zoom;
                painter.line_segment([pos2(area.min.x, y), pos2(area.max.x, y)], stroke);
            }
        }
    }
    if let Some([rx, ry, rw, rh]) = app.session.state.region_of_interest {
        let r = Rect::from_min_size(comp_rect.min + vec2(rx as f32 * zoom, ry as f32 * zoom), vec2(rw as f32 * zoom, rh as f32 * zoom));
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Middle);
        for c in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()] {
            painter.rect_filled(Rect::from_center_size(c, vec2(5.0, 5.0)), 0.0, Color32::WHITE);
        }
        app.auto.add("viewer.regionOfInterest", r, "Region of interest");
    }

    // Overlays + interaction.
    let selected = app.session.state.selected_layers.clone();
    let mut handle_hits: Vec<(LayerId, usize, Pos2)> = Vec::new();
    let mut vertex_hits: Vec<VertexHit> = Vec::new();
    let mut key_hits: Vec<ov::KeyHit> = Vec::new();
    let mut paths: Vec<ov::PathInfo> = Vec::new();
    let sel_vertices = app.session.state.selected_vertices.clone();
    // Pen path in progress ends when the tool changes or its mask is gone; Esc / Enter also end
    // free transform.
    let finish = ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter)) && !ctx.egui_wants_keyboard_input();
    if app.ui.tool != Tool::Pen || finish {
        ui.data_mut(|d| d.remove::<(LayerId, u64)>(pen_id()));
    }
    if finish || app.ui.tool != Tool::Selection {
        ov::end_free_transform(&ctx);
    }
    if app.ui.viewer.show_layer_controls {
        // Settings ▸ General ▸ Path Point and Handle Size; Appearance ▸ Use Label Color for
        // Layer Handles and Paths.
        let hs = app.session.prefs.general.path_point_size as f32 + 2.0;
        let label_handles = app.session.prefs.appearance.use_label_color_for_handles;
        for l in comp.layers.iter().filter(|l| selected.contains(&l.id) && l.is_active_at(time)) {
            let col = if label_handles { t.label(l.label) } else { t.accent };
            // Motion path of animated position (keyframes and spatial tangents are draggable).
            ov::motion_path(app, &painter, &map, &ectx, l, col, &mut key_hits);
            // Masks and shape paths.
            if app.ui.viewer.show_masks {
                let pen = ui.data(|d| d.get_temp::<(LayerId, u64)>(pen_id()));
                let hover = ui.input(|i| i.pointer.hover_pos());
                ov::draw_paths(app, &painter, &map, &ectx, l, col, &sel_vertices, pen, hover, &mut vertex_hits, &mut paths);
            }
            let Some((m, q, _)) = layer_quad(&ectx, l) else {
                // Cameras, lights, empty layers: just the anchor.
                if let Some(tr) = l.transform() {
                    let pos = ectx.v3(l, tr, "position", [0.0; 3]);
                    let s = map.to_screen([pos[0], pos[1]]);
                    painter.circle_stroke(s, 6.0, Stroke::new(1.0, col));
                }
                continue;
            };
            let sq: Vec<Pos2> = q.iter().map(|p| map.to_screen(*p)).collect();
            painter.add(egui::Shape::closed_line(sq.clone(), Stroke::new(1.0, col)));
            for i in 0..8 {
                let hp = if i < 4 { sq[i] } else { sq[i - 4] + (sq[(i - 3) % 4] - sq[i - 4]) * 0.5 };
                let hr = Rect::from_center_size(hp, vec2(hs, hs));
                painter.rect_filled(hr, 0.0, col);
                painter.rect_stroke(hr, 0.0, Stroke::new(1.0, Color32::from_black_alpha(120)), StrokeKind::Outside);
                handle_hits.push((l.id, i, hp));
                app.auto.add(&format!("viewer.handle.{}.{i}", l.id.0), hr, "handle");
            }
            // Anchor point.
            if let Some(tr) = l.transform() {
                let a = ectx.v3(l, tr, "anchor", [0.0; 3]);
                let c = m.apply(gv2(a[0], a[1]));
                let s = map.to_screen([c.x, c.y]);
                painter.circle_stroke(s, 5.0, Stroke::new(1.0, Color32::WHITE));
                painter.line_segment([s - vec2(9.0, 0.0), s + vec2(9.0, 0.0)], Stroke::new(1.0, Color32::WHITE));
                painter.line_segment([s - vec2(0.0, 9.0), s + vec2(0.0, 9.0)], Stroke::new(1.0, Color32::WHITE));
                app.auto.add(&format!("viewer.anchor.{}", l.id.0), Rect::from_center_size(s, vec2(12.0, 12.0)), "anchor point");
                // 3D axis gizmo.
                if l.is_3d() {
                    let w = ectx.world_matrix(l);
                    let pc = cam_state(&ectx).projection(cw as f64, ch as f64) * w;
                    let a3 = effectcraft_engine::geom::vec3(a[0], a[1], a[2]);
                    let o = pc.apply(a3);
                    let len = 60.0 / zoom as f64;
                    for (axis, col) in [
                        (effectcraft_engine::geom::vec3(len, 0.0, 0.0), Color32::from_rgb(0xe0, 0x50, 0x50)),
                        (effectcraft_engine::geom::vec3(0.0, len, 0.0), Color32::from_rgb(0x60, 0xd0, 0x60)),
                        (effectcraft_engine::geom::vec3(0.0, 0.0, len), Color32::from_rgb(0x50, 0x8c, 0xf0)),
                    ] {
                        let e = pc.apply(a3 + axis);
                        painter.arrow(map.to_screen([o.x, o.y]), map.to_screen([e.x, e.y]) - map.to_screen([o.x, o.y]), Stroke::new(2.0, col));
                    }
                }
            }
        }
    }

    // Track points of the current track (Tracker panel) on its layer, while that layer is selected.
    let mut track_hits = vec![];
    let track_cur = super::tracker::viewer_track(app);
    if app.ui.viewer.show_layer_controls
        && let Some((tl, tu)) = track_cur
        && selected.contains(&tl)
        && let Some(layer) = comp.layer(tl)
        && layer.is_active_at(time)
    {
        let (m, _) = l2c(&ectx, layer);
        track_hits = super::tracker::draw_overlay(app, &painter, &map, &m, layer, tu, time);
    }

    // Warp Stabilizer banner over the layer being analysed ("Analyzing in background (step 1
    // of 2)…" / "Stabilizing…").
    if let Some((wc, wl, _)) = app.session.warp_target()
        && wc == cid
        && let Some(pr) = app.session.warp_progress()
        && let Some(layer) = comp.layer(wl)
        && layer.is_active_at(time)
    {
        let (m, _) = l2c(&ectx, layer);
        let (w, h) = effectcraft_engine::render::source_size(&app.session.project, layer);
        let c = m.apply(effectcraft_engine::geom::vec2(w as f64 / 2.0, h as f64 / 2.0));
        let at = map.to_screen([c.x, c.y]);
        let text = pr.banner();
        let galley = painter.layout_no_wrap(text.clone(), Tokens::medium(13.0), Color32::WHITE);
        let r = Rect::from_center_size(at, galley.size() + vec2(28.0, 14.0)).intersect(map.area);
        painter.rect_filled(r, 3.0, Color32::from_rgba_unmultiplied(0x1d, 0x4f, 0x9c, 0xe0));
        painter.galley(pos2(r.center().x - galley.size().x / 2.0, r.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
        app.auto.add("viewer.warpBanner", r, &text);
    }

    // Free transform box of a path (double-click a path with the Selection tool).
    let ft = ov::draw_free_transform(app, &ctx, &painter, &map, &paths);

    // Cameras and lights (wireframes) and the 3D view name.
    if comp.has_3d() {
        draw_rigs(app, &painter, &map, &ectx, &selected);
        let view = app.session.state.views3d.get(&cid).map(|v| v.current).unwrap_or_default();
        let label = match (view, comp.active_camera(time)) {
            (View3D::ActiveCamera, Some(c)) => format!("Active Camera ({})", c.name),
            (v, _) => v.label().to_string(),
        };
        painter.text(area.left_top() + vec2(12.0, 10.0), Align2::LEFT_TOP, label, Tokens::ui(12.0), t.text_dim);
    }

    // Puppet pins and meshes of the selected layers (Puppet tools).
    let mut pin_hits: Vec<super::puppet_tool::PinHit> = vec![];
    if app.ui.tool.puppet_kind().is_some() {
        let sel_props: Vec<u64> = app.session.state.selected_props.iter().map(|(_, u)| *u).collect();
        for lid in &selected {
            if let Some(l) = comp.layer(*lid)
                && let Some(ov) = super::puppet_tool::overlay(app, &ctx, cid, l, time)
            {
                pin_hits.extend(super::puppet_tool::draw(&painter, &map, &ectx, l, &ov, app.session.state.puppet.show_mesh, &sel_props));
            }
        }
        for h in &pin_hits {
            app.auto.add(&format!("viewer.puppetPin.{}", h.pin), Rect::from_center_size(h.pos, vec2(10.0, 10.0)), "Puppet pin");
        }
    }

    // Interaction.
    let resp = ui.interact(area, egui::Id::new("viewer-interact"), Sense::click_and_drag());
    let gid = egui::Id::new("viewer-gesture");
    if let Some(hp) = resp.hover_pos() {
        app.pointer_comp = Some({
            let c = map.to_comp(hp);
            [c[0] as f32, c[1] as f32]
        });
        let cursor = match app.ui.tool {
            Tool::Hand => egui::CursorIcon::Grab,
            Tool::Zoom => egui::CursorIcon::ZoomIn,
            Tool::Type | Tool::TypeVertical => egui::CursorIcon::Text,
            Tool::Orbit => egui::CursorIcon::Alias,
            Tool::PanCamera => egui::CursorIcon::Move,
            Tool::Dolly => egui::CursorIcon::ResizeVertical,
            t if t.is_shape() || t.is_pen() => egui::CursorIcon::Crosshair,
            _ if app.ui.viewer.roi_draw => egui::CursorIcon::Crosshair,
            t if t.puppet_kind().is_some() => {
                if pin_hits.iter().any(|h| h.pos.distance(hp) < 8.0) {
                    egui::CursorIcon::Move
                } else {
                    egui::CursorIcon::Crosshair
                }
            }
            _ => {
                if super::tracker::hit_at(&track_hits, hp).is_some() || key_hits.iter().any(|h| h.pos.distance(hp) < 6.0) {
                    egui::CursorIcon::Move
                } else if let Some((_, vertical)) = vt::guide_at(app, &comp, &map, hp) {
                    if vertical { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical }
                } else if handle_hits.iter().any(|(_, _, h)| h.distance(hp) < HANDLE) {
                    egui::CursorIcon::ResizeNwSe
                } else {
                    egui::CursorIcon::Default
                }
            }
        };
        ctx.set_cursor_icon(cursor);
        // Scroll wheel zooms around the pointer.
        let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 300.0);
        if scroll.abs() > 0.1 && area.contains(hp) {
            let k = (scroll / 300.0).exp();
            let nz = (zoom * k).clamp(0.01, 32.0);
            let before = map.to_comp(hp);
            let new_origin = hp - vec2(before[0] as f32 * nz, before[1] as f32 * nz);
            let new_center = new_origin + vec2(cw * nz / 2.0, ch * nz / 2.0);
            let pan = new_center - area.center();
            app.ui.viewer.zoom = Some(nz);
            app.ui.viewer.pan = [pan.x, pan.y];
        }
    } else {
        app.pointer_comp = None;
    }
    app.ui.viewer.interacting = resp.dragged();
    let mods = ui.input(|i| i.modifiers);
    let space_pan = ui.input(|i| i.key_down(egui::Key::Space)) && !ctx.egui_wants_keyboard_input();
    // Pen tool: each press places a vertex (dragging pulls Bezier tangents); pressing on the
    // first vertex closes the path.
    if app.ui.tool == Tool::Pen
        && !space_pan
        && resp.hovered()
        && ui.input(|i| i.pointer.primary_pressed())
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
    {
        pen_press(app, ui, &ectx, &map, &paths, pos, mods);
    }
    // A pen click (press + release without a drag) ends its tangent gesture on release.
    if ui.input(|i| i.pointer.primary_released())
        && matches!(ui.data(|d| d.get_temp::<Gesture>(egui::Id::new("viewer-gesture"))), Some(Gesture::Pen { .. }))
        && !resp.drag_stopped()
    {
        ui.data_mut(|d| d.remove::<Gesture>(egui::Id::new("viewer-gesture")));
    }
    let vertex_at = |pos: Pos2, tangents: bool| -> Option<VertexHit> {
        vertex_hits.iter().rev().filter(|h| h.tangent.is_some() == tangents).find(|h| h.pos.distance(pos) < 6.0).copied()
    };
    // On-canvas text editing (Type tool, double-click a text layer) owns its clicks and drags.
    let text_owns = super::viewer_text::hook(app, ui, &painter, &map, &ectx, &resp, space_pan);
    if resp.drag_started()
        && !text_owns
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let cpt = map.to_comp(pos);
        let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
        let tool = if space_pan || resp.dragged_by(egui::PointerButton::Middle) { Tool::Hand } else { app.ui.tool };
        let g = match tool {
            Tool::Hand => Some(Gesture::Pan { start_pan: app.ui.viewer.pan }),
            // Region of Interest armed: the drag draws it.
            _ if app.ui.viewer.roi_draw => Some(Gesture::Overlay(ov::Drag::Roi { start: map.to_comp(press) })),
            Tool::Selection if ft.as_ref().and_then(|f| ov::begin_ft_drag(f, press, &map)).is_some() => {
                ft.as_ref().and_then(|f| ov::begin_ft_drag(f, press, &map)).map(Gesture::Overlay)
            }
            Tool::Selection if key_hits.iter().any(|h| h.pos.distance(press) < 6.0) => {
                ov::begin_key_drag(app, &key_hits, press, map.to_comp(press), mods.shift).map(Gesture::Overlay)
            }
            Tool::Selection if vt::guide_at(app, &comp, &map, press).is_some() => {
                vt::guide_at(app, &comp, &map, press).map(|(index, vertical)| Gesture::Overlay(ov::Drag::Guide { index, vertical }))
            }
            // Alt-drag a vertex with the Selection tool (or drag one with Convert Vertex): pull
            // new tangents out of it.
            Tool::Selection | Tool::PenConvert if (mods.alt || tool == Tool::PenConvert) && vertex_at(press, false).is_some() => {
                vertex_at(press, false).map(|h| {
                    let inv = h.l2c.inverse().unwrap_or(Mat3::IDENTITY);
                    Gesture::Pen { layer: h.layer, mask: h.mask, index: h.index, vertex: h.vertex, inv, press }
                })
            }
            Tool::MaskFeather => ov::begin_feather(app, &ectx, &paths, &map, press).map(Gesture::Overlay),
            Tool::Rotate => pick(app, &ectx, cpt, mods.shift).map(|l| {
                let layer = comp.layer(l).cloned();
                let center = layer
                    .as_ref()
                    .and_then(|l| l.transform().map(|tr| ectx.v3(l, tr, "position", [0.0; 3])))
                    .map(|p| map.to_screen([p[0], p[1]]))
                    .unwrap_or(pos);
                let rot = layer.as_ref().and_then(|l| l.transform().map(|tr| ectx.f(l, tr, "rotation", 0.0))).unwrap_or(0.0);
                Gesture::Rotate { layer: l, center, start_angle: ((pos.y - center.y) as f64).atan2((pos.x - center.x) as f64).to_degrees(), start_rot: rot }
            }),
            Tool::PanBehind => pick(app, &ectx, cpt, false).and_then(|l| {
                let layer = comp.layer(l)?;
                let tr = layer.transform()?;
                let (l2c, _) = l2c(&ectx, layer);
                let l2p = ectx.local_matrix(layer);
                let l2p = Mat3([[l2p.0[0][0], l2p.0[0][1], 0.0], [l2p.0[1][0], l2p.0[1][1], 0.0], [0.0, 0.0, 1.0]]);
                Some(Gesture::Anchor {
                    layer: l,
                    start_anchor: ectx.v3(layer, tr, "anchor", [0.0; 3]),
                    start_pos: ectx.v3(layer, tr, "position", [0.0; 3]),
                    start: cpt,
                    inv: l2c.inverse().unwrap_or(Mat3::IDENTITY),
                    l2p,
                })
            }),
            t if t.is_shape() => Some(Gesture::Create { tool: t, start: cpt }),
            t if t.puppet_kind().is_some() => pin_hits.iter().rev().find(|h| h.pos.distance(press) < 8.0).and_then(|h| {
                let l = comp.layer(h.layer)?;
                Some(Gesture::PuppetPin { layer: h.layer, pin: h.pin, inv: l2c(&ectx, l).0.inverse()? })
            }),
            Tool::Orbit | Tool::PanCamera | Tool::Dolly => Some(Gesture::Camera { tool }),
            Tool::Selection if super::tracker::hit_at(&track_hits, press).is_some() => {
                let hit = super::tracker::hit_at(&track_hits, press);
                track_cur.zip(hit).and_then(|((tl, tu), hit)| {
                    let layer = comp.layer(tl)?;
                    let (m, _) = l2c(&ectx, layer);
                    super::tracker::begin_drag(layer, tu, time, &m, hit, map.to_comp(press)).map(|drag| Gesture::Track { layer: tl, tracker: tu, drag })
                })
            }
            Tool::Selection if vertex_at(press, true).is_some() => vertex_at(press, true).map(|h| Gesture::Tangent {
                layer: h.layer,
                mask: h.mask,
                index: h.index,
                out: h.tangent == Some(true),
                vertex: h.vertex,
                inv: h.l2c.inverse().unwrap_or(Mat3::IDENTITY),
            }),
            Tool::Selection if vertex_at(press, false).is_some() => vertex_at(press, false).map(|h| {
                let me = json!({"layer": h.layer.0, "mask": h.mask, "index": h.index});
                let selected = app.session.state.selected_vertices.iter().any(|v| v.layer == h.layer && v.mask == h.mask && v.index == h.index);
                if !selected {
                    let _ = app.session.execute("mask.selectVertices", json!({"vertices": [me], "add": mods.shift}));
                }
                let src = h.l2c.apply(gv2(h.vertex[0], h.vertex[1]));
                Gesture::Vertices {
                    start: map.to_comp(press),
                    src: [src.x, src.y],
                    applied: [0.0; 2],
                    inv: h.l2c.inverse().unwrap_or(Mat3::IDENTITY),
                    layer: h.layer,
                }
            }),
            Tool::Selection => {
                if let Some((lid, hi, _)) = handle_hits.iter().find(|(_, _, h)| h.distance(pos) < HANDLE + 2.0).cloned() {
                    let layer = comp.layer(lid).cloned();
                    layer.and_then(|layer| {
                        let tr = layer.transform()?;
                        let (l2c, _) = l2c(&ectx, &layer);
                        let inv = l2c.inverse()?;
                        let a = ectx.v3(&layer, tr, "anchor", [0.0; 3]);
                        let lp = inv.apply(gv2(cpt[0], cpt[1]));
                        let _ = hi;
                        Some(Gesture::Scale { layer: lid, start_scale: ectx.v3(&layer, tr, "scale", [100.0; 3]), start_local: [lp.x - a[0], lp.y - a[1]], inv })
                    })
                } else if let Some(l) = pick(app, &ectx, cpt, mods.shift) {
                    let layers: Vec<(LayerId, [f64; 3], [f64; 3], [f64; 3])> = app
                        .session
                        .state
                        .selected_layers
                        .iter()
                        .filter_map(|id| comp.layer(*id))
                        .filter(|l| !l.switches.locked)
                        .filter_map(|l| {
                            let p0 = ectx.v3(l, l.transform()?, "position", [0.0; 3]);
                            let (ax, ay) = drag_axes(&ectx, l);
                            Some((l.id, p0, ax, ay))
                        })
                        .collect();
                    // Snap the grabbed layer's feature nearest the pointer (AE's snap handle).
                    let snap_src = comp.layer(l).map(|layer| effectcraft_engine::viewer::layer_features(&ectx, layer)).and_then(|f| {
                        let c = map.to_comp(press);
                        f.into_iter().min_by(|a, b| {
                            let da = (a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2);
                            let db = (b[0] - c[0]).powi(2) + (b[1] - c[1]).powi(2);
                            da.total_cmp(&db)
                        })
                    });
                    (!layers.is_empty()).then_some(Gesture::Move { layers, start: map.to_comp(press), snap_src })
                } else {
                    // With path vertices on screen the marquee selects vertices (layers stay
                    // selected); otherwise it starts from a clean selection.
                    if !vertex_hits.iter().any(|h| h.tangent.is_none()) {
                        let _ = app.session.execute("edit.deselectAll", json!({}));
                    }
                    Some(Gesture::Marquee { start: pos })
                }
            }
            _ => None,
        };
        if let Some(g) = g {
            ui.data_mut(|d| d.insert_temp(gid, g));
        }
    }
    let gesture: Option<Gesture> = ui.data(|d| d.get_temp(gid));
    if let (Some(g), Some(pos)) = (gesture.clone(), resp.interact_pointer_pos()) {
        let cpt = map.to_comp(pos);
        let merge = format!("viewer-drag-{}", ui.data(|d| d.get_temp::<u64>(egui::Id::new("viewer-drag-n")).unwrap_or(0)));
        match g {
            Gesture::Pan { start_pan } => {
                let d = resp.total_drag_delta().unwrap_or_default();
                app.ui.viewer.pan = [start_pan[0] + d.x, start_pan[1] + d.y];
            }
            Gesture::Move { layers, start, snap_src } => {
                let mut d = [cpt[0] - start[0], cpt[1] - start[1]];
                if mods.shift {
                    if d[0].abs() > d[1].abs() { d[1] = 0.0 } else { d[0] = 0.0 }
                }
                if let Some(src) = snap_src {
                    let ids: Vec<LayerId> = layers.iter().map(|x| x.0).collect();
                    let c = vt::snap(app, &ctx, &ectx, &map, &ids, &[[src[0] + d[0], src[1] + d[1]]], mods);
                    d = [d[0] + c[0], d[1] + c[1]];
                }
                for (lid, p0, ax, ay) in layers {
                    let v = json!([p0[0] + ax[0] * d[0] + ay[0] * d[1], p0[1] + ax[1] * d[0] + ay[1] * d[1], p0[2] + ax[2] * d[0] + ay[2] * d[1]]);
                    let _ = app.session.execute("prop.set", json!({"layer": lid.0, "path": "transform/position", "value": v, "merge": merge}));
                }
            }
            Gesture::Scale { layer, start_scale, start_local, inv, .. } => {
                let lp = inv.apply(gv2(cpt[0], cpt[1]));
                // Local point relative to the anchor in *unscaled* layer space.
                let a = comp.layer(layer).and_then(|l| l.transform().map(|tr| ectx.v3(l, tr, "anchor", [0.0; 3]))).unwrap_or([0.0; 3]);
                let cur = [lp.x - a[0], lp.y - a[1]];
                let fx = if start_local[0].abs() > 1e-6 { cur[0] / start_local[0] } else { 1.0 };
                let fy = if start_local[1].abs() > 1e-6 { cur[1] / start_local[1] } else { 1.0 };
                let (fx, fy) = if mods.shift {
                    let f = (fx + fy) / 2.0;
                    (f, f)
                } else {
                    (if start_local[0].abs() > 1e-3 { fx } else { 1.0 }, if start_local[1].abs() > 1e-3 { fy } else { 1.0 })
                };
                // The local point moved within the *current* scale; compose with the start scale.
                let cs = comp.layer(layer).and_then(|l| l.transform().map(|tr| ectx.v3(l, tr, "scale", [100.0; 3]))).unwrap_or([100.0; 3]);
                let nx = cs[0] * fx;
                let ny = cs[1] * fy;
                let _ = start_scale;
                let _ = app.session.execute("prop.set", json!({"layer": layer.0, "path": "transform/scale", "value": [nx, ny, cs[2]], "merge": merge}));
                ui.data_mut(|d| {
                    if let Some(Gesture::Scale { start_local, .. }) = d.get_temp_mut_or_default::<Option<Gesture>>(egui::Id::new("unused")).as_mut() {
                        let _ = start_local;
                    }
                });
                // Re-baseline so the next frame's ratios are relative to the new scale.
                let lp_unscaled = [cur[0] / fx.max(1e-9) * fx, cur[1] / fy.max(1e-9) * fy];
                let mut g2 = gesture.clone();
                if let Some(Gesture::Scale { start_local, inv: i2, .. }) = g2.as_mut() {
                    *start_local = lp_unscaled;
                    if let Some(l) = app.session.active_comp().and_then(|c| c.layer(layer)).cloned() {
                        let e2 = EvalCtx {
                            project: &app.session.project,
                            comp_id: cid,
                            comp: app.session.project.comp(cid).unwrap_or(&comp),
                            time,
                            expr: app.session.expr.as_deref(),
                            footage: None,
                        };
                        if let Some(m) = l2c(&e2, &l).0.inverse() {
                            *i2 = m;
                        }
                    }
                }
                if let Some(g2) = g2 {
                    ui.data_mut(|d| d.insert_temp(gid, g2));
                }
            }
            Gesture::Rotate { layer, center, start_angle, start_rot } => {
                let ang = ((pos.y - center.y) as f64).atan2((pos.x - center.x) as f64).to_degrees();
                let mut r = start_rot + (ang - start_angle);
                if mods.shift {
                    r = (r / 45.0).round() * 45.0;
                }
                let _ = app.session.execute("prop.set", json!({"layer": layer.0, "path": "transform/rotation", "value": r, "merge": merge}));
            }
            Gesture::Anchor { layer, start_anchor, start_pos, start, inv, l2p } => {
                let a = inv.apply(gv2(start[0], start[1]));
                let b = inv.apply(gv2(cpt[0], cpt[1]));
                let dl = gv2(b.x - a.x, b.y - a.y);
                let na = [start_anchor[0] + dl.x, start_anchor[1] + dl.y, start_anchor[2]];
                let dp = l2p.apply_vec(dl);
                let np = [start_pos[0] + dp.x, start_pos[1] + dp.y, start_pos[2]];
                let _ = app.session.execute("prop.set", json!({"layer": layer.0, "path": "transform/anchor", "value": na, "merge": merge}));
                let _ = app.session.execute("prop.set", json!({"layer": layer.0, "path": "transform/position", "value": np, "merge": merge}));
            }
            Gesture::Camera { tool } => {
                let d = resp.drag_delta();
                if d != egui::Vec2::ZERO {
                    let (dx, dy) = ((d.x / zoom) as f64, (d.y / zoom) as f64);
                    let (id, params) = match tool {
                        Tool::Orbit => ("camera.orbit", json!({"yaw": -d.x as f64 * 0.4, "pitch": d.y as f64 * 0.4, "merge": merge})),
                        Tool::PanCamera => ("camera.pan", json!({"dx": dx, "dy": dy, "merge": merge})),
                        _ => ("camera.dolly", json!({"amount": -dy * 2.0, "merge": merge})),
                    };
                    if let Err(e) = app.session.execute(id, params) {
                        app.ui.status = e.to_string();
                    }
                }
            }
            Gesture::Create { start, .. } => {
                let a = map.to_screen(start);
                let r = Rect::from_two_pos(a, pos);
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, t.accent), StrokeKind::Middle);
            }
            Gesture::Vertices { start, src, applied, inv, layer } => {
                // The grabbed vertex snaps (to other layers' vertices and features, guides, grid).
                let want = [cpt[0] - start[0], cpt[1] - start[1]];
                let c = vt::snap(app, &ctx, &ectx, &map, &[layer], &[[src[0] + want[0], src[1] + want[1]]], mods);
                let total = [want[0] + c[0], want[1] + c[1]];
                let d = inv.apply_vec(gv2(total[0] - applied[0], total[1] - applied[1]));
                if d.x != 0.0 || d.y != 0.0 {
                    let _ = app.session.execute("mask.moveVertices", json!({"delta": [d.x, d.y], "merge": merge}));
                }
                ui.data_mut(|dd| dd.insert_temp(gid, Gesture::Vertices { start, src, applied: total, inv, layer }));
            }
            Gesture::Overlay(d) => match d {
                ov::Drag::Guide { index, vertical } => {
                    let pos_v = if vertical { cpt[0] } else { cpt[1] }.round();
                    let _ = app.session.execute("view.moveGuide", json!({"index": index, "position": pos_v, "merge": merge}));
                }
                ov::Drag::Roi { start } => {
                    let r = Rect::from_two_pos(map.to_screen(start), pos);
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Middle);
                }
                d => {
                    let mut snapper = |a: &EffectcraftApp, p: [f64; 2], l: LayerId| vt::snap(a, &ctx, &ectx, &map, &[l], &[p], mods);
                    if let Some(nd) = ov::update(app, &d, cpt, pos, mods, &merge, zoom, &mut snapper) {
                        ui.data_mut(|dd| dd.insert_temp(gid, Gesture::Overlay(nd)));
                    }
                }
            },
            Gesture::Tangent { layer, mask, index, out, vertex, inv } => {
                let lp = inv.apply(gv2(cpt[0], cpt[1]));
                let t = [lp.x - vertex[0], lp.y - vertex[1]];
                let mut params = json!({"layer": layer.0, "mask": mask, "index": index, "merge": merge});
                params[if out { "out" } else { "in" }] = json!(t);
                if !mods.alt {
                    params[if out { "in" } else { "out" }] = json!([-t[0], -t[1]]);
                }
                let _ = app.session.execute("mask.setVertex", params);
            }
            Gesture::PuppetPin { layer, pin, inv } => {
                let lp = inv.apply(gv2(cpt[0], cpt[1]));
                if let Err(e) = app.session.execute("puppet.movePin", json!({"layer": layer.0, "pin": pin, "position": [lp.x, lp.y], "merge": merge})) {
                    app.ui.status = e.to_string();
                }
            }
            Gesture::Pen { layer, mask, index, vertex, inv, press } => {
                if pos.distance(press) > 3.0 {
                    let lp = inv.apply(gv2(cpt[0], cpt[1]));
                    let t = [lp.x - vertex[0], lp.y - vertex[1]];
                    let _ = app
                        .session
                        .execute("mask.setVertex", json!({"layer": layer.0, "mask": mask, "index": index, "out": t, "in": [-t[0], -t[1]], "merge": merge}));
                }
            }
            Gesture::Track { layer, tracker, drag } => {
                let mut q = super::tracker::drag_params(&drag, cpt);
                q["layer"] = json!(layer.0);
                q["tracker"] = json!(tracker);
                q["merge"] = json!(merge);
                let _ = app.session.execute("track.setPoint", q);
            }
            Gesture::Marquee { start } => {
                let r = Rect::from_two_pos(start, pos);
                painter.rect_filled(r, 0.0, t.accent.gamma_multiply(0.12));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, t.accent), StrokeKind::Middle);
            }
        }
    }
    if resp.drag_stopped() {
        if let (Some(g), Some(pos)) = (gesture, resp.interact_pointer_pos()) {
            match g {
                Gesture::Create { tool, start } => {
                    let end = map.to_comp(pos);
                    create_shape(app, tool, start, end, mods.shift);
                }
                Gesture::Overlay(ov::Drag::Roi { start }) => {
                    let end = map.to_comp(pos);
                    let (x0, y0) = (start[0].min(end[0]).max(0.0), start[1].min(end[1]).max(0.0));
                    let (x1, y1) = (start[0].max(end[0]).min(comp.width as f64), start[1].max(end[1]).min(comp.height as f64));
                    if x1 - x0 >= 2.0 && y1 - y0 >= 2.0 {
                        let rect = [x0.round(), y0.round(), (x1 - x0).round(), (y1 - y0).round()];
                        let _ = app.session.execute("view.setRegionOfInterest", json!({"rect": rect}));
                    }
                    app.ui.viewer.roi_draw = false;
                }
                // A guide dragged off the image (back to the rulers) is removed.
                Gesture::Overlay(ov::Drag::Guide { index, .. }) if !area.contains(pos) || !comp_rect.expand(40.0).contains(pos) => {
                    let _ = app.session.execute("view.removeGuide", json!({"index": index}));
                }
                Gesture::Marquee { start } if vertex_hits.iter().any(|h| h.tangent.is_none() && Rect::from_two_pos(start, pos).contains(h.pos)) => {
                    // Marquee over path vertices selects them.
                    let r = Rect::from_two_pos(start, pos);
                    let v: Vec<serde_json::Value> = vertex_hits
                        .iter()
                        .filter(|h| h.tangent.is_none() && r.contains(h.pos))
                        .map(|h| json!({"layer": h.layer.0, "mask": h.mask, "index": h.index}))
                        .collect();
                    let _ = app.session.execute("mask.selectVertices", json!({"vertices": v, "add": mods.shift}));
                }
                Gesture::Marquee { start } => {
                    let a = map.to_comp(start);
                    let b = map.to_comp(pos);
                    let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
                    let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
                    let hits: Vec<u64> = comp
                        .layers
                        .iter()
                        .filter(|l| l.is_active_at(time) && !l.switches.locked)
                        .filter(|l| layer_quad(&ectx, l).is_some_and(|(_, q, _)| q.iter().any(|p| p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1)))
                        .map(|l| l.id.0)
                        .collect();
                    let _ = app.session.execute("layer.select", json!({"layers": hits}));
                }
                _ => {}
            }
        }
        ui.data_mut(|d| {
            d.remove::<Gesture>(gid);
            let n: u64 = d.get_temp(egui::Id::new("viewer-drag-n")).unwrap_or(0);
            d.insert_temp(egui::Id::new("viewer-drag-n"), n + 1);
        });
    }
    if resp.clicked()
        && !text_owns
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let cpt = map.to_comp(pos);
        match app.ui.tool {
            Tool::Zoom => {
                let k = if mods.alt { 0.5 } else { 2.0 };
                let nz = (zoom * k).clamp(0.01, 32.0);
                let new_origin = pos - vec2(cpt[0] as f32 * nz, cpt[1] as f32 * nz);
                let pan = new_origin + vec2(cw * nz / 2.0, ch * nz / 2.0) - area.center();
                app.ui.viewer.zoom = Some(nz);
                app.ui.viewer.pan = [pan.x, pan.y];
            }
            Tool::Type | Tool::TypeVertical => {}
            Tool::Pen | Tool::MaskFeather => {}
            Tool::PenAdd => {
                if let Some((l, m, seg, tt)) = ov::segment_at(&paths, &map, pos, 6.0) {
                    let _ = app.session.execute("mask.insertVertex", json!({"layer": l.0, "mask": m, "segment": seg, "t": tt}));
                }
            }
            Tool::PenDelete => {
                if let Some(h) = vertex_at(pos, false) {
                    let _ = app.session.execute("mask.deleteVertices", json!({"vertices": [{"layer": h.layer.0, "mask": h.mask, "index": h.index}]}));
                }
            }
            Tool::PenConvert => {
                if let Some(h) = vertex_at(pos, false) {
                    let _ = app.session.execute("mask.convertVertex", json!({"layer": h.layer.0, "mask": h.mask, "index": h.index}));
                }
            }
            Tool::Selection if mods.alt && vertex_at(pos, false).is_some() => {
                if let Some(h) = vertex_at(pos, false) {
                    let _ = app.session.execute("mask.convertVertex", json!({"layer": h.layer.0, "mask": h.mask, "index": h.index}));
                }
            }
            Tool::Selection if ft.is_some() && ft.as_ref().and_then(|f| ov::begin_ft_drag(f, pos, &map)).is_none() => {
                // Clicking away from the free-transform box ends it.
                ov::end_free_transform(&ctx);
            }
            t if t.puppet_kind().is_some() => {
                if pin_hits.iter().all(|h| h.pos.distance(pos) >= 8.0)
                    && let Some(l) = pick(app, &ectx, cpt, false).and_then(|l| comp.layer(l))
                    && let Some(inv) = l2c(&ectx, l).0.inverse()
                {
                    let lp = inv.apply(gv2(cpt[0], cpt[1]));
                    let r = app.session.execute("puppet.addPin", json!({"layer": l.id.0, "kind": t.puppet_kind(), "position": [lp.x, lp.y]}));
                    if let Err(e) = r {
                        app.ui.status = e.to_string();
                    }
                }
            }
            Tool::Selection if vertex_at(pos, false).is_some() => {
                if let Some(h) = vertex_at(pos, false) {
                    let me = json!({"layer": h.layer.0, "mask": h.mask, "index": h.index});
                    let _ = app.session.execute("mask.selectVertices", json!({"vertices": [me], "add": mods.shift, "toggle": mods.shift}));
                }
            }
            _ => {
                if pick(app, &ectx, cpt, mods.shift).is_none() {
                    let _ = app.session.execute("edit.deselectAll", json!({}));
                }
            }
        }
    }
    if resp.double_clicked()
        && !text_owns
        && app.ui.tool == Tool::Selection
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some((l, m)) = vertex_at(pos, false).map(|h| (h.layer, h.mask)).or_else(|| ov::segment_at(&paths, &map, pos, 6.0).map(|s| (s.0, s.1)))
    {
        // Double-click a path: free transform its points (Layer ▸ Mask and Shape Path ▸ Free
        // Transform Points).
        ov::begin_free_transform(&ctx, l, m);
    } else if resp.double_clicked()
        && !text_owns
        && !app.ui.tool.is_pen()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let cpt = map.to_comp(pos);
        if let Some(l) = pick(app, &ectx, cpt, false).and_then(|l| comp.layer(l))
            && let effectcraft_engine::project::LayerSource::Comp { item } = &l.source
        {
            app.session.open_comp(*item);
        }
    }
    // Effect point controls and crosshair/eyedropper picks (on top of the viewer's gestures).
    crate::panels::effect_controls::viewer_hook(app, ui, &painter, &map, &ectx, &|c, l| l2c(c, l).0);

    // Status chips (render time / caching).
    if app.playback.playing && app.playback.waiting {
        let r = Rect::from_min_size(area.min + vec2(10.0, 10.0), vec2(150.0, 22.0));
        painter.rect_filled(r, 11.0, Color32::from_black_alpha(170));
        painter.text(r.center(), Align2::CENTER_CENTER, "Caching frames…", Tokens::ui(11.5), t.cache_green);
    }

    vt::draw_snap(&ctx, &painter, &map, &ectx);
    vt::rulers(app, ui, &map, outer, area);
    vt::bottom_bar(app, ui, bar, zoom, fit, time, &comp);
    if err_h > 0.0 {
        super::expr_bar::draw(app, ui, err_bar, cid);
    }
}

/// Parent-space position change per comp pixel of drag (x and y) for a layer: 2D layers map
/// through the parent transform; 3D layers move in the view plane at their depth.
fn drag_axes(ctx: &EvalCtx, l: &Layer) -> ([f64; 3], [f64; 3]) {
    if !l.is_3d() {
        let inv = parent_inverse(ctx, l);
        let a = inv.apply_vec(gv2(1.0, 0.0));
        let b = inv.apply_vec(gv2(0.0, 1.0));
        return ([a.x, a.y, 0.0], [b.x, b.y, 0.0]);
    }
    let cam = cam_state(ctx);
    let anchor = l.transform().map(|tr| ctx.v3(l, tr, "anchor", [0.0; 3])).unwrap_or([0.0; 3]);
    let wp = ctx.world_matrix(l).apply(anchor.into());
    let k = if cam.ortho { 1.0 / cam.zoom.max(1e-9) } else { cam.depth(wp).max(1.0) / cam.zoom.max(1e-9) };
    let pinv = match l.parent.and_then(|p| ctx.comp.layer(p)) {
        Some(p) => ctx.world_matrix(p).inverse().unwrap_or_default(),
        None => effectcraft_engine::geom::Mat4::IDENTITY,
    };
    let a = pinv.apply_vec(cam.right() * k);
    let b = pinv.apply_vec(cam.down() * k);
    ([a.x, a.y, a.z], [b.x, b.y, b.z])
}

/// Wireframes of cameras (frustum, point of interest) and lights (position, direction) as seen
/// through the current 3D view.
fn draw_rigs(app: &mut EffectcraftApp, painter: &egui::Painter, map: &ViewerMap, ectx: &EvalCtx, selected: &[LayerId]) {
    use effectcraft_engine::geom::Vec3;
    use effectcraft_engine::project::{LayerSource, LightKind};
    let cam = cam_state(ectx);
    let (w, h) = (ectx.comp.width as f64, ectx.comp.height as f64);
    let active = ectx.comp.active_camera(ectx.time).map(|c| c.id);
    let through_active = VIEW_CAM.with(|c| c.get()).is_none();
    let proj = |p: Vec3| cam.project(w, h, p).map(|v| map.to_screen([v.x, v.y]));
    let seg = |a: Vec3, b: Vec3, stroke: Stroke| {
        if let (Some(x), Some(y)) = (proj(a), proj(b)) {
            painter.line_segment([x, y], stroke);
        }
    };
    for l in ectx.comp.layers.iter().filter(|l| (l.is_camera() || l.is_light()) && l.is_active_at(ectx.time)) {
        let sel = selected.contains(&l.id);
        let col = app.tokens.label(l.label).gamma_multiply(if sel { 1.0 } else { 0.6 });
        let stroke = Stroke::new(if sel { 1.5 } else { 1.0 }, col);
        let (eye, fwd, down) = three_d::camera::layer_frame(ectx, l);
        let poi = |l: &Layer| -> Option<Vec3> {
            let pr = l.transform()?.get("poi").filter(|_| three_d::camera::is_two_node(l))?;
            Some(three_d::camera::parent_world(ectx, l).apply(ectx.value(l, pr).as_vec3().into()))
        };
        if l.is_camera() {
            if through_active && Some(l.id) == active {
                continue;
            }
            let zoom = l.props.prop("cameraOptions/zoom").map(|p| ectx.value(l, p).as_f64()).unwrap_or(1000.0);
            let right = down.cross(fwd);
            // Frustum to a plane at a quarter of the zoom distance.
            let k = 0.25;
            let c = eye + fwd * (zoom * k);
            let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(sx, sy)| c + right * (sx * w * 0.5 * k) + down * (sy * h * 0.5 * k));
            for i in 0..4 {
                seg(eye, corners[i], stroke);
                seg(corners[i], corners[(i + 1) % 4], stroke);
            }
            if let Some(poi) = poi(l) {
                seg(eye, poi, Stroke::new(1.0, col.gamma_multiply(0.6)));
                if let Some(s) = proj(poi) {
                    painter.circle_stroke(s, 4.0, stroke);
                }
            }
            if let Some(s) = proj(eye) {
                app.auto.add(&format!("viewer.camera.{}", l.id.0), Rect::from_center_size(s, vec2(14.0, 14.0)), &l.name);
            }
        } else {
            let LayerSource::Light { kind } = l.source else { continue };
            if kind == LightKind::Ambient {
                continue;
            }
            let Some(s) = proj(eye) else { continue };
            painter.circle_stroke(s, 6.0, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([s + d * 8.0, s + d * 12.0], stroke);
            }
            if matches!(kind, LightKind::Spot | LightKind::Parallel) {
                let target = poi(l).unwrap_or(eye + fwd * 300.0);
                seg(eye, target, Stroke::new(1.0, col.gamma_multiply(0.7)));
                if let Some(q) = proj(target) {
                    painter.circle_stroke(q, 3.0, stroke);
                }
            }
            app.auto.add(&format!("viewer.light.{}", l.id.0), Rect::from_center_size(s, vec2(16.0, 16.0)), &l.name);
        }
    }
}

/// Pen tool press at `pos`: continue the path in progress (pressing its first vertex closes it),
/// add a vertex on a visible path's segment, or start a new path: a mask on the selected
/// footage/solid layer, a shape path (Shape n group with Fill and Stroke) on the selected shape
/// layer, or a new shape layer when nothing is selected. The placed point snaps.
fn pen_press(app: &mut EffectcraftApp, ui: &mut egui::Ui, ectx: &EvalCtx, map: &ViewerMap, paths: &[ov::PathInfo], pos: Pos2, mods: egui::Modifiers) {
    use effectcraft_engine::project::LayerSource;
    let gid = egui::Id::new("viewer-gesture");
    let current: Option<(LayerId, u64)> = ui.data(|d| d.get_temp(pen_id()));
    let c0 = map.to_comp(pos);
    let corr = vt::snap(app, ui.ctx(), ectx, map, &[], &[c0], mods);
    let c = [c0[0] + corr[0], c0[1] + corr[1]];
    if let Some((lid, uid)) = current
        && let Some(p) = ov::path_of(ectx, lid, uid)
        && let Some(inv) = p.m.inverse()
    {
        let n = p.sp.vertices.len();
        let first = p.sp.vertices.first().map(|v| {
            let q = p.m.apply(gv2(v[0], v[1]));
            map.to_screen([q.x, q.y])
        });
        if n >= 2 && first.is_some_and(|f| f.distance(pos) < 8.0) {
            let _ = app.session.execute("mask.setClosed", json!({"layer": lid.0, "mask": uid, "closed": true}));
            ui.data_mut(|d| {
                d.remove::<(LayerId, u64)>(pen_id());
                d.remove::<Gesture>(gid);
            });
            return;
        }
        let lp = inv.apply(gv2(c[0], c[1]));
        let lp = [lp.x, lp.y];
        if let Ok(v) = app.session.execute("mask.addVertex", json!({"layer": lid.0, "mask": uid, "point": lp})) {
            let index = v.as_u64().unwrap_or(0) as usize;
            ui.data_mut(|d| d.insert_temp(gid, Gesture::Pen { layer: lid, mask: uid, index, vertex: lp, inv, press: pos }));
        }
        return;
    }
    // On a visible path's segment the Pen adds a vertex there.
    if let Some((l, m, seg, t)) = ov::segment_at(paths, map, pos, 5.0) {
        let _ = app.session.execute("mask.insertVertex", json!({"layer": l.0, "mask": m, "segment": seg, "t": t}));
        return;
    }
    let target = app
        .session
        .state
        .selected_layers
        .iter()
        .copied()
        .find(|id| ectx.comp.layer(*id).is_some_and(|l| !l.switches.locked && (l.masks().is_some() || matches!(l.source, LayerSource::Shape))));
    let layer = target.and_then(|id| ectx.comp.layer(id));
    let (lid, uid, inv) = match layer {
        Some(l) if !matches!(l.source, LayerSource::Shape) => {
            let Some(inv) = l2c(ectx, l).0.inverse() else { return };
            let lp = inv.apply(gv2(c[0], c[1]));
            let Ok(v) = app.session.execute("mask.new", json!({"layer": l.id.0, "vertices": [[lp.x, lp.y]]})) else { return };
            let Some(mask) = v["mask"].as_u64() else { return };
            (l.id, mask, inv)
        }
        Some(l) => {
            let Some(inv) = l2c(ectx, l).0.inverse() else { return };
            let lp = inv.apply(gv2(c[0], c[1]));
            let fill = app.ui.fill_color;
            let stroke = app.ui.stroke_color;
            let r = app.session.execute(
                "shape.newPath",
                json!({"layer": l.id.0, "vertices": [[lp.x, lp.y]], "space": "layer", "fill": [fill[0], fill[1], fill[2]], "stroke": [stroke[0], stroke[1], stroke[2]], "strokeWidth": app.ui.stroke_width}),
            );
            let Some(path) = r.ok().and_then(|v| v["path"].as_u64()) else { return };
            (l.id, path, inv)
        }
        None => {
            // A new shape layer at the comp centre (unrotated: comp → layer is a translation).
            let fill = app.ui.fill_color;
            let stroke = app.ui.stroke_color;
            let r = app.session.execute(
                "shape.newPath",
                json!({"vertices": [c], "space": "comp", "fill": [fill[0], fill[1], fill[2]], "stroke": [stroke[0], stroke[1], stroke[2]], "strokeWidth": app.ui.stroke_width}),
            );
            let Some(v) = r.ok() else { return };
            let (Some(l), Some(path)) = (v["layer"].as_u64(), v["path"].as_u64()) else { return };
            let inv = Mat3::translate(gv2(-(ectx.comp.width as f64) / 2.0, -(ectx.comp.height as f64) / 2.0));
            (LayerId(l), path, inv)
        }
    };
    let lp = inv.apply(gv2(c[0], c[1]));
    ui.data_mut(|d| {
        d.insert_temp(pen_id(), (lid, uid));
        d.insert_temp(gid, Gesture::Pen { layer: lid, mask: uid, index: 0, vertex: [lp.x, lp.y], inv, press: pos });
    });
}

/// Topmost layer under a comp point (selects it; shift toggles). Returns the hit layer.
pub(crate) fn pick(app: &mut EffectcraftApp, ectx: &EvalCtx, cpt: [f64; 2], toggle: bool) -> Option<LayerId> {
    let hit = ectx
        .comp
        .layers
        .iter()
        .filter(|l| l.is_active_at(ectx.time) && l.switches.video && !l.switches.locked && !l.is_camera() && !l.is_light())
        .find(|l| layer_quad(ectx, l).is_some_and(|(_, q, _)| point_in_quad(cpt, &q)))
        .map(|l| l.id)?;
    if toggle {
        let _ = app.session.execute("layer.select", json!({"layers": [hit.0], "toggle": true}));
    } else if !app.session.state.selected_layers.contains(&hit) {
        let _ = app.session.execute("layer.select", json!({"layers": [hit.0]}));
    }
    Some(hit)
}

fn create_shape(app: &mut EffectcraftApp, tool: Tool, a: [f64; 2], b: [f64; 2], square: bool) {
    let mut w = (b[0] - a[0]).abs();
    let mut h = (b[1] - a[1]).abs();
    if w < 2.0 && h < 2.0 {
        return;
    }
    if square {
        let m = w.max(h);
        w = m;
        h = m;
    }
    let cx = a[0].min(b[0]) + w / 2.0;
    let cy = a[1].min(b[1]) + h / 2.0;
    let kind = match tool {
        Tool::Rectangle => "rect",
        Tool::RoundedRect => "rounded",
        Tool::Ellipse => "ellipse",
        Tool::Polygon => "polygon",
        _ => "star",
    };
    // A non-shape layer selected and "creates mask": add a mask instead.
    let sel = app.session.state.selected_layers.first().copied();
    let sel_layer = sel.and_then(|id| app.session.active_comp().and_then(|c| c.layer(id)).cloned());
    if let Some(l) = sel_layer
        && !matches!(l.source, effectcraft_engine::project::LayerSource::Shape)
        && l.source.is_av()
        && matches!(kind, "rect" | "ellipse")
    {
        // Mask in layer space: invert the layer transform.
        let comp = app.session.active_comp().cloned();
        if let Some(comp) = comp {
            let ectx = EvalCtx {
                project: &app.session.project,
                comp_id: app.session.active_comp_id().unwrap_or_default(),
                comp: &comp,
                time: app.session.time(),
                expr: None,
                footage: None,
            };
            let inv = l2c(&ectx, &l).0.inverse().unwrap_or(Mat3::IDENTITY);
            let p0 = inv.apply(gv2(cx - w / 2.0, cy - h / 2.0));
            let p1 = inv.apply(gv2(cx + w / 2.0, cy + h / 2.0));
            let _ = app.session.execute(
                "layer.addMask",
                json!({"layer": l.id.0, "shape": if kind == "ellipse" { "ellipse" } else { "rect" }, "rect": [p0.x.min(p1.x), p0.y.min(p1.y), (p1.x - p0.x).abs(), (p1.y - p0.y).abs()]}),
            );
            return;
        }
    }
    let fill = app.ui.fill_color;
    let stroke = app.ui.stroke_color;
    let _ = app.session.execute(
        "layer.newShape",
        json!({"kind": kind, "size": [w, h], "position": [cx, cy], "fill": [fill[0], fill[1], fill[2]], "stroke": [stroke[0], stroke[1], stroke[2]], "strokeWidth": app.ui.stroke_width}),
    );
}

fn empty_state(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter();
    p.rect_filled(rect, 0.0, t.pasteboard);
    let c = rect.center();
    p.text(c - vec2(0.0, 40.0), Align2::CENTER_CENTER, "Composition", Tokens::semibold(16.0), t.text);
    let b1 = Rect::from_center_size(c + vec2(0.0, 0.0), vec2(220.0, 30.0));
    if widgets::text_button(ui, b1, "New Composition", true, &t, egui::Id::new("empty-newcomp")).clicked() {
        crate::panels::dialogs::open_new_comp(app);
    }
    app.auto.add("viewer.empty.newComp", b1, "New Composition");
    let b2 = Rect::from_center_size(c + vec2(0.0, 40.0), vec2(220.0, 30.0));
    if widgets::text_button(ui, b2, "New Composition From Footage", false, &t, egui::Id::new("empty-fromfootage")).clicked() {
        let _ = crate::menus::invoke(app, &ui.ctx().clone(), "file.import", json!({}));
    }
    app.auto.add("viewer.empty.import", b2, "New Composition From Footage");
}

/// `#rrggbb` → sRGB bytes.
pub(crate) fn hex_rgb(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

/// Put a rendered frame on screen: CPU pixels go into an egui texture; GPU frames are drawn
/// straight from their wgpu texture (registered with egui-wgpu, no readback).
fn show_frame(app: &mut EffectcraftApp, ctx: &egui::Context, key: crate::frames::FrameKey, img: crate::frames::FrameImage) {
    use crate::frames::FrameImage;
    match img {
        FrameImage::Cpu(img) => {
            match &mut app.viewer_tex {
                Some((tex, k)) if tex.size() == img.size => {
                    tex.set((*img).clone(), egui::TextureOptions::LINEAR);
                    *k = key;
                }
                _ => app.viewer_tex = Some((ctx.load_texture("viewer-frame", (*img).clone(), egui::TextureOptions::LINEAR), key)),
            }
            app.viewer_shown = app.viewer_tex.as_ref().map(|(t, k)| (t.id(), *k));
            app.viewer_image = Some(img);
            vt::set_texture_roi(ctx, app.session.state.region_of_interest);
        }
        FrameImage::Gpu(f) => {
            let Some(rs) = &app.wgpu else { return };
            let view = f.texture.create_view(&Default::default());
            let mut ren = rs.renderer.write();
            let id = match &app.viewer_native {
                Some((id, _, _)) => {
                    ren.update_egui_texture_from_wgpu_texture(&rs.device, &view, eframe::wgpu::FilterMode::Linear, *id);
                    *id
                }
                None => ren.register_native_texture(&rs.device, &view, eframe::wgpu::FilterMode::Linear),
            };
            drop(ren);
            app.viewer_native = Some((id, key, f));
            app.viewer_shown = Some((id, key));
            // Pixels are read back on demand (viewer_pixels).
            app.viewer_image = None;
        }
    }
}
