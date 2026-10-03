//! Persistence: the [`crate::store::Store`] mirrored to the browser's storage (the Origin
//! Private File System, IndexedDB where OPFS can't write, memory as a last resort; `js/host.js`).
//!
//! - [`load`] (before the app starts) opens the storage and reads every entry into the mirror.
//! - Changes flush in the background ([`schedule_flush`], wired as the store's change hook):
//!   one write per changed key, in order, with nothing written twice concurrently.
//! - The session snapshot ([`snapshot`]) records the open project and editor state every second
//!   or so while it changes, so a reload (or a crash) comes back to where the user was
//!   ([`restore`]).

use std::cell::Cell;

use effectcraft_engine::Session;
use effectcraft_engine::config::ConfigStore;
use effectcraft_engine::project::Project;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::files::STORE;
use crate::store::{SNAPSHOT, SNAPSHOT_META, SnapshotMeta, WebConfig};

#[wasm_bindgen(module = "/js/host.js")]
extern "C" {
    #[wasm_bindgen(js_name = storeOpen)]
    fn store_open(prefer: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = storeLoadAll)]
    fn store_load_all() -> js_sys::Promise;
    #[wasm_bindgen(js_name = storePut)]
    fn store_put(key: &str, data: js_sys::Uint8Array, modified: f64) -> js_sys::Promise;
    #[wasm_bindgen(js_name = storeDelete)]
    fn store_delete(key: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = storeEstimate)]
    pub(crate) fn store_estimate() -> js_sys::Promise;
}

thread_local! {
    static FLUSHING: Cell<bool> = const { Cell::new(false) };
    static BACKEND: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static LAST_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static FLUSH_WAITERS: std::cell::RefCell<Vec<js_sys::Function>> = const { std::cell::RefCell::new(Vec::new()) };
    static SNAP: Cell<(u64, f64)> = const { Cell::new((u64::MAX, 0.0)) };
    static LAST_META: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// The storage in use: `opfs`, `indexeddb` or `memory`.
pub fn backend() -> String {
    BACKEND.with(|b| b.borrow().clone())
}

/// The last storage error (quota exceeded…), if any.
pub fn last_error() -> Option<String> {
    LAST_ERROR.with(|e| e.borrow().clone())
}

fn js_err(e: JsValue) -> String {
    e.as_string().or_else(|| js_sys::Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| format!("{e:?}"))
}

/// Open the browser storage and read everything into the store. `prefer` forces a backend
/// (`?storage=indexeddb` / `memory`; empty = OPFS, then IndexedDB). Returns the entry count.
pub async fn load(prefer: &str) -> Result<usize, String> {
    let name = JsFuture::from(store_open(prefer)).await.map_err(js_err)?;
    BACKEND.with(|b| *b.borrow_mut() = name.as_string().unwrap_or_default());
    let all = JsFuture::from(store_load_all()).await.map_err(js_err)?;
    let all: js_sys::Array = all.dyn_into().map_err(js_err)?;
    let n = all.length() as usize;
    {
        let mut m = STORE.lock();
        for e in all.iter() {
            let get = |k: &str| js_sys::Reflect::get(&e, &k.into()).unwrap_or(JsValue::UNDEFINED);
            let Some(key) = get("key").as_string() else { continue };
            let data: js_sys::Uint8Array = match get("data").dyn_into() {
                Ok(d) => d,
                Err(_) => continue,
            };
            m.load(&key, data.to_vec().into(), get("modified").as_f64().unwrap_or(0.0));
        }
    }
    let _ = STORE.on_change.set(schedule_flush);
    Ok(n)
}

/// Write pending changes to the browser storage (in the background; one flush at a time).
pub fn schedule_flush() {
    if FLUSHING.with(|f| f.replace(true)) {
        return; // the running flush picks the new changes up
    }
    wasm_bindgen_futures::spawn_local(async {
        loop {
            let pending = STORE.lock().take_pending();
            if pending.is_empty() {
                break;
            }
            for p in pending {
                let r = match &p.data {
                    Some(d) => JsFuture::from(store_put(&p.key, js_sys::Uint8Array::from(&d[..]), p.modified)).await,
                    None => JsFuture::from(store_delete(&p.key)).await,
                };
                if let Err(e) = r {
                    let msg = format!("saving {} to browser storage failed: {}", p.key, js_err(e));
                    log::warn!("{msg}");
                    LAST_ERROR.with(|l| *l.borrow_mut() = Some(msg));
                }
            }
        }
        FLUSHING.with(|f| f.set(false));
        for w in FLUSH_WAITERS.with(|w| std::mem::take(&mut *w.borrow_mut())) {
            let _ = w.call0(&JsValue::NULL);
        }
    });
}

/// A promise that resolves once every change so far is in the browser storage.
pub fn flushed() -> js_sys::Promise {
    js_sys::Promise::new(&mut |resolve, _| {
        if !FLUSHING.with(|f| f.get()) && !STORE.lock().has_pending() {
            let _ = resolve.call0(&JsValue::NULL);
        } else {
            FLUSH_WAITERS.with(|w| w.borrow_mut().push(resolve));
            schedule_flush();
        }
    })
}

/// The config store for the session.
pub fn config() -> WebConfig {
    WebConfig::new(STORE.clone())
}

/// Record the open project and editor state when they changed (checked at most every
/// `min_secs`).
pub fn snapshot(s: &Session, now: f64, min_secs: f64) {
    let (rev, at) = SNAP.with(|c| c.get());
    if now - at < min_secs {
        return;
    }
    SNAP.with(|c| c.set((rev, now)));
    let meta = SnapshotMeta { path: s.path.clone(), dirty: s.is_dirty(), state: serde_json::to_value(&s.state).ok(), saved: 0.0 };
    let key = serde_json::to_string(&meta).unwrap_or_default();
    if rev == s.revision && LAST_META.with(|m| *m.borrow() == key) {
        return;
    }
    let c = config();
    if rev != s.revision {
        let _ = c.write(SNAPSHOT, &s.project.to_json());
    }
    let _ = c.write(SNAPSHOT_META, &serde_json::to_string(&SnapshotMeta { saved: js_sys::Date::now(), ..meta }).unwrap_or_default());
    SNAP.with(|c| c.set((s.revision, now)));
    LAST_META.with(|m| *m.borrow_mut() = key);
}

/// Reopen the last visit's project and editor state. Returns whether there was one.
pub fn restore(s: &mut Session) -> bool {
    let c = config();
    let (Some(json), Some(meta)) = (c.read(SNAPSHOT), c.read(SNAPSHOT_META)) else { return false };
    let meta: SnapshotMeta = serde_json::from_str(&meta).unwrap_or_default();
    let p = match Project::from_json(&json) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("restoring the last session: {e}");
            return false;
        }
    };
    s.replace_project(p, meta.path.clone());
    if let Some(st) = meta.state.and_then(|v| serde_json::from_value(v).ok()) {
        s.state = st;
    }
    if meta.dirty {
        // Unsaved changes stay unsaved.
        s.bump();
    }
    SNAP.with(|c| c.set((s.revision, 0.0)));
    log::info!("restored the last session ({})", meta.path.as_deref().unwrap_or("untitled"));
    true
}

/// Register persisted media with the media pool (so restored projects find their footage).
pub fn register_media(pool: &effectcraft_media::MediaPool) -> usize {
    let mut n = 0;
    for (path, _, _, persisted) in STORE.files() {
        if persisted
            && !path.to_ascii_lowercase().ends_with(".ecproj")
            && let Some(d) = STORE.file(&path)
        {
            pool.add_bytes(&path, d);
            n += 1;
        }
    }
    n
}
