//! EffectCraft headless CLI: one-shot agent commands, frame rendering and the MCP server.
//!
//! ```text
//! effectcraft-cli info                                        project + engine summary
//! effectcraft-cli commands [--filter TEXT] [--enabled]        list engine commands
//! effectcraft-cli exec <command-id> [--params JSON]           run one command
//! effectcraft-cli run <id> <json> [<id> <json> …]             run several commands in order
//! effectcraft-cli props <comp> <layer> [--flat] [--time S]    a layer's property tree (with paths)
//! effectcraft-cli get <comp> <layer> <path> [--time S]        read a property
//! effectcraft-cli set <comp> <layer> <path> <value> [--time S] [--expression E]
//! effectcraft-cli render-frame [--comp C] [--time S|--frame N] [--max-side PX|--scale K] [--out F.png]
//! effectcraft-cli render [--comp C] --out FILE [--format h264|hevc|av1|prores|webm|png|jpeg|tiff|exr|gif|wav|aiff] [--start S] [--end S]
//!     [--work-area] [--fps N] [--resolution full|half|third|quarter|K] [--quality best|draft] [--channels rgb|rgba]
//!     [--jpeg-quality N] [--bitrate KBPS] [--prores proxy|lt|standard|hq|4444|4444xq] [--audio auto|on|off]
//!     [--profile main|main10] [--level auto|4.1] [--rate-control bitrate|quality] [--video-quality 1-100]
//!     [--keyint FRAMES] [--webm-codec vp9|av1] [--audio-bitrate KBPS] [--opus-app audio|voice]
//! effectcraft-cli render F.ecproj --queue                    render the project's Render Queue
//! effectcraft-cli bench [--comp C] [--time S] [--scale K] [--n N] [--play N] [--gpu]   render timings
//!     (--gpu: CPU vs GPU ms/frame for every comp at Full and Half)
//! effectcraft-cli script FILE.jsx [F.ecproj] | --eval CODE    run an After Effects-style script
//! effectcraft-cli mcp [--bridge PORT]                         MCP server on stdio
//!
//! Project:  --project F.ecproj (or a positional *.ecproj) | --demo | --empty   (default: demo; mcp: empty)
//! Saving:   --save (back to --project) | --save-as F.ecproj
//! GPU:      --gpu renders on the GPU compositor (Mercury GPU Acceleration) when an adapter exists;
//!           the default is the CPU (Mercury Software Only)
//! Bridge:   --bridge PORT drives a running `effectcraft --control PORT` instead of a headless session
//! Output:   --json for one compact JSON document on stdout (errors: {"error": …}, exit 1)
//! ```
//!
//! `<comp>` is a comp id or name (`-` = the active comp); `<layer>` an id, `#n` or name; `<value>`
//! is JSON (`50`, `[960,540]`, `"#ff0000"`) or a bare string. See `docs/agents.md`.

use effectcraft_automation::tools::{self, Reply};
use effectcraft_automation::{Backend, McpServer};
use serde_json::{Value, json};

const USAGE: &str = "usage: effectcraft-cli <info|commands|exec|run|props|get|set|render-frame|render|script|mcp> [args] [--json]
  info                                     project + engine summary
  commands [--filter TEXT] [--enabled]     list engine commands
  exec <command-id> [--params JSON]        run one engine command
  run <id> <json> [<id> <json> ...]        run several commands in order
  props <comp> <layer> [--flat] [--time S] a layer's property tree with paths
  get <comp> <layer> <path> [--time S]     read a property
  set <comp> <layer> <path> <value> [--time S] [--expression E]
  render-frame [--comp C] [--time S | --frame N] [--max-side PX | --scale K] [--out F.png]
  render [--comp C] --out FILE [--format F] [--start S] [--end S] [--work-area] [--fps N]
         [--resolution full|half|third|quarter|K] [--quality best|draft] [--channels rgb|rgba]
         [--jpeg-quality N] [--bitrate KBPS] [--prores PROFILE] [--audio auto|on|off]
         [--profile main|main10] [--level auto|4.1] [--rate-control bitrate|quality] [--video-quality N]
         [--keyint FRAMES] [--webm-codec vp9|av1] [--audio-bitrate KBPS] [--opus-app audio|voice] | --queue
                                           (formats h264|hevc|av1|prores|webm|png|jpeg|tiff|exr|gif|wav|aiff;
                                           --profile..--keyint: HEVC / AV1, --audio-bitrate/--opus-app: WebM Opus)
  bench [--comp C] [--time S] [--scale K] [--n N] [--play N] [--gpu]   per-layer/effect render timings;
                                           --play N renders N consecutive frames with/without the layer cache;
                                           --gpu compares CPU and GPU ms/frame for every comp at Full and Half
  script FILE.jsx [F.ecproj] | --eval CODE run JavaScript with the After Effects-style object model
                                           (app.project, comps, layers, properties…); prints writeLn
                                           output and the result; errors exit 1 with file:line:col
  mcp [--bridge PORT]                      MCP server (JSON-RPC over stdio)
