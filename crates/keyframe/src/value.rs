//! Property values.

use serde::{Deserialize, Serialize};

/// A Bezier shape in the After Effects / Lottie vertex model: vertices with tangents relative to
/// their vertex.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapePath {
    pub vertices: Vec<[f64; 2]>,
    pub in_tangents: Vec<[f64; 2]>,
    pub out_tangents: Vec<[f64; 2]>,
    pub closed: bool,
}

impl ShapePath {
    pub fn polygon(points: &[[f64; 2]], closed: bool) -> ShapePath {
        ShapePath { vertices: points.to_vec(), in_tangents: vec![[0.0; 2]; points.len()], out_tangents: vec![[0.0; 2]; points.len()], closed }
    }
    pub fn len(&self) -> usize {
        self.vertices.len()
    }
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
    /// Rectangle centred at `c` (clockwise from top-left, as AE builds mask rectangles).
    pub fn rect(c: [f64; 2], w: f64, h: f64) -> ShapePath {
        let (x0, y0, x1, y1) = (c[0] - w / 2.0, c[1] - h / 2.0, c[0] + w / 2.0, c[1] + h / 2.0);
        ShapePath::polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true)
    }
    /// Ellipse with four Bezier arcs (kappa 0.5523).
    pub fn ellipse(c: [f64; 2], w: f64, h: f64) -> ShapePath {
        const K: f64 = 0.552_284_749_8;
        let (rx, ry) = (w / 2.0, h / 2.0);
        ShapePath {
            vertices: vec![[c[0], c[1] - ry], [c[0] + rx, c[1]], [c[0], c[1] + ry], [c[0] - rx, c[1]]],
            in_tangents: vec![[-rx * K, 0.0], [0.0, -ry * K], [rx * K, 0.0], [0.0, ry * K]],
            out_tangents: vec![[rx * K, 0.0], [0.0, ry * K], [-rx * K, 0.0], [0.0, -ry * K]],
            closed: true,
        }
    }
    fn lerp(&self, o: &ShapePath, t: f64) -> ShapePath {
        if self.len() != o.len() {
            return if t < 1.0 { self.clone() } else { o.clone() };
        }
        let l = |a: &[[f64; 2]], b: &[[f64; 2]]| a.iter().zip(b).map(|(p, q)| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]).collect();
        ShapePath {
            vertices: l(&self.vertices, &o.vertices),
            in_tangents: l(&self.in_tangents, &o.in_tangents),
            out_tangents: l(&self.out_tangents, &o.out_tangents),
            closed: self.closed,
        }
    }
}

pub use crate::text_doc::{Justify, TextDoc};

/// A gradient: colour stops `(position 0..1, rgba)` and opacity stops `(position, alpha)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub colors: Vec<(f64, [f32; 4])>,
    pub opacities: Vec<(f64, f32)>,
}

impl Default for Gradient {
    fn default() -> Self {
        Gradient { colors: vec![(0.0, [1.0, 1.0, 1.0, 1.0]), (1.0, [0.0, 0.0, 0.0, 1.0])], opacities: vec![(0.0, 1.0), (1.0, 1.0)] }
    }
}

impl Gradient {
    pub fn sample(&self, t: f64) -> [f32; 4] {
        let t = t.clamp(0.0, 1.0);
        let col = sample_stops(&self.colors, t, |a, b, f| {
            let mut o = [0.0; 4];
            for i in 0..4 {
                o[i] = a[i] + (b[i] - a[i]) * f as f32;
            }
            o
        })
        .unwrap_or([1.0; 4]);
        let a = sample_stops(&self.opacities, t, |a, b, f| a + (b - a) * f as f32).unwrap_or(1.0);
        [col[0], col[1], col[2], col[3] * a]
    }
}

fn sample_stops<T: Copy>(stops: &[(f64, T)], t: f64, lerp: impl Fn(T, T, f64) -> T) -> Option<T> {
    let first = stops.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    for w in stops.windows(2) {
        if t <= w[1].0 {
            let span = (w[1].0 - w[0].0).max(1e-12);
            return Some(lerp(w[0].1, w[1].1, (t - w[0].0) / span));
        }
    }
    stops.last().map(|s| s.1)
}

