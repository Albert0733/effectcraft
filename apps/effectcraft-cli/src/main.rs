//! EffectCraft headless CLI.
//!
//! ```text
//! effectcraft-cli frame [project.ecproj | --demo] [--comp NAME] [--time S | --frame N] [--scale K] --out out.png
//! effectcraft-cli render [project.ecproj | --demo] [--comp NAME] --out FILE [--format F] [--start S] [--end S]
//!     [--work-area] [--fps N] [--resolution full|half|third|quarter|K] [--quality best|draft] [--channels rgb|rgba]
//!     [--jpeg-quality N] [--bitrate KBPS] [--prores proxy|lt|standard|hq|4444|4444xq] [--audio auto|on|off]
//! effectcraft-cli render project.ecproj --queue                                   (renders the project's Render Queue)
//! effectcraft-cli run [project.ecproj | --demo] '<command-id>' '<json params>' …   (prints results)
//! effectcraft-cli commands                                                       (lists commands)
//! ```
//!
//! `render` formats: h264 (.mp4), prores (.mov), png / jpeg / tiff / exr sequences (`name_[#####].png`), gif.
//! The format defaults to the `--out` extension; the time span to the whole comp.

use effectcraft_engine::Session;
use effectcraft_engine::project::render_queue::RenderStatus;
use effectcraft_render::RenderOpts;
use effectcraft_time::Tick;
use serde_json::{Value, json};

