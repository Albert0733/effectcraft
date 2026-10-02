//! EffectCraft text engine.
//!
//! - [`fonts`], [`sfnt`], [`layout`]: font database, shaping (harfrust), bidi, line breaking and
//!   paragraph layout (shared design with FilmCraft's text engine).
//! - This module turns a layer's [`TextDoc`] into per-character glyph outlines (Bezier paths) with
//!   character / word / line indices, which is what text animators and selectors work on.

pub mod fonts;
pub mod layout;
pub mod path_text;
pub mod selectors;
pub mod sfnt;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use effectcraft_keyframe::{Justify, TextDoc};
pub use fonts::{FaceId, Resolved, families, resolve};
use kurbo::{Affine, BezPath, Point};
pub use layout::{Align, Caps, Glyph, Layout, Line, ParagraphStyle, TextStyle, layout as layout_text, measure};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{GlyphId, MetadataProvider};

struct Pen(BezPath);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((x as f64, y as f64));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to((cx0 as f64, cy0 as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to((cx0 as f64, cy0 as f64), (cx1 as f64, cy1 as f64), (x as f64, y as f64));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

/// Glyph outline in font units (y up), cached.
fn outline_units(face: FaceId, gid: u32) -> Option<Arc<BezPath>> {
    static C: OnceLock<Mutex<HashMap<(FaceId, u32), Option<Arc<BezPath>>>>> = OnceLock::new();
    let c = C.get_or_init(Default::default);
    if let Some(v) = c.lock().unwrap_or_else(|e| e.into_inner()).get(&(face, gid)) {
        return v.clone();
    }
    let f = fonts::face(face);
    let v = f.font().and_then(|font| {
        let g = font.outline_glyphs().get(GlyphId::new(gid))?;
        let mut pen = Pen(BezPath::new());
        g.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
        Some(Arc::new(pen.0))
    });
    let mut m = c.lock().unwrap_or_else(|e| e.into_inner());
    if m.len() > 50_000 {
        m.clear();
    }
    m.insert((face, gid), v.clone());
    v
}

/// The outline of a laid-out glyph in pixels, origin on its baseline (y down).
pub fn glyph_outline(g: &Glyph) -> BezPath {
    let Some(units) = outline_units(g.face, g.id) else { return BezPath::new() };
    let k = g.size as f64 / fonts::face(g.face).units_per_em() as f64;
    let slant = if g.synth_italic { 0.21 } else { 0.0 };
    Affine::new([k, 0.0, slant * k, -k, 0.0, 0.0]) * (*units).clone()
}

/// One character of laid-out text.
#[derive(Clone, Debug)]
pub struct CharGlyph {
    /// Outline relative to `origin`.
    pub path: BezPath,
    /// Baseline origin in layer space.
    pub origin: Point,
    pub advance: f64,
    /// Indices for selectors (characters, characters excluding spaces, words, lines).
    pub char_index: usize,
    pub char_index_no_space: usize,
    pub word_index: usize,
    pub line_index: usize,
    pub is_space: bool,
    /// The source character (after All Caps).
    pub ch: char,
    pub synth_bold: bool,
    pub size: f64,
}

#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub glyphs: Vec<CharGlyph>,
    pub chars: usize,
    pub chars_no_space: usize,
    pub words: usize,
    pub lines: usize,
    /// Bounds in layer space (x0, y0, x1, y1).
    pub bounds: [f64; 4],
    /// Line baselines (layer space) and widths for the viewer's text cursor.
    pub line_boxes: Vec<[f64; 4]>,
}

/// Lay out a Source Text value in layer space. Point text: the origin is the start of the first
/// baseline (left), its centre (centre) or end (right). Paragraph text fills `box_size` from
/// `box_pos`.
pub fn layout_doc(doc: &TextDoc) -> TextLayout {
    let text = if doc.all_caps { doc.text.to_uppercase() } else { doc.text.clone() };
    let (style, para) = styles(doc);
    let lay = layout_text(&text, &style, &para);
    let first_base = lay.lines.first().map(|l| l.baseline as f64).unwrap_or(0.0);
    // Point text origin: (0, 0) at the first baseline; alignment pivots around x = 0.
    let (dx, dy) = match doc.box_size {
        Some(_) => (doc.box_pos[0], doc.box_pos[1]),
        // The layout already pivots point text around x = 0 by alignment.
        None => (0.0, -first_base),
    };
    let hs = doc.h_scale / 100.0;
    let vs = doc.v_scale / 100.0;
    let mut out = TextLayout { lines: lay.lines.len(), ..Default::default() };
    // Map byte clusters → char/word indices.
    let mut char_of_byte = vec![0usize; text.len() + 1];
    let mut word_of_char = Vec::new();
    let mut nospace_of_char = Vec::new();
    let mut words = 0usize;
    let mut in_word = false;
    let mut nospace = 0usize;
    for (ci, (b, ch)) in text.char_indices().enumerate() {
        for k in b..b + ch.len_utf8() {
            char_of_byte[k] = ci;
        }
        let space = ch.is_whitespace();
        if !space && !in_word {
            words += 1;
        }
        in_word = !space;
        word_of_char.push(words.saturating_sub(1));
        nospace_of_char.push(nospace);
        if !space {
            nospace += 1;
        }
    }
    out.chars = text.chars().count();
    out.chars_no_space = nospace;
    out.words = words;
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (li, line) in lay.lines.iter().enumerate() {
        out.line_boxes.push([line.x as f64 + dx, line.baseline as f64 + dy, line.width as f64, (line.ascent + line.descent) as f64]);
        for gi in line.glyphs.clone() {
            let g = &lay.glyphs[gi];
            let ci = char_of_byte.get(g.cluster).copied().unwrap_or(0);
            let ch = text[g.cluster..].chars().next().unwrap_or(' ');
            let mut path = glyph_outline(g);
            if hs != 1.0 || vs != 1.0 {
                path = Affine::scale_non_uniform(hs, vs) * path;
            }
            let next_x = lay.glyphs.get(gi + 1).filter(|_| line.glyphs.contains(&(gi + 1))).map(|n| n.x).unwrap_or(line.x + line.width);
            let origin = Point::new(g.x as f64 * hs + dx, g.y as f64 + dy);
            if let Some(r) = (!path.elements().is_empty()).then(|| kurbo::Shape::bounding_box(&path)) {
                b[0] = b[0].min(origin.x + r.x0);
                b[1] = b[1].min(origin.y + r.y0);
                b[2] = b[2].max(origin.x + r.x1);
                b[3] = b[3].max(origin.y + r.y1);
            }
            out.glyphs.push(CharGlyph {
                path,
                origin,
                advance: (next_x - g.x) as f64 * hs,
                char_index: ci,
                char_index_no_space: nospace_of_char.get(ci).copied().unwrap_or(0),
                word_index: word_of_char.get(ci).copied().unwrap_or(0),
                line_index: li,
                is_space: ch.is_whitespace(),
                ch,
                synth_bold: g.synth_bold,
                size: g.size as f64,
            });
        }
    }
    if b[0].is_finite() {
        out.bounds = b;
    }
    out
}

/// Character and paragraph styles of a Source Text value.
fn styles(doc: &TextDoc) -> (TextStyle, ParagraphStyle) {
    let style = TextStyle {
        family: doc.font.clone(),
        style: doc.style.clone(),
        size: doc.size as f32,
        tracking: doc.tracking as f32,
        baseline_shift: doc.baseline_shift as f32,
        faux_bold: doc.faux_bold,
        faux_italic: doc.faux_italic,
        caps: if doc.small_caps { Caps::Small } else { Caps::Normal },
        ..Default::default()
    };
    let align = match doc.justify {
        Justify::Left | Justify::JustifyLastLeft => Align::Left,
        Justify::Center | Justify::JustifyLastCenter => Align::Center,
        Justify::Right | Justify::JustifyLastRight => Align::Right,
        Justify::JustifyAll => Align::Justify,
    };
    let natural = doc.size * 1.2;
    let leading = doc.leading.map(|l| (l - natural) as f32).unwrap_or(0.0);
    (style, ParagraphStyle { align, leading, width: doc.box_size.map(|b| b[0] as f32), rtl: None })
}

/// Outline (origin on the baseline) and advance of a single character in a Source Text's
/// style: used by Character Offset / Character Value substitutions.
pub fn char_glyph(doc: &TextDoc, ch: char) -> (BezPath, f64) {
    let (style, _) = styles(doc);
    let lay = layout_text(&ch.to_string(), &style, &ParagraphStyle::default());
    let hs = doc.h_scale / 100.0;
    let vs = doc.v_scale / 100.0;
    let Some(g) = lay.glyphs.first() else { return (BezPath::new(), 0.0) };
    let mut path = glyph_outline(g);
    if hs != 1.0 || vs != 1.0 {
        path = Affine::scale_non_uniform(hs, vs) * path;
    }
    let adv = lay.lines.first().map(|l| l.width as f64).unwrap_or(0.0) * hs;
    (Affine::translate((-(g.x as f64) * hs, 0.0)) * path, adv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_counts_and_bounds() {
        let doc = TextDoc { text: "Hello big world".into(), size: 100.0, ..Default::default() };
        let l = layout_doc(&doc);
        assert_eq!(l.chars, 15);
        assert_eq!(l.words, 3);
        assert_eq!(l.chars_no_space, 13);
        assert!(l.bounds[2] > 500.0, "{:?}", l.bounds);
        // Baseline at y = 0: caps rise above it.
        assert!(l.bounds[1] < -60.0 && l.bounds[3] < 30.0, "{:?}", l.bounds);
        assert!(l.glyphs.iter().filter(|g| !g.is_space).all(|g| !g.path.elements().is_empty()));
    }

    #[test]
    fn centered_point_text_straddles_origin() {
        let doc = TextDoc { text: "CENTER".into(), size: 80.0, justify: Justify::Center, ..Default::default() };
        let l = layout_doc(&doc);
        assert!(l.bounds[0] < -100.0 && l.bounds[2] > 100.0, "{:?}", l.bounds);
        assert!((l.bounds[0] + l.bounds[2]).abs() < 20.0);
    }
}
