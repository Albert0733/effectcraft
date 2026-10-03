//! ScriptUI: the windows, dialogs and dockable panels scripts build with After Effects'
//! ScriptUI object model (`new Window("dialog", "Title")`, `win.add("button", undefined, "OK")`,
//! `orientation`, `alignChildren`, `onClick`/`onChange`, `show()`, `close()`,
//! `layout.layout()`…).
//!
//! The scripting engine (`effectcraft-script`) owns the live JavaScript objects; after every
//! script step it publishes a serde description of each open window here ([`ScriptWindow`],
//! a [`Widget`] tree with computed [`Widget::bounds`]). Frontends draw that description (the egui
//! UI renders dialogs, palettes and dockable panels from it) and report what the user does as
//! [`ScriptUiEvent`]s through the `scriptui.*` commands, which hand them to the script's
//! handlers ([`ScriptUi::dispatch`]). Agents use the same commands: list the open script
//! windows, read their widget trees, click buttons and set values.
//!
//! [`layout`] is ScriptUI's automatic layout (orientation, alignChildren / alignment, spacing,
//! margins, preferredSize), so every frontend places widgets the same way.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// What kind of window a script made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowKind {
    /// Modal: the script waits in `show()` until it closes.
    #[default]
    Dialog,
    /// Floating, non-modal.
    Palette,
    /// A non-modal document window.
    Window,
    /// A dockable panel (a script from the ScriptUI Panels folder, opened from the Window menu).
    Panel,
}

/// ScriptUI control types (the `type` passed to `add()`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetKind {
    /// The window itself (the tree's root).
    #[default]
    Window,
    Panel,
    Group,
    Button,
    IconButton,
    StaticText,
    EditText,
    Checkbox,
    RadioButton,
    Slider,
    Scrollbar,
    Progressbar,
    DropDownList,
    ListBox,
    TabbedPanel,
    Tab,
    Image,
    /// A type we don't draw (it still lays out as an empty box).
    #[serde(other)]
    Unknown,
}

impl WidgetKind {
    pub fn is_container(self) -> bool {
        matches!(self, WidgetKind::Window | WidgetKind::Panel | WidgetKind::Group | WidgetKind::TabbedPanel | WidgetKind::Tab)
    }
}

/// One control (or container) of a script window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Widget {
    /// Unique within its window (the window itself is 0).
    pub id: u32,
    #[serde(rename = "type")]
    pub kind: WidgetKind,
    /// `properties.name` (agents can address controls by it).
    pub name: String,
    pub text: String,
    /// Slider / scrollbar / progressbar value.
    pub value: f64,
    pub min: f64,
    pub max: f64,
    /// Checkbox / radio button state.
    pub checked: bool,
    /// Drop-down list / list box items.
    pub items: Vec<String>,
    /// Selected item indices.
    pub selection: Vec<usize>,
    /// `column` (default for windows and panels), `row` (groups) or `stack`.
    pub orientation: String,
    /// `[horizontal, vertical]`: left / center / right / fill, top / center / bottom / fill.
    pub align_children: Vec<String>,
    /// This control's own alignment in its parent (overrides the parent's alignChildren).
    pub alignment: Vec<String>,
    pub spacing: Option<f64>,
    /// `[left, top, right, bottom]`.
    pub margins: Option<[f64; 4]>,
    pub preferred_size: Option<[f64; 2]>,
    /// Bounds set by the script (`[left, top, right, bottom]` relative to the parent).
    pub fixed_bounds: Option<[f64; 4]>,
    /// Laid-out bounds `[x, y, width, height]` relative to the window's content.
    pub bounds: [f64; 4],
    pub enabled: bool,
    pub visible: bool,
    pub help_tip: String,
    pub multiline: bool,
    pub read_only: bool,
    /// Event handlers the script attached (`onClick`, `onChange`, `onChanging`…).
    pub handlers: Vec<String>,
    /// The active tab of a tabbed panel (child index).
    pub active_tab: usize,
    pub children: Vec<Widget>,
}