options: --project F.ecproj | --demo | --empty   --save | --save-as F   --bridge PORT   --json   --gpu
<comp>: id or name, '-' = active comp; <layer>: id, '#n' or name; <value>: JSON or bare string";

/// Options that take a value.
const VALUED: &[&str] = &[
    "--n",
    "--play",
    "--params",
    "--project",
    "--bridge",
    "--save-as",
    "--filter",
    "--time",
    "--frame",
    "--comp",
    "--max-side",
    "--scale",
    "--out",
    "--expression",
    "--depth",
    "--eval",
    // render
    "--format",
    "--start",
    "--end",
    "--fps",
    "--resolution",
    "--quality",
    "--channels",
    "--jpeg-quality",
    "--bitrate",
    "--prores",
    "--audio",
    "--profile",
    "--level",
    "--rate-control",
    "--video-quality",
    "--keyint",
    "--webm-codec",
    "--audio-bitrate",
    "--opus-app",
];

struct Args {
    pos: Vec<String>,
    opts: Vec<(String, Option<String>)>,
    /// `--project F` or a positional `*.ecproj`.
    project: Option<String>,
}

impl Args {
    fn parse(raw: Vec<String>) -> Result<Args, String> {
        let (mut pos, mut opts) = (vec![], vec![]);
        let mut it = raw.into_iter();
        while let Some(a) = it.next() {
            if a.starts_with("--") && a.len() > 2 {
                let (k, inline) = match a.split_once('=') {
                    Some((k, v)) => (k.to_string(), Some(v.to_string())),
                    None => (a, None),
                };
                if VALUED.contains(&k.as_str()) {
                    let v = inline.or_else(|| it.next()).ok_or_else(|| format!("{k} needs a value"))?;
                    opts.push((k, Some(v)));
                } else {
                    opts.push((k, None));
                }
            } else {
                pos.push(a);
            }
        }
        let mut a = Args { pos, opts, project: None };
        a.project = match a.opt("--project") {
            Some(p) => Some(p.to_string()),
            None => a.pos.iter().position(|x| x.ends_with(".ecproj")).map(|i| a.pos.remove(i)),
        };
        Ok(a)
    }
    fn flag(&self, k: &str) -> bool {
        self.opts.iter().any(|(o, _)| o == k)
    }
    fn opt(&self, k: &str) -> Option<&str> {
        self.opts.iter().rev().find(|(o, _)| o == k).and_then(|(_, v)| v.as_deref())
    }
    fn num(&self, k: &str) -> Result<Option<f64>, String> {
        self.opt(k).map(|v| v.parse::<f64>().map_err(|_| format!("{k}: not a number: {v}"))).transpose()
    }
}

/// A comp/layer reference: number → id, `-` → none (active), else a name / `#n`.
fn reference(s: &str) -> Option<Value> {
    match s {
        "-" | "" => None,
        _ => Some(s.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(s))),
    }
}

/// JSON if it parses, else the bare string.
fn value_arg(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| json!(s))
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if matches!(raw.first().map(String::as_str), Some("--version" | "-V" | "version")) {
        println!("effectcraft-cli {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if raw.is_empty() || raw.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        std::process::exit(if raw.is_empty() { 2 } else { 0 });
    }
    let mut args = match Args::parse(raw) {
        Ok(a) => a,
        Err(e) => fail_usage(&e),
    };
    let json_out = args.flag("--json");
    if args.pos.is_empty() {
        fail_usage("missing subcommand");
    }
    let cmd = args.pos.remove(0);
    match run(&cmd, &args, json_out) {
        Ok(()) => {}
        Err(Failure::Usage(e)) => fail_usage(&e),
        Err(Failure::Error(e)) => {
            if json_out {
                println!("{}", json!({"error": e}));
            }
            eprintln!("effectcraft-cli {cmd}: {e}");
            std::process::exit(1);
        }
    }
}

fn fail_usage(e: &str) -> ! {
    eprintln!("effectcraft-cli: {e}\n{USAGE}");
    std::process::exit(2)
}

enum Failure {
    Usage(String),
    Error(String),
}
impl From<effectcraft_automation::Error> for Failure {
    fn from(e: effectcraft_automation::Error) -> Self {
        Failure::Error(e.to_string())
    }
}
impl From<String> for Failure {
    fn from(e: String) -> Self {
        Failure::Error(e)
    }
}

fn usage_err<T>(m: &str) -> Result<T, Failure> {
    Err(Failure::Usage(m.to_string()))
}

/// Build the backend: a bridge to the app, or a headless session with the requested project.
fn backend(args: &Args, default_demo: bool) -> Result<Backend, Failure> {
    if let Some(addr) = args.opt("--bridge") {
        if args.project.is_some() || args.flag("--demo") {
            return usage_err("--bridge drives the running app's project; use `exec file.open` to open another");
        }
        return Ok(Backend::bridge(addr)?);
    }
    let mut b = Backend::headless(session(args)?);
    if let Some(p) = &args.project {
        b.exec("file.open", json!({"path": p}))?;
    } else if args.flag("--demo") || (default_demo && !args.flag("--empty")) {
        b.exec("file.openDemoProject", json!({}))?;
    }
    Ok(b)
}

