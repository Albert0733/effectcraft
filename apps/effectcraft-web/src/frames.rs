//! Viewer frames rendered in Web Workers ([`effectcraft_engine::remote`]).
//!
//! **Page side** — [`WebFrames`], the frame cache's [`RemoteFrames`]: a few long-lived frame
//! workers (`js/host.js` `frameWorkerStart`), each holding a replica of the project. Before a
//! render, the worker gets the footage files it lacks and a sync (the whole project the first
//! time, then diffs: [`Mirror`]); the frame comes back as premultiplied RGBA8 in a transferred
//! buffer. One frame per worker at a time, so the page always decides what renders next (the
//! viewer's frame before prefetch). A worker that fails is dropped; with none left, frames render
//! on the page's thread again (`Frames::pump`).
//!
//! **Worker side** — [`worker_frame`]: the worker's [`FrameServer`] (footage from the worker's
//! media pool, expressions) handles each message and posts its replies.

use std::cell::RefCell;
use std::collections::HashSet;

use effectcraft_engine::offload::footage_files;
use effectcraft_engine::remote::{FrameMsg, FrameReply, FrameServer, Mirror};
use effectcraft_ui_egui::frames::{RemoteDone, RemoteFrame, RemoteFrames, RemoteJob};
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/js/host.js")]
extern "C" {
    #[wasm_bindgen(js_name = frameWorkerStart)]
    fn frame_worker_start(base: &str, on_message: &js_sys::Function) -> u32;
    #[wasm_bindgen(js_name = frameWorkerPost)]
    fn frame_worker_post(index: u32, msg: &JsValue, transfer: &js_sys::Array);
}

/// One frame worker as the page sees it.
struct Fw {
    index: u32,
    mirror: Mirror,
    files: HashSet<String>,
    busy: Option<(u64, RemoteDone)>,
    alive: bool,
}

#[derive(Default)]
struct State {
    workers: Vec<Fw>,
    next_id: u64,
    rendered: u64,
    failed: u64,
    last_ms: f64,
    /// Bytes of project sync messages sent (diffs keep this small).
    sync_bytes: u64,
    syncs: u64,
}

type OnMessage = Closure<dyn FnMut(u32, JsValue, JsValue, JsValue)>;

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static HANDLER: RefCell<Option<OnMessage>> = const { RefCell::new(None) };
    /// The worker's frame server (worker side).
    static SERVER: RefCell<Option<FrameServer>> = const { RefCell::new(None) };
}

fn obj(pairs: &[(&str, JsValue)]) -> JsValue {
    let o = js_sys::Object::new();
    for (k, v) in pairs {
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str(k), v);
    }
    o.into()
}

/// The page's frame workers (see the module docs).
pub struct WebFrames;

impl WebFrames {
    /// Start `n` frame workers.
    pub fn start(n: usize) -> WebFrames {
        let on: OnMessage = Closure::new(|index: u32, json: JsValue, bytes: JsValue, error: JsValue| on_message(index, json, bytes, error));
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            for _ in 0..n {
                let index = frame_worker_start(&crate::page_url(""), on.as_ref().unchecked_ref());
                s.workers.push(Fw { index, mirror: Mirror::default(), files: HashSet::new(), busy: None, alive: true });
            }
        });
        HANDLER.with(|h| *h.borrow_mut() = Some(on));
        WebFrames
    }
}

impl RemoteFrames for WebFrames {
    fn slots(&self) -> usize {
        STATE.with(|s| s.borrow().workers.iter().filter(|w| w.alive).count())
    }

    fn purge(&self) {
        purge();
    }

    fn start(&self, job: RemoteJob, done: RemoteDone) {
        let r = STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.next_id += 1;
            let id = s.next_id;
            let Some(k) = s.workers.iter().position(|w| w.alive && w.busy.is_none()) else { return Err((done, "no idle frame worker".to_string())) };
            // Footage the worker hasn't got yet.
            let w = &mut s.workers[k];
            for p in footage_files(&job.project) {
                if w.files.contains(&p) {
                    continue;
                }
                if let Some(d) = crate::files::get(&p) {
                    let bytes = js_sys::Uint8Array::from(&d[..]);
                    frame_worker_post(w.index, &obj(&[("type", "file".into()), ("path", p.as_str().into()), ("bytes", bytes.into())]), &js_sys::Array::new());
                    w.files.insert(p);
                }
            }
            let sync = w.mirror.sync(job.revision, &job.project);
            let index = w.index;
            let mut sent = 0;
            if let Some(m) = sync {
                let t = serde_json::to_string(&m).unwrap_or_default();
                sent = t.len() as u64;
                frame_worker_post(index, &obj(&[("type", "frame".into()), ("json", t.into())]), &js_sys::Array::new());
            }
            let req = FrameMsg::Render { id, revision: job.revision, comp: job.comp, time: job.t, opts: Box::new(job.opts) };
            frame_worker_post(
                index,
                &obj(&[("type", "frame".into()), ("json", serde_json::to_string(&req).unwrap_or_default().into())]),
                &js_sys::Array::new(),
            );
            s.workers[k].busy = Some((id, done));
            if sent > 0 {
                s.syncs += 1;
                s.sync_bytes += sent;
            }
            Ok(())
        });
        if let Err((done, e)) = r {
            done(Err(e));
        }
    }
}

