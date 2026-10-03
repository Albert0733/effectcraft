# Architecture

EffectCraft is a stack of small Rust crates. The engine knows nothing about the user interface;
the egui frontend, the command-line tool and the MCP server all sit on top of the same `Session`
and drive it through the same command registry.

## 1. Crates and layering

Dependencies only point downward (or along the few same-layer edges listed below). `cargo xtask
layers` enforces this, and fails the build if a crate below L5 depends on egui, eframe, winit, rfd,
cpal or muda. Everything in L0 to L4, the egui UI and the web app also build for
`wasm32-unknown-unknown` (`cargo xtask wasm`); see [web.md](web.md).

| Layer | Crate (`effectcraft-…`) | Responsibility |
|---|---|---|
| L0 | `time` | `Tick` (254 016 000 000 per second), rational frame rates incl. NTSC, SMPTE and drop-frame timecode |
| L0 | `geom` | Vectors, matrices, quaternions, the layer transform (anchor, position, scale, orientation, rotation) |
| L0 | `color` | sRGB and linear, HSL/HSV, luminance, the 38 blend modes, label colors |
| L0 | `vp9enc` | VP9 intra-frame encoder (profile 0, 8-bit 4:2:0; lossless at quality 100) for WebM export, from the VP9 bitstream specification |
| L0 | `opusenc` | Opus (CELT) encoder: RFC 6716 CELT-only fullband 48 kHz 20 ms packets, mono/stereo, and the RFC 7845 `OpusHead`, for WebM export audio |
| L1 | `raster` | Premultiplied float images, sampling, affine and projective warps, blurs, compositing (parallel with rayon) |
| L1 | `keyframe` | Animated values, keyframes with temporal ease and spatial Bezier, roving, hold, velocity |
| L1 | `psd` | Photoshop PSD/PSB reader (layers, groups, masks, blend modes, text, layer effects, adjustment layers; 8/16/32-bit; RGB/CMYK/Gray/Lab) and a minimal writer, from Adobe's published format specification |
| L1 | `path` | Bezier paths, path operators (trim, offset, round corners, zig zag, twist, merge…), stroking, coverage masks |
| L2 | `project` | The document: items, compositions, layers, the property tree, render queue model, `.ecproj` serde |
| L2 | `text` | Fonts, shaping, layout, per-glyph geometry, text animators and selectors |
| L2 | `effects` | The effect registry (241 effects) and their CPU implementations |
| L2 | `svg` | SVG import (W3C SVG 1.1/2 static subset: shapes, paths, transforms, `use`, CSS, gradients) and rasterisation at any scale |
| L2 | `model` | 3D models for Advanced 3D: glTF 2.0 (`.gltf`/`.glb`) and OBJ/MTL import (meshes, PBR metallic-roughness materials and textures, node hierarchy, skins, animations), parametric primitives, extruded/bevelled outline meshes and polygon triangulation |
| L2 | `track` | Motion tracking: feature/search region point tracking (pyramid normalized cross-correlation, Lucas–Kanade sub-pixel refinement), confidence, homography/affine/similarity solves |
| L3 | `render` | Evaluation and compositing: sources, masks, effects, transforms, 3D, motion blur, mattes, blending, layer cache, audio mixdown |
| L3 | `media` | Footage decoding (FilmCraft's pure-Rust codecs), image sequences, Photoshop and SVG stills, frame cache |
| L3 | `expr` | The expression engine (JavaScript via boa) with the After Effects object model |
| L3 | `export` | Render queue encoding: H.264, ProRes, WebM (VP9 + alpha, Opus), PNG/JPEG/TIFF/EXR sequences, GIF, WAV/AIFF |
| L3 | `gpu` | The GPU compositor and GPU effects on wgpu compute shaders (Metal, Vulkan, Direct3D 12, WebGPU), checked against the CPU renderer |
| L3 | `lottie` | Lottie JSON / dotLottie import and export (layers, precomps, eased and spatial keyframes, shapes, masks, mattes) with a warnings list for what Lottie cannot express |
| L4 | `engine` | `Session`: project, undo history, editor state, the command registry and menus |
| L4 | `script` | Scripting: JavaScript (boa) with an After Effects-style object model (`app.project`, comps, layers, properties, render queue) whose edits run engine commands; AE match names |
| L4 | `host` | A fully wired `Session` (media, expressions, scripting, exporter) for the frontends |
| L5 | `ui-egui` | The desktop interface: docking, panels, viewer, timeline, graph editor, dialogs, control channel |
| L5 | `automation` | The MCP server, headless or bridged to the running app |
| L6 | apps `effectcraft`, `effectcraft-cli`, `effectcraft-web` | Desktop app; command-line tool (render, exec, get/set, MCP); the browser app (wasm32, [web.md](web.md)) |

Allowed same-layer edges: `path → keyframe, raster`, `text → path`, `effects → project, text, path, track`,
`media / expr / export / gpu → render`, `export → media`, `lottie → format`, `host → engine, script`,
`script → engine`.

## 2. Time

All time is an integer `Tick`, 254 016 000 000 per second, the least common multiple of every
broadcast frame rate (×1001) and audio sample rate, so frame and sample positions are exact.
Keyframe times are in **layer time**; `layer_time = (comp_time - start_time) / stretch`, and a
Time Remap property overrides the source time. Commands snap layer times, keyframe times and
comp durations to whole frames, as After Effects does. Timecode is only a display.

## 3. Document model

```
Project { settings, items, render_queue }
Item    { id, name, label, parent folder, kind: Folder | Comp | Footage | Solid }
Comp    { size, pixel aspect, frame rate, duration, start timecode, background, work area,
          layers (index 0 = layer #1), markers, motion blur shutter, renderer }
Layer   { id, name, source (footage | comp | solid | text | shape | null | camera | light),
          in/out/start, stretch, switches, blend mode, track matte, parent, props }
```

Everything animatable is a `Property` in the layer's `PropGroup` tree, addressed by a path such as
`transform/position`, `effects/#1/blurriness`, `masks/#1/feather`, or by `@uid`. Effects, masks,
text animators and shape contents are groups in the same tree.

## 4. Rendering

`Renderer::comp_frame(comp, t)` walks the layers bottom to top. For each visible layer it renders
the **source** (solid, footage frame, text, shape contents, or a nested comp), applies **masks**,
then **effects** in order, then **layer styles**, then the **transform** into comp space (with motion blur sub-samples
when enabled), the **track matte**, and finally **blends** into the accumulator. Adjustment layers
apply their effects to the accumulator. Runs of 3D layers are composited per pixel through the
active camera, with lights, shadows and depth of field (Classic 3D), or rasterised as meshes by
the Advanced 3D renderer (below).

**Advanced 3D** (Composition Settings ▸ 3D Renderer ▸ Advanced 3D, `comp.renderer`;
`crates/render/src/three_d/adv`, `crates/model`). A run of 3D layers becomes one scene of
world-space triangles: 3D model layers (glTF 2.0 / OBJ footage, mapped from +Y-up model units
into the layer by Geometry Options ▸ Model Scale; skins and animation clips played along layer
time), primitive layers (Layer ▸ New ▸ Cube/Sphere/Plane/Torus/Cone/Cylinder, parametric in
Geometry Options), text and shape layers with Geometry Options ▸ Extrusion Depth (glyph and
fill outlines flattened, triangulated with holes and extruded with Angular/Concave/Convex
bevels, per character so per-character 3D carries over), and a textured card for every other
3D layer. Shading is physically based in linear light (glTF metallic-roughness; Cook–Torrance
GGX specular, Smith–Schlick geometry, Schlick Fresnel, Lambert diffuse, after Karis 2013):
parallel, spot and point lights with After Effects' falloffs, ambient lights, image-based
light from an Environment light whose Source (or the comp's Environment Layer, Layer ▸
Environment Layer) is an equirectangular image (cosine irradiance map; roughness-indexed mip
chain with the split-sum environment BRDF fit), and percentage-closer-filtered shadow maps
(spot: perspective, parallel: orthographic over the scene, point: six cube faces). Cards keep
Material Options (Accepts Lights/Shadows, Casts Shadows On/Only, Ambient, Diffuse, Specular);
models and primitives add Base Color, Metallic, Roughness, Emissive. Rasterisation runs at 2×2
supersampling with a reversed-Z depth buffer; opaque triangles resolve visibility first, the
transparent ones are sorted back to front and blended. The software rasteriser
(`adv::raster`) is the reference and the headless/CI path; `effectcraft-gpu` implements
`Accelerator::raster_3d` with a wgpu render pipeline (`advanced3d.wgsl`) that repeats the
shading step for step, and tests compare the two. Resolve, depth of field (a gather blur by
each pixel's circle of confusion from the camera's Depth of Field settings) and the conversion
back to the working encoding run on the CPU for both. Not yet: motion blur, track mattes and
blend modes inside an Advanced 3D run, collapsed 3D precomps (drawn flattened), morph targets,
extruded strokes.

A **layer cache** keeps each layer's finished pixels (source, masks and effects) keyed by a hash of
its evaluated inputs, excluding the transform. Static and transform-only layers render once;
editing one layer re-renders only that layer. Effects that read the clock directly are declared in
`effects::TIME_DEPENDENT`, and a test checks every registered effect against that list.

A persistent **disk cache** (`render::disk_cache`, Settings ▸ Media & Disk Cache) backs both
the layer cache and the viewer's RAM preview: layer buffers that were slow to render and every
preview frame are written (LZ4, atomic rename, checksummed) under 128-bit content keys — the
comp's content hash (the comp, what it uses, footage file size and time) plus frame, scale and
view for frames; the layer key salted with the footage fingerprint for layers — so entries
survive restarts and never go stale. LRU eviction keeps it under the size limit; Edit ▸ Purge
empties it, `cache.diskStats` reports it, and the timeline draws disk-only frames in blue.

**Time effects** (Echo, Posterize Time, Timewarp…) read the layer at other times through
`EffectHost::self_at`: the renderer renders the layer's source and masks (plus, optionally, the
effects before it) at that layer time via `Renderer::layer_input`, cached under separate *input*
keys, with a nesting-depth guard.

