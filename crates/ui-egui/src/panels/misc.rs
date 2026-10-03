//! Smaller panels: Preview, Audio, History, Markers, the Home screen and placeholders.

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;

use crate::dock::PanelKind;
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::{EffectcraftApp, widgets};

pub fn placeholder(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, p: PanelKind) {
    let t = app.tokens;
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, format!("{} — coming soon", p.title()), Tokens::ui(12.0), t.text_faint);
}

/// Preview panel: transport controls and RAM preview options.
pub fn preview(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    let p = ui.painter().with_clip_rect(rect);
    let y = rect.min.y + 12.0;
    let bw = 30.0;
    let total = bw * 5.0 + 16.0;
    let mut x = rect.center().x - total / 2.0;
    let buttons: [(Icon, &str, &str); 5] = [
        (Icon::First, "time.start", "First Frame"),
        (Icon::StepBack, "time.previousFrame", "Previous Frame"),
        (Icon::Play, "playback.toggle", "Play/Stop"),
        (Icon::StepFwd, "time.nextFrame", "Next Frame"),
        (Icon::Last, "time.end", "Last Frame"),
    ];
    for (icon, cmd, tip) in buttons {
        let r = Rect::from_min_size(pos2(x, y), vec2(bw, 28.0));
        let icon = if cmd == "playback.toggle" && app.playback.playing { Icon::Pause } else { icon };
        if widgets::icon_button(ui, r, icon, cmd == "playback.toggle" && app.playback.playing, &t, egui::Id::new(("pv", cmd))).on_hover_text(tip).clicked() {
            let _ = crate::menus::invoke(app, &ctx, cmd, json!({}));
        }
        app.auto.add(&format!("preview.{}", tip.replace([' ', '/'], "")), r, tip);
        x += bw + 4.0;
    }
    let mut yy = y + 44.0;
    let row = |p: &egui::Painter, yy: f32, k: &str, v: &str| {
        p.text(pos2(rect.min.x + 12.0, yy), Align2::LEFT_CENTER, k, Tokens::ui(12.0), t.text_dim);
        p.text(pos2(rect.min.x + 130.0, yy), Align2::LEFT_CENTER, v, Tokens::ui(12.0), t.text);
    };
    row(&p, yy, "Shortcut", "Spacebar");
    yy += 22.0;
    let lr = Rect::from_min_size(pos2(rect.min.x + 10.0, yy - 8.0), vec2(16.0, 16.0));
    if widgets::checkbox(ui, lr, app.ui.preview_loop, &t, egui::Id::new("pv-loop")).clicked() {
        app.ui.preview_loop = !app.ui.preview_loop;
    }
    app.auto.add("preview.loop", lr, "Loop");
    p.text(pos2(lr.max.x + 6.0, yy), Align2::LEFT_CENTER, "Loop", Tokens::ui(12.0), t.text);
    let ar = Rect::from_min_size(pos2(rect.min.x + 90.0, yy - 8.0), vec2(16.0, 16.0));
    if widgets::checkbox(ui, ar, app.ui.preview_audio, &t, egui::Id::new("pv-audio")).clicked() {
        let _ = crate::menus::invoke(app, &ctx, "playback.audio", json!({}));
    }
    app.auto.add("preview.audio", ar, "Include Audio");
    p.text(pos2(ar.max.x + 6.0, yy), Align2::LEFT_CENTER, "Include Audio", Tokens::ui(12.0), t.text);
    yy += 24.0;
    row(&p, yy, "Range", "Work Area Extended By Current Time");
    yy += 22.0;
    row(&p, yy, "Play From", "Current Time");
    yy += 22.0;
    let comp = app.session.active_comp().cloned();
    row(&p, yy, "Frame Rate", &comp.as_ref().map(|c| format!("({:.2})", c.frame_rate.as_f64())).unwrap_or_default());
    yy += 22.0;
    row(&p, yy, "Resolution", app.ui.viewer.res.label());
    yy += 26.0;
    if let Some(c) = comp {
        let cid = app.session.active_comp_id().map(|i| i.0).unwrap_or(0);
        let scale = app.viewer_shown.as_ref().map(|(_, k)| k.scale).unwrap_or(1000);
        let cached = app.frames.cached_frames(app.session.revision, cid, scale).len();
        let total = c.frame_rate.frame_at(c.work_area.1) - c.frame_rate.frame_at(c.work_area.0);
        let bar = Rect::from_min_size(pos2(rect.min.x + 12.0, yy), vec2(rect.width() - 24.0, 6.0));
        p.rect_filled(bar, 3.0, t.field_bg);
        let f = (cached as f32 / total.max(1) as f32).min(1.0);
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height())), 3.0, t.cache_green);
        p.text(pos2(bar.min.x, bar.max.y + 12.0), Align2::LEFT_CENTER, format!("{cached} / {total} frames cached"), Tokens::ui(11.0), t.text_faint);
    }
}

