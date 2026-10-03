//! Viewer frames rendered by another engine instance (the browser's frame worker).
//!
//! The page keeps no frame threads (a wasm build without shared memory has one thread per
//! instance), so the web app renders viewer frames in a long-lived Web Worker that holds a
//! replica of the project. The page sends the replica **project diffs** rather than the whole
//! project per edit, then render requests; the worker answers each with the frame's pixels.
//!
//! - [`diff`] / [`patch`]: a structural JSON diff of the serialized project (object members
//!   added, changed or removed; arrays element-wise when their length is unchanged, else
//!   replaced). Lossless (unlike JSON Merge Patch, `null` values survive).
//! - [`Mirror`] (page side): what the worker holds; turns the next project revision into a
//!   [`FrameMsg::Sync`] (a full project the first time, a diff afterwards).
//! - [`FrameServer`] (worker side): applies syncs, relayed Roto Brush segmentations and render
//!   requests, and renders frames as premultiplied RGBA8 ([`rgba8_premultiplied`]).
//!
//! The transport (structured-clone messages, transferable pixel buffers) is the web crate's;
//! everything here is portable and tested natively.

use std::sync::Arc;

use effectcraft_project::{ItemId, Project};
use effectcraft_render::{ExprHost, FootageSource, Image, LayerCache, RenderOpts, Renderer};
use effectcraft_time::Tick;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::offload::SegData;

/// One step of a path into a JSON document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Step {
    Key(String),
    Index(usize),
}

/// One change: set the value at `path` (creating or replacing the member/element), or remove
/// the object member at `path` (`value` absent).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op {
    pub path: Vec<Step>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// The changes that turn `old` into `new` (empty when equal).
pub fn diff(old: &Value, new: &Value) -> Vec<Op> {
    let mut ops = vec![];
    diff_into(old, new, &mut vec![], &mut ops);
    ops
}

fn diff_into(old: &Value, new: &Value, path: &mut Vec<Step>, ops: &mut Vec<Op>) {
    if old == new {
        return;
    }
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, va) in a {
                path.push(Step::Key(k.clone()));
                match b.get(k) {
                    Some(vb) => diff_into(va, vb, path, ops),
                    None => ops.push(Op { path: path.clone(), value: None }),
                }
                path.pop();
            }
            for (k, vb) in b {
                if !a.contains_key(k) {
                    path.push(Step::Key(k.clone()));
                    ops.push(Op { path: path.clone(), value: Some(vb.clone()) });
                    path.pop();
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (va, vb)) in a.iter().zip(b).enumerate() {
                path.push(Step::Index(i));
                diff_into(va, vb, path, ops);
                path.pop();
            }
        }
        _ => ops.push(Op { path: path.clone(), value: Some(new.clone()) }),
    }
}

/// Apply [`diff`]'s changes to `doc`.
pub fn patch(doc: &mut Value, ops: &[Op]) -> Result<(), String> {
    for op in ops {
        let Some((last, parents)) = op.path.split_last() else {
            *doc = op.value.clone().ok_or("cannot remove the document")?;
            continue;
        };
        let mut cur = &mut *doc;
        for st in parents {
            cur = match (st, cur) {
                (Step::Key(k), Value::Object(m)) => m.get_mut(k).ok_or_else(|| format!("no member `{k}`"))?,
                (Step::Index(i), Value::Array(a)) => a.get_mut(*i).ok_or_else(|| format!("no element {i}"))?,
                _ => return Err("path does not match the document".into()),
            };
        }
        match (last, cur, &op.value) {
            (Step::Key(k), Value::Object(m), Some(v)) => {
                m.insert(k.clone(), v.clone());
            }
            (Step::Key(k), Value::Object(m), None) => {
                m.remove(k);
            }
            (Step::Index(i), Value::Array(a), Some(v)) => *a.get_mut(*i).ok_or_else(|| format!("no element {i}"))? = v.clone(),
            _ => return Err("path does not match the document".into()),
        }
    }
    Ok(())
}

/// Page → frame worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FrameMsg {
    /// The whole project (JSON) at `revision`.
    Project { revision: u64, project: String },
    /// Changes from the worker's revision `from` to `revision`.
    Patch { from: u64, revision: u64, ops: Vec<Op> },
    /// Render a frame of the current project.
    Render { id: u64, revision: u64, comp: ItemId, time: Tick, opts: Box<RenderOpts> },
    /// Roto Brush segmentations computed elsewhere (a propagation worker).
    Segs { segs: Vec<SegData> },
    /// Drop cached layer buffers (Edit ▸ Purge).
    Purge,
}

/// Frame worker → page. A [`FrameReply::Frame`]'s pixels travel next to the message (a
/// transferable buffer in the browser).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FrameReply {
    /// Premultiplied RGBA8 pixels of request `id`, rendered in `ms` milliseconds.
    Frame { id: u64, width: u32, height: u32, ms: f64 },
    /// Request `id` was not rendered (`stale`: the worker holds another revision).
    Failed { id: u64, error: String, stale: bool },
    /// A sync could not be applied: the page must send the whole project again.
    Resync { error: String },
}

