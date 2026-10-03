# Driving EffectCraft from agents

Everything in EffectCraft is an engine command with a stable id and JSON params. Menus, shortcuts,
panel gestures, the CLI, the control channel and MCP all go through the same commands, so anything a
user can do, an agent can do too. There are three ways in:

| Interface | Best for | Needs a window |
|---|---|---|
| **MCP** (`effectcraft-cli mcp`) | Claude Code and other MCP clients | no (headless), or yes with `--bridge` |
| **CLI** (`effectcraft-cli exec/get/set/...`) | one-shot scripting and CI; JSON output with `--json` | no |
| **Control channel** (`effectcraft --control 9877`) | driving and seeing the live UI; see [control-protocol.md](control-protocol.md) | yes |

## MCP setup

Build once with `cargo build --release -p effectcraft-cli`, then register the server. For Claude
Code, add a `.mcp.json` at the project root:

```json
{
  "mcpServers": {
    "effectcraft": {
      "command": "/path/to/effectcraft/target/release/effectcraft-cli",
      "args": ["mcp"]
    }
  }
}
```

This repository ships a ready-made [`.mcp.json`](../.mcp.json): `effectcraft` (headless, demo
project loaded) and `effectcraft-app` (bridged to a desktop app started with `--control 9877`). Both
run through `cargo run --release`, so the first start compiles; run
`cargo build --release -p effectcraft-cli` once beforehand to avoid an MCP startup timeout.

You can also register it from the command line:
`claude mcp add effectcraft -- /path/to/target/release/effectcraft-cli mcp`. Other clients (Claude
Desktop, Cursor and the like) take the same `command` and `args`.

- **Headless** (`["mcp"]`): an in-process session with no window. Add `"--demo"` or
  `"--project", "file.ecproj"` to start with content. Startup is instant.
- **Bridge** (`["mcp", "--bridge", "9877"]`): drives a running `effectcraft --control 9877`, so you
  see every change live. Bridge mode adds `screenshot` and the `ui_*` tools.

The server speaks JSON-RPC 2.0 over stdio, one message per line, and supports MCP protocol versions
2025-06-18, 2025-03-26 and 2024-11-05 (`initialize`, `ping`, `tools/list`, `tools/call`).

### Tools

| Tool | What it does |
|---|---|
| `list_commands {filter?, enabled_only?}` | Discover command ids, their param docs, and whether each can run now. |
| `execute_command {command, params?}` | Run any command (undoable). |
| `run_script {code, name?}` | Run JavaScript with the After Effects-style scripting object model (`app.project`, `comp.layers.addText(…)`, `layer.property("ADBE Transform Group").property("ADBE Position").setValueAtTime(…)`…). Returns `{ok, result, output, error: {message, line, column}}`; edits are undoable. |
| `get_project` / `get_comp {comp?}` | Project items, comp settings and layers. |
| `get_layer {layer, comp?, time?, flat?}` | A layer's property tree. Every node has a `path`. |
| `get_property {layer, path, comp?, time?}` | Value at a time, keyframes and expression. |
| `set_property {layer, path, value?, time?, expression?, comp?}` | Sets a static value. With `time` it sets a keyframe; with `expression` it sets an expression. |
| `add_keyframe {layer, path, time+value \| keys:[...], interpolation?}` | Adds keys, then optionally applies linear/bezier/hold/easyEase. |
| `render_frame {comp?, time?, max_side?, path?, inline?}` | Returns a PNG image of a frame. |
| `open_project {path \| demo \| new}` / `save_project {path?}` | Open and save files. |
| `undo {steps?}` / `redo {steps?}` | History. |
| *(bridge)* `screenshot {panel?, id?}`, `ui_inspect`, `ui_elements {prefix?}`, `ui_click`, `ui_drag`, `ui_key`, `ui_type`, `ui_set`, `control {method, params}` | Look at and operate the live window. |

Layers are referenced by id (from `get_comp`), `"#n"` (1-based index from the top) or name. Comps are
referenced by id or name, and default to the active comp. Property paths come from `get_layer`, for
example `transform/position`, `transform/opacity`, `effects/#1/blurriness` or `@57` (by uid). Times
are in seconds. Keyframe times are layer time, which equals comp time unless the layer is offset or
stretched.

