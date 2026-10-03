//! The public effects catalogue (`docs/effects.md`), generated from the registry.
//!
//! Regenerate with `UPDATE_DOCS=1 cargo test -p effectcraft-effects --lib effects_doc_is_current`.

use crate::{CATEGORIES, registry};

/// Effects whose implementation is known to fall short of the reference behaviour:
/// `(effect id, what is missing)`. Everything else in the registry is implemented in full as far
/// as the public behaviour documentation describes it.
pub const PARTIAL: &[(&str, &str)] = &[];

/// Whether Effect Controls shows parameter (or twirl-down group) `param` (spec id path,
/// `group/param`) of an instance of effect `effect`, given the instance's current values
/// (`value(spec id)`). After Effects shows only the controls a popup selects: Levels' channel,
/// Hue/Saturation's Channel Control, the camera system of the card effects, and so on. Every
/// parameter stays animatable and addressable; this is presentation only.
pub fn param_shown(effect: &str, param: &str, value: &dyn Fn(&str) -> Option<effectcraft_keyframe::Value>) -> bool {
    let e = |id: &str| value(id).map(|v| v.as_enum()).unwrap_or(0);
    let b = |id: &str| value(id).is_some_and(|v| v.as_bool());
    match effect {
        "ec.color.levels" => {
            let ch = e("channel") as usize;
            for (i, pre) in crate::color_fx::LEVELS_CHANNEL_PREFIX.iter().enumerate() {
                if param.starts_with(pre) {
                    return ch == i + 1;
                }
            }
            let master = ["inBlack", "inWhite", "gamma", "outBlack", "outWhite", "clipToOutputBlack", "clipToOutputWhite"];
            !master.contains(&param) || ch == 0
        }
        "ec.color.huesaturation" => {
            let ch = e("channelControl") as usize;
            for (i, (pre, _, _)) in crate::color_fx::HUESAT_RANGES.iter().enumerate() {
                if param.starts_with(pre) {
                    return ch == i + 1;
                }
            }
            match param {
                "hue" | "saturation" | "lightness" => ch == 0,
                "colorizeSaturation" | "colorizeLightness" => b("colorize"),
                _ => true,
            }
        }
        "ec.color.selectivecolor" => match param.strip_prefix("details/") {
            Some(rest) => {
                let g = rest.split('/').next().unwrap_or("");
                crate::color2::SC_GROUPS.iter().position(|x| *x == g).is_none_or(|i| i == e("colors") as usize)
            }
            None => true,
        },
        "ec.sim.carddance" | "ec.transition.cardwipe" => crate::card3d::shown(param, &e).unwrap_or(true),
        "ec.sim.shatter" => match param {
            "shape/customShatterMap" | "shape/whiteTilesFixed" => e("shape/pattern") == 5,
            _ => crate::card3d::shown(param, &e).unwrap_or(true),
        },
        _ => {
            let _ = b("");
            true
        }
    }
}

/// Implementation status of effect `id` (`Implemented` or `Partial: …`).
pub fn status(id: &str) -> String {
    match PARTIAL.iter().find(|(e, _)| *e == id) {
        Some((_, note)) => format!("Partial: {note}"),
        None => "Implemented".to_string(),
    }
}

