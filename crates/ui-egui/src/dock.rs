//! Docking: a tree of splits and tab groups. Panels are rounded frames separated by
//! thin gutters; each group has a tab strip (active tab bright, with a panel menu "≡"); the focused
//! panel gets a blue outline. Gutters drag to resize; workspaces are serialized trees.

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::icons::{self, Icon};
use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelKind {
    Project,
    EffectControls,
    Composition,
    Layer,
    Timeline,
    Info,
    Audio,
    Preview,
    EffectsPresets,
    Properties,
    Character,
    Paragraph,
    Align,
    Tracker,
    Wiggler,
    Smoother,
    MotionSketch,
    Paint,
    Brushes,
    RenderQueue,
    Flowchart,
    History,
    Markers,
    MaskInterpolation,
    ScriptConsole,
}

impl PanelKind {
    pub const ALL: [PanelKind; 25] = [
        PanelKind::Project,
        PanelKind::EffectControls,
        PanelKind::Composition,
        PanelKind::Layer,
        PanelKind::Timeline,
        PanelKind::Info,
        PanelKind::Audio,
        PanelKind::Preview,
        PanelKind::EffectsPresets,
        PanelKind::Properties,
        PanelKind::Character,
        PanelKind::Paragraph,
        PanelKind::Align,
        PanelKind::Tracker,
        PanelKind::Wiggler,
        PanelKind::Smoother,
        PanelKind::MotionSketch,
        PanelKind::Paint,
        PanelKind::Brushes,
        PanelKind::RenderQueue,
        PanelKind::Flowchart,
        PanelKind::History,
        PanelKind::Markers,
        PanelKind::MaskInterpolation,
        PanelKind::ScriptConsole,
    ];
    pub fn title(self) -> &'static str {
        match self {
            PanelKind::Project => "Project",
            PanelKind::EffectControls => "Effect Controls",
            PanelKind::Composition => "Composition",
            PanelKind::Layer => "Layer",
            PanelKind::Timeline => "Timeline",
            PanelKind::Info => "Info",
            PanelKind::Audio => "Audio",
            PanelKind::Preview => "Preview",
            PanelKind::EffectsPresets => "Effects & Presets",
            PanelKind::Properties => "Properties",
            PanelKind::Character => "Character",
            PanelKind::Paragraph => "Paragraph",
            PanelKind::Align => "Align",
            PanelKind::Tracker => "Tracker",
            PanelKind::Wiggler => "Wiggler",
            PanelKind::Smoother => "Smoother",
            PanelKind::MotionSketch => "Motion Sketch",
            PanelKind::Paint => "Paint",
            PanelKind::Brushes => "Brushes",
            PanelKind::RenderQueue => "Render Queue",
            PanelKind::Flowchart => "Flowchart",
            PanelKind::History => "History",
            PanelKind::Markers => "Markers",
            PanelKind::MaskInterpolation => "Mask Interpolation",
            PanelKind::ScriptConsole => "Script Console",
        }
    }
    pub fn id(self) -> String {
        format!("{self:?}")
    }
    pub fn from_name(s: &str) -> Option<PanelKind> {
        let n = s.to_ascii_lowercase().replace([' ', '_', '-', '&'], "");
        Self::ALL.iter().copied().find(|p| format!("{p:?}").to_ascii_lowercase() == n || p.title().to_ascii_lowercase().replace([' ', '&'], "") == n)
    }
    /// Window-menu shortcut.
    pub fn window_shortcut(self) -> Option<&'static str> {
        match self {
            PanelKind::Project => Some("Cmd+0"),
            PanelKind::Info => Some("Cmd+2"),
            PanelKind::Preview => Some("Cmd+3"),
            PanelKind::Audio => Some("Cmd+4"),
            PanelKind::EffectsPresets => Some("Cmd+5"),
            PanelKind::Character => Some("Cmd+6"),
            PanelKind::Paragraph => Some("Cmd+7"),
            PanelKind::Paint => Some("Cmd+8"),
            PanelKind::Brushes => Some("Cmd+9"),
            PanelKind::RenderQueue => Some("Cmd+Alt+0"),
            PanelKind::EffectControls => Some("F3"),
            _ => None,
        }
    }
    /// Chrome-less panels (none in the AE layout; kept for parity with the docking code).
    pub fn compact(self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SplitSize {
    Ratio(f32),
    /// First child has a fixed size in points.
    FixedA(f32),
    /// Second child has a fixed size in points.
    FixedB(f32),
}

/// One panel in a [`DockNode::Stack`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StackEntry {
    pub panel: PanelKind,
    pub open: bool,
    /// Content height when open; `None` = share the remaining height.
    pub height: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DockNode {
    /// `vertical`: children stacked top/bottom; else side by side.
    Split {
        vertical: bool,
        size: SplitSize,
        a: Box<DockNode>,
        b: Box<DockNode>,
    },
    Tabs {
        panels: Vec<PanelKind>,
        active: usize,
    },
    /// After Effects' stacked panels (e.g. the Default workspace's right column): each panel has
    /// a header row; clicking it expands or collapses the panel in place.
    Stack {
        entries: Vec<StackEntry>,
    },
}

fn tabs(p: &[PanelKind], active: usize) -> DockNode {
    DockNode::Tabs { panels: p.to_vec(), active }
}
fn stack(e: &[(PanelKind, bool, Option<f32>)]) -> DockNode {
    DockNode::Stack { entries: e.iter().map(|&(panel, open, height)| StackEntry { panel, open, height }).collect() }
}
fn hsplit(size: SplitSize, a: DockNode, b: DockNode) -> DockNode {
    DockNode::Split { vertical: false, size, a: Box::new(a), b: Box::new(b) }
}
fn vsplit(size: SplitSize, a: DockNode, b: DockNode) -> DockNode {
    DockNode::Split { vertical: true, size, a: Box::new(a), b: Box::new(b) }
}

/// The built-in workspaces: After Effects 2026's (Window ▸ Workspace), less Libraries (an Adobe
/// service). Learn shows the Home screen (community links instead of Adobe's tutorials);
/// Essential Graphics uses the Properties panel; Undocked Panels floats the panels over the
/// Composition panel.
pub const WORKSPACES: &[&str] = &[
    "Default",
    "Standard",
    "Small Screen",
    "Animation",
    "Effects",
    "Motion Tracking",
    "Paint",
    "Text",
    "Minimal",
    "All Panels",
    "Review",
    "Learn",
    "Essential Graphics",
    "Color",
    "Undocked Panels",
];

/// The floating panels a workspace starts with (Undocked Panels), in points for a window of
/// about 1680×1020 (they are kept inside the window when drawn).
pub fn workspace_floating(name: &str) -> Vec<Floating> {
    use PanelKind::*;
    let f = |panels: &[PanelKind], rect: [f32; 4]| Floating { panels: panels.to_vec(), active: 0, rect };
    match name {
        "Undocked Panels" => vec![
            f(&[Project, EffectControls], [24.0, 110.0, 320.0, 420.0]),
            f(&[Preview, Info, Audio], [1330.0, 110.0, 320.0, 300.0]),
            f(&[EffectsPresets, Properties, Character, Paragraph], [1330.0, 430.0, 320.0, 360.0]),
            f(&[Timeline, RenderQueue], [24.0, 640.0, 1280.0, 340.0]),
        ],
        _ => vec![],
    }
}

/// The default layout of a named workspace.
pub fn workspace(name: &str) -> DockNode {
    use PanelKind::*;
    use SplitSize::*;
    let right = |p: &[PanelKind], q: &[PanelKind]| vsplit(Ratio(0.45), tabs(p, 0), tabs(q, 0));
    match name {
        "Animation" => vsplit(
            Ratio(0.55),
            hsplit(
                FixedA(300.0),
                tabs(&[Project, EffectControls], 1),
                hsplit(FixedB(300.0), tabs(&[Composition, Layer], 0), right(&[Info, Preview, Audio], &[EffectsPresets, Smoother, Wiggler, MotionSketch])),
            ),
            tabs(&[Timeline, RenderQueue], 0),
        ),
        "Effects" => vsplit(
            Ratio(0.58),
            hsplit(
                FixedA(340.0),
                tabs(&[EffectControls, Project], 0),
                hsplit(FixedB(300.0), tabs(&[Composition, Layer], 0), right(&[Info, Preview], &[EffectsPresets])),
            ),
            tabs(&[Timeline, RenderQueue], 0),
        ),
        "Motion Tracking" => vsplit(
            Ratio(0.6),
            hsplit(
                FixedA(300.0),
                tabs(&[Project, EffectControls], 0),
                hsplit(FixedB(300.0), tabs(&[Layer, Composition], 0), right(&[Info, Preview], &[Tracker])),
            ),
            tabs(&[Timeline], 0),
        ),
        "Paint" => vsplit(
            Ratio(0.6),
            hsplit(
                FixedA(300.0),
                tabs(&[Project, EffectControls], 0),
                hsplit(FixedB(300.0), tabs(&[Layer, Composition], 0), right(&[Paint, Info], &[Brushes, Preview])),
            ),
            tabs(&[Timeline], 0),
        ),
        "Text" => vsplit(
            Ratio(0.58),
            hsplit(
                FixedA(300.0),
                tabs(&[Project, EffectControls], 0),
                hsplit(FixedB(300.0), tabs(&[Composition], 0), right(&[Character, Paragraph], &[Align, Preview, EffectsPresets])),
            ),
            tabs(&[Timeline], 0),
        ),
        "Minimal" => vsplit(Ratio(0.6), tabs(&[Composition], 0), tabs(&[Timeline], 0)),
        // Review: a large viewer with the markers and the project beside it.
        "Review" => vsplit(
            Ratio(0.66),
            hsplit(FixedA(280.0), tabs(&[Project, Markers], 0), hsplit(FixedB(260.0), tabs(&[Composition, Layer], 0), right(&[Preview, Info], &[Audio]))),
            tabs(&[Timeline, RenderQueue], 0),
        ),
        // Essential Graphics: the Properties panel (After Effects' successor to the Essential
        // Graphics panel) full height on the right, type panels beside the viewer.
        "Essential Graphics" => hsplit(
            FixedB(320.0),
            vsplit(
                Ratio(0.58),
                hsplit(
                    FixedA(280.0),
                    tabs(&[Project, EffectControls], 0),
                    hsplit(FixedB(260.0), tabs(&[Composition, Layer], 0), right(&[Character, Paragraph], &[Align, Preview])),
                ),
                tabs(&[Timeline], 0),
            ),
            stack(&[(Properties, true, None), (EffectsPresets, false, None)]),
        ),
        // Color: wide Effect Controls for grading effects, Info for colour readouts.
        "Color" => vsplit(
            Ratio(0.6),
            hsplit(
                FixedA(360.0),
                tabs(&[EffectControls, Project], 0),
                hsplit(FixedB(300.0), tabs(&[Composition, Layer], 0), right(&[Info, Preview], &[EffectsPresets])),
            ),
            tabs(&[Timeline], 0),
        ),
        // Undocked Panels: the Composition panel docked, the others floating
        // ([`workspace_floating`]).
        "Undocked Panels" => tabs(&[Composition, Layer, Flowchart], 0),
        "Small Screen" => vsplit(
            Ratio(0.55),
            hsplit(
                FixedA(240.0),
                tabs(&[Project, EffectControls], 0),
                hsplit(FixedB(240.0), tabs(&[Composition], 0), tabs(&[Info, Preview, EffectsPresets], 1)),
            ),
            tabs(&[Timeline], 0),
        ),
        "All Panels" => vsplit(
            Ratio(0.55),
            hsplit(
                FixedA(300.0),
                tabs(&[Project, EffectControls, Flowchart, History], 0),
                hsplit(
                    FixedB(300.0),
                    tabs(&[Composition, Layer], 0),
                    right(
                        &[Info, Preview, Audio, Align, Character, Paragraph],
                        &[EffectsPresets, Properties, Tracker, Wiggler, Smoother, MotionSketch, Paint, Brushes, Markers],
                    ),
                ),
            ),
            tabs(&[Timeline, RenderQueue], 0),
        ),
        "Standard" => vsplit(
            Ratio(0.56),
            hsplit(
                FixedA(300.0),
                tabs(&[Project, EffectControls], 0),
                hsplit(FixedB(290.0), tabs(&[Composition, Layer], 0), right(&[Info, Audio], &[Preview, EffectsPresets])),
            ),
            tabs(&[Timeline, RenderQueue], 0),
        ),
        // Default (as in After Effects): Project/Effect Controls | Composition over the Timeline,
        // and a full-height right column with Preview above the Properties panel.
        _ => hsplit(
            FixedB(300.0),
            vsplit(Ratio(0.56), hsplit(FixedA(300.0), tabs(&[Project, EffectControls], 0), tabs(&[Composition, Layer], 0)), tabs(&[Timeline, RenderQueue], 0)),
            stack(&[(Preview, true, Some(46.0)), (Properties, true, None), (Align, false, None), (Audio, false, None), (EffectsPresets, false, None)]),
        ),
    }
}

impl DockNode {
    /// Every panel in the tree.
    pub fn panels(&self, out: &mut Vec<PanelKind>) {
        match self {
            DockNode::Split { a, b, .. } => {
                a.panels(out);
                b.panels(out);
            }
            DockNode::Tabs { panels, .. } => out.extend(panels.iter().copied()),
            DockNode::Stack { entries } => out.extend(entries.iter().map(|e| e.panel)),
        }
    }
    pub fn contains(&self, p: PanelKind) -> bool {
        let mut v = Vec::new();
        self.panels(&mut v);
        v.contains(&p)
    }
    /// Make `p` the active tab of its group. Returns false if not present.
    pub fn activate(&mut self, p: PanelKind) -> bool {
        match self {
            DockNode::Split { a, b, .. } => a.activate(p) || b.activate(p),
            DockNode::Tabs { panels, active } => {
                if let Some(i) = panels.iter().position(|x| *x == p) {
                    *active = i;
                    true
                } else {
                    false
                }
            }
            DockNode::Stack { entries } => match entries.iter_mut().find(|e| e.panel == p) {
                Some(e) => {
                    e.open = true;
                    true
                }
                None => false,
            },
        }
    }
    /// Expand or collapse a stacked panel. Returns false if `p` is not in a stack.
    pub fn toggle_stacked(&mut self, p: PanelKind) -> bool {
        match self {
            DockNode::Split { a, b, .. } => a.toggle_stacked(p) || b.toggle_stacked(p),
            DockNode::Tabs { .. } => false,
            DockNode::Stack { entries } => match entries.iter_mut().find(|e| e.panel == p) {
                Some(e) => {
                    e.open = !e.open;
                    true
                }
                None => false,
            },
        }
    }
    pub fn is_visible(&self, p: PanelKind) -> bool {
        match self {
            DockNode::Split { a, b, .. } => a.is_visible(p) || b.is_visible(p),
            DockNode::Tabs { panels, active } => panels.get(*active) == Some(&p),
            DockNode::Stack { entries } => entries.iter().any(|e| e.panel == p && e.open),
        }
    }
    /// Close a panel (remove its tab; empty groups collapse their split).
    pub fn close(&mut self, p: PanelKind) {
        if let DockNode::Split { a, b, .. } = self {
            a.close(p);
            b.close(p);
            let empty = |n: &DockNode| match n {
                DockNode::Tabs { panels, .. } => panels.is_empty(),
                DockNode::Stack { entries } => entries.is_empty(),
                DockNode::Split { .. } => false,
            };
            if empty(a) {
                *self = (**b).clone();
            } else if empty(b) {
                *self = (**a).clone();
            }
        } else if let DockNode::Tabs { panels, active } = self {
            panels.retain(|x| *x != p);
            *active = (*active).min(panels.len().saturating_sub(1));
        } else if let DockNode::Stack { entries } = self {
            entries.retain(|e| e.panel != p);
        }
    }
    /// Add a panel as a tab next to `near` (or into the first group).
    pub fn open_near(&mut self, p: PanelKind, near: PanelKind) {
        if self.contains(p) {
            self.activate(p);
            return;
        }
        fn add(n: &mut DockNode, p: PanelKind, near: PanelKind) -> bool {
            match n {
                DockNode::Split { a, b, .. } => add(a, p, near) || add(b, p, near),
                DockNode::Tabs { panels, active } => {
                    if panels.contains(&near) {
                        panels.push(p);
                        *active = panels.len() - 1;
                        true
                    } else {
                        false
                    }
                }
                DockNode::Stack { entries } => match entries.iter().position(|e| e.panel == near) {
                    Some(i) => {
                        entries.insert(i + 1, StackEntry { panel: p, open: true, height: None });
                        true
                    }
                    None => false,
                },
            }
        }
        if !add(self, p, near) && !add(self, p, PanelKind::EffectsPresets) {
            add(self, p, PanelKind::Project);
        }
    }
}

/// Where a dragged panel tab lands on a group: as a tab (center) or split off to one side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Zone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl Zone {
    pub fn from_name(s: &str) -> Option<Zone> {
        Some(match s.to_ascii_lowercase().as_str() {
            "center" | "tab" => Zone::Center,
            "left" => Zone::Left,
            "right" => Zone::Right,
            "top" => Zone::Top,
            "bottom" => Zone::Bottom,
            _ => return None,
        })
    }
}

