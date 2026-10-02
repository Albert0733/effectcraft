//! `window.effectcraft`: the desktop control channel (`docs/control-protocol.md`) as promises,
//! for agents and browser tests. `web/index.html` wraps the exported [`request`] as
//! `effectcraft.request(method, params)`, `effectcraft.execute(command, params)` and friends.

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender};

use effectcraft_ui_egui::ControlRequest;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

type Pending = (Receiver<Value>, js_sys::Function, js_sys::Function);

thread_local! {
    static TX: RefCell<Option<Sender<ControlRequest>>> = const { RefCell::new(None) };
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    static INFO: RefCell<Value> = const { RefCell::new(Value::Null) };
}

pub(crate) fn set_sender(tx: Sender<ControlRequest>) {
    TX.with(|t| *t.borrow_mut() = Some(tx));
}

/// Environment facts reported by `effectcraft.info()`.
pub fn set_info(key: &str, v: Value) {
    INFO.with(|i| {
        let mut i = i.borrow_mut();
        if !i.is_object() {
            *i = json!({});
        }
        i[key] = v;
    });
}

fn to_js(v: &Value) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

fn from_js(v: &JsValue) -> Value {
    if v.is_undefined() || v.is_null() {
        return json!({});
    }
    js_sys::JSON::stringify(v).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}

/// Send a control request to the app; the promise resolves with its `result` or rejects with its
/// `error`.
#[wasm_bindgen]
pub fn request(method: String, params: JsValue) -> js_sys::Promise {
    let params = from_js(&params);
    js_sys::Promise::new(&mut |resolve, reject| {
        let (req, rx) = ControlRequest::new(method.clone(), params.clone());
        let sent = TX.with(|t| t.borrow().as_ref().is_some_and(|tx| tx.send(req).is_ok()));
        if !sent {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("EffectCraft is not running yet"));
            return;
        }
        PENDING.with(|p| p.borrow_mut().push((rx, resolve, reject)));
        crate::repaint();
    })
}

/// Files in the in-memory file table: `[{path, size}]` (picked, dropped, saved and rendered files).
#[wasm_bindgen]
pub fn files() -> JsValue {
    to_js(&Value::Array(crate::files::list().into_iter().map(|(path, size)| json!({"path": path, "size": size})).collect()))
}

/// A file's bytes from the table (e.g. a render), or `undefined`.
#[wasm_bindgen(js_name = readFile)]
pub fn read_file(path: String) -> JsValue {
    crate::files::get(&path).map(|d| js_sys::Uint8Array::from(&d[..]).into()).unwrap_or(JsValue::UNDEFINED)
}

/// Put bytes into the file table under `/<name>` and open (`.ecproj`) or import them. Resolves
/// with the table path.
#[wasm_bindgen(js_name = addFile)]
pub fn add_file(name: String, data: js_sys::Uint8Array) -> String {
    let path = crate::files::path_for(&name);
    crate::files::put(&path, data.to_vec().into());
    let p = path.clone();
    crate::post(move |app| crate::open_or_import(app, vec![p]));
    path
}

/// Backend and build facts.
#[wasm_bindgen]
pub fn info() -> JsValue {
    let mut v = INFO.with(|i| i.borrow().clone());
    if !v.is_object() {
        v = json!({});
    }
    v["version"] = json!(env!("CARGO_PKG_VERSION"));
    v["threads"] = json!(false);
    to_js(&v)
}

/// Settle the promises whose replies arrived (called every frame).
pub(crate) fn poll_replies() {
    PENDING.with(|p| {
        p.borrow_mut().retain(|(rx, resolve, reject)| match rx.try_recv() {
            Ok(v) => {
                if v["ok"].as_bool() == Some(true) {
                    let _ = resolve.call1(&JsValue::NULL, &to_js(&v["result"]));
                } else {
                    let msg = v["error"].as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                    let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&msg));
                }
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("request dropped"));
                false
            }
        });
    });
}
