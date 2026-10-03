//! Settings (After Effects' Preferences): a versioned serde model, addressed by dotted keys
//! (`general.undoLevels`, `autoSave.intervalMinutes`, `labels.3.name`), plus the page schema the
//! Settings dialog and agents (`prefs.pages`) are built from.
//!
//! Settings are stored as JSON (`prefs.json`) through the session's [`crate::config::ConfigStore`].
//! Loading is forgiving: missing keys take their defaults, unknown keys (written by a newer
//! version) are kept and written back, and older layouts are migrated ([`migrate`]).
//!
//! Every setting in [`pages`] says whether it already changes behaviour (`live`). Settings that
//! don't yet are still shown (After Effects parity of the dialog) and are listed in
//! `docs/preferences.md`; a test keeps that list in step with the schema.

use std::collections::BTreeMap;

use effectcraft_color::Label;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Current settings layout version.
pub const PREFS_VERSION: u32 = 2;
/// Settings file name in the config store.
pub const PREFS_FILE: &str = "prefs.json";

/// Unknown keys inside a page, kept so a newer version's settings survive a round trip.
type Extra = BTreeMap<String, Value>;

macro_rules! page {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $f:ident : $t:ty = $d:expr),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(default, rename_all = "camelCase")]
        pub struct $name {
            $($(#[$fm])* pub $f: $t,)*
            #[serde(flatten)]
            pub extra: Extra,
        }
        impl Default for $name {
            fn default() -> Self {
                $name { $($f: $d,)* extra: Extra::new() }
            }
        }
    };
}

page!(General {
    /// Levels of Undo (1–99).
    undo_levels: u32 = 32,
    /// Path Point and Handle Size (px).
    path_point_size: u32 = 5,
    show_tool_tips: bool = true,
    /// Create New Layers at Composition Start Time (off: at the current time).
    create_layers_at_comp_start: bool = true,
    switches_affect_nested_comps: bool = true,
    /// Default Spatial Interpolation to Linear.
    default_spatial_linear: bool = false,
    preserve_constant_vertex_count: bool = true,
    /// Synchronize Time of All Related Items.
    sync_time_related_items: bool = true,
    expression_pick_whip_compact: bool = true,
    create_split_layers_above: bool = false,
    use_system_color_picker: bool = false,
    /// Number of projects in File ▸ Open Recent.
    recent_items: u32 = 10,
});

page!(Startup {
    show_home_on_launch: bool = true,
    show_home_on_open_project: bool = false,
    /// After a crash, offer to open the most recent auto-save.
    offer_crash_recovery: bool = true,
});

page!(ProjectPrefs {
    /// Open a template project for File ▸ New ▸ New Project.
    use_template: bool = false,
    template_path: String = String::new(),
});

page!(AutoSave {
    enabled: bool = true,
    interval_minutes: u32 = 20,
    max_versions: u32 = 5,
    /// `nextToProject` (an "EffectCraft Auto-Save" folder beside the project) or `custom`.
    location: String = "nextToProject".into(),
    folder: String = String::new(),
    save_on_render_start: bool = false,
});

page!(CompositionPrefs {
    /// Motion path: `all`, `none`, `seconds`, `keyframes`.
    motion_path: String = "keyframes".into(),
    motion_path_seconds: f64 = 15.0,
    motion_path_keyframes: u32 = 15,
    show_rendering_progress: bool = true,
    hardware_accelerate_panels: bool = true,
});

page!(Previews {
    /// Largest downsampling while playing back: `1/2`, `1/4`, `1/8`.
    adaptive_resolution_limit: String = "1/8".into(),
    show_internal_wireframes: bool = false,
    cache_frames_when_idle: bool = false,
    fast_previews: bool = false,
    /// `faster` or `moreAccurate`.
    zoom_quality: String = "moreAccurate".into(),
    show_gpu_info: bool = false,
    /// The monitor's colour space for View ▸ Use Display Color Management: `srgb` or `p3`.
    display_profile: String = "srgb".into(),
});

page!(Appearance {
    /// `dark`, `darker` or `light`.
    theme: String = "dark".into(),
    /// User interface brightness, -1 (darker) … 1 (lighter).
    brightness: f64 = 0.0,
    use_label_color_for_handles: bool = true,
    use_label_color_for_tabs: bool = true,
    cycle_mask_colors: bool = true,
    use_gradients: bool = true,
    /// macOS: draw the menu bar inside the window instead of the system menu bar.
    in_window_menu_bar_mac: bool = false,
});