/// The drop zone for a pointer at `pos` over a group `rect` (with a `tab_h` tab strip): the tab
/// strip and the middle of the group are Center; within a quarter of the size from an edge,
/// the nearest edge.
pub fn drop_zone(rect: Rect, tab_h: f32, pos: egui::Pos2) -> Zone {
    if pos.y < rect.min.y + tab_h {
        return Zone::Center;
    }
    let fx = ((pos.x - rect.min.x) / rect.width().max(1.0)).clamp(0.0, 1.0);
    let fy = ((pos.y - rect.min.y) / rect.height().max(1.0)).clamp(0.0, 1.0);
    let d = [(fx, Zone::Left), (1.0 - fx, Zone::Right), (fy, Zone::Top), (1.0 - fy, Zone::Bottom)];
    let (m, z) = d.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap_or((1.0, Zone::Center));
    if m > 0.25 { Zone::Center } else { z }
}

/// The part of a group `rect` a drop into `zone` would occupy (the AE-style highlight).
pub fn zone_rect(rect: Rect, zone: Zone) -> Rect {
    let c = rect.center();
    match zone {
        Zone::Center => rect,
        Zone::Left => Rect::from_min_max(rect.min, pos2(c.x, rect.max.y)),
        Zone::Right => Rect::from_min_max(pos2(c.x, rect.min.y), rect.max),
        Zone::Top => Rect::from_min_max(rect.min, pos2(rect.max.x, c.y)),
        Zone::Bottom => Rect::from_min_max(pos2(rect.min.x, c.y), rect.max),
    }
}

