//! Content-Aware Fill (Window ▸ Content-Aware Fill, Layer ▸ New ▸ Content-Aware Fill Layer):
//! fill the transparent area of a layer (what its masks cut away) across the work area or the
//! whole layer, render the result as a PNG sequence and add it as a "Fill" layer above the
//! source, like After Effects. The algorithms are `effectcraft_raster::inpaint` (Object:
//! flow-guided propagation + PatchMatch; Surface: propagation + one synthesised fill carried by
//! the flow; Edge Blend: membrane fill). It runs as a background job (Window ▸ Progress).

use std::sync::Arc;

use effectcraft_color::Label;
use effectcraft_project::{Footage, FootageKind, ItemId, ItemKind, LayerId, LayerSource, build};
use effectcraft_raster::Image;
use effectcraft_raster::inpaint::{FillInput, FillMethod, FillOpts, dilate, fill_sequence};
use effectcraft_render::{EvalCtx, RenderOpts, Renderer};
use effectcraft_time::Tick;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::scene_detect::LayerFrames;
use super::{CommandSpec, always, b_p, bad, f_p, has_layers, layer_p, str_p};
use crate::jobs::Apply;
use crate::{EngineError, Result, Session, cmd};

/// Content-Aware Fill panel settings (serde, in [`crate::EditorState`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FillSettings {
    /// `object`, `surface` or `edgeBlend`.
    pub method: String,
    /// `workArea` or `entire` (the layer's duration).
    pub range: String,
    /// Grow the fill area by this many pixels.
    pub alpha_expansion: f64,
    /// `none`, `subtle`, `moderate`, `strong`.
    pub lighting_correction: String,
    /// Reference frame: a layer of the comp and the comp time of the frame to use.
    pub reference_layer: Option<LayerId>,
    pub reference_time: Option<f64>,
}

impl Default for FillSettings {
    fn default() -> Self {
        FillSettings {
            method: "object".into(),
            range: "workArea".into(),
            alpha_expansion: 0.0,
            lighting_correction: "none".into(),
            reference_layer: None,
            reference_time: None,
        }
    }
}

fn lighting(s: &str) -> Option<f32> {
    Some(match s {
        "none" | "off" => 0.0,
        "subtle" => 0.35,
        "moderate" => 0.65,
        "strong" => 1.0,
        _ => return None,
    })
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "contentFill.set";
    let mut st = s.state.content_fill.clone();
    if let Some(m) = str_p(p, "method") {
        st.method = FillMethod::from_name(m).ok_or_else(|| bad(c, "method: object|surface|edgeBlend"))?.name().into();
    }
    if let Some(r) = str_p(p, "range") {
        st.range = match r {
            "workArea" | "work area" => "workArea".into(),
            "entire" | "entireDuration" | "layer" => "entire".into(),
            _ => return Err(bad(c, "range: workArea|entire")),
        };
    }
    if let Some(a) = f_p(p, "alphaExpansion") {
        st.alpha_expansion = a.clamp(0.0, 100.0);
    }
    if let Some(l) = str_p(p, "lightingCorrection") {
        lighting(l).ok_or_else(|| bad(c, "lightingCorrection: none|subtle|moderate|strong"))?;
        st.lighting_correction = l.into();
    }
    match p.get("referenceLayer") {
        Some(Value::Null) => st.reference_layer = None,
        Some(v) => {
            let comp = s.active_comp().ok_or(EngineError::NoComp)?;
            st.reference_layer = Some(super::resolve_layer(comp, v).ok_or_else(|| bad(c, format!("no layer {v}")))?);
            st.reference_time = Some(f_p(p, "referenceTime").unwrap_or(s.time().seconds()));
        }
        None => {
            if let Some(t) = f_p(p, "referenceTime") {
                st.reference_time = Some(t);
            }
        }
    }
    s.state.content_fill = st.clone();
    Ok(serde_json::to_value(st).unwrap_or_default())
}

/// A layer's masked source at comp time `t` on a `w × h` canvas in layer pixels (straight colour
/// with alpha 1 where kept), and the hole (`true` where the alpha is under one half).
fn masked_frame(r: &Renderer, ctx: &EvalCtx, layer: &effectcraft_project::Layer, w: u32, h: u32) -> (Image, Vec<bool>) {
    let mut img = Image::new(w, h);
    let mut hole = vec![true; (w * h) as usize];
    if let Some(buf) = r.layer_input(ctx, layer, 0) {
        let sc = if buf.scale > 0.0 { buf.scale } else { 1.0 };
        for y in 0..h {
            for x in 0..w {
                let (bx, by) = ((x as f64 + 0.5) * sc - buf.offset[0], (y as f64 + 0.5) * sc - buf.offset[1]);
                let p = if (sc - 1.0).abs() < 1e-9 { buf.img.get(bx.floor() as i64, by.floor() as i64) } else { buf.img.sample_bilinear(bx, by) };
                let i = (y * w + x) as usize;
                if p[3] >= 0.5 {
                    img.data[i] = [p[0] / p[3], p[1] / p[3], p[2] / p[3], 1.0];
                    hole[i] = false;
                }
            }
        }
    }
    (img, hole)
}