Command params are checked against each command's `params` doc. An unknown key, such as
`layer.select {"index": 2}`, returns an error that lists the accepted keys (here `layers, add, toggle`)
instead of being ignored.

### A typical session

1. `execute_command {"command":"comp.new","params":{"name":"Intro","width":1920,"height":1080,"frameRate":30,"duration":4}}`
2. `execute_command {"command":"layer.newText","params":{"text":"Hello","size":160,"fill":"#ffffff"}}` returns `{"layer": 2}`.
3. `add_keyframe {"layer":2,"path":"transform/position","keys":[{"time":0,"value":[-300,540]},{"time":1.5,"value":[960,540]}],"interpolation":"easyEase"}`
4. `execute_command {"command":"effect.apply","params":{"layer":2,"effect":"Gaussian Blur"}}`, then `set_property {"layer":2,"path":"effects/#1/blurriness","value":8}`
5. `render_frame {"time":1.0}` lets you look at the result, and `save_project {"path":"intro.ecproj"}` saves it.

### Motion tracking

Trackers live on the tracked layer (`Motion Trackers ▸ Tracker n ▸ Track Point n`) and are driven
with `track.*` commands. `track.analyze` runs in the background in the app (poll `track.status`);
pass `"wait": true` to block until it finishes, which is what the CLI and headless MCP want.

1. `execute_command {"command":"track.motion","params":{"layer":"clip.mov"}}` creates a Transform
   tracker (Motion Target: the layer above). Use `track.stabilize` for Stabilize, or `track.new` with
   `kind` `transform|stabilize|affine|perspective|raw` and `rotation`/`scale`.
2. `execute_command {"command":"track.setPoint","params":{"point":1,"center":[812,440],"featureSize":[40,40],"searchSize":[96,96]}}`
3. `execute_command {"command":"track.analyze","params":{"direction":"forward","wait":true}}` keys Feature
   Center, Confidence and Attach Point on every frame from the current time to the layer's end.
4. `execute_command {"command":"track.apply","params":{"dimensions":"xy"}}` keys the target's Position
   (and Rotation/Scale), the layer's own Anchor Point and Position for Stabilize, or a Corner Pin
   effect for `affine`/`perspective` tracks. `track.options` sets the channel, blur/enhance,
   adapt-feature and "If Confidence is Below" behaviour; `track.status` reports the track points.

```sh
effectcraft-cli run clip.ecproj track.motion '{"layer":"#2"}' \
  track.setPoint '{"point":1,"center":[812,440]}' \
  track.analyze '{"wait":true}' track.apply '{}' --save
```

### Mask tracking and Mask Interpolation

