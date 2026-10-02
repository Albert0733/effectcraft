//! RGB colour spaces for colour management: the project working space, footage interpretation
//! and the display.
//!
//! Each space is defined by its published primaries and white point and its transfer function:
//!
//! * **sRGB IEC 61966-2-1** — Rec. 709 primaries, D65, the piecewise sRGB curve.
//! * **Rec. 709** — ITU-R BT.709-6 primaries, D65, the BT.1886 display EOTF (gamma 2.4).
//! * **Rec. 2020** — ITU-R BT.2020-2 primaries, D65, BT.1886 (gamma 2.4).
//! * **Display P3** — SMPTE EG 432-1 (DCI-P3) primaries with D65 and the sRGB curve.
//!
//! RGB → XYZ matrices are derived from the primaries (the standard construction from chromaticity
//! coordinates: columns of xyY → XYZ scaled so that RGB (1, 1, 1) maps to the white point), so
//! conversions between any two spaces are exact up to `f64` rounding. Transfer functions extend
//! to over-range and negative values (32 bpc) by mirroring around zero.

use serde::{Deserialize, Serialize};

/// An RGB colour space (all D65).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorSpace {
    /// sRGB IEC 61966-2-1.
    Srgb,
    /// HDTV Rec. 709 (gamma 2.4).
    Rec709,
    /// UHDTV Rec. 2020 (gamma 2.4).
    Rec2020,
    /// Display P3.
    DisplayP3,
}

/// CIE xy chromaticity of D65.
const D65: [f64; 2] = [0.3127, 0.3290];

impl ColorSpace {
    pub const ALL: [ColorSpace; 4] = [ColorSpace::Srgb, ColorSpace::Rec709, ColorSpace::Rec2020, ColorSpace::DisplayP3];

    /// Display name (as in Project Settings ▸ Color ▸ Working Space).
    pub fn label(self) -> &'static str {
        match self {
            ColorSpace::Srgb => "sRGB IEC61966-2.1",
            ColorSpace::Rec709 => "HDTV (Rec. 709)",
            ColorSpace::Rec2020 => "Rec. 2020",
            ColorSpace::DisplayP3 => "Display P3",
        }
    }

    /// Short identifier used by commands and files (`srgb`, `rec709`, `rec2020`, `p3`).
    pub fn id(self) -> &'static str {
        match self {
            ColorSpace::Srgb => "srgb",
            ColorSpace::Rec709 => "rec709",
            ColorSpace::Rec2020 => "rec2020",
            ColorSpace::DisplayP3 => "p3",
        }
    }

    /// Parse an id or label (case-insensitive, loose).
    pub fn parse(s: &str) -> Option<ColorSpace> {
        let k: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        match k.as_str() {
            "srgb" | "srgbiec6196621" | "srgbiec61966" | "iec6196621" => Some(ColorSpace::Srgb),
            "rec709" | "bt709" | "hdtvrec709" | "hdtv" | "709" => Some(ColorSpace::Rec709),
            "rec2020" | "bt2020" | "2020" | "uhdtv" => Some(ColorSpace::Rec2020),
            "p3" | "displayp3" | "p3d65" | "dcip3d65" => Some(ColorSpace::DisplayP3),
            _ => None,
        }
    }

    /// Red, green and blue primaries (CIE xy).
    pub fn primaries(self) -> [[f64; 2]; 3] {
        match self {
            ColorSpace::Srgb | ColorSpace::Rec709 => [[0.640, 0.330], [0.300, 0.600], [0.150, 0.060]],
            ColorSpace::Rec2020 => [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]],
            ColorSpace::DisplayP3 => [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]],
        }
    }

    /// Linear RGB → CIE XYZ (Y of white = 1).
    pub fn to_xyz(self) -> [[f64; 3]; 3] {
        rgb_to_xyz(self.primaries(), D65)
    }

    /// Encoded value → linear light.
    pub fn decode(self, v: f32) -> f32 {
        let a = v.abs();
        let l = match self {
            ColorSpace::Srgb | ColorSpace::DisplayP3 => crate::srgb_to_linear(a),
            ColorSpace::Rec709 | ColorSpace::Rec2020 => a.powf(2.4),
        };
        l.copysign(v)
    }

    /// Linear light → encoded value.
    pub fn encode(self, v: f32) -> f32 {
        let a = v.abs();
        let e = match self {
            ColorSpace::Srgb | ColorSpace::DisplayP3 => crate::linear_to_srgb(a),
            ColorSpace::Rec709 | ColorSpace::Rec2020 => a.powf(1.0 / 2.4),
        };
        e.copysign(v)
    }
}