page!(Grids {
    grid_color: String = "#7a9cff".into(),
    grid_spacing: f64 = 100.0,
    grid_subdivisions: u32 = 4,
    /// `lines`, `dashed` or `dots`.
    grid_style: String = "lines".into(),
    proportional_horizontal: u32 = 4,
    proportional_vertical: u32 = 4,
    guide_color: String = "#3cc8f0".into(),
    guide_style: String = "lines".into(),
    action_safe: f64 = 10.0,
    title_safe: f64 = 20.0,
});

page!(TypePrefs {
    /// `latin` or `southAsian` (South Asian and Middle Eastern).
    text_engine: String = "latin".into(),
    font_preview: bool = true,
    recent_fonts: u32 = 10,
    font_names_in_english: bool = false,
});

page!(Import {
    /// Still footage duration: `compLength` or `seconds`.
    still_footage: String = "compLength".into(),
    still_seconds: f64 = 5.0,
    sequence_fps: f64 = 30.0,
    report_missing_frames: bool = true,
    /// Interpret unlabeled alpha as: `ask`, `guess`, `ignore`, `straight`, `premultiplied`.
    unlabeled_alpha: String = "ask".into(),
    /// Default drag import as: `footage`, `comp`, `compLayerSizes`.
    drag_import_as: String = "footage".into(),
});

page!(Export {
    default_output_folder: String = String::new(),
    segment_sequences: bool = false,
    segment_sequence_files: u32 = 700,
    segment_movies: bool = false,
    segment_movie_mb: u32 = 1024,
    append_bits_to_name: bool = false,
});

page!(Audio {
    /// Output device name (empty = the system default).
    output_device: String = String::new(),
    /// Output mapping: device channel for the left / right output (1-based).
    output_left: u32 = 1,
    output_right: u32 = 2,
    preview_sample_rate: u32 = 48_000,
});

page!(Disk {
    disk_cache_enabled: bool = true,
    disk_cache_max_gb: u32 = 100,
    disk_cache_folder: String = String::new(),
    media_cache_folder: String = String::new(),
    conformed_media_folder: String = String::new(),
});

page!(Memory {
    ram_reserved_gb: u32 = 6,
    /// Processed-layer cache budget (MB).
    layer_cache_mb: u32 = 1024,
    /// Decoded footage frame cache budget (MB).
    media_cache_mb: u32 = 1024,
    /// RAM preview (viewer frame) cache budget (MB).
    preview_cache_mb: u32 = 3072,
    reduce_cache_when_low: bool = true,
});

page!(Video {
    enable_output: bool = false,
    device: String = String::new(),
    output_during_playback: bool = true,
    mirror_on_monitor: bool = true,
    disable_when_background: bool = false,
});

page!(ThreeD {
    /// Default 3D renderer for new compositions: `classic` or `advanced`.
    default_renderer: String = "classic".into(),
    show_reference_axes: bool = true,
    extended_viewer: bool = false,
    realtime_shadows: bool = true,
});

page!(Scripting {
    allow_scripts_write_files: bool = false,
    warn_executing_files: bool = true,
    enable_js_debugger: bool = false,
    editor_font_size: u32 = 13,
    syntax_highlighting: bool = true,
    line_numbers: bool = true,
    auto_complete: bool = true,
    bracket_matching: bool = true,
    word_wrap: bool = false,
    error_banner: bool = true,
});

/// One editable label: name and `#rrggbb` colour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LabelPref {
    pub name: String,
    pub color: String,
}

fn default_labels() -> Vec<LabelPref> {
    Label::ALL.iter().skip(1).map(|l| LabelPref { name: l.name().to_string(), color: hex(l.rgb()) }).collect()
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

/// All settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub version: u32,
    pub general: General,
    pub startup: Startup,
    pub project: ProjectPrefs,
    pub auto_save: AutoSave,
    pub composition: CompositionPrefs,
    pub previews: Previews,
    pub appearance: Appearance,
    pub grids: Grids,
    /// The 16 labels (Red … Dark Green), in order.
    pub labels: Vec<LabelPref>,
    #[serde(rename = "type")]
    pub type_: TypePrefs,
    pub import: Import,
    pub export: Export,
    pub audio: Audio,
    pub disk: Disk,
    pub memory: Memory,
    pub video: Video,
    #[serde(rename = "threeD")]
    pub three_d: ThreeD,
    pub scripting: Scripting,
    /// File ▸ Open Recent, newest first.
    pub recent_projects: Vec<String>,
    /// Top-level keys this version doesn't know (kept for newer versions).
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            version: PREFS_VERSION,
            general: General::default(),
            startup: Startup::default(),
            project: ProjectPrefs::default(),
            auto_save: AutoSave::default(),
            composition: CompositionPrefs::default(),
            previews: Previews::default(),
            appearance: Appearance::default(),
            grids: Grids::default(),
            labels: default_labels(),
            type_: TypePrefs::default(),
            import: Import::default(),
            export: Export::default(),
            audio: Audio::default(),
            disk: Disk::default(),
            memory: Memory::default(),
            video: Video::default(),
            three_d: ThreeD::default(),
            scripting: Scripting::default(),
            recent_projects: vec![],
            extra: Extra::new(),
        }
    }
}

