//! Render Queue panel, laid out like After Effects': a Current Render strip (progress bar,
//! elapsed / estimated remaining, Stop and Render buttons), the queue list (twirl, Render
//! checkbox, label, #, Comp Name, Status, Started, Render Time) with expandable Render Settings,
//! Output Module and Output To rows, and a status footer.
//!
//! Every control dispatches an engine `renderQueue.*` command (so agents can do the same) and
//! registers an automation id: `renderQueue.render`, `renderQueue.stop`,
//! `renderQueue.item.<n>.{row,twirl,render,renderSettings,outputModule,outputTo,outputPath}`.

use effectcraft_engine::project::render_queue::{AudioOutput, Channels, OutputFormat, ProResProfile, RenderQuality, RenderQueueItem, RenderStatus, TimeSpan};
use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{EffectcraftApp, widgets};

const TOP_H: f32 = 74.0;
const HEAD_H: f32 = 22.0;
const ROW_H: f32 = 24.0;
const FOOT_H: f32 = 24.0;

/// `h:mm:ss` for a duration in seconds.
pub fn hms(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Unix seconds → `YYYY-MM-DD hh:mm:ss` (UTC).
pub fn utc_stamp(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let rem = unix % 86_400;
    // Civil-from-days (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

fn status_color(t: &Tokens, s: &RenderStatus) -> Color32 {
    match s {
        RenderStatus::Done => Color32::from_rgb(0x6c, 0xc0, 0x6c),
        RenderStatus::Rendering => t.accent_hover,
        RenderStatus::Failed(_) | RenderStatus::NeedsOutput => t.danger,
        RenderStatus::UserStopped => t.warning,
        RenderStatus::Unqueued => t.text_faint,
        RenderStatus::Queued => t.text,
    }
}

/// Render Settings menu: (label, params).
fn settings_menu(it: &RenderQueueItem) -> Vec<(String, Value)> {
    let s = &it.settings;
    let mark = |on: bool, l: &str| if on { format!("✓ {l}") } else { format!("   {l}") };
    let mut v = vec![
        (mark(s.quality == RenderQuality::Best, "Best Settings"), json!({"quality": "best"})),
        (mark(s.quality == RenderQuality::Draft, "Draft Settings"), json!({"quality": "draft"})),
        ("-".into(), Value::Null),
    ];
    for (l, k) in [("Full", "full"), ("Half", "half"), ("Third", "third"), ("Quarter", "quarter")] {
        v.push((mark(s.resolution_label() == l, &format!("Resolution: {l}")), json!({"resolution": k})));
    }
    v.push(("-".into(), Value::Null));
    v.push((mark(s.time_span == TimeSpan::WorkArea, "Time Span: Work Area Only"), json!({"timeSpan": "workArea"})));
    v.push((mark(s.time_span == TimeSpan::LengthOfComp, "Time Span: Length of Comp"), json!({"timeSpan": "comp"})));
    v.push(("-".into(), Value::Null));
    v.push((mark(s.frame_rate.is_none(), "Frame Rate: Use comp's frame rate"), json!({"frameRate": null})));
    for fps in [12.0, 15.0, 23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 60.0] {
        let on = s.frame_rate.is_some_and(|r| (r.as_f64() - fps).abs() < 0.01);
        v.push((mark(on, &format!("Frame Rate: {fps}")), json!({"frameRate": fps})));
    }
    v.push(("-".into(), Value::Null));
    v.push((mark(s.motion_blur, "Motion Blur: On for Checked Layers"), json!({"motionBlur": !s.motion_blur})));
    v.push((mark(s.skip_existing, "Skip Existing Files"), json!({"skipExisting": !s.skip_existing})));
    v
}

/// Output Module menu: (label, params).
fn output_menu(it: &RenderQueueItem, available: &[OutputFormat]) -> Vec<(String, Value)> {
    let o = &it.output;
    let mark = |on: bool, l: &str| if on { format!("✓ {l}") } else { format!("   {l}") };
    let mut v = Vec::new();
    for f in OutputFormat::ALL {
        if available.contains(&f) {
            v.push((mark(o.format == f, &format!("Format: {}", f.label())), json!({"format": format!("{f:?}")})));
        }
    }
    v.push(("-".into(), Value::Null));
    v.push((mark(o.channels == Channels::Rgb, "Channels: RGB"), json!({"channels": "rgb"})));
    if o.format.supports_alpha() {
        v.push((mark(o.channels == Channels::Rgba, "Channels: RGB + Alpha"), json!({"channels": "rgba"})));
    }
    match o.format {
        OutputFormat::ProRes => {
            v.push(("-".into(), Value::Null));
            for p in ProResProfile::ALL {
                v.push((mark(o.prores_profile == p, p.label()), json!({"proresProfile": format!("{p:?}")})));
            }
        }
        OutputFormat::H264 => {
            v.push(("-".into(), Value::Null));
            for kbps in [2_000, 5_000, 10_000, 20_000, 40_000] {
                v.push((mark(o.bitrate_kbps == kbps, &format!("Bitrate: {} Mbps", kbps / 1000)), json!({"bitrate": kbps})));
            }
        }
        OutputFormat::JpegSequence => {
            v.push(("-".into(), Value::Null));
            for q in [60u8, 80, 90, 100] {
                v.push((mark(o.quality == q, &format!("Quality: {q}")), json!({"quality": q})));
            }
        }
        OutputFormat::Gif => {
            v.push(("-".into(), Value::Null));
            v.push((mark(o.gif_loop, "Loop Forever"), json!({"loop": !o.gif_loop})));
        }
        _ => {}
    }
    if o.format.supports_audio() {
        v.push(("-".into(), Value::Null));
        for (a, l) in [(AudioOutput::Auto, "auto"), (AudioOutput::On, "on"), (AudioOutput::Off, "off")] {
            let label = match a {
                AudioOutput::Auto => "Audio Output: Auto",
                AudioOutput::On => "Audio Output: On (48 kHz stereo)",
                AudioOutput::Off => "Audio Output: Off",
            };
            v.push((mark(o.audio == a, label), json!({"audio": l})));
        }
    }
    v
}

const OUTPUT_TEMPLATES: [&str; 4] = [
    "[compName].[fileExtension]",
    "[compName]_[width]x[height].[fileExtension]",
    "[projectName]_[compName].[fileExtension]",
    "[compName]/[compName].[fileExtension]",
];

fn output_to_menu(it: &RenderQueueItem) -> Vec<(String, Value)> {
    let mut v: Vec<(String, Value)> = OUTPUT_TEMPLATES
        .iter()
        .map(|t| {
            let t = if it.output.format.is_sequence() { t.replace(".[fileExtension]", "_[#####].[fileExtension]") } else { t.to_string() };
            (t.clone(), json!({"path": t}))
        })
        .collect();
    v.push(("-".into(), Value::Null));
    v.push(("Choose File…".into(), json!({"choose": true})));
    v
}

pub fn show(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let queue: Vec<RenderQueueItem> = app.session.project.render_queue.clone();
    let progress = app.session.render_progress();
    let rendering = app.session.is_rendering();
    let available = app.session.exporter.as_ref().map(|e| e.formats()).unwrap_or_default();
    let mut actions: Vec<(&'static str, Value)> = Vec::new();
    let state_id = egui::Id::new("rq-ui");
    let (mut open, mut selected): (Vec<u64>, Option<u64>) = ui.data(|d| d.get_temp(state_id).unwrap_or_default());

    // ---------------------------------------------------------- Current Render strip
    let top = Rect::from_min_size(rect.min, vec2(rect.width(), TOP_H));
    p.rect_filled(top, 0.0, t.tl_ruler_bg);
    let x0 = rect.min.x + 12.0;
    p.text(pos2(x0, top.min.y + 16.0), Align2::LEFT_CENTER, "Current Render", Tokens::semibold(12.5), t.text);
    let cur_item = progress.as_ref().and_then(|p| p.current).and_then(|id| queue.iter().find(|i| i.id == id));
    let cur_name = cur_item.and_then(|i| app.session.project.item(i.comp)).map(|i| i.name.clone());
    if let Some(pr) = &progress {
        let rem = pr.remaining.map(hms).unwrap_or_else(|| "–".into());
        p.text(
            pos2(top.max.x - 12.0, top.min.y + 16.0),
            Align2::RIGHT_CENTER,
            format!("Elapsed: {}     Est. Remain: {rem}", hms(pr.item_elapsed)),
            Tokens::ui(11.5),
            t.text_dim,
        );
    }
    let bw = 76.0;
    let render_b = Rect::from_min_size(pos2(top.max.x - 12.0 - bw, top.min.y + 34.0), vec2(bw, 26.0));
    let stop_b = Rect::from_min_size(pos2(render_b.min.x - 8.0 - bw, render_b.min.y), vec2(bw, 26.0));
    let can_render = !rendering && queue.iter().any(RenderQueueItem::is_queued) && !available.is_empty();
    if widgets::text_button(ui, render_b, "Render", can_render, &t, egui::Id::new("rq-render")).clicked() && can_render {
        actions.push(("renderQueue.render", json!({"wait": false})));
    }
    app.auto.add("renderQueue.render", render_b, "Render");
    if rendering {
        if widgets::text_button(ui, stop_b, "Stop", false, &t, egui::Id::new("rq-stop")).clicked() {
            actions.push(("renderQueue.stop", json!({})));
        }
        app.auto.add("renderQueue.stop", stop_b, "Stop");
    }
    // Progress bar.
    let bar =
        Rect::from_min_max(pos2(x0, render_b.min.y + 4.0), pos2(if rendering { stop_b.min.x - 16.0 } else { render_b.min.x - 16.0 }, render_b.max.y - 4.0));
    p.rect_filled(bar, 3.0, t.field_bg);
    p.rect_stroke(bar, 3.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    let frac = progress.as_ref().filter(|p| p.total > 0).map(|p| (p.done as f32 / p.total as f32).clamp(0.0, 1.0)).unwrap_or(0.0);
    if frac > 0.0 {
        let fill = Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height()));
        p.rect_filled(fill, 3.0, t.accent);
    }
    let bar_text = match (&progress, &cur_name) {
        (Some(pr), Some(n)) if rendering => format!("Rendering “{n}”   {} / {} frames", pr.done, pr.total),
        _ => {
            let n = queue.iter().filter(|i| i.is_queued()).count();
            if available.is_empty() {
                "Export is not available in this build".to_string()
            } else if n == 0 {
                "Composition ▸ Add to Render Queue (Ctrl/⌘ M) to queue a render".to_string()
            } else {
                format!("{n} item(s) queued")
            }
        }
    };
    p.text(pos2(bar.min.x + 8.0, bar.center().y), Align2::LEFT_CENTER, bar_text, Tokens::ui(11.5), t.text);
    app.auto.add("renderQueue.progress", bar, &format!("{:.0}%", frac * 100.0));

    // ---------------------------------------------------------- column header
    let head = Rect::from_min_size(pos2(rect.min.x, top.max.y), vec2(rect.width(), HEAD_H));
    p.rect_filled(head, 0.0, t.panel_bg);
    p.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, t.separator));
    // Columns: twirl | Render | label | # | Comp Name | Status | Started | Render Time
    let cx_twirl = rect.min.x + 4.0;
    let cx_render = cx_twirl + 20.0;
    let cx_label = cx_render + 48.0;
    let cx_num = cx_label + 20.0;
    let cx_name = cx_num + 28.0;
    let w_time = 92.0;
    let w_started = 156.0;
    let w_status = 112.0;
    let cx_time = (rect.max.x - w_time).max(cx_name + 120.0);
    let cx_started = cx_time - w_started;
    let cx_status = cx_started - w_status;
    let hy = head.center().y;
    for (x, l) in [(cx_render, "Render"), (cx_num, "#"), (cx_name, "Comp Name"), (cx_status, "Status"), (cx_started, "Started"), (cx_time, "Render Time")] {
        p.text(pos2(x + 2.0, hy), Align2::LEFT_CENTER, l, Tokens::ui(11.0), t.text_dim);
    }
    for x in [cx_render, cx_label, cx_num, cx_name, cx_status, cx_started, cx_time] {
        p.line_segment([pos2(x - 3.0, head.min.y + 4.0), pos2(x - 3.0, head.max.y - 4.0)], Stroke::new(1.0, t.separator));
    }

    // ---------------------------------------------------------- items
    let list = Rect::from_min_max(pos2(rect.min.x, head.max.y), pos2(rect.max.x, rect.max.y - FOOT_H));
    let lp = p.with_clip_rect(list);
    let mut y = list.min.y;
    if queue.is_empty() {
        lp.text(pos2(list.center().x, list.min.y + 30.0), Align2::CENTER_CENTER, "The render queue is empty.", Tokens::ui(12.0), t.text_faint);
    }
    let editing_id = egui::Id::new("rq-edit-output");
    let mut editing: Option<(u64, String)> = ui.data(|d| d.get_temp(editing_id));
    for (k, it) in queue.iter().enumerate() {
        let n = k + 1;
        let aid = |s: &str| format!("renderQueue.item.{n}.{s}");
        let row = Rect::from_min_size(pos2(list.min.x, y), vec2(list.width(), ROW_H));
        y += ROW_H;
        let is_sel = selected == Some(it.id);
        lp.rect_filled(
            row,
            0.0,
            if is_sel {
                t.row_selected
            } else if k % 2 == 0 {
                t.row
            } else {
                t.row_alt
            },
        );
        let row_resp = ui.interact(row, egui::Id::new(("rq-row", it.id)), Sense::click());
        if row_resp.clicked() {
            selected = Some(it.id);
            app.ui.focused = crate::dock::PanelKind::RenderQueue;
        }
        let item_id = it.id;
        row_resp.context_menu(|ui| {
            if ui.button("Duplicate").clicked() {
                actions.push(("renderQueue.duplicate", json!({"item": item_id})));
                ui.close();
            }
            if ui.button("Remove").clicked() {
                actions.push(("renderQueue.remove", json!({"item": item_id})));
                ui.close();
            }
            if k > 0 && ui.button("Move Up").clicked() {
                actions.push(("renderQueue.move", json!({"item": item_id, "to": n - 1})));
                ui.close();
            }
            if n < queue.len() && ui.button("Move Down").clicked() {
                actions.push(("renderQueue.move", json!({"item": item_id, "to": n + 1})));
                ui.close();
            }
        });
        app.auto.add(&aid("row"), row, &format!("#{n}"));
        let is_open = open.contains(&it.id);
        let tw = Rect::from_center_size(pos2(cx_twirl + 8.0, row.center().y), vec2(14.0, 14.0));
        if widgets::twirl(ui, tw, is_open, egui::Id::new(("rq-twirl", it.id)), &t).clicked() {
            if is_open {
                open.retain(|i| *i != it.id);
            } else {
                open.push(it.id);
            }
        }
        app.auto.add(&aid("twirl"), tw, "Twirl");
        let cb = Rect::from_center_size(pos2(cx_render + 20.0, row.center().y), vec2(18.0, 18.0));
        if widgets::checkbox(ui, cb, it.render, &t, egui::Id::new(("rq-cb", it.id))).clicked() && !rendering {
            actions.push(("renderQueue.setRender", json!({"item": it.id, "render": !it.render})));
        }
        app.auto.add(&aid("render"), cb, if it.render { "Render: on" } else { "Render: off" });
        let comp_item = app.session.project.item(it.comp);
        let label_col = comp_item.map(|i| t.label(i.label)).unwrap_or(t.text_faint);
        lp.rect_filled(Rect::from_center_size(pos2(cx_label + 6.0, row.center().y), vec2(10.0, 10.0)), 2.0, label_col);
        lp.text(pos2(cx_num + 2.0, row.center().y), Align2::LEFT_CENTER, n.to_string(), Tokens::ui(12.0), t.text_dim);
        let name = comp_item.map(|i| i.name.clone()).unwrap_or_else(|| "(missing composition)".into());
        lp.text(pos2(cx_name + 2.0, row.center().y), Align2::LEFT_CENTER, &name, Tokens::ui(12.0), t.text);
        let status = if rendering && progress.as_ref().and_then(|p| p.current) == Some(it.id) {
            let pr = progress.as_ref().expect("progress");
            format!("Rendering {:.0}%", if pr.total > 0 { pr.done as f64 * 100.0 / pr.total as f64 } else { 0.0 })
        } else {
            it.status.label().to_string()
        };
        let st_rect = Rect::from_min_size(pos2(cx_status, row.min.y), vec2(w_status, ROW_H));
        lp.text(pos2(cx_status + 2.0, row.center().y), Align2::LEFT_CENTER, &status, Tokens::ui(12.0), status_color(&t, &it.status));
        if let RenderStatus::Failed(e) = &it.status {
            ui.interact(st_rect, egui::Id::new(("rq-st", it.id)), Sense::hover()).on_hover_text(e);
        }
        app.auto.add(&aid("status"), st_rect, &status);
        if let Some(s) = it.started {
            lp.text(pos2(cx_started + 2.0, row.center().y), Align2::LEFT_CENTER, utc_stamp(s), Tokens::ui(11.5), t.text_dim);
        } else {
            lp.text(pos2(cx_started + 2.0, row.center().y), Align2::LEFT_CENTER, "-", Tokens::ui(11.5), t.text_faint);
        }
        let rt = if progress.as_ref().and_then(|p| p.current) == Some(it.id) && rendering {
            progress.as_ref().map(|p| hms(p.item_elapsed)).unwrap_or_default()
        } else {
            it.render_time.map(|s| if s < 60.0 { format!("{s:.1} s") } else { hms(s) }).unwrap_or_else(|| "-".into())
        };
        lp.text(pos2(cx_time + 2.0, row.center().y), Align2::LEFT_CENTER, rt, Tokens::ui(11.5), t.text_dim);

        if !is_open {
            continue;
        }
        // Expanded: Render Settings / Output Module + Output To.
        let ix = cx_name;
        for sub in 0..2 {
            let r = Rect::from_min_size(pos2(list.min.x, y), vec2(list.width(), ROW_H));
            y += ROW_H;
            lp.rect_filled(r, 0.0, t.panel_bg);
            let (label, summary, key, menu): (&str, String, &str, Vec<(String, Value)>) = if sub == 0 {
                ("Render Settings:", it.settings.summary(), "renderSettings", settings_menu(it))
            } else {
                ("Output Module:", it.output.summary(), "outputModule", output_menu(it, &available))
            };
            lp.text(pos2(cx_num - 6.0, r.center().y), Align2::LEFT_CENTER, label, Tokens::ui(11.5), t.text_dim);
            let dd = Rect::from_min_size(pos2(ix + 70.0, r.min.y + 3.0), vec2(220.0, ROW_H - 6.0));
            let did = egui::Id::new(("rq-dd", it.id, sub));
            if widgets::dropdown(ui, dd, &summary, &t, did).clicked() && !rendering {
                widgets::open_popup(ui, did);
            }
            let labels: Vec<String> = menu.iter().map(|(l, _)| l.clone()).collect();
            if let Some(i) = widgets::popup_menu(ui, did, dd.left_bottom(), &labels, None) {
                let mut params = menu[i].1.clone();
                params["item"] = json!(it.id);
                actions.push((if sub == 0 { "renderQueue.setRenderSettings" } else { "renderQueue.setOutputModule" }, params));
            }
            app.auto.add(&aid(key), dd, &summary);
            if sub == 0 {
                let comp = app.session.project.comp(it.comp);
                if let Some(c) = comp {
                    let (w, h) = it.settings.output_size(c);
                    let (a, b) = it.settings.span(c);
                    let info = format!(
                        "{w}×{h} · {:.3} fps · {:.2}s – {:.2}s · {} frames",
                        it.settings.rate(c).as_f64(),
                        a.seconds(),
                        b.seconds(),
                        it.settings.frame_count(c)
                    );
                    lp.text(pos2(dd.max.x + 14.0, r.center().y), Align2::LEFT_CENTER, info, Tokens::ui(11.0), t.text_faint);
                }
                continue;
            }
            // Output To.
            let ox = dd.max.x + 14.0;
            lp.text(pos2(ox, r.center().y), Align2::LEFT_CENTER, "Output To:", Tokens::ui(11.5), t.text_dim);
            let odd = Rect::from_min_size(pos2(ox + 62.0, r.min.y + 3.0), vec2(22.0, ROW_H - 6.0));
            let oid = egui::Id::new(("rq-out", it.id));
            if widgets::dropdown(ui, odd, "", &t, oid).clicked() && !rendering {
                widgets::open_popup(ui, oid);
            }
            app.auto.add(&aid("outputTo"), odd, "Output To");
            let omenu = output_to_menu(it);
            let olabels: Vec<String> = omenu.iter().map(|(l, _)| l.clone()).collect();
            if let Some(i) = widgets::popup_menu(ui, oid, odd.left_bottom(), &olabels, None) {
                if omenu[i].1.get("choose").is_some() {
                    let default = app.session.resolve_output(it).unwrap_or_default();
                    let picked = app
                        .hooks
                        .pick_save
                        .as_ref()
                        .and_then(|f| f(std::path::Path::new(&default).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default().as_str()));
                    match picked {
                        Some(path) => actions.push(("renderQueue.setOutput", json!({"item": it.id, "path": path}))),
                        None if app.hooks.pick_save.is_none() => editing = Some((it.id, it.output.output.clone())),
                        None => {}
                    }
                } else {
                    let mut params = omenu[i].1.clone();
                    params["item"] = json!(it.id);
                    actions.push(("renderQueue.setOutput", params));
                }
            }
            let path_rect = Rect::from_min_max(pos2(odd.max.x + 6.0, r.min.y + 2.0), pos2(r.max.x - 8.0, r.max.y - 2.0));
            if editing.as_ref().is_some_and(|(id, _)| *id == it.id) {
                let mut buf = editing.as_ref().map(|(_, b)| b.clone()).unwrap_or_default();
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(path_rect));
                let resp = child.add(egui::TextEdit::singleline(&mut buf).desired_width(path_rect.width()).font(Tokens::ui(11.5)));
                if resp.lost_focus() {
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !buf.trim().is_empty() {
                        actions.push(("renderQueue.setOutput", json!({"item": it.id, "path": buf.trim()})));
                    }
                    editing = None;
                } else {
                    resp.request_focus();
                    editing = Some((it.id, buf));
                }
            } else {
                let shown = app.session.resolve_output(it).unwrap_or_else(|| "Not yet specified".into());
                let resp = ui.interact(path_rect, egui::Id::new(("rq-path", it.id)), Sense::click());
                let col = if resp.hovered() { t.accent_hover } else { t.hot_text };
                lp.with_clip_rect(path_rect.intersect(list)).text(pos2(path_rect.min.x, r.center().y), Align2::LEFT_CENTER, &shown, Tokens::ui(11.5), col);
                if resp.clicked() && !rendering {
                    editing = Some((it.id, it.output.output.clone()));
                }
                resp.on_hover_text(format!("Template: {}\nClick to edit", it.output.output));
                app.auto.add(&aid("outputPath"), path_rect, &shown);
            }
        }
    }
    ui.data_mut(|d| match &editing {
        Some(e) => {
            d.insert_temp(editing_id, e.clone());
        }
        None => d.remove::<(u64, String)>(editing_id),
    });

    // Delete removes the selected item when the panel has focus.
    if app.ui.focused == crate::dock::PanelKind::RenderQueue
        && editing.is_none()
        && !rendering
        && let Some(id) = selected
        && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        && !ui.ctx().egui_wants_keyboard_input()
    {
        actions.push(("renderQueue.remove", json!({"item": id})));
        selected = None;
    }

    // ---------------------------------------------------------- footer
    let foot = Rect::from_min_max(pos2(rect.min.x, rect.max.y - FOOT_H), rect.max);
    p.rect_filled(foot, 0.0, t.tl_ruler_bg);
    p.line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0, t.separator));
    let msg = match &progress {
        Some(pr) if rendering => format!("Message: Rendering {} of {}", (pr.items_done + 1).min(pr.items_total), pr.items_total),
        _ => {
            let done = queue.iter().filter(|i| i.status == RenderStatus::Done).count();
            format!("Message: {done} of {} done", queue.len())
        }
    };
    p.text(pos2(foot.min.x + 12.0, foot.center().y), Align2::LEFT_CENTER, msg, Tokens::ui(11.0), t.text_dim);
    let started = queue.iter().filter(|i| i.started.is_some()).count();
    let total: f64 = queue.iter().filter_map(|i| i.render_time).sum();
    p.text(
        pos2(foot.max.x - 12.0, foot.center().y),
        Align2::RIGHT_CENTER,
        format!("Renders Started: {started}     Total Time Elapsed: {}", hms(progress.as_ref().filter(|_| rendering).map(|p| p.elapsed).unwrap_or(total))),
        Tokens::ui(11.0),
        t.text_dim,
    );

    open.retain(|id| queue.iter().any(|i| i.id == *id));
    ui.data_mut(|d| d.insert_temp(state_id, (open, selected)));
    let ctx = ui.ctx().clone();
    for (id, params) in actions {
        if let Err(e) = crate::menus::invoke(app, &ctx, id, params) {
            app.ui.status = e;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stamps() {
        assert_eq!(super::utc_stamp(0), "1970-01-01 00:00:00");
        assert_eq!(super::utc_stamp(1_700_000_000), "2023-11-14 22:13:20");
        assert_eq!(super::hms(3725.4), "1:02:05");
    }
}