/// A wired session; `--gpu` attaches the GPU compositor (renders then follow the project's
/// renderer setting, Mercury GPU Acceleration by default).
fn session(args: &Args) -> Result<effectcraft_engine::Session, Failure> {
    let mut s = effectcraft_host::session();
    if args.flag("--gpu") {
        let g = effectcraft_gpu::Gpu::headless().ok_or_else(|| Failure::Error("--gpu: no usable GPU adapter".into()))?;
        s.accel = Some(std::sync::Arc::new(g));
    }
    Ok(s)
}

/// `--save` / `--save-as` after a mutating command.
fn maybe_save(b: &mut Backend, args: &Args) -> Result<Option<String>, Failure> {
    let r = if let Some(p) = args.opt("--save-as") {
        b.exec("file.saveAs", json!({"path": p}))?
    } else if args.flag("--save") {
        b.exec("file.save", json!({})).map_err(|e| Failure::Error(format!("{e} (use --project F to save in place, or --save-as F)")))?
    } else {
        return Ok(None);
    };
    Ok(r.get("path").and_then(Value::as_str).map(str::to_string).or_else(|| args.opt("--save-as").map(str::to_string)))
}

fn tool(b: &mut Backend, name: &str, a: Value) -> Result<Value, Failure> {
    match tools::run(b, name, &a)? {
        Reply::Json(v) => Ok(v),
        Reply::Image { info, .. } => Ok(info),
    }
}

/// `script FILE.jsx [project]` / `script --eval CODE`: run JavaScript against a headless session
/// (an empty project unless one is given, or `--demo`) or the running app (`--bridge`).
fn script_cmd(args: &Args, json_out: bool) -> Result<(), Failure> {
    let (code, name) = match (args.opt("--eval"), args.pos.first()) {
        (Some(c), _) => (c.to_string(), "eval".to_string()),
        (None, Some(f)) => (std::fs::read_to_string(f).map_err(|e| Failure::Error(format!("cannot read {f}: {e}")))?, f.clone()),
        (None, None) => return usage_err("script needs a .jsx/.js file or --eval CODE"),
    };
    let mut b = backend(args, false)?;
    let r = b.exec("script.run", json!({"code": code, "name": name}))?;
    let saved = maybe_save(&mut b, args)?;
    if json_out {
        let mut o = r.clone();
        if let Some(p) = &saved {
            o["saved"] = json!(p);
        }
        emit(&o, true);
    } else {
        if let Some(out) = r["output"].as_str().filter(|o| !o.is_empty()) {
            println!("{out}");
        }
        if !r["result"].is_null() && r["error"].is_null() {
            println!("{}", r["result"]);
        }
        if let Some(p) = saved {
            eprintln!("saved {p}");
        }
    }
    if let Some(e) = r.get("error").filter(|e| !e.is_null()) {
        let pos = match (e["line"].as_u64(), e["column"].as_u64()) {
            (Some(l), Some(c)) => format!(":{l}:{c}"),
            (Some(l), None) => format!(":{l}"),
            _ => String::new(),
        };
        let msg = format!("{}{pos}: {}", e["file"].as_str().unwrap_or(&name), e["message"].as_str().unwrap_or("script error"));
        if json_out {
            std::process::exit(1);
        }
        return Err(Failure::Error(msg));
    }
    Ok(())
}

/// Print a result: compact JSON with `--json`, else pretty JSON.
fn emit(v: &Value, json_out: bool) {
    if json_out {
        println!("{v}");
    } else {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    }
}

