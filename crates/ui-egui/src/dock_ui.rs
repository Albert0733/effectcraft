//! The dock area: lays out the docked tree (or the maximized panel), draws each group's chrome
//! and body, floating (undocked) panel groups over it, and tab drag-and-drop: drag a tab onto a
//! group to tab it there (center) or split it off to a side (After Effects' drop zones, with a
//! highlight of where it will land); hold Cmd/Ctrl when releasing, or drop outside every group,
//! to float it. `` ` `` maximizes the panel under the pointer.

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::dock::{self, DockAction, DockNode, Group, Layout, PanelKind, Zone};
use crate::{EffectcraftApp, panels};

fn drag_id() -> egui::Id {
    egui::Id::new("dock-tab-drag")
}

impl EffectcraftApp {
    /// Tab labels that name what the panel shows, as After Effects does: "Composition Intro",
    /// "Effect Controls Title", "Properties: Title", and the Timeline tab named after its comp.
    fn tab_titles(&self) -> Vec<(PanelKind, String)> {
        let mut out = Vec::new();
        // ScriptUI panels are named after their script.
        for w in self.session.script_ui.windows.iter().filter(|w| w.kind == effectcraft_engine::scriptui::WindowKind::Panel) {
            if let Some(t) = panels::scriptui_view::panel_title(self, w.id) {
                out.push((PanelKind::ScriptPanel(w.id), t));
            }
        }
        let Some(comp) = self.session.active_comp() else { return out };
        let cname = self.session.active_comp_id().and_then(|id| self.session.project.item(id)).map(|i| i.name.clone()).unwrap_or_default();
        out.push((PanelKind::Composition, format!("Composition {cname}")));
        out.push((PanelKind::Timeline, cname));
        if let Some(l) = self.session.state.selected_layers.first().and_then(|id| comp.layer(*id)) {
            out.push((PanelKind::EffectControls, format!("Effect Controls {}", l.name)));
            out.push((PanelKind::Properties, format!("Properties: {}", l.name)));
        }
        out
    }

    /// The Composition and Timeline tabs' close button, label-colour swatch and viewer lock.
    fn tab_decos(&self) -> Vec<(PanelKind, dock::TabDeco)> {
        let Some(cid) = self.session.active_comp_id() else { return vec![] };
        let swatch = match self.session.project.item(cid).map(|i| i.label) {
            Some(l) if l != effectcraft_engine::color::Label::None => self.tokens.label(l),
            _ => self.tokens.text_faint,
        };
        let mut v: Vec<(PanelKind, dock::TabDeco)> = [PanelKind::Composition, PanelKind::Timeline]
            .into_iter()
            .map(|p| (p, dock::TabDeco { swatch, locked: self.ui.locked_tabs.contains(&p.id()), viewer: true }))
            .collect();
        // Effect Controls carries the selected layer's label colour.
        if let Some(l) = self.session.active_comp().and_then(|c| self.session.state.selected_layers.first().and_then(|id| c.layer(*id)))
            && l.label != effectcraft_engine::color::Label::None
        {
            v.push((PanelKind::EffectControls, dock::TabDeco { swatch: self.tokens.label(l.label), locked: false, viewer: false }));
        }
        v
    }

    /// Settings ▸ Appearance ▸ Use Label Color for Related Tabs: the Composition and Timeline
    /// tabs carry their comp's label colour, Effect Controls and Properties the layer's.
    fn label_tab_marks(&self, ui: &egui::Ui) {
        if !self.session.prefs.appearance.use_label_color_for_tabs {
            return;
        }
        let Some(cid) = self.session.active_comp_id() else { return };
        let comp_label = self.session.project.item(cid).map(|i| i.label);
        let layer_label = self.session.active_comp().and_then(|c| self.session.state.selected_layers.first().and_then(|l| c.layer(*l))).map(|l| l.label);
        // (the Composition, Timeline and Effect Controls tabs carry their swatch already)
        let _ = comp_label;
        for (p, label) in [(PanelKind::Properties, layer_label)] {
            let (Some(label), Some(e)) = (label, self.auto.find(&format!("panel.tab.{}", p.id()))) else { continue };
            if label == effectcraft_engine::color::Label::None {
                continue;
            }
            let r = Rect::from_min_size(pos2(e.rect[0] + 2.0, e.rect[1] + 9.0), vec2(4.0, (e.rect[3] - 16.0).max(6.0)));
            ui.painter().rect_filled(r, 1.0, self.tokens.label(label));
        }
    }

