//! Settings ▸ Roto Brush: the model registry, choosing, installing (verified) and loading a
//! trained model into Roto Brush.

use std::sync::Arc;

use serde_json::json;

use crate::Session;
use crate::config::{ConfigStore, DirConfig};

fn session() -> (Session, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("ec-roto-models-{}-{}", std::process::id(), line!()));
    let store: Arc<dyn ConfigStore> = Arc::new(DirConfig::new(d.join("config")));
    (Session { config: Some(store), ..Default::default() }, d)
}

#[test]
fn models_are_listed_chosen_and_verified() {
    let (mut s, d) = session();
    let list = s.execute_checked("roto.models", json!({})).unwrap();
    let ids: Vec<&str> = list["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["classical", "mobilesam"]);
    let sam = &list["models"][1];
    assert_eq!((sam["licence"].as_str(), sam["installed"].as_bool()), (Some("Apache-2.0"), Some(false)));
    assert!(sam["url"].as_str().unwrap().starts_with("https://"));
    // Unknown ids and files that aren't the published weights are refused.
    assert!(s.execute_checked("roto.model.select", json!({"id": "nope"})).is_err());
    let bogus = d.join("bogus.pt");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(&bogus, b"not the weights").unwrap();
    let e = s.execute_checked("roto.model.install", json!({"path": bogus.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("SHA-256"), "{e}");
    assert!(s.execute_checked("roto.model.install", json!({"path": bogus.to_string_lossy(), "id": "mobilesam"})).is_err());
    // Choosing a model that isn't installed keeps the classic engine.
    s.execute_checked("roto.model.select", json!({"id": "mobilesam"})).unwrap();
    assert_eq!(s.prefs.roto.model, "mobilesam");
    assert!(s.roto_models.status().active.is_none());
    assert!(crate::prefs::pages().iter().any(|p| p.id == "roto"));
    // With the real weights (EFFECTCRAFT_MOBILESAM=path/to/mobile_sam.pt): install, load, use.
    if let Ok(path) = std::env::var("EFFECTCRAFT_MOBILESAM") {
        s.execute_checked("roto.model.install", json!({"path": path})).unwrap();
        for _ in 0..200 {
            s.poll_roto_models();
            if s.roto_models.status().active.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(s.roto_models.status().active.as_deref(), Some("mobilesam"));
        // Roto Brush 3.0 (the default version) now segments with it.
        let params = crate::effects::Params { values: [("version".to_string(), effectcraft_keyframe::Value::Enum(2))].into_iter().collect() };
        assert_eq!(crate::effects::roto::model_for(&params).map(|m| m.info().id), Some("mobilesam"));
        s.execute_checked("roto.model.select", json!({"id": "classical"})).unwrap();
        assert!(crate::effects::roto::model_for(&params).is_none());
    }
    let _ = std::fs::remove_dir_all(d);
}