/// A panel group floating over the dock (undocked).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Floating {
    pub panels: Vec<PanelKind>,
    pub active: usize,
    /// [x, y, w, h] in points.
    pub rect: [f32; 4],
}

impl DockNode {
    /// Insert panel `p` relative to the group that holds `anchor`: as a tab next to it
    /// (Center) or in a new group split off to one side. `p` is removed from its old place
    /// first. Returns false (tree unchanged) if `anchor` isn't docked or is `p` itself alone.
    pub fn dock_panel(&mut self, p: PanelKind, anchor: PanelKind, zone: Zone) -> bool {
        if p == anchor || !self.contains(anchor) {
            return false;
        }
        let before = self.clone();
        self.close(p);
        fn rec(n: &mut DockNode, p: PanelKind, anchor: PanelKind, zone: Zone) -> bool {
            let here = match n {
                DockNode::Split { a, b, .. } => return rec(a, p, anchor, zone) || rec(b, p, anchor, zone),
                DockNode::Tabs { panels, .. } => panels.contains(&anchor),
                DockNode::Stack { entries } => entries.iter().any(|e| e.panel == anchor),
            };
            if !here {
                return false;
            }
            match (zone, &mut *n) {
                (Zone::Center, DockNode::Tabs { panels, active }) => {
                    let i = panels.iter().position(|x| *x == anchor).map_or(panels.len(), |i| i + 1);
                    panels.insert(i, p);
                    *active = i;
                }
                (Zone::Center, DockNode::Stack { entries }) => {
                    let i = entries.iter().position(|e| e.panel == anchor).map_or(entries.len(), |i| i + 1);
                    entries.insert(i, StackEntry { panel: p, open: true, height: None });
                }
                (z, node) => {
                    let new = tabs(&[p], 0);
                    let old = std::mem::replace(node, tabs(&[], 0));
                    let vertical = matches!(z, Zone::Top | Zone::Bottom);
                    let (a, b) = if matches!(z, Zone::Left | Zone::Top) { (new, old) } else { (old, new) };
                    *node = DockNode::Split { vertical, size: SplitSize::Ratio(0.5), a: Box::new(a), b: Box::new(b) };
                }
            }
            true
        }
        if rec(self, p, anchor, zone) {
            true
        } else {
            *self = before;
            false
        }
    }

