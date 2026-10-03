// An EffectCraft engine instance in a Web Worker (src/worker.rs, src/frames.rs,
// crates/engine/src/offload.rs and remote.rs). The page sends the compiled module, then:
// - job workers: footage files and jobs (Render Queue, analyses, Roto Brush); the worker replies
//   with JSON messages and rendered files;
// - frame workers: footage files and frame messages (project syncs as diffs, render requests,
//   Roto Brush segmentations); the worker replies with frames (pixels transferred).
import init, * as ec from "./effectcraft_web.js";

let ready = null;
self.onmessage = async (e) => {
  const m = e.data;
  if (m.type === "init") {
    ready = init({ module_or_path: m.module ?? new URL("effectcraft_web_bg.wasm", m.base) }).then(() => ec.workerInit());
    await ready;
    self.postMessage({ type: "ready" });
    return;
  }
  await ready;
  if (m.type === "file") ec.workerFile(m.path, m.bytes);
  else if (m.type === "job") ec.workerJob(m.json);
  else if (m.type === "frame") ec.workerFrame(m.json);
};
