//! Render Queue: Composition ▸ Add to Render Queue, the queue's Render Settings / Output Module /
//! Output To, Render and Stop.
//!
//! Items are addressed by `item` (the stable id from `renderQueue.add` / `renderQueue.list`) or
//! `index` (1-based, the # column).

use effectcraft_project::render_queue::{
    AudioOutput, Channels, OutputFormat, OutputModule, PostRenderAction, ProResProfile, RenderQuality, RenderQueueItem, RenderSettings, RenderStatus, TimeSpan,
};
use effectcraft_time::{FrameRate, Tick};
use serde_json::{Value, json};

use super::{CommandSpec, b_p, bad, comp_id, f_p, has_comp, str_p};
use crate::{EngineError, Result, Session, cmd, query};

fn can_add(s: &Session) -> std::result::Result<(), String> {
    not_rendering(s)?;
    if s.comp_for_queue(&Value::Null).is_some() { Ok(()) } else { Err("select or open a composition".into()) }
}
fn not_rendering(s: &Session) -> std::result::Result<(), String> {
    if s.is_rendering() { Err("the render queue is rendering".into()) } else { Ok(()) }
}
fn has_items(s: &Session) -> std::result::Result<(), String> {
    not_rendering(s)?;
    if s.project.render_queue.is_empty() { Err("the render queue is empty".into()) } else { Ok(()) }
}
fn has_queued(s: &Session) -> std::result::Result<(), String> {
    not_rendering(s)?;
    if s.exporter.is_none() {
        return Err("export is not available in this build".into());
    }
    if s.project.render_queue.iter().any(RenderQueueItem::is_queued) { Ok(()) } else { Err("nothing is queued".into()) }
}
fn rendering(s: &Session) -> std::result::Result<(), String> {
    if s.is_rendering() { Ok(()) } else { Err("not rendering".into()) }
}

/// Index of the item named by `item` (id) or `index` (1-based).
fn item_index(s: &Session, p: &Value, cmd: &str) -> Result<usize> {
    let q = &s.project.render_queue;
    if let Some(id) = p.get("item").and_then(Value::as_u64) {
        return q.iter().position(|i| i.id == id).ok_or_else(|| bad(cmd, format!("no render queue item {id}")));
    }
    if let Some(n) = p.get("index").and_then(Value::as_u64) {
        return (n as usize).checked_sub(1).filter(|i| *i < q.len()).ok_or_else(|| bad(cmd, format!("no render queue item #{n}")));
    }
    if q.len() == 1 {
        return Ok(0);
    }
    Err(bad(cmd, "pass `item` (id) or `index` (1-based)"))
}

fn time_p(p: &Value, k: &str) -> Option<Tick> {
    f_p(p, k).map(Tick::from_seconds_f64)
}

/// Apply Render Settings parameters.
fn apply_settings(rs: &mut RenderSettings, p: &Value, cmd: &str) -> Result<bool> {
    let mut any = false;
    if let Some(q) = str_p(p, "quality") {
        rs.quality = match q.to_ascii_lowercase().as_str() {
            "best" => RenderQuality::Best,
            "draft" => RenderQuality::Draft,
            _ => return Err(bad(cmd, "quality: best|draft")),
        };
        any = true;
    }
    match p.get("resolution") {
        Some(Value::String(r)) => {
            rs.resolution = match r.to_ascii_lowercase().as_str() {
                "full" => 1.0,
                "half" => 0.5,
                "third" => 1.0 / 3.0,
                "quarter" => 0.25,
                _ => return Err(bad(cmd, "resolution: full|half|third|quarter|<scale>")),
            };
            any = true;
        }
        Some(Value::Number(n)) => {
            rs.resolution = n.as_f64().unwrap_or(1.0).clamp(0.01, 4.0);
            any = true;
        }
        _ => {}
    }
    let (start, end) = (time_p(p, "start"), time_p(p, "end"));
    if let Some(ts) = str_p(p, "timeSpan") {
        rs.time_span = match ts.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
            "workarea" | "workareaonly" => TimeSpan::WorkArea,
            "comp" | "lengthofcomp" | "full" => TimeSpan::LengthOfComp,
            "custom" => TimeSpan::Custom { start: start.unwrap_or(Tick::ZERO), end: end.unwrap_or(Tick::from_seconds_f64(1.0)) },
            _ => return Err(bad(cmd, "timeSpan: workArea|comp|custom")),
        };
        any = true;
    } else if start.is_some() || end.is_some() {
        let (a, b) = match rs.time_span {
            TimeSpan::Custom { start, end } => (start, end),
            _ => (Tick::ZERO, Tick(i64::MAX / 4)),
        };
        rs.time_span = TimeSpan::Custom { start: start.unwrap_or(a), end: end.unwrap_or(b) };
        any = true;
    }
    if let TimeSpan::Custom { start, end } = rs.time_span
        && end <= start
    {
        return Err(bad(cmd, "end must be after start"));
    }
    match p.get("frameRate") {
        Some(Value::Null) => {
            rs.frame_rate = None;
            any = true;
        }
        Some(v) => {
            let f = v.as_f64().filter(|f| *f > 0.0 && *f <= 1000.0).ok_or_else(|| bad(cmd, "frameRate: fps > 0 or null (comp rate)"))?;
            rs.frame_rate = Some(FrameRate::from_f64(f));
            any = true;
        }
        None => {}
    }
    if let Some(b) = b_p(p, "motionBlur") {
        rs.motion_blur = b;
        any = true;
    }
    if let Some(b) = b_p(p, "skipExisting") {
        rs.skip_existing = b;
        any = true;
    }
    Ok(any)
}