fn run(cmd: &str, args: &Args, json_out: bool) -> Result<(), Failure> {
    match cmd {
        "render" => render(args, json_out)?,
        "bench" => bench_cmd(args)?,
        "info" => {
            let mut b = backend(args, true)?;
            let p = tool(&mut b, "get_project", json!({}))?;
            let comp = tool(&mut b, "get_comp", json!({})).ok();
            let info = json!({
                "version": env!("CARGO_PKG_VERSION"),
                "mode": if b.is_bridge() { "bridge" } else { "headless" },
                "commands": effectcraft_engine::command_specs().len(),
                "effects": effectcraft_engine::effects::registry().len(),
                "project": p,
                "activeComp": comp,
            });
            if json_out {
                emit(&info, true);
            } else {
                println!(
                    "EffectCraft {} ({}), {} commands, {} effects",
                    info["version"].as_str().unwrap_or(""),
                    info["mode"].as_str().unwrap_or(""),
                    info["commands"],
                    info["effects"]
                );
                println!("project: {}", p["path"].as_str().unwrap_or("(unsaved)"));
                for i in p["items"].as_array().into_iter().flatten() {
                    println!("  item {:>3}  {:<12} {}", i["id"], i["type"].as_str().unwrap_or(""), i["name"].as_str().unwrap_or(""));
                }
                if let Some(c) = comp {
                    println!(
                        "active comp {} \"{}\" {}x{} @ {} fps, {} s",
                        c["id"],
                        c["name"].as_str().unwrap_or(""),
                        c["width"],
                        c["height"],
                        c["frameRate"],
                        c["duration"]
                    );
                    for l in c["layers"].as_array().into_iter().flatten() {
                        println!("  #{:<3} id {:<4} {:<10} {}", l["index"], l["id"], l["type"].as_str().unwrap_or(""), l["name"].as_str().unwrap_or(""));
                    }
                }
            }
        }
        "commands" => {
            let mut b = backend(args, false)?;
            let v = tool(&mut b, "list_commands", json!({"filter": args.opt("--filter"), "enabled_only": args.flag("--enabled")}))?;
            if json_out {
                emit(&v, true);
            } else {
                for c in v.as_array().into_iter().flatten() {
                    println!(
                        "{:<36} {:<36} {:<16} {}",
                        c["id"].as_str().unwrap_or(""),
                        c["label"].as_str().unwrap_or(""),
                        c["shortcut"].as_str().unwrap_or(""),
                        c["params"].as_str().unwrap_or("")
                    );
                }
            }
        }
        "script" => script_cmd(args, json_out)?,
        "exec" => {
            let Some(id) = args.pos.first().cloned() else { return usage_err("exec needs a command id") };
            let params = match (args.opt("--params"), args.pos.get(1).map(String::as_str)) {
                (Some(p), _) | (None, Some(p)) => serde_json::from_str(p).map_err(|e| Failure::Usage(format!("params are not valid JSON: {e}")))?,
                (None, None) => json!({}),
            };
            let mut b = backend(args, true)?;
            let r = b.exec(&id, params)?;
            let saved = maybe_save(&mut b, args)?;
            emit(&with_saved(r, saved), json_out);
        }
        "run" => {
            if args.pos.is_empty() {
                return usage_err("run needs <command-id> [<json>] pairs");
            }
            let pairs = args.pos.clone();
            let mut b = backend(args, true)?;
            let mut results = vec![];
            let mut i = 0;
            while i < pairs.len() {
                let id = &pairs[i];
                let params = match pairs.get(i + 1).filter(|p| p.trim_start().starts_with('{')) {
                    Some(p) => {
                        i += 1;
                        serde_json::from_str(p).map_err(|e| Failure::Usage(format!("{id}: params are not valid JSON: {e}")))?
                    }
                    None => json!({}),
                };
                i += 1;
                let r = b.exec(id, params).map_err(|e| Failure::Error(format!("{id}: {e}")))?;
                if !json_out {
                    emit(&r, false);
                }
                results.push(json!({"command": id, "result": r}));
            }
            let saved = maybe_save(&mut b, args)?;
            if json_out {
                emit(&with_saved(json!(results), saved), true);
            }
        }
        "props" => {
            let [comp, layer] = positional::<2>(args, "props <comp> <layer>")?;
            let mut b = backend(args, true)?;
            let a = json!({"comp": reference(&comp), "layer": reference(&layer), "flat": args.flag("--flat") || !json_out, "time": args.num("--time")?, "depth": args.num("--depth")?});
            let v = tool(&mut b, "get_layer", a)?;
            if json_out {
                emit(&v, true);
            } else {
                println!("layer {} \"{}\" ({}) at {} s", v["id"], v["name"].as_str().unwrap_or(""), v["type"].as_str().unwrap_or(""), v["time"]);
                for p in v["properties"].as_array().into_iter().flatten() {
                    let mut extra = String::new();
                    if let Some(k) = p.get("keys") {
                        extra += &format!("  [{k} keys]");
                    }
                    if let Some(e) = p.get("expression").and_then(Value::as_str) {
                        extra += &format!("  expr: {e}");
                    }
                    println!("  {:<44} {:<8} {}{extra}", p["path"].as_str().unwrap_or(""), p["type"].as_str().unwrap_or(""), p["value"]);
                }
            }
        }
        "get" => {
            let [comp, layer, path] = positional::<3>(args, "get <comp> <layer> <path>")?;
            let mut b = backend(args, true)?;
            let v = tool(&mut b, "get_property", json!({"comp": reference(&comp), "layer": reference(&layer), "path": path, "time": args.num("--time")?}))?;
            emit(&v, json_out);
        }
        "set" => {
            let expr = args.opt("--expression").map(str::to_string);
            let need = if expr.is_some() { 3 } else { 4 };
            if args.pos.len() < need {
                return usage_err("set <comp> <layer> <path> <value> [--time S] [--expression E]");
            }
            let (comp, layer, path) = (args.pos[0].clone(), args.pos[1].clone(), args.pos[2].clone());
            let value = args.pos.get(3).map(|v| value_arg(v));
            let mut b = backend(args, true)?;
            let a =
                json!({"comp": reference(&comp), "layer": reference(&layer), "path": path, "value": value, "time": args.num("--time")?, "expression": expr});
            let v = tool(&mut b, "set_property", a)?;
            let saved = maybe_save(&mut b, args)?;
            emit(&with_saved(v, saved), json_out);
        }
        "render-frame" | "frame" => {
            let mut b = backend(args, true)?;
            let comp = args.opt("--comp").and_then(reference);
            let mut time = args.num("--time")?;
            let mut max_side = args.num("--max-side")?.map(|m| m as u32).unwrap_or(0);
            let needs_info = args.opt("--frame").is_some() || args.opt("--scale").is_some();
            if needs_info {
                let c = tool(&mut b, "get_comp", json!({"comp": comp}))?;
                if let Some(f) = args.num("--frame")? {
                    time = Some(f / c["frameRate"].as_f64().unwrap_or(30.0));
                }
                if let Some(k) = args.num("--scale")? {
                    let long = c["width"].as_f64().unwrap_or(0.0).max(c["height"].as_f64().unwrap_or(0.0));
                    max_side = (long * k).round().max(1.0) as u32;
                }
            }
            let out = args.opt("--out").unwrap_or("frame.png").to_string();
            let t0 = std::time::Instant::now();
            let f = b.render(comp.as_ref(), time, max_side)?;
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            std::fs::write(&out, &f.png).map_err(|e| format!("cannot write {out}: {e}"))?;
            let info = json!({"path": out, "comp": f.comp, "time": f.time, "width": f.width, "height": f.height, "ms": (ms * 10.0).round() / 10.0});
            if json_out {
                emit(&info, true);
            } else {
                eprintln!("rendered {}x{} at {:.3}s in {ms:.1} ms -> {out}", f.width, f.height, f.time);
            }
        }
        "mcp" => {
            let b = backend(args, false)?;
            McpServer::new(b).serve_stdio().map_err(|e| e.to_string())?;
        }
        other => return usage_err(&format!("unknown subcommand `{other}`")),
    }
    Ok(())
}

