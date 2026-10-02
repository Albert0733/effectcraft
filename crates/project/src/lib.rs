//! The EffectCraft document model.
//!
//! `Project` → items (folders, compositions, footage, solids) → `Comp` → `Layer` → property tree
//! ([`props`]). Compositions sit behind `Arc` so undo snapshots share everything that did not
//! change.

pub mod build;
pub mod props;
pub mod render_queue;
pub mod styles;
pub mod tracking;

use std::collections::BTreeMap;
use std::sync::Arc;

pub use effectcraft_color::ColorSpace;
use effectcraft_color::{BlendMode, Label};
use effectcraft_time::{FrameRate, Tick};
pub use props::{Expression, FeatherFalloff, GroupKind, MaskMode, MaskMotionBlur, Node, ParamUi, PropGroup, Property, Uid, parse_path};
use serde::{Deserialize, Serialize};

pub use effectcraft_keyframe as keyframe;
pub use effectcraft_keyframe::{Keyframe, Value};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ItemId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LayerId(pub u64);

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("no item {0:?}")]
    NoItem(ItemId),
    #[error("item {0:?} is not a composition")]
    NotComp(ItemId),
    #[error("no layer {0:?}")]
    NoLayer(LayerId),
    #[error("no property `{0}`")]
    NoProperty(String),
    #[error("{0}")]
    Invalid(String),
}

/// Project bit depth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BitDepth {
    #[default]
    Bpc8,
    Bpc16,
    Bpc32,
}

