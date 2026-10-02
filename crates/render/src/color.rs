//! Project colour settings in the compositor: bit depth, working space, linear blending.
//!
//! * **Bit depth.** Pixels are `f32` throughout, but 8 and 16 bpc projects behave like integer
//!   pipelines: each layer's pixels are clamped to 0..1 and quantised to the depth after its
//!   source and masks and after every effect, and the comp is clamped and quantised after every
//!   layer is blended. Over-range values (Add/Screen of bright layers, Exposure…) therefore only
//!   survive in 32 bpc. 16 bpc uses After Effects' 0..32768 range.
//! * **Working space** (colour management). Footage is converted from its colour profile (its
//!   metadata, else sRGB) into the working space; the top-level comp output is converted from
//!   the working space to the sRGB display. Without a working space nothing is converted.
//! * **Linearize Working Space**: the working space is linear light, so sources (footage and
//!   the colours of solids, text and shapes, which are authored in the working space's encoding)
//!   are linearised before masks and effects, and everything runs in linear.
//! * **Blend Colors Using 1.0 Gamma**: only blending is linear. Each layer's finished pixels
//!   are linearised just before they are transformed and blended, the comp accumulates in linear
//!   and is encoded back to the working space when the comp is done (so precomps hand encoded
//!   pixels to their parent). Without a working space the sRGB curve is used.

use effectcraft_color::{ColorSpace, Conversion};
use effectcraft_project::ProjectSettings;
use effectcraft_raster::Image;
use rayon::prelude::*;

/// The colour pipeline of a project (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pipe {
    /// Quantisation levels (255, 32768) or `None` for 32 bpc float.
    pub levels: Option<f32>,
    /// Working space (`None` = unmanaged).
    pub space: Option<ColorSpace>,
    /// The working space is linear.
    pub linear: bool,
    /// Blend in linear light (and the working space is not already linear).
    pub linear_blend: bool,
}

impl Pipe {
    pub fn of(s: &ProjectSettings) -> Pipe {
        let linear = s.linearize && s.working_space.is_some();
        Pipe { levels: s.bit_depth.levels(), space: s.working_space, linear, linear_blend: s.blend_linear && !linear }
    }

    /// The space whose curve encodes working-space pixels (sRGB when unmanaged).
    fn curve(&self) -> ColorSpace {
        self.space.unwrap_or(ColorSpace::Srgb)
    }

    /// Footage with colour profile `profile` (`None` = sRGB) → working space.
    pub fn media_in(&self, profile: Option<ColorSpace>) -> Option<Conversion> {
        let ws = self.space?;
        Conversion::new(profile.unwrap_or(ColorSpace::Srgb), false, ws, self.linear)
    }

    /// Authored colours (solids, text, shapes) → working space: linearised in a linear working
    /// space.
    pub fn authored_in(&self) -> Option<Conversion> {
        self.linear.then(|| Conversion::linearize(self.curve()))
    }

    /// Working space → blending space (linear when blending with 1.0 gamma).
    pub fn to_blend(&self) -> Option<Conversion> {
        self.linear_blend.then(|| Conversion::linearize(self.curve()))
    }

    /// Blending space → working space.
    pub fn from_blend(&self) -> Option<Conversion> {
        self.linear_blend.then(|| Conversion::delinearize(self.curve()))
    }

    /// Working space → sRGB display (top-level comp output).
    pub fn output(&self) -> Option<Conversion> {
        let ws = self.space?;
        Conversion::new(ws, self.linear, ColorSpace::Srgb, false)
    }

    /// Clamp and quantise to the bit depth (no-op in 32 bpc).
    pub fn quantize(&self, img: &mut Image) {
        if let Some(l) = self.levels {
            quantize(img, l);
        }
    }

    /// Hash of everything that changes pixels (for cache keys).
    pub fn key(&self) -> u64 {
        let mut k = self.levels.map_or(0, |l| l as u64);
        k = k.wrapping_mul(31).wrapping_add(self.space.map_or(7, |s| s as u64 + 11));
        k = k.wrapping_mul(31).wrapping_add(self.linear as u64 * 2 + self.linear_blend as u64);
        k
    }
}

/// Clamp premultiplied pixels to 0..1 (colour ≤ alpha) and round to `levels` steps.
pub fn quantize(img: &mut Image, levels: f32) {
    let inv = 1.0 / levels;
    img.data.par_iter_mut().for_each(|p| {
        let a = (p[3].clamp(0.0, 1.0) * levels).round() * inv;
        p[3] = a;
        for c in 0..3 {
            p[c] = (p[c].clamp(0.0, a) * levels).round() * inv;
        }
    });
}

/// Convert premultiplied pixels (on straight colour).
pub fn convert(img: &mut Image, c: &Conversion) {
    img.data.par_iter_mut().for_each(|p| {
        let a = p[3];
        if a <= 0.0 {
            return;
        }
        let o = c.apply([p[0] / a, p[1] / a, p[2] / a]);
        *p = [o[0] * a, o[1] * a, o[2] * a, a];
    });
}