/// Audio panel: L/R VU meters (dBFS, 0 to -48) with peak hold and clip indicators, fed by the
/// audio preview; the selected layer's Audio Levels on the right.
pub fn audio(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    use crate::audio::METER_FLOOR;
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let top = rect.min.y + 26.0;
    let bottom = rect.max.y - 22.0;
    if bottom - top < 40.0 {
        return;
    }
    let y_of = |db: f32| top + (bottom - top) * (db.clamp(METER_FLOOR, 0.0) / METER_FLOOR);
    let m = app.meter;
    let green = Color32::from_rgb(0x3c, 0xc8, 0x5a);
    let yellow = Color32::from_rgb(0xe6, 0xc8, 0x3c);
    let red = Color32::from_rgb(0xe6, 0x46, 0x3c);
    let seg_col = |db: f32| {
        if db > -3.0 {
            red
        } else if db > -12.0 {
            yellow
        } else {
            green
        }
    };
    for c in 0..2 {
        let x = rect.min.x + 18.0 + c as f32 * 16.0;
        let bar = Rect::from_min_max(pos2(x, top), pos2(x + 12.0, bottom));
        p.rect_filled(bar, 1.0, t.field_bg);
        // Lit segments, 1.5 dB each, coloured by their level.
        let lvl = m.level_db[c];
        let mut db = METER_FLOOR;
        while db < lvl.min(0.0) {
            let hi = (db + 1.5).min(lvl);
            p.rect_filled(Rect::from_min_max(pos2(bar.min.x + 1.0, y_of(hi)), pos2(bar.max.x - 1.0, y_of(db) - 0.5)), 0.0, seg_col(db));
            db += 1.5;
        }
        if m.peak_db[c] > METER_FLOOR {
            let py = y_of(m.peak_db[c]);
            p.line_segment([pos2(bar.min.x, py), pos2(bar.max.x, py)], Stroke::new(2.0, seg_col(m.peak_db[c])));
        }
        // Clip indicator (click to reset).
        let clip = Rect::from_min_max(pos2(bar.min.x, top - 14.0), pos2(bar.max.x, top - 4.0));
        p.rect_filled(clip, 1.0, if m.clipped[c] { red } else { t.field_bg });
        let id = if c == 0 { "audio.clipLeft" } else { "audio.clipRight" };
        if ui.interact(clip, egui::Id::new(id), Sense::click()).clicked() {
            app.meter.clipped[c] = false;
        }
        app.auto.add(id, clip, if c == 0 { "Left clip indicator" } else { "Right clip indicator" });
        app.auto.add(if c == 0 { "audio.meterLeft" } else { "audio.meterRight" }, bar, &format!("{:.1} dB", m.level_db[c]));
        p.text(pos2(bar.center().x, bottom + 9.0), Align2::CENTER_CENTER, if c == 0 { "L" } else { "R" }, Tokens::ui(10.0), t.text_dim);
    }
    let sx = rect.min.x + 54.0;
    for db in [0, -6, -12, -18, -24, -30, -36, -42, -48] {
        let y = y_of(db as f32);
        p.line_segment([pos2(sx - 4.0, y), pos2(sx - 1.0, y)], Stroke::new(1.0, t.text_faint));
        p.text(pos2(sx + 2.0, y), Align2::LEFT_CENTER, format!("{db:.1}"), Tokens::ui(10.0), t.text_faint);
    }
    p.text(pos2(sx + 30.0, top - 9.0), Align2::LEFT_CENTER, "dB", Tokens::ui(10.0), t.text_dim);
    // Selected layer's Audio Levels.
    let lx = rect.min.x + 120.0;
    if rect.max.x - lx < 80.0 {
        return;
    }
    let sel = app.session.state.selected_layers.first().copied();
    let levels = sel.and_then(|id| {
        let c = app.session.active_comp()?;
        let l = c.layer(id)?;
        let pr = l.props.sub("audio")?.get("levels")?;
        Some((l.name.clone(), pr.value.as_vec2()))
    });
    match levels {
        Some((name, [a, b])) => {
            p.text(pos2(lx, top - 9.0), Align2::LEFT_CENTER, &name, Tokens::ui(11.0), t.text_dim);
            for (i, (k, v)) in [("Left", a), ("Right", b)].into_iter().enumerate() {
                let y = top + 12.0 + i as f32 * 20.0;
                p.text(pos2(lx, y), Align2::LEFT_CENTER, k, Tokens::ui(12.0), t.text_dim);
                p.text(pos2(lx + 44.0, y), Align2::LEFT_CENTER, format!("{v:+.1} dB"), Tokens::ui(12.0), t.hot_text);
            }
        }
        None => {
            p.text(pos2(lx, top + 12.0), Align2::LEFT_CENTER, "No audio layer selected", Tokens::ui(11.0), t.text_faint);
        }
    }
    if app.meter.active() || app.audio.is_some() {
        ui.ctx().request_repaint();
    }
}

