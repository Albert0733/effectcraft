// Browser smoke test of the EffectCraft web app over the Chrome DevTools Protocol.
// No npm dependencies (Node ≥ 22: global WebSocket and fetch).
//
//   cargo xtask web --serve 8765 &            # build + serve <target>/web/dist
//   node apps/effectcraft-web/tests/smoke.mjs --url http://127.0.0.1:8765/ --out /tmp/ec-web
//
// Steps: load (timing), the GPU path (WebGPU → GPU compositor, viewer frames on the GPU), demo
// comp in the viewer (screenshot, viewer pixels), `render.frame` through `window.effectcraft`,
// Render Queue GIF and PNG-sequence (.zip) exports (downloads), a background Render Queue job in
// a Web Worker while the page stays responsive (event-loop gaps measured), audio (the
// AudioContext starts on a user gesture; preview playback feeds it), File ▸ Save As (download),
// re-opening the saved project through `effectcraft.addFile`, persistence across a reload (the
// session, imported media, settings, a project saved to browser storage, Open Recent) and an
// offline reload through the service worker. Prints a JSON report; exits non-zero on failure.
// `--headed` shows the browser.
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync, readdirSync, statSync, mkdtempSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const arg = (k, d) => {
  const i = process.argv.indexOf(`--${k}`);
  return i > 0 ? process.argv[i + 1] : d;
};
const url = arg("url", "http://127.0.0.1:8765/");
const out = arg("out", join(tmpdir(), "effectcraft-web-smoke"));
const chrome = arg("chrome", process.platform === "darwin" ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" : "google-chrome");
const port = Number(arg("port", "9334"));
const headless = !process.argv.includes("--headed");
mkdirSync(out, { recursive: true });
const downloads = join(out, "downloads");
mkdirSync(downloads, { recursive: true });

const profile = mkdtempSync(join(tmpdir(), "ec-chrome-"));
const proc = spawn(chrome, [
  ...(headless ? ["--headless=new"] : []),
  `--remote-debugging-port=${port}`,
  `--user-data-dir=${profile}`,
  "--no-first-run",
  "--no-default-browser-check",
  "--enable-unsafe-webgpu",
  "--window-size=1600,1000",
  "about:blank",
], { stdio: "ignore" });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let ws;
let nextId = 1;
const waiting = new Map();
const logs = [];

async function connect() {
  for (let i = 0; i < 100; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) {
        ws = new WebSocket(page.webSocketDebuggerUrl);
        await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
        ws.onmessage = (m) => {
          const msg = JSON.parse(m.data);
          if (msg.id && waiting.has(msg.id)) {
            waiting.get(msg.id)(msg);
            waiting.delete(msg.id);
          } else if (msg.method === "Runtime.consoleAPICalled") {
            logs.push(msg.params.args.map((a) => a.value ?? a.description ?? "").join(" "));
          } else if (msg.method === "Runtime.exceptionThrown") {
            logs.push("EXCEPTION " + JSON.stringify(msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text));
          }
        };
        return;
      }
    } catch {}
    await sleep(100);
  }
  throw new Error("Chrome did not start");
}

function send(method, params = {}) {
  const id = nextId++;
  ws.send(JSON.stringify({ id, method, params }));
  return new Promise((res, rej) => waiting.set(id, (m) => (m.error ? rej(new Error(`${method}: ${m.error.message}`)) : res(m.result))));
}