/// A message from frame worker `index`: a JSON [`FrameReply`] (with the pixels in `bytes`), or
/// `error` when the worker died.
fn on_message(index: u32, json: JsValue, bytes: JsValue, error: JsValue) {
    let done = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let st = &mut *s;
        let w = st.workers.iter_mut().find(|w| w.index == index)?;
        if let Some(e) = error.as_string() {
            log::warn!("frame worker {index}: {e}; frames render on the page's thread if none is left");
            w.alive = false;
            return w.busy.take().map(|(_, d)| (d, Err(e)));
        }
        let reply: FrameReply = serde_json::from_str(&json.as_string()?).ok()?;
        match reply {
            FrameReply::Frame { id, width, height, ms } => {
                let (_, d) = w.busy.take_if(|b| b.0 == id)?;
                let rgba = bytes.dyn_into::<js_sys::Uint8Array>().map(|b| b.to_vec()).unwrap_or_default();
                st.rendered += 1;
                st.last_ms = ms;
                Some((d, Ok(RemoteFrame { width, height, rgba, ms })))
            }
            FrameReply::Failed { id, error, .. } => {
                st.failed += 1;
                let (_, d) = w.busy.take_if(|b| b.0 == id)?;
                Some((d, Err(error)))
            }
            FrameReply::Resync { error } => {
                log::info!("frame worker {index}: {error}: sending the whole project again");
                w.mirror.reset();
                None
            }
        }
    });
    if let Some((d, r)) = done {
        d(r);
    }
    crate::repaint();
}

/// Relay Roto Brush segmentations (from a propagation worker) to the frame workers, so their
/// renders use them instead of computing them again.
pub fn relay_segs(segs: &[effectcraft_engine::offload::SegData]) {
    if segs.is_empty() {
        return;
    }
    let t = serde_json::to_string(&FrameMsg::Segs { segs: segs.to_vec() }).unwrap_or_default();
    STATE.with(|s| {
        for w in s.borrow().workers.iter().filter(|w| w.alive) {
            frame_worker_post(w.index, &obj(&[("type", "frame".into()), ("json", t.as_str().into())]), &js_sys::Array::new());
        }
    });
}

/// Ask every frame worker to drop its cached layer buffers (Edit ▸ Purge).
pub fn purge() {
    let t = serde_json::to_string(&FrameMsg::Purge).unwrap_or_default();
    STATE.with(|s| {
        for w in s.borrow().workers.iter().filter(|w| w.alive) {
            frame_worker_post(w.index, &obj(&[("type", "frame".into()), ("json", t.as_str().into())]), &js_sys::Array::new());
        }
    });
}

/// `effectcraft.info().frameWorkers`.
pub fn stats() -> Value {
    STATE.with(|s| {
        let s = s.borrow();
        json!({
            "workers": s.workers.len(),
            "alive": s.workers.iter().filter(|w| w.alive).count(),
            "busy": s.workers.iter().filter(|w| w.busy.is_some()).count(),
            "rendered": s.rendered,
            "failed": s.failed,
            "lastMs": s.last_ms,
            "syncs": s.syncs,
            "syncBytes": s.sync_bytes,
        })
    })
}

// ---------------------------------------------------------------- worker side

/// Handle one frame message (a JSON [`FrameMsg`]) in a frame worker; replies are posted with
/// their pixels transferred.
#[wasm_bindgen(js_name = workerFrame)]
pub fn worker_frame(json: String) {
    let msg: FrameMsg = match serde_json::from_str(&json) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("frame worker: bad message: {e}");
            return;
        }
    };
    let replies = SERVER.with(|c| {
        let mut c = c.borrow_mut();
        let server = c.get_or_insert_with(|| {
            let pool = crate::worker::pool();
            FrameServer::new(pool, Some(std::sync::Arc::new(effectcraft_expr::Expressions)))
        });
        server.handle(msg)
    });
    let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    for (r, px) in replies {
        let t = serde_json::to_string(&r).unwrap_or_default();
        let res = match px {
            Some(px) => {
                let buf = js_sys::Uint8Array::from(&px[..]).buffer();
                scope.post_message_with_transfer(
                    &obj(&[("type", "frameReply".into()), ("json", t.into()), ("bytes", buf.clone().into())]),
                    &js_sys::Array::of1(&buf),
                )
            }
            None => scope.post_message(&obj(&[("type", "frameReply".into()), ("json", t.into())])),
        };
        if let Err(e) = res {
            log::warn!("frame worker: postMessage: {e:?}");
        }
    }
}