fn positional<const N: usize>(args: &Args, usage: &str) -> Result<[String; N], Failure> {
    if args.pos.len() < N {
        return usage_err(usage);
    }
    Ok(std::array::from_fn(|i| args.pos[i].clone()))
}

fn with_saved(v: Value, saved: Option<String>) -> Value {
    match saved {
        None => v,
        Some(p) => json!({"result": v, "saved": p}),
    }
}

/// `render`: queue `--comp` (or the active comp) with the given settings unless `--queue`, then
/// render the queue with a progress line on stderr. Fails if any item fails.
fn render(args: &Args, json_out: bool) -> Result<(), Failure> {
    use effectcraft_engine::project::render_queue::RenderStatus;
    if args.opt("--bridge").is_some() {
        return usage_err("render runs headless; use `exec renderQueue.add` / `renderQueue.render` with --bridge");
    }
    let mut s = session(args)?;
    match &args.project {
        Some(p) => s.execute("file.open", json!({"path": p})).map_err(|e| Failure::Error(e.to_string()))?,
        None => s.execute("file.openDemoProject", json!({})).map_err(|e| Failure::Error(e.to_string()))?,
    };
    let err = |e: effectcraft_engine::EngineError| Failure::Error(e.to_string());
    if !args.flag("--queue") {
        let Some(out) = args.opt("--out") else { return usage_err("render: --out FILE is required (or --queue)") };
        let mut p = json!({"output": out});
        if let Some(c) = args.opt("--comp") {
            p["comp"] = json!(c);
        }
        let ext = std::path::Path::new(out).extension().map(|e| e.to_string_lossy().to_string());
        if let Some(f) = args.opt("--format").map(str::to_string).or(ext) {
            p["format"] = json!(f);
        }
        match (args.num("--start")?, args.num("--end")?) {
            (None, None) => p["timeSpan"] = json!(if args.flag("--work-area") { "workArea" } else { "comp" }),
            (a, b) => {
                p["start"] = json!(a.unwrap_or(0.0));
                if let Some(b) = b {
                    p["end"] = json!(b);
                }
            }
        }
        if let Some(v) = args.num("--fps")? {
            p["frameRate"] = json!(v);
        }
        if let Some(r) = args.opt("--resolution").or(args.opt("--scale")) {
            p["resolution"] = r.parse::<f64>().map(|v| json!(v)).unwrap_or(json!(r));
        }
        for (flag, key) in [("--quality", "quality"), ("--channels", "channels"), ("--prores", "proresProfile"), ("--audio", "audio")] {
            if let Some(v) = args.opt(flag) {
                p[key] = json!(v);
            }
        }
        if let Some(v) = args.num("--jpeg-quality")? {
            p["quality"] = json!(v);
        }
        if let Some(v) = args.num("--bitrate")? {
            p["bitrate"] = json!(v);
        }
        for (flag, key) in [
            ("--profile", "profile"),
            ("--level", "level"),
            ("--rate-control", "rateControl"),
            ("--webm-codec", "webmCodec"),
            ("--opus-app", "opusApplication"),
        ] {
            if let Some(v) = args.opt(flag) {
                p[key] = json!(v);
            }
        }
        for (flag, key) in [("--video-quality", "quality"), ("--keyint", "keyframeInterval"), ("--audio-bitrate", "audioBitrate")] {
            if let Some(v) = args.num(flag)? {
                p[key] = json!(v);
            }
        }
        // Render exactly this item: unqueue whatever the project's queue already holds.
        for k in 0..s.project.render_queue.len() {
            let _ = s.execute("renderQueue.setRender", json!({"index": k + 1, "render": false}));
        }
        let r = s.execute("renderQueue.add", p).map_err(err)?;
        eprintln!(
            "{} → {} ({}×{}, {} frames, {})",
            r["compName"].as_str().unwrap_or("?"),
            r["outputPath"].as_str().unwrap_or("?"),
            r["width"],
            r["height"],
            r["frames"],
            r["outputModuleSummary"].as_str().unwrap_or("")
        );
    }
    s.execute("renderQueue.render", json!({"wait": false})).map_err(err)?;
    let tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    while s.is_rendering() {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if tty && let Some(p) = s.render_progress() {
            let left = p.remaining.map(|r| format!(", ~{r:.1}s left")).unwrap_or_default();
            eprint!(
                "
  [{}/{}] frame {}/{}  {:.1}s{left}\x1b[K",
                (p.items_done + 1).min(p.items_total),
                p.items_total,
                p.done,
                p.total,
                p.item_elapsed
            );
        }
        s.poll_render();
    }
    s.poll_render();
    if tty {
        eprintln!();
    }
    let mut results = vec![];
    let mut failed = vec![];
    for it in s.project.render_queue.iter().filter(|i| i.render && i.started.is_some()) {
        let name = s.project.item(it.comp).map(|i| i.name.clone()).unwrap_or_else(|| "?".into());
        match &it.status {
            RenderStatus::Done => {
                eprintln!("done: {name} → {} in {:.2}s", it.last_output.as_deref().unwrap_or("?"), it.render_time.unwrap_or(0.0));
                results.push(json!({"comp": name, "output": it.last_output, "seconds": it.render_time}));
            }
            RenderStatus::Failed(e) => failed.push(format!("{name}: {e}")),
            other => failed.push(format!("{name}: {}", other.label())),
        }
    }
    if !failed.is_empty() {
        return Err(Failure::Error(failed.join("; ")));
    }
    if json_out {
        emit(&json!({"rendered": results}), true);
    }
    Ok(())
}

/// `bench`: time one frame N times without the layer cache (per-layer and per-effect breakdown),
/// and optionally `--play N` consecutive frames with and without the cache.
fn bench_cmd(args: &Args) -> Result<(), Failure> {
    let mut s = effectcraft_host::session();
    match &args.project {
        Some(p) => s.execute("file.open", json!({"path": p})),
        None => s.execute("file.openDemoProject", json!({})),
    }
    .map_err(|e| Failure::Error(e.to_string()))?;
    if let Some(c) = args.opt("--comp") {
        s.execute("comp.open", json!({"comp": c})).map_err(|e| Failure::Error(e.to_string()))?;
    }
    if args.flag("--gpu") {
        return bench_gpu(&s, args);
    }
    let cid = s.active_comp_id().ok_or_else(|| Failure::Error("no composition".into()))?;
    let t = Tick::from_seconds_f64(args.num("--time")?.unwrap_or(3.0));
    let opts = RenderOpts { scale: args.num("--scale")?.unwrap_or(1.0), ..Default::default() };
    bench(&s, cid, t, opts, args.num("--n")?.unwrap_or(10.0).max(1.0) as usize);
    if let Some(n) = args.num("--play")? {
        bench_play(&s, cid, t, opts, n.max(1.0) as usize);
    }
    Ok(())
}

// ---------------------------------------------------------------- benchmark helpers

use effectcraft_engine::Session;
use effectcraft_engine::project::ItemId;
use effectcraft_render::{LayerCache, LayerTiming, RenderOpts, Renderer};
use effectcraft_time::Tick;

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    if v.is_empty() { 0.0 } else { v[v.len() / 2] }
}

