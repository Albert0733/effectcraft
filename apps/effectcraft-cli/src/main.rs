//! EffectCraft headless CLI.
//!
//! ```text
//! effectcraft-cli frame [project.ecproj | --demo] [--comp NAME] [--time S | --frame N] [--scale K] --out out.png
//! effectcraft-cli run [project.ecproj | --demo] '<command-id>' '<json params>' …   (prints results)
//! effectcraft-cli commands                                                       (lists commands)
//! ```

use effectcraft_engine::Session;
use effectcraft_render::RenderOpts;
use effectcraft_time::Tick;
use serde_json::{Value, json};

fn usage() -> ! {
    eprintln!("usage: effectcraft-cli <frame|run|commands> [args]  (see --help in source docs)");
    std::process::exit(2)
}

fn open(s: &mut Session, args: &[String]) {
    if let Some(p) = args.iter().find(|a| a.ends_with(".ecproj")) {
        if let Err(e) = s.execute("file.open", json!({"path": p})) {
            eprintln!("effectcraft: {e}");
            std::process::exit(1);
        }
    } else {
        // No project given (with or without --demo): open the demo project.
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
    let mut s = Session::default();
    match cmd.as_str() {
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
