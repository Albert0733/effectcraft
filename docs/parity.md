# Parity with After Effects

How close EffectCraft is to After Effects 2026, feature by feature, and how much work is left.

## Current status (audit at commit `fa26ad9`, 2 October 2026, late)

| Measure | Value |
|---|---|
| **Feature parity, weighted by tier** (P0 ×3, P1 ×2, P2 ×1) | **≈ 93%** |
| Unweighted | ≈ 91% |
| P0 / P1 / P2 | ≈ 96% / 90% / 65% |
| Features done / partial / missing | 41 / 50 / 1 of 92 (missing: a plug-in API) |
| **Effects** | **298 of 298** After Effects 2026 effects exist (some still simplified) |
| Disabled menu entries left | 26 commands (31 menu items) |
| Remaining work | ≈ 50–55 agent-hours at the pace measured so far (≈ 150 by the original conservative audit scale) |
| **Wall-clock estimate** | **≈ 11–14 hours** with five agents in parallel; ≈ 6–8 hours for 100% of P0 + P1 |

By area: Layers 98%, Output 95%, Compositions 95%, Automation 96%, Paint 95%, Text 95%, Import 96%,
Animation 94%, Masks 94%, Preview 94%, Interface 93%, Shapes 93%, 3D 91%, Audio 90%, Project 90%,
Tracking 88%, Effects 85%, Web 65%.

What is left, in priority order: the web app's depth (threads, storage, audio, non-blocking
renders); stroke taper/wave and multi-segment dashes; variable mask feather points; camera iris/bokeh and focus-link commands; Render Queue field render, crop/resize and templates;
approximated effects (Subspace Warp, Key Cleaner) and Liquify's viewer brush; Advanced 3D motion blur
and blend modes; a few viewer and colour-management menu items; OpenType features and variable
font axes; AI/EPS/PDF import; more codecs; 59 preferences not yet wired; ScriptUI;
Lumetri Scopes, Footage, Media Browser and Metadata panels; Content-Aware Fill; face tracking; and
the "better than After Effects" items (a plug-in API, branching history; GPU particles landed in M12.7).

The sections below are the original audit (morning of 2 October, ≈ 64%) and its updates, kept for
history.

## Summary

