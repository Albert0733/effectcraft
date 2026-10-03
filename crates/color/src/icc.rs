//! A reader for matrix/TRC RGB ICC profiles (ICC.1:2001-04 v2 and ICC.1:2010 v4, the public
//! International Color Consortium specification): the red/green/blue colorants (`rXYZ`, `gXYZ`,
//! `bXYZ`), the media white point (`wtpt`), the chromatic adaptation (`chad`), the tone curves
//! (`rTRC`, `gTRC`, `bTRC` as `curv` or `para`) and the description (`desc` / `mluc`).
//!
//! Used by View ▸ Simulate Output ▸ My Custom RGB: the profile becomes primaries, a white point
//! and a curve. LUT-based (`A2B0`) and non-RGB profiles are rejected.

use crate::space::{bradford, invert, mul_vec};

/// CIE D50 (the ICC profile connection space white), xy.
pub const D50: [f64; 2] = [0.3457, 0.3585];

/// A tone curve of a profile.
#[derive(Clone, Debug, PartialEq)]
pub enum Trc {
    /// `curv` with one entry, or `para` function type 0: y = x^g.
    Gamma(f64),
    /// `curv` table (0..=65535 samples over 0..1).
    Table(Vec<u16>),
    /// `para` function types 1–4: (type, parameters g a b c d e f).
    Parametric(u16, [f64; 7]),
}

impl Trc {
    /// Encoded value (0..1) → linear.
    pub fn eval(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Trc::Gamma(g) => x.powf(*g),
            Trc::Table(t) if t.is_empty() => x,
            Trc::Table(t) => {
                let f = x * (t.len() - 1) as f64;
                let i = (f.floor() as usize).min(t.len() - 1);
                let j = (i + 1).min(t.len() - 1);
                let k = f - i as f64;
                (t[i] as f64 * (1.0 - k) + t[j] as f64 * k) / 65535.0
            }
            Trc::Parametric(ty, p) => {
                let [g, a, b, c, d, e, f] = *p;
                let pw = |v: f64| if v > 0.0 { v.powf(g) } else { 0.0 };
                match ty {
                    1 => {
                        if x >= -b / a {
                            pw(a * x + b)
                        } else {
                            0.0
                        }
                    }
                    2 => {
                        if x >= -b / a {
                            pw(a * x + b) + c
                        } else {
                            c
                        }
                    }
                    3 => {
                        if x >= d {
                            pw(a * x + b)
                        } else {
                            c * x
                        }
                    }
                    _ => {
                        if x >= d {
                            pw(a * x + b) + e
                        } else {
                            c * x + f
                        }
                    }
                }
            }
        }
    }

    /// The single gamma that best matches the curve (least squares in log space over mid-tones).
    pub fn fit_gamma(&self) -> f64 {
        if let Trc::Gamma(g) = self {
            return *g;
        }
        let (mut num, mut den) = (0.0, 0.0);
        for i in 4..20 {
            let x = i as f64 / 20.0;
            let y = self.eval(x);
            if y > 1e-6 && y < 1.0 {
                num += x.ln() * y.ln();
                den += x.ln() * x.ln();
            }
        }
        if den > 0.0 { (num / den).clamp(0.1, 10.0) } else { 1.0 }
    }

    /// Whether the curve is the sRGB (IEC 61966-2-1) piecewise curve, within 8-bit precision.
    pub fn is_srgb(&self) -> bool {
        let srgb = |x: f64| if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
        !matches!(self, Trc::Gamma(_)) && (0..=32).all(|i| (self.eval(i as f64 / 32.0) - srgb(i as f64 / 32.0)).abs() < 0.004)
    }
}

