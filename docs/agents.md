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
