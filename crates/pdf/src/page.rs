//! Pages and content streams (ISO 32000-1 §7.7.3, §8): the page tree, the graphics state,
//! path construction and painting, clipping, colour, shading patterns and `sh`, form XObjects
//! and optional-content marked sequences. Text, images and tiling patterns are skipped (listed
//! in [`effectcraft_svg::Doc::skipped`]).

use effectcraft_svg::{Affine, BezPath, Cap, FillRule, Gradient, GradientKind, Join, Paint, Spread, Stroke};
use kurbo::Point;

use crate::build::Builder;
use crate::color::{Cs, Func, color_space, text_string};
use crate::object::{Dict, File, Lexer, Obj, decode_stream};

/// A page: its dictionary with inherited attributes resolved.
pub struct Page {
    pub dict: Dict,
    pub resources: Dict,
    /// CropBox (else MediaBox) `[x0, y0, x1, y1]`.
    pub bbox: [f64; 4],
    pub rotate: i32,
}

/// The pages in order.
pub fn pages(file: &File) -> Vec<Page> {
    let mut out = vec![];
    let Some(cat) = file.catalog() else { return out };
    let Some(root) = file.get_dict(cat, "Pages") else { return out };
    fn walk(file: &File, node: &Dict, inherited: (Option<Dict>, Option<Vec<f64>>, Option<Vec<f64>>, i32), out: &mut Vec<Page>, depth: usize) {
        if depth > 32 || out.len() > 10_000 {
            return;
        }
        let res = file.get_dict(node, "Resources").cloned().or(inherited.0);
        let media = file.get(node, "MediaBox").map(|m| file.nums(m)).filter(|v| v.len() == 4).or(inherited.1);
        let crop = file.get(node, "CropBox").map(|m| file.nums(m)).filter(|v| v.len() == 4).or(inherited.2);
        let rot = file.get_num(node, "Rotate").map(|r| r as i32).unwrap_or(inherited.3);
        match file.get(node, "Kids").and_then(Obj::array) {
            Some(kids) if file.get(node, "Type").and_then(Obj::name) != Some("Page") => {
                for k in kids {
                    if let Some(d) = file.resolve(k).dict() {
                        walk(file, d, (res.clone(), media.clone(), crop.clone(), rot), out, depth + 1);
                    }
                }
            }
            _ => {
                let b = crop.or(media).unwrap_or_else(|| vec![0.0, 0.0, 612.0, 792.0]);
                let bbox = [b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3])];
                out.push(Page { dict: node.clone(), resources: res.unwrap_or_default(), bbox, rotate: rot.rem_euclid(360) });
            }
        }
    }
    walk(file, root, (None, None, None, 0), &mut out, 0);
    out
}

/// The page's content streams, decoded and joined.
pub fn page_content(file: &File, page: &Page) -> Vec<u8> {
    let mut out = vec![];
    let parts: Vec<&Obj> = match page.dict.get("Contents").map(|c| file.resolve(c)) {
        Some(Obj::Array(a)) => a.iter().map(|x| file.resolve(x)).collect(),
        Some(o) => vec![o],
        None => vec![],
    };
    for p in parts {
        if let Obj::Stream(d, raw) = p
            && let Some(data) = decode_stream(file, d, raw)
        {
            out.extend_from_slice(&data);
            out.push(b'\n');
        }
    }
    out
}

#[derive(Clone)]
enum Fill {
    Color([f64; 3]),
    Paint(Paint),
    None,
}

#[derive(Clone)]
struct GState {
    ctm: Affine,
    fill_cs: Cs,
    stroke_cs: Cs,
    fill: Fill,
    stroke: Fill,
    lw: f64,
    cap: Cap,
    join: Join,
    miter: f64,
    dash: Option<(Vec<f64>, f64)>,
    fill_alpha: f64,
    stroke_alpha: f64,
}

impl Default for GState {
    fn default() -> Self {
        GState {
            ctm: Affine::IDENTITY,
            fill_cs: Cs::Gray,
            stroke_cs: Cs::Gray,
            fill: Fill::Color([0.0; 3]),
            stroke: Fill::Color([0.0; 3]),
            lw: 1.0,
            cap: Cap::Butt,
            join: Join::Miter,
            miter: 10.0,
            dash: None,
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
        }
    }
}

pub(crate) struct Interp<'a> {
    file: &'a File,
    pub b: Builder,
    gs: GState,
    saved: Vec<(GState, usize)>,
    path: BezPath,
    cur: Option<Point>,
    start: Option<Point>,
    pending_clip: Option<FillRule>,
    marked: Vec<Option<usize>>,
    /// The page box in default user space (for `sh`).
    page_box: [f64; 4],
    budget: usize,
}

