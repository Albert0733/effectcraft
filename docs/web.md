# EffectCraft on the web

`apps/effectcraft-web` runs the same engine and egui UI as the desktop app in the browser: the
workspace compiled to `wasm32-unknown-unknown`, started by eframe's web runner on WebGPU (WebGL2
fallback). It is a static site: one `.wasm`, its `wasm-bindgen` JavaScript glue, `index.html`, a
few small scripts (worker, audio worklet, service worker), a web manifest and icons. Nothing is
uploaded anywhere: projects, media, settings and renders stay in the browser (its private storage)
and on the user's machine.

## Build and run

| Tool | Version | Install |
|---|---|---|
| Rust target | `wasm32-unknown-unknown` | `rustup target add wasm32-unknown-unknown` |
| wasm-bindgen CLI | exactly the `wasm-bindgen` crate version (0.2.129) | `cargo install wasm-bindgen-cli --version 0.2.129 --locked` |
| wasm-opt (optional) | any recent binaryen | `brew install binaryen`; used by `cargo xtask web` when found |

```sh
cargo xtask web                 # release build → <target>/web/dist
cargo xtask web --dev           # unoptimised build (faster to compile, slow to run)
cargo xtask web --serve 8765    # build, then serve dist on http://127.0.0.1:8765/
```

`<target>` is `$CARGO_TARGET_DIR` or `target`. `dist` holds `index.html`, `effectcraft_web.js` +
`effectcraft_web_bg.wasm` (+ `snippets/`, the browser glue `js/host.js`), `worker.js`,
`audio-worklet.js`, `sw.js` (its precache list and version filled in by the build),
`manifest.webmanifest`, `favicon.svg` and `icon-256.png` / `icon-512.png` (copied from
`assets/app-icon`). `--serve` is a tiny localhost-only static server that sends
`Cross-Origin-Opener-Policy` / `Cross-Origin-Embedder-Policy`, so the page is cross-origin
isolated like a production deployment should be. Any static server works
(`python3 -m http.server -d target/web/dist 8765`); it must serve `.wasm` as `application/wasm`.
Service workers need a secure context: `https://`, or `http://localhost` / `127.0.0.1`.

The release `.wasm` is about 38 MB before `wasm-opt` (code: the effects, codecs, the expression
engine and the UI; data, mostly the bundled fonts; function names for readable panics). Served
compressed it is a fraction of that, and the service worker caches it after the first visit.

On other targets the web crate holds only its portable storage model (`store.rs`, unit-tested by
`cargo test --workspace`); `cargo xtask wasm` (part of `cargo xtask ci`) checks the whole crate,
with every L0–L4 crate and `effectcraft-ui-egui`, for `wasm32-unknown-unknown`.

