//! The Home screen (shown in the Composition panel at launch, from the header's Home button, and
//! by the Learn workspace): New Project / Open Project / New Composition / Open Demo, the recent
//! projects (File ▸ Open Recent, from Settings) with thumbnails, folders and dates, and where
//! After Effects has its Learn / What's New area, the ArtCraft community links.
//!
//! Thumbnails: when a project is opened or saved and the viewer has its frame, a 96×54 RGB
//! thumbnail is stored in the config store (`thumb-<hash>.txt`, hex) and shown here.

use std::hash::{Hash, Hasher};

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde::Serialize;
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::{EffectcraftApp, widgets};

pub const THUMB_W: usize = 96;
pub const THUMB_H: usize = 54;

/// One row of the recent projects list.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecentEntry {
    pub index: usize,
    pub path: String,
    pub name: String,
    pub folder: String,
    /// Last modified (`2026-10-02 14:05`, UTC), when the file is readable.
    pub modified: Option<String>,
    pub exists: bool,
}

/// The recent projects, most recent first (Settings ▸ General ▸ recent items).
pub fn recent_entries(prefs: &effectcraft_engine::prefs::Prefs) -> Vec<RecentEntry> {
    prefs
        .recent_projects
        .iter()
        .enumerate()
        .map(|(index, p)| {
            let path = std::path::Path::new(p);
            let meta = std::fs::metadata(path).ok();
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| format_utc(d.as_secs() as i64));
            RecentEntry {
                index,
                path: p.clone(),
                name: path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone()),
                folder: path.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default(),
                modified,
                exists: meta.is_some(),
            }
        })
        .collect()
}

/// `YYYY-MM-DD HH:MM` (UTC) from Unix seconds.
pub fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

/// The config-store name of a project's thumbnail.
pub fn thumb_name(path: &str) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!("thumb-{:016x}.txt", h.finish())
}

/// Downscale an image to a letterboxed THUMB_W×THUMB_H RGB thumbnail (hex text).
pub fn encode_thumb(img: &egui::ColorImage) -> String {
    let [w, h] = img.size;
    let mut out = String::with_capacity(THUMB_W * THUMB_H * 6);
    let k = (THUMB_W as f32 / w.max(1) as f32).min(THUMB_H as f32 / h.max(1) as f32);
    let (dw, dh) = (w as f32 * k, h as f32 * k);
    let (ox, oy) = ((THUMB_W as f32 - dw) / 2.0, (THUMB_H as f32 - dh) / 2.0);
    for y in 0..THUMB_H {
        for x in 0..THUMB_W {
            let (fx, fy) = ((x as f32 + 0.5 - ox) / k, (y as f32 + 0.5 - oy) / k);
            let c = if fx >= 0.0 && fy >= 0.0 && (fx as usize) < w && (fy as usize) < h {
                let p = img.pixels[fy as usize * w + fx as usize];
                [p.r(), p.g(), p.b()]
            } else {
                [0x16, 0x17, 0x1c]
            };
            for v in c {
                out.push_str(&format!("{v:02x}"));
            }
        }
    }
    out
}

pub fn decode_thumb(text: &str) -> Option<egui::ColorImage> {
    let t = text.trim();
    if t.len() != THUMB_W * THUMB_H * 6 {
        return None;
    }
    let b: Vec<u8> = (0..t.len()).step_by(2).map(|i| u8::from_str_radix(&t[i..i + 2], 16)).collect::<Result<_, _>>().ok()?;
    let px = b.chunks_exact(3).map(|c| Color32::from_rgb(c[0], c[1], c[2])).collect();
    Some(egui::ColorImage::new([THUMB_W, THUMB_H], px))
}