fn affine(v: &[f64]) -> Affine {
    if v.len() < 6 {
        return Affine::IDENTITY;
    }
    Affine::new([v[0], v[1], v[2], v[3], v[4], v[5]])
}

impl<'a> Interp<'a> {
    pub fn new(file: &'a File, page_box: [f64; 4]) -> Self {
        Interp {
            file,
            b: Builder::new(),
            gs: GState::default(),
            saved: vec![],
            path: BezPath::new(),
            cur: None,
            start: None,
            pending_clip: None,
            marked: vec![],
            page_box,
            budget: 20_000_000,
        }
    }

    /// Run a content stream with resources `res`; `pattern_base` maps pattern space to the page
    /// (the CTM where the stream's default coordinate system starts).
    pub fn run(&mut self, content: &[u8], res: &Dict, pattern_base: Affine, depth: usize) {
        let mut lx = Lexer::new(content, 0);
        let mut args: Vec<Obj> = Vec::with_capacity(8);
        let mut in_text = false;
        while let Some(o) = lx.next() {
            let Obj::Op(op) = o else {
                if args.len() < 64 {
                    args.push(o);
                }
                continue;
            };
            if self.budget == 0 {
                self.b.skip("content (operation limit)");
                return;
            }
            self.budget -= 1;
            if in_text {
                if op == "ET" {
                    in_text = false;
                }
                args.clear();
                continue;
            }
            let n = |i: usize| args.get(i).and_then(Obj::num).unwrap_or(0.0);
            let nums = || args.iter().filter_map(Obj::num).collect::<Vec<f64>>();
            match op.as_str() {
                "q" => self.saved.push((self.gs.clone(), self.b.depth())),
                "Q" => {
                    if let Some((g, d)) = self.saved.pop() {
                        self.gs = g;
                        self.b.close_to(d);
                    }
                }
                "cm" => self.gs.ctm *= affine(&nums()),
                "w" => self.gs.lw = n(0),
                "J" => self.gs.cap = [Cap::Butt, Cap::Round, Cap::Square][(n(0) as usize).min(2)],
                "j" => self.gs.join = [Join::Miter, Join::Round, Join::Bevel][(n(0) as usize).min(2)],
                "M" => self.gs.miter = n(0),
                "d" => {
                    let arr = args.first().map(|a| self.file.nums(a)).unwrap_or_default();
                    self.gs.dash = (!arr.is_empty() && arr.iter().any(|v| *v > 0.0)).then(|| (arr, n(1)));
                }
                "gs" => {
                    if let Some(name) = args.first().and_then(Obj::name)
                        && let Some(g) = self.file.get_dict(res, "ExtGState").and_then(|e| self.file.get_dict(e, name))
                    {
                        self.ext_gstate(g);
                    }
                }
                // Path construction.
                "m" => {
                    let p = Point::new(n(0), n(1));
                    self.path.move_to(p);
                    self.cur = Some(p);
                    self.start = Some(p);
                }
                "l" => {
                    let p = Point::new(n(0), n(1));
                    self.ensure_start();
                    self.path.line_to(p);
                    self.cur = Some(p);
                }
                "c" => {
                    self.ensure_start();
                    let p = Point::new(n(4), n(5));
                    self.path.curve_to(Point::new(n(0), n(1)), Point::new(n(2), n(3)), p);
                    self.cur = Some(p);
                }
                "v" => {
                    self.ensure_start();
                    let c0 = self.cur.unwrap_or_default();
                    let p = Point::new(n(2), n(3));
                    self.path.curve_to(c0, Point::new(n(0), n(1)), p);
                    self.cur = Some(p);
                }
                "y" => {
                    self.ensure_start();
                    let p = Point::new(n(2), n(3));
                    self.path.curve_to(Point::new(n(0), n(1)), p, p);
                    self.cur = Some(p);
                }
                "h" => {
                    if self.cur.is_some() {
                        self.path.close_path();
                        self.cur = self.start;
                    }
                }
                "re" => {
                    let (x, y, w, h) = (n(0), n(1), n(2), n(3));
                    self.path.move_to((x, y));
                    self.path.line_to((x + w, y));
                    self.path.line_to((x + w, y + h));
                    self.path.line_to((x, y + h));
                    self.path.close_path();
                    self.cur = Some(Point::new(x, y));
                    self.start = self.cur;
                }
                // Painting.
                "S" => self.paint(false, None, true),
                "s" => {
                    self.path.close_path();
                    self.paint(false, None, true)
                }
                "f" | "F" => self.paint(true, Some(FillRule::NonZero), false),
                "f*" => self.paint(true, Some(FillRule::EvenOdd), false),
                "B" => self.paint(true, Some(FillRule::NonZero), true),
                "B*" => self.paint(true, Some(FillRule::EvenOdd), true),
                "b" => {
                    self.path.close_path();
                    self.paint(true, Some(FillRule::NonZero), true)
                }
                "b*" => {
                    self.path.close_path();
                    self.paint(true, Some(FillRule::EvenOdd), true)
                }
                "n" => self.paint(false, None, false),
                "W" => self.pending_clip = Some(FillRule::NonZero),
                "W*" => self.pending_clip = Some(FillRule::EvenOdd),
                // Colour.
                "g" => self.gs.fill_cs = Cs::Gray,
                "G" => self.gs.stroke_cs = Cs::Gray,
                "rg" => self.gs.fill_cs = Cs::Rgb,
                "RG" => self.gs.stroke_cs = Cs::Rgb,
                "k" => self.gs.fill_cs = Cs::Cmyk,
                "K" => self.gs.stroke_cs = Cs::Cmyk,
                "cs" | "CS" => {
                    let cs = args.first().map(|a| color_space(self.file, a, Some(res))).unwrap_or(Cs::Gray);
                    let init = Fill::Color(cs.to_rgb(&cs.initial()));
                    if op == "cs" {
                        self.gs.fill_cs = cs;
                        self.gs.fill = init;
                    } else {
                        self.gs.stroke_cs = cs;
                        self.gs.stroke = init;
                    }
                }
                "sc" | "scn" | "SC" | "SCN" => {
                    let fill = op.starts_with('s');
                    let cs = if fill { self.gs.fill_cs.clone() } else { self.gs.stroke_cs.clone() };
                    let v = if let (Cs::Pattern, Some(name)) = (&cs, args.last().and_then(Obj::name)) {
                        self.pattern(res, name, pattern_base)
                    } else {
                        Fill::Color(cs.to_rgb(&nums()))
                    };
                    if fill {
                        self.gs.fill = v;
                    } else {
                        self.gs.stroke = v;
                    }
                }
                "sh" => {
                    if let Some(name) = args.first().and_then(Obj::name) {
                        self.shade(res, name);
                    }
                }
                "Do" => {
                    if let Some(name) = args.first().and_then(Obj::name) {
                        self.xobject(res, name, depth);
                    }
                }
                "BT" => {
                    in_text = true;
                    self.b.skip("text");
                }
                "BI" => {
                    // Inline image: skip to EI.
                    let d = lx.data;
                    let mut p = lx.pos;
                    while p + 2 < d.len() && !(d[p] == b'I' && d[p - 1] == b'D' && crate::object::is_white(d[p + 1])) {
                        p += 1;
                    }
                    while p + 2 < d.len()
                        && !(crate::object::is_white(d[p - 1]) && d[p] == b'E' && d[p + 1] == b'I' && (p + 2 == d.len() || crate::object::is_white(d[p + 2])))
                    {
                        p += 1;
                    }
                    lx.pos = (p + 2).min(d.len());
                    self.b.skip("image");
                }
                "BMC" => self.marked.push(None),
                "BDC" => {
                    let layer = match (args.first().and_then(Obj::name), args.get(1)) {
                        (Some("OC"), Some(Obj::Name(prop))) => self
                            .file
                            .get_dict(res, "Properties")
                            .and_then(|p| self.file.get_dict(p, prop))
                            .map(|ocg| self.ocg_name(ocg).unwrap_or_else(|| prop.clone())),
                        (Some("OC"), Some(Obj::Dict(ocg))) => self.ocg_name(ocg),
                        _ => None,
                    };
                    match layer {
                        Some(name) => {
                            self.marked.push(Some(self.b.depth()));
                            self.b.open(&name, true);
                        }
                        None => self.marked.push(None),
                    }
                }
                "EMC" => {
                    if let Some(Some(d)) = self.marked.pop() {
                        self.b.close_to(d);
                    }
                }
                _ => {}
            }
            // Colour operands set by the device-space shorthands.
            match op.as_str() {
                "g" | "rg" | "k" => self.gs.fill = Fill::Color(self.gs.fill_cs.to_rgb(&nums())),
                "G" | "RG" | "K" => self.gs.stroke = Fill::Color(self.gs.stroke_cs.to_rgb(&nums())),
                _ => {}
            }
            args.clear();
        }
    }