/// `docs/effects.md`.
pub fn effects_markdown() -> String {
    let r = registry();
    let mut s = String::new();
    s.push_str("# Effects\n\n");
    s.push_str("<!-- Generated from the effect registry (crates/effects/src/catalog.rs); do not edit by hand.\n");
    s.push_str("     Regenerate: UPDATE_DOCS=1 cargo test -p effectcraft-effects --lib effects_doc_is_current -->\n\n");
    let params: usize = r.iter().map(|e| e.params.len()).sum();
    let gpu = r.iter().filter(|e| e.gpu).count();
    let float = r.iter().filter(|e| e.float).count();
    let partial = r.iter().filter(|e| PARTIAL.iter().any(|(id, _)| *id == e.id)).count();
    s.push_str(&format!(
        "EffectCraft ships {} effects with {} parameters in total, grouped into the same categories as the Effects & Presets \
         panel. Every effect is our own implementation, written from public behaviour descriptions and standard \
         image-processing literature. Parameter names, order, popup options, units, defaults and ranges follow the \
         reference application so that projects, expressions and muscle memory carry over.\n\n",
        r.len(),
        params
    ));
    s.push_str(&format!(
        "- **GPU**: {gpu} effects also run on the GPU compositor with identical results.\n\
         - **32**: {float} effects process 32-bit float (HDR, overbright) pixels without clamping.\n\
         - **Status**: {} implemented in full, {partial} partial (what is missing is listed).\n\n",
        r.len() - partial
    ));
    s.push_str("Effects are addressed by id (`ec.<category>.<name>`) or display name in commands, scripts and the control channel, and every parameter by its id (`effects/#1/blurriness`).\n\n");
    for cat in CATEGORIES {
        let list: Vec<_> = r.iter().filter(|e| e.category == *cat).collect();
        if list.is_empty() {
            continue;
        }
        s.push_str(&format!("## {cat}\n\n| Effect | Id | Params | GPU | 32 | Status |\n|---|---|---:|:-:|:-:|---|\n"));
        for e in list {
            s.push_str(&format!(
                "| {} | `{}` | {} | {} | {} | {} |\n",
                e.name.replace('|', "\\|"),
                e.id,
                e.params.iter().filter(|p| !matches!(p.ui, effectcraft_project::ParamUi::Hidden)).count(),
                if e.gpu { "GPU" } else { "" },
                if e.float { "32" } else { "" },
                status(e.id).replace('|', "\\|"),
            ));
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn effects_doc_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/effects.md");
        let want = super::effects_markdown();
        if std::env::var_os("UPDATE_DOCS").is_some() {
            std::fs::write(path, &want).unwrap();
            return;
        }
        let have = std::fs::read_to_string(path).unwrap_or_default();
        assert!(have == want, "docs/effects.md is stale: UPDATE_DOCS=1 cargo test -p effectcraft-effects --lib effects_doc_is_current");
    }

    #[test]
    fn popups_choose_the_shown_controls() {
        use effectcraft_keyframe::Value;
        let with = |pairs: &'static [(&'static str, u32)]| move |id: &str| pairs.iter().find(|(k, _)| *k == id).map(|(_, v)| Value::Enum(*v));
        let shown = |e: &str, p: &str, pairs: &'static [(&'static str, u32)]| super::param_shown(e, p, &with(pairs));
        // Levels: RGB shows the master controls only; Red shows the red ones.
        assert!(shown("ec.color.levels", "inBlack", &[]));
        assert!(!shown("ec.color.levels", "redInBlack", &[]));
        assert!(shown("ec.color.levels", "redInBlack", &[("channel", 1)]));
        assert!(!shown("ec.color.levels", "inBlack", &[("channel", 1)]));
        assert!(shown("ec.color.levels", "channel", &[("channel", 1)]));
        // Hue/Saturation: Master or one colour range.
        assert!(shown("ec.color.huesaturation", "hue", &[]));
        assert!(shown("ec.color.huesaturation", "bluesHue", &[("channelControl", 5)]));
        assert!(!shown("ec.color.huesaturation", "redsHue", &[("channelControl", 5)]));
        // Card effects: the camera system's group.
        assert!(shown("ec.sim.carddance", "cameraPosition", &[]));
        assert!(!shown("ec.sim.carddance", "cornerPins", &[]));
        assert!(shown("ec.transition.cardwipe", "cornerPins/upperLeftCorner", &[("cameraSystem", 1)]));
        assert!(!shown("ec.sim.shatter", "shape/customShatterMap", &[]));
        // Selective Color: the chosen range's Details group.
        assert!(shown("ec.color.selectivecolor", "details/reds", &[]));
        assert!(!shown("ec.color.selectivecolor", "details/blues/bluesCyan", &[]));
        assert!(shown("ec.color.selectivecolor", "details", &[]));
        // Everything else is always shown.
        assert!(shown("ec.blur.gaussian", "blurriness", &[]));
    }

    #[test]
    fn partial_notes_name_registered_effects() {
        for (id, note) in super::PARTIAL {
            assert!(crate::find(id).is_some(), "PARTIAL names unknown effect {id}");
            assert!(!note.trim().is_empty());
        }
    }
}