fn min(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::INFINITY, f64::min)
}

/// CPU time of the whole process (all threads) in ms: a load-independent measure of work.
fn cpu_ms() -> f64 {
    cpu_time::ProcessTime::try_now().map(|t| t.as_duration().as_secs_f64() * 1e3).unwrap_or(0.0)
}

/// One frame: (wall ms, CPU ms, per-layer timings).
fn render_timed(s: &Session, cid: ItemId, t: Tick, opts: RenderOpts, cache: Option<&LayerCache>) -> (f64, f64, Vec<LayerTiming>) {
    let prof = std::sync::Mutex::new(Vec::<LayerTiming>::new());
    let mut r = Renderer::new(&s.project, s.footage.as_ref(), opts);
    r.expr = s.expr.as_deref();
    r.cache = cache;
    r.profile = Some(&prof);
    let c0 = cpu_ms();
    let t0 = std::time::Instant::now();
    let img = r.comp_frame(cid, t);
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    let cpu = cpu_ms() - c0;
    std::hint::black_box(&img);
    (ms, cpu, prof.into_inner().unwrap_or_default())
}

type LayerRow = (usize, String, Vec<f64>, Vec<f64>, Vec<(String, Vec<f64>)>);

/// `frame --bench N`: render the same frame N times without the layer cache (the full cost of a
/// frame) and print min/median frame time plus min per-layer and per-effect times.
fn bench(s: &Session, cid: ItemId, t: Tick, opts: RenderOpts, n: usize) {
    let mut totals = Vec::with_capacity(n);
    let mut cpus = Vec::with_capacity(n);
    let mut per_layer: Vec<LayerRow> = Vec::new();
    for _ in 0..n {
        let (ms, cpu, prof) = render_timed(s, cid, t, opts, None);
        totals.push(ms);
        cpus.push(cpu);
        for (i, lt) in prof.into_iter().enumerate() {
            if per_layer.len() <= i {
                per_layer.push((lt.depth, lt.layer.clone(), vec![], vec![], vec![]));
            }
            let e = &mut per_layer[i];
            e.2.push(lt.process_ms);
            e.3.push(lt.composite_ms);
            for (j, (id, ms)) in lt.effects.into_iter().enumerate() {
                if e.4.len() <= j {
                    e.4.push((id, vec![]));
                }
                e.4[j].1.push(ms);
            }
        }
    }
    eprintln!(
        "bench (no cache): {n} runs at scale {}: wall min {:.2} ms, median {:.2} ms; CPU (all threads) min {:.2} ms",
        opts.scale,
        min(&totals),
        median(&mut totals),
        min(&cpus)
    );
    for (d, name, p, c, fx) in per_layer {
        eprintln!("  {:indent$}{name:<28} process {:8.2} ms  composite {:8.2} ms  (min)", "", min(&p), min(&c), indent = d * 2);
        for (id, v) in fx {
            eprintln!("  {:indent$}  fx {id:<30} {:8.2} ms", "", min(&v), indent = d * 2);
        }
    }
}