    fn ocg_name(&self, ocg: &Dict) -> Option<String> {
        // An optional-content membership dictionary names its groups in /OCGs.
        let d = match self.file.get(ocg, "OCGs") {
            Some(Obj::Dict(g)) => g,
            Some(Obj::Array(a)) => a.first().map(|x| self.file.resolve(x)).and_then(Obj::dict).unwrap_or(ocg),
            _ => ocg,
        };
        match self.file.get(d, "Name") {
            Some(Obj::Str(s)) => Some(text_string(s)),
            _ => None,
        }
    }

    fn ensure_start(&mut self) {
        if self.cur.is_none() {
            self.path.move_to((0.0, 0.0));
            self.cur = Some(Point::ZERO);
            self.start = self.cur;
        }
    }

    fn ext_gstate(&mut self, g: &Dict) {
        let f = self.file;
        if let Some(v) = f.get_num(g, "LW") {
            self.gs.lw = v;
        }
        if let Some(v) = f.get_num(g, "LC") {
            self.gs.cap = [Cap::Butt, Cap::Round, Cap::Square][(v as usize).min(2)];
        }
        if let Some(v) = f.get_num(g, "LJ") {
            self.gs.join = [Join::Miter, Join::Round, Join::Bevel][(v as usize).min(2)];
        }
        if let Some(v) = f.get_num(g, "ML") {
            self.gs.miter = v;
        }
        if let Some(v) = f.get_num(g, "CA") {
            self.gs.stroke_alpha = v.clamp(0.0, 1.0);
        }
        if let Some(v) = f.get_num(g, "ca") {
            self.gs.fill_alpha = v.clamp(0.0, 1.0);
        }
        if let Some(Obj::Array(d)) = f.get(g, "D") {
            let arr = d.first().map(|a| f.nums(a)).unwrap_or_default();
            let ph = d.get(1).and_then(|p| f.resolve(p).num()).unwrap_or(0.0);
            self.gs.dash = (!arr.is_empty() && arr.iter().any(|v| *v > 0.0)).then_some((arr, ph));
        }
        if f.get(g, "SMask").is_some_and(|s| !matches!(s, Obj::Name(n) if n == "None")) {
            self.b.skip("soft mask");
        }
    }

