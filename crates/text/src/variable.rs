//! Variable fonts: a face's variation axes and glyph outlines at a point of its design space
//! (Animate Text ▸ Variable Font Axes).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use kurbo::BezPath;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::DrawSettings;
use skrifa::{GlyphId, MetadataProvider, Tag};

use crate::fonts::{self, FaceId};
use crate::{CharGlyph, Pen};

/// One variation axis of a face, in user units.
#[derive(Clone, Debug, PartialEq)]
pub struct FontAxis {
    /// Four-letter tag (`wght`, `wdth`, `opsz`, `slnt`, `ital` or a custom one).
    pub tag: String,
    pub name: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

/// The variation axes of a face (empty for static fonts).
pub fn font_axes(face: FaceId) -> Vec<FontAxis> {
    let f = fonts::face(face);
    let Some(font) = f.font() else { return Vec::new() };
    font.axes()
        .iter()
        .map(|a| {
            let tag = String::from_utf8_lossy(&a.tag().to_be_bytes()).to_string();
            let name = font.localized_strings(a.name_id()).english_or_first().map(|s| s.to_string()).unwrap_or_else(|| tag.clone());
            FontAxis { tag, name, min: a.min_value(), default: a.default_value(), max: a.max_value() }
        })
        .collect()
}

/// A tag string as a [`Tag`] (padded with spaces, at most four characters).
fn tag_of(s: &str) -> Tag {
    let mut b = [b' '; 4];
    for (i, c) in s.bytes().take(4).enumerate() {
        b[i] = c;
    }
    Tag::new(&b)
}

type Key = (FaceId, u32, Vec<(String, i32)>);

/// A glyph's outline in font units at the given axis values (user units, unspecified axes at
/// their defaults).
pub fn outline_units_at(face: FaceId, gid: u32, coords: &[(String, f32)]) -> Option<Arc<BezPath>> {
    if coords.is_empty() {
        return crate::outline_units(face, gid);
    }
    static C: OnceLock<Mutex<HashMap<Key, Option<Arc<BezPath>>>>> = OnceLock::new();
    let c = C.get_or_init(Default::default);
    // Quantise to 1/100 of a unit so animation frames reuse outlines.
    let key: Key = (face, gid, coords.iter().map(|(t, v)| (t.clone(), (v * 100.0).round() as i32)).collect());
    if let Some(v) = c.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return v.clone();
    }
    let f = fonts::face(face);
    let v = f.font().and_then(|font| {
        let loc = font.axes().location(coords.iter().map(|(t, v)| (tag_of(t), *v)));
        let g = font.outline_glyphs().get(GlyphId::new(gid))?;
        let mut pen = Pen(BezPath::new());
        g.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::from(&loc)), &mut pen).ok()?;
        Some(Arc::new(pen.0))
    });
    let mut m = c.lock().unwrap_or_else(|e| e.into_inner());
    if m.len() > 20_000 {
        m.clear();
    }
    m.insert(key, v.clone());
    v
}

/// A laid-out character's outline (same placement as [`CharGlyph::path`]) with its face's axes
/// moved by `deltas` (tag, user-unit offset from the axis default).
pub fn char_outline_varied(g: &CharGlyph, deltas: &[(String, f32)]) -> Option<BezPath> {
    let axes = font_axes(g.face);
    let coords: Vec<(String, f32)> =
        deltas.iter().filter_map(|(t, d)| axes.iter().find(|a| a.tag == *t).map(|a| (t.clone(), (a.default + d).clamp(a.min, a.max)))).collect();
    if coords.is_empty() {
        return None;
    }
    let units = outline_units_at(g.face, g.gid, &coords)?;
    Some(g.outline_xf * (*units).clone())
}

/// The first installed face with variation axes (system fonts are scanned), for tests and
/// demos: (face, its axes).
pub fn find_variable_face() -> Option<(FaceId, Vec<FontAxis>)> {
    fonts::scan_system();
    (0..fonts::all_faces().len()).map(|f| (f, font_axes(f))).find(|(_, a)| a.iter().any(|x| x.max > x.min))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    #[test]
    fn static_fonts_have_no_axes_and_variable_outlines_move() {
        let inter = crate::resolve("Inter", "Regular").face;
        assert!(font_axes(inter).is_empty());
        // A variable system font, when one is installed (macOS ships several): its outline
        // changes along an axis. Skipped on machines without one.
        let Some((face, axes)) = find_variable_face() else {
            eprintln!("no variable font installed: outline check skipped");
            return;
        };
        let a = axes.iter().find(|x| x.max > x.min).unwrap();
        let gid = fonts::face(face).glyph('H').or_else(|| fonts::face(face).glyph('A')).unwrap_or(1);
        let lo = outline_units_at(face, gid, &[(a.tag.clone(), a.min)]).unwrap();
        let hi = outline_units_at(face, gid, &[(a.tag.clone(), a.max)]).unwrap();
        assert_ne!(lo.area(), hi.area(), "{} {}..{}", a.tag, a.min, a.max);
        // The default location is the static outline.
        let d = outline_units_at(face, gid, &[(a.tag.clone(), a.default)]).unwrap();
        let s = crate::outline_units(face, gid).unwrap();
        assert!((d.area() - s.area()).abs() < 1e-3);
    }
}