/// `frame --bench-play N`: render N consecutive frames from `t` as playback does, without and
/// with the layer cache, and print per-frame times.
fn bench_play(s: &Session, cid: ItemId, t: Tick, opts: RenderOpts, n: usize) {
    let Some(comp) = s.project.comp(cid) else { return };
    let f0 = comp.frame_rate.frame_at(t);
    let cache = LayerCache::default();
    for (label, c) in [("no cache", None), ("layer cache", Some(&cache))] {
        let runs: Vec<(f64, f64)> = (0..n as i64)
            .map(|i| {
                let (ms, cpu, _) = render_timed(s, cid, comp.frame_rate.tick_of(f0 + i), opts, c);
                (ms, cpu)
            })
            .collect();
        let mut v: Vec<f64> = runs.iter().map(|r| r.0).collect();
        let mean = v.iter().sum::<f64>() / n as f64;
        let cpu = runs.iter().map(|r| r.1).sum::<f64>() / n as f64;
        eprintln!(
            "bench-play ({label}): {n} frames at scale {}: wall mean {mean:.2} ms, median {:.2} ms, min {:.2} ms; CPU mean {cpu:.2} ms",
            opts.scale,
            median(&mut v),
            min(&v)
        );
    }
    let st = cache.stats();
    eprintln!("  layer cache: {} hits, {} misses, {} entries, {:.1} MB", st.hits, st.misses, st.entries, st.bytes as f64 / 1e6);
}

