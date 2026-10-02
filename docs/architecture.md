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
then **effects** in order, then the **transform** into comp space (with motion blur sub-samples
when enabled), the **track matte**, and finally **blends** into the accumulator. Adjustment layers
apply their effects to the accumulator. Runs of 3D layers are composited per pixel through the
active camera, with lights, shadows and depth of field.

A **layer cache** keeps each layer's finished pixels (source, masks and effects) keyed by a hash of
its evaluated inputs, excluding the transform. Static and transform-only layers render once;
editing one layer re-renders only that layer. Effects that read the clock directly are declared in
`effects::TIME_DEPENDENT`, and a test checks every registered effect against that list.

Half, Third and Quarter resolution render proportionally fewer pixels end to end.

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
