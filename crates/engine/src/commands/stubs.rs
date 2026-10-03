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
        // Mask options not in the mask model yet; pen / vertex editing (timeline milestone).
        // Cameras / lights / materials / 3D views (3D milestone).
        stub!("camera.stereoRig", "Create Stereo 3D Rig", ["Layer", "Camera"], "{}"),
        stub!("camera.orbitNull", "Create Orbit Null", ["Layer", "Camera"], "{}"),
        stub!("camera.fromModel", "Create Cameras from 3D Model", ["Layer", "Camera"], "{}"),
        stub!("light.fromModel", "Create Lights from 3D Model", ["Layer", "Light"], "{}"),
        stub!("light.controlWithCamera", "Control Light with Camera", ["Layer", "Light"], "{}"),
        stub!("light.environmentBackground", "Create Environment Light Background Layer", ["Layer", "Light"], "{}"),
        stub!("view.3d.default", "Default", ["View", "Switch 3D View"], "{}"),
        stub!("view.splitLockedViewer", "Split with New Locked Viewer", ["View"], "{}"),
        // Color management.
        stub!("view.displayColorManagement", "Use Display Color Management", ["View"], "{}"),
        stub!("view.simulateOutput", "Simulate Output", [], "{profile}"),
        // Keyframes / text / tracking.
        stub!("keys.audioToKeyframes", "Convert Audio to Keyframes", ["Animation", "Keyframe Assistant"], "{}"),
        stub!("keys.rpfCameraImport", "RPF Camera Import", ["Animation", "Keyframe Assistant"], "{}"),
    ]
}