    /// Path of the group containing `p` (as in [`Group::path`]; stacks add `sN`).
    pub fn path_of(&self, p: PanelKind) -> Option<String> {
        fn rec(n: &DockNode, p: PanelKind, path: &str) -> Option<String> {
            match n {
                DockNode::Split { a, b, .. } => rec(a, p, &format!("{path}a")).or_else(|| rec(b, p, &format!("{path}b"))),
                DockNode::Tabs { panels, .. } => panels.contains(&p).then(|| path.to_string()),
                DockNode::Stack { entries } => entries.iter().position(|e| e.panel == p).map(|i| format!("{path}s{i}")),
            }
        }
        rec(self, p, "")
    }

    /// Number of panels in the tree.
    pub fn panel_count(&self) -> usize {
        let mut v = vec![];
        self.panels(&mut v);
        v.len()
    }
}

/// A full layout: the docked tree plus floating panels (what a workspace saves).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub root: DockNode,
    #[serde(default)]
    pub floating: Vec<Floating>,
}

impl Layout {
    /// Undock `p` into a floating group at `rect` (no-op if it is the last docked panel).
    pub fn float(&mut self, p: PanelKind, rect: [f32; 4]) -> bool {
        if self.root.contains(p) {
            if self.root.panel_count() <= 1 {
                return false;
            }
            self.root.close(p);
        } else if !self.unfloat(p) {
            return false;
        }
        self.floating.push(Floating { panels: vec![p], active: 0, rect });
        true
    }