`track.mask` follows the pixels inside a mask and keys its Mask Path on every frame (vertices and
tangents move with the fitted motion). `method` is `position`, `positionScale`,
`positionScaleRotation` (the default, also kept as the Tracker panel's Method), `positionScaleRotationSkew`
or `perspective`; `direction` is `forward|backward|frameForward|frameBackward` from the current
time (or `start` / `end` in seconds). Like `track.analyze` it runs in the background in the app
(progress in `track.status` under `mask`, `track.stop` cancels and keeps what was tracked) and is
one undo step; `"wait": true` blocks.

`mask.interpolate` (Window ▸ Mask Interpolation ▸ Apply) adds in-between Mask Path keys between
each pair of selected Mask Path keys (or the existing keys at `times`, in seconds), giving both
ends the same vertex count with a matched correspondence. Options: `keyframeRate` (number or
`"auto"` = the comp rate), `keyframeFields`, `linearVertexPaths`, `bendingResistance`, `quality`,
`addVertices` (number or `false`) with `addVerticesUnit` `pixels|total|percent`, `matchingMethod`
`auto|curve|polyline`, `oneToOne`, `firstVerticesMatch`. `mask.interpolationOptions` sets the
panel's defaults.

```sh
effectcraft-cli exec track.mask '{"layer":"#2","mask":1,"method":"perspective","wait":true}' clip.ecproj --save
effectcraft-cli exec mask.interpolate '{"layer":"#2","mask":"Mask 1","times":[0,2],"addVertices":10}' clip.ecproj --save --json
```

### Warp Stabilizer

Effect ▸ Distort ▸ Warp Stabilizer (`effect.apply {"effect":"Warp Stabilizer"}`), Animation ▸ Warp
Stabilizer VFX and the Tracker panel's button (`track.warpStabilizer`, which applies the effect and
starts analysing) stabilize a layer. `warp.analyze {layer?, effect?, wait?}` analyses every frame
between the layer's In and Out points in the background ("Analyzing in background (step 1 of 2)",
then "Stabilizing..."; `warp.status` reports `progress`, `banner`, whether the effect is
`analyzed`, and the current frame's `warp` matrix, `autoScale` and `crop`); `warp.cancel` stops it
without writing anything. The analysis is stored in the effect (saved with the project) and is
one undo step; trimming, slipping, time-remapping or replacing the layer's source clears it and
the app re-analyses automatically (headless: run `warp.analyze` again). Settings are ordinary
properties under the instance's groups, e.g. `effects/#1/stabilization/result` (0 Smooth Motion,
1 No Motion), `effects/#1/stabilization/smoothness`, `effects/#1/stabilization/method`,
`effects/#1/borders/framing` (0 Stabilize Only … 3 Stabilize, Synthesize Edges),
`effects/#1/borders/autoScale/maximumScale`, `effects/#1/advanced/showTrackPoints`.

```sh
effectcraft-cli run shaky.ecproj track.warpStabilizer '{"layer":"#1","wait":true}' \
  prop.set '{"layer":"#1","path":"effects/#1/stabilization/result","value":1}' --save
effectcraft-cli exec warp.analyze '{"layer":"#1","wait":true}' shaky.ecproj --save --json
effectcraft-cli exec warp.status '{"layer":"#1"}' shaky.ecproj --json
```

### 3D Camera Tracker

Animation ▸ Track Camera and the Tracker panel's button (`track.camera {layer?, shotType?:
fixed|variable|specify, aov?, solveMethod?: auto|typical|flat|tripod, detailed?, wait?}`) apply
Effect ▸ Perspective ▸ 3D Camera Tracker (or reuse the layer's) and analyse it in the background:
"Analyzing in background (step 1 of 2)" tracks features, "Solving camera" solves.
`camera.analyze {layer?, effect?, wait?}` re-runs it, `camera.cancel` stops it without writing
anything. `camera.solveStatus` reports `progress`/`banner`, `analyzed`, `solved`, `methodUsed`,
`averageError` (pixels), `focalLength`, `horizontalAngleOfView`, the current frame's `camera`
(position, orientation, zoom) and `groundPlane`. `camera.points {time?}` lists the solved points
visible now with their `id`, `comp` position, `depth`, `world` position and `error`.

Select points with `camera.selectPoints {points, add?, toggle?}` (the viewer's click / Shift-click /
marquee), then the right-click menu's commands: `camera.setGroundPlane {points?}` (Set Ground Plane
and Origin), `camera.createFromSolve {kind: text|solid|null|shadowCatcher, points? | target:
{center, normal, size?}, multiple?}` (Create Text / Solid / Null / Shadow Catcher, Camera and Light,
Create Multiple …), `camera.deletePoints {points?, wait?}` (re-solves; with Auto-delete Points
Across Time the same feature's other tracks go too) and `camera.create` (the Create Camera button).
Each is one undo step; the first create adds the one-node "3D Tracker Camera" keyed on every frame.
Changing the layer's frames clears the analysis; changing `effects/#1/shotType`,
`effects/#1/horizontalAngleOfView`, `effects/#1/advanced/solveMethod` re-solves the stored tracks.

```sh
effectcraft-cli run shot.ecproj track.camera '{"layer":"#1","wait":true}' \
  camera.points '{}' --json
effectcraft-cli run shot.ecproj camera.createFromSolve '{"kind":"solid","points":[12,40,77]}' --save
```

### Roto Brush & Refine Edge