impl BitDepth {
    pub fn label(self) -> &'static str {
        match self {
            BitDepth::Bpc8 => "8 bpc",
            BitDepth::Bpc16 => "16 bpc",
            BitDepth::Bpc32 => "32 bpc",
        }
    }
    pub fn next(self) -> BitDepth {
        match self {
            BitDepth::Bpc8 => BitDepth::Bpc16,
            BitDepth::Bpc16 => BitDepth::Bpc32,
            BitDepth::Bpc32 => BitDepth::Bpc8,
        }
    }
    pub fn is_float(self) -> bool {
        self == BitDepth::Bpc32
    }
    /// Quantisation levels per channel of the integer depths (8 bpc: 255; 16 bpc: 32768, After
    /// Effects' "15 + 1" bit range), `None` for 32 bpc float.
    pub fn levels(self) -> Option<f32> {
        match self {
            BitDepth::Bpc8 => Some(255.0),
            BitDepth::Bpc16 => Some(32768.0),
            BitDepth::Bpc32 => None,
        }
    }
    pub fn bits(self) -> u32 {
        match self {
            BitDepth::Bpc8 => 8,
            BitDepth::Bpc16 => 16,
            BitDepth::Bpc32 => 32,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeDisplayStyle {
    #[default]
    Timecode,
    Frames,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub bit_depth: BitDepth,
    /// Linearize Working Space: the working space uses linear light (1.0 gamma) for everything
    /// (sources, effects, blending). Needs a working space.
    pub linearize: bool,
    /// Working Space (colour management). `None` = no colour management.
    #[serde(default)]
    pub working_space: Option<ColorSpace>,
    /// Blend Colors Using 1.0 Gamma: layers blend in linear light (sources and effects stay in
    /// the working space's encoding).
    #[serde(default)]
    pub blend_linear: bool,
    pub time_display: TimeDisplayStyle,
    /// Frame numbering starts at 0 (or 1).
    pub frame_start: i64,
    pub audio_sample_rate: u32,
    /// Video Rendering and Effects ▸ Use: Mercury GPU Acceleration (`true`, the default; used
    /// when a GPU adapter exists) or Mercury Software Only (`false`, the CPU compositor).
    #[serde(default = "yes")]
    pub gpu_acceleration: bool,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        ProjectSettings {
            bit_depth: BitDepth::Bpc8,
            linearize: false,
            working_space: None,
            blend_linear: false,
            time_display: TimeDisplayStyle::Timecode,
            frame_start: 0,
            audio_sample_rate: 48_000,
            gpu_acceleration: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub time: Tick,
    #[serde(default)]
    pub duration: Tick,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub label: Label,
    #[serde(default)]
    pub chapter: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub protected: bool,
    /// Web link frame target (`_blank`, a frame name…).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub frame_target: String,
    /// Flash/video cue point (Event or Navigation) with name and parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cue_point: Option<CuePoint>,
}

/// A marker's cue point (Composition/Layer Marker dialog ▸ Cue Point).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CuePoint {
    pub name: String,
    /// Navigation (true) or Event cue point.
    #[serde(default)]
    pub navigation: bool,
    /// (name, value) parameter pairs.
    #[serde(default)]
    pub params: Vec<(String, String)>,
}

/// 3D renderer of a composition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Renderer {
    #[default]
    Classic3D,
    Advanced3D,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comp {
    pub width: u32,
    pub height: u32,
    pub pixel_aspect: f64,
    pub frame_rate: FrameRate,
    pub duration: Tick,
    /// Start timecode/frame shown for time 0.
    pub display_start: Tick,
    pub background: [f32; 3],
    pub work_area: (Tick, Tick),
    /// Layer #1 first (top of the stack).
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    pub shutter_angle: f64,
    pub shutter_phase: f64,
    pub motion_blur_samples: u32,
    #[serde(default)]
    pub renderer: Renderer,
    /// Comp switches (timeline toolbar).
    #[serde(default)]
    pub hide_shy: bool,
    #[serde(default = "yes")]
    pub enable_motion_blur: bool,
    #[serde(default = "yes")]
    pub enable_frame_blending: bool,
    #[serde(default)]
    pub draft_3d: bool,
    #[serde(default)]
    pub poster_time: Tick,
    /// Global Light for layer styles (Layer ▸ Layer Styles ▸ Blending Options).
    #[serde(default)]
    pub global_light: styles::GlobalLight,
    /// Viewer guides (View ▸ Add Guide…), in comp pixels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<Guide>,
}

/// A viewer guide line: vertical guides sit at an x position, horizontal ones at a y position.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub vertical: bool,
    pub position: f64,
}

fn yes() -> bool {
    true
}

impl Comp {
    pub fn new(width: u32, height: u32, frame_rate: FrameRate, duration: Tick) -> Comp {
        Comp {
            width,
            height,
            pixel_aspect: 1.0,
            frame_rate,
            duration,
            display_start: Tick::ZERO,
            background: [0.0, 0.0, 0.0],
            work_area: (Tick::ZERO, duration),
            layers: vec![],
            markers: vec![],
            shutter_angle: 180.0,
            shutter_phase: -90.0,
            motion_blur_samples: 16,
            renderer: Renderer::Classic3D,
            hide_shy: false,
            enable_motion_blur: true,
            enable_frame_blending: true,
            draft_3d: false,
            poster_time: Tick::ZERO,
            guides: vec![],
            global_light: styles::GlobalLight::default(),
        }
    }
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }
    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }
    /// 1-based layer number (#) of a layer.
    pub fn index_of(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id).map(|i| i + 1)
    }
    pub fn layer_by_name(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }
    pub fn frame_duration(&self) -> Tick {
        self.frame_rate.frame_duration()
    }
    /// Any 3D layer present (camera/lights or a 3D switch).
    pub fn has_3d(&self) -> bool {
        self.layers.iter().any(|l| l.switches.three_d || matches!(l.source, LayerSource::Camera | LayerSource::Light { .. }))
    }
    /// The active camera at `t`: the topmost enabled camera layer active at that time.
    pub fn active_camera(&self, t: Tick) -> Option<&Layer> {
        self.layers.iter().find(|l| matches!(l.source, LayerSource::Camera) && l.switches.video && l.is_active_at(t))
    }
    /// Unique layer name with a numeric suffix (`Shape Layer 2`).
    pub fn unique_layer_name(&self, base: &str) -> String {
        if !self.layers.iter().any(|l| l.name == base) {
            return base.to_string();
        }
        let stem = base.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end();
        (2..).map(|i| format!("{stem} {i}")).find(|n| !self.layers.iter().any(|l| &l.name == n)).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightKind {
    Parallel,
    Spot,
    #[default]
    Point,
    Ambient,
    Environment,
}

impl LightKind {
    pub const ALL: [LightKind; 5] = [LightKind::Parallel, LightKind::Spot, LightKind::Point, LightKind::Ambient, LightKind::Environment];
    pub fn label(self) -> &'static str {
        match self {
            LightKind::Parallel => "Parallel",
            LightKind::Spot => "Spot",
            LightKind::Point => "Point",
            LightKind::Ambient => "Ambient",
            LightKind::Environment => "Environment",
        }
    }
}