/// Apply Output Module parameters.
fn apply_output(om: &mut OutputModule, p: &Value, cmd: &str) -> Result<bool> {
    let mut any = false;
    if let Some(f) = str_p(p, "format") {
        let f = OutputFormat::from_name(f).ok_or_else(|| bad(cmd, "format: h264|prores|webm|png|jpeg|tiff|exr|gif|wav|aiff"))?;
        om.set_format(f);
        any = true;
    }
    if let Some(c) = str_p(p, "channels") {
        om.channels = match c.to_ascii_lowercase().replace([' ', '+'], "").as_str() {
            "rgb" => Channels::Rgb,
            "rgba" | "rgbalpha" => Channels::Rgba,
            _ => return Err(bad(cmd, "channels: rgb|rgba")),
        };
        if om.channels == Channels::Rgba && !om.format.supports_alpha() {
            return Err(bad(cmd, format!("{} has no alpha channel", om.format.label())));
        }
        any = true;
    }
    if let Some(q) = f_p(p, "quality") {
        om.quality = q.clamp(1.0, 100.0) as u8;
        any = true;
    }
    if let Some(b) = f_p(p, "bitrate") {
        om.bitrate_kbps = b.clamp(100.0, 500_000.0) as u32;
        any = true;
    }
    if let Some(pp) = str_p(p, "proresProfile") {
        om.prores_profile = ProResProfile::from_name(pp).ok_or_else(|| bad(cmd, "proresProfile: proxy|lt|standard|hq|4444|4444xq"))?;
        any = true;
    }
    if let Some(a) = p.get("audio") {
        om.audio = match a {
            Value::Bool(true) => AudioOutput::On,
            Value::Bool(false) => AudioOutput::Off,
            Value::String(s) => match s.to_ascii_lowercase().as_str() {
                "auto" => AudioOutput::Auto,
                "on" => AudioOutput::On,
                "off" => AudioOutput::Off,
                _ => return Err(bad(cmd, "audio: auto|on|off")),
            },
            _ => return Err(bad(cmd, "audio: auto|on|off")),
        };
        any = true;
    }
    if let Some(r) = p.get("sampleRate").and_then(Value::as_u64) {
        om.audio_sample_rate = (r as u32).clamp(8_000, 192_000);
        any = true;
    }
    if let Some(l) = b_p(p, "loop") {
        om.gif_loop = l;
        any = true;
    }
    if let Some(o) = str_p(p, "output").or(str_p(p, "path")) {
        om.output = o.to_string();
        if p.get("format").is_none()
            && let Some(f) = OutputFormat::from_path(o).filter(|f| *f != om.format && !o.contains("[fileExtension]"))
        {
            // `out.mov` with an H.264 module → ProRes, like picking a file type in Output To.
            let keep = om.output.clone();
            om.set_format(f);
            om.output = keep;
        }
        any = true;
    }
    Ok(any)
}