    /// Edit the full layout (docked tree + floating groups) with a [`Layout`] operation.
    pub fn edit_layout(&mut self, f: impl FnOnce(&mut Layout) -> bool) -> bool {
        let mut l = Layout { root: self.ui.dock.clone(), floating: self.ui.floating.clone() };
        if !f(&mut l) {
            return false;
        }
        self.ui.dock = l.root;
        self.ui.floating = l.floating;
        if self.ui.maximized.is_some_and(|m| !self.ui.dock.contains(m)) {
            self.ui.maximized = None;
        }
        true
    }

    /// Close a panel wherever it is (docked or floating).
    pub fn close_panel(&mut self, p: PanelKind) {
        // Closing a ScriptUI panel closes its script window (its script's onClose runs).
        if let PanelKind::ScriptPanel(id) = p
            && self.session.script_ui.window(id).is_some()
            && let Err(e) = self.session.execute("scriptui.close", serde_json::json!({"window": id}))
        {
            self.ui.status = e.to_string();
        }
        self.ui.dock.close(p);
        self.edit_layout(|l| l.unfloat(p));
        if self.ui.maximized == Some(p) {
            self.ui.maximized = None;
        }
    }

    /// Toggle the maximized panel (`p`, or none).
    pub fn toggle_maximize(&mut self, p: PanelKind) {
        self.ui.maximized = if self.ui.maximized == Some(p) || !self.ui.dock.contains(p) { None } else { Some(p) };
    }

    /// The docked panel whose group is under `pos` (from the last frame), else the focused one.
    pub fn panel_at(&self, pos: Option<egui::Pos2>) -> PanelKind {
        pos.and_then(|p| self.dock_rects.iter().rev().find(|(_, r)| r.contains(p)).map(|(k, _)| *k)).unwrap_or(self.ui.focused)
    }