impl Widget {
    pub fn find(&self, id: u32) -> Option<&Widget> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }
    pub fn find_mut(&mut self, id: u32) -> Option<&mut Widget> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter_mut().find_map(|c| c.find_mut(id))
    }
    /// Every control, depth first.
    pub fn walk<'a>(&'a self, out: &mut Vec<&'a Widget>) {
        out.push(self);
        for c in &self.children {
            c.walk(out);
        }
    }
    /// A control by id (number or `"#12"`), `properties.name` or text.
    pub fn lookup(&self, key: &Value) -> Option<&Widget> {
        let mut all = vec![];
        self.walk(&mut all);
        match key {
            Value::Number(n) => n.as_u64().and_then(|n| self.find(n as u32)),
            Value::String(s) => {
                if let Some(n) = s.strip_prefix('#').and_then(|n| n.parse::<u32>().ok()) {
                    return self.find(n);
                }
                all.iter().find(|w| !w.name.is_empty() && w.name == *s).or_else(|| all.iter().find(|w| w.id != 0 && w.text == *s)).copied()
            }
            _ => None,
        }
    }
}

/// An open script window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScriptWindow {
    /// Unique in the process.
    pub id: u32,
    /// The script context that owns it (its handlers run there).
    pub host: u32,
    pub kind: WindowKind,
    pub title: String,
    /// The script that made it.
    pub script: String,
    /// Shown and not closed.
    pub visible: bool,
    /// The script is waiting in this dialog's `show()`.
    pub modal: bool,
    /// The window and its controls (`root.bounds` is the content size).
    pub root: Widget,
}

/// A user (or agent) action on a script window control.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptUiEvent {
    pub window: u32,
    pub widget: u32,
    /// `click`, `change`, `changing` or `close`.
    pub kind: String,
    /// The new value: text, number, bool, or selected index(es); for `close`, the result.
    pub value: Value,
}

/// Hands an event to the script that owns the window; returns what the handler reported
/// (`{ok, output, error}`) and updates [`Session::script_ui`].
pub type ScriptUiDispatch = fn(&mut Session, &ScriptUiEvent) -> std::result::Result<Value, String>;

/// The open script windows.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScriptUi {
    pub windows: Vec<ScriptWindow>,
    /// Bumped whenever a window changes.
    pub revision: u64,
    /// The scripting engine's event hand-off (set with [`Session::script`]).
    #[serde(skip)]
    pub dispatch: Option<ScriptUiDispatch>,
}

impl ScriptUi {
    pub fn window(&self, id: u32) -> Option<&ScriptWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Replace the windows a script host published (laying them out).
    pub fn publish(&mut self, host: u32, mut windows: Vec<ScriptWindow>) {
        for w in &mut windows {
            w.host = host;
            layout(w);
        }
        windows.retain(|w| w.visible);
        let mut out: Vec<ScriptWindow> = vec![];
        // Keep the order windows first appeared in.
        for old in self.windows.drain(..) {
            if old.host != host {
                out.push(old);
            } else if let Some(i) = windows.iter().position(|w| w.id == old.id) {
                out.push(windows.remove(i));
            }
        }
        out.extend(windows);
        self.windows = out;
        self.revision += 1;
    }
}

// ---------------------------------------------------------------- layout

/// Average character width and line height of the default dialog font (points).
const CHAR_W: f64 = 7.0;
const LINE_H: f64 = 16.0;

fn text_w(s: &str) -> f64 {
    s.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * CHAR_W
}

fn lines(s: &str) -> f64 {
    s.lines().count().max(1) as f64
}

fn orientation(w: &Widget) -> &str {
    match w.orientation.as_str() {
        "" => match w.kind {
            WidgetKind::Group => "row",
            WidgetKind::TabbedPanel => "stack",
            _ => "column",
        },
        o => o,
    }
}