    /// Remove `p` from the floating groups (dropping emptied groups). True if it was floating.
    pub fn unfloat(&mut self, p: PanelKind) -> bool {
        let mut found = false;
        for f in &mut self.floating {
            if let Some(i) = f.panels.iter().position(|x| *x == p) {
                f.panels.remove(i);
                f.active = f.active.min(f.panels.len().saturating_sub(1));
                found = true;
            }
        }
        self.floating.retain(|f| !f.panels.is_empty());
        found
    }

    /// Dock `p` (docked or floating) next to `anchor`; `anchor` may be floating too (then Center
    /// adds a tab to that floating group).
    pub fn dock(&mut self, p: PanelKind, anchor: PanelKind, zone: Zone) -> bool {
        if p == anchor {
            return false;
        }
        if self.floating.iter().any(|f| f.panels.contains(&anchor)) {
            if zone != Zone::Center {
                return false;
            }
            if self.root.contains(p) {
                if self.root.panel_count() <= 1 {
                    return false;
                }
                self.root.close(p);
            } else {
                self.unfloat(p);
            }
            let Some(f) = self.floating.iter_mut().find(|f| f.panels.contains(&anchor)) else { return false };
            f.panels.push(p);
            f.active = f.panels.len() - 1;
            return true;
        }
        if !self.root.contains(anchor) {
            return false;
        }
        // A floating panel isn't in the tree, so dock_panel's removal is a no-op for it.
        self.unfloat(p);
        self.root.dock_panel(p, anchor, zone)
    }

    pub fn contains(&self, p: PanelKind) -> bool {
        self.root.contains(p) || self.floating.iter().any(|f| f.panels.contains(&p))
    }
}

/// One laid-out tab group.
pub struct Group {
    pub path: String,
    pub rect: Rect,
    pub content: Rect,
    pub panels: Vec<PanelKind>,
    pub active: usize,
    /// In a [`DockNode::Stack`]: whether the panel is expanded.
    pub stacked: Option<bool>,
}

/// Actions produced by interacting with the dock chrome.
#[derive(Debug, Clone, PartialEq)]
pub enum DockAction {
    Activate(PanelKind),
    /// Expand/collapse a stacked panel.
    ToggleStacked(PanelKind),
    Focus(PanelKind),
    Close(PanelKind),
    PanelMenu(PanelKind, egui::Pos2),
    /// A tab started being dragged (re-dock or undock).
    BeginDrag(PanelKind),
}