| Measure | Value |
|---|---|
| **Feature parity, weighted by tier** (P0 ×3, P1 ×2, P2 ×1) | **≈ 64%** |
| Feature parity, unweighted | ≈ 58% |
| P0 (it isn't After Effects without it) | ≈ 75% |
| P1 (professional daily use) | ≈ 38% |
| P2 (long tail) | ≈ 17% |
| Features done / partial / missing | 18 / 60 / 14 of 92 |
| **Effects** | **298 of 298** After Effects 2026 effects by name (100%, after M9.11, Warp Stabilizer and the 3D Camera Tracker; was 257) |
| Remaining work | ≈ **167 agent-hours** (139 h of features + ≈ 20% for 1:1 polish against After Effects) |
| Wall-clock estimate | ≈ **42–50 hours** with five agents working in parallel and one integrating; ≈ 31–35 h for P0 + P1 only |

"Agent-hours" are hours of one autonomous coding agent (Claude Opus 5.5) working in its own git
worktree with tests. The estimate is calibrated on this repository's own history: Classic 3D
(cameras, lights, shadows, depth of field, views, tools) took one agent about 2.7 h; timeline
depth (graph editor, keyframe dialogs, time remapping, pen and masks) about 2.3 h; 85 effects
about 1.2 h; the After Effects menu bar about 1.3 h; layer styles about 0.6 h; the web build
about 0.8 h. What is left is harder on average (tracking, roto, puppet, GPU, Advanced 3D), and
sequential chains (tracker → mask tracking → Warp Stabilizer → 3D camera tracker; GPU
compositor → GPU effects) limit how much can run in parallel.

Work in progress at the time of the audit, not yet counted: motion tracking, preferences and the
keyboard shortcut editor, auto-save and crash recovery, paint and puppet tools, Lottie import and
export.

## Update after the second wave (same day, commit `e1f7241`)

Motion tracking (1-, 2- and 4-point, stabilize, corner pin), paint (Brush, Clone Stamp, Eraser
with write-on) and the puppet tools, preferences, a keyboard shortcut editor that rebinds,
auto-save and crash recovery, Lottie import and export, Effect Controls widgets (angle dial,
point crosshair, eyedropper, curves and levels editors, effect copy/paste) and 20 more menu
commands landed. Re-scoring only the features they touch:

| Measure | Before | After |
|---|---|---|
| Feature parity, weighted | 63.7% | **≈ 70%** |
| P0 / P1 / P2 | 75% / 38% / 17% | 78% / 56% / 17% |
| Remaining, audit estimate | 167 agent-hours | ≈ 141 agent-hours |

**Calibration.** The audit priced this wave at about 21 agent-hours; five agents delivered it in
about 1.5 hours of wall-clock time (roughly 6 agent-hours of actual work), though some features
landed at 80–90% rather than fully done. So the per-feature estimates above are conservative.
Remaining wall-clock time with five parallel agents is therefore somewhere between **≈ 12–15
hours** at the pace observed so far and **≈ 35–40 hours** if the audit's figures hold. The
hardest remaining systems (Roto Brush, Warp Stabilizer, the 3D camera tracker, Advanced 3D, GPU
rendering, on-canvas text editing) are where the conservative figure is most likely to be right.

## Update at the end of 2 October 2026

A third wave landed: viewer interactions (snapping, rulers, channels, exposure, snapshots, region
of interest, shape pen and pen-tool family, motion-path handles, graph editor transform box),
on-canvas text editing with per-character styles and paragraph settings, render fidelity (8/16/32
bpc, Rec.709/Rec.2020/P3 working spaces and linear blending, frame blending with optical flow,
collapse transformations, slip edits), a GPU compositor with 16 GPU effects, Wiggler/Smoother/
Motion Sketch, the marker dialog, precompose options and the comp navigator, Project panel folder
moves/rename/columns/thumbnails, drag-and-drop docking with floating panels, and the tracking
library groundwork for mask tracking and Warp Stabilizer.

| Measure | Morning | Now |
|---|---|---|
| Feature parity, weighted | 63.7% | **≈ 79%** |
| Unweighted | 57.8% | ≈ 74% |
| P0 / P1 / P2 | 75% / 38% / 17% | 87% / 67% / 21% |
| Remaining (audit-scale agent-hours) | ≈ 167 | ≈ 100 |
| Wall clock, five parallel agents | 42–50 h | **≈ 8–10 h at today's measured pace; ≈ 25 h by the audit's conservative figures** |

What is left is concentrated in large systems: the Warp Stabilizer and mask-tracking UI on top of
the new tracking library, and wasm threads. (The missing effect categories, 3D Channel, Immersive
Video and OCIO, landed in M9.11; the scripting object model in M14.4; Roto Brush and Refine Edge in
M6.7; native macOS menus, the Composition Flowchart, Timeline column/search depth, the Home screen
and every After Effects workspace in the UI-polish wave; PSD/SVG import, WebM/WAV/AIFF output and
the disk cache in the formats wave; the 3D Camera Tracker in M12.6.)

## By area

| Area | Weighted parity | Remaining (agent-hours) | Biggest gaps |
|---|---|---|---|
| Layers | 88% | 4.8 | frame blending, collapse transformations, slip edit |
| Output | 90% | 3.0 | multiple output modules, pre-render (WebM with VP9 alpha + Opus and WAV/AIFF audio-only landed: VP9 is intra-only, Opus CELT-only) |
| Audio | 85% | 0.5 | audio to keyframes |
| Import | 88% | 2.0 | AI/EPS/PDF vector footage, PSD smart objects and 3D layers (PSD as footage/composition/retain layer sizes, SVG footage and Create Shapes from Vector Layer landed) |
| Automation | ≈ 90% | 0.8 | scripting covers the documented core object model (AUT-2, M14.4: `app`, project items, comps, layers, properties and keyframes, text documents, markers, render queue, Script Console, `effectcraft-cli script`, MCP `run_script`); still missing: ScriptUI panels/dialogs, `.jsxbin`, sockets |
| Shapes | ≈ 80% | 1.5 | Lottie can't carry stroke taper/wave (stroke Taper and Wave, Dash 2/Gap 2/Dash 3/Gap 3 and radial-gradient Highlight Length/Angle landed in M13.5; pen tool for shape paths and vertex editing in M6.5) |
| Compositions | ≈ 80% | 3.5 | Mocha-style planar tracks for templates, Essential Graphics' rare controls (font menus, mirrored properties) (the marker dialog, Composition Flowchart, Essential Graphics with master properties, `.ectemplate` templates and Responsive Design — Time landed: CMP-6, CMP-7) |
| Animation | 67% | 10.0 | puppet, Wiggler/Smoother/Motion Sketch (motion-path handles and the graph editor transform box landed in M5.8; keyframe colour labels and Select Keyframe Label Group, Graph Editor snapping to markers / layer ends in M13.5) |
| Text | ≈ 91% | 0.8 | no OpenType feature panel, no extruded strokes, variable-axis animation changes outlines but not advances (vertical Roman / Tate-Chu-Yoko, forced LTR paragraphs, caret on animated and path text, Variable Font Axes and Lottie style runs landed in M13.5; extruded, bevelled text in M7.6; per-character styles, paragraph settings, on-canvas editing and the `sourceText` style API in M9.9–M9.10) |
| Web | 85% | 1.0 | viewer frames and Roto Brush propagation still on the page's thread; GPU effects in the browser (browser storage, Web Audio, Web Worker renders/analyses, WebGPU viewer and offline install landed in M15.2) |
| 3D | 78% | 9.3 | multi-view layouts, Advanced 3D motion blur and blend modes (and iris shapes in Advanced 3D's depth of field), cameras/lights from models; Classic 3D iris-shaped bokeh with highlights, progressive depth of field on tilted layers and the focus-link commands landed in M13.5; Advanced 3D (glTF/OBJ models, primitives, extruded text and shapes, PBR, image-based light, shadow maps, GPU rasteriser) in M7.4–M7.6 |
| Effects | ≈ 80% | 4.0 | GPU versions of the remaining effects (58 run on the GPU since M12.7) and the missing controls listed as partial in [effects.md](effects.md) (every After Effects effect exists since M9.11, M12.5 and M12.6; parameter names, order, twirl-downs, popups, units and defaults were aligned in M9.12) |
| Interface | 74% | 5.0 | a richer Learn area (Timeline outline and Project panel columns scroll horizontally, the Layer Style dialog, ROI resize handles, Pan Behind snapping and 3D Reference Axes landed in M13.5; native macOS menu bar, Timeline columns/search/reveal-add, Home screen with recent projects and all AE workspaces landed; viewer rulers/snapping/channels/snapshots landed in M0.13) |
| Project | ≈ 60% | 6.5 | 8/16/32-bit pipeline and colour management, auto-save, folder moves (proxies and Interpret Footage fields / pixel aspect / alpha guess landed: PRJ-8, PRJ-3) |
| Masks & roto | 74% | 5.0 | Roto Brush's learned (3.0) segmentation model (variable-width mask feather points with the Mask Feather tool landed in M13.5; mask tracking and Mask Interpolation landed in M6.6; Roto Brush & Refine Edge with graph-cut segmentation, flow propagation, edge matting, decontamination and Freeze in M6.7) |
| Preview | 66% | 4.0 | GPU bokeh depth of field, wireframes and Advanced 3D compositing (Classic 3D runs and adjustment layers composite on the GPU since M12.7; persistent disk cache with the blue cache bar landed; region of interest, snapshots, exposure and Fast Previews landed in M0.13) |
| Tracking | ≈ 85% | 2.5 | face tracking, Subspace Warp's mesh warp, lens distortion in the camera solve (Rolling Shutter Repair landed in M9.11; point tracker, mask tracking, Warp Stabilizer and the 3D Camera Tracker landed in M6.x / M12.5 / M12.6) |
| Paint | 0% | 4.0 | Brush, Clone Stamp, Eraser |

## Effects still missing

The full catalogue with GPU / 32-bpc badges and per-effect status is [effects.md](effects.md)
(generated from the registry).

None: the 3D Camera Tracker landed in M12.6. Boris FX
Mocha and Cineware are third-party and not counted.

### Added in M9.11 (39 effects)

| Category | Effects | Notes |
|---|---|---|
| 3D Channel | 3D Channel Extract, Cryptomatte, Depth Matte, Depth of Field, EXtractoR, Fog 3D, ID Matte, IDentifier | Read auxiliary channels: multi-layer OpenEXR (any `layer.channel`, Cryptomatte manifests from the header) or the compositor's own depth / layer-ID / normal / UV / Cryptomatte pass for precomps of 3D comps. |
| Immersive Video | VR Blur, VR Chromatic Aberrations, VR Color Gradients, VR Converter, VR De-Noise, VR Digital Glitch, VR Fractal Noise, VR Glow, VR Plane to Sphere, VR Rotate Sphere, VR Sharpen, VR Sphere to Plane | Seam-aware equirectangular processing (wrap-around, pole continuation, latitude-scaled filters), mono / over-under / side-by-side layouts; VR Converter between equirectangular, cube maps (4:3, 3:2, 6:1), sphere map and 2D. |
| Color Correction | OCIO CDL / Color Space / Display / File / Look Transform, Color Stabilizer | Built-in minimal OCIO-style config (ACES2065-1, ACEScg, ACEScct, sRGB, Rec.709, Rec.2020, Display P3, linear variants, XYZ) from published matrices and transfer functions; `.cube` / `.3dl` / `.csp` LUTs and ASC `.cc` / `.ccc` / `.cdl`. Custom `.ocio` config files are not read yet. |
| Distort | CC Flo Motion, Liquify, Rolling Shutter Repair | Liquify strokes are effect data replayed into a displacement mesh (`liquify.stroke` / `liquify.clear` commands; no interactive viewer brush yet). Rolling Shutter Repair uses the KLT tracker. |
| Blur & Sharpen | Camera-Shake Deblur, CC Radial Blur | Deblur substitutes aligned patches from sharper neighbouring frames. |
| Audio | Compressor, Distortion, Gate | Applied in the mixdown like the other audio effects. |
| Simulation | CC Hair, Particle Playground | Particle Playground: cannon, grid, layer exploder, layer map, gravity, repel, wall, persistent property mapper (no Particle Exploder, text particles or ephemeral mapper yet). |
| Keying | Key Light | The full Keylight 1.2 control set under a generic name ("Keylight" is a vendor trademark); `lookup("Keylight (1.2)")` finds it. |
| Utility | Color Profile Converter | Our colour spaces and ACES; rendering intents (perceptual gamut compression, relative / absolute colorimetric, saturation). |
| Matte | Mocha shape | Mocha's export format is not public: reads a documented JSON shape format instead. |

## Update: M13.5 long-tail polish

Stroke Taper / Wave / multi-segment dashes and the radial gradient highlight (SHP-2);
variable-width mask feather points (MSK-1; mask motion blur verified by tests); the Adaptive
Sample Limit driving per-layer motion-blur samples (LYR-10); Classic 3D depth of field with
the camera's iris shape, rotation, roundness, aspect ratio, diffraction fringe and highlights,
progressive blur on tilted layers, and Link Focus Distance to Point of Interest / to Layer and
Set Focus Distance to Layer (3D-1); Timeline outline and Project panel column scrolling;
keyframe colour labels and Select Keyframe Label Group (ANM-1); vertical Roman text,
Tate-Chu-Yoko, forced LTR paragraphs, the editing caret on animated / path text and Lottie
style runs; the Layer Style dialog (LYR-9); Graph Editor snapping to markers and layer ends
(ANM-4; the reference graph existed); viewer ROI resize handles, Pan Behind snapping and 3D
Reference Axes; Animate Text ▸ Variable Font Axes. Remaining in these rows: Lottie can't carry
taper/wave, Advanced 3D's DOF has no iris shapes, and variable axes don't change advances.

## Highest-value gaps, in order

1. ~~On-canvas text editing and per-character styles~~ (landed: M9.9–M9.10).
2. Effect Controls widgets: angle dial, point crosshair, eyedropper, curves and levels editors.
3. ~~Viewer basics: snapping, rulers, channel view, snapshots, exposure, a drawable region of interest.~~ (M0.13)
4. ~~A GPU (wgpu) compositor, then GPU effects~~ (M12.2: 2D compositing and 16 GPU effects; M12.7:
   Classic 3D runs, adjustment layers, 58 GPU effects and GPU particles).
5. ~~Pen tool for shape paths, shape vertex editing, free transform~~ (M6.5); the Layer viewer.
6. ~~Motion-path handles in the viewer; graph editor transform box and snapping.~~ (M5.8)
7. Point tracking and stabilization (in progress).
8. Frame blending, collapse transformations, slip edit.
9. A real 8/16/32-bit pipeline with linear blending and colour management.
10. ~~Drag-to-dock and floating panels, saved workspaces, native macOS menus.~~
11. Auto-save, crash recovery, recent projects (in progress).
12. ~~Expression gaps: `sampleImage`, `footage()`~~ (landed with data footage, the error bar and the Expression Language menu; the `sourceText` style API landed in M9.9).
13. ~~Lottie, WebM, SVG and PSD import~~ (landed; a disk cache too).
14. Puppet and paint tools (in progress).
15. Preferences and a shortcut editor that can rebind (in progress); real Wiggler, Smoother and Motion Sketch; the marker dialog.

The engine underneath (keyframes, expressions, shape operators, text animators, Classic 3D, all
38 blend modes, the render queue) is deep. Most of what is missing is interactive tooling in the
viewer, settings that are stored but not yet rendered, about 69 menu entries that are still
disabled, and the large systems: tracking, puppet, paint and roto (Advanced 3D landed in M7.4–M7.6).