/// The page's record of a worker's replica.
#[derive(Default)]
pub struct Mirror {
    sent: Option<(u64, Value)>,
}

impl Mirror {
    /// The revision the worker holds.
    pub fn revision(&self) -> Option<u64> {
        self.sent.as_ref().map(|s| s.0)
    }

    /// Forget the replica (the next sync sends the whole project).
    pub fn reset(&mut self) {
        self.sent = None;
    }

    /// The message that brings the worker to `revision` of `project` (`None` when it has it).
    pub fn sync(&mut self, revision: u64, project: &Project) -> Option<FrameMsg> {
        if self.revision() == Some(revision) {
            return None;
        }
        let v = serde_json::to_value(project).unwrap_or(Value::Null);
        let msg = match self.sent.take() {
            Some((from, old)) => FrameMsg::Patch { from, revision, ops: diff(&old, &v) },
            None => FrameMsg::Project { revision, project: v.to_string() },
        };
        self.sent = Some((revision, v));
        Some(msg)
    }
}

/// Premultiplied f32 → premultiplied RGBA8 (colour clamped to alpha), as the viewer shows it.
pub fn rgba8_premultiplied(img: &Image) -> Vec<u8> {
    let mut out = Vec::with_capacity(img.data.len() * 4);
    for p in &img.data {
        let a = p[3].clamp(0.0, 1.0);
        let c = |v: f32| (v.clamp(0.0, a) * 255.0 + 0.5) as u8;
        out.extend_from_slice(&[c(p[0]), c(p[1]), c(p[2]), (a * 255.0 + 0.5) as u8]);
    }
    out
}

/// The worker's side: a project replica and what rendering needs.
pub struct FrameServer {
    pub footage: Arc<dyn FootageSource>,
    pub expr: Option<Arc<dyn ExprHost>>,
    pub cache: Arc<LayerCache>,
    doc: Value,
    project: Option<Arc<Project>>,
    revision: Option<u64>,
}

impl FrameServer {
    pub fn new(footage: Arc<dyn FootageSource>, expr: Option<Arc<dyn ExprHost>>) -> FrameServer {
        FrameServer { footage, expr, cache: Arc::new(LayerCache::default()), doc: Value::Null, project: None, revision: None }
    }

    pub fn revision(&self) -> Option<u64> {
        self.revision
    }

    pub fn project(&self) -> Option<&Arc<Project>> {
        self.project.as_ref()
    }

    fn load(&mut self, revision: u64) -> Result<(), String> {
        let p = Project::from_json(&self.doc.to_string()).map_err(|e| e.to_string())?;
        self.project = Some(Arc::new(p));
        self.revision = Some(revision);
        Ok(())
    }

    /// Handle one message; returns the replies with their pixels.
    pub fn handle(&mut self, msg: FrameMsg) -> Vec<(FrameReply, Option<Vec<u8>>)> {
        match msg {
            FrameMsg::Project { revision, project } => {
                let r = serde_json::from_str(&project).map_err(|e| e.to_string()).and_then(|v| {
                    self.doc = v;
                    self.load(revision)
                });
                match r {
                    Ok(()) => vec![],
                    Err(e) => self.lost(e),
                }
            }
            FrameMsg::Patch { from, revision, ops } => {
                if self.revision != Some(from) {
                    return self.lost(format!("the replica is at {:?}, the patch starts at {from}", self.revision));
                }
                match patch(&mut self.doc, &ops).and_then(|_| self.load(revision)) {
                    Ok(()) => vec![],
                    Err(e) => self.lost(e),
                }
            }
            FrameMsg::Segs { segs } => {
                for s in &segs {
                    s.store();
                }
                vec![]
            }
            FrameMsg::Purge => {
                self.cache.clear();
                vec![]
            }
            FrameMsg::Render { id, revision, comp, time, opts } => {
                let fail = |error: String, stale: bool| vec![(FrameReply::Failed { id, error, stale }, None)];
                if self.revision != Some(revision) {
                    return fail(format!("the replica is at {:?}, not {revision}", self.revision), true);
                }
                let Some(p) = self.project.clone() else { return fail("no project".into(), true) };
                if p.comp(comp).is_none() {
                    return fail(format!("no composition {}", comp.0), false);
                }
                let t0 = web_time::Instant::now();
                let mut r = Renderer::new(&p, self.footage.as_ref(), *opts);
                r.expr = self.expr.as_deref();
                r.cache = Some(&self.cache);
                let img = r.comp_frame(comp, time);
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                vec![(FrameReply::Frame { id, width: img.width, height: img.height, ms }, Some(rgba8_premultiplied(&img)))]
            }
        }
    }