fn margins(w: &Widget) -> [f64; 4] {
    w.margins.unwrap_or(match w.kind {
        WidgetKind::Window => [15.0; 4],
        WidgetKind::Panel => [10.0, 15.0, 10.0, 10.0],
        WidgetKind::TabbedPanel => [0.0, 24.0, 0.0, 0.0],
        WidgetKind::Tab => [10.0; 4],
        _ => [0.0; 4],
    })
}

fn spacing(w: &Widget) -> f64 {
    w.spacing.unwrap_or(10.0)
}

fn align_children(w: &Widget) -> (String, String) {
    let d = match w.kind {
        WidgetKind::Group => ("center", "center"),
        _ => ("center", "top"),
    };
    let a = |i: usize, d: &str| w.align_children.get(i).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| d.to_string());
    // A single value applies to the axis it names.
    if w.align_children.len() == 1 {
        let v = w.align_children[0].clone();
        return match v.as_str() {
            "top" | "bottom" => (d.0.into(), v),
            "fill" if orientation(w) == "row" => (d.0.into(), v),
            _ => (v, d.1.into()),
        };
    }
    (a(0, d.0), a(1, d.1))
}

/// Preferred size of a control (its own content; containers include their children).
pub fn preferred(w: &Widget) -> [f64; 2] {
    let ps = w.preferred_size.unwrap_or([0.0; 2]);
    let content = if w.kind.is_container() {
        let kids: Vec<[f64; 2]> = w.children.iter().filter(|c| c.visible).map(preferred).collect();
        let m = margins(w);
        let sp = spacing(w);
        let n = kids.len() as f64;
        let (cw, ch) = match orientation(w) {
            "row" => (kids.iter().map(|k| k[0]).sum::<f64>() + sp * (n - 1.0).max(0.0), kids.iter().map(|k| k[1]).fold(0.0, f64::max)),
            "stack" => (kids.iter().map(|k| k[0]).fold(0.0, f64::max), kids.iter().map(|k| k[1]).fold(0.0, f64::max)),
            _ => (kids.iter().map(|k| k[0]).fold(0.0, f64::max), kids.iter().map(|k| k[1]).sum::<f64>() + sp * (n - 1.0).max(0.0)),
        };
        let title_w = if matches!(w.kind, WidgetKind::Panel) { text_w(&w.text) + 20.0 } else { 0.0 };
        [(cw + m[0] + m[2]).max(title_w), ch + m[1] + m[3]]
    } else {
        match w.kind {
            WidgetKind::Button | WidgetKind::IconButton => [(text_w(&w.text) + 24.0).max(80.0), 25.0],
            WidgetKind::StaticText => [text_w(&w.text) + 4.0, LINE_H * lines(&w.text)],
            WidgetKind::EditText => {
                if w.multiline {
                    [(text_w(&w.text) + 12.0).max(160.0), (LINE_H * lines(&w.text) + 8.0).max(60.0)]
                } else {
                    [(text_w(&w.text) + 12.0).max(40.0), 22.0]
                }
            }
            WidgetKind::Checkbox | WidgetKind::RadioButton => [text_w(&w.text) + 24.0, 18.0],
            WidgetKind::Slider | WidgetKind::Scrollbar => [100.0, 22.0],
            WidgetKind::Progressbar => [100.0, 10.0],
            WidgetKind::DropDownList => [w.items.iter().map(|i| text_w(i)).fold(0.0, f64::max) + 36.0, 22.0],
            WidgetKind::ListBox => [(w.items.iter().map(|i| text_w(i)).fold(0.0, f64::max) + 24.0).max(80.0), (w.items.len().clamp(3, 10) as f64 * 18.0 + 4.0)],
            _ => [20.0, 20.0],
        }
    };
    [if ps[0] > 0.0 { ps[0] } else { content[0] }, if ps[1] > 0.0 { ps[1] } else { content[1] }]
}