async function js(expr, timeout = 300000) {
  // (promise rejections carry plain strings: wrap them so the message survives)
  const wrapped = `(async () => { try { return await (${expr}); } catch (e) { throw new Error(String(e && e.message || e)); } })()`;
  const r = await send("Runtime.evaluate", { expression: wrapped, awaitPromise: true, returnByValue: true, timeout });
  if (r.exceptionDetails) throw new Error(`${expr}: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
  return r.result.value;
}

const check = (cond, msg) => {
  if (!cond) throw new Error(`check failed: ${msg}`);
};

// Wait until the app runs (after a navigation or reload).
async function ready(ms = 180000) {
  await until("!!(window.effectcraftLoad && (window.effectcraftLoad.readyMs || window.effectcraftLoad.error))", ms);
  const load = await js("window.effectcraftLoad");
  if (load.error) throw new Error(load.error);
  return load;
}

async function idle() {
  await until(`effectcraft.execute("renderQueue.list", {}).then(r => !r.rendering)`, 300000, 200);
}

async function shot(name) {
  const r = await send("Page.captureScreenshot", { format: "png" });
  const p = join(out, `${name}.png`);
  writeFileSync(p, Buffer.from(r.data, "base64"));
  return p;
}

async function until(expr, ms = 60000, step = 200) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    if (await js(expr)) return Date.now() - t0;
    await sleep(step);
  }
  throw new Error(`timeout waiting for ${expr}`);
}

async function waitDownload(pred, ms = 120000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const f = readdirSync(downloads).find((n) => pred(n) && !n.endsWith(".crdownload"));
    if (f) return { name: f, bytes: statSync(join(downloads, f)).size };
    await sleep(200);
  }
  throw new Error("no download");
}

const report = { url, steps: {} };
let failed = false;
try {
  await connect();
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Browser.setDownloadBehavior", { behavior: "allow", downloadPath: downloads }).catch(() => {});
  await send("Emulation.setDeviceMetricsOverride", { width: 1600, height: 1000, deviceScaleFactor: 1, mobile: false });
  const t0 = Date.now();
  await send("Page.navigate", { url });
  report.load = await ready();
  report.load.wallMs = Date.now() - t0;
  report.info = await js("effectcraft.info()");
  // Storage: OPFS where the browser writes it (Chrome does), else IndexedDB.
  check(["opfs", "indexeddb"].includes(report.info.storage?.backend), `storage backend ${JSON.stringify(report.info.storage)}`);

  // The demo comp in the viewer: wait for the first viewer frame (renderMs > 0).
  const ms = await until("effectcraft.inspect().then(i => i.renderMs > 0)", 120000, 500);
  await sleep(1500);
  const insp = await js("effectcraft.inspect()");
  report.steps.viewer = { waitMs: ms, renderMs: insp.renderMs, activeComp: insp.activeComp, window: insp.window, screenshot: await shot("01-demo") };

  // GPU: with WebGPU, eframe runs on it and the compositor renders viewer frames on the GPU
  // (no readback); without it, WebGL2 / CPU.
  const info = await js("effectcraft.info()");
  report.steps.gpu = { webgpu: info.webgpu, backend: info.backend, ...info.gpu };
  if (info.webgpu && info.backend === "BrowserWebGpu") {
    check(info.gpu && info.gpu.compositor, "GPU compositor on WebGPU");
    check(info.gpu.viewerOnGpu, "viewer frames on the GPU");
  }

  // Comp pixels through the API.
  const f = await js("effectcraft.renderFrame({max_side: 480}).then(r => ({w: r.width, h: r.height, pngBytes: atob(r.png).length}))");
  report.steps.renderFrame = f;

  // Render Queue: a short, small GIF → download.
  const rqT = Date.now();
  // (`ui.menu.invoke`: `engine.execute`'s strict parameter check does not know the spread
  // Render Settings / Output Module keys)
  await js(`effectcraft.request("ui.menu.invoke", {id: "renderQueue.add", params: {format: "gif", output: "smoke_[compName].gif", resolution: 0.25, timeSpan: "custom", start: 0, end: 1}})`);
  await js(`effectcraft.execute("renderQueue.render", {})`);
  const gif = await waitDownload((n) => n.endsWith(".gif"));
  report.steps.gifExport = { ...gif, ms: Date.now() - rqT };
  await idle();

  // An image sequence arrives as one .zip.
  await js(`effectcraft.execute("renderQueue.setRender", {index: 1, render: false})`);
  await js(`effectcraft.request("ui.menu.invoke", {id: "renderQueue.add", params: {format: "png", output: "seq_[####].png", resolution: 0.125, timeSpan: "custom", start: 0, end: 0.2}})`);
  await js(`effectcraft.execute("renderQueue.render", {})`);
  report.steps.pngSequence = await waitDownload((n) => n.endsWith(".zip"));
  await idle();

  // A background render (`wait: false`, what the Render button does) runs in a Web Worker: the
  // page keeps its event loop. Measure the longest gap between 10 ms timers while it renders.
  await js(`effectcraft.execute("renderQueue.setRender", {index: 2, render: false})`);
  await js(`effectcraft.request("ui.menu.invoke", {id: "renderQueue.add", params: {format: "gif", output: "bg_[compName].gif", resolution: 0.5, timeSpan: "custom", start: 0, end: 4}})`);
  report.steps.backgroundRender = await js(`(async () => {
    const t0 = performance.now();
    const r = await effectcraft.execute("renderQueue.render", {wait: false});
    const callMs = performance.now() - t0;
    let maxGapMs = 0, last = performance.now(), ticks = 0, progress = 0;
    for (;;) {
      await new Promise((res) => setTimeout(res, 10));
      const now = performance.now();
      maxGapMs = Math.max(maxGapMs, now - last);
      last = now;
      if (++ticks % 5) continue;
      const l = await effectcraft.execute("renderQueue.list", {});
      if (l.progress) progress = Math.max(progress, l.progress.done);
      if (!l.rendering) break;
    }
    return {startedRendering: r.rendering, callMs, renderMs: performance.now() - t0, maxGapMs, ticks, progressFrames: progress, workers: effectcraft.info().workers};
  })()`);
  const bg = report.steps.backgroundRender;
  bg.download = await waitDownload((n) => n.startsWith("bg_") && n.endsWith(".gif"));
  check(bg.startedRendering, "render started in the background");
  check(bg.maxGapMs < 400, `event loop blocked ${bg.maxGapMs} ms during the render`);
  check(bg.progressFrames > 0, "progress reported while rendering");

  // Audio: the AudioContext starts on a user gesture.
  report.steps.audio = { before: (await js("effectcraft.info()")).audio };
  check(report.steps.audio.before.state !== "running", "audio context not running before a gesture");
  // (a click in the window's bottom-right corner, the status bar: modifier keys alone don't
  // count as user activation)
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: 1590, y: 995, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: 1590, y: 995, button: "left", clickCount: 1 });
  await until("effectcraft.info().audio.state === 'running'", 10000);
  report.steps.audio.afterGesture = (await js("effectcraft.info()")).audio;
  // A 2 s tone in the comp; preview playback feeds the output and the meters.
  await js(`(async () => {
    const rate = 48000, n = 2 * rate, b = new ArrayBuffer(44 + n * 4), v = new DataView(b);
    const w = (o, t) => [...t].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
    w(0, "RIFF"); v.setUint32(4, 36 + n * 4, true); w(8, "WAVE"); w(12, "fmt "); v.setUint32(16, 16, true);
    v.setUint16(20, 1, true); v.setUint16(22, 2, true); v.setUint32(24, rate, true); v.setUint32(28, rate * 4, true);
    v.setUint16(32, 4, true); v.setUint16(34, 16, true); w(36, "data"); v.setUint32(40, n * 4, true);
    for (let i = 0; i < n; i++) { const x = Math.round(Math.sin(2 * Math.PI * 440 * i / rate) * 0.5 * 32767); v.setInt16(44 + 4 * i, x, true); v.setInt16(46 + 4 * i, x, true); }
    return effectcraft.addFile(new File([b], "tone.wav"));
  })()`);
  await until(`effectcraft.execute("layer.addItem", {item: "tone.wav", time: 0}).then(() => true, () => false)`, 20000, 300);
  await js(`effectcraft.request("ui.playback", {action: "stop"})`);
  await js(`effectcraft.execute("time.set", {time: 0})`);
  await js(`effectcraft.request("ui.playback", {action: "play"})`);
  await sleep(1500);
  const pb = await js(`effectcraft.request("ui.playback", {action: "status"})`);
  const au = (await js("effectcraft.info()")).audio;
  await js(`effectcraft.request("ui.playback", {action: "stop"})`);
  report.steps.audio.playing = { playback: pb, output: au };
  check(pb.audio, "audio preview running");
  check(au.posted > 0.5 * au.sampleRate, `audio fed (${au.posted} frames)`);
  check(au.played > 0, "audio clock advancing");
  check(Math.max(...(pb.levelsDb || [-99])) > -20, `meters (${JSON.stringify(pb.levelsDb)})`);

  // Save As → .ecproj download; reopen it.
  await js(`effectcraft.execute("file.saveAs", {path: "/smoke.ecproj"})`);
  const proj = await waitDownload((n) => n.endsWith(".ecproj"));
  report.steps.save = proj;
  await js(`effectcraft.execute("file.newProject", {})`);
  const opened = await js(`(async () => { const d = effectcraft.readFile("/smoke.ecproj"); const p = await effectcraft.addFile(new File([d], "smoke-copy.ecproj")); await new Promise(r => setTimeout(r, 800)); return {path: p, files: effectcraft.files()}; })()`);
  const comps = await js(`effectcraft.inspect().then(i => i.activeComp)`);
  if (!comps) throw new Error("reopened project has no active comp");
  report.steps.reopen = { ...opened, activeComp: comps };
  report.steps.final = { screenshot: await shot("02-after") };

  // Persistence: the session, imported media, settings and a project saved to browser storage
  // survive a reload; File ▸ Open Recent reopens the project.
  await js(`effectcraft.execute("layer.newSolid", {name: "Persisted Solid", color: "#22aa66"})`);
  await js(`effectcraft.execute("prefs.set", {key: "general.recentItems", value: 7})`);
  const saved = await js(`effectcraft.saveToBrowser("/persist-test.ecproj")`);
  await js(`effectcraft.execute("layer.newSolid", {name: "Unsaved Solid"})`);
  await sleep(1500); // the session snapshot is taken at most every second
  await js("effectcraft.flush()");
  const before = await js("effectcraft.listStored()");
  const tr = Date.now();
  await send("Page.reload", {});
  await sleep(500);
  const load2 = await ready();
  const after = await js("effectcraft.listStored()");
  const info2 = await js("effectcraft.info()");
  const layers = await js(`effectcraft.execute("comp.info", {}).then(c => c.layers.map(l => l.name))`);
  const recentItems = await js(`effectcraft.execute("prefs.get", {key: "general.recentItems"})`);
  const rec = await js(`effectcraft.execute("file.recoveryInfo", {})`);
  const paths = after.files.map((f) => f.path);
  report.steps.persistence = { saved, reloadMs: Date.now() - tr, load: load2, restored: info2.restored, storage: info2.storage, layers, recentItems, recent: rec.recentProjects, stored: paths, usage: after.usage, filesBefore: before.files.length };
  check(info2.restored, "session restored after reload");
  check(layers.includes("Persisted Solid") && layers.includes("Unsaved Solid"), `layers after reload: ${layers}`);
  check(paths.includes("/tone.wav") && paths.includes("/persist-test.ecproj"), `stored files: ${paths}`);
  check(recentItems === 7, `setting after reload: ${recentItems}`);
  check(rec.recentProjects[0] === "/persist-test.ecproj", `recent projects: ${rec.recentProjects}`);
  await js(`effectcraft.execute("file.openRecent", {index: 0})`);
  const reopened = await js(`effectcraft.execute("comp.info", {}).then(c => c.layers.map(l => l.name))`);
  check(reopened.includes("Persisted Solid") && !reopened.includes("Unsaved Solid"), `Open Recent: ${reopened}`);
  // The imported tone is still in the project and decodes from storage after the reload.
  const tone = await js(`effectcraft.execute("layer.addItem", {item: "tone.wav", time: 0}).then(() => "ok", (e) => String(e))`);
  check(tone === "ok", `tone.wav after reload: ${tone}`);
  report.steps.persistence.openRecent = reopened;

  // PWA: manifest, service worker in control, and an offline reload.
  const pwa = await js(`(async () => {
    const m = await (await fetch("manifest.webmanifest")).json();
    const reg = await navigator.serviceWorker.getRegistration();
    const keys = await caches.keys();
    const cached = keys.length ? (await (await caches.open(keys[0])).keys()).length : 0;
    return {manifest: m.name, icons: m.icons.length, display: m.display, sw: !!(reg && reg.active), controlled: !!navigator.serviceWorker.controller, caches: keys, cached};
  })()`);
  check(pwa.sw && pwa.cached > 4, `service worker: ${JSON.stringify(pwa)}`);
  await send("Network.enable", {});
  await send("Network.emulateNetworkConditions", { offline: true, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
  await send("Page.reload", {});
  await sleep(500);
  const offline = await ready();
  const info3 = await js("effectcraft.info()");
  await send("Network.emulateNetworkConditions", { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
  report.steps.pwa = { ...pwa, offlineLoad: offline, offlineControlled: info3.serviceWorker, crossOriginIsolated: info3.crossOriginIsolated };
  check(info3.serviceWorker, "offline reload served by the service worker");
  check(info3.crossOriginIsolated, "cross-origin isolated under the service worker");
  report.steps.final2 = { screenshot: await shot("03-offline") };
} catch (e) {
  failed = true;
  report.error = String(e && e.stack || e);
  try { report.errorShot = await shot("error"); } catch {}
}
report.console = logs.slice(-40);
writeFileSync(join(out, "report.json"), JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));
try { ws.close(); } catch {}
proc.kill();
process.exit(failed ? 1 : 0);