URL flags: `?empty` (a blank project; the session is not restored or recorded), `?demo` (the demo
project instead of the last session), `?home` (show the start screen), `?storage=indexeddb` /
`?storage=memory` (force a storage backend), `?noworkers` (render and analyse on the page's
thread), `?nosw` (don't register the service worker).

## How the desktop pieces map to the browser

| Desktop | Web |
|---|---|
| frame render thread pool (`ui-egui/src/frames.rs`) | `Frames::pump`: queued frames render on the UI thread after each egui frame, most important first (the viewer's frame, then prefetch), within 40 ms (24 ms while playing) |
| Render Queue on a background thread | a Web Worker running a second engine instance (below); progress streams back, the page never waits |
| tracker / mask tracker / Warp Stabilizer / 3D Camera Tracker / Roto Brush Freeze threads | the same workers; the analysed property group comes back as one undo step |
| `rayon` parallel loops (raster, effects, export batches) | rayon's global pool falls back to the calling thread when threads are unavailable: same code, serial |
| `std::time::Instant` / `SystemTime` (panic on wasm32) | `web-time` (re-exports `std::time` on native) |
| `std::fs` project reads/writes (`FsServices`) | `files::WebServices`: the virtual file table, persisted (below); saving also downloads the `.ecproj` |
| config directory: settings, shortcut presets, recent projects, the recovery sentinel | `store::WebConfig` in browser storage |
| auto-save folder | `/EffectCraft Auto-Save/` in browser storage (`ConfigStore::files`) |
| media reads (`MediaPool`, `probe`) | `files::WebImporter`: bytes from the file table, handed to `MediaPool::add_bytes` / `probe_bytes` |
| export writes (`effectcraft-export`) | the job's `sink` (`effectcraft_host::FileExporter { sink }`): files land in the table and download after the render; several files (an image sequence) download as one stored `.zip` |
| rfd file dialogs | `<input type=file>` for File ▸ Open / Import; drop files anywhere on the page (`.ecproj` opens, everything else imports); "Save As" / "Output To" pick a download name |
| system fonts | not scanned; the bundled fonts (Inter, Noto Serif, JetBrains Mono) are always there |
| TCP control channel / MCP | `window.effectcraft` (below) |
| cpal audio output | Web Audio (below) |
| GPU compositor on the desktop's wgpu device | the same compositor on eframe's WebGPU device (below) |

### Persistence

Everything that lives in files on the desktop lives in the browser's **Origin Private File
System** (OPFS); where OPFS can't be written from the page (older Safari), **IndexedDB**; with
neither (some private windows), memory only. `effectcraft.info().storage.backend` says which.
The page asks for persistent storage (`navigator.storage.persist()`), which browsers grant to
installed apps, so the store isn't evicted under storage pressure.

The engine's storage traits are synchronous and the browser's are not, so the app works on an
in-memory mirror (`store.rs`): every stored entry is read before the app starts, reads and writes
hit the mirror, and changes flush in the background (one write per changed key, in order; OPFS
writes are atomic: a swap file replaced on close). Keys:

| Key | What |
|---|---|
| `config/prefs.json`, `config/shortcuts.json` | Settings (with File ▸ Open Recent's list) and keyboard shortcut presets |
| `config/session.lock` | the crash-recovery sentinel |
| `config/session.ecproj`, `config/session.json` | the session snapshot: the open project and editor state (active comp, time, selection), written at most once a second while they change |
| `files/<path>` | the file table: imported media, projects saved with File ▸ Save / Save As or `saveToBrowser`, auto-saves |

On the next visit the app reopens the snapshot (unsaved changes stay unsaved: the project is
dirty), registers every stored media file with the media pool so its footage decodes, and File ▸
Open Recent reopens saved projects from storage. Without a snapshot (first visit) it opens the
demo project. Render outputs are not stored: they download.

### Background jobs: Web Workers

wasm threads need shared memory, which needs a nightly `build-std` toolchain, so this build is
single-threaded per instance. Long jobs run in **Web Workers** instead, each with its own instance
of the same module (the page compiles it once and posts the `WebAssembly.Module`, so nothing is
compiled twice) and its own memory (`crates/engine/src/offload.rs`, `src/worker.rs`):

1. The session's `Offload` serializes a `WorkerRequest`: the project JSON, the footage paths it
   reads, and the job (`render` with the resolved queue items and output paths; `warp`, `camera`,
   `track`, `maskTrack`, `rotoFreeze` with the target and analysis parameters).
2. `js/host.js` takes an idle worker from its pool (or starts one), sends it the footage bytes it
   doesn't have yet, then the request.
3. The worker runs the job on a plain engine session, blocking, and posts `WorkerReply`s: render
   progress (at most every 100 ms), item started/finished, rendered files (transferred), analysis
   progress, the analysed property group, `failed`, `done`.
4. The page applies them every frame: `Session::poll_render` updates the queue items and the
   progress bar, `Session::poll_offload` writes an analysis result as one undo step (the effect /
   tracker / mask group is replaced; other edits made meanwhile are kept) and the downloads start.

Stopping a job terminates its worker (a worker can't be interrupted mid-frame): the item being
rendered becomes "User Stopped", an analysis writes nothing. `renderQueue.render {wait: true}` (the
default for agents) and analysis commands with `wait: true` still run on the page's thread and
return when done. Roto Brush propagation fills the session's segmentation cache rather than the
project, so it runs on the page's thread too, as do viewer frames (`Frames::pump`).

### Audio

Audio preview goes through **Web Audio**: an AudioWorklet (`audio-worklet.js`; a
ScriptProcessorNode where worklets are unavailable) plays interleaved stereo blocks. The mixdown
is the desktop's (`AudioPlayback`, `mix_comp`): without a feeder thread, `AudioPlayback::pump`
mixes ahead on every UI frame (≈ 350 ms queued) and the output device takes what keeps it
≈ 150 ms ahead of the playhead. A/V sync follows `AudioContext.currentTime`: the playback clock
is the frames played by the context clock minus its output latency, and the viewer shows the
frame under the audible sample, as on the desktop. The Audio panel's meters read the same feed.

Browsers start an AudioContext suspended until the user interacts with the page; the app resumes
it on the first pointer or key event, so audio plays from the first preview after any click or
key press.

### GPU

eframe starts on WebGPU where the browser has it (WebGL2 otherwise), and the GPU compositor
(`effectcraft-gpu`) runs on the same device: viewer frames are composited by compute shaders and
drawn straight from their texture, with no readback. Steps that need the CPU mid-render (3D runs,
adjustment layers, CPU-only effects) fall back to the CPU renderer for that frame, because the
page's thread can't wait for a GPU readback. The Info panel's pixel readout and the eyedroppers
read GPU frames back asynchronously (`Gpu::read_display_async`: the pixels arrive a frame
later). On WebGL2, or without a usable adapter, everything renders on the CPU.
`effectcraft.info().gpu` reports `{compositor, viewerOnGpu}`.

### Offline and install (PWA)

`manifest.webmanifest` (name, icons, standalone display) makes the app installable; `sw.js`
precaches every file of the build into a cache named after the build's hash, serves them
cache-first (the cached responses keep the server's COOP/COEP headers, so the page stays
cross-origin isolated offline), and drops older caches when a new build activates. After the
first visit the app loads without a network.

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
| `effectcraft.addFile(fileOrUrl, name?)` | put a `File`/`Blob` (or fetched URL) into the file table (stored) and open (`.ecproj`) or import it; resolves with its path |
| `effectcraft.files()` / `effectcraft.readFile(path)` | the file table `[{path, size}]` / a file's bytes (`Uint8Array`), e.g. a render |
| `effectcraft.saveToBrowser(path?)` | save the project to browser storage without downloading it (default: its path, or `/<name>.ecproj`); resolves with `{path, bytes}` |
| `effectcraft.listStored()` | `{backend, usage, quota, persisted, pending, files: [{path, size, modified}], config: [name]}` |
| `effectcraft.removeStored(path)` | delete a stored file |
| `effectcraft.flush()` | resolves once every change is written to browser storage |
| `effectcraft.info()` | graphics backend, `gpu`, `storage`, `audio` (`{state, sampleRate, backend, posted, played, underruns}`), `workers`, `restored`, `webgpu`, `serviceWorker`, version, `crossOriginIsolated`, load timings |

`window.effectcraftLoad` holds `{wasmMs, readyMs}` (module fetch + compile, and until the app runs).

```js
await effectcraft.execute("layer.newSolid", {color: "#ff8800"});
await effectcraft.request("ui.menu.invoke", {id: "renderQueue.add", params: {format: "gif", resolution: 0.25}});
await effectcraft.execute("renderQueue.render", {wait: false});  // renders in a worker; the GIF downloads
await effectcraft.saveToBrowser("/my-project.ecproj");            // File ▸ Open Recent has it after a reload
```

## Browser test

`apps/effectcraft-web/tests/smoke.mjs` drives headless Chrome over the DevTools protocol (Node ≥ 22,
no npm packages): load, the GPU path (WebGPU → GPU compositor, viewer on the GPU), the demo comp
in the viewer, `render.frame`, Render Queue GIF and PNG sequence (`.zip`) downloads, a background
render in a worker while measuring the page's event-loop gaps (must stay under 400 ms) and its
progress, the AudioContext starting on a (simulated) key press and a tone playing through it
with the meters moving, Save As and re-opening the saved project, persistence across a reload
(session snapshot, a stored project, imported media, a setting, Open Recent) and an offline
reload through the service worker. It writes screenshots and `report.json`:

```sh
cargo xtask web --serve 8765 &
node apps/effectcraft-web/tests/smoke.mjs --url http://127.0.0.1:8765/ --out target/web/smoke
```

Native unit tests cover the storage model (`apps/effectcraft-web/src/store.rs`: write
coalescing, file table, `ConfigStore` / `FileOps`, auto-save and crash recovery through the
browser store) and the worker protocol (`crates/engine/src/offload.rs`: serde round trips of
every request and reply, a render through an in-process offload).

## Gaps

- Viewer frames, RAM preview and Roto Brush propagation still render on the page's thread (one
  frame at a time between UI frames).
- GPU effects run only in the desktop GPU path: in the browser, effects run on the CPU (inside
  workers for renders) because the page's thread can't wait for mid-render readbacks.
- No disk cache (Settings ▸ Media & Disk Cache) in the browser: the layer cache stays in memory.
- Cancelling an analysis drops its partial result (the worker is terminated).
- Stored media count against the origin's storage quota; `effectcraft.listStored()` shows usage,
  `removeStored` frees it (there is no storage manager in the UI yet).
- `engine.execute` rejects the spread Render Settings / Output Module keys of `renderQueue.add`
  (`resolution`, `timeSpan`…) through its strict parameter check; use `ui.menu.invoke` as above.
