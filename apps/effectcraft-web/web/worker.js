// An EffectCraft engine instance in a Web Worker: renders the Render Queue and runs analyses
// off the page's thread (src/worker.rs, crates/engine/src/offload.rs). The page sends the
// compiled module, then footage files and jobs; the worker replies with JSON messages and
// rendered files.
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
};