**Audio** is mixed by `render::audio::mix_comp` in blocks: Audio switch, solo, Audio Levels and
the Effect > Audio effects (`effects::audio_fx`, applied per layer with a pre-roll so blocks are
independent). Export and preview playback share it; the desktop app plays it through cpal and the
audio clock drives preview playback (`ui-egui::audio`).

**Layer styles** (Layer ▸ Layer Styles; `crates/render/src/styles.rs`) render in layer space and
may grow the layer's bounds. Drop Shadow and Outer Glow become separate passes composited below the
layer with their own blend modes; the interior styles, Stroke and Bevel and Emboss are baked into
the layer body. Layer opacity fades the whole stack; Knockout and the R/G/B channel switches apply
at composite time. Styled pixels are cached separately from the layer content, so editing a style
reuses the cached source/masks/effects. Global Light is one setting per comp, mirrored into every
layer's Blending Options and kept in step after each edit.

**Paint and Puppet** are effects whose instances carry nested groups. Paint (`effects::paint`)
holds Brush / Clone / Eraser strokes (Path, Stroke Options, Transform, hidden Duration span);
strokes rasterize as brush-tip dabs and composite in order, and the layer cache key includes which
strokes are visible at the frame. Puppet (`effects::puppet`) holds meshes and pins: the mesh is
traced from the input alpha, expanded, and triangulated (Delaunay + constraint recovery, cached
by content); pins drive an as-rigid-as-possible solve (Igarashi et al. 2005) each frame and the
input is texture-mapped through the deformed triangles. The renderer flattens nested effect
groups into `Params` keys (`effects::flatten_params`). Commands: `paint.*`, `puppet.*`.

