//! Puppet tools in the Composition viewer: the deformed mesh and pins of the selected layers'
//! Puppet effect, click to add a pin (`puppet.addPin`), drag a pin to move it
//! (`puppet.movePin`, merged into one undo step per drag).

use std::sync::Arc;

use effectcraft_engine::effects::puppet::{self, Mesh, Pin, PinKind};
use effectcraft_engine::geom::vec2 as gv2;
use effectcraft_engine::project::{ItemId, Layer, LayerId};
use effectcraft_engine::render::EvalCtx;
use effectcraft_engine::time::Tick;
use egui::{Color32, Pos2, Stroke};

use super::viewer::{ViewerMap, l2c};
use crate::EffectcraftApp;

/// Deformed meshes of a layer's Puppet effect: (mesh uid, mesh, deformed vertices, pins).
pub type Overlay = Arc<Vec<(u64, Arc<Mesh>, Vec<[f64; 2]>, Vec<Pin>)>>;

/// A pin drawn this frame.
#[derive(Clone, Copy, Debug)]
pub struct PinHit {
    pub layer: LayerId,
    pub pin: u64,
    pub kind: PinKind,
    pub pos: Pos2,
}

#[derive(Clone)]
struct Cached {
    key: (u64, u64, u64, i64),
    overlay: Overlay,
}

/// The deformed mesh of `layer`'s Puppet effect at comp time `t` (cached per project revision).
pub fn overlay(app: &EffectcraftApp, ctx: &egui::Context, cid: ItemId, layer: &Layer, t: Tick) -> Option<Overlay> {
    let fx = layer.effects()?.groups().find(|g| puppet::is_puppet(g))?;
    let key = (app.session.revision, cid.0, layer.id.0, t.0);
    let id = egui::Id::new(("puppet-overlay", layer.id.0));
    if let Some(c) = ctx.data(|d| d.get_temp::<Cached>(id))
        && c.key == key
    {
        return Some(c.overlay);
    }
    let (buf, params) = effectcraft_engine::commands::puppet::puppet_eval(&app.session, cid, layer.id, fx.uid, t)?;
    let overlay: Overlay = Arc::new(puppet::overlay(&buf, &params));
    ctx.data_mut(|d| d.insert_temp(id, Cached { key, overlay: overlay.clone() }));
    Some(overlay)
}

/// Where a pin sits now (layer space): its Position, or (Bend / Starch / Overlap pins) its rest
/// point carried along by the deformation.
pub fn pin_position(mesh: &Mesh, def: &[[f64; 2]], p: &Pin) -> [f64; 2] {
    if p.kind.moves() {
        return p.position;
    }
    let v = (0..mesh.verts.len()).min_by(|a, b| {
        let da = (mesh.verts[*a][0] - p.rest[0]).hypot(mesh.verts[*a][1] - p.rest[1]);
        let db = (mesh.verts[*b][0] - p.rest[0]).hypot(mesh.verts[*b][1] - p.rest[1]);
        da.total_cmp(&db)
    });
    match v {
        Some(v) => [def[v][0] + p.rest[0] - mesh.verts[v][0], def[v][1] + p.rest[1] - mesh.verts[v][1]],
        None => p.rest,
    }
}

/// Draw the mesh (when `show_mesh`) and pins of `layer`; returns pin hit targets.
pub fn draw(painter: &egui::Painter, map: &ViewerMap, ectx: &EvalCtx, layer: &Layer, ov: &Overlay, show_mesh: bool, selected: &[u64]) -> Vec<PinHit> {
    let (m, _) = l2c(ectx, layer);
    let scr = |q: [f64; 2]| {
        let c = m.apply(gv2(q[0], q[1]));
        map.to_screen([c.x, c.y])
    };
    let mut hits = vec![];
    for (_, mesh, def, pins) in ov.iter() {
        if show_mesh {
            let st = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xf0, 0xd0, 0x40, 110));
            for t in &mesh.tris {
                let (a, b, c) = (scr(def[t[0]]), scr(def[t[1]]), scr(def[t[2]]));
                painter.line_segment([a, b], st);
                painter.line_segment([b, c], st);
                painter.line_segment([c, a], st);
            }
        }
        for pn in pins {
            let pos = scr(pin_position(mesh, def, pn));
            let sel = selected.contains(&pn.uid);
            let col = match pn.kind {
                PinKind::Starch => Color32::from_rgb(0xe0, 0x50, 0x50),
                PinKind::Overlap => Color32::from_rgb(0x40, 0x90, 0xf0),
                PinKind::Bend => Color32::from_rgb(0xf0, 0x90, 0x30),
                _ => Color32::from_rgb(0xf0, 0xd0, 0x40),
            };
            if matches!(pn.kind, PinKind::Starch | PinKind::Overlap) {
                // Extent ring (layer pixels → screen).
                let r = (scr([pn.rest[0] + pn.extent, pn.rest[1]]) - scr(pn.rest)).length();
                painter.circle_stroke(pos, r.max(2.0), Stroke::new(1.0, col.gamma_multiply(0.6)));
            }
            if matches!(pn.kind, PinKind::Bend | PinKind::Advanced) {
                painter.circle_stroke(pos, 9.0, Stroke::new(1.5, col));
            }
            painter.circle_filled(pos, if sel { 5.5 } else { 4.5 }, if sel { col } else { col.gamma_multiply(0.85) });
            painter.circle_stroke(pos, if sel { 5.5 } else { 4.5 }, Stroke::new(1.0, Color32::BLACK));
            hits.push(PinHit { layer: layer.id, pin: pn.uid, kind: pn.kind, pos });
        }
    }
    hits
}
