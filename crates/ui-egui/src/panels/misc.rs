//! Smaller panels: Preview, Audio, History, Markers, Wiggler, Render Queue, the Home screen and
//! placeholders.

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;

use crate::dock::PanelKind;
use crate::icons::{self, Icon};
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
        let scale = app.viewer_tex.as_ref().map(|(_, k)| k.scale).unwrap_or(1000);
        let cached = app.frames.cached_frames(app.session.revision, cid, scale).len();
        let total = c.frame_rate.frame_at(c.work_area.1) - c.frame_rate.frame_at(c.work_area.0);
        let bar = Rect::from_min_size(pos2(rect.min.x + 12.0, yy), vec2(rect.width() - 24.0, 6.0));
        p.rect_filled(bar, 3.0, t.field_bg);
        let f = (cached as f32 / total.max(1) as f32).min(1.0);
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height())), 3.0, t.cache_green);
        p.text(pos2(bar.min.x, bar.max.y + 12.0), Align2::LEFT_CENTER, format!("{cached} / {total} frames cached"), Tokens::ui(11.0), t.text_faint);
    }
}

pub fn audio(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    // Level meters (silent until audio playback lands).
    for (i, x) in [rect.min.x + 20.0, rect.min.x + 34.0].into_iter().enumerate() {
        let r = Rect::from_min_max(pos2(x, rect.min.y + 14.0), pos2(x + 10.0, rect.max.y - 20.0));
        p.rect_filled(r, 1.0, t.field_bg);
        let _ = i;
    }
    for (k, db) in [0, -6, -12, -18, -24, -36, -48].iter().enumerate() {
        let y = rect.min.y + 14.0 + (rect.height() - 34.0) * (-*db as f32 / 48.0);
        p.text(pos2(rect.min.x + 52.0, y), Align2::LEFT_CENTER, format!("{db} dB"), Tokens::ui(10.0), t.text_faint);
        let _ = k;
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

/// Wiggler: add random keyframes between two selected keys (applied as a wiggle expression).
pub fn wiggler(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let freq_id = egui::Id::new("wiggler-freq");
    let mag_id = egui::Id::new("wiggler-mag");
    let mut freq: f64 = ui.data(|d| d.get_temp(freq_id).unwrap_or(5.0));
    let mut mag: f64 = ui.data(|d| d.get_temp(mag_id).unwrap_or(30.0));
    let x0 = rect.min.x + 12.0;
    let mut y = rect.min.y + 14.0;
    p.text(pos2(x0, y + 8.0), Align2::LEFT_CENTER, "Frequency:", Tokens::ui(12.0), t.text_dim);
    let (_, nv, _) = widgets::hot_number_at(ui, pos2(x0 + 110.0, y), egui::Id::new("wg-f"), freq, 0.1, (0.1, 100.0), 1, " per second", &t);
    if let Some(v) = nv {
        freq = v;
    }
    y += 26.0;
    p.text(pos2(x0, y + 8.0), Align2::LEFT_CENTER, "Magnitude:", Tokens::ui(12.0), t.text_dim);
    let (_, nv, _) = widgets::hot_number_at(ui, pos2(x0 + 110.0, y), egui::Id::new("wg-m"), mag, 0.5, (0.0, 10000.0), 1, "", &t);
    if let Some(v) = nv {
        mag = v;
    }
    ui.data_mut(|d| {
        d.insert_temp(freq_id, freq);
        d.insert_temp(mag_id, mag);
    });
    y += 34.0;
    let b = Rect::from_min_size(pos2(rect.max.x - 92.0, y), vec2(80.0, 24.0));
    if widgets::text_button(ui, b, "Apply", true, &t, egui::Id::new("wg-apply")).clicked() {
        let props = app.session.state.selected_props.clone();
        if props.is_empty() {
            app.ui.status = "Select a property in the Timeline first".into();
        }
        for (l, u) in props {
            let _ = app.session.execute("prop.setExpression", json!({"layer": l.0, "prop": u, "expression": format!("wiggle({freq}, {mag})")}));
        }
    }
    app.auto.add("wiggler.apply", b, "Apply");
}

/// Render Queue: comps queued for export with output settings.
pub fn render_queue(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let hdr = Rect::from_min_size(rect.min, vec2(rect.width(), 40.0));
    p.text(pos2(hdr.min.x + 14.0, hdr.center().y), Align2::LEFT_CENTER, "Render Queue", Tokens::semibold(13.0), t.text);
    let b = Rect::from_min_size(pos2(hdr.max.x - 100.0, hdr.min.y + 8.0), vec2(84.0, 24.0));
    let _ = widgets::text_button(ui, b, "Render", true, &t, egui::Id::new("rq-render"));
    app.auto.add("renderQueue.render", b, "Render");
    p.line_segment([hdr.left_bottom(), hdr.right_bottom()], Stroke::new(1.0, t.separator));
    p.text(
        pos2(rect.min.x + 14.0, hdr.max.y + 20.0),
        Align2::LEFT_CENTER,
        "Composition ▸ Add to Render Queue (⌃⌘M) to queue a render.",
        Tokens::ui(12.0),
        t.text_faint,
    );
}

/// Home screen: new/open, recent, and community links.
pub fn start_screen(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 0.0, Color32::from_rgb(0x1b, 0x1d, 0x24));
    let c = rect.center();
    let logo = Rect::from_center_size(c + vec2(0.0, -150.0), vec2(64.0, 64.0));
    crate::header::paint_logo(&p, logo);
    p.text(c + vec2(0.0, -96.0), Align2::CENTER_CENTER, "EffectCraft", Tokens::semibold(28.0), Color32::WHITE);
    p.text(c + vec2(0.0, -66.0), Align2::CENTER_CENTER, "Motion graphics and visual effects, in pure Rust.", Tokens::ui(13.0), t.text_dim);
    let mut y = c.y - 30.0;
    for (label, id, primary) in [
        ("New Composition", "app.newComp", true),
        ("Open Demo Project", "file.openDemoProject", false),
        ("Open Project…", "file.open", false),
        ("Import Footage…", "file.import", false),
    ] {
        let r = Rect::from_center_size(pos2(c.x, y + 16.0), vec2(240.0, 32.0));
        if widgets::text_button(ui, r, label, primary, &t, egui::Id::new(("home", id))).clicked() {
            let _ = crate::menus::invoke(app, &ctx, id, json!({}));
            app.ui.start_screen = false;
        }
        app.auto.add(&format!("home.{id}"), r, label);
        y += 40.0;
    }
    // Community.
    y += 14.0;
    let links = [(Icon::Chat, "Join the Discord", "help.discord"), (Icon::Globe, "getartcraft.com", "help.website"), (Icon::Code, "GitHub", "help.github")];
    let w = 170.0;
    let mut x = c.x - w * 1.5 - 8.0;
    for (icon, label, cmd) in links {
        let r = Rect::from_min_size(pos2(x, y), vec2(w, 30.0));
        let resp = ui.interact(r, egui::Id::new(("home-link", cmd)), Sense::click());
        let discord = cmd == "help.discord";
        let bg = if discord {
            Color32::from_rgb(0x58, 0x65, 0xf2)
        } else if resp.hovered() {
            t.hover
        } else {
            Color32::from_rgb(0x2a, 0x2c, 0x34)
        };
        p.rect_filled(r, 15.0, bg);
        icons::paint(&p, Rect::from_center_size(pos2(r.min.x + 20.0, r.center().y), vec2(14.0, 14.0)), icon, Color32::WHITE);
        p.text(pos2(r.min.x + 34.0, r.center().y), Align2::LEFT_CENTER, label, Tokens::medium(12.0), Color32::WHITE);
        app.auto.add(&format!("home.{cmd}"), r, label);
        if resp.clicked() {
            let _ = app.session.execute(cmd, json!({}));
        }
        x += w + 8.0;
    }
    // Siblings.
    y += 52.0;
    p.text(pos2(c.x, y), Align2::CENTER_CENTER, "More ArtCraft apps", Tokens::ui(11.5), t.text_faint);
    y += 18.0;
    let sib = effectcraft_engine::links::SIBLINGS;
    let sw = 104.0;
    let mut x = c.x - sw * sib.len() as f32 / 2.0;
    for (name, slug) in sib {
        let r = Rect::from_min_size(pos2(x + 2.0, y), vec2(sw - 4.0, 24.0));
        let resp = ui.interact(r, egui::Id::new(("sib", *slug)), Sense::click());
        p.rect_filled(r, 12.0, if resp.hovered() { t.hover } else { Color32::from_rgb(0x24, 0x26, 0x2e) });
        p.text(r.center(), Align2::CENTER_CENTER, *name, Tokens::ui(11.5), t.text);
        if resp.clicked() {
            let _ = app.session.execute("help.sibling", json!({"app": slug}));
        }
        x += sw;
    }
}