fn usage() -> ! {
    eprintln!(
        "usage: effectcraft-cli <frame|render|run|commands> [args]

  frame  [project.ecproj|--demo] [--comp NAME] [--time S|--frame N] [--scale K] --out out.png
  render [project.ecproj|--demo] [--comp NAME] --out FILE [--format h264|prores|png|jpeg|tiff|exr|gif]
         [--start S] [--end S] [--work-area] [--fps N] [--resolution full|half|third|quarter|K]
         [--quality best|draft] [--channels rgb|rgba] [--jpeg-quality N] [--bitrate KBPS]
         [--prores proxy|lt|standard|hq|4444|4444xq] [--audio auto|on|off]
  render project.ecproj --queue      (render the project's Render Queue)
  run    [project.ecproj|--demo] '<command-id>' '<json params>' …
  commands"
    );
    std::process::exit(2)
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("effectcraft: {msg}");
    std::process::exit(1)
}

/// Queue `--comp` (or the active comp) with the given settings unless `--queue`, then render the
/// queue with a progress line on stderr. Exit status 1 if any item fails.
fn render(s: &mut Session, rest: &[String]) {
    if !rest.iter().any(|a| a == "--queue") {
        let out = opt(rest, "--out").unwrap_or_else(|| fail("render: --out FILE is required (or --queue)"));
        let mut p = json!({"output": out});
        if let Some(c) = opt(rest, "--comp") {
            p["comp"] = json!(c);
        }
        let ext = std::path::Path::new(out).extension().map(|e| e.to_string_lossy().to_string());
        if let Some(f) = opt(rest, "--format").map(str::to_string).or(ext) {
            p["format"] = json!(f);
        }
        let num = |k: &str| opt(rest, k).map(|v| v.parse::<f64>().unwrap_or_else(|_| fail(format!("{k}: not a number: {v}"))));
        match (num("--start"), num("--end")) {
            (None, None) => p["timeSpan"] = json!(if rest.iter().any(|a| a == "--work-area") { "workArea" } else { "comp" }),
            (a, b) => {
                p["start"] = json!(a.unwrap_or(0.0));
                if let Some(b) = b {
                    p["end"] = json!(b);
                }
            }
        }
        if let Some(v) = num("--fps") {
            p["frameRate"] = json!(v);
        }
        if let Some(r) = opt(rest, "--resolution").or(opt(rest, "--scale")) {
            p["resolution"] = r.parse::<f64>().map(|v| json!(v)).unwrap_or(json!(r));
        }
        for (flag, key) in [("--quality", "quality"), ("--channels", "channels"), ("--prores", "proresProfile"), ("--audio", "audio")] {
            if let Some(v) = opt(rest, flag) {
                p[key] = json!(v);
            }
        }
        if let Some(v) = num("--jpeg-quality") {
            p["quality"] = json!(v);
        }
        if let Some(v) = num("--bitrate") {
            p["bitrate"] = json!(v);
        }
        // Render exactly this item: unqueue whatever the project's queue already holds.
        for k in 0..s.project.render_queue.len() {
            let _ = s.execute("renderQueue.setRender", json!({"index": k + 1, "render": false}));
        }
        let r = s.execute("renderQueue.add", p).unwrap_or_else(|e| fail(e));
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
    s.execute("renderQueue.render", json!({"wait": false})).unwrap_or_else(|e| fail(e));
    let tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    while s.is_rendering() {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if tty && let Some(p) = s.render_progress() {
            let left = p.remaining.map(|r| format!(", ~{r:.1}s left")).unwrap_or_default();
            eprint!("\r  [{}/{}] frame {}/{}  {:.1}s{left}\x1b[K", (p.items_done + 1).min(p.items_total), p.items_total, p.done, p.total, p.item_elapsed);
        }
        s.poll_render();
    }
    s.poll_render();
    if tty {
        eprintln!();
    }
    let mut failed = false;
    for it in s.project.render_queue.iter().filter(|i| i.render && i.started.is_some()) {
        let name = s.project.item(it.comp).map(|i| i.name.as_str()).unwrap_or("?");
        match &it.status {
            RenderStatus::Done => eprintln!("done: {name} → {} in {:.2}s", it.last_output.as_deref().unwrap_or("?"), it.render_time.unwrap_or(0.0)),
            RenderStatus::Failed(e) => {
                failed = true;
                eprintln!("failed: {name}: {e}");
            }
            other => {
                failed = true;
                eprintln!("{name}: {}", other.label());
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}

fn open(s: &mut Session, args: &[String]) {
    if let Some(p) = args.iter().find(|a| a.ends_with(".ecproj")) {
        if let Err(e) = s.execute("file.open", json!({"path": p})) {
            eprintln!("effectcraft: {e}");
            std::process::exit(1);
        }
    } else {
        // `--demo` (or no project): the built-in demo project.
        let _ = s.execute("file.openDemoProject", json!({}));
    }
}

fn opt<'a>(args: &'a [String], k: &str) -> Option<&'a str> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().cloned() else { usage() };
    let rest = &args[1..];
    let mut s = effectcraft_host::session();
    match cmd.as_str() {
        "render" => {
            open(&mut s, rest);
            render(&mut s, rest);
        }
        "frame" => {
            open(&mut s, rest);
            if let Some(c) = opt(rest, "--comp") {
                let _ = s.execute("comp.open", json!({"comp": c}));
            }
            let cid = s.active_comp_id().unwrap_or_else(|| {
                eprintln!("no composition");
                std::process::exit(1)
            });
            let comp = s.project.comp(cid).cloned().unwrap_or_else(|| std::process::exit(1));
            let t = if let Some(f) = opt(rest, "--frame").and_then(|f| f.parse().ok()) {
                comp.frame_rate.tick_of(f)
            } else {
                opt(rest, "--time").and_then(|v| v.parse().ok()).map(Tick::from_seconds_f64).unwrap_or(s.time())
            };
            let scale = opt(rest, "--scale").and_then(|v| v.parse().ok()).unwrap_or(1.0);
            let out = opt(rest, "--out").unwrap_or("frame.png");
            let t0 = std::time::Instant::now();
            let img = s.render(cid, t, RenderOpts { scale, ..Default::default() });
            let el = t0.elapsed();
            let bg = comp.background;
            let rgba = img.to_rgba8_over(bg);
            image::save_buffer(out, &rgba, img.width, img.height, image::ColorType::Rgba8).unwrap_or_else(|e| {
                eprintln!("cannot write {out}: {e}");
                std::process::exit(1)
            });
            eprintln!("rendered {}x{} at {:.3}s in {:.1} ms → {out}", img.width, img.height, t.seconds(), el.as_secs_f64() * 1000.0);
        }
        "run" => {
            open(&mut s, rest);
            let pairs: Vec<&String> = rest.iter().filter(|a| !a.ends_with(".ecproj") && *a != "--demo").collect();
            for ch in pairs.chunks(2) {
                let params: Value = ch.get(1).and_then(|p| serde_json::from_str(p).ok()).unwrap_or(json!({}));
                match s.execute(ch[0], params) {
                    Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
                    Err(e) => {
                        eprintln!("{}: {e}", ch[0]);
                        std::process::exit(1)
                    }
                }
            }
        }
        "commands" => {
            for c in effectcraft_engine::command_specs() {
                println!("{:<36} {:<40} {}", c.id, c.label, c.shortcut.unwrap_or(""));
            }
        }
        _ => usage(),
    }
}