Half, Third and Quarter resolution render proportionally fewer pixels end to end.

**GPU compositor** (`crates/gpu`; Project Settings ▸ Video Rendering and Effects ▸ Mercury GPU
Acceleration, the default, or Mercury Software Only; `render.backend`). The CPU renderer is the
reference and keeps rendering layer content (sources, masks, CPU effects, layer styles) into the
layer cache. The render crate defines an `Accelerator` trait; `effectcraft_gpu::Gpu` implements it
on wgpu compute shaders. `RenderOpts::backend` picks `Cpu`, `Gpu` or `Auto` (GPU when an
accelerator is attached and the project's renderer is the GPU). A GPU frame walks the comp like
`draw_comp`: cached layer buffers are uploaded once per buffer, then transformed with the CPU's
sampling (nearest, bilinear, Catmull-Rom bicubic, the same minification pre-filter), motion-blur
sub-samples are accumulated, and track mattes, Preserve Transparency, layer style passes,
knockout, all 38 blend modes (a WGSL port of `color::blend`), the 8/16 bpc clamp-and-quantise steps
and colour-space conversions run on the GPU. 3D runs, adjustment layers and wireframes run on the
CPU between GPU steps (read back, draw, upload). GPU effects (`effects::GPU_EFFECTS`: Gaussian,
Fast Box and Directional Blur, Glow, Levels, Curves, Hue/Saturation, Tint, Fill, Gradient Ramp,
Fractal Noise, Drop Shadow, Brightness & Contrast, Exposure, Invert, Transform) repeat the CPU
effect's steps (padding, box radii, parameters) as kernels; consecutive GPU effects run as one
chain with one upload and one readback. Tests render scenes on both paths and compare them
(≤ 1/255 at 8 bpc, ≤ 1e-3 at 32 bpc); they skip without an adapter. The desktop viewer builds the
`Gpu` on egui-wgpu's device and shows frames from GPU textures without reading them back
(`ui-egui::frames`); headless renders, the CLI (unless `--gpu`) and CI use the CPU. On the web
(WebGPU) the GPU composites viewer frames; steps that need a readback fall back to the CPU.

**Colour and bit depth** (`crates/render/src/color.rs`, `crates/color/src/space.rs`). Pixels are
`f32`, but 8 and 16 bpc projects clamp and quantise each layer after its source and masks and
after every effect, and the comp after every layer (16 bpc uses 0..32768), so over-range values
(Add, Screen, Exposure…) only survive in 32 bpc; blend modes without HDR support clamp their
inputs in 32 bpc. With a working space (sRGB, Rec. 709, Rec. 2020, Display P3; matrices derived
from the published primaries), footage is converted from its colour profile (stream metadata or
Interpret Footage, else sRGB) and the top-level comp is converted to the sRGB display. Linearize
Working Space runs everything in linear light; Blend Colors Using 1.0 Gamma linearises only for
blending. Layer cache keys include these settings.

**Frame blending**: footage and precomp layers whose source time falls between source frames
(rate conform, stretch, remap) blend the two neighbouring frames, by cross-fade (Frame Mix) or by
hierarchical block-matching optical flow and a bidirectional warp (Pixel Motion,
`raster::flow`).

**Collapse Transformations**: a collapsed precomp layer without masks, effects or styles draws
its nested layers straight into the parent with concatenated transforms (one resample), so
nested blend modes and adjustment layers act on the parent's layers; nested 3D layers use the
parent's camera and lights and, when the precomp layer is 3D, are depth-sorted with the parent's
3D run. Masks, effects or styles force a flattened render. On text and shape layers the switch is
Continuously Rasterize: the source is rasterised at its on-screen scale. Quality: Draft samples
nearest-neighbour, Wireframe draws the layer bounds. Slip edit (`layer.slip`, Alt+PageUp/Down,
dragging the source bar in the timeline) moves the source under fixed in/out points.

**Motion tracking** (Animation ▸ Track Motion / Stabilize Motion, Window ▸ Tracker): a tracker
is a `Tracker` group under the layer's Motion Trackers group, with Track Point groups (Feature
Center, Feature Size, Search Offset, Search Size, Confidence, Attach Point, Attach Point Offset)
keyframed per analysed frame. `track.analyze` renders the layer's source frames
(`Renderer::layer_source`) on a background thread (`engine::tracking`, polled like the render
queue; blocking with `wait`), and `effectcraft-track` matches each point in parallel. One undo step
covers an analysis. `track.apply` keys the target's Position/Rotation/Scale, the tracked layer's
Anchor Point and Position (Stabilize), or a Corner Pin effect (Parallel / Perspective).