fn place(w: &mut Widget, x: f64, y: f64, width: f64, height: f64) {
    w.bounds = [x, y, width, height];
    if !w.kind.is_container() {
        return;
    }
    let m = margins(w);
    let sp = spacing(w);
    let (ah, av) = align_children(w);
    let inner = [m[0], m[1], (width - m[0] - m[2]).max(0.0), (height - m[1] - m[3]).max(0.0)];
    let orient = orientation(w).to_string();
    let mut cursor = 0.0;
    for c in w.children.iter_mut() {
        if !c.visible {
            c.bounds = [0.0; 4];
            continue;
        }
        if let Some(b) = c.fixed_bounds {
            let (cx, cy, cw, ch) = (x + b[0], y + b[1], b[2] - b[0], b[3] - b[1]);
            place(c, cx, cy, cw, ch);
            continue;
        }
        let pref = preferred(c);
        let h_align = c.alignment.first().filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| ah.clone());
        let v_align = c.alignment.get(1).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| av.clone());
        let along = |a: &str, free: f64| match a {
            "center" => free / 2.0,
            "right" | "bottom" => free,
            _ => 0.0,
        };
        let (cx, cy, cw, ch) = match orient.as_str() {
            "row" => {
                let ch = if v_align == "fill" { inner[3] } else { pref[1] };
                let cy = inner[1] + along(&v_align, inner[3] - ch);
                let r = (inner[0] + cursor, cy, pref[0], ch);
                cursor += pref[0] + sp;
                r
            }
            "stack" => {
                let cw = if h_align == "fill" { inner[2] } else { pref[0] };
                let ch = if v_align == "fill" { inner[3] } else { pref[1] };
                (inner[0] + along(&h_align, inner[2] - cw), inner[1] + along(&v_align, inner[3] - ch), cw, ch)
            }
            _ => {
                let cw = if h_align == "fill" { inner[2] } else { pref[0] };
                let cx = inner[0] + along(&h_align, inner[2] - cw);
                let r = (cx, inner[1] + cursor, cw, pref[1]);
                cursor += pref[1] + sp;
                r
            }
        };
        place(c, x + cx, y + cy, cw, ch);
    }
    // Rows centre their content run as a whole when it is narrower than the row.
    if orient == "row" && (ah == "center" || ah == "right") {
        let used = (cursor - sp).max(0.0);
        let shift = along_shift(&ah, inner[2] - used);
        if shift > 0.0 {
            for c in w.children.iter_mut().filter(|c| c.visible && c.fixed_bounds.is_none()) {
                shift_all(c, shift, 0.0);
            }
        }
    }
    // Columns aligned to the bottom / centre move the whole run.
    if orient == "column" && (av == "center" || av == "bottom") {
        let used = (cursor - sp).max(0.0);
        let shift = along_shift(&av, inner[3] - used);
        if shift > 0.0 {
            for c in w.children.iter_mut().filter(|c| c.visible && c.fixed_bounds.is_none()) {
                shift_all(c, 0.0, shift);
            }
        }
    }
}

fn along_shift(a: &str, free: f64) -> f64 {
    match a {
        "center" => (free / 2.0).max(0.0),
        "right" | "bottom" => free.max(0.0),
        _ => 0.0,
    }
}

fn shift_all(w: &mut Widget, dx: f64, dy: f64) {
    w.bounds[0] += dx;
    w.bounds[1] += dy;
    for c in &mut w.children {
        shift_all(c, dx, dy);
    }
}

/// ScriptUI automatic layout: size the window to its preferred size (or the size the script
/// gave it) and place every control (`bounds` relative to the window's top-left).
pub fn layout(w: &mut ScriptWindow) {
    let pref = preferred(&w.root);
    let (width, height) = match w.root.fixed_bounds {
        Some(b) if b[2] > b[0] && b[3] > b[1] => (b[2] - b[0], b[3] - b[1]),
        _ => (pref[0], pref[1]),
    };
    place(&mut w.root, 0.0, 0.0, width, height);
}