    fn paint_of(f: &Fill) -> Option<Paint> {
        match f {
            Fill::Color(c) => Some(Paint::Color(*c)),
            Fill::Paint(p) => Some(p.clone()),
            Fill::None => None,
        }
    }

    fn paint(&mut self, fill: bool, rule: Option<FillRule>, stroke: bool) {
        let path = std::mem::take(&mut self.path);
        self.cur = None;
        self.start = None;
        let ctm = self.gs.ctm;
        let fill = if fill { Self::paint_of(&self.gs.fill).map(|p| (p, self.gs.fill_alpha, rule.unwrap_or(FillRule::NonZero))) } else { None };
        let stroke = if stroke {
            Self::paint_of(&self.gs.stroke).map(|p| {
                // Width 0: the thinnest line the device can draw (one pixel).
                let s = { (ctm.determinant().abs()).sqrt() };
                let width = if self.gs.lw <= 0.0 { if s > 0.0 { 1.0 / s } else { 1.0 } } else { self.gs.lw };
                Stroke {
                    paint: p,
                    opacity: self.gs.stroke_alpha,
                    width,
                    cap: self.gs.cap,
                    join: self.gs.join,
                    miter: self.gs.miter.max(1.0),
                    dash: self.gs.dash.clone(),
                }
            })
        } else {
            None
        };
        if fill.is_some() || stroke.is_some() {
            self.b.fill_stroke(path.clone(), ctm, fill, stroke);
        }
        if let Some(r) = self.pending_clip.take() {
            self.b.clip(ctm * path, r);
        }
    }