/// Lay out the tree into group rects (with `gap` gutters) and handle gutter dragging.
pub fn layout(ui: &mut egui::Ui, node: &mut DockNode, rect: Rect, t: &Tokens, path: &str, out: &mut Vec<Group>, reg: &mut crate::automation::Registry) {
    match node {
        DockNode::Tabs { panels, active } => {
            let tab_h = if panels.len() == 1 && panels[0].compact() { 8.0 } else { t.tab_h };
            let content = Rect::from_min_max(pos2(rect.min.x, rect.min.y + tab_h), rect.max);
            out.push(Group { path: path.to_string(), rect, content, panels: panels.clone(), active: *active, stacked: None });
        }
        DockNode::Stack { entries } => {
            // Headers for every entry; fixed-height panels next; flexible ones share the rest.
            let head = t.tab_h;
            let g = t.gap;
            let n = entries.len() as f32;
            let fixed: f32 = entries.iter().filter(|e| e.open).filter_map(|e| e.height).sum();
            let flex = entries.iter().filter(|e| e.open && e.height.is_none()).count().max(1) as f32;
            let spare = (rect.height() - n * head - (n - 1.0).max(0.0) * g - fixed).max(0.0);
            let mut y = rect.min.y;
            for (i, e) in entries.iter().enumerate() {
                let body = if !e.open { 0.0 } else { e.height.unwrap_or(spare / flex) };
                let r = Rect::from_min_max(pos2(rect.min.x, y), pos2(rect.max.x, (y + head + body).min(rect.max.y)));
                let content = Rect::from_min_max(pos2(r.min.x, r.min.y + head), r.max);
                out.push(Group { path: format!("{path}s{i}"), rect: r, content, panels: vec![e.panel], active: 0, stacked: Some(e.open) });
                y = r.max.y + g;
            }
        }
        DockNode::Split { vertical, size, a, b } => {
            let g = t.gap;
            let total = if *vertical { rect.height() } else { rect.width() };
            let avail = (total - g).max(0.0);
            let first = match *size {
                SplitSize::Ratio(r) => avail * r,
                SplitSize::FixedA(px) => px.min(avail - 20.0),
                SplitSize::FixedB(px) => avail - px.min(avail - 20.0),
            }
            .clamp(20.0_f32.min(avail), (avail - 20.0).max(0.0));
            let (ra, gutter, rb) = if *vertical {
                (
                    Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + first)),
                    Rect::from_min_max(pos2(rect.min.x, rect.min.y + first), pos2(rect.max.x, rect.min.y + first + g)),
                    Rect::from_min_max(pos2(rect.min.x, rect.min.y + first + g), rect.max),
                )
            } else {
                (
                    Rect::from_min_max(rect.min, pos2(rect.min.x + first, rect.max.y)),
                    Rect::from_min_max(pos2(rect.min.x + first, rect.min.y), pos2(rect.min.x + first + g, rect.max.y)),
                    Rect::from_min_max(pos2(rect.min.x + first + g, rect.min.y), rect.max),
                )
            };
            // gutter drag (a slightly larger hit area than the visible gap)
            let hit = gutter.expand2(if *vertical { vec2(0.0, 3.0) } else { vec2(3.0, 0.0) });
            let id = egui::Id::new(("dock-gutter", path.to_string()));
            let resp = ui.interact(hit, id, Sense::drag());
            reg.add(&format!("dock.gutter.{path}"), hit, "gutter");
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(if *vertical { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
            }
            if resp.dragged() {
                let d = if *vertical { resp.drag_delta().y } else { resp.drag_delta().x };
                let nf = (first + d).clamp(40.0, (avail - 40.0).max(40.0));
                *size = match *size {
                    SplitSize::Ratio(_) => SplitSize::Ratio(nf / avail.max(1.0)),
                    SplitSize::FixedA(_) => SplitSize::FixedA(nf),
                    SplitSize::FixedB(_) => SplitSize::FixedB(avail - nf),
                };
            }
            if resp.dragged() || resp.hovered() {
                ui.painter().rect_filled(gutter, 0.0, t.focus.gamma_multiply(if resp.dragged() { 0.9 } else { 0.4 }));
            }
            layout(ui, a, ra, t, &format!("{path}a"), out, reg);
            layout(ui, b, rb, t, &format!("{path}b"), out, reg);
        }
    }
}

/// Draw a group's frame + tab strip. Returns actions (tab clicks, panel menu, focus).
/// `title` gives a tab's label, which may name the comp or layer it shows (After Effects style).
pub fn draw_group_chrome(
    ui: &mut egui::Ui,
    g: &Group,
    focused: PanelKind,
    t: &Tokens,
    reg: &mut crate::automation::Registry,
    title: &dyn Fn(PanelKind) -> String,
) -> Vec<DockAction> {
    let mut actions = Vec::new();
    let painter = ui.painter().clone();
    painter.rect_filled(g.rect, t.radius, t.panel_bg);
    let active_panel = g.panels.get(g.active).copied();
    let compact = g.panels.len() == 1 && g.panels[0].compact();
    if compact {
        // grip dots
        let c = pos2(g.rect.center().x, g.rect.min.y + 4.0);
        for dx in [-4.0, 0.0, 4.0] {
            painter.circle_filled(c + vec2(dx, 0.0), 1.0, t.text_faint);
        }
    } else {
        let strip = Rect::from_min_size(g.rect.min, vec2(g.rect.width(), t.tab_h));
        if t.gradients {
            // Settings ▸ Appearance ▸ Use Gradients.
            crate::theme::gradient_rect(&painter, strip.shrink2(vec2(t.radius, 0.0)), t.grad_top(t.panel_bg), t.panel_bg);
        }
        let mut x = strip.min.x + 12.0;
        let text_y = strip.min.y + 16.0;
        for (i, p) in g.panels.iter().enumerate() {
            // A collapsed stacked panel's header is plain text (no underline or panel menu).
            let is_active = i == g.active && g.stacked != Some(false);
            let label = title(*p);
            let galley = painter.layout_no_wrap(label.clone(), Tokens::ui(12.0), if is_active { t.tab_text_active } else { t.tab_text });
            let menu_w = if is_active { 20.0 } else { 0.0 };
            let w = galley.size().x + 16.0 + menu_w;
            if x + w > strip.max.x - 20.0 && i > g.active {
                let r = Rect::from_min_size(pos2(strip.max.x - 22.0, strip.min.y + 6.0), vec2(18.0, 20.0));
                let resp = ui.interact(r, egui::Id::new(("tab-overflow", g.path.clone())), Sense::click());
                icons::paint(&painter, r.shrink(4.0).translate(vec2(-2.0, 0.0)), Icon::ChevronRight, t.tab_text);
                icons::paint(&painter, r.shrink(4.0).translate(vec2(2.0, 0.0)), Icon::ChevronRight, t.tab_text);
                if resp.clicked() {
                    let next = g.panels[(g.active + 1) % g.panels.len()];
                    actions.push(DockAction::Activate(next));
                }
                break;
            }
            let tab = Rect::from_min_size(pos2(x, strip.min.y), vec2(w, t.tab_h));
            let resp = ui.interact(tab, egui::Id::new(("tab", g.path.clone(), i)), Sense::click_and_drag());
            if resp.drag_started() {
                actions.push(DockAction::BeginDrag(*p));
            }
            reg.add(&format!("panel.tab.{}", p.id()), tab, &label);
            let label_x = tab.min.x + 8.0;
            let label_w = galley.size().x;
            let col = if is_active || resp.hovered() { t.tab_text_active } else { t.tab_text };
            painter.galley_with_override_text_color(pos2(label_x, text_y - galley.size().y / 2.0), galley, col);
            if is_active {
                let mr = Rect::from_center_size(pos2(label_x + label_w + 12.0, text_y), vec2(12.0, 10.0));
                let mresp = ui.interact(mr.expand(3.0), egui::Id::new(("tab-menu", g.path.clone())), Sense::click());
                reg.add(&format!("panel.menu.{}", p.id()), mr, "panel menu");
                let mc = if mresp.hovered() { t.tab_text_active } else { t.tab_text };
                for dy in [-3.5, 0.0, 3.5] {
                    painter.line_segment([pos2(mr.min.x, mr.center().y + dy), pos2(mr.max.x, mr.center().y + dy)], Stroke::new(1.5, mc));
                }
                let uy = strip.min.y + 23.0;
                painter.line_segment([pos2(label_x, uy), pos2(mr.max.x, uy)], Stroke::new(1.0, t.tab_text_active));
                if mresp.clicked() {
                    actions.push(DockAction::PanelMenu(*p, mr.left_bottom()));
                }
            }
            if resp.clicked() {
                if g.stacked.is_some() {
                    actions.push(DockAction::ToggleStacked(*p));
                } else {
                    actions.push(DockAction::Activate(*p));
                }
                actions.push(DockAction::Focus(*p));
            }
            if resp.middle_clicked() {
                actions.push(DockAction::Close(*p));
            }
            x += w + 8.0;
        }
    }
    // focus outline
    if active_panel == Some(focused) {
        painter.rect_stroke(g.rect, 0.0, Stroke::new(1.0, t.focus), StrokeKind::Inside);
    }
    // clicking anywhere in the panel focuses it
    if let Some(p) = active_panel
        && ui.rect_contains_pointer(g.rect)
        && ui.input(|i| i.pointer.any_pressed())
    {
        actions.push(DockAction::Focus(p));
    }
    actions
}