/// Store the viewer's frame as the open project's thumbnail after it is opened or saved (once per
/// saved revision, when the viewer shows that revision).
pub fn capture_thumbnail(app: &mut EffectcraftApp) {
    let s = &app.session;
    let (Some(path), Some(_)) = (s.path.clone(), s.config.as_ref()) else { return };
    if s.is_dirty() {
        return;
    }
    let key = (path.clone(), s.saved_revision);
    if app.home_thumb_saved.as_ref() == Some(&key) {
        return;
    }
    if app.viewer_shown.as_ref().map(|(_, k)| k.revision) != Some(s.revision) {
        return;
    }
    let Some(img) = app.viewer_pixels() else { return };
    let text = encode_thumb(&img);
    if let Some(c) = &app.session.config
        && let Err(e) = c.write(&thumb_name(&path), &text)
    {
        log::warn!("project thumbnail: {e}");
    }
    app.home_thumbs.remove(&path);
    app.home_thumb_saved = Some(key);
}

fn thumb_texture(app: &mut EffectcraftApp, ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
    if let Some(t) = app.home_thumbs.get(path) {
        return t.clone();
    }
    let tex = app
        .session
        .config
        .as_ref()
        .and_then(|c| c.read(&thumb_name(path)))
        .and_then(|t| decode_thumb(&t))
        .map(|img| ctx.load_texture(format!("home-thumb-{path}"), img, egui::TextureOptions::LINEAR));
    app.home_thumbs.insert(path.to_string(), tex.clone());
    tex
}