// ---------------------------------------------------------------- commands

fn window_p(s: &Session, p: &Value, cmd: &str) -> Result<u32> {
    let key = p.get("window");
    let w = match key {
        Some(Value::Number(n)) => n.as_u64().map(|n| n as u32).filter(|n| s.script_ui.window(*n).is_some()),
        Some(Value::String(t)) => s.script_ui.windows.iter().find(|w| w.title == *t).map(|w| w.id),
        None if s.script_ui.windows.len() == 1 => s.script_ui.windows.first().map(|w| w.id),
        _ => None,
    };
    w.ok_or_else(|| crate::commands::bad(cmd, "no such script window (give `window`: id or title from scriptui.list)"))
}

fn widget_p(s: &Session, win: u32, p: &Value, cmd: &str) -> Result<u32> {
    let w = s.script_ui.window(win).ok_or_else(|| crate::commands::bad(cmd, "window closed"))?;
    let key = p.get("widget").ok_or_else(|| crate::commands::bad(cmd, "missing `widget` (id, `#id`, properties.name or text)"))?;
    w.root.lookup(key).map(|w| w.id).ok_or_else(|| crate::commands::bad(cmd, format!("no control {key} in window {win}")))
}

fn dispatch(s: &mut Session, ev: ScriptUiEvent) -> Result<Value> {
    let f = s.script_ui.dispatch.ok_or_else(|| EngineError::Other("scripting is not available in this build".into()))?;
    f(s, &ev).map_err(EngineError::Other)
}

/// `scriptui.list`: the open script windows.
pub(crate) fn list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!(
        s.script_ui
            .windows
            .iter()
            .map(|w| json!({"window": w.id, "title": w.title, "kind": w.kind, "script": w.script, "modal": w.modal, "size": [w.root.bounds[2], w.root.bounds[3]]}))
            .collect::<Vec<_>>()
    ))
}

/// `scriptui.get`: one window's control tree.
pub(crate) fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let id = window_p(s, p, "scriptui.get")?;
    Ok(serde_json::to_value(s.script_ui.window(id)).unwrap_or(Value::Null))
}

/// `scriptui.click`: press a button / toggle a checkbox / pick a radio button.
pub(crate) fn click(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "scriptui.click";
    let window = window_p(s, p, c)?;
    let widget = widget_p(s, window, p, c)?;
    let w = s.script_ui.window(window).and_then(|w| w.root.find(widget)).cloned().unwrap_or_default();
    if !w.enabled {
        return Err(crate::commands::bad(c, format!("control {widget} is disabled")));
    }
    dispatch(s, ScriptUiEvent { window, widget, kind: "click".into(), value: Value::Null })
}

/// `scriptui.set`: set a control's value (edit text, slider, checkbox, list selection) as the
/// user would, firing onChanging / onChange.
pub(crate) fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "scriptui.set";
    let window = window_p(s, p, c)?;
    let widget = widget_p(s, window, p, c)?;
    let w = s.script_ui.window(window).and_then(|w| w.root.find(widget)).cloned().unwrap_or_default();
    if !w.enabled {
        return Err(crate::commands::bad(c, format!("control {widget} is disabled")));
    }
    let mut value = p.get("value").cloned().ok_or_else(|| crate::commands::bad(c, "missing `value`"))?;
    // List selections by item text.
    if matches!(w.kind, WidgetKind::DropDownList | WidgetKind::ListBox)
        && let Value::String(t) = &value
    {
        let i = w.items.iter().position(|it| it == t).ok_or_else(|| crate::commands::bad(c, format!("no item `{t}` (items: {:?})", w.items)))?;
        value = json!(i);
    }
    dispatch(s, ScriptUiEvent { window, widget, kind: "change".into(), value })
}