/// Placeholder body for panels that are not implemented yet.
pub fn placeholder(ui: &mut egui::Ui, rect: Rect, t: &Tokens, text: &str) {
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, Tokens::ui(12.0), t.text_faint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacked_panels_toggle_open_close() {
        use PanelKind::*;
        let mut d = stack(&[(Preview, true, Some(46.0)), (Properties, true, None), (Align, false, None)]);
        assert!(d.is_visible(Properties) && !d.is_visible(Align));
        assert!(d.toggle_stacked(Align));
        assert!(d.is_visible(Align));
        assert!(d.toggle_stacked(Align) && !d.is_visible(Align));
        // Showing a collapsed panel expands it; opening a new panel near one inserts after it.
        assert!(d.activate(Align) && d.is_visible(Align));
        d.open_near(Info, Properties);
        let mut v = vec![];
        d.panels(&mut v);
        assert_eq!(v, [Preview, Properties, Info, Align]);
        d.close(Preview);
        assert!(!d.contains(Preview));
        assert!(!d.toggle_stacked(Timeline));
    }

    #[test]
    fn default_workspace_has_ae_right_column_stack() {
        let d = workspace("Default");
        assert!(d.is_visible(PanelKind::Properties) && d.is_visible(PanelKind::Preview));
        assert!(d.contains(PanelKind::EffectsPresets) && !d.is_visible(PanelKind::EffectsPresets));
    }

    #[test]
    fn workspaces_contain_core_panels() {
        for w in WORKSPACES {
            let d = workspace(w);
            let floating: Vec<PanelKind> = workspace_floating(w).into_iter().flat_map(|f| f.panels).collect();
            assert!(d.contains(PanelKind::Timeline) || floating.contains(&PanelKind::Timeline), "{w}");
            assert!(d.contains(PanelKind::Composition) || d.contains(PanelKind::Layer), "{w}");
            // A panel is in one place only.
            let mut all = vec![];
            d.panels(&mut all);
            all.extend(floating);
            let n = all.len();
            all.sort_by_key(|p| p.id());
            all.dedup();
            assert_eq!(all.len(), n, "{w} has a panel twice");
        }
    }

    #[test]
    fn after_effects_workspaces_exist() {
        // Window ▸ Workspace in After Effects 2026, less Libraries (an Adobe service).
        for w in [
            "Default",
            "Review",
            "Learn",
            "Small Screen",
            "Standard",
            "All Panels",
            "Animation",
            "Essential Graphics",
            "Color",
            "Effects",
            "Minimal",
            "Motion Tracking",
            "Paint",
            "Text",
            "Undocked Panels",
        ] {
            assert!(WORKSPACES.contains(&w), "{w}");
        }
        assert!(!WORKSPACES.contains(&"Libraries"));
        // Layouts differ from Default (except Learn, which is Default plus the Home screen).
        let def = workspace("Default");
        for w in WORKSPACES.iter().filter(|w| !matches!(**w, "Default" | "Learn")) {
            assert_ne!(workspace(w), def, "{w} falls back to Default");
        }
        assert!(workspace("Essential Graphics").contains(PanelKind::Properties));
        assert!(workspace("Color").contains(PanelKind::EffectControls));
        assert!(!workspace_floating("Undocked Panels").is_empty());
        // Every built-in workspace is in Window ▸ Workspace.
        let menu: Vec<String> = effectcraft_engine::menus::entries()
            .into_iter()
            .filter(|(_, e)| e.command == "window.workspace")
            .filter_map(|(_, e)| e.params.get("name").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        for w in WORKSPACES {
            assert!(menu.iter().any(|m| m == w), "{w} missing from Window > Workspace");
        }
    }

    #[test]
    fn drop_zone_math() {
        let r = Rect::from_min_size(pos2(100.0, 100.0), vec2(400.0, 300.0));
        let tab = 28.0;
        assert_eq!(drop_zone(r, tab, pos2(300.0, 110.0)), Zone::Center, "tab strip");
        assert_eq!(drop_zone(r, tab, r.center()), Zone::Center);
        assert_eq!(drop_zone(r, tab, pos2(120.0, 250.0)), Zone::Left);
        assert_eq!(drop_zone(r, tab, pos2(490.0, 250.0)), Zone::Right);
        assert_eq!(drop_zone(r, tab, pos2(300.0, 140.0)), Zone::Top);
        assert_eq!(drop_zone(r, tab, pos2(300.0, 390.0)), Zone::Bottom);
        // Corners go to the nearer edge.
        assert_eq!(drop_zone(r, tab, pos2(105.0, 395.0)), Zone::Left);
        assert_eq!(zone_rect(r, Zone::Right), Rect::from_min_max(pos2(300.0, 100.0), r.max));
        assert_eq!(zone_rect(r, Zone::Top).height(), 150.0);
        assert_eq!(zone_rect(r, Zone::Center), r);
    }

    #[test]
    fn dock_panel_splits_and_inserts_tabs() {
        use PanelKind::*;
        let mut d = workspace("Standard");
        let n = d.panel_count();
        // Tab next to an anchor.
        assert!(d.dock_panel(Info, Timeline, Zone::Center));
        assert_eq!(d.panel_count(), n);
        let p = d.path_of(Timeline).unwrap();
        assert_eq!(d.path_of(Info).unwrap(), p);
        assert!(d.is_visible(Info));
        // Split to the right of the Composition group: a new single-tab group beside it.
        assert!(d.dock_panel(Info, Composition, Zone::Right));
        let comp_path = d.path_of(Composition).unwrap();
        let info_path = d.path_of(Info).unwrap();
        assert_eq!(comp_path.len(), info_path.len());
        assert!(comp_path.ends_with('a') && info_path.ends_with('b'));
        assert!(d.is_visible(Info));
        // Top: the new group comes first in a vertical split.
        assert!(d.dock_panel(Audio, Timeline, Zone::Top));
        assert!(d.path_of(Audio).unwrap().ends_with('a'));
        // Dropping a panel on itself or an undocked anchor changes nothing.
        let before = d.clone();
        assert!(!d.dock_panel(Audio, Audio, Zone::Left));
        assert!(!d.dock_panel(Audio, Tracker, Zone::Left));
        assert_eq!(d, before);
        // Into a stack.
        let mut s = workspace("Default");
        assert!(s.dock_panel(Tracker, Properties, Zone::Center));
        assert!(s.path_of(Tracker).unwrap().contains('s'));
        // Moving the only panel of a group collapses its split.
        let mut m = hsplit(SplitSize::Ratio(0.5), tabs(&[Project], 0), tabs(&[Composition], 0));
        assert!(m.dock_panel(Project, Composition, Zone::Center));
        assert_eq!(m, tabs(&[Composition, Project], 1));
    }

    #[test]
    fn floating_layout_round_trips() {
        use PanelKind::*;
        let mut l = Layout { root: workspace("Standard"), floating: vec![] };
        assert!(l.float(Info, [200.0, 150.0, 320.0, 240.0]));
        assert!(!l.root.contains(Info) && l.contains(Info));
        // Tab another panel into the floating group, then dock it back.
        assert!(l.dock(Audio, Info, Zone::Center));
        assert_eq!(l.floating[0].panels, [Info, Audio]);
        assert!(!l.dock(Audio, Info, Zone::Left), "floating groups only take tabs");
        assert!(l.dock(Info, Timeline, Zone::Bottom));
        assert_eq!(l.floating[0].panels, [Audio]);
        assert!(l.root.contains(Info));
        let s = serde_json::to_string(&l).unwrap();
        let back: Layout = serde_json::from_str(&s).unwrap();
        assert_eq!(back, l);
        // A workspace saved before floating panels existed still loads.
        let old: Layout = serde_json::from_str(&format!("{{\"root\": {}}}", serde_json::to_string(&l.root).unwrap())).unwrap();
        assert!(old.floating.is_empty());
        // The last docked panel can't float.
        let mut one = Layout { root: tabs(&[Timeline], 0), floating: vec![] };
        assert!(!one.float(Timeline, [0.0; 4]));
        assert!(l.unfloat(Audio) && l.floating.is_empty());
    }

    #[test]
    fn close_and_open() {
        let mut d = workspace("Default");
        d.close(PanelKind::Paragraph);
        assert!(!d.contains(PanelKind::Paragraph));
        d.open_near(PanelKind::Tracker, PanelKind::EffectsPresets);
        assert!(d.is_visible(PanelKind::Tracker));
        let s = serde_json::to_string(&d).unwrap();
        let back: DockNode = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d);
        assert_eq!(PanelKind::from_name("effects & presets"), Some(PanelKind::EffectsPresets));
    }
}
