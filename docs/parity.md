# Parity with After Effects

How close EffectCraft is to After Effects 2026, feature by feature, and how much work is left.

## Current status (audit at commit `978e8d7`, 3 October 2026)

| Measure | Value |
|---|---|
| **Feature parity, weighted by tier** (P0 ×3, P1 ×2, P2 ×1) | **≈ 94%** counting every partial feature as half done; ≈ 98% using per-feature fractions |
| Unweighted | ≈ 94% (half-credit) / 98% (fractions) |
| P0 / P1 / P2 | ≈ 96% / 92% / 85% (half-credit); 99% / 97% / 96% (fractions) |
| Features done / partial / missing | 80 / 12 / 0 of 92 |
| **Effects** | **298 of 298** After Effects 2026 effects exist; 42 still simplified ([effects.md](effects.md)); 58 run on the GPU |
| Disabled menu entries left | 0 (the stub list is empty) |
| Remaining work | ≈ 19–22 agent-hours at the pace measured so far (≈ 58 on the conservative audit scale) |
| **Wall-clock estimate** | **≈ 4–5 hours** with five agents in parallel; ≈ 3 hours for 100% of P0 + P1 |

Partial features: GPU coverage (Advanced 3D, more GPU effects), the web app (no threads for
viewer frames and Roto Brush), Essential Graphics mirrored properties, and the learned-model quality
of Roto Brush and face tracking (both are classical). Since this audit, M9.13 completed all 42
simplified effects, M13.5 added the Learn panel, and M13.8 the Preview panel settings, Align to
Selection / Distribute and Advanced 3D iris DOF, collapsed precomps and extruded strokes. M13.7 added ScriptUI resource strings,
`onDraw` (ScriptUIGraphics draw lists), live `onChanging`, `Socket`, the Essential Graphics
scripting hooks and Font / uniform Scale controls, and the last missing menu items: Save Frame As ▸
Photoshop Layers / ProEXR, Composition ▸ Open in Essential Graphics, File ▸ Watch Folder,
Window ▸ Create Nulls From Paths / VR Comp Editor (built-in panels) and Help ▸ In-App / Online
Tutorials. File ▸ Import ▸ Vanishing Point (.vpe) is present but permanently disabled: the format
has no public specification, so a clean-room implementation is not possible (its tooltip says so).

### Previous audit (commit `fa26ad9`, 2 October 2026, late): ≈ 93%

By area: Layers 98%, Output 95%, Compositions 95%, Automation 96%, Paint 95%, Text 95%, Import 96%,
Animation 94%, Masks 94%, Preview 94%, Interface 93%, Shapes 93%, 3D 91%, Audio 90%, Project 90%,
Tracking 96%, Effects 85%, Web 65%.