/// Encode a straight-alpha frame (alpha 1) as 8-bit PNG.
fn png(img: &Image) -> std::result::Result<Vec<u8>, String> {
    let rgba: Vec<u8> = img.data.iter().flat_map(|p| [p[0], p[1], p[2], p[3]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    super::comp_more::encode_png(&rgba, img.width, img.height)
}

fn default_dir(s: &Session, name: &str) -> String {
    let base = s
        .path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).parent().map(|d| d.join("Fill")))
        .unwrap_or_else(|| std::env::temp_dir().join("effectcraft-fill"));
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let mut k = 1;
    loop {
        let d = base.join(format!("{safe}_Fill_{k}"));
        if !d.exists() {
            return d.to_string_lossy().to_string();
        }
        k += 1;
    }
}

fn generate(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "contentFill.generate";
    let (cid, lid) = layer_p(s, p, c)?;
    // Parameters override the panel settings.
    let mut st = s.state.content_fill.clone();
    {
        let saved = s.state.content_fill.clone();
        let mut q = p.clone();
        if let Some(o) = q.as_object_mut() {
            o.retain(|k, _| ["method", "range", "alphaExpansion", "lightingCorrection", "referenceLayer", "referenceTime"].contains(&k.as_str()));
        }
        set(s, &q)?;
        std::mem::swap(&mut st, &mut s.state.content_fill);
        s.state.content_fill = saved;
    }
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let layer = comp.layer(lid).ok_or(EngineError::NoComp)?.clone();
    if !layer.source.is_av() || layer.source.item().is_none() {
        return Err(bad(c, "Content-Aware Fill needs a footage, solid or precomp layer"));
    }
    let (w, h) = effectcraft_render::source_size(&s.project, &layer);
    if w == 0 || h == 0 {
        return Err(bad(c, "the layer has no pixels"));
    }
    let fr = comp.frame_rate;
    let (a, b) =
        if st.range == "entire" { (layer.in_point, layer.out_point) } else { (comp.work_area.0.max(layer.in_point), comp.work_area.1.min(layer.out_point)) };
    let (a, b) = (a.max(Tick::ZERO), b.min(comp.duration));
    if b <= a {
        return Err(bad(c, "the range doesn't overlap the layer"));
    }
    let (fa, fb) = (fr.frame_at(fr.snap_nearest(a)), fr.frame_at(b - Tick(1)));
    let times: Vec<Tick> = (fa..=fb).map(|f| fr.tick_of(f)).collect();
    let opts = FillOpts {
        method: FillMethod::from_name(&st.method).unwrap_or_default(),
        lighting: lighting(&st.lighting_correction).unwrap_or(0.0),
        ..Default::default()
    };
    let expand = st.alpha_expansion.round().max(0.0) as usize;
    let reference = st.reference_layer.filter(|r| comp.layer(*r).is_some()).map(|r| (r, Tick::from_seconds_f64(st.reference_time.unwrap_or(0.0))));
    let dir = str_p(p, "outputDir").map(str::to_string).unwrap_or_else(|| default_dir(s, &layer.name));
    let lf = LayerFrames::new(s, cid, lid);
    let services = s.services.clone();
    let importer = s.importer.clone();
    let wait = b_p(p, "wait").unwrap_or(false);
    let name = layer.name.clone();
    s.spawn_task("contentFill", format!("Content-Aware Fill: {name}"), wait, move |ctl| {
        ctl.message("Reading frames");
        let project = lf.project.clone();
        let comp = project.comp(cid).ok_or("composition missing")?;
        let layer = comp.layer(lid).ok_or("layer missing")?;
        let mut r = Renderer::new(&project, lf.footage.as_ref(), RenderOpts::default());
        r.expr = lf.expr.as_deref();
        r.cache = Some(&lf.cache);
        let ctx0 = EvalCtx { expr: lf.expr.as_deref(), ..EvalCtx::new(&project, cid, comp, Tick::ZERO) };
        let n = times.len() as u64;
        let mut frames = Vec::with_capacity(times.len());
        let mut holes = Vec::with_capacity(times.len());
        for (k, t) in times.iter().enumerate() {
            let (img, hole) = masked_frame(&r, &ctx0.at(*t), layer, w, h);
            holes.push(dilate(&hole, w as usize, h as usize, expand));
            frames.push(img);
            if !ctl.progress(k as u64 + 1, n * 3) {
                return Err(crate::render_queue::CANCELLED.into());
            }
        }
        if !holes.iter().any(|h| h.iter().any(|b| *b)) {
            return Err("the layer has no transparent area to fill: draw a mask (Subtract) around what to remove".into());
        }
        let mut references = vec![];
        if let Some((rl, rt)) = reference
            && let Some(l) = comp.layer(rl)
        {
            let (img, hole) = masked_frame(&r, &ctx0.at(rt), l, w, h);
            if hole.iter().all(|b| !*b) {
                let k = times.iter().enumerate().min_by_key(|(_, t)| (t.0 - rt.0).abs()).map(|(k, _)| k).unwrap_or(0);
                references.push((k, img));
            }
        }
        ctl.message("Filling");
        let input = FillInput { frames, holes, references };
        let out = fill_sequence(&input, &opts, &mut |d, t| ctl.progress(n + (d as u64 * n / t.max(1) as u64), n * 3)).ok_or(crate::render_queue::CANCELLED)?;
        ctl.message("Writing the fill layer");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
        let mut paths = vec![];
        for (k, img) in out.iter().enumerate() {
            let path = std::path::Path::new(&dir).join(format!("fill_{k:05}.png")).to_string_lossy().to_string();
            services.write_file(&path, &png(img)?).map_err(|e| format!("{path}: {e}"))?;
            paths.push(path);
            if !ctl.progress(2 * n + k as u64 + 1, n * 3) {
                return Err(crate::render_queue::CANCELLED.into());
            }
        }
        let start = times[0];
        let count = times.len();
        let apply: Apply = Box::new(move |s: &mut Session| add_fill_layer(s, cid, lid, &name, paths, (w, h), start, count, importer));
        Ok(apply)
    })
}