/// Bring an older settings document up to [`PREFS_VERSION`].
///
/// - version 0/1 (early builds) kept a flat object: `undoLevels`, `autoSaveEnabled`,
///   `autoSaveMinutes`, `autoSaveVersions`, `labelNames` (array) and `theme` at the top level.
pub fn migrate(mut v: Value) -> Value {
    let Some(obj) = v.as_object_mut() else { return json!({}) };
    let version = obj.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version < 2 {
        let moved = |from: &str, page: &str, key: &str, obj: &mut Map<String, Value>| {
            if let Some(x) = obj.remove(from) {
                let p = obj.entry(page.to_string()).or_insert_with(|| json!({}));
                if let Some(p) = p.as_object_mut() {
                    p.entry(key.to_string()).or_insert(x);
                }
            }
        };
        moved("undoLevels", "general", "undoLevels", obj);
        moved("autoSaveEnabled", "autoSave", "enabled", obj);
        moved("autoSaveMinutes", "autoSave", "intervalMinutes", obj);
        moved("autoSaveVersions", "autoSave", "maxVersions", obj);
        moved("theme", "appearance", "theme", obj);
        if let Some(Value::Array(names)) = obj.remove("labelNames") {
            let mut labels = default_labels();
            for (l, n) in labels.iter_mut().zip(names) {
                if let Some(n) = n.as_str() {
                    l.name = n.to_string();
                }
            }
            obj.insert("labels".into(), serde_json::to_value(labels).unwrap_or_default());
        }
    }
    obj.insert("version".into(), json!(PREFS_VERSION));
    v
}