**Mask tracking** (`track.mask`, `engine::mask_track`) runs the same kind of background job over
the layer's source frames: `effectcraft-track`'s mask tracker detects features inside the mask,
tracks them with pyramidal Lucas–Kanade and fits the chosen model (position … perspective) with
RANSAC; the motion moves the Mask Path's vertices and tangents and is keyed per frame. **Mask
Interpolation** (`mask.interpolate`) uses `effectcraft-path`'s smart interpolation (arc-length
vertex insertion, shape-context matching, rigid in-betweens) to key in-between shapes.

**Warp Stabilizer** (`effects::warp_stab`, `engine::warp`): `warp.analyze` renders the layer's
input to the effect (`Renderer::layer_input`: source, masks and the effects above it) for every
frame on a background thread and stores `effectcraft-track`'s `WarpAnalysis` (per-frame
translation / similarity / homography fits) as JSON in the effect's hidden Analysis parameter,
with a key of the layer's source, In/Out, start, stretch and Time Remap. `Session::edit` clears
analyses whose key no longer matches and queues them; the desktop app re-analyses them in the
background. Rendering derives the stabilization plan (smoothed camera path or No Motion, framing,
auto-scale; cached per analysis and settings) and warps each frame; Synthesize Edges fills the
borders from neighbouring frames read with `EffectHost::self_at`. The effect lives in the
effects crate, hence the `effects → track` edge.