/// Import the rendered sequence and put it in a layer above the source.
#[allow(clippy::too_many_arguments)]
fn add_fill_layer(
    s: &mut Session,
    cid: ItemId,
    lid: LayerId,
    name: &str,
    paths: Vec<String>,
    size: (u32, u32),
    start: Tick,
    count: usize,
    importer: Option<Arc<dyn crate::Importer>>,
) -> Result<Value> {
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let src = comp.layer(lid).ok_or_else(|| bad("contentFill.generate", "the source layer is gone"))?.clone();
    let fr = comp.frame_rate;
    let dur = fr.tick_of(count as i64);
    let mut footage = importer.and_then(|i| i.probe(&paths[0]).ok()).filter(|f| f.has_video).unwrap_or_else(|| Footage {
        path: paths[0].clone(),
        kind: FootageKind::Sequence,
        width: size.0,
        height: size.1,
        has_video: true,
        codec: "PNG".into(),
        ..Default::default()
    });
    footage.kind = FootageKind::Sequence;
    footage.sequence = paths.clone();
    footage.frame_rate = fr;
    footage.duration = dur;
    let (lname, item, layer) = s.edit("Content-Aware Fill", None, |proj, st| {
        let folder = proj.folder_named("Fill").unwrap_or_else(|| proj.add_item("Fill", Label::Yellow, None, ItemKind::Folder));
        let n = proj.items.values().filter(|i| i.parent == Some(folder)).count() + 1;
        let lname = format!("Fill {n} [{name}]");
        let iid = proj.add_item(&lname, Label::Lavender, Some(folder), ItemKind::Footage(footage));
        let mut l = build::layer(proj, &comp, &lname, LayerSource::Footage { item: iid }, size, Some(dur));
        l.start_time = start;
        l.in_point = start;
        l.out_point = (start + dur).min(comp.duration);
        l.parent = src.parent;
        l.switches.three_d = src.switches.three_d;
        if let (Some(dst), Some(t)) = (l.props.sub_mut("transform"), src.transform()) {
            let mut tr = t.clone();
            tr.reassign_uids(&mut proj.next_id);
            *dst = tr;
        }
        let id = l.id;
        let cm = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        let at = cm.layers.iter().position(|x| x.id == lid).unwrap_or(0);
        cm.layers.insert(at, l);
        st.selected_layers = vec![id];
        Ok((lname, iid, id))
    })?;
    Ok(json!({"layer": layer.0, "item": item.0, "name": lname, "frames": count, "files": paths}))
}

fn open_panel(s: &mut Session, p: &Value) -> Result<Value> {
    // Parameters given: generate straight away (agents); else show the panel.
    if p.as_object().is_some_and(|o| !o.is_empty()) {
        return generate(s, p);
    }
    s.events.push(crate::Event::Frontend { command: "window.panel".into(), params: json!({"panel": "contentAwareFill"}) });
    Ok(json!({"frontend": "window.panel"}))
}

pub fn specs() -> Vec<CommandSpec> {
    const P: &str = "{layer?, method?: object|surface|edgeBlend, range?: workArea|entire, alphaExpansion? (px), lightingCorrection?: none|subtle|moderate|strong, referenceLayer?, referenceTime? (s), outputDir?, wait?: bool}";
    vec![
        cmd!(
            "contentFill.set",
            "Content-Aware Fill Settings",
            [],
            None,
            "{method?: object|surface|edgeBlend, range?: workArea|entire, alphaExpansion? (px), lightingCorrection?: none|subtle|moderate|strong, referenceLayer?: layer|null, referenceTime? (s)}",
            always,
            set
        ),
        CommandSpec {
            id: "contentFill.generate",
            label: "Generate Fill Layer",
            menu: &[],
            shortcut: None,
            params: P,
            enabled: has_layers,
            run: generate,
            journal: true,
        },
        CommandSpec {
            id: "layer.newContentAwareFill",
            label: "Content-Aware Fill Layer...",
            menu: &["Layer", "New"],
            shortcut: None,
            params: P,
            enabled: always,
            run: open_panel,
            journal: true,
        },
    ]
}