/// Median wall time (ms) of `n` runs of `f` after one warm-up run.
fn time_ms(n: usize, mut f: impl FnMut()) -> f64 {
    f();
    let mut v: Vec<f64> = (0..n)
        .map(|_| {
            let t0 = std::time::Instant::now();
            f();
            t0.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    median(&mut v)
}

/// `bench --gpu`'s adjustment-layer comp: a copy of the first comp with a full-frame adjustment
/// layer on top running Gaussian Blur, Levels, Vibrance and Glow (all GPU effects).
fn adjustment_bench_comp(p: &mut effectcraft_engine::project::Project) -> Option<ItemId> {
    use effectcraft_engine::effects;
    use effectcraft_engine::keyframe::Value;
    use effectcraft_engine::project::build::{self, Ids};
    use effectcraft_engine::project::{ItemKind, LayerSource, Solid};
    let (_, base) = p.comps().next().map(|(id, c)| (*id, c.clone()))?;
    let mut comp = base.clone();
    let (w, h) = (comp.width, comp.height);
    let sid = p.add_item(
        "Adjustment (bench)",
        effectcraft_engine::color::Label::None,
        None,
        ItemKind::Solid(Solid { color: [1.0; 3], width: w, height: h, pixel_aspect: 1.0 }),
    );
    let mut l = build::layer(p, &comp, "Adjustment (bench)", LayerSource::Solid { item: sid }, (w, h), None);
    l.switches.adjustment = true;
    for (id, vals) in [
        ("ec.blur.gaussian", vec![("blurriness", Value::Scalar(12.0))]),
        ("ec.color.levels", vec![("gamma", Value::Scalar(1.3))]),
        ("ec.color.vibrance", vec![("vibrance", Value::Scalar(40.0))]),
        ("ec.stylize.glow", vec![("threshold", Value::Scalar(55.0))]),
    ] {
        let spec = effects::find(id)?;
        let mut next = p.next_id;
        let mut g = effects::instantiate(spec, &mut Ids(&mut next), spec.name, [w as f64, h as f64]);
        p.next_id = next;
        for (k, v) in vals {
            g.prop_mut(k)?.value = v;
        }
        l.props.sub_mut("effects")?.children.push(g.into());
    }
    comp.layers.insert(0, l);
    Some(p.add_item("Adjustment Layers (bench)", effectcraft_engine::color::Label::None, None, ItemKind::Comp(comp.into())))
}

/// `bench --gpu`: CPU vs GPU ms/frame for every comp (or `--comp`) at Full and Half. "cold"
/// renders everything (no layer cache); "warm" reuses the layer cache (playback / scrubbing of
/// unchanged layers: compositing cost); "viewer" is the GPU display path without readback.
fn bench_gpu(s: &Session, args: &Args) -> Result<(), Failure> {
    use effectcraft_render::Backend;
    let gpu = effectcraft_gpu::Gpu::headless().ok_or_else(|| Failure::Error("--gpu: no usable GPU adapter".into()))?;
    let n = args.num("--n")?.unwrap_or(10.0).max(1.0) as usize;
    // Every comp, plus an adjustment-layer comp built for the benchmark (the main comp under a
    // full-frame adjustment layer with a GPU effect stack).
    let mut project = (*s.project).clone();
    let comps: Vec<ItemId> = match args.opt("--comp") {
        Some(_) => s.active_comp_id().into_iter().collect(),
        None => {
            let mut v: Vec<ItemId> = s.project.comps().map(|(id, _)| *id).collect();
            v.extend(adjustment_bench_comp(&mut project));
            v
        }
    };
    let project = &project;
    eprintln!("GPU: {} — median of {n} runs (ms/frame)", effectcraft_render::Accelerator::name(&gpu));
    eprintln!(
        "{:<28} {:>5} {:>10} {:>9} {:>9} {:>9} {:>9} {:>9} {:>8} {:>9}",
        "comp", "res", "size", "cpu cold", "gpu cold", "cpu warm", "gpu warm", "gpu view", "speedup", "gpu≠cpu"
    );
    for cid in comps {
        let Some(comp) = project.comp(cid) else { continue };
        let name = project.item(cid).map(|i| i.name.clone()).unwrap_or_default();
        let t = Tick::from_seconds_f64(args.num("--time")?.unwrap_or(3.0).min(comp.duration.seconds() * 0.5));
        for (label, scale) in [("Full", 1.0), ("Half", 0.5)] {
            let mk = |backend: Backend, cache: Option<&LayerCache>| {
                let mut r = Renderer::new(project, s.footage.as_ref(), RenderOpts { scale, backend, ..Default::default() });
                r.expr = s.expr.as_deref();
                r.cache = cache;
                r.accel = Some(&gpu);
                r.comp_frame(cid, t)
            };
            let cpu_cold = time_ms(n, || {
                std::hint::black_box(mk(Backend::Cpu, None));
            });
            let gpu_cold = time_ms(n, || {
                std::hint::black_box(mk(Backend::Gpu, None));
            });
            let cache = LayerCache::default();
            let cpu_warm = time_ms(n, || {
                std::hint::black_box(mk(Backend::Cpu, Some(&cache)));
            });
            let gcache = LayerCache::default();
            let gpu_warm = time_ms(n, || {
                std::hint::black_box(mk(Backend::Gpu, Some(&gcache)));
            });
            let view = time_ms(n, || {
                let mut r = Renderer::new(project, s.footage.as_ref(), RenderOpts { scale, backend: Backend::Gpu, ..Default::default() });
                r.expr = s.expr.as_deref();
                r.cache = Some(&gcache);
                r.accel = Some(&gpu);
                std::hint::black_box(gpu.render_display(&r, cid, t));
                gpu.wait();
            });
            let size = format!("{}x{}", (comp.width as f64 * scale).round(), (comp.height as f64 * scale).round());
            // Agreement: share of pixels where the GPU frame differs from the CPU reference by
            // more than 1/255 on any channel.
            let (a, b) = (mk(Backend::Cpu, Some(&cache)), mk(Backend::Gpu, Some(&gcache)));
            let off = a.data.iter().zip(&b.data).filter(|(p, q)| (0..4).any(|c| (p[c] - q[c]).abs() > 1.0 / 255.0 + 1e-6)).count();
            let pct = 100.0 * off as f64 / a.data.len().max(1) as f64;
            let speedup = cpu_warm / gpu_warm.max(1e-9);
            eprintln!(
                "{name:<28} {label:>5} {size:>10} {cpu_cold:>9.2} {gpu_cold:>9.2} {cpu_warm:>9.2} {gpu_warm:>9.2} {view:>9.2} {speedup:>7.2}x {pct:>8.3}%"
            );
        }
    }
    Ok(())
}