The Roto Brush tool (Alt+W cycles Roto Brush / Refine Edge) paints in the Layer panel; agents use
`roto.stroke {layer, kind: fg|bg|refine|refineErase, points: [[x, y], …], frame?, radius?}`
(layer pixels, layer frames). The first foreground stroke applies the Roto Brush & Refine Edge
effect and sets the base frame with a span of 20 frames each side. `roto.propagate {layer,
direction?: forward|backward|both, to?, wait?}` segments the span (in the background unless
`wait`); strokes on any other frame correct it and propagation restarts from there.
`roto.span {start?, end?}`, `roto.freeze {wait?}` / `roto.unfreeze`, `roto.clearStrokes {frame?,
kind?}`, `roto.cancel` and `roto.options {diameter?, refineDiameter?, view?: alphaBoundary|alpha|
alphaOverlay|none}` complete the set; every edit is one undo step. `roto.status {layer, frame?,
matte?, compute?, compareTo?}` reports the base frame, span, computed and stroked frames, frozen
state, job progress and, for a frame, the matte's area, centroid, RLE matte and IoU against a
reference. Matte settings are ordinary properties (`effects/#1/rotoBrushMatte/searchRadius`,
`effects/#1/refineEdgeMatte/decontaminateEdgeColors`, …).

```sh
effectcraft-cli run clip.ecproj roto.stroke '{"layer":"#1","points":[[300,200],[360,230]],"radius":10}' \
  roto.stroke '{"layer":"#1","kind":"bg","points":[[40,40],[600,40]],"radius":12}' \
  roto.propagate '{"layer":"#1","wait":true}' roto.freeze '{"layer":"#1","wait":true}' --save
```

### Text: styles and editing

Source Text holds character style runs and per-paragraph settings. `layer.setText` changes the
whole layer, or only characters `range: [start, end]` (character indices): character attributes
(`font`, `size`, `fill`, `tracking`, `kerning: metrics|optical|<1/1000 em>`, `tsume`,
`baseline: superscript|subscript`, `allCaps`…) split the text into runs; paragraph attributes
(`justify`, `indentLeft`, `indentFirst`, `spaceBefore`, `direction`, `composer`,
`hangingPunctuation`…) apply to the paragraphs the range touches. Editing mirrors the Type tool:

1. `layer.newText {"text":"", "box":[100,100,600,300], "edit":true}` makes paragraph text and
   starts editing (`position` instead of `box` makes point text; `vertical: true` vertical type).
2. `text.insert {"text":"Hello world"}` types at the caret (replacing the selection);
   `text.setSelection {"start":6,"end":11}` selects "world"; `text.moveCaret {"to":"wordLeft",
   "extend":true}` moves like the arrow keys; `text.delete {"word":true}` is Alt+Backspace.
3. `layer.setText {"range":[6,11], "size":40, "fill":"#ff5500"}` styles the selection (the
   Character panel does the same); with an empty range it sets the style the next typed text takes.
4. `edit.copy`, `edit.paste`, `edit.pasteTextMatchFormatting` and `edit.pasteTextFormattingOnly`
   work on the selected text; `text.endEdit` commits (an empty Type-tool layer is removed).

In expressions, `text.sourceText.style` / `getStyleAt(i, t)` read styles and the setters
(`setFontSize(v, start?, count?)`, `setFillColor`, `setText`, `setJustification`…) return a
styled document.

### Scripts and ScriptUI windows

Scripts that build ScriptUI windows publish them to the session; agents drive them like a user:

1. `scriptui.list` → `[{window, title, kind: dialog|palette|window|panel, script, modal, size}]`.
2. `scriptui.get {"window": id}` → the control tree (`type`, `name`, `text`, `value`, `checked`,
   `items`, `selection`, laid-out `bounds`, `handlers`…).
3. `scriptui.click {"widget": "ok"}` presses a button / toggles a checkbox / picks a radio button or
   tab; `scriptui.set {"widget": "#4", "value": "Shot_"}` types into edit text, moves a slider or
   picks a list item (index or text); `scriptui.close {"result": 2}` closes. Controls are addressed
   by id, `#id`, `properties.name` or text; `window` can be omitted when one window is open. Each
   returns the handler run's `{ok, output, error}`.

