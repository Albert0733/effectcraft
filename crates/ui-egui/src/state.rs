//! Frontend view state (not project data). Serde so the control channel can read and set it.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::dock::{DockNode, PanelKind};
use crate::icons::Icon;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tool {
    #[default]
    Selection,
    Hand,
    Zoom,
    Orbit,
    PanCamera,
    Dolly,
    Rotate,
    PanBehind,
    Rectangle,
    RoundedRect,
    Ellipse,
    Polygon,
    Star,
    Pen,
    Type,
    TypeVertical,
    Brush,
    Clone,
    Eraser,
    RotoBrush,
    Puppet,
    PuppetStarch,
    PuppetBend,
    PuppetAdvanced,
    PuppetOverlap,
}

impl Tool {
    /// Toolbar slots: each slot shows its current tool; the slot cycles with its shortcut.
    pub const SLOTS: &'static [&'static [Tool]] = &[
        &[Tool::Selection],
        &[Tool::Hand],
        &[Tool::Zoom],
        &[Tool::Orbit],
        &[Tool::PanCamera],
        &[Tool::Dolly],
        &[Tool::Rotate],
        &[Tool::PanBehind],
        &[Tool::Rectangle, Tool::RoundedRect, Tool::Ellipse, Tool::Polygon, Tool::Star],
        &[Tool::Pen],
        &[Tool::Type, Tool::TypeVertical],
        &[Tool::Brush],
        &[Tool::Clone],
        &[Tool::Eraser],
        &[Tool::RotoBrush],
        &[Tool::Puppet, Tool::PuppetStarch, Tool::PuppetBend, Tool::PuppetAdvanced, Tool::PuppetOverlap],
    ];
    pub const ALL: [Tool; 25] = [
        Tool::Selection,
        Tool::Hand,
        Tool::Zoom,
        Tool::Orbit,
        Tool::PanCamera,
        Tool::Dolly,
        Tool::Rotate,
        Tool::PanBehind,
        Tool::Rectangle,
        Tool::RoundedRect,
        Tool::Ellipse,
        Tool::Polygon,
        Tool::Star,
        Tool::Pen,
        Tool::Type,
        Tool::TypeVertical,
        Tool::Brush,
        Tool::Clone,
        Tool::Eraser,
        Tool::RotoBrush,
        Tool::Puppet,
        Tool::PuppetStarch,
        Tool::PuppetBend,
        Tool::PuppetAdvanced,
        Tool::PuppetOverlap,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Tool::Selection => "Selection Tool",
            Tool::Hand => "Hand Tool",
            Tool::Zoom => "Zoom Tool",
            Tool::Orbit => "Orbit Around Cursor Tool",
            Tool::PanCamera => "Pan Under Cursor Tool",
            Tool::Dolly => "Dolly Towards Cursor Tool",
            Tool::Rotate => "Rotation Tool",
            Tool::PanBehind => "Pan Behind (Anchor Point) Tool",
            Tool::Rectangle => "Rectangle Tool",
            Tool::RoundedRect => "Rounded Rectangle Tool",
            Tool::Ellipse => "Ellipse Tool",
            Tool::Polygon => "Polygon Tool",
            Tool::Star => "Star Tool",
            Tool::Pen => "Pen Tool",
            Tool::Type => "Horizontal Type Tool",
            Tool::TypeVertical => "Vertical Type Tool",
            Tool::Brush => "Brush Tool",
            Tool::Clone => "Clone Stamp Tool",
            Tool::Eraser => "Eraser Tool",
            Tool::RotoBrush => "Roto Brush Tool",
            Tool::Puppet => "Puppet Position Pin Tool",
            Tool::PuppetStarch => "Puppet Starch Pin Tool",
            Tool::PuppetBend => "Puppet Bend Pin Tool",
            Tool::PuppetAdvanced => "Puppet Advanced Pin Tool",
            Tool::PuppetOverlap => "Puppet Overlap Pin Tool",
        }
    }
    pub fn shortcut(self) -> Option<&'static str> {
        match self {
            Tool::Selection => Some("V"),
            Tool::Hand => Some("H"),
            Tool::Zoom => Some("Z"),
            Tool::Orbit => Some("1"),
            Tool::PanCamera => Some("2"),
            Tool::Dolly => Some("3"),
            Tool::Rotate => Some("W"),
            Tool::PanBehind => Some("Y"),
            Tool::Rectangle | Tool::RoundedRect | Tool::Ellipse | Tool::Polygon | Tool::Star => Some("Q"),
            Tool::Pen => Some("G"),
            Tool::Type | Tool::TypeVertical => Some("Cmd+T"),
            Tool::Brush | Tool::Clone | Tool::Eraser => Some("Cmd+B"),
            Tool::RotoBrush => Some("Alt+W"),
            Tool::Puppet | Tool::PuppetStarch | Tool::PuppetBend | Tool::PuppetAdvanced | Tool::PuppetOverlap => Some("Cmd+P"),
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Tool::Selection => Icon::Selection,
            Tool::Hand => Icon::Hand,
            Tool::Zoom => Icon::Zoom,
            Tool::Orbit => Icon::Orbit,
            Tool::PanCamera => Icon::PanCamera,
            Tool::Dolly => Icon::Dolly,
            Tool::Rotate => Icon::Rotate,
            Tool::PanBehind => Icon::PanBehind,
            Tool::Rectangle => Icon::Rectangle,
            Tool::RoundedRect => Icon::RoundedRect,
            Tool::Ellipse => Icon::Ellipse,
            Tool::Polygon => Icon::Polygon,
            Tool::Star => Icon::Star,
            Tool::Pen => Icon::Pen,
            Tool::Type => Icon::Type,
            Tool::TypeVertical => Icon::TypeVertical,
            Tool::Brush => Icon::Brush,
            Tool::Clone => Icon::Clone,
            Tool::Eraser => Icon::Eraser,
            Tool::RotoBrush => Icon::RotoBrush,
            Tool::Puppet | Tool::PuppetStarch | Tool::PuppetBend | Tool::PuppetAdvanced | Tool::PuppetOverlap => Icon::Puppet,
        }
    }
    pub fn from_name(s: &str) -> Option<Tool> {
        let n = s.to_ascii_lowercase().replace([' ', '_', '-'], "");
        Tool::ALL.into_iter().find(|t| format!("{t:?}").to_ascii_lowercase() == n || t.label().to_ascii_lowercase().replace(' ', "").starts_with(&n))
    }
    /// Puppet pin tools, with the pin kind they place (`puppet.addPin` kind).
    pub fn puppet_kind(self) -> Option<&'static str> {
        Some(match self {
            Tool::Puppet => "position",
            Tool::PuppetStarch => "starch",
            Tool::PuppetBend => "bend",
            Tool::PuppetAdvanced => "advanced",
            Tool::PuppetOverlap => "overlap",
            _ => return None,
        })
    }
    /// Paint tools, with their stroke kind (`paint.stroke` kind).
    pub fn paint_kind(self) -> Option<&'static str> {
        Some(match self {
            Tool::Brush => "brush",
            Tool::Clone => "clone",
            Tool::Eraser => "eraser",
            _ => return None,
        })
    }
    pub fn is_shape(self) -> bool {
        matches!(self, Tool::Rectangle | Tool::RoundedRect | Tool::Ellipse | Tool::Polygon | Tool::Star)
    }
}