/// Draw the Home screen.
pub fn show(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 0.0, Color32::from_rgb(0x1b, 0x1d, 0x24));
    let wide = rect.width() >= 820.0;
    let pad = 32.0;
    let left_w = if wide { 250.0 } else { rect.width() - pad * 2.0 };
    let mut actions: Vec<(&str, serde_json::Value)> = vec![];

    // ---- left: brand + start actions.
    let x0 = rect.min.x + pad;
    let mut y = rect.min.y + pad;
    crate::header::paint_logo(&p, Rect::from_min_size(pos2(x0, y), vec2(40.0, 40.0)));
    p.text(pos2(x0 + 52.0, y + 12.0), Align2::LEFT_CENTER, "EffectCraft", Tokens::semibold(22.0), Color32::WHITE);
    p.text(pos2(x0 + 52.0, y + 32.0), Align2::LEFT_CENTER, format!("Version {}", env!("CARGO_PKG_VERSION")), Tokens::ui(11.0), t.text_faint);
    y += 64.0;
    for (label, id, primary) in [
        ("New Project", "file.newProject", true),
        ("Open Project…", "file.open", false),
        ("New Composition", "app.newComp", false),
        ("Open Demo Project", "file.openDemoProject", false),
        ("Import Footage…", "file.import", false),
    ] {
        let r = Rect::from_min_size(pos2(x0, y), vec2(left_w.min(250.0), 32.0));
        if widgets::text_button(ui, r, label, primary, &t, egui::Id::new(("home", id))).clicked() {
            actions.push((id, json!({})));
        }
        app.auto.add(&format!("home.{id}"), r, label);
        y += 40.0;
    }

    // ---- community (After Effects' Learn / What's New area).
    let (cx, mut cy, cw) = if wide { (x0, y + 18.0, left_w) } else { (x0, y + 10.0, left_w) };
    p.text(pos2(cx, cy), Align2::LEFT_CENTER, "Community", Tokens::semibold(13.0), t.text);
    cy += 16.0;
    p.text(pos2(cx, cy), Align2::LEFT_CENTER, "Tutorials, help and what's new", Tokens::ui(11.0), t.text_faint);
    cy += 16.0;
    let links = [
        (Icon::Chat, "Join the ArtCraft Discord", "help.discord"),
        (Icon::Globe, "getartcraft.com", "help.website"),
        (Icon::Globe, "EffectCraft home page", "help.appPage"),
        (Icon::Code, "EffectCraft on GitHub", "help.github"),
    ];
    for (icon, label, cmd) in links {
        let r = Rect::from_min_size(pos2(cx, cy), vec2(cw.min(250.0), 30.0));
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
        p.text(pos2(r.min.x + 36.0, r.center().y), Align2::LEFT_CENTER, label, Tokens::medium(12.0), Color32::WHITE);
        app.auto.add(&format!("home.{cmd}"), r, label);
        if resp.clicked() {
            let _ = app.session.execute(cmd, json!({}));
        }
        cy += 36.0;
    }
    cy += 8.0;
    p.text(pos2(cx, cy), Align2::LEFT_CENTER, "More ArtCraft apps", Tokens::ui(11.0), t.text_faint);
    cy += 12.0;
    let sib = effectcraft_engine::links::SIBLINGS;
    let per_row = 2usize;
    let sw = (cw.min(250.0) - 6.0) / per_row as f32;
    for (i, (name, slug)) in sib.iter().enumerate() {
        let r = Rect::from_min_size(pos2(cx + (i % per_row) as f32 * (sw + 6.0), cy + (i / per_row) as f32 * 28.0), vec2(sw, 24.0));
        let resp = ui.interact(r, egui::Id::new(("sib", *slug)), Sense::click());
        p.rect_filled(r, 12.0, if resp.hovered() { t.hover } else { Color32::from_rgb(0x24, 0x26, 0x2e) });
        p.text(r.center(), Align2::CENTER_CENTER, *name, Tokens::ui(11.5), t.text);
        app.auto.add(&format!("home.sibling.{slug}"), r, name);
        if resp.clicked() {
            let _ = app.session.execute("help.sibling", json!({"app": slug}));
        }
    }
    let community_bottom = cy + sib.len().div_ceil(per_row) as f32 * 28.0;

    // ---- recent projects.
    let (rx, mut ry, rw) = if wide {
        let rx = x0 + left_w + 40.0;
        (rx, rect.min.y + pad + 6.0, rect.max.x - pad - rx)
    } else {
        (x0, community_bottom + 24.0, left_w)
    };
    p.text(pos2(rx, ry + 6.0), Align2::LEFT_CENTER, "Recent", Tokens::semibold(16.0), Color32::WHITE);
    let entries = recent_entries(&app.session.prefs);
    if !entries.is_empty() {
        let cr = Rect::from_min_size(pos2(rx + rw - 100.0, ry - 4.0), vec2(100.0, 20.0));
        let resp = ui.interact(cr, egui::Id::new("home-clear-recent"), Sense::click());
        p.text(cr.right_center(), Align2::RIGHT_CENTER, "Clear list", Tokens::ui(11.0), if resp.hovered() { t.text } else { t.text_faint });
        app.auto.add("home.clearRecent", cr, "Clear Recent Projects");
        if resp.clicked() {
            actions.push(("file.clearRecent", json!({})));
        }
    }
    ry += 26.0;
    // Column headings.
    let date_w = 130.0;
    p.text(pos2(rx + 112.0, ry), Align2::LEFT_CENTER, "Name", Tokens::ui(11.0), t.text_faint);
    p.text(pos2(rx + rw - date_w, ry), Align2::LEFT_CENTER, "Modified", Tokens::ui(11.0), t.text_faint);
    ry += 10.0;
    p.line_segment([pos2(rx, ry), pos2(rx + rw, ry)], Stroke::new(1.0, t.separator));
    ry += 6.0;
    if entries.is_empty() {
        p.text(pos2(rx, ry + 20.0), Align2::LEFT_CENTER, "No recent projects. Projects you open or save appear here.", Tokens::ui(12.0), t.text_faint);
    }
    let row_h = 66.0;
    for e in &entries {
        let r = Rect::from_min_size(pos2(rx, ry), vec2(rw, row_h - 4.0));
        if r.min.y > rect.max.y {
            break;
        }
        let resp = ui.interact(r, egui::Id::new(("home-recent", e.index)), Sense::click()).on_hover_text(&e.path);
        if resp.hovered() {
            p.rect_filled(r, 6.0, t.hover);
        }
        let tr = Rect::from_min_size(pos2(r.min.x + 6.0, r.min.y + 4.0), vec2(THUMB_W as f32, THUMB_H as f32));
        match thumb_texture(app, &ctx, &e.path) {
            Some(tex) => {
                p.image(tex.id(), tr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.rect_filled(tr, 3.0, Color32::from_rgb(0x24, 0x26, 0x2e));
                icons::paint(&p, Rect::from_center_size(tr.center(), vec2(20.0, 20.0)), Icon::Comp, t.text_faint);
            }
        }
        p.rect_stroke(tr, 3.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
        let tx = tr.max.x + 12.0;
        let text_clip = p.with_clip_rect(Rect::from_min_max(pos2(tx, r.min.y), pos2(r.max.x - date_w - 8.0, r.max.y)));
        text_clip.text(pos2(tx, r.min.y + 20.0), Align2::LEFT_CENTER, &e.name, Tokens::semibold(13.0), if e.exists { Color32::WHITE } else { t.text_dim });
        let sub = if e.exists { e.folder.clone() } else { format!("{} (missing)", e.folder) };
        text_clip.text(pos2(tx, r.min.y + 40.0), Align2::LEFT_CENTER, sub, Tokens::ui(11.0), t.text_faint);
        if let Some(d) = &e.modified {
            p.text(pos2(r.max.x - date_w, r.min.y + 20.0), Align2::LEFT_CENTER, d, Tokens::ui(11.0), t.text_dim);
        }
        app.auto.add(&format!("home.recent.{}", e.index), r, &e.path);
        if resp.clicked() {
            actions.push(("file.openRecent", json!({"index": e.index})));
        }
        ry += row_h;
    }

    for (id, params) in actions {
        let before = (app.session.path.clone(), app.session.revision);
        match crate::menus::invoke(app, &ctx, id, params) {
            Err(e) => app.ui.status = e,
            // Leave Home unless only the list changed or a file dialog was cancelled.
            Ok(_) => {
                let cancelled = matches!(id, "file.open" | "file.import") && (app.session.path.clone(), app.session.revision) == before;
                if id != "file.clearRecent" && !cancelled {
                    app.ui.start_screen = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_list_comes_from_settings() {
        let mut prefs = effectcraft_engine::prefs::Prefs::default();
        prefs.push_recent("/projects/a/Alpha.ecproj");
        prefs.push_recent("/projects/b/Beta.ecproj");
        let e = recent_entries(&prefs);
        assert_eq!(e.len(), 2);
        assert_eq!((e[0].name.as_str(), e[0].folder.as_str(), e[0].index), ("Beta", "/projects/b", 0));
        assert_eq!(e[1].name, "Alpha");
        assert!(!e[0].exists && e[0].modified.is_none());
        // A real file has a date.
        let dir = std::env::temp_dir().join(format!("ec-home-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("Real.ecproj");
        std::fs::write(&f, "{}").unwrap();
        prefs.push_recent(&f.to_string_lossy());
        let e = recent_entries(&prefs);
        assert!(e[0].exists && e[0].modified.as_ref().is_some_and(|d| d.len() == 16), "{:?}", e[0]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dates_and_thumbnails() {
        assert_eq!(format_utc(0), "1970-01-01 00:00");
        assert_eq!(format_utc(1_790_000_000), "2026-09-21 14:13");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00");
        let img = egui::ColorImage::new([200, 100], vec![Color32::from_rgb(10, 200, 30); 200 * 100]);
        let text = encode_thumb(&img);
        let back = decode_thumb(&text).unwrap();
        assert_eq!(back.size, [THUMB_W, THUMB_H]);
        // Letterboxed: the middle is the image, the top row is the background (2:1 into 16:9).
        assert_eq!(back.pixels[THUMB_H / 2 * THUMB_W + THUMB_W / 2], Color32::from_rgb(10, 200, 30));
        assert_ne!(back.pixels[THUMB_W / 2], Color32::from_rgb(10, 200, 30));
        assert!(decode_thumb("abc").is_none());
        assert_ne!(thumb_name("/a.ecproj"), thumb_name("/b.ecproj"));
    }

    #[test]
    fn home_screen_registers_its_controls() {
        let mut s = effectcraft_engine::Session::default();
        s.prefs.push_recent("/projects/Alpha.ecproj");
        let mut app = EffectcraftApp::new(s);
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, &app.tokens);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        ctx.run_ui(Default::default(), |ui| {
            app.auto.begin_frame();
            show(&mut app, ui, Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0)));
        })
        .textures_delta
        .clear();
        for id in [
            "home.file.newProject",
            "home.file.open",
            "home.app.newComp",
            "home.file.openDemoProject",
            "home.help.discord",
            "home.help.github",
            "home.recent.0",
        ] {
            assert!(app.auto.find(id).is_some(), "{id}");
        }
    }
}