A dialog's `show()` waits for the user: the `script.run` / `file.runScript` reply carries
`"waiting": true`, and the script continues (its final output arrives in the reply of the click
that closes the dialog). File ▸ Scripts: `file.scripts.list`, `file.runScript {"name": …}`,
`file.installScript` / `file.installScriptUIPanel {"path": …}`, `window.scriptPanel {"name": …}`.

### History, puppet recording, plug-ins

* `edit.history.list` lists every undo state as a tree (undoing then editing keeps the undone
  states as a branch); `edit.history.goto {"index": n}` (or `id`, or `steps`) jumps to any of
  them. MCP: the `history` tool.
* `puppet.recordPin {"layer": "#1", "pin": "Puppet Pin 1", "samples": [[t, x, y]…]}` records a
  drag (t = seconds since it began, layer space) into Position keys at the comp frame rate from the
  current time; `puppet.recordOptions {speed, smoothing, useDraftDeformation, showMesh}`.
* `effect.plugins.load {"path": "x.wasm"}` / `effect.plugins.list`: WebAssembly effect plug-ins
  ([plugins.md](plugins.md)), then `effect.apply` by id like a built-in.

## CLI

Each invocation runs a headless engine with no window. It opens the demo project unless you pass
`--project F.ecproj`, a positional `*.ecproj` or `--empty`. Add `--json` for one compact JSON document
on stdout. Errors print `{"error": ...}` and exit with status 1; usage errors exit with status 2.

```sh
effectcraft-cli info --json
effectcraft-cli commands --filter keys
effectcraft-cli exec comp.new --params '{"name":"Main","width":1280,"height":720}' --empty --save-as main.ecproj
effectcraft-cli exec layer.newSolid '{"color":"#3366ff"}' main.ecproj --save
effectcraft-cli props Main '#1' --project main.ecproj           # flat property list with paths
effectcraft-cli set Main '#1' transform/opacity 40 main.ecproj --save
effectcraft-cli set Main '#1' transform/position '[100,360]' --time 0 main.ecproj --save
effectcraft-cli get Main '#1' transform/position --time 0.5 main.ecproj --json
effectcraft-cli run main.ecproj comp.open '{"comp":"Main"}' time.set '{"time":1}' --json
effectcraft-cli render-frame main.ecproj --time 1 --max-side 640 --out f.png --json
effectcraft-cli exec layer.newNull --bridge 9877                 # same commands, against the live app
effectcraft-cli exec file.exportLottie '{"comp":"Main","path":"main.json","includeExpressions":true}' main.ecproj --json
effectcraft-cli exec file.importLottie '{"path":"anim.json"}' main.ecproj --save
```

Scripts written against After Effects' documented scripting API run with `script`:

```sh
effectcraft-cli script build.jsx --save-as main.ecproj     # empty project unless one is given
effectcraft-cli script --eval 'app.project.item(1).numLayers' main.ecproj --json
effectcraft-cli script tweak.jsx --bridge 9877              # against the live app
```

`writeLn`/`$.writeln`/`alert` output is printed, then the value of the last expression. A script
error exits with status 1 and `file:line:col: message`. Scripts may read files only in the
project's folder and may not write files or use the network unless the user turns on Preferences ▸
Scripting & Expressions ▸ Allow Scripts to Write Files and Access Network.

`file.exportLottie` returns `{path, bytes, warnings}`: the warnings list every feature Lottie
cannot express (most effects, cameras and lights, layer styles, audio, video footage…), so an
agent can check what a player will not show. A `.lottie` path writes a dotLottie archive.
`file.importLottie` returns `{comp, items, warnings}` and opens the new composition.

`<comp>` is an id or name, or `-` for the active comp. `<value>` is JSON (`50`, `[960,540]`,
`"#ff0000"`) or a bare string.

## Seeing the UI

To work on the UI, start the app with `cargo run -p effectcraft -- --control 9877` and use MCP bridge
mode or the raw control channel. A good loop is: `ui_elements` to find an id, `ui_click` or `ui_drag`
to act, then `screenshot {"panel":"Timeline"}` to check the result. `render_frame` shows the
composition itself at any zoom.
