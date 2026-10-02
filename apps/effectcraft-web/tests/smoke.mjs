// Browser smoke test of the EffectCraft web app over the Chrome DevTools Protocol.
// No npm dependencies (Node ≥ 22: global WebSocket and fetch).
//
//   cargo xtask web --serve 8765 &            # build + serve <target>/web/dist
//   node apps/effectcraft-web/tests/smoke.mjs --url http://127.0.0.1:8765/ --out /tmp/ec-web
//
// Steps: load (timing), demo comp in the viewer (screenshot, viewer pixels), `render.frame`
// through `window.effectcraft`, Render Queue GIF and PNG-sequence (.zip) exports (downloads), File ▸ Save As (download),
// re-opening the saved project through `effectcraft.addFile`. Prints a JSON report; exits non-zero
// on failure. `--headed` shows the browser.
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
  await until("!!(window.effectcraftLoad && (window.effectcraftLoad.readyMs || window.effectcraftLoad.error))", 180000);
  report.load = await js("window.effectcraftLoad");
  report.load.wallMs = Date.now() - t0;
  if (report.load.error) throw new Error(report.load.error);
  report.info = await js("effectcraft.info()");

  // The demo comp in the viewer: wait for the first viewer frame (renderMs > 0).
  const ms = await until("effectcraft.inspect().then(i => i.renderMs > 0)", 120000, 500);
  await sleep(1500);
  const insp = await js("effectcraft.inspect()");
  report.steps.viewer = { waitMs: ms, renderMs: insp.renderMs, activeComp: insp.activeComp, window: insp.window, screenshot: await shot("01-demo") };

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

  // An image sequence arrives as one .zip.
  await js(`effectcraft.execute("renderQueue.setRender", {index: 1, render: false})`);
  await js(`effectcraft.request("ui.menu.invoke", {id: "renderQueue.add", params: {format: "png", output: "seq_[####].png", resolution: 0.125, timeSpan: "custom", start: 0, end: 0.2}})`);
  await js(`effectcraft.execute("renderQueue.render", {})`);
  report.steps.pngSequence = await waitDownload((n) => n.endsWith(".zip"));

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