What is left, in priority order: the web app's depth (threads, storage, audio, non-blocking
renders); stroke taper/wave and multi-segment dashes; variable mask feather points; camera iris/bokeh and focus-link commands; 
approximated effects (Key Cleaner) and Liquify's viewer brush; JPX / JBIG2 images in
imported PDF/AI files (mesh shadings and CCITT landed in M13.12) and Illustrator procset EPS; and more codecs. Face tracking and a true Subspace Warp
landed in M13.3. ScriptUI (dialogs that block `show()`, palettes, dockable ScriptUI panels, File ▸ Scripts install
and sample scripts), puppet pin recording and the remaining "better than After Effects" items — a
versioned effect plug-in API with sandboxed WebAssembly plug-ins ([plugins.md](plugins.md)),
branching undo history in the History panel, and a bit-identical rendering test across runs and
thread counts — landed in M13.1 (GPU particles landed in M12.7). Render Queue field render, crop/resize, templates and logs, and VP9 inter frames landed in M10.2.

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
| Output | 97% | 0.8 | Render Settings complete (field render + 3:2 pulldown, effects/solo/guide/depth/blending/blur overrides, time sampling, storage overflow), Output Module crop/ROI/resize, alpha modes, post-render actions, PCM formats, templates with defaults, render logs, Notify (M10.2); WebM VP9 key + inter frames with motion search, loop filter and rate control. Left: Opus is CELT-only, Photoshop sequence output, overflow for movies only checks at file creation |
| Audio | 85% | 0.5 | audio to keyframes |
| Import | 97% | 0.5 | JPX / JBIG2 images and predefined non-Identity CJK CMaps inside PDF/AI files, Illustrator EPS relying on Adobe procsets, PSD 3D layers (M13.12: mesh shadings — free-form / lattice Gouraud triangles, Coons and tensor patches — and function-based shadings, CCITT G3 / G4 images, knockout / isolated transparency groups with group alpha and soft masks on the group's result, `/W2` vertical metrics, EPS text with embedded Type 1 / CFF or bundled fonts, and Create Shapes from Vector Layer keeping images as parented footage layers in paint order; M13.6: PDF/AI text with embedded TrueType / CFF / Type 1 / Type 3 fonts and standard-14 fallback to the bundled fonts, Flate / DCT / inline / stencil images with soft masks, Indexed and ICC alternates, luminosity / alpha soft masks, the 16 blend modes, tiling patterns, calculator functions, any page via `file.import page` and the Import dialog, clip groups kept by Create Shapes from Vector Layer as layer masks / Merge Paths; PSD smart-object perspective quads and placed-layer warps — named styles and custom quilt meshes — baked as placed; PDF / PDF-compatible AI / EPS vector footage with Continuously Rasterize, layered composition import and Create Shapes from Vector Layer, and PSD smart objects with embedded files landed in M13.2; PSD as footage/composition/retain layer sizes, SVG footage earlier) |
| Automation | ≈ 99% | 0.1 | `.jsxbin` (AUT-2: ScriptUI resource strings, `onDraw`/ScriptUIGraphics, live `onChanging`, `Socket`, Essential Graphics hooks — `addToMotionGraphicsTemplate(As)`, `canAddToMotionGraphicsTemplate`, `exportAsMotionGraphicsTemplate`, `motionGraphicsTemplateName`, controller count/names — and Watch Folder landed in M13.7; the core object model landed in M14.4; ScriptUI windows/dialogs/dockable panels with `scriptui.*` agent commands, File ▸ Scripts install + sample scripts, and the effect plug-in API (EFF-6, WebAssembly) landed in M13.1) |
| Shapes | ≈ 80% | 1.5 | Lottie can't carry stroke taper/wave (stroke Taper and Wave, Dash 2/Gap 2/Dash 3/Gap 3 and radial-gradient Highlight Length/Angle landed in M13.5; pen tool for shape paths and vertex editing in M6.5) |
| Compositions | ≈ 80% | 3.5 | Mocha-style planar tracks for templates, Essential Graphics' mirrored properties (CMP-7: Font and uniform Scale controls, Composition ▸ Open in Essential Graphics, Save Frame As ▸ Photoshop Layers / ProEXR and the VR Comp Editor landed in M13.7; the marker dialog, Composition Flowchart, Essential Graphics with master properties, `.ectemplate` templates and Responsive Design — Time landed: CMP-6, CMP-7) |
| Animation | 70% | 9.0 | puppet depth beyond pins and recording (puppet pin recording with Record Options landed in M13.1), Wiggler/Smoother/Motion Sketch (motion-path handles and the graph editor transform box landed in M5.8; keyframe colour labels and Select Keyframe Label Group, Graph Editor snapping to markers / layer ends in M13.5) |
| Text | ≈ 96% | 0.3 | no extruded strokes (M13.12: the Variable Font Axes animator re-spaces the text — advances follow the animated axes; M13.6: variable font axes in the character style — `layer.setText variations`, the Character panel's Variable Font Axes fields — shape with HVAR / gvar advances and draw at that design-space position; OpenType features — stylistic sets, discretionary ligatures, contextual / stylistic alternates, swash, titling, ordinals, fractions, figure styles, true small caps / all small caps and superior / inferior glyphs with faux fallback — per character with the Character panel's OpenType popup and `text.fontFeatures` landed in M13.2; vertical Roman / Tate-Chu-Yoko, forced LTR paragraphs, caret on animated and path text, Variable Font Axes and Lottie style runs landed in M13.5; extruded, bevelled text in M7.6; per-character styles, paragraph settings, on-canvas editing and the `sourceText` style API in M9.9–M9.10) |
| Web | 85% | 1.0 | viewer frames and Roto Brush propagation still on the page's thread; GPU effects in the browser (browser storage, Web Audio, Web Worker renders/analyses, WebGPU viewer and offline install landed in M15.2) |
| 3D | 88% | 5.5 | multi-view layouts, the Extended Viewer for Advanced 3D comps (Classic 3D Extended Viewer landed in M13.5 UI completion); collapsed precomps of another size seen through the parent's camera render (fixed in M13.2); stereo rigs, orbit nulls, lights controlled by the camera, cameras/lights from glTF models, environment backgrounds, Advanced 3D motion blur, blend modes and track mattes landed in M7.7; Classic 3D iris-shaped bokeh with highlights, progressive depth of field on tilted layers and the focus-link commands landed in M13.5; Advanced 3D depth of field with the iris and highlight options, collapsed precomps as real Advanced 3D geometry and extruded text/shape strokes landed in M13.8; Advanced 3D (glTF/OBJ models, primitives, extruded text and shapes, PBR, image-based light, shadow maps, GPU rasteriser) in M7.4–M7.6 |
| Effects | ≈ 80% | 4.0 | GPU versions of the remaining effects (58 run on the GPU since M12.7) and the missing controls listed as partial in [effects.md](effects.md) (every After Effects effect exists since M9.11, M12.5 and M12.6; parameter names, order, twirl-downs, popups, units and defaults were aligned in M9.12) |
| Interface | 74% | 5.0 | more Learn tutorials and pixel-level fidelity of dialogs (the Home ▸ Learn tab with interactive tutorials and a UI fidelity pass landed in M13.5 UI completion; Timeline outline and Project panel columns scroll horizontally, the Layer Style dialog, ROI resize handles, Pan Behind snapping and 3D Reference Axes landed in M13.5; native macOS menu bar, Timeline columns/search/reveal-add, Home screen with recent projects and all AE workspaces landed; viewer rulers/snapping/channels/snapshots landed in M0.13) |
| Project | ≈ 68% | 5.5 | auto-save, folder moves, OCIO displays beyond the built-in tone map (Color Engine with OCIO/ACES working spaces, HDR compand/tone mapping, Rec. 2100 PQ/HLG output, Feet + Frames, display colour management, Simulate Output and the locked viewer landed in M7.7; proxies and Interpret Footage fields / pixel aspect / alpha guess landed: PRJ-8, PRJ-3) |
| Masks & roto | 74% | 5.0 | Roto Brush's learned (3.0) segmentation model (variable-width mask feather points with the Mask Feather tool landed in M13.5; mask tracking and Mask Interpolation landed in M6.6; Roto Brush & Refine Edge with graph-cut segmentation, flow propagation, edge matting, decontamination and Freeze in M6.7) |
| Preview | 74% | 3.2 | wireframes and Advanced 3D compositing on the GPU (Classic 3D bokeh depth of field — iris shapes, highlights, fringe, progressive blur on tilted planes — runs in WGSL since M13.6: 3D Showcase GPU warm 205 → 54 ms/frame at full size on an M4 Pro; the Preview panel's five shortcuts with their own Include / Loop / Cache Before Playback / Range / Play From / Frame Rate / Skip / Resolution / Full Screen / stop options and `playback.settings.get/set` landed in M13.8; Classic 3D runs and adjustment layers composite on the GPU since M12.7; persistent disk cache with the blue cache bar landed; region of interest, snapshots, exposure and Fast Previews landed in M0.13) |
| Tracking | ≈ 96% | 0.3 | face tracking is a classical (skin model + feature components + shape model) fitter, not a learned detector: profile views and occluded faces are weak; Rolling Shutter Ripple is approximated by Subspace Warp's mesh density (face tracking (Outline Only / Detailed Features with Face Track Points and Extract & Copy Face Measurements), Subspace Warp's content-preserving mesh warp on subspace-smoothed trajectories, and radial lens distortion (k1, k2) in the camera bundle adjustment with Undistort Footage landed in M13.3; Rolling Shutter Repair in M9.11; point tracker, mask tracking, Warp Stabilizer and the 3D Camera Tracker in M6.x / M12.5 / M12.6) |
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
| Color Correction | OCIO CDL / Color Space / Display / File / Look Transform, Color Stabilizer | Built-in minimal OCIO-style config (ACES2065-1, ACEScg, ACEScct, sRGB, Rec.709, Rec.2020, Display P3, linear variants, XYZ) from published matrices and transfer functions; `.cube` / `.3dl` / `.csp` LUTs and ASC `.cc` / `.ccc` / `.cdl`. Custom `.ocio` config files are read (Configuration ▸ Custom: colorspaces, roles, displays/views; Matrix, File, Exponent, Log, LogAffine, CDL, Range and Group transforms). |
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
taper/wave (the Variable Font Axes animator changes advances since M13.12, the Character panel's
axes since M13.6; Advanced 3D's DOF has iris shapes since M13.8).

## Update: M13.8 Preview panel, Align panel, Advanced 3D completion

- **Preview panel (PRV-1)**: Shortcut popup (Spacebar, Shift+Spacebar, Numpad 0, Shift+Numpad
  0, Alt+Numpad 0), each shortcut with its own saved options (settings `preview`): Include
  Video / Audio / Overlays / Layer Controls, Loop, Cache Before Playback, Range (Work Area,
  Work Area Extended By Current Time, Entire Duration, Play Around Current Time with pre/post
  roll), Play From (Range Start / Current Time), Frame Rate (Auto or a rate), Skip, Resolution
  (Auto, Full … Quarter, Custom), Full Screen, "If caching, play cached frames" and "Move time to
  preview time". `playback.settings.get/set`; playback follows the plan (range, start, skip grid,
  rate, loop, cache-first, audio-only, resolution override, hidden overlays / layer controls,
  maximized viewer); the keys trigger their shortcut (`playback.toggle {shortcut?}`,
  `playback.ramPreview`, `playback.preview.shiftSpacebar|shiftNumpad0|altNumpad0`). Full Screen
  maximizes the Composition panel rather than taking over the display.
- **Align panel (UI-1)**: `layer.align {edge, to: composition|selection}` and `layer.distribute
  {mode}` (edges or centres) on content bounds through transform and parents, one undo step;
  Align Layers to dropdown and the Distribute Layers row.
- **Advanced 3D (3D-3)**: depth of field with the camera's iris and highlight options (the
  Classic 3D bokeh kernel, by each pixel's depth); collapsed precomps add their nested layers
  as real geometry lit by the parent (2D collapsed precomps draw nested 3D layers with the
  parent's renderer); text and shape strokes extrude as bevelled meshes in paint order.

## Update: M13.5 UI completion & fidelity

**Extended Viewer** (Settings ▸ 3D, the viewer's Extended Viewer button, `view.extendedViewer`):
custom 3D views (and the Active Camera view with Draft 3D on) render the visible pasteboard
around the comp frame (the frame is outlined) so 3D layers reaching past the frame stay
visible; Classic 3D planes project through the offset canvas natively, Advanced 3D comps show
the frame only. **Home ▸ Learn**: three original interactive tutorials (Animate a title, Track
and attach, Make a 3D scene) whose steps highlight their UI target by automation id, advance when
the user runs the step's command, and can be performed with Show me (`learn.list`,
`learn.start`, `learn.step`, `learn.stop`, `learn.state`). **UI fidelity pass** against After
Effects 2026 captures: compact 19 pt Timeline / Project rows, flat rows with dark hairlines,
dark switch wells, the selected layer / item name in a light cell, label-coloured bars muted
toward grey, the time navigator with blue end handles and the work area bar (with the cache
bar under it) below the ruler, open twirl chevrons, a 16 pt current-time display, the
workspace bar order (Default, Review, Learn, Small Screen, Standard) and a single breadcrumb row
above the viewer. Not yet: Composition Settings / Render Queue / Settings dialogs compared
pixel by pixel (no reference captures of them yet).

## Update: M13.6 panels and content tools

The last placeholder panels became real panels with automation ids: **Lumetri Scopes**
(waveform RGB / Luma / YC, vectorscope YUV / HLS, histogram, parade RGB / YUV; Rec. 601 / 709 /
2020, 8-bit or float scale, clamp; `scopes.analyze`), **Footage** (double-click footage: its own
time ruler, play, In/Out, Overlay Edit and Ripple Insert Edit; `footage.*`), **Media Browser**
(desktop file system, favourites, thumbnails, import and drag to the Project panel or timeline;
`mediaBrowser.*`), **Metadata** (codec, size, rate, duration, colour profile, file dates; item
and project comments; `item.metadata`) and **Progress** (every render and analysis with
progress and cancel; `jobs.*`). **Content-Aware Fill** (panel and Layer ▸ New ▸ Content-Aware
Fill Layer): Object / Surface / Edge Blend, Work Area / Entire Duration, Alpha Expansion,
Lighting Correction and a reference frame from a layer, rendered to a PNG sequence in a Fill
layer above the source as a background job. **Scene Edit Detection** (markers, split, split and
precompose), **Auto-trace** (alpha / RGB / luminance, current frame or work area, to the layer
or a new layer) and **Align Video to Data** (JSON / CSV / TSV time keys) replace their disabled
menu entries. Not yet: creating a reference frame by editing a still (After Effects hands it to
Photoshop), Content-Aware Fill's learned model (ours is PatchMatch + flow propagation), and Media
Browser in the web app.

## Update: M13.12 import and text leftovers, test hardening

- **PDF / AI**: mesh shadings — free-form and lattice-form Gouraud triangle meshes (types 4, 5),
  Coons and tensor-product patch meshes (6, 7, tessellated to triangles) — and function-based
  shadings (type 1, with multi-input sampled functions), for `sh` and shading-pattern fills,
  rasterised into image nodes; CCITT Group 3 (1-D and 2-D) and Group 4 images from ITU-T T.4 /
  T.6 (checked against an independent encoder's output); knockout and isolated transparency
  groups, the current alpha / blend mode / soft mask applied to a transparency group XObject's
  combined result, and blend modes inside clipping groups reaching the backdrop; `/W2` / `/DW2`
  vertical metrics and CMap `WMode`. Not done: JBIG2 images (deferred) and predefined CJK CMaps (no permissively licensed
  pure-Rust source of the CMap data; Adobe's CMap resources are Adobe assets).
- **EPS**: text through the PostScript interpreter — `findfont` / `scalefont` / `makefont` /
  `selectfont` / `setfont`, `show`, `ashow`, `widthshow`, `awidthshow`, `xshow` / `yshow` /
  `xyshow`, `kshow`, `glyphshow`, `charpath`, `stringwidth`, re-encoded fonts (`definefont`
  copies with a new `/Encoding`) — with embedded Type 1 and CFF programs or the bundled fonts.
- **Create Shapes from Vector Layer**: images become PNG footage layers parented to the shape
  layer, and the shapes between images are split into layers of their own so the stack keeps
  the document's paint order; soft masks and clipped images are reported.
- **Text**: the Variable Font Axes animator re-spaces the text (advances at the animated
  design-space position), not only the outlines.
- **Hardening**: PDF / EPS parsers survive truncated and corrupted files (fixed an
  out-of-range stream slice and unbounded PostScript recursion); WebAssembly plug-ins get a
  memory cap, a manifest length limit and slider schema checks, with tests for missing exports,
  wrong signatures and versions, fuel exhaustion, traps and bit-identical output; script
  `Socket` validates ports, and ScriptUI resource strings accept trailing array commas.

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