## 5. Expressions

Expressions are JavaScript, run by boa, with After Effects' object model (`thisComp`, `thisLayer`,
`time`, `value`, `wiggle`, `loopOut`, vector maths on arrays, and so on). A syntax error keeps the
text, disables the expression and shows a warning, like After Effects.

## 5a. Scripting

`effectcraft-script` runs JavaScript (boa, a fresh context per run; the Script Console keeps one)
with an object model written from the behaviour the After Effects Scripting Guide documents:
`app`, `Project`, `ItemCollection`, `CompItem`/`FootageItem`/`FolderItem`, `LayerCollection`,
`AVLayer`/`TextLayer`/`ShapeLayer`/`CameraLayer`/`LightLayer`, `PropertyGroup`/`Property`
(values, keyframes, eases, expressions), `TextDocument`, `Shape`, `KeyframeEase`, `MarkerValue`,
`RenderQueue`/`RenderQueueItem`/`OutputModule`, `File`/`Folder` and the enums. The model lives in
`prelude.js`; it reads the session through `__query` (JSON snapshots of items, layers, property
nodes, values and keys) and edits only through `__exec`, which runs engine commands, so every
scripted edit is undoable and identical to the UI's. While a script runs the session is moved
into a thread-local the natives borrow (nothing borrowed enters the JS heap); the comp a call
targets is made active for that command only. `app.beginUndoGroup`/`endUndoGroup` fold the undo
steps in between into one named step. Match names (`ADBE Transform Group`, `ADBE Position`,
`ADBE Gaussian Blur 2`, `ADBE Vector Shape - Rect`…) come from a table in `matchnames.rs`; effect
parameters are `<effect match name>-0001…`. `File` reads are limited to the project's folder, and
writes and the network are refused, unless Preferences ▸ Scripting & Expressions ▸ Allow Scripts
to Write Files and Access Network is on. Entry points: the `script.run` command
(`Session::script`, set by the host), File ▸ Scripts ▸ Run Script File… (`.jsx`/`.js`; `.json`
command scripts still run as steps), Window ▸ Script Console, `effectcraft-cli script` and the
MCP `run_script` tool.

## 6. Commands, the UI seam and automation

Every action is a command: an id, a label, a menu path, a shortcut, a parameter description,
`enabled()` and `run()`. The menu bar is a tree of command ids. The desktop UI, the command line,
the JSON control channel and the MCP server all dispatch by id, so anything a person can do from a
menu, an agent can do too. Every interactive widget registers an automation id, so agents can also
inspect and click the interface. See [agents.md](agents.md) and
[control-protocol.md](control-protocol.md).

Settings (`engine::prefs`), keyboard shortcut presets (`engine::shortcuts`) and auto-save /
crash recovery (`engine::autosave`) live in the engine too; they persist through a
`ConfigStore` the frontend provides (a directory on the desktop, `localStorage` on the web).
The shortcut dispatcher and the menus read the active preset. See
[preferences.md](preferences.md).
