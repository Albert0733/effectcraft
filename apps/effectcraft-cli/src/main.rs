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
//! effectcraft-cli mcp [--bridge PORT]                         MCP server on stdio
//!
//! Project:  --project F.ecproj (or a positional *.ecproj) | --demo | --empty   (default: demo; mcp: empty)
//! Saving:   --save (back to --project) | --save-as F.ecproj
//! Bridge:   --bridge PORT drives a running `effectcraft --control PORT` instead of a headless session
//! Output:   --json for one compact JSON document on stdout (errors: {"error": …}, exit 1)
//! ```
//!
//! `<comp>` is a comp id or name (`-` = the active comp); `<layer>` an id, `#n` or name; `<value>`
//! is JSON (`50`, `[960,540]`, `"#ff0000"`) or a bare string. See `docs/agents.md`.

use effectcraft_automation::tools::{self, Reply};
use effectcraft_automation::{Backend, McpServer};
use serde_json::{Value, json};

const USAGE: &str = "usage: effectcraft-cli <info|commands|exec|run|props|get|set|render-frame|mcp> [args] [--json]
  info                                     project + engine summary
  commands [--filter TEXT] [--enabled]     list engine commands
  exec <command-id> [--params JSON]        run one engine command
  run <id> <json> [<id> <json> ...]        run several commands in order
  props <comp> <layer> [--flat] [--time S] a layer's property tree with paths
  get <comp> <layer> <path> [--time S]     read a property
  set <comp> <layer> <path> <value> [--time S] [--expression E]
  render-frame [--comp C] [--time S | --frame N] [--max-side PX | --scale K] [--out F.png]
  mcp [--bridge PORT]                      MCP server (JSON-RPC over stdio)
options: --project F.ecproj | --demo | --empty   --save | --save-as F   --bridge PORT   --json
<comp>: id or name, '-' = active comp; <layer>: id, '#n' or name; <value>: JSON or bare string";

/// Options that take a value.
const VALUED: &[&str] =
    &["--params", "--project", "--bridge", "--save-as", "--filter", "--time", "--frame", "--comp", "--max-side", "--scale", "--out", "--expression", "--depth"];

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
    let mut b = Backend::headless(effectcraft_host::session());
    if let Some(p) = &args.project {
        b.exec("file.open", json!({"path": p}))?;
    } else if args.flag("--demo") || (default_demo && !args.flag("--empty")) {
        b.exec("file.openDemoProject", json!({}))?;
    }
    Ok(b)
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