    fn panel_body(&mut self, ui: &mut egui::Ui, p: PanelKind, rect: Rect) {
        self.auto.add(&format!("panel.{}", p.id()), rect, p.title());
        let mut content = rect;
        if p == PanelKind::Composition && !self.ui.start_screen && panels::precomp::has_flow(self) {
            // Composition Navigator: the flow of nested comps above the viewer.
            let nav = Rect::from_min_size(content.min, vec2(content.width(), panels::precomp::NAV_H));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(nav).id_salt("comp-navigator"));
            child.set_clip_rect(nav.intersect(ui.clip_rect()));
            panels::precomp::navigator(self, &mut child, nav);
            content.min.y = nav.max.y;
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content).id_salt(("panel", p.id())));
        child.set_clip_rect(content.intersect(ui.clip_rect()));
        panels::show(self, &mut child, p, content);
        if p == PanelKind::Composition && !self.ui.start_screen {
            panels::anim_tools::sketch_overlay(self, &mut child);
        }
    }

    pub(crate) fn dock_area(&mut self, ui: &mut egui::Ui, body: Rect) {
        let t = self.tokens;
        let ctx = ui.ctx().clone();
        // The maximized panel fills the area; the tree is kept as it is.
        let maximized = self.ui.maximized.filter(|m| self.ui.dock.contains(*m));
        let mut dock = match maximized {
            Some(m) => DockNode::Tabs { panels: vec![m], active: 0 },
            None => std::mem::replace(&mut self.ui.dock, DockNode::Tabs { panels: vec![], active: 0 }),
        };
        let mut groups = Vec::new();
        dock::layout(ui, &mut dock, body, &t, "", &mut groups, &mut self.auto);
        let mut actions = Vec::new();
        let titles = self.tab_titles();
        let decos = self.tab_decos();
        let title = |p: PanelKind| titles.iter().find(|(k, _)| *k == p).map(|(_, s)| s.clone()).unwrap_or_else(|| p.title().to_string());
        for g in &groups {
            actions.extend(dock::draw_group_chrome(ui, g, self.ui.focused, &t, &mut self.auto, &title, &decos));
        }
        self.label_tab_marks(ui);
        if maximized.is_none() {
            self.ui.dock = dock;
        }
        self.dock_rects = groups.iter().filter_map(|g| Some((*g.panels.get(g.active)?, g.rect))).collect();
        for g in &groups {
            let Some(p) = g.panels.get(g.active).copied() else { continue };
            if g.content.height() < 2.0 {
                continue; // a collapsed stacked panel: header only
            }
            self.panel_body(ui, p, g.content);
        }
        // Floating groups over the dock.
        let mut float_groups: Vec<Group> = vec![];
        for i in 0..self.ui.floating.len() {
            if let Some(g) = self.floating_window(&ctx, i, &title, &mut actions) {
                float_groups.push(g);
            }
        }
        for a in actions {
            match a {
                DockAction::Activate(p) => {
                    if !self.ui.dock.activate(p)
                        && let Some(f) = self.ui.floating.iter_mut().find(|f| f.panels.contains(&p))
                    {
                        f.active = f.panels.iter().position(|x| *x == p).unwrap_or(0);
                    }
                }
                DockAction::ToggleStacked(p) => {
                    self.ui.dock.toggle_stacked(p);
                }
                DockAction::Focus(p) => self.ui.focused = p,
                DockAction::Close(p) => self.close_panel(p),
                DockAction::PanelMenu(p, pos) => {
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("panel-menu"), (p, pos)));
                }
                DockAction::BeginDrag(p) => ctx.data_mut(|d| {
                    d.insert_temp(drag_id(), p);
                }),
                DockAction::ToggleLock(p) => {
                    if !self.ui.locked_tabs.remove(&p.id()) {
                        self.ui.locked_tabs.insert(p.id().to_string());
                    }
                }
            }
        }
        self.tab_drag(&ctx, &groups, &float_groups);
        panels::panel_menu_popup(self, ui);
    }

    /// One floating group: a window with a tab strip (drag the empty strip to move it, the
    /// corner to resize it) and the active panel's body.
    fn floating_window(&mut self, ctx: &egui::Context, i: usize, title: &dyn Fn(PanelKind) -> String, actions: &mut Vec<DockAction>) -> Option<Group> {
        let t = self.tokens;
        let f = self.ui.floating.get(i)?.clone();
        let screen = ctx.content_rect();
        let mut rect = Rect::from_min_size(pos2(f.rect[0], f.rect[1]), vec2(f.rect[2].max(160.0), f.rect[3].max(100.0)));
        // Keep the strip reachable.
        rect = rect.translate(vec2(
            (screen.min.x - rect.min.x).max(0.0) + (screen.max.x - 40.0 - rect.min.x).min(0.0),
            (screen.min.y - rect.min.y).max(0.0) + (screen.max.y - 30.0 - rect.min.y).min(0.0),
        ));
        let id = egui::Id::new(("floating-panel", f.panels.first().map(|p| p.id()).unwrap_or_default()));
        let mut group = None;
        egui::Area::new(id).order(egui::Order::Middle).fixed_pos(rect.min).interactable(true).show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
            ui.painter().rect_filled(r.expand(1.0), t.radius, Color32::from_black_alpha(90));
            let strip = Rect::from_min_size(r.min, vec2(r.width(), t.tab_h));
            // Move by the strip (tabs, registered later, take precedence over it).
            let mv = ui.interact(strip, id.with("move"), Sense::drag());
            self.auto.add(&format!("panel.float.{i}.move"), strip, "Move floating panel");
            let mut nr = r;
            if mv.dragged() {
                nr = nr.translate(mv.drag_delta());
            }
            let g = Group {
                path: format!("f{i}"),
                rect: r,
                content: Rect::from_min_max(pos2(r.min.x, r.min.y + t.tab_h), r.max),
                panels: f.panels.clone(),
                active: f.active.min(f.panels.len().saturating_sub(1)),
                stacked: None,
            };
            let decos = self.tab_decos();
            actions.extend(dock::draw_group_chrome(ui, &g, self.ui.focused, &t, &mut self.auto, title, &decos));
            ui.painter().rect_stroke(r, t.radius, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
            if let Some(p) = g.panels.get(g.active).copied() {
                self.panel_body(ui, p, g.content);
            }
            // Resize from the corner.
            let grip = Rect::from_min_max(r.max - vec2(14.0, 14.0), r.max);
            let rs = ui.interact(grip, id.with("resize"), Sense::drag());
            self.auto.add(&format!("panel.float.{i}.resize"), grip, "Resize floating panel");
            for k in 0..3 {
                let o = 4.0 + k as f32 * 4.0;
                ui.painter().line_segment([pos2(r.max.x - o, r.max.y - 2.0), pos2(r.max.x - 2.0, r.max.y - o)], Stroke::new(1.0, t.text_faint));
            }
            if rs.hovered() || rs.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            }
            if rs.dragged() {
                nr = Rect::from_min_size(nr.min, (nr.size() + rs.drag_delta()).max(vec2(160.0, 100.0)));
            }
            if nr != r
                && let Some(fl) = self.ui.floating.get_mut(i)
            {
                fl.rect = [nr.min.x, nr.min.y, nr.width(), nr.height()];
            }
            group = Some(g);
        });
        group
    }

    /// A tab being dragged: highlight the drop zone under the pointer; on release, dock (or
    /// float with Cmd/Ctrl or outside every group).
    fn tab_drag(&mut self, ctx: &egui::Context, groups: &[Group], floats: &[Group]) {
        let Some(p) = ctx.data(|d| d.get_temp::<PanelKind>(drag_id())) else { return };
        let t = self.tokens;
        let Some(pos) = ctx.pointer_latest_pos() else { return };
        // Floating windows are on top; then the docked groups.
        let target = floats.iter().map(|g| (g, true)).chain(groups.iter().map(|g| (g, false))).find(|(g, _)| g.rect.contains(pos));
        let float_drop = ctx.input(|i| i.modifiers.command);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("dock-drop")));
        let plan: Option<(PanelKind, Zone)> = match target {
            Some((g, is_float)) if !float_drop => {
                let anchor = g.panels.get(g.active).copied().unwrap_or(p);
                let zone = if is_float { Zone::Center } else { dock::drop_zone(g.rect, t.tab_h, pos) };
                let self_only = anchor == p && g.panels.len() == 1;
                if self_only {
                    None
                } else {
                    let anchor = if anchor == p { g.panels.iter().copied().find(|x| *x != p).unwrap_or(p) } else { anchor };
                    let hr = dock::zone_rect(g.rect, zone).shrink(2.0);
                    painter.rect_filled(hr, t.radius, t.focus.gamma_multiply(0.25));
                    painter.rect_stroke(hr, t.radius, Stroke::new(2.0, t.focus), StrokeKind::Inside);
                    // Outline the whole group for the side zones, like AE's drop-zone frame.
                    if zone != Zone::Center {
                        painter.rect_stroke(g.rect, t.radius, Stroke::new(1.0, t.focus.gamma_multiply(0.6)), StrokeKind::Inside);
                    }
                    Some((anchor, zone))
                }
            }
            _ => None,
        };
        // Ghost of the dragged tab.
        let g = painter.layout_no_wrap(p.title().to_string(), crate::theme::Tokens::ui(12.0), Color32::WHITE);
        let gr = Rect::from_min_size(pos + vec2(12.0, 10.0), g.size() + vec2(16.0, 8.0));
        painter.rect_filled(gr, 4.0, t.focus.gamma_multiply(0.85));
        painter.galley(gr.min + vec2(8.0, 4.0), g, Color32::WHITE);
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        ctx.request_repaint();
        if ctx.input(|i| i.pointer.any_released()) {
            ctx.data_mut(|d| d.remove::<PanelKind>(drag_id()));
            let ok = match plan {
                Some((anchor, zone)) => self.edit_layout(|l| l.dock(p, anchor, zone)),
                None if float_drop || target.is_none() => {
                    let r = [pos.x - 40.0, pos.y - 12.0, 420.0, 320.0];
                    self.edit_layout(|l| l.float(p, r))
                }
                None => false,
            };
            if ok {
                self.ui.focused = p;
                self.show_panel(p);
            }
        }
    }
}

/// A new floating group's default rect near the centre of `screen`.
pub fn default_float_rect(screen: Rect) -> [f32; 4] {
    let c = screen.center();
    [c.x - 210.0, c.y - 160.0, 420.0, 320.0]
}