impl Prefs {
    /// Parse a settings document (migrating older layouts; unknown keys are kept, bad values
    /// fall back to defaults).
    pub fn from_json(text: &str) -> Prefs {
        let v: Value = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
        let v = migrate(v);
        let mut p: Prefs = match serde_json::from_value(v.clone()) {
            Ok(p) => p,
            // One bad value shouldn't lose everything: take each page that parses.
            Err(_) => {
                let mut base = serde_json::to_value(Prefs::default()).unwrap_or_default();
                if let (Some(b), Some(o)) = (base.as_object_mut(), v.as_object()) {
                    for (k, x) in o {
                        let mut trial = b.clone();
                        trial.insert(k.clone(), x.clone());
                        if serde_json::from_value::<Prefs>(Value::Object(trial)).is_ok() {
                            b.insert(k.clone(), x.clone());
                        }
                    }
                }
                serde_json::from_value(base).unwrap_or_default()
            }
        };
        p.normalize();
        p
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Clamp values into their valid ranges and fill missing labels.
    pub fn normalize(&mut self) {
        self.version = PREFS_VERSION;
        let g = &mut self.general;
        g.undo_levels = g.undo_levels.clamp(1, 99);
        g.path_point_size = g.path_point_size.clamp(3, 20);
        g.recent_items = g.recent_items.clamp(1, 30);
        let a = &mut self.auto_save;
        a.interval_minutes = a.interval_minutes.clamp(1, 240);
        a.max_versions = a.max_versions.clamp(1, 99);
        self.appearance.brightness = self.appearance.brightness.clamp(-1.0, 1.0);
        let defaults = default_labels();
        self.labels.truncate(defaults.len());
        for d in defaults.iter().skip(self.labels.len()) {
            self.labels.push(d.clone());
        }
        for (l, d) in self.labels.iter_mut().zip(&defaults) {
            if parse_hex(&l.color).is_none() {
                l.color = d.color.clone();
            }
        }
        let m = &mut self.memory;
        m.layer_cache_mb = m.layer_cache_mb.clamp(64, 1 << 20);
        m.media_cache_mb = m.media_cache_mb.clamp(64, 1 << 20);
        m.preview_cache_mb = m.preview_cache_mb.clamp(64, 1 << 20);
        self.grids.grid_spacing = self.grids.grid_spacing.clamp(1.0, 10_000.0);
        let n = self.general.recent_items as usize;
        self.recent_projects.truncate(n);
    }

    /// The value at a dotted key (`general.undoLevels`, `labels.3.color`); `""` = everything.
    pub fn get(&self, key: &str) -> Option<Value> {
        let v = serde_json::to_value(self).ok()?;
        if key.is_empty() {
            return Some(v);
        }
        let ptr = format!("/{}", key.replace('.', "/"));
        v.pointer(&ptr).cloned()
    }

    /// Set the value at a dotted key. The key must exist and the value must have its type.
    pub fn set(&mut self, key: &str, value: Value) -> Result<(), String> {
        let mut v = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        let ptr = format!("/{}", key.replace('.', "/"));
        let slot = v.pointer_mut(&ptr).ok_or_else(|| format!("unknown setting `{key}`"))?;
        let value = coerce(slot, value).ok_or_else(|| format!("`{key}` expects a {}", kind_name(slot)))?;
        *slot = value;
        let mut next: Prefs = serde_json::from_value(v).map_err(|e| format!("`{key}`: {e}"))?;
        next.normalize();
        *self = next;
        Ok(())
    }

    /// Reset one page (`general`, `labels`, `autoSave`…) or everything (`None`). Open Recent is
    /// kept.
    pub fn reset(&mut self, page: Option<&str>) -> Result<(), String> {
        let d = Prefs::default();
        let Some(page) = page else {
            *self = Prefs { recent_projects: std::mem::take(&mut self.recent_projects), ..d };
            return Ok(());
        };
        let keys: Vec<&str> = match page {
            "project" => vec!["project", "autoSave"],
            p => vec![section_key(p).ok_or_else(|| format!("unknown settings page `{p}`"))?],
        };
        let dv = serde_json::to_value(&d).map_err(|e| e.to_string())?;
        for k in keys {
            if let Some(x) = dv.get(k) {
                self.set(k, x.clone())?;
            }
        }
        Ok(())
    }

    /// Display name of a label (Labels settings).
    pub fn label_name(&self, l: Label) -> String {
        match label_index(l) {
            Some(i) => self.labels.get(i).map(|p| p.name.clone()).unwrap_or_else(|| l.name().to_string()),
            None => l.name().to_string(),
        }
    }

    /// sRGB colour of a label (Labels settings).
    pub fn label_rgb(&self, l: Label) -> [u8; 3] {
        label_index(l).and_then(|i| self.labels.get(i)).and_then(|p| parse_hex(&p.color)).unwrap_or(l.rgb())
    }

    /// A label by its built-in name or its (renamed) display name.
    pub fn label_from_name(&self, s: &str) -> Option<Label> {
        Label::from_name(s).or_else(|| Label::ALL.iter().skip(1).copied().find(|l| self.label_name(*l).eq_ignore_ascii_case(s)))
    }

    /// Remember a project in File ▸ Open Recent.
    pub fn push_recent(&mut self, path: &str) {
        self.recent_projects.retain(|p| p != path);
        self.recent_projects.insert(0, path.to_string());
        self.recent_projects.truncate(self.general.recent_items as usize);
    }

    /// Layer / media / preview cache budgets in bytes.
    pub fn layer_cache_bytes(&self) -> usize {
        (self.memory.layer_cache_mb as usize) << 20
    }
    pub fn media_cache_bytes(&self) -> usize {
        (self.memory.media_cache_mb as usize) << 20
    }
    pub fn preview_cache_bytes(&self) -> usize {
        (self.memory.preview_cache_mb as usize) << 20
    }

    /// The adaptive resolution limit as a render-scale floor (1/2 → 0.5).
    pub fn adaptive_limit(&self) -> f64 {
        match self.previews.adaptive_resolution_limit.as_str() {
            "1/2" => 0.5,
            "1/4" => 0.25,
            _ => 0.125,
        }
    }
}

fn label_index(l: Label) -> Option<usize> {
    Label::ALL.iter().position(|x| *x == l).and_then(|i| i.checked_sub(1))
}

/// Settings key of a page id (`3d` → `threeD`).
pub fn section_key(page: &str) -> Option<&'static str> {
    Some(match page {
        "general" => "general",
        "startup" => "startup",
        "project" => "project",
        "autoSave" | "autosave" => "autoSave",
        "composition" | "display" => "composition",
        "previews" => "previews",
        "appearance" => "appearance",
        "grids" => "grids",
        "labels" => "labels",
        "type" => "type",
        "import" => "import",
        "export" => "export",
        "audio" => "audio",
        "disk" => "disk",
        "memory" => "memory",
        "video" => "video",
        "3d" | "threeD" => "threeD",
        "scripting" => "scripting",
        _ => return None,
    })
}

