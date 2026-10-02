//! A fully wired [`Session`]: footage decoding through `effectcraft-media` (FilmCraft's codecs),
//! the media importer, the expression engine and Render Queue export through
//! `effectcraft-export` (FilmCraft's encoders). Frontends (desktop, CLI, MCP, web) start here.

use std::sync::Arc;

use effectcraft_engine::project::render_queue::OutputFormat;
use effectcraft_engine::{ExportJob, ExportResult, Exporter, Importer, Session};
use effectcraft_project::Footage;

struct MediaImporter;

impl Importer for MediaImporter {
    fn probe(&self, path: &str) -> Result<Footage, String> {
        effectcraft_media::probe(path).map_err(|e| e.to_string())
    }
}

/// Render Queue export via `effectcraft-export`.
pub struct FileExporter;

impl Exporter for FileExporter {
    fn formats(&self) -> Vec<OutputFormat> {
        effectcraft_export::available_formats()
    }
    fn export(&self, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
        let j = effectcraft_export::Job {
            project: job.project,
            footage: job.footage,
            expr: job.expr,
            comp: job.item.comp,
            settings: &job.item.settings,
            output: &job.item.output,
            path: job.path,
        };
        match effectcraft_export::export(&j, &mut |p| progress(p.done, p.total)) {
            Ok(r) => Ok(ExportResult { path: r.path, frames: r.frames, width: r.width, height: r.height, bytes: r.bytes, seconds: r.seconds, audio: r.audio }),
            Err(effectcraft_export::ExportError::Cancelled) => Err(effectcraft_engine::render_queue::CANCELLED.into()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// A new session with media, import, expressions and export enabled.
pub fn session() -> Session {
    Session {
        exporter: Some(Arc::new(FileExporter)),
        footage: Arc::new(effectcraft_media::MediaPool::new()),
        importer: Some(Arc::new(MediaImporter)),
        expr: Some(Arc::new(effectcraft_expr::Expressions)),
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
}