/// What a layer shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LayerSource {
    /// Footage item (video, still, sequence, audio).
    Footage {
        item: ItemId,
    },
    /// Nested composition (precomp).
    Comp {
        item: ItemId,
    },
    /// Solid item (solids also back adjustment layers).
    Solid {
        item: ItemId,
    },
    Text,
    Shape,
    Null,
    Camera,
    Light {
        kind: LightKind,
    },
}

impl LayerSource {
    pub fn item(&self) -> Option<ItemId> {
        match self {
            LayerSource::Footage { item } | LayerSource::Comp { item } | LayerSource::Solid { item } => Some(*item),
            _ => None,
        }
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            LayerSource::Footage { .. } => "Footage",
            LayerSource::Comp { .. } => "Composition",
            LayerSource::Solid { .. } => "Solid",
            LayerSource::Text => "Text",
            LayerSource::Shape => "Shape",
            LayerSource::Null => "Null",
            LayerSource::Camera => "Camera",
            LayerSource::Light { .. } => "Light",
        }
    }
    /// Layers with pixels (cameras, lights and nulls have none).
    pub fn is_av(&self) -> bool {
        !matches!(self, LayerSource::Camera | LayerSource::Light { .. } | LayerSource::Null)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    #[default]
    Best,
    Draft,
    Wireframe,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sampling {
    #[default]
    Bilinear,
    Bicubic,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameBlend {
    #[default]
    Off,
    FrameMix,
    PixelMotion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Switches {
    pub video: bool,
    pub audio: bool,
    pub solo: bool,
    pub locked: bool,
    pub shy: bool,
    /// Collapse transformations (precomps) / continuously rasterize (vector layers).
    pub collapse: bool,
    pub quality: Quality,
    pub sampling: Sampling,
    pub effects: bool,
    pub frame_blend: FrameBlend,
    pub motion_blur: bool,
    pub adjustment: bool,
    pub three_d: bool,
    pub guide: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Switches {
            video: true,
            audio: true,
            solo: false,
            locked: false,
            shy: false,
            collapse: false,
            quality: Quality::Best,
            sampling: Sampling::Bilinear,
            effects: true,
            frame_blend: FrameBlend::Off,
            motion_blur: false,
            adjustment: false,
            three_d: false,
            guide: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatteKind {
    #[default]
    Alpha,
    AlphaInverted,
    Luma,
    LumaInverted,
}

impl MatteKind {
    pub const ALL: [MatteKind; 4] = [MatteKind::Alpha, MatteKind::AlphaInverted, MatteKind::Luma, MatteKind::LumaInverted];
    pub fn label(self) -> &'static str {
        match self {
            MatteKind::Alpha => "Alpha Matte",
            MatteKind::AlphaInverted => "Alpha Inverted Matte",
            MatteKind::Luma => "Luma Matte",
            MatteKind::LumaInverted => "Luma Inverted Matte",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackMatte {
    pub layer: LayerId,
    pub kind: MatteKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoOrient {
    #[default]
    Off,
    AlongPath,
    TowardsCamera,
    TowardsPointOfInterest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub source: LayerSource,
    pub label: Label,
    #[serde(default)]
    pub comment: String,
    /// Comp time where the layer's time 0 sits.
    pub start_time: Tick,
    /// Visible span in comp time.
    pub in_point: Tick,
    pub out_point: Tick,
    /// Time stretch in percent (100 = normal, negative = reversed).
    pub stretch: f64,
    pub switches: Switches,
    pub blend_mode: BlendMode,
    #[serde(default)]
    pub preserve_transparency: bool,
    #[serde(default)]
    pub track_matte: Option<TrackMatte>,
    #[serde(default)]
    pub parent: Option<LayerId>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    /// Layer ▸ Markers ▸ Lock Markers.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub markers_locked: bool,
    #[serde(default)]
    pub auto_orient: AutoOrient,
    /// The property tree (Masks, Effects, Transform, Text, Contents, Camera/Light options…).
    pub props: PropGroup,
}

impl Layer {
    pub fn is_active_at(&self, t: Tick) -> bool {
        t >= self.in_point && t < self.out_point
    }
    /// Layer time for a comp time (stretch-aware).
    pub fn layer_time(&self, comp_t: Tick) -> Tick {
        let d = comp_t - self.start_time;
        if (self.stretch - 100.0).abs() < 1e-9 { d } else { Tick((d.0 as f64 * 100.0 / self.stretch) as i64) }
    }
    /// Comp time for a layer time.
    pub fn comp_time(&self, layer_t: Tick) -> Tick {
        if (self.stretch - 100.0).abs() < 1e-9 { self.start_time + layer_t } else { self.start_time + Tick((layer_t.0 as f64 * self.stretch / 100.0) as i64) }
    }
    pub fn transform(&self) -> Option<&PropGroup> {
        self.props.sub("transform")
    }
    pub fn transform_mut(&mut self) -> Option<&mut PropGroup> {
        self.props.sub_mut("transform")
    }
    pub fn effects(&self) -> Option<&PropGroup> {
        self.props.sub("effects")
    }
    pub fn masks(&self) -> Option<&PropGroup> {
        self.props.sub("masks")
    }
    pub fn is_camera(&self) -> bool {
        matches!(self.source, LayerSource::Camera)
    }
    pub fn is_light(&self) -> bool {
        matches!(self.source, LayerSource::Light { .. })
    }
    /// The layer participates in 3D (3D switch, or cameras/lights which are always 3D).
    pub fn is_3d(&self) -> bool {
        self.switches.three_d || self.is_camera() || self.is_light()
    }
    pub fn has_video(&self) -> bool {
        self.source.is_av() && self.switches.video
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FootageKind {
    #[default]
    Video,
    Still,
    Sequence,
    Audio,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlphaMode {
    #[default]
    Straight,
    Premultiplied,
    Ignore,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Footage {
    pub path: String,
    pub kind: FootageKind,
    pub width: u32,
    pub height: u32,
    pub pixel_aspect: f64,
    pub frame_rate: FrameRate,
    /// Native rate before "Conform to frame rate".
    #[serde(default)]
    pub native_rate: Option<FrameRate>,
    pub duration: Tick,
    pub has_video: bool,
    pub has_audio: bool,
    pub alpha: AlphaMode,
    #[serde(default)]
    pub premul_color: [f32; 3],
    #[serde(default = "one")]
    pub loop_count: u32,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub missing: bool,
    /// Image sequence files (when kind = Sequence).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence: Vec<String>,
    /// Colour profile (from the file's metadata, or Interpret Footage). `None` = sRGB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_profile: Option<ColorSpace>,
}

fn one() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Solid {
    pub color: [f32; 3],
    pub width: u32,
    pub height: u32,
    pub pixel_aspect: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ItemKind {
    Folder,
    Comp(Arc<Comp>),
    Footage(Footage),
    Solid(Solid),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    pub name: String,
    pub label: Label,
    #[serde(default)]
    pub comment: String,
    /// Containing folder (None = project root).
    #[serde(default)]
    pub parent: Option<ItemId>,
    pub kind: ItemKind,
}

impl Item {
    pub fn type_name(&self) -> &'static str {
        match &self.kind {
            ItemKind::Folder => "Folder",
            ItemKind::Comp(_) => "Composition",
            ItemKind::Footage(f) => match f.kind {
                FootageKind::Video => "Video",
                FootageKind::Still => "Image",
                FootageKind::Sequence => "Image Sequence",
                FootageKind::Audio => "Audio",
            },
            ItemKind::Solid(_) => "Solid",
        }
    }
    pub fn as_comp(&self) -> Option<&Comp> {
        if let ItemKind::Comp(c) = &self.kind { Some(c) } else { None }
    }
    pub fn is_folder(&self) -> bool {
        matches!(self.kind, ItemKind::Folder)
    }
    /// (width, height) for visual items.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        match &self.kind {
            ItemKind::Comp(c) => Some((c.width, c.height)),
            ItemKind::Footage(f) if f.has_video => Some((f.width, f.height)),
            ItemKind::Solid(s) => Some((s.width, s.height)),
            _ => None,
        }
    }
    pub fn duration(&self) -> Option<Tick> {
        match &self.kind {
            ItemKind::Comp(c) => Some(c.duration),
            ItemKind::Footage(f) if f.kind != FootageKind::Still => Some(f.duration),
            _ => None,
        }
    }
    pub fn frame_rate(&self) -> Option<FrameRate> {
        match &self.kind {
            ItemKind::Comp(c) => Some(c.frame_rate),
            ItemKind::Footage(f) if f.kind == FootageKind::Video || f.kind == FootageKind::Sequence => Some(f.frame_rate),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema: u32,
    pub settings: ProjectSettings,
    pub items: BTreeMap<ItemId, Item>,
    pub next_id: u64,
    /// The Render Queue (Composition ▸ Add to Render Queue).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub render_queue: Vec<render_queue::RenderQueueItem>,
}

impl Default for Project {
    fn default() -> Self {
        Project { schema: SCHEMA_VERSION, settings: ProjectSettings::default(), items: BTreeMap::new(), next_id: 1, render_queue: Vec::new() }
    }
}

impl Project {
    /// Allocate an id (items, layers and property uids share one counter).
    pub fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.items.get(&id)
    }
    pub fn item_mut(&mut self, id: ItemId) -> Option<&mut Item> {
        self.items.get_mut(&id)
    }
    pub fn comp(&self, id: ItemId) -> Option<&Comp> {
        self.items.get(&id)?.as_comp()
    }
    /// Mutable comp (copy-on-write).
    pub fn comp_mut(&mut self, id: ItemId) -> Option<&mut Comp> {
        match &mut self.items.get_mut(&id)?.kind {
            ItemKind::Comp(c) => Some(Arc::make_mut(c)),
            _ => None,
        }
    }
    pub fn add_item(&mut self, name: &str, label: Label, parent: Option<ItemId>, kind: ItemKind) -> ItemId {
        let id = ItemId(self.alloc());
        self.items.insert(id, Item { id, name: name.into(), label, comment: String::new(), parent, kind });
        id
    }
    /// Children of a folder (None = root), folders first then by name.
    pub fn children(&self, folder: Option<ItemId>) -> Vec<&Item> {
        let mut v: Vec<&Item> = self.items.values().filter(|i| i.parent == folder).collect();
        v.sort_by(|a, b| b.is_folder().cmp(&a.is_folder()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        v
    }
    pub fn comps(&self) -> impl Iterator<Item = (&ItemId, &Comp)> {
        self.items.iter().filter_map(|(id, i)| i.as_comp().map(|c| (id, c)))
    }
    pub fn find_by_name(&self, name: &str) -> Option<&Item> {
        self.items.values().find(|i| i.name == name)
    }
    pub fn folder_named(&self, name: &str) -> Option<ItemId> {
        self.items.values().find(|i| i.is_folder() && i.name == name).map(|i| i.id)
    }
    /// Find the comp that contains `layer`.
    pub fn comp_of_layer(&self, layer: LayerId) -> Option<ItemId> {
        self.comps().find(|(_, c)| c.layer(layer).is_some()).map(|(id, _)| *id)
    }
    /// Whether comp `inner` is (transitively) nested inside comp `outer` (to prevent cycles).
    pub fn comp_contains(&self, outer: ItemId, inner: ItemId) -> bool {
        if outer == inner {
            return true;
        }
        let Some(c) = self.comp(outer) else { return false };
        c.layers.iter().any(|l| matches!(l.source, LayerSource::Comp { item } if self.comp_contains(item, inner)))
    }
    /// Make sure `next_id` exceeds every id in the project (after loading or merging).
    pub fn fix_next_id(&mut self) {
        let mut m = self.next_id;
        for (id, it) in &self.items {
            m = m.max(id.0 + 1);
            if let ItemKind::Comp(c) = &it.kind {
                for l in &c.layers {
                    m = m.max(l.id.0 + 1).max(l.props.max_uid() + 1);
                }
            }
        }
        self.next_id = m;
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Result<Project, ProjectError> {
        let mut p: Project = serde_json::from_str(s).map_err(|e| ProjectError::Invalid(e.to_string()))?;
        if p.schema > SCHEMA_VERSION {
            return Err(ProjectError::Invalid(format!("project schema {} is newer than this version supports ({SCHEMA_VERSION})", p.schema)));
        }
        p.fix_next_id();
        Ok(p)
    }
}

#[cfg(test)]
mod tests;