/// `module` (1-based; 1 = the first output module).
fn module_p(s: &Session, i: usize, p: &Value, cmd: &str) -> Result<usize> {
    let n = p.get("module").and_then(Value::as_u64).unwrap_or(1) as usize;
    let have = s.project.render_queue[i].extra_outputs.len() + 1;
    if n == 0 || n > have {
        return Err(bad(cmd, format!("module must be 1..={have}")));
    }
    Ok(n - 1)
}

fn module_ref(it: &RenderQueueItem, m: usize) -> &OutputModule {
    if m == 0 { &it.output } else { &it.extra_outputs[m - 1] }
}

fn module_mut(it: &mut RenderQueueItem, m: usize) -> &mut OutputModule {
    if m == 0 { &mut it.output } else { &mut it.extra_outputs[m - 1] }
}

/// A finished item that is edited goes back to Queued (AE re-queues on change).
fn requeue(it: &mut RenderQueueItem) {
    if it.render && !matches!(it.status, RenderStatus::Rendering) {
        it.status = RenderStatus::Queued;
    }
}

fn item_json(s: &Session, it: &RenderQueueItem, index: usize) -> Value {
    let comp = s.project.comp(it.comp);
    let name = s.project.item(it.comp).map(|i| i.name.clone());
    let (w, h) = comp.map(|c| it.settings.output_size(c)).map(|(w, h)| it.output.format.coded_size(w, h)).unwrap_or((0, 0));
    let frames = comp.map(|c| it.settings.frame_count(c)).unwrap_or(0);
    let mut v = serde_json::to_value(it).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert("index".into(), json!(index + 1));
        o.insert("compName".into(), json!(name));
        o.insert("statusLabel".into(), json!(it.status.label()));
        o.insert("outputPath".into(), json!(s.resolve_output(it)));
        o.insert("renderSettingsSummary".into(), json!(it.settings.summary()));
        o.insert("outputModuleSummary".into(), json!(it.output.summary()));
        let extra: Vec<Value> = it
            .extra_outputs
            .iter()
            .map(|om| {
                let mut c = it.clone();
                c.output = om.clone();
                json!({"summary": om.summary(), "outputPath": s.resolve_output(&c)})
            })
            .collect();
        o.insert("outputModules".into(), json!(it.extra_outputs.len() + 1));
        o.insert("extraOutputs".into(), json!(extra));
        o.insert("postRenderAction".into(), json!(it.post_render.label()));
        o.insert("width".into(), json!(w));
        o.insert("height".into(), json!(h));
        o.insert("frames".into(), json!(frames));
    }
    v
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let cid = s.comp_for_queue(p).ok_or(EngineError::NoComp)?;
    let mut it = RenderQueueItem::new(0, cid);
    if let Some(f) = str_p(p, "format") {
        let f = OutputFormat::from_name(f).ok_or_else(|| bad("renderQueue.add", "format: h264|prores|webm|png|jpeg|tiff|exr|gif|wav|aiff"))?;
        it.output = OutputModule::for_format(f);
    }
    apply_settings(&mut it.settings, p, "renderQueue.add")?;
    let mut op = p.clone();
    if let Some(o) = op.as_object_mut() {
        o.remove("format");
    }
    apply_output(&mut it.output, &op, "renderQueue.add")?;
    let id = s.edit("Add to Render Queue", None, |proj, _| {
        it.id = proj.render_queue.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        let id = it.id;
        proj.render_queue.push(it);
        Ok(id)
    })?;
    let idx = s.project.render_queue.len() - 1;
    let mut v = item_json(s, &s.project.render_queue[idx], idx);
    v["item"] = json!(id);
    Ok(v)
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let i = item_index(s, p, "renderQueue.remove")?;
    s.edit("Remove from Render Queue", None, |proj, _| {
        proj.render_queue.remove(i);
        Ok(())
    })?;
    Ok(json!({"remaining": s.project.render_queue.len()}))
}