/// The standard RGB → XYZ matrix from primaries and white point.
pub fn rgb_to_xyz(p: [[f64; 2]; 3], white: [f64; 2]) -> [[f64; 3]; 3] {
    let xyz = |c: [f64; 2]| [c[0] / c[1], 1.0, (1.0 - c[0] - c[1]) / c[1]];
    let (r, g, b) = (xyz(p[0]), xyz(p[1]), xyz(p[2]));
    let m = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let w = xyz(white);
    let s = mul_vec(&invert(&m), w);
    let mut o = m;
    for row in &mut o {
        for j in 0..3 {
            row[j] *= s[j];
        }
    }
    o
}

/// 3×3 matrix inverse (the matrices here are always well conditioned).
pub fn invert(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let det = m[0][0] * c(1, 1, 2, 2) - m[0][1] * c(1, 0, 2, 2) + m[0][2] * c(1, 0, 2, 1);
    let k = 1.0 / det;
    [
        [c(1, 1, 2, 2) * k, -c(0, 1, 2, 2) * k, c(0, 1, 1, 2) * k],
        [-c(1, 0, 2, 2) * k, c(0, 0, 2, 2) * k, -c(0, 0, 1, 2) * k],
        [c(1, 0, 2, 1) * k, -c(0, 0, 2, 1) * k, c(0, 0, 1, 1) * k],
    ]
}

pub fn mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

pub fn mul_vec(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// A colour conversion between two spaces, each either encoded (with its transfer curve) or
/// linear. Applied to straight (un-premultiplied) RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conversion {
    /// Decode the input with this space's curve first.
    pub decode: Option<ColorSpace>,
    /// Linear RGB matrix (None = identity primaries).
    pub matrix: Option<[[f32; 3]; 3]>,
    /// Encode the output with this space's curve last.
    pub encode: Option<ColorSpace>,
}

impl Conversion {
    /// From `from` (linear when `from_linear`) to `to` (linear when `to_linear`). `None` when
    /// the conversion is the identity.
    pub fn new(from: ColorSpace, from_linear: bool, to: ColorSpace, to_linear: bool) -> Option<Conversion> {
        let same_primaries = from.primaries() == to.primaries();
        let same_curve = from_linear == to_linear && (from_linear || curve_eq(from, to));
        if same_primaries && same_curve {
            return None;
        }
        let matrix = (!same_primaries).then(|| {
            let m = mul(&invert(&to.to_xyz()), &from.to_xyz());
            m.map(|r| r.map(|v| v as f32))
        });
        // Same primaries, both encoded with different curves (sRGB ↔ Rec. 709), or a matrix in
        // between: go through linear.
        let decode = (!from_linear).then_some(from);
        let encode = (!to_linear).then_some(to);
        Some(Conversion { decode, matrix, encode })
    }

    /// Decode only (encoded → linear in the same space).
    pub fn linearize(space: ColorSpace) -> Conversion {
        Conversion { decode: Some(space), matrix: None, encode: None }
    }

    /// Encode only (linear → encoded in the same space).
    pub fn delinearize(space: ColorSpace) -> Conversion {
        Conversion { decode: None, matrix: None, encode: Some(space) }
    }

