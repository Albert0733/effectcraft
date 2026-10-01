//! Info panel: colour and position under the pointer, plus the current selection.

use egui::{Align2, Color32, Rect, pos2, vec2};

use crate::EffectcraftApp;
use crate::theme::Tokens;

pub fn show(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    let x0 = rect.min.x + 12.0;
    let mut y = rect.min.y + 14.0;
    let (rgba, xy) = match (app.pointer_comp, &app.viewer_image) {
        (Some([cx, cy]), Some(img)) => {
            let comp = app.session.active_comp();
            let (cw, ch) = comp.map(|c| (c.width as f32, c.height as f32)).unwrap_or((1.0, 1.0));
            let sx = (cx / cw * img.size[0] as f32) as i64;
            let sy = (cy / ch * img.size[1] as f32) as i64;
            let px = if sx >= 0 && sy >= 0 && (sx as usize) < img.size[0] && (sy as usize) < img.size[1] {
                Some(img.pixels[sy as usize * img.size[0] + sx as usize])
            } else {
                None
            };
            (px, Some((cx, cy)))
        }
        _ => (None, None),
    };
    let row = |p: &egui::Painter, y: f32, k: &str, v: String, col: Color32| {
        p.text(pos2(x0, y), Align2::LEFT_CENTER, k, Tokens::ui(12.0), col);
        p.text(pos2(x0 + 22.0, y), Align2::LEFT_CENTER, v, Tokens::mono(12.0), t.text);
    };
    let unpre = |c: Color32| -> [u8; 4] {
        let a = c.a();
        if a == 0 {
            [0, 0, 0, 0]
        } else {
            let f = |v: u8| ((v as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            [f(c.r()), f(c.g()), f(c.b()), a]
        }
    };
    let px = rgba.map(unpre).unwrap_or([0, 0, 0, 0]);
    row(&p, y, "R :", format!("{}", px[0]), Color32::from_rgb(0xe0, 0x60, 0x60));
    row(&p, y + 18.0, "G :", format!("{}", px[1]), Color32::from_rgb(0x60, 0xd0, 0x60));
    row(&p, y + 36.0, "B :", format!("{}", px[2]), Color32::from_rgb(0x60, 0x90, 0xf0));
    row(&p, y + 54.0, "A :", format!("{}", px[3]), t.text_dim);
    let cx = rect.min.x + rect.width() * 0.5;
    if let Some((x, yy)) = xy {
        p.text(pos2(cx, y), Align2::LEFT_CENTER, format!("X : {x:.0}"), Tokens::mono(12.0), t.text);
        p.text(pos2(cx, y + 18.0), Align2::LEFT_CENTER, format!("Y : {yy:.0}"), Tokens::mono(12.0), t.text);
    }
    if let Some(c) = rgba {
        let sw = egui::Rect::from_min_size(pos2(cx, y + 34.0), vec2(36.0, 30.0));
        p.rect_filled(sw, 3.0, Color32::from_rgb(px[0], px[1], px[2]));
        let _ = c;
    }
    y += 82.0;
    p.line_segment([pos2(rect.min.x + 8.0, y), pos2(rect.max.x - 8.0, y)], egui::Stroke::new(1.0, t.separator));
    y += 14.0;
    let comp = app.session.active_comp().cloned();
    if let Some(c) = comp {
        let sel: Vec<String> = app.session.state.selected_layers.iter().filter_map(|id| c.layer(*id)).map(|l| l.name.clone()).collect();
        let tc = crate::panels::timecode(&app.session, &c, app.session.time());
        p.text(pos2(x0, y), Align2::LEFT_CENTER, if sel.is_empty() { "No layer selected".to_string() } else { sel.join(", ") }, Tokens::semibold(12.0), t.text);
        p.text(pos2(x0, y + 18.0), Align2::LEFT_CENTER, format!("Time: {tc}"), Tokens::ui(12.0), t.text_dim);
        let ms = app.frames.last_ms.lock().map(|v| *v).unwrap_or(0.0);
        p.text(pos2(x0, y + 36.0), Align2::LEFT_CENTER, format!("Render: {ms:.0} ms  •  UI {:.0} fps", app.fps), Tokens::ui(11.5), t.text_faint);
        if !app.session.state.selected_keys.is_empty() {
            p.text(
                pos2(x0, y + 54.0),
                Align2::LEFT_CENTER,
                format!("{} keyframes selected", app.session.state.selected_keys.len()),
                Tokens::ui(11.5),
                t.text_dim,
            );
        }
    }
}