/// Settings page id for a page name, including older After Effects page names
/// (Auto-Save → Project, Media & Disk Cache → Disk, Memory & Performance → Memory & CPU…).
pub fn page_id(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase().replace([' ', '&', '-', '_'], "");
    let id = match n.as_str() {
        "general" => "general",
        "startup" | "startuprepair" => "startup",
        "project" | "autosave" | "newproject" => "project",
        "composition" | "display" => "composition",
        "previews" | "preview" => "previews",
        "appearance" => "appearance",
        "grids" | "gridsguides" => "grids",
        "labels" => "labels",
        "type" => "type",
        "import" => "import",
        "export" | "output" => "export",
        "audio" | "audiohardware" | "audiooutputmapping" => "audio",
        "disk" | "mediadiskcache" | "mediacache" => "disk",
        "memory" | "memorycpu" | "memoryperformance" => "memory",
        "video" | "videopreview" => "video",
        "3d" | "threed" => "3d",
        "scripting" | "scriptingexpressions" => "scripting",
        _ => return None,
    };
    Some(id)
}

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
        Value::Null => "value",
    }
}

/// `value` converted to the type of `slot` (numbers from strings, booleans from 0/1…).
fn coerce(slot: &Value, value: Value) -> Option<Value> {
    Some(match (slot, value) {
        (Value::Bool(_), Value::Bool(b)) => json!(b),
        (Value::Bool(_), Value::Number(n)) => json!(n.as_f64()? != 0.0),
        (Value::Bool(_), Value::String(s)) => json!(matches!(s.as_str(), "true" | "on" | "1" | "yes")),
        (Value::Number(cur), v) => {
            let f = match &v {
                Value::Number(n) => n.as_f64()?,
                Value::String(s) => s.trim().parse().ok()?,
                _ => return None,
            };
            if cur.is_u64() || cur.is_i64() { json!(f.round().max(0.0) as u64) } else { json!(f) }
        }
        (Value::String(_), Value::String(s)) => json!(s),
        (Value::String(_), Value::Number(n)) => json!(n.to_string()),
        (Value::Array(_), v @ Value::Array(_)) => v,
        (Value::Object(_), v @ Value::Object(_)) => v,
        _ => return None,
    })
}

// ---------------------------------------------------------------- schema (Settings dialog)

