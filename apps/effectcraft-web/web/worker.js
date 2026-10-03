// An EffectCraft engine instance in a Web Worker (src/worker.rs, src/frames.rs,
// crates/engine/src/offload.rs and remote.rs). The page sends the compiled module, then:
// - job workers: footage files and jobs (Render Queue, analyses, Roto Brush); the worker replies
//   with JSON messages and rendered files;
// - frame workers (`frames: true`): footage files and frame messages (project syncs as diffs,
//   render requests, Roto Brush segmentations); the worker replies with frames (pixels
//   transferred). With `gpu: true` it opens its own WebGPU device and renders GPU effects on
//   it; a frame then takes several passes, awaiting the device's readbacks in between.
// Messages are handled one at a time, in order: a frame awaiting its readbacks finishes before
// the next message (a project sync, the next request) is looked at.
import init, * as ec from "./effectcraft_web.js";

let ready = null;
let queue = Promise.resolve();

async function handle(m) {
  if (m.type === "file") ec.workerFile(m.path, m.bytes);
  else if (m.type === "job") ec.workerJob(m.json);
  else if (m.type === "frame") await ec.workerFrame(m.json);
}

self.onmessage = (e) => {
  const m = e.data;
  if (m.type === "init") {
    ready = (async () => {
      await init({ module_or_path: m.module ?? new URL("effectcraft_web_bg.wasm", m.base) });
      ec.workerInit();
      if (m.frames) await ec.workerFrameInit(!!m.gpu);
      self.postMessage({ type: "ready" });
    })();
    return;
  }
  // A failure (a panic) is re-raised outside the queue so the page's `onerror` sees it, as
  // before the queue: the page then drops the worker.
  queue = queue.then(() => ready).then(() => handle(m)).catch((err) => setTimeout(() => { throw err; }));
};