fn set_render(s: &mut Session, p: &Value) -> Result<Value> {
    let i = item_index(s, p, "renderQueue.setRender")?;
    let on = b_p(p, "render").unwrap_or(!s.project.render_queue[i].render);
    s.edit("Render Queue: Render", None, |proj, _| {
        let it = &mut proj.render_queue[i];
        it.render = on;
        it.status = if on { RenderStatus::Queued } else { RenderStatus::Unqueued };
        Ok(())
    })?;
    Ok(json!({"render": on}))
}

fn set_render_settings(s: &mut Session, p: &Value) -> Result<Value> {
    let i = item_index(s, p, "renderQueue.setRenderSettings")?;
    let mut rs = s.project.render_queue[i].settings.clone();
    if !apply_settings(&mut rs, p, "renderQueue.setRenderSettings")? {
        return Err(bad("renderQueue.setRenderSettings", "nothing to change"));
    }
    s.edit("Render Settings", None, |proj, _| {
        let it = &mut proj.render_queue[i];
        it.settings = rs;
        requeue(it);
        Ok(())
    })?;
    Ok(item_json(s, &s.project.render_queue[i], i))
}

fn set_output_module(s: &mut Session, p: &Value) -> Result<Value> {
    let i = item_index(s, p, "renderQueue.setOutputModule")?;
    let m = module_p(s, i, p, "renderQueue.setOutputModule")?;
    let mut om = module_ref(&s.project.render_queue[i], m).clone();
    if !apply_output(&mut om, p, "renderQueue.setOutputModule")? {
        return Err(bad("renderQueue.setOutputModule", "nothing to change"));
    }
    s.edit("Output Module Settings", None, |proj, _| {
        let it = &mut proj.render_queue[i];
        *module_mut(it, m) = om;
        requeue(it);
        Ok(())
    })?;
    Ok(item_json(s, &s.project.render_queue[i], i))
}

fn set_output(s: &mut Session, p: &Value) -> Result<Value> {
    let i = item_index(s, p, "renderQueue.setOutput")?;
    let path = str_p(p, "path").or(str_p(p, "output")).ok_or_else(|| bad("renderQueue.setOutput", "pass `path` (file path or template)"))?;
    let m = module_p(s, i, p, "renderQueue.setOutput")?;
    let mut om = module_ref(&s.project.render_queue[i], m).clone();
    apply_output(&mut om, &json!({"output": path}), "renderQueue.setOutput")?;
    s.edit("Output To", None, |proj, _| {
        let it = &mut proj.render_queue[i];
        *module_mut(it, m) = om;
        requeue(it);
        Ok(())
    })?;
    Ok(item_json(s, &s.project.render_queue[i], i))
}

fn move_item(s: &mut Session, p: &Value) -> Result<Value> {
    let i = item_index(s, p, "renderQueue.move")?;
    let n = s.project.render_queue.len();
    let to = p.get("to").and_then(Value::as_u64).ok_or_else(|| bad("renderQueue.move", "pass `to` (1-based position)"))? as usize;
    let to = to.clamp(1, n) - 1;
    s.edit("Reorder Render Queue", None, |proj, _| {
        let it = proj.render_queue.remove(i);
        proj.render_queue.insert(to, it);
        Ok(())
    })?;
    Ok(json!({"index": to + 1}))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let i = item_index(s, p, "renderQueue.duplicate")?;
    let id = s.edit("Duplicate Render Item", None, |proj, _| {
        let mut it = proj.render_queue[i].clone();
        it.id = proj.render_queue.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        it.render = true;
        it.status = RenderStatus::Queued;
        it.started = None;
        it.render_time = None;
        it.last_output = None;
        let id = it.id;
        proj.render_queue.insert(i + 1, it);
        Ok(id)
    })?;
    Ok(json!({"item": id, "index": i + 2}))
}

fn render(s: &mut Session, p: &Value) -> Result<Value> {
    let wait = b_p(p, "wait").unwrap_or(true);
    // Settings ▸ Project ▸ Auto-Save ▸ Save When Starting Render Queue.
    if s.prefs.auto_save.enabled
        && s.prefs.auto_save.save_on_render_start
        && s.is_dirty()
        && let Err(e) = s.autosave_now()
    {
        log::warn!("{e}");
    }
    let ids = s.start_render(wait).map_err(EngineError::Other)?;
    let items: Vec<Value> = s.project.render_queue.iter().enumerate().filter(|(_, i)| ids.contains(&i.id)).map(|(k, i)| item_json(s, i, k)).collect();
    Ok(json!({"rendering": s.is_rendering(), "items": items}))
}

