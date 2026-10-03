//! Menu entries whose implementation is owned by another milestone (3D, render queue/export,
//! time remapping, pen/mask-vertex editing, motion tracking, layer styles…) or that have no
//! EffectCraft equivalent yet. They are registered so the menu bar matches After Effects, and are
//! always disabled. When a feature lands, delete its line here and register the real command
//! with the same id (the menu tree in `menus.rs` already points at it).

use super::{CommandSpec, not_yet, not_yet_run};

macro_rules! stub {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: not_yet, run: not_yet_run, journal: true }
    };
    ($id:literal, $label:literal, [$($m:literal),*], $sc:literal, $params:literal) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: Some($sc), params: $params, enabled: not_yet, run: not_yet_run, journal: true }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        stub!("keys.selectLabelGroup", "Select Keyframe Label Group", [], "{scope}"),
        // Layer ▸ New (3D / content-aware fill).
        stub!("layer.newContentAwareFill", "Content-Aware Fill Layer...", ["Layer", "New"], "{}"),
        // Mask options not in the mask model yet; pen / vertex editing (timeline milestone).
        // Time (timeline milestone).
        stub!("layer.alignVideoToData", "Align Video to Data", ["Layer", "Time"], "{}"),
        stub!("layer.autoTrace", "Auto-trace...", ["Layer"], "{}"),
        stub!("layer.sceneEditDetection", "Scene Edit Detection...", ["Layer"], "{}"),
        // Cameras / lights / materials / 3D views (3D milestone).
        stub!("camera.linkFocusToPoi", "Link Focus Distance to Point of Interest", ["Layer", "Camera"], "{}"),
        stub!("camera.linkFocusToLayer", "Link Focus Distance to Layer", ["Layer", "Camera"], "{}"),
        stub!("camera.setFocusToLayer", "Set Focus Distance to Layer", ["Layer", "Camera"], "{}"),
        // Keyframes / text / tracking.
        stub!("keys.audioToKeyframes", "Convert Audio to Keyframes", ["Animation", "Keyframe Assistant"], "{}"),
        stub!("keys.rpfCameraImport", "RPF Camera Import", ["Animation", "Keyframe Assistant"], "{}"),
        stub!("text.animatorFontAxes", "Variable Font Axes", ["Animation", "Animate Text"], "{}"),
        // Workspaces and panels that don't exist yet.
        stub!("window.unavailablePanel", "Panel", [], "{panel}"),
    ]
}
