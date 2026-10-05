//! Roto Brush's trained models (Settings ▸ Roto Brush, `roto.model*` commands): which models the
//! registry offers (`effectcraft_segment::MODELS`, all open source), which are installed (in the
//! `models` folder next to the settings), installing one from a file or downloading the official
//! weights (verified against the registry's SHA-256 either way), choosing one, and loading it in
//! the background into the Roto Brush effect (`effects::roto::set_model`).
//!
//! Weights are never bundled: the install stays small and nothing is fetched unless asked for.
//! Downloads use the system's `curl` (shipped with Windows 10+, macOS and most Linux systems),
//! so no networking code is linked into EffectCraft; without it, the file can be downloaded in a
//! browser and installed with Install from File. The browser build segments with the built-in
//! engine only, for now.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use effectcraft_segment::{self as seg, ModelInfo};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, str_p};
use crate::{EngineError, Result, Session, cmd, query};

/// What the background worker reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// What is happening now ("Downloading MobileSAM…").
    pub busy: Option<String>,
    /// The model loaded into Roto Brush.
    pub active: Option<String>,
    /// The last failure, for the Settings page.
    pub error: Option<String>,
    /// A finished job's message, for a toast (taken by [`Session::poll_roto_models`]).
    pub done: Option<String>,
}

/// The session's model state.
#[derive(Default)]
pub struct Models {
    pub status: Arc<Mutex<Status>>,
}

impl Models {
    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn set(status: &Arc<Mutex<Status>>, f: impl FnOnce(&mut Status)) {
    if let Ok(mut s) = status.lock() {
        f(&mut s);
    }
}

impl Session {
    /// The folder installed models live in: the host's, else `models` next to the settings.
    pub fn models_dir(&self) -> Option<PathBuf> {
        self.models_dir.clone().or_else(|| Some(self.config.as_ref()?.dir()?.join("models")))
    }

    /// Where `m` is installed, if it is (a file of the published size).
    pub fn model_path(&self, m: &ModelInfo) -> Option<PathBuf> {
        let p = self.models_dir()?.join(m.file_name);
        std::fs::metadata(&p).ok().filter(|md| md.len() == m.size).map(|_| p)
    }

    /// Load (or drop) the model Settings ▸ Roto Brush chooses; called whenever settings change.
    pub fn apply_roto_model(&mut self) {
        let want = self.prefs.roto.model.clone();
        let st = self.roto_models.status();
        if want == seg::CLASSICAL || seg::info(&want).is_none() {
            if st.active.is_some() {
                crate::effects::roto::set_model(None);
                set(&self.roto_models.status, |s| s.active = None);
            }
            return;
        }
        if st.active.as_deref() == Some(want.as_str()) || st.busy.is_some() {
            return;
        }
        let Some(info) = seg::info(&want) else { return };
        let Some(path) = self.model_path(info) else {
            crate::effects::roto::set_model(None);
            set(&self.roto_models.status, |s| s.active = None);
            return;
        };
        spawn(&self.roto_models.status, format!("Loading {}…", info.name), move |status| {
            let m = load_file(&path, info.id)?;
            crate::effects::roto::set_model(Some(m));
            set(status, |s| s.active = Some(info.id.to_string()));
            Ok(format!("Roto Brush: {} loaded", info.name))
        });
    }

    /// Finish what the model worker did: a toast, and loading a model that just arrived.
    pub fn poll_roto_models(&mut self) {
        let done = self.roto_models.status.lock().ok().and_then(|mut s| s.done.take());
        if let Some(msg) = done {
            let error = msg.starts_with("Roto Brush model:");
            self.events.push(crate::Event::Toast { message: msg, error });
            self.apply_roto_model();
        }
    }
}

fn load_file(path: &Path, id: &str) -> std::result::Result<Arc<dyn seg::MaskModel>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    seg::load(id, &bytes)
}

/// Run `job` on a worker thread, reporting through `status` (desktop; the browser build has no
/// trained models yet).
fn spawn(status: &Arc<Mutex<Status>>, what: String, job: impl FnOnce(&Arc<Mutex<Status>>) -> std::result::Result<String, String> + Send + 'static) {
    set(status, |s| {
        s.busy = Some(what);
        s.error = None;
    });
    let st = status.clone();
    let run = move || {
        let r = job(&st);
        set(&st, |s| {
            s.busy = None;
            match r {
                Ok(msg) => s.done = Some(msg),
                Err(e) => {
                    s.done = Some(format!("Roto Brush model: {e}"));
                    s.error = Some(e);
                }
            }
        });
    };
    if cfg!(target_arch = "wasm32") {
        set(status, |s| {
            s.busy = None;
            s.error = Some("trained models are available in the desktop app".into());
        });
        return;
    }
    if let Err(e) = std::thread::Builder::new().name("ec-roto-model".into()).spawn(run) {
        set(status, |s| {
            s.busy = None;
            s.error = Some(format!("cannot start: {e}"));
        });
    }
}