/// `scriptui.close`: close a window (a dialog's `show()` returns `result`, default 2 = Cancel).
pub(crate) fn close(s: &mut Session, p: &Value) -> Result<Value> {
    let window = window_p(s, p, "scriptui.close")?;
    let result = p.get("result").cloned().unwrap_or(json!(2));
    dispatch(s, ScriptUiEvent { window, widget: 0, kind: "close".into(), value: result })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(kind: WidgetKind, text: &str) -> Widget {
        Widget { kind, text: text.into(), enabled: true, visible: true, ..Default::default() }
    }

    #[test]
    fn column_and_row_layout() {
        let mut row = w(WidgetKind::Group, "");
        row.id = 1;
        let mut ok = w(WidgetKind::Button, "OK");
        ok.id = 2;
        let mut cancel = w(WidgetKind::Button, "Cancel");
        cancel.id = 3;
        row.children = vec![ok, cancel];
        let mut label = w(WidgetKind::StaticText, "Name:");
        label.id = 4;
        let mut root = w(WidgetKind::Window, "Dialog");
        root.children = vec![label, row];
        let mut win = ScriptWindow { id: 1, root, visible: true, ..Default::default() };
        layout(&mut win);
        let r = &win.root;
        // Window: 15 px margins, children stacked with 10 px spacing.
        let row = r.find(1).unwrap();
        assert_eq!(row.bounds[2], 170.0, "two 80 px buttons + 10 spacing");
        assert_eq!(r.bounds[2], 200.0);
        assert_eq!(r.find(4).unwrap().bounds[1], 15.0);
        assert_eq!(row.bounds[1], 15.0 + 16.0 + 10.0);
        // Centred (the default alignChildren of windows).
        assert_eq!(row.bounds[0], 15.0);
        let label = r.find(4).unwrap();
        assert!((label.bounds[0] + label.bounds[2] / 2.0 - 100.0).abs() < 1e-9);
        assert_eq!(r.find(3).unwrap().bounds[0], 15.0 + 80.0 + 10.0);
        // alignChildren left / fill.
        win.root.align_children = vec!["fill".into(), "top".into()];
        layout(&mut win);
        assert_eq!(win.root.find(4).unwrap().bounds[2], 170.0);
        win.root.align_children = vec!["left".into()];
        layout(&mut win);
        assert_eq!(win.root.find(4).unwrap().bounds[0], 15.0);
        // preferredSize wins.
        win.root.find_mut(2).unwrap().preferred_size = Some([120.0, 30.0]);
        layout(&mut win);
        assert_eq!(win.root.find(2).unwrap().bounds[2..], [120.0, 30.0]);
        // Lookup by name / text / id.
        win.root.find_mut(2).unwrap().name = "ok".into();
        assert_eq!(win.root.lookup(&json!("ok")).unwrap().id, 2);
        assert_eq!(win.root.lookup(&json!("Cancel")).unwrap().id, 3);
        assert_eq!(win.root.lookup(&json!("#4")).unwrap().id, 4);
        assert_eq!(win.root.lookup(&json!(1)).unwrap().id, 1);
    }

    #[test]
    fn publish_replaces_a_hosts_windows() {
        let mut ui = ScriptUi::default();
        let win = |id: u32, visible: bool| ScriptWindow { id, visible, root: w(WidgetKind::Window, ""), ..Default::default() };
        ui.publish(7, vec![win(1, true), win(2, true)]);
        ui.publish(8, vec![win(3, true)]);
        assert_eq!(ui.windows.iter().map(|w| (w.id, w.host)).collect::<Vec<_>>(), [(1, 7), (2, 7), (3, 8)]);
        ui.publish(7, vec![win(2, true), win(1, false)]);
        assert_eq!(ui.windows.iter().map(|w| w.id).collect::<Vec<_>>(), [2, 3]);
        let j = serde_json::to_value(&ui).unwrap();
        assert_eq!(j["windows"][0]["root"]["type"], "window");
    }
}