/// Viewer resolution (Auto follows the magnification).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    #[default]
    Auto,
    Full,
    Half,
    Third,
    Quarter,
}

impl Resolution {
    pub const ALL: [Resolution; 5] = [Resolution::Auto, Resolution::Full, Resolution::Half, Resolution::Third, Resolution::Quarter];
    pub fn label(self) -> &'static str {
        match self {
            Resolution::Auto => "Auto",
            Resolution::Full => "Full",
            Resolution::Half => "Half",
            Resolution::Third => "Third",
            Resolution::Quarter => "Quarter",
        }
    }
    /// Render scale for a viewer magnification (and display pixel density).
    pub fn scale(self, zoom: f32, ppp: f32) -> f64 {
        match self {
            Resolution::Auto => {
                let z = (zoom * ppp) as f64;
                if z >= 0.75 {
                    1.0
                } else if z >= 0.45 {
                    0.5
                } else if z >= 0.3 {
                    1.0 / 3.0
                } else {
                    0.25
                }
            }
            Resolution::Full => 1.0,
            Resolution::Half => 0.5,
            Resolution::Third => 1.0 / 3.0,
            Resolution::Quarter => 0.25,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewerState {
    /// Magnification (1 = 100%); None = fit.
    pub zoom: Option<f32>,
    /// Pan offset in points from the centred position.
    pub pan: [f32; 2],
    pub res: Resolution,
    pub transparency_grid: bool,
    pub show_masks: bool,
    pub safe_margins: bool,
    pub grid: bool,
    pub rulers: bool,
    pub channel: String,
    pub show_layer_controls: bool,
    pub fast_preview: bool,
    /// View ▸ Show Guides / Snap to Guides / Lock Guides / Snap to Grid.
    pub guides: bool,
    pub snap_guides: bool,
    pub lock_guides: bool,
    pub snap_grid: bool,
    /// View ▸ Panel Background Color (`None` = theme default).
    pub pasteboard: Option<[u8; 3]>,
    pub custom_pasteboard: [u8; 3],
}

impl Default for ViewerState {
    fn default() -> Self {
        ViewerState {
            zoom: None,
            pan: [0.0, 0.0],
            res: Resolution::Auto,
            transparency_grid: false,
            show_masks: true,
            safe_margins: false,
            grid: false,
            rulers: false,
            channel: "RGB".into(),
            show_layer_controls: true,
            fast_preview: false,
            guides: true,
            snap_guides: false,
            lock_guides: false,
            snap_grid: false,
            pasteboard: None,
            custom_pasteboard: [0x80, 0x80, 0x80],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelineState {
    /// Visible span start (seconds) and pixels per second; None = fit comp.
    pub start: f64,
    pub pps: Option<f64>,
    pub scroll_y: f32,
    /// Left column area width.
    pub columns_w: f32,
    pub show_modes: bool,
    pub graph_editor: bool,
    pub search: String,
    /// Twirled-open layers and groups (by layer id / group uid).
    pub open_layers: BTreeSet<u64>,
    pub open_groups: BTreeSet<u64>,
    /// "Reveal" filter: only show these property match ids (P/S/R/T/A…) — empty = normal.
    pub reveal: Vec<String>,
    /// Properties / groups (uids) shown by the `props` reveal (Animation ▸ Reveal Properties…).
    #[serde(default)]
    pub reveal_props: BTreeSet<u64>,
    /// Graph Editor: `value` or `speed` graph.
    #[serde(default = "value_graph")]
    pub graph_mode: String,
    /// Show only the selected properties (else every animated property of the selected layers).
    #[serde(default = "yes")]
    pub graph_show_selected: bool,
    /// Auto-zoom the graph height to the visible curves.
    #[serde(default = "yes")]
    pub graph_auto_zoom: bool,
    /// Manual graph value range (when auto-zoom is off).
    #[serde(default)]
    pub graph_range: Option<(f64, f64)>,
    /// Properties whose inline expression editor is collapsed.
    #[serde(default)]
    pub expr_closed: BTreeSet<u64>,
}

fn value_graph() -> String {
    "value".into()
}
fn yes() -> bool {
    true
}

impl Default for TimelineState {
    fn default() -> Self {
        TimelineState {
            start: 0.0,
            pps: None,
            scroll_y: 0.0,
            columns_w: 560.0,
            show_modes: false,
            graph_editor: false,
            search: String::new(),
            open_layers: BTreeSet::new(),
            open_groups: BTreeSet::new(),
            reveal: vec![],
            reveal_props: BTreeSet::new(),
            graph_mode: value_graph(),
            graph_show_selected: true,
            graph_auto_zoom: true,
            graph_range: None,
            expr_closed: BTreeSet::new(),
        }
    }
}

fn default_project_sort() -> String {
    "name".into()
}

/// What an Effect Controls crosshair / eyedropper click in the viewer sets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FxPick {
    /// `point` (point parameter, layer space) or `color` (colour sampled from the frame).
    pub kind: String,
    pub layer: u64,
    pub prop: u64,
    /// Parameter name (for the viewer hint).
    pub name: String,
}

/// Effects & Presets contents-menu options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectsView {
    /// `all`, `32` (32 bpc effects only) or `gpu` (GPU-accelerated only).
    pub depth: String,
    /// Show the "* Animation Presets" folder.
    pub presets: bool,
    /// Flat alphabetical list instead of categories.
    pub alphabetical: bool,
}

impl Default for EffectsView {
    fn default() -> Self {
        EffectsView { depth: "all".into(), presets: true, alphabetical: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiState {
    pub tool: Tool,
    /// Tool shown in each toolbar slot.
    pub slot_tools: Vec<Tool>,
    pub workspace: String,
    pub dock: DockNode,
    /// Saved workspace layouts (Save Changes to this Workspace / Save as New Workspace).
    #[serde(default)]
    pub saved_workspaces: std::collections::BTreeMap<String, DockNode>,
    pub focused: PanelKind,
    pub viewer: ViewerState,
    pub timeline: TimelineState,
    pub theme: crate::theme::ThemeKind,
    pub show_menu_bar: bool,
    pub status: String,
    /// Effects & Presets search text.
    pub effects_search: String,
    pub effects_open: BTreeSet<String>,
    pub project_search: String,
    pub project_open_folders: BTreeSet<u64>,
    /// Project panel sort column (`name`, `type`, `size`, `fps`) and direction, as in AE's
    /// clickable column headers. Folders sort with everything else.
    #[serde(default = "default_project_sort")]
    pub project_sort: String,
    #[serde(default)]
    pub project_sort_desc: bool,
    /// Effect Controls twirl state (group uids that are collapsed).
    pub fx_closed: BTreeSet<u64>,
    /// Slider params whose slider row is twirled open (AE hides sliders by default).
    #[serde(default)]
    pub fx_slider_open: BTreeSet<u64>,
    /// Effect Controls: a point crosshair or colour eyedropper waiting for a viewer click.
    #[serde(default)]
    pub fx_pick: Option<FxPick>,
    /// Curves editor: shown channel (index into RGB/Red/Green/Blue/Alpha) per effect uid.
    #[serde(default)]
    pub fx_curve_channel: BTreeMap<u64, usize>,
    /// Curves editors in pencil (freehand) mode, by effect uid.
    #[serde(default)]
    pub fx_curve_pencil: BTreeSet<u64>,
    /// Levels (Individual Controls) editor: shown channel per effect uid.
    #[serde(default)]
    pub fx_levels_channel: BTreeMap<u64, usize>,
    /// Effects & Presets: favourite effect ids (starred).
    #[serde(default)]
    pub effects_favorites: BTreeSet<String>,
    /// Effects & Presets: recently applied effect ids, most recent first.
    #[serde(default)]
    pub effects_recent: Vec<String>,
    /// Effects & Presets contents-menu view options.
    #[serde(default)]
    pub effects_view: EffectsView,
    /// Shape tool options.
    pub fill_color: [f32; 3],
    pub stroke_color: [f32; 3],
    pub stroke_width: f32,
    pub snapping: bool,
    /// Tool creates shape (true) or mask (false) when a layer is selected.
    pub tool_creates_shape: bool,
    /// Preview panel options.
    pub preview_loop: bool,
    pub preview_cache_first: bool,
    /// Preview panel "Include Audio" (Mute Audio off).
    #[serde(default = "yes")]
    pub preview_audio: bool,
    pub start_screen: bool,
    /// Composition ▸ Preview ▸ Cache Frames When Idle.
    pub cache_when_idle: bool,
    /// Tracker panel ▸ Motion Source chosen without a tracker yet (layer id).
    #[serde(default)]
    pub tracker_source: Option<u64>,
    /// Layer shown in the Layer panel (None = the first selected layer).
    #[serde(default)]
    pub layer_panel: Option<u64>,
    /// Layer panel View: number of effects rendered (None = through the last Paint effect).
    #[serde(default)]
    pub layer_view: Option<usize>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            tool: Tool::Selection,
            slot_tools: Tool::SLOTS.iter().map(|s| s[0]).collect(),
            workspace: "Default".into(),
            dock: crate::dock::workspace("Default"),
            saved_workspaces: Default::default(),
            focused: PanelKind::Composition,
            viewer: ViewerState::default(),
            timeline: TimelineState::default(),
            theme: Default::default(),
            show_menu_bar: true,
            status: String::new(),
            effects_search: String::new(),
            effects_open: BTreeSet::new(),
            project_search: String::new(),
            project_open_folders: BTreeSet::new(),
            project_sort: default_project_sort(),
            project_sort_desc: false,
            fx_closed: BTreeSet::new(),
            fx_slider_open: BTreeSet::new(),
            fx_pick: None,
            fx_curve_channel: BTreeMap::new(),
            fx_curve_pencil: BTreeSet::new(),
            fx_levels_channel: BTreeMap::new(),
            effects_favorites: BTreeSet::new(),
            effects_recent: Vec::new(),
            effects_view: EffectsView::default(),
            fill_color: [0.24, 0.55, 0.96],
            stroke_color: [1.0, 1.0, 1.0],
            stroke_width: 0.0,
            snapping: true,
            tool_creates_shape: true,
            preview_loop: true,
            preview_cache_first: false,
            preview_audio: true,
            start_screen: false,
            cache_when_idle: false,
            tracker_source: None,
            layer_panel: None,
            layer_view: None,
        }
    }
}
