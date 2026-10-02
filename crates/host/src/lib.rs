//! A fully wired [`Session`]: footage decoding through `effectcraft-media` (FilmCraft's codecs),
//! the media importer and the expression engine. Frontends (desktop, CLI, MCP, web) start here.

use std::sync::Arc;

use effectcraft_engine::{Importer, Session};
use effectcraft_project::Footage;

struct MediaImporter;

impl Importer for MediaImporter {
    fn probe(&self, path: &str) -> Result<Footage, String> {
        effectcraft_media::probe(path).map_err(|e| e.to_string())
    }
}

/// A new session with media, import and expressions enabled.
pub fn session() -> Session {
    Session {
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
}
