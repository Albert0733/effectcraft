//! EffectCraft headless CLI.
//!
//! ```text
//! effectcraft-cli frame [project.ecproj | --demo] [--comp NAME] [--time S | --frame N] [--scale K]
//!                       [--bench N] [--bench-play N] --out out.png
//! effectcraft-cli run [project.ecproj | --demo] '<command-id>' '<json params>' …   (prints results)
//! effectcraft-cli commands                                                       (lists commands)
//! ```
//!
//! `--bench N` renders the frame N times without the layer cache and prints per-layer and
//! per-effect timings; `--bench-play N` renders N consecutive frames with and without the cache.

use effectcraft_engine::Session;
use effectcraft_engine::project::ItemId;
use effectcraft_render::{LayerCache, LayerTiming, RenderOpts, Renderer};
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
        // `--demo` or no project: the demo project.
        let _ = s.execute("file.openDemoProject", json!({}));
    }
}

fn opt<'a>(args: &'a [String], k: &str) -> Option<&'a str> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).map(String::as_str)
}

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
            if let Some(n) = opt(rest, "--bench").and_then(|v| v.parse::<usize>().ok()) {
                bench(&s, cid, t, RenderOpts { scale, ..Default::default() }, n.max(1));
            }
            if let Some(n) = opt(rest, "--bench-play").and_then(|v| v.parse::<usize>().ok()) {
                bench_play(&s, cid, t, RenderOpts { scale, ..Default::default() }, n.max(1));
            }
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
