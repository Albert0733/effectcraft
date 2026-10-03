//! Colour spaces (ISO 32000-1 §8.6) reduced to sRGB, and functions (§7.10: sampled,
//! exponential and stitching; PostScript calculator functions are not evaluated).

use crate::object::{Dict, File, Obj, decode_stream};

#[derive(Clone, Debug, PartialEq)]
pub enum Cs {
    Gray,
    Rgb,
    Cmyk,
    /// CIE L*a*b* (approximated).
    Lab,
    Indexed {
        base: Box<Cs>,
        hival: usize,
        lookup: Vec<u8>,
    },
    /// Separation / DeviceN: components → alternate space through a tint transform.
    Tint {
        n: usize,
        alt: Box<Cs>,
        func: Func,
    },
    Pattern,
}

impl Cs {
    pub fn components(&self) -> usize {
        match self {
            Cs::Gray | Cs::Indexed { .. } => 1,
            Cs::Rgb | Cs::Lab => 3,
            Cs::Cmyk => 4,
            Cs::Tint { n, .. } => *n,
            Cs::Pattern => 0,
        }
    }

    /// The initial colour of the space (black, or tint 1).
    pub fn initial(&self) -> Vec<f64> {
        match self {
            Cs::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
            Cs::Tint { n, .. } => vec![1.0; *n],
            Cs::Lab => vec![0.0, 0.0, 0.0],
            c => vec![0.0; c.components()],
        }
    }

    pub fn to_rgb(&self, c: &[f64]) -> [f64; 3] {
        let g = |i: usize| c.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        match self {
            Cs::Gray => [g(0); 3],
            Cs::Rgb | Cs::Pattern => [g(0), g(1), g(2)],
            Cs::Cmyk => {
                let k = g(3);
                [(1.0 - g(0)) * (1.0 - k), (1.0 - g(1)) * (1.0 - k), (1.0 - g(2)) * (1.0 - k)]
            }
            Cs::Lab => lab_to_rgb(c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0), c.get(2).copied().unwrap_or(0.0)),
            Cs::Indexed { base, hival, lookup } => {
                let i = (c.first().copied().unwrap_or(0.0).round().max(0.0) as usize).min(*hival);
                let n = base.components();
                let v: Vec<f64> = (0..n).map(|k| lookup.get(i * n + k).copied().unwrap_or(0) as f64 / 255.0).collect();
                base.to_rgb(&v)
            }
            Cs::Tint { alt, func, .. } => alt.to_rgb(&func.eval(c)),
        }
    }
}

fn lab_to_rgb(l: f64, a: f64, b: f64) -> [f64; 3] {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let f = |t: f64| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f64 / 29.0).powi(2) * (t - 4.0 / 29.0) };
    let (x, y, z) = (0.9505 * f(fx), f(fy), 1.089 * f(fz));
    let lin = [3.2406 * x - 1.5372 * y - 0.4986 * z, -0.9689 * x + 1.8758 * y + 0.0415 * z, 0.0557 * x - 0.204 * y + 1.057 * z];
    lin.map(|v| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
    })
}