    /// A shading dictionary → gradient paint in shading space.
    fn shading_gradient(&mut self, sh: &Dict) -> Option<Gradient> {
        let f = self.file;
        let ty = f.get_num(sh, "ShadingType").unwrap_or(0.0) as i32;
        let cs = f.get(sh, "ColorSpace").map(|c| color_space(f, c, None)).unwrap_or(Cs::Rgb);
        let coords = f.get(sh, "Coords").map(|c| f.nums(c)).unwrap_or_default();
        let func = f.get(sh, "Function").map(|x| Func::read(f, x)).unwrap_or(Func::Unsupported);
        let dom = f.get(sh, "Domain").map(|d| f.nums(d)).filter(|d| d.len() == 2).unwrap_or_else(|| vec![0.0, 1.0]);
        let kind = match (ty, coords.len()) {
            (2, 4) => GradientKind::Linear { x1: coords[0], y1: coords[1], x2: coords[2], y2: coords[3] },
            (3, 6) => GradientKind::Radial { cx: coords[3], cy: coords[4], r: coords[5], fx: coords[0], fy: coords[1] },
            _ => {
                self.b.skip(&format!("shading type {ty}"));
                return None;
            }
        };
        if func == Func::Unsupported {
            self.b.skip("shading function");
        }
        // Sample the function: evenly, plus exactly at stitching bounds.
        let mut ts: Vec<f64> = (0..=24).map(|i| i as f64 / 24.0).collect();
        for b in func.breakpoints() {
            if dom[1] > dom[0] {
                ts.push(((b - dom[0]) / (dom[1] - dom[0])).clamp(0.0, 1.0));
            }
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let stops = ts
            .into_iter()
            .map(|t| {
                let c = cs.to_rgb(&func.eval1(dom[0] + t * (dom[1] - dom[0])));
                (t, [c[0], c[1], c[2], 1.0])
            })
            .collect();
        Some(Gradient { kind, stops, bbox_units: false, transform: Affine::IDENTITY, spread: Spread::Pad })
    }

    fn pattern(&mut self, res: &Dict, name: &str, base: Affine) -> Fill {
        let f = self.file;
        let Some(p) = f.get_dict(res, "Pattern").and_then(|pp| f.get_dict(pp, name)) else { return Fill::None };
        match f.get_num(p, "PatternType").unwrap_or(0.0) as i32 {
            2 => {
                let m = f.get(p, "Matrix").map(|m| affine(&f.nums(m))).unwrap_or(Affine::IDENTITY);
                let Some(sh) = f.get(p, "Shading").and_then(Obj::dict) else { return Fill::None };
                let sh = sh.clone();
                match self.shading_gradient(&sh) {
                    // Gradient space → shape user space (shapes carry the CTM).
                    Some(mut g) => {
                        g.transform = self.gs.ctm.inverse() * base * m;
                        Fill::Paint(Paint::Gradient(g))
                    }
                    None => Fill::None,
                }
            }
            _ => {
                self.b.skip("tiling pattern");
                Fill::Color([0.5; 3])
            }
        }
    }

    fn shade(&mut self, res: &Dict, name: &str) {
        let f = self.file;
        let Some(sh) = f.get_dict(res, "Shading").and_then(|s| f.get_dict(s, name)).cloned() else { return };
        let Some(g) = self.shading_gradient(&sh) else { return };
        // Fill everything (the current clip limits it): the page box in user space.
        let inv = self.gs.ctm.inverse();
        let b = self.page_box;
        let mut path = BezPath::new();
        for (i, (x, y)) in [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])].into_iter().enumerate() {
            let p = inv * Point::new(x, y);
            if i == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
        path.close_path();
        self.b.fill_stroke(path, self.gs.ctm, Some((Paint::Gradient(g), self.gs.fill_alpha, FillRule::NonZero)), None);
    }

    fn xobject(&mut self, res: &Dict, name: &str, depth: usize) {
        let f = self.file;
        let Some(x) = f.get_dict(res, "XObject").and_then(|xs| xs.get(name)).map(|x| f.resolve(x)) else { return };
        let Obj::Stream(d, raw) = x else { return };
        match f.get(d, "Subtype").and_then(Obj::name) {
            Some("Form") if depth < 12 => {
                let Some(content) = decode_stream(f, d, raw) else { return };
                let m = f.get(d, "Matrix").map(|m| affine(&f.nums(m))).unwrap_or(Affine::IDENTITY);
                let form_res = f.get_dict(d, "Resources").cloned().unwrap_or_else(|| res.clone());
                self.saved.push((self.gs.clone(), self.b.depth()));
                self.gs.ctm *= m;
                let bb = f.get(d, "BBox").map(|b| f.nums(b)).unwrap_or_default();
                if bb.len() == 4 {
                    let r = kurbo::Rect::new(bb[0], bb[1], bb[2], bb[3]);
                    self.b.clip(self.gs.ctm * kurbo::Shape::to_path(&r, 0.1), FillRule::NonZero);
                }
                let base = self.gs.ctm;
                let path = std::mem::take(&mut self.path);
                self.run(&content, &form_res, base, depth + 1);
                self.path = path;
                if let Some((g, dd)) = self.saved.pop() {
                    self.gs = g;
                    self.b.close_to(dd);
                }
            }
            Some("Image") => self.b.skip("image"),
            _ => {}
        }
    }
}
