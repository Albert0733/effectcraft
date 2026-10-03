//! Renders and analyses in Web Workers ([`effectcraft_engine::offload`]).
//!
//! **Page side** — [`WorkerOffload`], the session's [`Offload`]: serializes the job, ships the
//! footage files it reads and the request to a worker from the host's pool (`js/host.js`
//! `workerRun`), and turns the worker's messages back into [`WorkerReply`]s in the job's
//! [`Inbox`]. Rendered files come back as their bytes and download like any render.
//!
//! **Worker side** — `web/worker.js` instantiates the same wasm module (the page's compiled
//! `WebAssembly.Module`, so nothing is compiled twice) and calls [`worker_init`], then
//! [`worker_file`] per footage file and [`worker_job`] per job: a plain engine session (footage
//! from memory, expressions, the exporter) runs it blocking and posts replies as it goes. Each
//! worker has its own memory: no shared-memory threads, so this works on stable Rust.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use effectcraft_engine::Session;
use effectcraft_engine::offload::{Inbox, Offload, WorkerReply, WorkerRequest};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/js/host.js")]
extern "C" {
    #[wasm_bindgen(js_name = workerRun)]
    fn worker_run(id: f64, json: &str, files: js_sys::Array, base: &str, on_message: &js_sys::Function);
    #[wasm_bindgen(js_name = workerCancel)]
    fn worker_cancel(id: f64);
    #[wasm_bindgen(js_name = workerStats)]
    pub fn stats() -> JsValue;
}

type OnMessage = Closure<dyn FnMut(String, JsValue, JsValue, JsValue)>;

thread_local! {
    /// Message handlers of running jobs (dropped when the job ends).
    static HANDLERS: RefCell<HashMap<u64, OnMessage>> = RefCell::new(HashMap::new());
    /// The worker's session and its media pool.
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static POOL: RefCell<Option<Arc<effectcraft_media::MediaPool>>> = const { RefCell::new(None) };
}

/// The page's offload: one worker per running job (idle workers are reused).
pub struct WorkerOffload;

impl Offload for WorkerOffload {
    fn start(&self, req: WorkerRequest, inbox: Arc<Inbox>) -> Result<(), String> {
        let id = req.id;
        let files = js_sys::Array::new();
        for p in &req.files {
            if let Some(d) = crate::files::get(p) {
                files.push(&js_sys::Array::of2(&JsValue::from_str(p), &js_sys::Uint8Array::from(&d[..])));
            }
        }
        let json = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        let on: OnMessage = Closure::new(move |kind: String, json: JsValue, path: JsValue, bytes: JsValue| {
            match kind.as_str() {
                "reply" => {
                    let Some(t) = json.as_string() else { return };
                    match serde_json::from_str::<WorkerReply>(&t) {
                        Ok(r) => {
                            let done = matches!(r, WorkerReply::Done);
                            inbox.push(r);
                            if done {
                                end(id);
                            }
                        }
                        Err(e) => log::warn!("worker reply: {e}"),
                    }
                }
                "file" => {
                    if let (Some(p), Ok(b)) = (path.as_string(), bytes.dyn_into::<js_sys::Uint8Array>()) {
                        crate::files::add_output(&p, b.to_vec().into());
                    }
                }
                _ => {
                    inbox.push(WorkerReply::Failed { error: json.as_string().unwrap_or_else(|| "worker failed".into()) });
                    inbox.push(WorkerReply::Done);
                    end(id);
                }
            }
            crate::repaint();
        });
        worker_run(id as f64, &json, files, &crate::page_url(""), on.as_ref().unchecked_ref());
        HANDLERS.with(|h| h.borrow_mut().insert(id, on));
        Ok(())
    }

    fn cancel(&self, id: u64) {
        worker_cancel(id as f64);
        end(id);
    }
}

/// Drop a job's handler (after the current call returns).
fn end(id: u64) {
    wasm_bindgen_futures::spawn_local(async move {
        HANDLERS.with(|h| h.borrow_mut().remove(&id));
    });
}

// ---------------------------------------------------------------- worker side

fn post(msg: &JsValue, transfer: Option<&js_sys::Array>) {
    let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let r = match transfer {
        Some(t) => scope.post_message_with_transfer(msg, t),
        None => scope.post_message(msg),
    };
    if let Err(e) = r {
        log::warn!("worker: postMessage: {e:?}");
    }
}

fn obj(pairs: &[(&str, JsValue)]) -> JsValue {
    let o = js_sys::Object::new();
    for (k, v) in pairs {
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str(k), v);
    }
    o.into()
}

/// Set up the worker's engine session (called once by `web/worker.js`).
#[wasm_bindgen(js_name = workerInit)]
pub fn worker_init() {
    crate::set_worker();
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    let pool = Arc::new(effectcraft_media::MediaPool::new());
    POOL.with(|p| *p.borrow_mut() = Some(pool.clone()));
    // Rendered files go back to the page (transferred, not copied).
    let sink: Arc<effectcraft_export::Sink> = Arc::new(|path: &str, data: Vec<u8>| {
        let bytes = js_sys::Uint8Array::from(&data[..]);
        let buf = bytes.buffer();
        post(&obj(&[("type", "file".into()), ("path", path.into()), ("bytes", buf.clone().into())]), Some(&js_sys::Array::of1(&buf)));
    });
    let s = Session {
        services: Arc::new(crate::files::WebServices),
        footage: pool.clone(),
        importer: Some(Arc::new(crate::files::WebImporter { pool })),
        expr: Some(Arc::new(effectcraft_expr::Expressions)),
        expr_check: Some(effectcraft_expr::check_syntax),
        exporter: Some(Arc::new(effectcraft_host::FileExporter { sink: Some(sink) })),
        ..Default::default()
    };
    SESSION.with(|c| *c.borrow_mut() = Some(s));
}

/// A footage file for the jobs that follow.
#[wasm_bindgen(js_name = workerFile)]
pub fn worker_file(path: String, bytes: js_sys::Uint8Array) {
    let data: Arc<[u8]> = bytes.to_vec().into();
    crate::files::STORE.put_file(&path, data.clone(), false);
    POOL.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.add_bytes(&path, data);
        }
    });
}

/// Run one job (a JSON [`WorkerRequest`]) and post its replies.
#[wasm_bindgen(js_name = workerJob)]
pub fn worker_job(json: String) {
    let send: effectcraft_engine::offload::Post = Rc::new(|r: WorkerReply| {
        let done = matches!(r, WorkerReply::Done);
        let t = serde_json::to_string(&r).unwrap_or_default();
        post(&obj(&[("type", "reply".into()), ("json", t.into()), ("done", done.into())]), None);
    });
    let req: WorkerRequest = match serde_json::from_str(&json) {
        Ok(r) => r,
        Err(e) => {
            send(WorkerReply::Failed { error: format!("bad request: {e}") });
            send(WorkerReply::Done);
            return;
        }
    };
    SESSION.with(|c| {
        let mut c = c.borrow_mut();
        let Some(s) = c.as_mut() else {
            send(WorkerReply::Failed { error: "worker not initialised".into() });
            send(WorkerReply::Done);
            return;
        };
        effectcraft_engine::offload::run_request(s, req, &send);
    });
}
