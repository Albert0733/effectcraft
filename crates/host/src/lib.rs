//! A fully wired [`Session`]: footage decoding through `effectcraft-media` (FilmCraft's codecs),
//! the media importer, the expression engine, JavaScript scripting (`effectcraft-script`) and
//! Render Queue export through `effectcraft-export` (FilmCraft's encoders). Frontends (desktop,
//! CLI, MCP, web) start here.

use std::sync::Arc;

use effectcraft_engine::project::render_queue::OutputFormat;
use effectcraft_engine::{ExportJob, ExportResult, Exporter, Importer, Session};
use effectcraft_project::Footage;
pub use effectcraft_script as script;

struct MediaImporter;

impl Importer for MediaImporter {
    fn probe(&self, path: &str) -> Result<Footage, String> {
        effectcraft_media::probe(path).map_err(|e| e.to_string())
    }
}

/// Render Queue export via `effectcraft-export`: to the file system, or to `sink` (the web app
/// turns written files into downloads).
#[derive(Default)]
pub struct FileExporter {
    pub sink: Option<Arc<effectcraft_export::Sink>>,
}

impl Exporter for FileExporter {
    fn formats(&self) -> Vec<OutputFormat> {
        effectcraft_export::available_formats()
    }
    fn export(&self, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
        let j = effectcraft_export::Job {
            project: job.project,
            footage: job.footage,
            expr: job.expr,
            accel: job.accel,
            comp: job.item.comp,
            settings: &job.item.settings,
            output: &job.item.output,
            path: job.path,
            sink: self.sink.as_deref(),
        };
        match effectcraft_export::export(&j, &mut |p| progress(p.done, p.total)) {
            Ok(r) => Ok(ExportResult { path: r.path, frames: r.frames, width: r.width, height: r.height, bytes: r.bytes, seconds: r.seconds, audio: r.audio }),
            Err(effectcraft_export::ExportError::Cancelled) => Err(effectcraft_engine::render_queue::CANCELLED.into()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// A new session with media, import, expressions, scripting and export enabled.
pub fn session() -> Session {
    Session {
        exporter: Some(Arc::new(FileExporter::default())),
        footage: Arc::new(effectcraft_media::MediaPool::new()),
        importer: Some(Arc::new(MediaImporter)),
        expr: Some(Arc::new(effectcraft_expr::Expressions)),
        expr_check: Some(effectcraft_expr::check_syntax),
        script: Some(effectcraft_script::runner),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn wired_session_renders_demo_with_expressions() {
        let mut s = super::session();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let cid = s.active_comp_id().unwrap();
        let lid = s.active_comp().unwrap().layers[1].id.0;
        s.execute("prop.setExpression", json!({"layer": lid, "path": "transform/rotation", "expression": "time * 90"})).unwrap();
        let img = s.render(cid, s.time(), effectcraft_engine::render::RenderOpts { scale: 0.25, ..Default::default() });
        assert!(img.data.iter().any(|p| p[3] > 0.5));
    }

    /// File ▸ Import of glTF/GLB/OBJ models through the media layer, placed as model layers and
    /// drawn by the Advanced 3D renderer.
    #[test]
    fn imported_models_render_in_advanced_3d() {
        let fx = |n: &str| format!("{}/../model/tests/fixtures/{n}", env!("CARGO_MANIFEST_DIR"));
        let mut s = super::session();
        s.execute("comp.new", json!({"name": "M", "width": 160, "height": 120, "renderer": "advanced3d"})).unwrap();
        let r = s.execute("file.import", json!({"paths": [fx("cube.obj"), fx("quad.glb"), fx("quad.gltf")]})).unwrap();
        assert_eq!(r["errors"], json!([]), "{r}");
        let items: Vec<u64> = r["items"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        assert_eq!(items.len(), 3);
        let cube = s.execute("layer.addItem", json!({"item": items[0]})).unwrap()["layer"].as_u64().unwrap();
        s.execute("prop.set", json!({"layer": cube, "path": "transform/rotationY", "value": 30})).unwrap();
        s.execute("prop.set", json!({"layer": cube, "path": "transform/rotationX", "value": 20})).unwrap();
        let cid = s.active_comp_id().unwrap();
        let opts = effectcraft_engine::render::RenderOpts::default();
        let img = s.render(cid, s.time(), opts);
        let covered = img.data.iter().filter(|p| p[3] > 0.99).count();
        assert!(covered > 1000, "the cube covers part of the frame: {covered}");
        // Red faces (material Red) are visible.
        assert!(img.data.iter().any(|p| p[3] > 0.99 && p[0] > 0.5 && p[1] < 0.2));
        // The textured binary glTF quad.
        for _ in 0..3 {
            s.execute("edit.undo", json!({})).unwrap();
        }
        s.execute("layer.addItem", json!({"item": items[1]})).unwrap();
        let img = s.render(cid, s.time(), opts);
        assert!(img.data.iter().filter(|p| p[3] > 0.99).count() > 1000);
        // Classic 3D doesn't draw models.
        s.execute("comp.renderer", json!({"renderer": "classic3d"})).unwrap();
        assert!(s.render(cid, s.time(), opts).data.iter().all(|p| p[3] == 0.0));
    }

    /// The web app's path: outputs go to a sink (downloads), never to the file system.
    #[test]
    fn render_queue_exports_to_a_sink() {
        use std::sync::{Arc, Mutex};
        let got: Arc<Mutex<Vec<(String, Vec<u8>)>>> = Arc::default();
        let g = got.clone();
        let mut s = super::session();
        s.exporter = Some(Arc::new(super::FileExporter { sink: Some(Arc::new(move |p: &str, d: Vec<u8>| g.lock().unwrap().push((p.to_string(), d)))) }));
        s.execute("file.openDemoProject", json!({})).unwrap();
        let dir = "/no-such-dir-effectcraft-sink-test";
        s.execute(
            "renderQueue.add",
            json!({"format": "gif", "output": format!("{dir}/a.gif"), "resolution": 0.0625, "timeSpan": "custom", "start": 0.0, "end": 0.2}),
        )
        .unwrap();
        s.execute(
            "renderQueue.add",
            json!({"format": "png", "output": format!("{dir}/seq_[##].png"), "resolution": 0.0625, "timeSpan": "custom", "start": 0.0, "end": 0.1}),
        )
        .unwrap();
        s.execute("renderQueue.render", json!({"wait": true})).unwrap();
        s.poll_render();
        let mut got = got.lock().unwrap().clone();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        let names: Vec<&str> = got.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, [format!("{dir}/a.gif"), format!("{dir}/seq_00.png"), format!("{dir}/seq_01.png"), format!("{dir}/seq_02.png")]);
        assert!(got[0].1.starts_with(b"GIF89a"));
        assert!(got[1].1.starts_with(b"\x89PNG"));
        assert!(!std::path::Path::new(dir).exists());
    }

    #[test]
    fn render_queue_exports_demo_comp() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-out/host-rq");
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = super::session();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let out = dir.join("[compName]_[width]x[height].[fileExtension]");
        let r = s
            .execute(
                "renderQueue.add",
                json!({"format": "h264", "output": out.to_string_lossy(), "resolution": 0.125, "timeSpan": "custom", "start": 0.0, "end": 0.4}),
            )
            .unwrap();
        let path = r["outputPath"].as_str().unwrap().to_string();
        assert!(path.ends_with("_240x134.mp4"), "{path}");
        // A second item: PNG sequence, rendered in the background.
        s.execute("renderQueue.add", json!({"format": "png", "output": dir.join("seq_[###].png").to_string_lossy(), "resolution": 0.0625, "timeSpan": "custom", "start": 0.0, "end": 0.2})).unwrap();
        let r = s.execute("renderQueue.render", json!({"wait": false})).unwrap();
        assert_eq!(r["items"].as_array().unwrap().len(), 2);
        let t0 = std::time::Instant::now();
        while s.is_rendering() {
            assert!(t0.elapsed().as_secs() < 300, "render timed out");
            std::thread::sleep(std::time::Duration::from_millis(20));
            s.poll_render();
        }
        s.poll_render();
        let list = s.execute("renderQueue.list", json!({})).unwrap();
        for it in list["items"].as_array().unwrap() {
            assert_eq!(it["statusLabel"], "Done", "{it}");
            assert!(it["render_time"].as_f64().is_some());
            assert!(std::path::Path::new(it["last_output"].as_str().unwrap()).exists(), "{it}");
        }
        assert!(std::path::Path::new(&path).metadata().unwrap().len() > 500);
        assert!(dir.join("seq_000.png").exists() && dir.join("seq_005.png").exists() && !dir.join("seq_006.png").exists());
        // Rendering again needs a re-queue.
        assert!(s.execute("renderQueue.render", json!({})).is_err());
        s.execute("renderQueue.setRender", json!({"index": 1, "render": true})).unwrap();
        assert_eq!(s.project.render_queue[0].status, effectcraft_engine::project::render_queue::RenderStatus::Queued);
    }

    /// Value of a property of layer `l` at time 0 (keys + expression).
    fn value(s: &effectcraft_engine::Session, l: u64, path: &str) -> effectcraft_engine::keyframe::Value {
        let cid = s.active_comp_id().unwrap();
        let comp = s.project.comp(cid).unwrap();
        let layer = comp.layer(effectcraft_engine::project::LayerId(l)).unwrap();
        let ctx =
            effectcraft_engine::render::EvalCtx { project: &s.project, comp_id: cid, comp, time: Default::default(), expr: s.expr.as_deref(), footage: None };
        ctx.value(layer, layer.props.prop(path).unwrap())
    }

    #[test]
    fn pick_whip_expressions_evaluate() {
        let mut s = super::session();
        s.execute("comp.new", json!({"name": "C", "width": 200, "height": 200, "frameRate": 30, "duration": 2})).unwrap();
        let a = s.execute("layer.newSolid", json!({"color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        let b = s.execute("layer.newSolid", json!({"name": "Src", "color": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("prop.set", json!({"layer": b, "path": "transform/rotation", "value": 33})).unwrap();
        s.execute("prop.set", json!({"layer": b, "path": "transform/position", "value": [12, 34]})).unwrap();
        s.execute("effect.apply", json!({"layer": b, "effect": "Gaussian Blur"})).unwrap();
        s.execute("prop.set", json!({"layer": b, "path": "effects/#1/blurriness", "value": 21})).unwrap();
        s.execute("layer.addMask", json!({"layer": b})).unwrap();
        s.execute("prop.set", json!({"layer": b, "path": "masks/#1/feather", "value": [7, 7]})).unwrap();
        let link = |s: &mut effectcraft_engine::Session, from: &str, to: &str| {
            s.execute("prop.pickWhip", json!({"layer": a, "path": from, "target": {"layer": b, "path": to}})).unwrap();
        };
        link(&mut s, "transform/opacity", "transform/rotation");
        assert_eq!(value(&s, a, "transform/opacity").as_f64(), 33.0);
        link(&mut s, "transform/scale", "transform/rotation");
        let cid = s.active_comp_id().unwrap();
        let ep = effectcraft_expr::eval_property(&s.project, cid, effectcraft_engine::project::LayerId(a), "transform/scale", 0.0);
        assert!(ep.is_ok(), "{ep:?}");
        assert_eq!(value(&s, a, "transform/scale").as_vec2(), [33.0, 33.0]);
        link(&mut s, "transform/position", "transform/position");
        assert_eq!(value(&s, a, "transform/position").as_vec2(), [12.0, 34.0]);
        link(&mut s, "transform/rotation", "effects/#1/blurriness");
        assert_eq!(value(&s, a, "transform/rotation").as_f64(), 21.0);
        link(&mut s, "transform/opacity", "masks/#1/feather");
        assert_eq!(value(&s, a, "transform/opacity").as_f64(), 7.0);
        // Separated dimensions: X Position reference.
        s.execute("prop.separateDimensions", json!({"layer": b})).unwrap();
        link(&mut s, "transform/rotation", "transform/positionX");
        assert_eq!(value(&s, a, "transform/rotation").as_f64(), 12.0);
    }

    /// The expression error bar's list: evaluation errors and syntax errors, with the layer and
    /// property they belong to.
    #[test]
    fn expression_errors_are_collected() {
        let mut s = super::session();
        s.execute("comp.new", json!({"name": "C", "width": 100, "height": 100, "frameRate": 30, "duration": 2})).unwrap();
        let a = s.execute("layer.newSolid", json!({"name": "Box", "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("prop.setExpression", json!({"layer": a, "path": "transform/opacity", "expression": "50"})).unwrap();
        assert_eq!(s.execute("expr.errors", json!({})).unwrap()["count"], 0);
        s.execute("prop.setExpression", json!({"layer": a, "path": "transform/rotation", "expression": "thisComp.layer(\"Nope\").rotation"})).unwrap();
        s.execute("prop.setExpression", json!({"layer": a, "path": "transform/opacity", "expression": "1 +* ("})).unwrap();
        let r = s.execute("expr.errors", json!({})).unwrap();
        assert_eq!(r["count"], 2, "{r}");
        let errs = r["errors"].as_array().unwrap();
        let rot = errs.iter().find(|e| e["path"] == "transform/rotation").unwrap();
        assert!(rot["message"].as_str().unwrap().contains("Nope"), "{rot}");
        assert_eq!(rot["layerIndex"], 1);
        assert_eq!(rot["layerName"], "Box");
        assert_eq!(rot["disabled"], false);
        let op = errs.iter().find(|e| e["path"] == "transform/opacity").unwrap();
        assert_eq!(op["disabled"], true);
        assert!(r["text"][0].as_str().unwrap().contains("of layer 1 ('Box') in comp 'C'"), "{r}");
        // A user-disabled (but valid) expression is not an error.
        s.execute("prop.setExpression", json!({"layer": a, "path": "transform/rotation", "expression": "45"})).unwrap();
        s.execute("prop.setExpression", json!({"layer": a, "path": "transform/rotation", "enabled": false})).unwrap();
        assert_eq!(s.execute("expr.errors", json!({})).unwrap()["count"], 1);
        // The Expression Language menu is categorised.
        let m = s.execute("expr.languageMenu", json!({})).unwrap();
        assert!(m.as_array().unwrap().iter().any(|c| c["category"] == "Footage" && c["items"].as_array().unwrap().iter().any(|i| i["text"] == "sourceData")));
    }

    /// File ▸ Import of JSON/CSV data files as data footage, read by expressions.
    #[test]
    fn imported_data_files_drive_expressions() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-out/host-data");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("levels.csv"), "label,value\nlow,10\nhigh,90\n").unwrap();
        std::fs::write(dir.join("cfg.json"), r#"{"angle": 33}"#).unwrap();
        std::fs::write(dir.join("broken.json"), "{nope").unwrap();
        let mut s = super::session();
        s.execute("comp.new", json!({"name": "C", "width": 100, "height": 100, "frameRate": 30, "duration": 2})).unwrap();
        let paths: Vec<String> = ["levels.csv", "cfg.json", "broken.json"].iter().map(|n| dir.join(n).to_string_lossy().to_string()).collect();
        let r = s.execute("file.import", json!({"paths": paths})).unwrap();
        assert_eq!(r["items"].as_array().unwrap().len(), 2, "{r}");
        assert_eq!(r["errors"].as_array().unwrap().len(), 1, "{r}");
        let csv = r["items"][0].as_u64().unwrap();
        assert_eq!(s.project.item(effectcraft_engine::project::ItemId(csv)).unwrap().type_name(), "Data");
        assert!(s.execute("layer.addItem", json!({"item": csv})).is_err(), "data files are not layers");
        let a = s.execute("layer.newSolid", json!({"color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("prop.setExpression", json!({"layer": a, "path": "transform/opacity", "expression": "footage(\"levels.csv\").sourceData[1].value"})).unwrap();
        s.execute("prop.setExpression", json!({"layer": a, "path": "transform/rotation", "expression": "footage(\"cfg.json\").sourceData.angle"})).unwrap();
        assert_eq!(value(&s, a, "transform/opacity").as_f64(), 90.0);
        assert_eq!(value(&s, a, "transform/rotation").as_f64(), 33.0);
    }

    /// sampleImage through the renderer: a box's Fill colour follows the solid under it.
    #[test]
    fn sample_image_drives_a_colour() {
        let mut s = super::session();
        s.execute("comp.new", json!({"name": "C", "width": 100, "height": 100, "frameRate": 30, "duration": 2})).unwrap();
        s.execute("layer.newSolid", json!({"name": "Bg", "color": "#00ff00"})).unwrap();
        let a = s.execute("layer.newSolid", json!({"name": "Fg", "color": "#ffffff", "width": 20, "height": 20})).unwrap()["layer"].as_u64().unwrap();
        s.execute("effect.apply", json!({"layer": a, "effect": "Fill"})).unwrap();
        s.execute(
            "prop.setExpression",
            json!({"layer": a, "path": "effects/#1/color", "expression": "thisComp.layer(\"Bg\").sampleImage(thisComp.layer(\"Bg\").fromComp(toComp(anchorPoint)))"}),
        )
        .unwrap();
        let cid = s.active_comp_id().unwrap();
        let img = s.render(cid, s.time(), effectcraft_engine::render::RenderOpts::default());
        let px = img.get(50, 50);
        assert!(px[1] > 0.99 && px[0] < 0.01, "the box is filled with the sampled green: {px:?}");
    }

    /// Proxies through the real media and export layers: Render Settings ▸ Proxy Use decides
    /// whether an export uses the proxy, and Create Proxy renders and attaches one.
    #[test]
    fn proxies_in_exports_and_create_proxy() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-out/host-proxy");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = |n: &str| dir.join(n).to_string_lossy().to_string();
        let mut s = super::session();
        // Source and proxy stills, written by the app itself.
        s.execute("comp.new", json!({"name": "Red", "width": 16, "height": 16, "frameRate": 10, "duration": 1})).unwrap();
        let red = s.active_comp_id().unwrap();
        s.execute("layer.newSolid", json!({"color": "#ff0000"})).unwrap();
        s.execute("comp.saveFrameAs", json!({"path": p("red.png")})).unwrap();
        s.execute("comp.new", json!({"name": "Blue", "width": 8, "height": 8, "frameRate": 10, "duration": 1})).unwrap();
        s.execute("layer.newSolid", json!({"color": "#0000ff"})).unwrap();
        s.execute("comp.saveFrameAs", json!({"path": p("blue.png")})).unwrap();
        let foot = s.execute("file.import", json!({"paths": [p("red.png")]})).unwrap()["items"][0].as_u64().unwrap();
        s.execute("file.setProxy", json!({"item": foot, "path": p("blue.png")})).unwrap();
        s.execute("comp.new", json!({"name": "Out", "width": 16, "height": 16, "frameRate": 10, "duration": 1})).unwrap();
        let out = s.active_comp_id().unwrap();
        s.execute("layer.addItem", json!({"item": foot})).unwrap();
        // Export one frame with each Proxy Use.
        for (name, using) in [("all", "use_all_[#].png"), ("none", "use_none_[#].png")] {
            s.execute(
                "renderQueue.add",
                json!({"comp": out.0, "format": "png", "output": p(using), "timeSpan": "custom", "start": 0.0, "end": 0.1, "proxyUse": name}),
            )
            .unwrap();
        }
        s.execute("renderQueue.render", json!({"wait": true})).unwrap();
        s.poll_render();
        let centre = |s: &mut effectcraft_engine::Session, path: &str| {
            let id = s.execute("file.import", json!({"paths": [path]})).unwrap()["items"][0].as_u64().unwrap();
            s.execute("comp.new", json!({"name": "Probe", "width": 16, "height": 16, "frameRate": 10, "duration": 1})).unwrap();
            s.execute("layer.addItem", json!({"item": id})).unwrap();
            let c = s.active_comp_id().unwrap();
            s.render(c, Default::default(), Default::default()).get(8, 8)
        };
        let all = centre(&mut s, &p("use_all_0.png"));
        assert!(all[2] > 0.99 && all[0] < 0.01, "Use All Proxies renders the blue proxy: {all:?}");
        let none = centre(&mut s, &p("use_none_0.png"));
        assert!(none[0] > 0.99 && none[2] < 0.01, "Use No Proxies renders the red source: {none:?}");
        // Create Proxy ▸ Still: rendered at half size and attached to the comp.
        s.execute("file.createProxy", json!({"kind": "still", "comp": red.0, "path": p("red_proxy_[#####].png")})).unwrap();
        s.execute("renderQueue.render", json!({"wait": true})).unwrap();
        s.poll_render();
        let px = s.project.item(red).unwrap().proxy.clone().expect("Create Proxy attached the rendered still");
        assert!(px.enabled);
        assert_eq!((px.footage.width, px.footage.height), (8, 8));
        assert!(std::path::Path::new(&px.footage.path).exists(), "{}", px.footage.path);
    }

    /// A script builds a comp, queues it and renders it through the Render Queue.
    #[test]
    fn script_renders_through_the_render_queue() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-out/host-script");
        let _ = std::fs::remove_dir_all(&dir);
        let out = dir.join("scripted.gif");
        let mut s = super::session();
        let code = format!(
            r#"
            var comp = app.project.items.addComp("Scripted", 64, 48, 1, 0.2, 10);
            var sq = comp.layers.addSolid([1, 0.5, 0], "Square", 16, 16, 1);
            sq.transform.position.setValueAtTime(0, [8, 24]);
            sq.transform.position.setValueAtTime(0.1, [56, 24]);
            var item = app.project.renderQueue.items.add(comp);
            item.outputModule(1).applyTemplate("GIF");
            item.outputModule(1).file = new File({});
            app.project.renderQueue.render();
            [item.status == RQItemStatus.DONE, item.outputModule(1).file.fsName, app.project.renderQueue.numItems]
            "#,
            serde_json::to_string(&out.to_string_lossy()).unwrap()
        );
        let r = s.execute("script.run", json!({"code": code})).unwrap();
        assert_eq!(r["ok"], json!(true), "{r}");
        assert_eq!(r["result"][0], json!(true), "{r}");
        assert_eq!(r["result"][2], json!(1));
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"GIF89a"));
    }

    /// importFile, layers.add(footage) and project save/open from a script.
    #[test]
    fn script_imports_footage_and_saves() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-out/host-script-import");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let q = |n: &str| serde_json::to_string(&dir.join(n).to_string_lossy()).unwrap();
        let mut s = super::session();
        let code = format!(
            r#"
            var c = app.project.items.addComp("Src", 32, 32, 1, 1, 10);
            c.layers.addSolid([0, 1, 0], "G", 32, 32, 1);
            app.run("comp.saveFrameAs", {{comp: c.id, path: {png}}});
            var still = app.project.importFile(new ImportOptions(new File({png})));
            var main = app.project.items.addComp("Main", 64, 64, 1, 1, 10);
            var l = main.layers.add(still);
            app.project.save(new File({proj}));
            [still instanceof FootageItem, still.width, still.mainSource.isStill, l.source.name, app.project.file.name]
            "#,
            png = q("frame.png"),
            proj = q("p.ecproj")
        );
        let r = s.execute("script.run", json!({"code": code})).unwrap();
        assert_eq!(r["ok"], json!(true), "{r}");
        assert_eq!(r["result"], json!([true, 32, true, "frame.png", "p.ecproj"]), "{r}");
        let r = s.execute("script.run", json!({"code": format!("app.open(new File({})); app.project.numItems", q("p.ecproj"))})).unwrap();
        assert_eq!(r["result"], json!(5), "{r}"); // Src, Solids, G, frame.png, Main
    }

    #[test]
    fn expression_syntax_errors_disable_the_expression() {
        let mut s = super::session();
        s.execute("comp.new", json!({"name": "C", "width": 100, "height": 100, "frameRate": 30, "duration": 2})).unwrap();
        let a = s.execute("layer.newSolid", json!({"color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        let r = s.execute("prop.setExpression", json!({"layer": a, "path": "transform/opacity", "expression": "1 +* ("})).unwrap();
        assert!(r["error"].is_string(), "{r}");
        let pr = s.active_comp().unwrap().layers[0].props.prop("transform/opacity").unwrap().clone();
        assert!(!pr.expr.unwrap().enabled);
        assert_eq!(value(&s, a, "transform/opacity").as_f64(), 100.0);
        s.execute("prop.setExpression", json!({"layer": a, "path": "transform/opacity", "expression": "50"})).unwrap();
        assert_eq!(value(&s, a, "transform/opacity").as_f64(), 50.0);
    }
}
