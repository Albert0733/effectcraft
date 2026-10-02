# EffectCraft on the web

`apps/effectcraft-web` runs the same engine and egui UI as the desktop app in the browser: the
workspace compiled to `wasm32-unknown-unknown`, started by eframe's web runner on WebGPU (WebGL2
fallback). It is a static site: one `.wasm`, its `wasm-bindgen` JavaScript glue, `index.html` and an
icon. Nothing is uploaded anywhere; projects, media and renders stay in the page and on the user's
machine.

## Build and run

| Tool | Version | Install |
|---|---|---|
| Rust target | `wasm32-unknown-unknown` | `rustup target add wasm32-unknown-unknown` |
| wasm-bindgen CLI | exactly the `wasm-bindgen` crate version (0.2.129) | `cargo install wasm-bindgen-cli --version 0.2.129 --locked` |
| wasm-opt (optional) | any recent binaryen | `brew install binaryen`; used by `cargo xtask web` when found |

```sh
cargo xtask web                 # release build → <target>/web/dist (index.html, effectcraft_web.js, effectcraft_web_bg.wasm, favicon.svg)
cargo xtask web --dev           # unoptimised build (faster to compile, slow to run)
cargo xtask web --serve 8765    # build, then serve dist on http://127.0.0.1:8765/
```

`<target>` is `$CARGO_TARGET_DIR` or `target`. `--serve` is a tiny localhost-only static server that
sends `Cross-Origin-Opener-Policy` / `Cross-Origin-Embedder-Policy`, so the page is cross-origin
isolated like a production deployment should be. Any static server works
(`python3 -m http.server -d target/web/dist 8765`); it must serve `.wasm` as `application/wasm`.

The release `.wasm` is about 31 MB before `wasm-opt` (20 MB code: the effects, codecs, the
expression engine and the UI; 8.5 MB data, mostly the bundled fonts; 2 MB function names for
readable panics). Served compressed it is a fraction of that.

The web crate is empty on non-wasm targets, so `cargo test --workspace` and clippy are unaffected;
`cargo xtask wasm` (part of `cargo xtask ci`) checks it, with every L0–L4 crate and
`effectcraft-ui-egui`, for `wasm32-unknown-unknown`.

URL flags: `?empty` (start without the demo project), `?home` (show the start screen).

## How the desktop pieces map to the browser

| Desktop | Web |
|---|---|
| frame render thread pool (`ui-egui/src/frames.rs`) | `Frames::pump`: queued frames render on the UI thread after each egui frame, most important first (the viewer's frame, then prefetch), within 40 ms (24 ms while playing) |
| Render Queue on a background thread | `Session::start_render` always runs inline on wasm32: the page waits until the render is done |
| `rayon` parallel loops (raster, effects, export batches) | rayon's global pool falls back to the calling thread when threads are unavailable: same code, serial |
| `std::time::Instant` / `SystemTime` (panic on wasm32) | `web-time` (re-exports `std::time` on native) |
| `std::fs` project reads/writes (`FsServices`) | `files::WebServices`: an in-memory file table; saving downloads the `.ecproj` |
| media reads (`MediaPool`, `probe`) | `files::WebImporter`: bytes from the file table, handed to `MediaPool::add_bytes` / `probe_bytes` |
| export writes (`effectcraft-export`) | the job's `sink` (`effectcraft_host::FileExporter { sink }`): files land in the table and download after the render; several files (an image sequence) download as one stored `.zip` |
| rfd file dialogs | `<input type=file>` for File ▸ Open / Import; drop files anywhere on the page (`.ecproj` opens, everything else imports); "Save As" / "Output To" pick a download name |
| system fonts | not scanned; the bundled fonts (Inter, Noto Serif, JetBrains Mono) are always there |
| TCP control channel / MCP | `window.effectcraft` (below) |
| audio output | none yet (if `cpal` lands for desktop playback it must stay behind a cfg/feature off wasm32; a WebAudio backend would replace it) |

### Threads

wasm threads need a cross-origin isolated page **and** a wasm build with atomics (a nightly
`build-std` toolchain), so this build is single-threaded: frames, effects and exports run on the UI
thread. A long Render Queue job freezes the page until it finishes; keep web renders short or
small (resolution, time span).

## `window.effectcraft`: the agent / test API

The control channel of the desktop app (`docs/control-protocol.md`) as promises. Results resolve
with the method's `result`, errors reject with the message.

| Call | |
|---|---|
| `effectcraft.request(method, params)` | any control method: `ui.inspect`, `ui.click`, `ui.key`, `ui.playback`, `ui.menu.invoke`, `render.frame`… |
| `effectcraft.execute(command, params)` | `engine.execute`: an engine command (same ids and params as MCP / `effectcraft-cli`) |
| `effectcraft.commands()` / `effectcraft.inspect()` | `engine.commands` / `ui.inspect` |
| `effectcraft.renderFrame(params)` | `render.frame` with `base64: true`: `{comp, time, width, height, png}` |
| `effectcraft.screenshot(params)` | `ui.screenshot`; without `path` the PNG comes back inline as `png` (base64) |
| `effectcraft.addFile(fileOrUrl, name?)` | put a `File`/`Blob` (or fetched URL) into the file table and open (`.ecproj`) or import it; resolves with its path |
| `effectcraft.files()` / `effectcraft.readFile(path)` | the file table `[{path, size}]` / a file's bytes (`Uint8Array`), e.g. a render |
| `effectcraft.info()` | graphics backend, version, `crossOriginIsolated`, load timings |

`window.effectcraftLoad` holds `{wasmMs, readyMs}` (module fetch + compile, and until the app runs).

```js
await effectcraft.execute("layer.newSolid", {color: "#ff8800"});
await effectcraft.request("ui.menu.invoke", {id: "renderQueue.add", params: {format: "gif", resolution: 0.25}});
await effectcraft.execute("renderQueue.render", {});   // the GIF downloads
```

## Browser test

`apps/effectcraft-web/tests/smoke.mjs` drives headless Chrome over the DevTools protocol (Node ≥ 22,
no npm packages): load, the demo comp in the viewer, `render.frame`, Render Queue GIF and PNG
sequence (`.zip`) downloads, Save As and re-opening the saved project. It writes screenshots and
`report.json`:

```sh
cargo xtask web --serve 8765 &
node apps/effectcraft-web/tests/smoke.mjs --url http://127.0.0.1:8765/ --out target/web/smoke
```

## Gaps

- Single-threaded (above); Render Queue renders block the page.
- No audio playback.
- Imported media live in memory for the session only (nothing is persisted; reload = fresh start).
- `engine.execute` rejects the spread Render Settings / Output Module keys of `renderQueue.add`
  (`resolution`, `timeSpan`…) through its strict parameter check; use `ui.menu.invoke` as above.