fn stop(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({"stopped": s.stop_render()}))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    s.poll_render();
    let items: Vec<Value> = s.project.render_queue.iter().enumerate().map(|(k, i)| item_json(s, i, k)).collect();
    Ok(json!({"items": items, "rendering": s.is_rendering(), "progress": s.render_progress()}))
}

fn formats(s: &mut Session, _: &Value) -> Result<Value> {
    let avail = s.exporter.as_ref().map(|e| e.formats()).unwrap_or_default();
    let v: Vec<Value> = OutputFormat::ALL
        .iter()
        .map(|f| {
            json!({
                "id": format!("{f:?}"),
                "label": f.label(),
                "extension": f.extension(),
                "sequence": f.is_sequence(),
                "alpha": f.supports_alpha(),
                "audio": f.supports_audio(),
                "available": avail.contains(f),
            })
        })
        .collect();
    Ok(json!({"formats": v, "proresProfiles": ProResProfile::ALL.iter().map(|p| p.label()).collect::<Vec<_>>()}))
}

/// `item?|index?`, else the last item (the queue's newest entry).
fn item_or_last(s: &Session, p: &Value, cmd: &str) -> Result<usize> {
    if p.get("item").is_some() || p.get("index").is_some() {
        return item_index(s, p, cmd);
    }
    s.project.render_queue.len().checked_sub(1).ok_or_else(|| bad(cmd, "the render queue is empty"))
}

/// Composition ▸ Add Output Module: another output module on a render item (the same frames are
/// encoded once more).
fn add_output_module(s: &mut Session, p: &Value) -> Result<Value> {
    let i = item_or_last(s, p, "render.addOutputModule")?;
    let it = &s.project.render_queue[i];
    let n = it.extra_outputs.len() + 2;
    let mut om = it.output.clone();
    if let Some(f) = str_p(p, "format") {
        let f = OutputFormat::from_name(f).ok_or_else(|| bad("render.addOutputModule", "format: h264|prores|webm|png|jpeg|tiff|exr|gif|wav|aiff"))?;
        om = OutputModule::for_format(f);
    }
    if str_p(p, "output").is_none() && str_p(p, "path").is_none() {
        // Same name with a module suffix so the files don't collide.
        om.output = match om.output.rfind('.') {
            Some(dot) => format!("{}_{n}{}", &om.output[..dot], &om.output[dot..]),
            None => format!("{}_{n}", om.output),
        };
    }
    let mut op = p.clone();
    if let Some(o) = op.as_object_mut() {
        o.remove("format");
    }
    apply_output(&mut om, &op, "render.addOutputModule")?;
    s.edit("Add Output Module", None, |proj, _| {
        let it = &mut proj.render_queue[i];
        it.extra_outputs.push(om);
        requeue(it);
        Ok(())
    })?;
    let mut v = item_json(s, &s.project.render_queue[i], i);
    v["module"] = json!(n);
    Ok(v)
}

/// Composition ▸ Pre-render…: queue the comp with a lossless-with-alpha module whose post-render
/// action imports the result and replaces the comp's uses.
fn pre_render(s: &mut Session, p: &Value) -> Result<Value> {
    let cid = s.comp_for_queue(p).ok_or(EngineError::NoComp)?;
    let mut it = RenderQueueItem::new(0, cid);
    it.output = OutputModule::for_format(OutputFormat::ProRes);
    it.output.prores_profile = ProResProfile::P4444;
    it.output.channels = Channels::Rgba;
    it.output.output = "[compName]_prerender.[fileExtension]".into();
    apply_output(&mut it.output, p, "render.preRender")?;
    it.post_render = PostRenderAction::ImportAndReplace;
    let id = s.edit("Pre-render", None, |proj, _| {
        it.id = proj.render_queue.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        let id = it.id;
        proj.render_queue.push(it);
        Ok(id)
    })?;
    let idx = s.project.render_queue.len() - 1;
    let mut v = item_json(s, &s.project.render_queue[idx], idx);
    v["item"] = json!(id);
    Ok(v)
}

