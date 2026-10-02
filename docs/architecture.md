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
| L1 | `raster` | Premultiplied float images, sampling, affine and projective warps, blurs, compositing (parallel with rayon) |
| L1 | `keyframe` | Animated values, keyframes with temporal ease and spatial Bezier, roving, hold, velocity |
| L1 | `path` | Bezier paths, path operators (trim, offset, round corners, zig zag, twist, merge…), stroking, coverage masks |
| L2 | `project` | The document: items, compositions, layers, the property tree, render queue model, `.ecproj` serde |
| L2 | `text` | Fonts, shaping, layout, per-glyph geometry, text animators and selectors |
| L2 | `effects` | The effect registry (241 effects) and their CPU implementations |
| L2 | `track` | Motion tracking: feature/search region point tracking (pyramid normalized cross-correlation, Lucas–Kanade sub-pixel refinement), confidence, homography/affine/similarity solves |
| L3 | `render` | Evaluation and compositing: sources, masks, effects, transforms, 3D, motion blur, mattes, blending, layer cache, audio mixdown |
| L3 | `media` | Footage decoding (FilmCraft's pure-Rust codecs), image sequences, frame cache |
| L3 | `expr` | The expression engine (JavaScript via boa) with the After Effects object model |
| L3 | `export` | Render queue encoding: H.264, ProRes, PNG/JPEG/TIFF/EXR sequences, GIF, audio |
| L4 | `engine` | `Session`: project, undo history, editor state, the command registry and menus |
| L4 | `host` | A fully wired `Session` (media, expressions, exporter) for the frontends |
| L5 | `ui-egui` | The desktop interface: docking, panels, viewer, timeline, graph editor, dialogs, control channel |
| L5 | `automation` | The MCP server, headless or bridged to the running app |
| L6 | apps `effectcraft`, `effectcraft-cli`, `effectcraft-web` | Desktop app; command-line tool (render, exec, get/set, MCP); the browser app (wasm32, [web.md](web.md)) |

Allowed same-layer edges: `path → keyframe, raster`, `text → path`, `effects → project, text, path`,
`media / expr / export → render`, `export → media`, `host → engine`.

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
active camera, with lights, shadows and depth of field.

A **layer cache** keeps each layer's finished pixels (source, masks and effects) keyed by a hash of
its evaluated inputs, excluding the transform. Static and transform-only layers render once;
editing one layer re-renders only that layer. Effects that read the clock directly are declared in
`effects::TIME_DEPENDENT`, and a test checks every registered effect against that list.

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

**Motion tracking** (Animation ▸ Track Motion / Stabilize Motion, Window ▸ Tracker): a tracker
is a `Tracker` group under the layer's Motion Trackers group, with Track Point groups (Feature
Center, Feature Size, Search Offset, Search Size, Confidence, Attach Point, Attach Point Offset)
keyframed per analysed frame. `track.analyze` renders the layer's source frames
(`Renderer::layer_source`) on a background thread (`engine::tracking`, polled like the render
queue; blocking with `wait`), and `effectcraft-track` matches each point in parallel. One undo step
covers an analysis. `track.apply` keys the target's Position/Rotation/Scale, the tracked layer's
Anchor Point and Position (Stabilize), or a Corner Pin effect (Parallel / Perspective).

## 5. Expressions

Expressions are JavaScript, run by boa, with After Effects' object model (`thisComp`, `thisLayer`,
`time`, `value`, `wiggle`, `loopOut`, vector maths on arrays, and so on). A syntax error keeps the
text, disables the expression and shows a warning, like After Effects.

## 6. Commands, the UI seam and automation

Every action is a command: an id, a label, a menu path, a shortcut, a parameter description,
`enabled()` and `run()`. The menu bar is a tree of command ids. The desktop UI, the command line,
the JSON control channel and the MCP server all dispatch by id, so anything a person can do from a
menu, an agent can do too. Every interactive widget registers an automation id, so agents can also
inspect and click the interface. See [agents.md](agents.md) and
[control-protocol.md](control-protocol.md).