pub fn history(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let mut y = rect.min.y + 6.0;
    let undo: Vec<String> = app.session.history.undo.iter().map(|u| u.0.clone()).collect();
    let redo: Vec<String> = app.session.history.redo.iter().rev().map(|u| u.0.clone()).collect();
    let n = undo.len();
    let mut jump: Option<i64> = None;
    for (i, label) in undo.iter().chain(redo.iter()).enumerate() {
        let r = Rect::from_min_size(pos2(rect.min.x, y), vec2(rect.width(), 22.0));
        y += 22.0;
        let current = i + 1 == n;
        let future = i >= n;
        let resp = ui.interact(r, egui::Id::new(("hist", i)), Sense::click());
        if current {
            p.rect_filled(r, 0.0, t.row_selected);
        } else if resp.hovered() {
            p.rect_filled(r, 0.0, t.hover);
        }
        p.text(pos2(r.min.x + 12.0, r.center().y), Align2::LEFT_CENTER, label, Tokens::ui(12.0), if future { t.text_faint } else { t.text });
        if resp.clicked() {
            jump = Some(i as i64 + 1 - n as i64);
        }
    }
    if let Some(d) = jump {
        for _ in 0..d.unsigned_abs() {
            if d < 0 {
                app.session.undo();
            } else {
                app.session.redo();
            }
        }
    }
}

pub fn markers(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let Some(c) = app.session.active_comp().cloned() else { return };
    let mut y = rect.min.y + 8.0;
    if c.markers.is_empty() {
        p.text(rect.center(), Align2::CENTER_CENTER, "No composition markers", Tokens::ui(12.0), t.text_faint);
    }
    for m in &c.markers {
        let r = Rect::from_min_size(pos2(rect.min.x, y), vec2(rect.width(), 22.0));
        y += 22.0;
        let tc = crate::panels::timecode(&app.session, &c, m.time);
        p.text(pos2(r.min.x + 12.0, r.center().y), Align2::LEFT_CENTER, tc, Tokens::mono(11.5), t.timecode);
        p.text(pos2(r.min.x + 120.0, r.center().y), Align2::LEFT_CENTER, &m.comment, Tokens::ui(12.0), t.text);
        if ui.interact(r, egui::Id::new(("mk", m.time.0)), Sense::click()).clicked() {
            app.session.set_time(m.time);
        }
    }
}