/// The value of a property.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
pub enum Value {
    Scalar(f64),
    Vec2([f64; 2]),
    Vec3([f64; 3]),
    /// Straight RGBA 0..1.
    Color([f64; 4]),
    Bool(bool),
    /// Index into a popup's options (1-based like AE's dropdown values is a UI concern; this is 0-based).
    Enum(u32),
    Path(ShapePath),
    Text(Box<TextDoc>),
    /// Layer reference (layer id), e.g. Layer Control effects, track matte sources.
    Layer(Option<u64>),
    Gradient(Gradient),
    Str(String),
}

impl Value {
    /// Numeric components (empty for non-numeric values).
    pub fn components(&self) -> Vec<f64> {
        match self {
            Value::Scalar(v) => vec![*v],
            Value::Vec2(v) => v.to_vec(),
            Value::Vec3(v) => v.to_vec(),
            Value::Color(v) => v.to_vec(),
            Value::Bool(b) => vec![if *b { 1.0 } else { 0.0 }],
            Value::Enum(i) => vec![*i as f64],
            _ => vec![],
        }
    }
    pub fn dims(&self) -> usize {
        match self {
            Value::Scalar(_) | Value::Bool(_) | Value::Enum(_) => 1,
            Value::Vec2(_) => 2,
            Value::Vec3(_) => 3,
            Value::Color(_) => 4,
            _ => 0,
        }
    }
    /// Rebuild a value of the same kind from components.
    pub fn with_components(&self, c: &[f64]) -> Value {
        let g = |i: usize| c.get(i).copied().unwrap_or(0.0);
        match self {
            Value::Scalar(_) => Value::Scalar(g(0)),
            Value::Vec2(_) => Value::Vec2([g(0), g(1)]),
            Value::Vec3(_) => Value::Vec3([g(0), g(1), g(2)]),
            Value::Color(_) => Value::Color([g(0), g(1), g(2), g(3)]),
            Value::Bool(_) => Value::Bool(g(0) >= 0.5),
            Value::Enum(_) => Value::Enum(g(0).round().max(0.0) as u32),
            v => v.clone(),
        }
    }
    /// Can be interpolated (otherwise keyframes behave as hold).
    pub fn interpolates(&self) -> bool {
        matches!(self, Value::Scalar(_) | Value::Vec2(_) | Value::Vec3(_) | Value::Color(_) | Value::Path(_) | Value::Gradient(_))
    }
    pub fn lerp(&self, o: &Value, t: f64) -> Value {
        match (self, o) {
            (Value::Path(a), Value::Path(b)) => Value::Path(a.lerp(b, t)),
            (Value::Gradient(a), Value::Gradient(b)) if a.colors.len() == b.colors.len() && a.opacities.len() == b.opacities.len() => {
                let colors = a
                    .colors
                    .iter()
                    .zip(&b.colors)
                    .map(|(x, y)| {
                        let mut c = [0.0; 4];
                        for i in 0..4 {
                            c[i] = x.1[i] + (y.1[i] - x.1[i]) * t as f32;
                        }
                        (x.0 + (y.0 - x.0) * t, c)
                    })
                    .collect();
                let opacities = a.opacities.iter().zip(&b.opacities).map(|(x, y)| (x.0 + (y.0 - x.0) * t, x.1 + (y.1 - x.1) * t as f32)).collect();
                Value::Gradient(Gradient { colors, opacities })
            }
            _ if self.interpolates() && self.dims() > 0 && self.dims() == o.dims() => {
                let a = self.components();
                let b = o.components();
                self.with_components(&a.iter().zip(&b).map(|(x, y)| x + (y - x) * t).collect::<Vec<_>>())
            }
            _ => {
                if t < 1.0 {
                    self.clone()
                } else {
                    o.clone()
                }
            }
        }
    }
    pub fn as_f64(&self) -> f64 {
        self.components().first().copied().unwrap_or(0.0)
    }
    pub fn as_vec2(&self) -> [f64; 2] {
        let c = self.components();
        [c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0)]
    }
    pub fn as_vec3(&self) -> [f64; 3] {
        let c = self.components();
        [c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0), c.get(2).copied().unwrap_or(0.0)]
    }
    pub fn as_color(&self) -> [f32; 4] {
        let c = self.components();
        let g = |i: usize, d: f64| c.get(i).copied().unwrap_or(d) as f32;
        [g(0, 0.0), g(1, 0.0), g(2, 0.0), g(3, 1.0)]
    }
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            v => v.as_f64() != 0.0,
        }
    }
    pub fn as_enum(&self) -> u32 {
        match self {
            Value::Enum(i) => *i,
            v => v.as_f64().max(0.0) as u32,
        }
    }
    pub fn as_path(&self) -> Option<&ShapePath> {
        if let Value::Path(p) = self { Some(p) } else { None }
    }
    pub fn as_text(&self) -> Option<&TextDoc> {
        if let Value::Text(t) = self { Some(t) } else { None }
    }
    pub fn as_layer(&self) -> Option<u64> {
        if let Value::Layer(l) = self { *l } else { None }
    }
    pub fn kind_name(&self) -> &'static str {
        match self {
            Value::Scalar(_) => "scalar",
            Value::Vec2(_) => "vec2",
            Value::Vec3(_) => "vec3",
            Value::Color(_) => "color",
            Value::Bool(_) => "bool",
            Value::Enum(_) => "enum",
            Value::Path(_) => "path",
            Value::Text(_) => "text",
            Value::Layer(_) => "layer",
            Value::Gradient(_) => "gradient",
            Value::Str(_) => "string",
        }
    }
    /// Lenient conversion from JSON (numbers, arrays, booleans, strings) to a value shaped like `self`.
    pub fn coerce_json(&self, j: &serde_json::Value) -> Option<Value> {
        use serde_json::Value as J;
        let nums = |j: &J| -> Option<Vec<f64>> {
            match j {
                J::Number(n) => n.as_f64().map(|v| vec![v]),
                J::Array(a) => a.iter().map(J::as_f64).collect(),
                J::Bool(b) => Some(vec![if *b { 1.0 } else { 0.0 }]),
                _ => None,
            }
        };
        match self {
            Value::Scalar(_) | Value::Vec2(_) | Value::Vec3(_) | Value::Enum(_) => {
                let mut c = nums(j)?;
                if c.len() == 1 && self.dims() > 1 {
                    c = vec![c[0]; self.dims()];
                }
                Some(self.with_components(&c))
            }
            Value::Color(_) => {
                if let Some(s) = j.as_str() {
                    let c = effectcraft_color::Rgba::from_hex(s)?;
                    return Some(Value::Color([c.r as f64, c.g as f64, c.b as f64, c.a as f64]));
                }
                let mut c = nums(j)?;
                if c.len() == 3 {
                    c.push(1.0);
                }
                Some(self.with_components(&c))
            }
            Value::Bool(_) => j.as_bool().or_else(|| j.as_f64().map(|v| v != 0.0)).map(Value::Bool),
            Value::Str(_) => j.as_str().map(|s| Value::Str(s.to_string())),
            Value::Text(t) => {
                if let Some(s) = j.as_str() {
                    let mut t = (**t).clone();
                    t.set_text(s);
                    Some(Value::Text(Box::new(t)))
                } else {
                    serde_json::from_value::<TextDoc>(j.clone()).ok().map(|mut t| {
                        t.normalize();
                        Value::Text(Box::new(t))
                    })
                }
            }
            Value::Layer(_) => Some(Value::Layer(j.as_u64())),
            Value::Path(_) => serde_json::from_value::<ShapePath>(j.clone()).ok().map(Value::Path).or_else(|| serde_json::from_value::<Value>(j.clone()).ok()),
            _ => serde_json::from_value::<Value>(j.clone()).ok(),
        }
    }
    /// JSON for display/automation (numbers/arrays for numeric kinds).
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Value::Scalar(v) => json!(v),
            Value::Vec2(v) => json!(v),
            Value::Vec3(v) => json!(v),
            Value::Color(v) => json!(v),
            Value::Bool(b) => json!(b),
            Value::Enum(i) => json!(i),
            Value::Str(s) => json!(s),
            Value::Layer(l) => json!(l),
            Value::Text(t) => json!(t.text),
            v => serde_json::to_value(v).unwrap_or_default(),
        }
    }
}