/// Read a colour space (a name, or an array such as `[/ICCBased 5 0 R]`); named resources are
/// looked up in `res` (`/ColorSpace`).
pub fn color_space(file: &File, o: &Obj, res: Option<&Dict>) -> Cs {
    let o = file.resolve(o);
    match o {
        Obj::Name(n) => match n.as_str() {
            "DeviceGray" | "G" | "CalGray" => Cs::Gray,
            "DeviceRGB" | "RGB" | "CalRGB" => Cs::Rgb,
            "DeviceCMYK" | "CMYK" => Cs::Cmyk,
            "Pattern" => Cs::Pattern,
            other => match res.and_then(|r| file.get_dict(r, "ColorSpace")).and_then(|cs| file.get(cs, other)) {
                Some(x) if !matches!(x, Obj::Name(m) if m == other) => color_space(file, x, None),
                _ => Cs::Gray,
            },
        },
        Obj::Array(a) => {
            let head = a.first().map(|h| file.resolve(h)).and_then(Obj::name).unwrap_or("");
            match head {
                "ICCBased" => {
                    let n = a.get(1).map(|s| file.resolve(s)).and_then(Obj::dict).and_then(|d| file.get_num(d, "N")).unwrap_or(3.0) as usize;
                    match n {
                        1 => Cs::Gray,
                        4 => Cs::Cmyk,
                        _ => Cs::Rgb,
                    }
                }
                "CalRGB" => Cs::Rgb,
                "CalGray" => Cs::Gray,
                "Lab" => Cs::Lab,
                "Pattern" => Cs::Pattern,
                "Indexed" | "I" => {
                    let base = a.get(1).map(|b| color_space(file, b, res)).unwrap_or(Cs::Rgb);
                    let hival = a.get(2).and_then(|h| file.resolve(h).num()).unwrap_or(0.0) as usize;
                    let lookup = match a.get(3).map(|l| file.resolve(l)) {
                        Some(Obj::Str(s)) => s.clone(),
                        Some(Obj::Stream(d, raw)) => decode_stream(file, d, raw).unwrap_or_default(),
                        _ => vec![],
                    };
                    Cs::Indexed { base: Box::new(base), hival, lookup }
                }
                "Separation" | "DeviceN" => {
                    let n = if head == "Separation" { 1 } else { a.get(1).map(|x| file.resolve(x)).and_then(Obj::array).map_or(1, <[Obj]>::len) };
                    let alt = a.get(2).map(|b| color_space(file, b, res)).unwrap_or(Cs::Gray);
                    let func = a.get(3).map(|f| Func::read(file, f)).unwrap_or(Func::Unsupported);
                    // An unreadable tint transform: tint 1 = black, 0 = white.
                    if func == Func::Unsupported {
                        return Cs::Tint { n, alt: Box::new(Cs::Gray), func: Func::Exp { domain: [0.0, 1.0], c0: vec![1.0], c1: vec![0.0], n: 1.0 } };
                    }
                    Cs::Tint { n, alt: Box::new(alt), func }
                }
                _ => Cs::Rgb,
            }
        }
        _ => Cs::Gray,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Func {
    /// Type 2.
    Exp {
        domain: [f64; 2],
        c0: Vec<f64>,
        c1: Vec<f64>,
        n: f64,
    },
    /// Type 3.
    Stitch {
        domain: [f64; 2],
        funcs: Vec<Func>,
        bounds: Vec<f64>,
        encode: Vec<f64>,
    },
    /// Type 0 with one input.
    Sampled {
        domain: [f64; 2],
        size: usize,
        outputs: usize,
        encode: [f64; 2],
        decode: Vec<f64>,
        samples: Vec<f64>,
    },
    /// An array of one-output functions.
    Array(Vec<Func>),
    Unsupported,
}

impl Func {
    pub fn read(file: &File, o: &Obj) -> Func {
        let o = file.resolve(o);
        if let Obj::Array(a) = o {
            return Func::Array(a.iter().map(|f| Func::read(file, f)).collect());
        }
        let Some(d) = o.dict() else { return Func::Unsupported };
        let dom = file.get(d, "Domain").map(|x| file.nums(x)).unwrap_or_default();
        let domain = [dom.first().copied().unwrap_or(0.0), dom.get(1).copied().unwrap_or(1.0)];
        match file.get_num(d, "FunctionType").unwrap_or(-1.0) as i32 {
            2 => Func::Exp {
                domain,
                c0: file.get(d, "C0").map(|x| file.nums(x)).unwrap_or_else(|| vec![0.0]),
                c1: file.get(d, "C1").map(|x| file.nums(x)).unwrap_or_else(|| vec![1.0]),
                n: file.get_num(d, "N").unwrap_or(1.0),
            },
            3 => Func::Stitch {
                domain,
                funcs: file.get(d, "Functions").and_then(Obj::array).map(|a| a.iter().map(|f| Func::read(file, f)).collect()).unwrap_or_default(),
                bounds: file.get(d, "Bounds").map(|x| file.nums(x)).unwrap_or_default(),
                encode: file.get(d, "Encode").map(|x| file.nums(x)).unwrap_or_default(),
            },
            0 => {
                let Obj::Stream(sd, raw) = o else { return Func::Unsupported };
                let Some(data) = decode_stream(file, sd, raw) else { return Func::Unsupported };
                let size = file.get(d, "Size").map(|x| file.nums(x)).and_then(|v| v.first().copied()).unwrap_or(0.0) as usize;
                let bps = file.get_num(d, "BitsPerSample").unwrap_or(8.0) as usize;
                let range = file.get(d, "Range").map(|x| file.nums(x)).unwrap_or_default();
                let outputs = range.len() / 2;
                if size == 0 || outputs == 0 || !matches!(bps, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32) {
                    return Func::Unsupported;
                }
                let enc = file.get(d, "Encode").map(|x| file.nums(x)).unwrap_or_default();
                let encode = [enc.first().copied().unwrap_or(0.0), enc.get(1).copied().unwrap_or(size as f64 - 1.0)];
                let decode = file.get(d, "Decode").map(|x| file.nums(x)).filter(|v| v.len() >= outputs * 2).unwrap_or_else(|| range.clone());
                let max = ((1u64 << bps) - 1) as f64;
                let mut samples = Vec::with_capacity(size * outputs);
                let mut bit = 0usize;
                for _ in 0..size * outputs {
                    let mut v = 0u64;
                    for _ in 0..bps {
                        let byte = data.get(bit / 8).copied().unwrap_or(0);
                        v = (v << 1) | ((byte >> (7 - bit % 8)) & 1) as u64;
                        bit += 1;
                    }
                    samples.push(v as f64 / max);
                }
                Func::Sampled { domain, size, outputs, encode, decode, samples }
            }
            _ => Func::Unsupported,
        }
    }

    /// Evaluate at the first input (the functions used by shadings and tints take one input
    /// in practice; extra inputs of DeviceN tints are averaged in).
    pub fn eval(&self, input: &[f64]) -> Vec<f64> {
        let t = input.first().copied().unwrap_or(0.0);
        self.eval1(t)
    }

    pub fn eval1(&self, t: f64) -> Vec<f64> {
        match self {
            Func::Exp { domain, c0, c1, n } => {
                let t = t.clamp(domain[0].min(domain[1]), domain[0].max(domain[1]));
                let x = if *n == 1.0 { t } else { t.max(0.0).powf(*n) };
                (0..c0.len().max(c1.len()))
                    .map(|i| c0.get(i).copied().unwrap_or(0.0) + x * (c1.get(i).copied().unwrap_or(1.0) - c0.get(i).copied().unwrap_or(0.0)))
                    .collect()
            }
            Func::Stitch { domain, funcs, bounds, encode } => {
                if funcs.is_empty() {
                    return vec![0.0];
                }
                let t = t.clamp(domain[0], domain[1]);
                let k = bounds.iter().take_while(|b| t >= **b).count().min(funcs.len() - 1);
                let lo = if k == 0 { domain[0] } else { bounds[k - 1] };
                let hi = bounds.get(k).copied().unwrap_or(domain[1]);
                let (e0, e1) = (encode.get(2 * k).copied().unwrap_or(0.0), encode.get(2 * k + 1).copied().unwrap_or(1.0));
                let u = if hi > lo { e0 + (t - lo) / (hi - lo) * (e1 - e0) } else { e0 };
                funcs[k].eval1(u)
            }
            Func::Sampled { domain, size, outputs, encode, decode, samples } => {
                let t = t.clamp(domain[0], domain[1]);
                let e = if domain[1] > domain[0] { encode[0] + (t - domain[0]) / (domain[1] - domain[0]) * (encode[1] - encode[0]) } else { encode[0] };
                let e = e.clamp(0.0, *size as f64 - 1.0);
                let (i0, f) = (e.floor() as usize, e.fract());
                let i1 = (i0 + 1).min(size - 1);
                (0..*outputs)
                    .map(|o| {
                        let s = samples[i0 * outputs + o] * (1.0 - f) + samples[i1 * outputs + o] * f;
                        let (d0, d1) = (decode.get(2 * o).copied().unwrap_or(0.0), decode.get(2 * o + 1).copied().unwrap_or(1.0));
                        d0 + s * (d1 - d0)
                    })
                    .collect()
            }
            Func::Array(fs) => fs.iter().map(|f| f.eval1(t).first().copied().unwrap_or(0.0)).collect(),
            Func::Unsupported => vec![0.5],
        }
    }

    /// Input values worth sampling exactly (stitching bounds).
    pub fn breakpoints(&self) -> Vec<f64> {
        match self {
            Func::Stitch { bounds, .. } => bounds.clone(),
            _ => vec![],
        }
    }
}

/// PDF text string → Rust string (UTF-16BE with a byte-order mark, else PDFDocEncoding ≈ Latin-1).
pub fn text_string(b: &[u8]) -> String {
    if b.starts_with(&[0xFE, 0xFF]) {
        let u: Vec<u16> = b[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&u);
    }
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    b.iter().map(|&c| c as char).collect()
}