/// Composition ▸ Save Current Preview…: render the work area of the comp straight to a movie file.
fn save_current_preview(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").ok_or_else(|| bad("render.saveCurrentPreview", "missing `path`"))?.to_string();
    let cid = comp_id(s, p)?;
    let format = OutputFormat::from_path(&path).unwrap_or(OutputFormat::H264);
    let exporter = s.exporter.clone().ok_or_else(|| EngineError::Other("export is not available in this build".into()))?;
    if !exporter.formats().contains(&format) {
        return Err(EngineError::Other(format!("{} export is not available", format.label())));
    }
    let mut item = RenderQueueItem::new(0, cid);
    item.output = OutputModule::for_format(format);
    item.output.output = path.clone();
    let job =
        crate::ExportJob { project: &s.project, footage: s.footage.as_ref(), expr: s.expr.as_deref(), accel: s.accel.as_deref(), item: &item, path: &path };
    let r = exporter.export(&job, &mut |_, _| true).map_err(EngineError::Other)?;
    s.toast(format!("Saved preview to {}", r.path));
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "renderQueue.add",
            "Add to Render Queue",
            ["Composition"],
            Some("Cmd+M"),
            "{comp?: id|name, format?: h264|prores|webm|png|jpeg|tiff|exr|gif|wav|aiff, output?: path|template, quality?: best|draft|1-100 (jpeg, webm), resolution?: full|half|third|quarter|scale, timeSpan?: workArea|comp|custom, start?: s, end?: s, frameRate?: fps|null, motionBlur?: bool, skipExisting?: bool, channels?: rgb|rgba, bitrate?: kbps, proresProfile?: proxy|lt|standard|hq|4444|4444xq, audio?: auto|on|off, sampleRate?, loop?: bool}",
            can_add,
            add
        ),
        cmd!("renderQueue.remove", "Remove from Render Queue", [], None, "{item?: id, index?: n}", has_items, remove),
        cmd!("renderQueue.setRender", "Render Queue: Render Checkbox", [], None, "{item?|index?, render?: bool (toggles)}", has_items, set_render),
        cmd!(
            "renderQueue.setRenderSettings",
            "Render Settings...",
            [],
            None,
            "{item?|index?, quality?: best|draft, resolution?: full|half|third|quarter|scale, timeSpan?: workArea|comp|custom, start?: s, end?: s, frameRate?: fps|null, motionBlur?: bool, skipExisting?: bool}",
            has_items,
            set_render_settings
        ),
        cmd!(
            "renderQueue.setOutputModule",
            "Output Module Settings...",
            [],
            None,
            "{item?|index?, module?: n (1 = first), format?, channels?: rgb|rgba, quality?: 1-100, bitrate?: kbps, proresProfile?: proxy|lt|standard|hq|4444|4444xq, audio?: auto|on|off, sampleRate?, loop?: bool, output?}",
            has_items,
            set_output_module
        ),
        cmd!(
            "renderQueue.setOutput",
            "Output To...",
            [],
            None,
            "{item?|index?, module?: n, path: file path or template like [compName].[fileExtension]}",
            has_items,
            set_output
        ),
        cmd!(
            "render.addOutputModule",
            "Add Output Module",
            ["Composition"],
            None,
            "{item?|index? (default: the last item), format?, output?, channels?, quality?, bitrate?, proresProfile?, audio?}",
            has_items,
            add_output_module
        ),
        cmd!("render.preRender", "Pre-render...", ["Composition"], None, "{comp?, output?: path|template, format?}", can_add, pre_render),
        cmd!("render.saveCurrentPreview", "Save Current Preview...", ["Composition"], None, "{comp?, path}", has_comp, save_current_preview),
        cmd!("renderQueue.move", "Move in Render Queue", [], None, "{item?|index?, to: n (1-based)}", has_items, move_item),
        cmd!("renderQueue.duplicate", "Duplicate Render Item", [], None, "{item?|index?}", has_items, duplicate),
        cmd!("renderQueue.render", "Render", [], None, "{wait?: bool (default true; the UI renders in the background)}", has_queued, render),
        cmd!("renderQueue.stop", "Stop Rendering", [], None, "{}", rendering, stop),
        query!("renderQueue.list", "Render Queue Items", "{}", list),
        query!("renderQueue.formats", "Output Formats", "{}", formats),
    ]
}
