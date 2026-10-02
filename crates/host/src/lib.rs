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
        expr_check: Some(effectcraft_expr::check_syntax),
        ..Session::default()
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

    /// Value of a property of layer `l` at time 0 (keys + expression).
    fn value(s: &effectcraft_engine::Session, l: u64, path: &str) -> effectcraft_engine::keyframe::Value {
        let cid = s.active_comp_id().unwrap();
        let comp = s.project.comp(cid).unwrap();
        let layer = comp.layer(effectcraft_engine::project::LayerId(l)).unwrap();
        let ctx = effectcraft_engine::render::EvalCtx { project: &s.project, comp_id: cid, comp, time: Default::default(), expr: s.expr.as_deref() };
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