    fn lost(&mut self, error: String) -> Vec<(FrameReply, Option<Vec<u8>>)> {
        self.doc = Value::Null;
        self.project = None;
        self.revision = None;
        vec![(FrameReply::Resync { error }, None)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use serde_json::json;

    fn round<T: Serialize + for<'de> Deserialize<'de>>(v: &T) -> T {
        serde_json::from_str(&serde_json::to_string(v).unwrap()).unwrap()
    }

    #[test]
    fn diff_and_patch_round_trip() {
        let a = json!({"a": 1, "b": [1, 2, {"c": null}], "d": {"e": "x", "f": [1]}, "g": null});
        let b = json!({"a": 2, "b": [1, 3, {"c": 4}], "d": {"f": [1, 2]}, "g": null, "h": {"i": null}});
        let ops = diff(&a, &b);
        assert!(!ops.is_empty());
        let mut x = a.clone();
        patch(&mut x, &round(&ops)).unwrap();
        assert_eq!(x, b);
        // Removal and the reverse direction.
        let mut y = b.clone();
        patch(&mut y, &diff(&b, &a)).unwrap();
        assert_eq!(y, a);
        assert!(diff(&a, &a).is_empty());
        // A whole-document change.
        let mut z = json!(1);
        patch(&mut z, &diff(&json!(1), &json!([1]))).unwrap();
        assert_eq!(z, json!([1]));
        // A patch for another document fails cleanly.
        assert!(patch(&mut json!(3), &diff(&a, &b)).is_err());
    }

    fn server(s: &Session) -> FrameServer {
        FrameServer::new(s.footage.clone(), s.expr.clone())
    }

    /// Pixels the page itself would render.
    fn local(s: &Session, comp: ItemId, t: Tick, opts: RenderOpts) -> Vec<u8> {
        let mut r = Renderer::new(&s.project, s.footage.as_ref(), opts);
        r.expr = s.expr.as_deref();
        rgba8_premultiplied(&r.comp_frame(comp, t))
    }

    #[test]
    fn replica_follows_edits_through_diffs_and_renders_the_same_frames() {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let comp = s.active_comp_id().unwrap();
        let opts = RenderOpts { scale: 0.25, guides: true, ..Default::default() };
        let mut m = Mirror::default();
        let mut w = server(&s);
        let send = |m: FrameMsg, w: &mut FrameServer| w.handle(round(&m));
        // First sync: the whole project.
        let first = m.sync(s.revision, &s.project).unwrap();
        assert!(matches!(first, FrameMsg::Project { .. }));
        assert!(send(first, &mut w).is_empty());
        assert!(m.sync(s.revision, &s.project).is_none(), "nothing new");
        let t = Tick::from_seconds_f64(1.0);
        let render = |w: &mut FrameServer, rev: u64, id: u64| w.handle(round(&FrameMsg::Render { id, revision: rev, comp, time: t, opts: Box::new(opts) }));
        let out = render(&mut w, s.revision, 1);
        let (FrameReply::Frame { id, width, height, .. }, Some(px)) = &out[0] else { panic!("{:?}", out[0].0) };
        assert_eq!((*id, *width as usize * *height as usize * 4), (1, px.len()));
        assert_eq!(*px, local(&s, comp, t, opts));
        // An edit travels as a small diff; the replica renders the edited frame.
        s.execute("layer.newSolid", json!({"color": "#ff8800", "name": "Orange"})).unwrap();
        let msg = m.sync(s.revision, &s.project).unwrap();
        let FrameMsg::Patch { ops, .. } = &msg else { panic!("expected a patch") };
        assert!(!ops.is_empty() && serde_json::to_string(ops).unwrap().len() < s.project.to_json().len() / 2);
        assert!(send(msg, &mut w).is_empty());
        assert_eq!(**w.project().unwrap(), *s.project);
        let out = render(&mut w, s.revision, 2);
        assert_eq!(out[0].1.as_deref(), Some(&local(&s, comp, t, opts)[..]));
        // A request for another revision is refused as stale.
        let out = render(&mut w, s.revision + 7, 3);
        assert!(matches!(out[0].0, FrameReply::Failed { id: 3, stale: true, .. }));
        // A patch from the wrong base asks for a resync; the mirror then sends the project.
        let bad = FrameMsg::Patch { from: 999, revision: 1000, ops: vec![] };
        assert!(matches!(send(bad, &mut w)[0].0, FrameReply::Resync { .. }));
        m.reset();
        assert!(matches!(m.sync(s.revision, &s.project), Some(FrameMsg::Project { .. })));
    }

    #[test]
    fn frame_messages_round_trip() {
        for m in [
            FrameMsg::Purge,
            FrameMsg::Segs { segs: vec![SegData { key: 7, data: "AAAA".into() }] },
            FrameMsg::Render {
                id: 3,
                revision: 4,
                comp: ItemId(2),
                time: Tick::from_seconds_f64(0.5),
                opts: Box::new(RenderOpts { scale: 0.5, draft: true, roi: Some([1.0, 2.0, 3.0, 4.0]), ..Default::default() }),
            },
        ] {
            let a = serde_json::to_string(&m).unwrap();
            assert_eq!(serde_json::to_string(&round(&m)).unwrap(), a);
            assert!(a.starts_with("{\"type\":"), "{a}");
        }
        for r in [
            FrameReply::Frame { id: 1, width: 2, height: 3, ms: 4.5 },
            FrameReply::Failed { id: 1, error: "x".into(), stale: true },
            FrameReply::Resync { error: "y".into() },
        ] {
            assert_eq!(round(&r), r);
        }
    }
}