    #[inline]
    pub fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let mut c = c;
        if let Some(s) = self.decode {
            c = c.map(|v| s.decode(v));
        }
        if let Some(m) = &self.matrix {
            c = [0, 1, 2].map(|i| m[i][0] * c[0] + m[i][1] * c[1] + m[i][2] * c[2]);
        }
        if let Some(s) = self.encode {
            c = c.map(|v| s.encode(v));
        }
        c
    }
}

fn curve_eq(a: ColorSpace, b: ColorSpace) -> bool {
    let srgb = |s: ColorSpace| matches!(s, ColorSpace::Srgb | ColorSpace::DisplayP3);
    srgb(a) == srgb(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_matrix_matches_iec() {
        // IEC 61966-2-1 (rounded to 4 places).
        let m = ColorSpace::Srgb.to_xyz();
        let want = [[0.4124, 0.3576, 0.1805], [0.2126, 0.7152, 0.0722], [0.0193, 0.1192, 0.9505]];
        for i in 0..3 {
            for j in 0..3 {
                assert!((m[i][j] - want[i][j]).abs() < 2e-4, "{i}{j}: {} vs {}", m[i][j], want[i][j]);
            }
        }
        // Rec. 2020 luminance row (BT.2020: 0.2627, 0.6780, 0.0593).
        let y = ColorSpace::Rec2020.to_xyz()[1];
        assert!((y[0] - 0.2627).abs() < 1e-4 && (y[1] - 0.6780).abs() < 1e-4 && (y[2] - 0.0593).abs() < 1e-4, "{y:?}");
    }

    #[test]
    fn round_trips() {
        let samples = [[0.2f32, 0.5, 0.8], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.03, 0.9, 0.4], [1.5, 0.2, 2.0]];
        for a in ColorSpace::ALL {
            for b in ColorSpace::ALL {
                for (al, bl) in [(false, false), (true, false), (false, true), (true, true)] {
                    let fwd = Conversion::new(a, al, b, bl);
                    let back = Conversion::new(b, bl, a, al);
                    assert_eq!(fwd.is_none(), back.is_none());
                    for c in samples {
                        let mid = fwd.map(|f| f.apply(c)).unwrap_or(c);
                        let o = back.map(|f| f.apply(mid)).unwrap_or(mid);
                        for i in 0..3 {
                            // Pure power curves amplify f32 rounding near zero.
                            assert!((o[i] - c[i]).abs() < 2e-3, "{a:?}{al}→{b:?}{bl}: {c:?} → {mid:?} → {o:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn white_is_preserved_and_gamut_grows() {
        let c = Conversion::new(ColorSpace::Srgb, false, ColorSpace::Rec2020, false).unwrap();
        let w = c.apply([1.0, 1.0, 1.0]);
        assert!(w.iter().all(|v| (v - 1.0).abs() < 1e-4), "{w:?}");
        // Pure sRGB red sits inside Rec. 2020: less saturated there.
        let r = c.apply([1.0, 0.0, 0.0]);
        assert!(r[0] < 1.0 && r[1] > 0.0 && r[2] > 0.0, "{r:?}");
        // sRGB ↔ Rec. 709 share primaries: only the curve changes.
        let k = Conversion::new(ColorSpace::Srgb, false, ColorSpace::Rec709, false).unwrap();
        assert!(k.matrix.is_none());
        assert!(Conversion::new(ColorSpace::Srgb, true, ColorSpace::Rec709, true).is_none());
    }

    #[test]
    fn transfer_extends_over_range() {
        let s = ColorSpace::Srgb;
        assert!((s.decode(s.encode(4.0)) - 4.0).abs() < 1e-4);
        assert!((s.decode(s.encode(-0.25)) + 0.25).abs() < 1e-5);
        assert!((ColorSpace::Rec709.decode(0.5) - 0.5f32.powf(2.4)).abs() < 1e-6);
        assert_eq!(ColorSpace::parse("sRGB IEC61966-2.1"), Some(ColorSpace::Srgb));
        assert_eq!(ColorSpace::parse("Display P3"), Some(ColorSpace::DisplayP3));
    }
}