/// What a setting row edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bool,
    /// Integer with range and unit.
    Int(i64, i64, &'static str),
    /// Number with range and unit.
    Float(f64, f64, &'static str),
    /// One of (label, value).
    Choice(&'static [(&'static str, &'static str)]),
    Text,
    /// A folder or file path (text + Choose…).
    Path,
    /// `#rrggbb`.
    Color,
    /// UI brightness slider.
    Slider(f64, f64),
}

/// One row of a settings page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Item {
    Section(&'static str),
    Setting {
        key: &'static str,
        label: &'static str,
        kind: Kind,
        /// Changes behaviour today (false = shown for parity, listed in docs/preferences.md).
        live: bool,
    },
    /// A button running a command (`params` is JSON).
    Button {
        label: &'static str,
        command: &'static str,
        params: &'static str,
    },
    /// The 16 label names and colours.
    Labels,
    /// Output device picker (device list from the host).
    AudioDevices {
        key: &'static str,
    },
    /// Read-only information line.
    Note(&'static str),
}

/// One page of the Settings dialog.
#[derive(Clone, Debug)]
pub struct Page {
    pub id: &'static str,
    pub title: &'static str,
    pub items: Vec<Item>,
}

const fn s(key: &'static str, label: &'static str, kind: Kind, live: bool) -> Item {
    Item::Setting { key, label, kind, live }
}

const B: Kind = Kind::Bool;
const ON_OFF_RES: &[(&str, &str)] = &[("1/2", "1/2"), ("1/4", "1/4"), ("1/8", "1/8")];
const STYLES: &[(&str, &str)] = &[("Lines", "lines"), ("Dashed Lines", "dashed"), ("Dots", "dots")];

/// The Settings pages in After Effects 2026 order.
pub fn pages() -> Vec<Page> {
    use Item::*;
    vec![
        Page {
            id: "general",
            title: "General",
            items: vec![
                s("general.undoLevels", "Levels of Undo", Kind::Int(1, 99, ""), true),
                s("general.pathPointSize", "Path Point and Handle Size", Kind::Int(3, 20, "px"), true),
                s("general.recentItems", "Recent Projects Shown", Kind::Int(1, 30, ""), true),
                Section("Options"),
                s("general.showToolTips", "Show Tool Tips", B, true),
                s("general.createLayersAtCompStart", "Create Layers at Composition Start Time", B, true),
                s("general.switchesAffectNestedComps", "Switches Affect Nested Comps", B, false),
                s("general.defaultSpatialLinear", "Default Spatial Interpolation to Linear", B, true),
                s("general.preserveConstantVertexCount", "Preserve Constant Vertex and Feather Point Count when Editing Masks", B, false),
                s("general.syncTimeRelatedItems", "Synchronize Time of All Related Items", B, false),
                s("general.expressionPickWhipCompact", "Expression Pick Whip Writes Compact English", B, false),
                s("general.createSplitLayersAbove", "Create Split Layers Above Original Layer", B, false),
                s("general.useSystemColorPicker", "Use System Color Picker", B, false),
            ],
        },
        Page {
            id: "startup",
            title: "Startup & Repair",
            items: vec![
                s("startup.showHomeOnLaunch", "Show Home Screen When Launching", B, true),
                s("startup.showHomeOnOpenProject", "Show Home Screen When Opening a Project", B, false),
                s("startup.offerCrashRecovery", "Offer to Open the Latest Auto-Save After a Crash", B, true),
                Section("Repair"),
                Button { label: "Reset Settings", command: "prefs.reset", params: "{}" },
                Button { label: "Reset Keyboard Shortcuts", command: "shortcuts.reset", params: "{}" },
                Button { label: "Empty Disk Cache", command: "edit.purge", params: r#"{"what":"disk"}"# },
            ],
        },
        Page {
            id: "project",
            title: "Project",
            items: vec![
                Section("New Project"),
                s("project.useTemplate", "New Project Loads Template", B, true),
                s("project.templatePath", "Template Project", Kind::Path, true),
                Section("Auto-Save"),
                s("autoSave.enabled", "Automatically Save Projects", B, true),
                s("autoSave.intervalMinutes", "Save Every", Kind::Int(1, 240, "Minutes"), true),
                s("autoSave.maxVersions", "Maximum Project Versions", Kind::Int(1, 99, ""), true),
                s("autoSave.location", "Auto-Save Location", Kind::Choice(&[("Next to Project", "nextToProject"), ("Custom Location", "custom")]), true),
                s("autoSave.folder", "Custom Location", Kind::Path, true),
                s("autoSave.saveOnRenderStart", "Save When Starting Render Queue", B, true),
                Button { label: "Save Now", command: "file.autoSave", params: "{}" },
            ],
        },
        Page {
            id: "composition",
            title: "Composition",
            items: vec![
                Section("Motion Path"),
                s(
                    "composition.motionPath",
                    "Motion Path",
                    Kind::Choice(&[
                        ("All Keyframes", "all"),
                        ("No Motion Path", "none"),
                        ("No More Than (Seconds)", "seconds"),
                        ("No More Than (Keyframes)", "keyframes"),
                    ]),
                    false,
                ),
                s("composition.motionPathSeconds", "Seconds", Kind::Float(0.0, 3600.0, "s"), false),
                s("composition.motionPathKeyframes", "Keyframes", Kind::Int(1, 10_000, ""), false),
                Section("Display"),
                s("composition.showRenderingProgress", "Show Rendering Progress in Info Panel and Flowchart", B, true),
                s("composition.hardwareAcceleratePanels", "Hardware Accelerate Composition, Layer and Footage Panels", B, false),
            ],
        },
        Page {
            id: "previews",
            title: "Previews",
            items: vec![
                s("previews.adaptiveResolutionLimit", "Adaptive Resolution Limit", Kind::Choice(ON_OFF_RES), true),
                s("previews.showInternalWireframes", "Show Internal Wireframes", B, false),
                s("previews.cacheFramesWhenIdle", "Cache Frames When Idle", B, true),
                s("previews.fastPreviews", "Fast Previews (Draft 3D, Faster Effects)", B, true),
                s("previews.zoomQuality", "Viewer Zoom Quality", Kind::Choice(&[("Faster", "faster"), ("More Accurate", "moreAccurate")]), false),
                s("previews.displayProfile", "Display Color Space", Kind::Choice(&[("sRGB", "srgb"), ("Display P3", "p3")]), true),
                Button { label: "GPU Information...", command: "app.gpuInfo", params: "{}" },
            ],
        },
        Page {
            id: "appearance",
            title: "Appearance",
            items: vec![
                s("appearance.theme", "Theme", Kind::Choice(&[("Dark", "dark"), ("Darker", "darker"), ("Light", "light")]), true),
                s("appearance.brightness", "Brightness", Kind::Slider(-1.0, 1.0), true),
                Section("Labels and Colors"),
                s("appearance.useLabelColorForHandles", "Use Label Color for Layer Handles and Paths", B, true),
                s("appearance.useLabelColorForTabs", "Use Label Color for Related Tabs", B, false),
                s("appearance.cycleMaskColors", "Cycle Mask Colors", B, false),
                s("appearance.useGradients", "Use Gradients", B, false),
                Section("Menu Bar"),
                s("appearance.inWindowMenuBarMac", "Use In-Window Menu Bar on macOS", B, true),
            ],
        },
        Page {
            id: "grids",
            title: "Grids & Guides",
            items: vec![
                Section("Grid"),
                s("grids.gridColor", "Color", Kind::Color, true),
                s("grids.gridSpacing", "Gridline Every", Kind::Float(1.0, 10_000.0, "px"), true),
                s("grids.gridSubdivisions", "Subdivisions", Kind::Int(1, 100, ""), true),
                s("grids.gridStyle", "Style", Kind::Choice(STYLES), false),
                Section("Proportional Grid"),
                s("grids.proportionalHorizontal", "Horizontal", Kind::Int(1, 100, ""), false),
                s("grids.proportionalVertical", "Vertical", Kind::Int(1, 100, ""), false),
                Section("Guides"),
                s("grids.guideColor", "Color", Kind::Color, true),
                s("grids.guideStyle", "Style", Kind::Choice(STYLES), false),
                Section("Safe Margins"),
                s("grids.actionSafe", "Action-safe", Kind::Float(0.0, 50.0, "%"), true),
                s("grids.titleSafe", "Title-safe", Kind::Float(0.0, 50.0, "%"), true),
            ],
        },
        Page { id: "labels", title: "Labels", items: vec![Labels] },
        Page {
            id: "type",
            title: "Type",
            items: vec![
                s("type.textEngine", "Text Engine", Kind::Choice(&[("Latin", "latin"), ("South Asian and Middle Eastern", "southAsian")]), false),
                s("type.fontPreview", "Show Font Preview", B, false),
                s("type.recentFonts", "Number of Recent Fonts to Display", Kind::Int(0, 30, ""), false),
                s("type.fontNamesInEnglish", "Show Font Names in English", B, false),
            ],
        },
        Page {
            id: "import",
            title: "Import",
            items: vec![
                s("import.stillFootage", "Still Footage", Kind::Choice(&[("Length of Composition", "compLength"), ("Duration (Seconds)", "seconds")]), true),
                s("import.stillSeconds", "Still Duration", Kind::Float(0.04, 86_400.0, "s"), true),
                s("import.sequenceFps", "Sequence Footage", Kind::Float(1.0, 999.0, "frames per second"), true),
                s("import.reportMissingFrames", "Report Missing Frames", B, false),
                s(
                    "import.unlabeledAlpha",
                    "Interpret Unlabeled Alpha As",
                    Kind::Choice(&[
                        ("Ask User", "ask"),
                        ("Guess", "guess"),
                        ("Ignore", "ignore"),
                        ("Straight", "straight"),
                        ("Premultiplied", "premultiplied"),
                    ]),
                    false,
                ),
                s(
                    "import.dragImportAs",
                    "Default Drag Import As",
                    Kind::Choice(&[("Footage", "footage"), ("Composition", "comp"), ("Composition - Retain Layer Sizes", "compLayerSizes")]),
                    false,
                ),
            ],
        },
        Page {
            id: "export",
            title: "Export",
            items: vec![
                s("export.defaultOutputFolder", "Default Output Folder", Kind::Path, false),
                s("export.segmentSequences", "Segment Sequences", B, false),
                s("export.segmentSequenceFiles", "Files per Segment", Kind::Int(1, 100_000, "files"), false),
                s("export.segmentMovies", "Segment Movie Files", B, false),
                s("export.segmentMovieMb", "Segment Size", Kind::Int(1, 1_000_000, "MB"), false),
                s("export.appendBitsToName", "Append Bit Depth to File Name", B, false),
            ],
        },
        Page {
            id: "audio",
            title: "Audio",
            items: vec![
                Section("Audio Hardware"),
                AudioDevices { key: "audio.outputDevice" },
                s("audio.previewSampleRate", "Preview Sample Rate", Kind::Int(8_000, 192_000, "Hz"), false),
                Section("Audio Output Mapping"),
                s("audio.outputLeft", "Left", Kind::Int(1, 64, "channel"), true),
                s("audio.outputRight", "Right", Kind::Int(1, 64, "channel"), true),
            ],
        },
        Page {
            id: "disk",
            title: "Disk",
            items: vec![
                Section("Disk Cache"),
                s("disk.diskCacheEnabled", "Enable Disk Cache", B, false),
                s("disk.diskCacheMaxGb", "Maximum Disk Cache Size", Kind::Int(1, 100_000, "GB"), false),
                s("disk.diskCacheFolder", "Disk Cache Folder", Kind::Path, false),
                Button { label: "Empty Disk Cache", command: "edit.purge", params: r#"{"what":"disk"}"# },
                Section("Media Cache"),
                s("disk.mediaCacheFolder", "Database and Cache Folder", Kind::Path, false),
                s("disk.conformedMediaFolder", "Conformed Audio Folder", Kind::Path, false),
            ],
        },
        Page {
            id: "memory",
            title: "Memory & CPU",
            items: vec![
                s("memory.ramReservedGb", "RAM Reserved for Other Applications", Kind::Int(0, 1024, "GB"), false),
                Section("Cache Budgets"),
                s("memory.layerCacheMb", "Layer Cache", Kind::Int(64, 1 << 20, "MB"), true),
                s("memory.mediaCacheMb", "Footage Frame Cache", Kind::Int(64, 1 << 20, "MB"), true),
                s("memory.previewCacheMb", "Preview (RAM) Cache", Kind::Int(64, 1 << 20, "MB"), true),
                s("memory.reduceCacheWhenLow", "Reduce Cache Size When System Is Low on Memory", B, false),
            ],
        },
        Page {
            id: "video",
            title: "Video",
            items: vec![
                s("video.enableOutput", "Enable Video Preview Output", B, false),
                s("video.device", "Video Device", Kind::Text, false),
                s("video.outputDuringPlayback", "Video Output During Playback", B, false),
                s("video.mirrorOnMonitor", "Mirror on Computer Monitor", B, false),
                s("video.disableWhenBackground", "Disable Video Output When in Background", B, false),
            ],
        },
        Page {
            id: "3d",
            title: "3D",
            items: vec![
                s("threeD.defaultRenderer", "Default 3D Renderer", Kind::Choice(&[("Classic 3D", "classic"), ("Advanced 3D", "advanced")]), true),
                s("threeD.showReferenceAxes", "Show 3D Reference Axes", B, false),
                s("threeD.extendedViewer", "Extended Viewer", B, false),
                s("threeD.realtimeShadows", "Realtime Shadows in Draft", B, false),
            ],
        },
        Page {
            id: "scripting",
            title: "Scripting & Expressions",
            items: vec![
                Section("Application Scripting"),
                s("scripting.allowScriptsWriteFiles", "Allow Scripts to Write Files and Access Network", B, false),
                s("scripting.warnExecutingFiles", "Warn User When Executing Files", B, false),
                s("scripting.enableJsDebugger", "Enable JavaScript Debugger", B, false),
                Section("Expressions Editor"),
                s("scripting.editorFontSize", "Font Size", Kind::Int(8, 40, "pt"), false),
                s("scripting.syntaxHighlighting", "Syntax Highlighting", B, false),
                s("scripting.lineNumbers", "Line Numbers", B, false),
                s("scripting.autoComplete", "Auto-complete", B, false),
                s("scripting.bracketMatching", "Bracket Matching", B, false),
                s("scripting.wordWrap", "Word Wrap", B, false),
                s("scripting.errorBanner", "Show Expression Error Banner", B, false),
                Note("Expressions use the JavaScript engine."),
            ],
        },
    ]
}

/// Keys of every setting that doesn't change behaviour yet (the TODO list in
/// `docs/preferences.md`).
pub fn todo_keys() -> Vec<&'static str> {
    pages()
        .into_iter()
        .flat_map(|p| p.items)
        .filter_map(|i| match i {
            Item::Setting { key, live: false, .. } => Some(key),
            _ => None,
        })
        .collect()
}

/// The schema as JSON (for `prefs.pages`).
pub fn pages_json() -> Value {
    let kind = |k: &Kind| -> Value {
        match k {
            Kind::Bool => json!({"type": "bool"}),
            Kind::Int(a, b, u) => json!({"type": "int", "min": a, "max": b, "unit": u}),
            Kind::Float(a, b, u) => json!({"type": "number", "min": a, "max": b, "unit": u}),
            Kind::Choice(o) => json!({"type": "choice", "options": o.iter().map(|(l, v)| json!({"label": l, "value": v})).collect::<Vec<_>>()}),
            Kind::Text => json!({"type": "text"}),
            Kind::Path => json!({"type": "path"}),
            Kind::Color => json!({"type": "color"}),
            Kind::Slider(a, b) => json!({"type": "slider", "min": a, "max": b}),
        }
    };
    let pages: Vec<Value> = pages()
        .iter()
        .map(|p| {
            let items: Vec<Value> = p
                .items
                .iter()
                .map(|i| match i {
                    Item::Section(t) => json!({"section": t}),
                    Item::Setting { key, label, kind: k, live } => {
                        let mut o = kind(k);
                        o["key"] = json!(key);
                        o["label"] = json!(label);
                        o["live"] = json!(live);
                        o
                    }
                    Item::Button { label, command, params } => {
                        json!({"button": label, "command": command, "params": serde_json::from_str::<Value>(params).unwrap_or_default()})
                    }
                    Item::Labels => json!({"labels": "labels.N.name / labels.N.color (N = 0..15)"}),
                    Item::AudioDevices { key } => json!({"type": "device", "key": key, "label": "Default Output"}),
                    Item::Note(t) => json!({"note": t}),
                })
                .collect();
            json!({"id": p.id, "title": p.title, "items": items})
        })
        .collect();
    json!(pages)
}