/// Copy verified weights into the models folder (atomically).
fn install_bytes(dir: &Path, m: &ModelInfo, bytes: &[u8]) -> std::result::Result<PathBuf, String> {
    if bytes.len() as u64 != m.size || seg::sha256::hex(bytes) != m.sha256 {
        return Err(format!("this is not the published {} file (size or SHA-256 differs)", m.name));
    }
    let path = dir.join(m.file_name);
    crate::config::FileOps::write(&crate::config::StdFiles, &path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

fn model_p(p: &Value, cmd: &str) -> Result<&'static ModelInfo> {
    let id = str_p(p, "id").ok_or_else(|| bad(cmd, "missing `id`"))?;
    seg::info(id).ok_or_else(|| bad(cmd, format!("unknown model `{id}` (see roto.models)")))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.roto_models.status();
    let chosen = s.prefs.roto.model.clone();
    let mut v = vec![json!({
        "id": seg::CLASSICAL,
        "name": "Classic (graph cut)",
        "description": "Built in: colour models and graph cuts, propagated by optical flow. Always available.",
        "licence": "MIT OR Apache-2.0",
        "installed": true,
        "selected": chosen == seg::CLASSICAL,
        "active": st.active.is_none(),
    })];
    for m in seg::MODELS {
        v.push(json!({
            "id": m.id,
            "name": m.name,
            "description": m.description,
            "licence": m.licence,
            "licenceUrl": m.licence_url,
            "homepage": m.homepage,
            "url": m.url,
            "size": m.size,
            "sha256": m.sha256,
            "installed": s.model_path(m).is_some(),
            "selected": chosen == m.id,
            "active": st.active.as_deref() == Some(m.id),
        }));
    }
    Ok(json!({"models": v, "busy": st.busy, "error": st.error, "folder": s.models_dir().map(|d| d.display().to_string())}))
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_p(p, "id").ok_or_else(|| bad("roto.model.select", "missing `id`"))?;
    if id != seg::CLASSICAL && seg::info(id).is_none() {
        return Err(bad("roto.model.select", format!("unknown model `{id}` (see roto.models)")));
    }
    s.prefs.roto.model = id.to_string();
    s.save_prefs();
    s.prefs_changed();
    Ok(json!({"model": id}))
}

fn install(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "roto.model.install";
    let path = str_p(p, "path").ok_or_else(|| bad(cmd, "missing `path` (the downloaded weights file)"))?;
    let dir = s.models_dir().ok_or_else(|| bad(cmd, "no settings folder to install into"))?;
    let bytes = std::fs::read(path).map_err(|e| EngineError::Other(format!("cannot read {path}: {e}")))?;
    // Which model it is: the one asked for, or the one whose checksum it has.
    let m = match str_p(p, "id") {
        Some(_) => model_p(p, cmd)?,
        None => {
            let h = seg::sha256::hex(&bytes);
            seg::MODELS.iter().find(|m| m.sha256 == h).ok_or_else(|| bad(cmd, "this file is not a published model in the registry (SHA-256 differs)"))?
        }
    };
    let dest = install_bytes(&dir, m, &bytes).map_err(|e| bad(cmd, e))?;
    s.apply_roto_model();
    Ok(json!({"id": m.id, "path": dest.display().to_string()}))
}

fn download(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "roto.model.download";
    let m = model_p(p, cmd)?;
    let dir = s.models_dir().ok_or_else(|| bad(cmd, "no settings folder to install into"))?;
    if s.roto_models.status().busy.is_some() {
        return Err(bad(cmd, "a model is already downloading or loading"));
    }
    spawn(&s.roto_models.status, format!("Downloading {} ({} MB)…", m.name, m.size / 1_000_000), move |_| {
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let tmp = dir.join(format!("{}.part", m.file_name));
        let out = std::process::Command::new("curl")
            .args(["-fsSL", "--retry", "2", "--connect-timeout", "20", "-o"])
            .arg(&tmp)
            .arg(m.url)
            .output()
            .map_err(|e| format!("downloading needs curl ({e}); or download {} in a browser and use Install from File", m.url))?;
        if !out.status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("download failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        let bytes = std::fs::read(&tmp).map_err(|e| format!("cannot read the download: {e}"))?;
        let _ = std::fs::remove_file(&tmp);
        install_bytes(&dir, m, &bytes)?;
        Ok(format!("Roto Brush: {} downloaded and verified", m.name))
    });
    Ok(json!({"id": m.id, "downloading": true}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let m = model_p(p, "roto.model.remove")?;
    let Some(path) = s.model_path(m) else { return Ok(json!({"removed": false})) };
    if s.roto_models.status().active.as_deref() == Some(m.id) {
        crate::effects::roto::set_model(None);
        set(&s.roto_models.status, |st| st.active = None);
    }
    std::fs::remove_file(&path).map_err(|e| EngineError::Other(format!("cannot remove {}: {e}", path.display())))?;
    Ok(json!({"removed": true}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        query!("roto.models", "Roto Brush Models", "{}", list),
        cmd!("roto.model.select", "Use Roto Brush Model", [], None, "{id: classical|mobilesam} (Roto Brush 2.0 / 3.0 use it)", always, select),
        cmd!(
            "roto.model.install",
            "Install Roto Brush Model",
            [],
            None,
            "{path: weights file, id?} — verified against the registry's SHA-256",
            always,
            install
        ),
        cmd!(
            "roto.model.download",
            "Download Roto Brush Model",
            [],
            None,
            "{id} — the official weights, verified (desktop; uses the system curl)",
            always,
            download
        ),
        cmd!("roto.model.remove", "Remove Roto Brush Model", [], None, "{id}", always, remove),
    ]
}
