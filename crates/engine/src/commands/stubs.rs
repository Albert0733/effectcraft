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
        // macOS application menu items handled by the OS.
        stub!("app.hideOthers", "Hide Others", [], "{}"),
        stub!("app.showAll", "Show All", [], "{}"),
        // Render queue / export (export milestone).
        stub!("render.addToQueue", "Add to Render Queue", ["Composition"], "Cmd+M", "{comp?}"),
        stub!("render.addOutputModule", "Add Output Module", ["Composition"], "{}"),
        stub!("render.preRender", "Pre-render...", ["Composition"], "{}"),
        stub!("render.saveCurrentPreview", "Save Current Preview...", ["Composition"], "{path}"),
        stub!("file.createProxy", "Create Proxy", [], "{kind: still|movie}"),
        // Proxies (no proxy model yet).
        stub!("file.setProxy", "File...", ["File", "Set Proxy"], "{path}"),
        stub!("file.setProxyNone", "None", ["File", "Set Proxy"], "{}"),
        stub!("file.interpretProxy", "Proxy...", ["File", "Interpret Footage"], "{}"),
        // Text editing in the viewer.
        stub!("edit.pasteTextMatchFormatting", "Paste Text and Match Formatting", ["Edit"], "{}"),
        stub!("edit.pasteTextFormattingOnly", "Paste Text Formatting Only", ["Edit"], "{}"),
        stub!("keys.selectLabelGroup", "Select Keyframe Label Group", [], "{scope}"),
        // Preview audio.
        stub!("playback.audio", "Audio", ["Composition", "Preview"], "{}"),
        // Layer ▸ New (3D / content-aware fill).
        stub!("layer.newContentAwareFill", "Content-Aware Fill Layer...", ["Layer", "New"], "{}"),
        stub!("layer.new3dPrimitive", "3D Primitive", [], "{kind: cube|sphere|plane|torus|cone|cylinder}"),
        // Mask options not in the mask model yet; pen / vertex editing (timeline milestone).
        stub!("layer.mask.motionBlur", "Motion Blur", [], "{mode: sameAsLayer|on|off}"),
        stub!("layer.mask.featherFalloff", "Feather Falloff", [], "{mode: smooth|linear}"),
        stub!("layer.mask.hideLocked", "Hide Locked Masks", ["Layer", "Mask"], "{}"),
        stub!("path.rotoBezier", "RotoBezier", ["Layer", "Mask and Shape Path"], "{}"),
        stub!("path.closed", "Closed", ["Layer", "Mask and Shape Path"], "{}"),
        stub!("path.convertToBezier", "Convert To Bezier Path", ["Layer", "Mask and Shape Path"], "{}"),
        stub!("path.groupShapes", "Group Shapes", ["Layer", "Mask and Shape Path"], "Cmd+G", "{}"),
        stub!("path.ungroupShapes", "Ungroup Shapes", ["Layer", "Mask and Shape Path"], "Cmd+Shift+G", "{}"),
        stub!("path.setFirstVertex", "Set First Vertex", ["Layer", "Mask and Shape Path"], "{}"),
        stub!("path.freeTransform", "Free Transform Points", ["Layer", "Mask and Shape Path"], "{}"),
        // Time (timeline milestone).
        stub!("layer.timeRemap", "Enable Time Remapping", ["Layer", "Time"], "Cmd+Alt+T", "{layers?}"),
        stub!("layer.freezeFrame", "Freeze Frame", ["Layer", "Time"], "{layers?}"),
        stub!("layer.freezeOnLastFrame", "Freeze On Last Frame", ["Layer", "Time"], "{layers?}"),
        stub!("layer.alignVideoToData", "Align Video to Data", ["Layer", "Time"], "{}"),
        stub!("layer.environment", "Environment Layer", ["Layer"], "{}"),
        stub!("layer.updateMarkersFromSource", "Update Markers From Source", ["Layer", "Markers"], "{}"),
        stub!("layer.styles", "Layer Styles", [], "{op?|style?}"),
        stub!("layer.create", "Create", [], "{op}"),
        stub!("layer.autoTrace", "Auto-trace...", ["Layer"], "{}"),
        stub!("layer.sceneEditDetection", "Scene Edit Detection...", ["Layer"], "{}"),
        // Cameras / lights / materials / 3D views (3D milestone).
        stub!("camera.fromView", "Create Camera from 3D View", ["Layer", "Camera"], "{}"),
        stub!("camera.stereoRig", "Create Stereo 3D Rig", ["Layer", "Camera"], "{}"),
        stub!("camera.orbitNull", "Create Orbit Null", ["Layer", "Camera"], "{}"),
        stub!("camera.fromModel", "Create Cameras from 3D Model", ["Layer", "Camera"], "{}"),
        stub!("camera.linkFocusToPoi", "Link Focus Distance to Point of Interest", ["Layer", "Camera"], "{}"),
        stub!("camera.linkFocusToLayer", "Link Focus Distance to Layer", ["Layer", "Camera"], "{}"),
        stub!("camera.setFocusToLayer", "Set Focus Distance to Layer", ["Layer", "Camera"], "{}"),
        stub!("camera.settings", "Camera Settings...", ["Layer", "Camera"], "{}"),
        stub!("view.reset3dView", "Reset 3D View", ["Layer", "Camera"], "{}"),
        stub!("light.fromModel", "Create Lights from 3D Model", ["Layer", "Light"], "{}"),
        stub!("light.controlWithCamera", "Control Light with Camera", ["Layer", "Light"], "{}"),
        stub!("light.environmentBackground", "Create Environment Light Background Layer", ["Layer", "Light"], "{}"),
        stub!("material.revealSource", "Reveal Material Source in Project", ["Layer", "Material"], "{}"),
        stub!("material.reset", "Reset Material", ["Layer", "Material"], "{}"),
        stub!("material.duplicateAssign", "Duplicate and Assign Material", ["Layer", "Material"], "{}"),
        stub!("view.layout", "Switch View Layout", [], "{views: 1|2|4}"),
        stub!("view.shareViewOptions", "Share View Options", ["View", "Switch View Layout"], "{}"),
        stub!("view.3d", "Switch 3D View", [], "{view}"),
        stub!("view.last3dView", "Switch to Last 3D View", ["View"], "Escape", "{}"),
        stub!("view.lookAtSelected", "Look at Selected Layers", ["View"], "Cmd+Alt+Shift+\\", "{}"),
        stub!("view.lookAtAll", "Look at All Layers", ["View"], "{}"),
        stub!("view.splitLockedViewer", "Split with New Locked Viewer", ["View"], "{}"),
        stub!("view.res.custom", "Custom...", ["View", "Resolution"], "{}"),
        // Color management.
        stub!("view.displayColorManagement", "Use Display Color Management", ["View"], "{}"),
        stub!("view.simulateOutput", "Simulate Output", [], "{profile}"),
        // Keyframes / text / tracking.
        stub!("keys.audioToKeyframes", "Convert Audio to Keyframes", ["Animation", "Keyframe Assistant"], "{}"),
        stub!("keys.rpfCameraImport", "RPF Camera Import", ["Animation", "Keyframe Assistant"], "{}"),
        stub!("text.perCharacter3d", "Enable Per-character 3D", ["Animation", "Animate Text"], "{}"),
        stub!("text.animatorLineAnchor", "Line Anchor", ["Animation", "Animate Text"], "{}"),
        stub!("text.animatorCharacterValue", "Character Value", ["Animation", "Animate Text"], "{}"),
        stub!("text.animatorFontAxes", "Variable Font Axes", ["Animation", "Animate Text"], "{}"),
        stub!("text.addWigglySelector", "Wiggly", ["Animation", "Add Text Selector"], "{}"),
        stub!("text.addExpressionSelector", "Expression", ["Animation", "Add Text Selector"], "{}"),
        stub!("prop.separateDimensions", "Separate Dimensions", ["Animation"], "{}"),
        stub!("track.camera", "Track Camera", ["Animation"], "{}"),
        stub!("track.warpStabilizer", "Warp Stabilizer VFX", ["Animation"], "{}"),
        stub!("track.motion", "Track Motion", ["Animation"], "{}"),
        stub!("track.mask", "Track Mask", ["Animation"], "{}"),
        stub!("track.property", "Track this Property", ["Animation"], "{}"),
        // Workspaces and panels that don't exist yet.
        stub!("window.saveWorkspace", "Save Changes to this Workspace", ["Window", "Workspace"], "{}"),
        stub!("window.saveWorkspaceAs", "Save as New Workspace...", ["Window", "Workspace"], "{name}"),
        stub!("window.editWorkspaces", "Edit Workspaces...", ["Window", "Workspace"], "{}"),
        stub!("window.unavailablePanel", "Panel", [], "{panel}"),
    ]
}