/// What a matrix/TRC RGB profile says.
#[derive(Clone, Debug, PartialEq)]
pub struct RgbProfile {
    pub description: String,
    /// Red, green, blue chromaticities (CIE xy) under the profile's own white.
    pub primaries: [[f64; 2]; 3],
    /// The white point (CIE xy) of the device.
    pub white: [f64; 2],
    /// Tone curves (red, green, blue).
    pub trc: [Trc; 3],
    /// ICC version major number.
    pub version: u8,
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn be16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn s15f16(b: &[u8], o: usize) -> Option<f64> {
    Some(be32(b, o)? as i32 as f64 / 65536.0)
}

fn xy(v: [f64; 3]) -> [f64; 2] {
    let s = v[0] + v[1] + v[2];
    if s.abs() < 1e-12 { D50 } else { [v[0] / s, v[1] / s] }
}

fn xyz_of(c: [f64; 2]) -> [f64; 3] {
    [c[0] / c[1], 1.0, (1.0 - c[0] - c[1]) / c[1]]
}

/// Parse an RGB matrix/TRC ICC profile.
pub fn parse(b: &[u8]) -> Result<RgbProfile, String> {
    if b.len() < 132 || b.get(36..40) != Some(b"acsp") {
        return Err("not an ICC profile".into());
    }
    if b.get(16..20) != Some(b"RGB ") {
        return Err("not an RGB profile (only RGB display profiles can be simulated)".into());
    }
    let version = b[8];
    let n = be32(b, 128).ok_or("truncated tag table")? as usize;
    let mut tags = std::collections::HashMap::new();
    for i in 0..n.min(200) {
        let o = 132 + i * 12;
        let sig = b.get(o..o + 4).ok_or("truncated tag table")?;
        let off = be32(b, o + 4).ok_or("truncated tag table")? as usize;
        let size = be32(b, o + 8).ok_or("truncated tag table")? as usize;
        if off.checked_add(size).is_some_and(|e| e <= b.len()) {
            tags.insert(String::from_utf8_lossy(sig).into_owned(), &b[off..off + size]);
        }
    }
    let xyz_tag = |name: &str| -> Result<[f64; 3], String> {
        let t = tags.get(name).ok_or_else(|| format!("the profile has no {name} tag (only matrix/TRC profiles are supported)"))?;
        if t.get(0..4) != Some(b"XYZ ") {
            return Err(format!("{name}: not an XYZ tag"));
        }
        Ok([s15f16(t, 8).ok_or("bad XYZ")?, s15f16(t, 12).ok_or("bad XYZ")?, s15f16(t, 16).ok_or("bad XYZ")?])
    };
    let trc_tag = |name: &str| -> Result<Trc, String> {
        let t = tags.get(name).ok_or_else(|| format!("the profile has no {name} tag"))?;
        match t.get(0..4) {
            Some(b"curv") => {
                let count = be32(t, 8).ok_or("bad curv")? as usize;
                match count {
                    0 => Ok(Trc::Gamma(1.0)),
                    1 => Ok(Trc::Gamma(be16(t, 12).ok_or("bad curv")? as f64 / 256.0)),
                    _ => Ok(Trc::Table((0..count).map(|i| be16(t, 12 + i * 2)).collect::<Option<Vec<_>>>().ok_or("truncated curv")?)),
                }
            }
            Some(b"para") => {
                let ty = be16(t, 8).ok_or("bad para")?;
                let np = [1, 3, 4, 5, 7].get(ty as usize).copied().ok_or("unknown para function")?;
                let mut p = [0.0; 7];
                for (k, v) in p.iter_mut().enumerate().take(np) {
                    *v = s15f16(t, 12 + k * 4).ok_or("truncated para")?;
                }
                Ok(if ty == 0 { Trc::Gamma(p[0]) } else { Trc::Parametric(ty, p) })
            }
            _ => Err(format!("{name}: unsupported curve type")),
        }
    };
    let col = [xyz_tag("rXYZ")?, xyz_tag("gXYZ")?, xyz_tag("bXYZ")?];
    // The device white: from the chromatic adaptation (v4, and many v2 profiles), else the media
    // white point (v2 profiles that keep it un-adapted), else D50.
    let d50 = xyz_of(D50);
    let chad = tags.get("chad").filter(|t| t.get(0..4) == Some(b"sf32") && t.len() >= 44).map(|t| {
        let v: Vec<f64> = (0..9).map(|i| s15f16(t, 8 + i * 4).unwrap_or(0.0)).collect();
        [[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]]
    });
    let white = match (chad, xyz_tag("wtpt").ok()) {
        (Some(m), _) => xy(mul_vec(&invert(&m), d50)),
        (None, Some(w)) => xy(w),
        _ => D50,
    };
    // Colorants are adapted to D50 in the profile: undo that to get the device primaries.
    let undo = match chad {
        Some(m) => invert(&m),
        None => bradford(D50, white),
    };
    let primaries = col.map(|c| xy(mul_vec(&undo, c)));
    let trc = [trc_tag("rTRC")?, trc_tag("gTRC")?, trc_tag("bTRC")?];
    Ok(RgbProfile { description: description(tags.get("desc").copied()).unwrap_or_default(), primaries, white, trc, version })
}

fn description(t: Option<&[u8]>) -> Option<String> {
    let t = t?;
    match t.get(0..4)? {
        b"desc" => {
            let n = be32(t, 8)? as usize;
            let s = t.get(12..12 + n)?;
            Some(String::from_utf8_lossy(s).trim_end_matches('\0').to_string())
        }
        b"mluc" => {
            // The first record: UTF-16BE.
            let len = be32(t, 20)? as usize;
            let off = be32(t, 24)? as usize;
            let u: Vec<u16> = t.get(off..off + len)?.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            Some(String::from_utf16_lossy(&u).trim_end_matches('\0').to_string())
        }
        _ => None,
    }
}

/// Build a minimal v2 matrix/TRC RGB profile (tests, and saving a custom simulation): colorants
/// Bradford-adapted to D50, `wtpt` the device white, gamma curves.
pub fn write_matrix_profile(desc: &str, primaries: [[f64; 2]; 3], white: [f64; 2], gamma: f64) -> Vec<u8> {
    let m = crate::space::rgb_to_xyz(primaries, white);
    let a = bradford(white, D50);
    let col = |j: usize| mul_vec(&a, [m[0][j], m[1][j], m[2][j]]);
    let f = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
    let xyz = |v: [f64; 3]| {
        let mut t = b"XYZ \0\0\0\0".to_vec();
        for c in v {
            t.extend(f(c));
        }
        t
    };
    let curv = {
        let mut t = b"curv\0\0\0\0".to_vec();
        t.extend(1u32.to_be_bytes());
        t.extend(((gamma * 256.0).round() as u16).to_be_bytes());
        t.extend([0, 0]);
        t
    };
    let dsc = {
        let mut t = b"desc\0\0\0\0".to_vec();
        t.extend((desc.len() as u32 + 1).to_be_bytes());
        t.extend(desc.as_bytes());
        t.push(0);
        t.extend([0u8; 12 + 67]);
        while !t.len().is_multiple_of(4) {
            t.push(0);
        }
        t
    };
    let w = xyz_of(white);
    let tags: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"desc", dsc),
        (b"wtpt", xyz(w)),
        (b"rXYZ", xyz(col(0))),
        (b"gXYZ", xyz(col(1))),
        (b"bXYZ", xyz(col(2))),
        (b"rTRC", curv.clone()),
        (b"gTRC", curv.clone()),
        (b"bTRC", curv),
    ];
    let mut table = vec![];
    let mut data = vec![];
    let base = 128 + 4 + tags.len() * 12;
    for (sig, t) in &tags {
        table.extend(*sig);
        table.extend(((base + data.len()) as u32).to_be_bytes());
        table.extend((t.len() as u32).to_be_bytes());
        data.extend(t);
        while !data.len().is_multiple_of(4) {
            data.push(0);
        }
    }
    let total = base + data.len();
    let mut h = vec![0u8; 128];
    h[0..4].copy_from_slice(&(total as u32).to_be_bytes());
    h[8] = 2;
    h[9] = 0x10;
    h[12..16].copy_from_slice(b"mntr");
    h[16..20].copy_from_slice(b"RGB ");
    h[20..24].copy_from_slice(b"XYZ ");
    h[36..40].copy_from_slice(b"acsp");
    for (k, v) in xyz_of(D50).into_iter().enumerate() {
        h[68 + k * 4..72 + k * 4].copy_from_slice(&f(v));
    }
    let mut out = h;
    out.extend((tags.len() as u32).to_be_bytes());
    out.extend(table);
    out.extend(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const P709: [[f64; 2]; 3] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]];
    const D65: [f64; 2] = [0.3127, 0.3290];

    #[test]
    fn matrix_profile_round_trips() {
        let b = write_matrix_profile("Test RGB", P709, D65, 2.2);
        let p = parse(&b).unwrap();
        assert_eq!(p.description, "Test RGB");
        assert_eq!(p.version, 2);
        for (a, e) in p.primaries.iter().zip(P709) {
            assert!((a[0] - e[0]).abs() < 2e-4 && (a[1] - e[1]).abs() < 2e-4, "{a:?} vs {e:?}");
        }
        assert!((p.white[0] - D65[0]).abs() < 2e-4 && (p.white[1] - D65[1]).abs() < 2e-4, "{:?}", p.white);
        assert!((p.trc[0].fit_gamma() - 2.2).abs() < 0.01);
        assert!(!p.trc[0].is_srgb());
    }

    #[test]
    fn curves() {
        let srgb = Trc::Parametric(3, [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045, 0.0, 0.0]);
        assert!(srgb.is_srgb());
        assert!((srgb.fit_gamma() - 2.2).abs() < 0.15, "{}", srgb.fit_gamma());
        let table = Trc::Table((0..256).map(|i| ((i as f64 / 255.0).powf(1.8) * 65535.0).round() as u16).collect());
        assert!((table.fit_gamma() - 1.8).abs() < 0.02);
        assert!((Trc::Gamma(2.0).eval(0.5) - 0.25).abs() < 1e-12);
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse(b"hello").is_err());
        let mut b = write_matrix_profile("x", P709, D65, 2.2);
        b[16..20].copy_from_slice(b"CMYK");
        assert!(parse(&b).unwrap_err().contains("RGB"));
    }
}
