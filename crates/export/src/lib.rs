//! Render Queue jobs: render a composition's time span and write it through an output module.
//!
//! | Format | Container / files | Video | Alpha | Audio |
//! |---|---|---|---|---|
//! | H.264 | MP4 | FilmCraft H.264 High, 8-bit 4:2:0 BT.709 limited, VBR | no | AAC-LC (FilmCraft), stereo |
//! | ProRes | QuickTime MOV | FilmCraft ProRes 422 Proxy/LT/422/HQ, 10-bit; 4444/4444 XQ 4:4:4 | 4444 (16-bit alpha coding) | 16-bit PCM, stereo |
//! | PNG / JPEG / TIFF sequence | one file per frame | 8-bit | PNG, TIFF | — |
//! | OpenEXR sequence | one file per frame | 32-bit float, linear light, premultiplied | yes | — |
//! | Animated GIF | GIF89a | 256-colour palette per frame (NeuQuant) | 1-bit | — |
//! | WebM | WebM (Matroska) | VP9 profile 0 (`effectcraft-vp9enc`: key + inter frames, a key frame every 2 s, loop filter), 8-bit 4:2:0 | VP9 alpha (BlockAdditional) | Opus (`effectcraft-opusenc`), 48 kHz stereo |
//! | WAV / AIFF | RIFF WAVE / AIFF | — | — | 16-bit PCM stereo (audio only) |
//!
//! Frames are rendered in parallel batches (rayon; one batch ≈ one frame per core) and handed to
//! the encoder in order. Progress is reported after each batch through a callback that can cancel
//! the job by returning `false`. Codecs and the MP4/MOV muxer are FilmCraft's pure-Rust crates
//! (plan ADR 0001); nothing outside this crate touches them.
//!
//! H.264 requires even dimensions: odd output sizes are cropped by one pixel (padded when the
//! output is a single pixel wide/high).

mod encode;
mod out;
mod webm;

use web_time::Instant;

use effectcraft_project::render_queue::{AudioOutput, Channels, OutputFormat, OutputModule, RenderQuality, RenderSettings, sequence_path};
use effectcraft_project::{Comp, ItemId, Project};
use effectcraft_raster::Image;
use effectcraft_render::{ExprHost, FootageSource, RenderOpts, Renderer};
use effectcraft_time::Tick;
use rayon::prelude::*;

pub use effectcraft_project::render_queue;
pub use out::Sink;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("composition not found")]
    NoComp,
    #[error("cancelled")]
    Cancelled,
    #[error("i/o: {0}")]
    Io(String),
    #[error("encode: {0}")]
    Encode(String),
    #[error("{0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, ExportError>;

fn io(e: impl std::fmt::Display) -> ExportError {
    ExportError::Io(e.to_string())
}

/// One export: a comp, its Render Settings and Output Module, and the resolved output path.
pub struct Job<'a> {
    pub project: &'a Project,
    pub footage: &'a dyn FootageSource,
    pub expr: Option<&'a dyn ExprHost>,
    /// GPU compositor: used when the project's renderer is Mercury GPU Acceleration.
    pub accel: Option<&'a dyn effectcraft_render::Accelerator>,
    pub comp: ItemId,
    pub settings: &'a RenderSettings,
    pub output: &'a OutputModule,
    /// Output file. For image sequences a run of `#` (or `[#####]`) becomes the frame number; a
    /// path without one gets `_NNNNN` before the extension.
    pub path: &'a str,
    /// Hand finished files to this instead of writing them to disk (web downloads, tests).
    pub sink: Option<&'a Sink>,
}

/// Progress after a batch of frames.
#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub elapsed: f64,
}

impl Progress {
    pub fn fraction(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.done as f64 / self.total as f64 }
    }
    /// Estimated seconds remaining.
    pub fn remaining(&self) -> Option<f64> {
        (self.done > 0).then(|| self.elapsed / self.done as f64 * self.total.saturating_sub(self.done) as f64)
    }
}

/// What an export produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// The movie file, or the first file of a sequence.
    pub path: String,
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub seconds: f64,
    pub bytes: u64,
    pub audio: bool,
}

/// Formats this build can write (all of them; kept as the hook for optional encoders).
pub fn available_formats() -> Vec<OutputFormat> {
    OutputFormat::ALL.to_vec()
}

/// The size of the written frames for a job (H.264 rounds to even).
pub fn output_size(comp: &Comp, settings: &RenderSettings, output: &OutputModule) -> (u32, u32) {
    let (w, h) = settings.output_size(comp);
    output.format.coded_size(w, h)
}

/// Whether the job writes an audio track.
pub fn wants_audio(job: &Job) -> bool {
    job.output.format.supports_audio()
        && match job.output.audio {
            AudioOutput::On => true,
            AudioOutput::Off => false,
            AudioOutput::Auto => effectcraft_render::audio::comp_has_audio(job.project, job.comp),
        }
}

/// Run the export (blocking). `progress` is called after every batch; returning `false` cancels.
pub fn export(job: &Job, progress: &mut dyn FnMut(&Progress) -> bool) -> Result<Report> {
    let comp = job.project.comp(job.comp).ok_or(ExportError::NoComp)?;
    let t0 = Instant::now();
    let total = job.settings.frame_count(comp);
    let mut st = State { t0, total, done: 0, progress };
    st.advance(0)?;
    let (w, h) = output_size(comp, job.settings, job.output);
    let mut report = match job.output.format {
        OutputFormat::H264 | OutputFormat::ProRes => encode::movie(job, comp, w, h, &mut st)?,
        OutputFormat::Gif => gif_export(job, comp, w, h, &mut st)?,
        OutputFormat::WebM => webm::webm(job, comp, w, h, &mut st)?,
        OutputFormat::Wav => webm::audio_file(job, comp, false, &mut st)?,
        OutputFormat::Aiff => webm::audio_file(job, comp, true, &mut st)?,
        f if f.is_sequence() => sequence(job, comp, w, h, &mut st)?,
        f => return Err(ExportError::Unsupported(f.label().into())),
    };
    report.seconds = t0.elapsed().as_secs_f64();
    report.frames = total;
    (report.width, report.height) = (w, h);
    Ok(report)
}

pub(crate) struct State<'p> {
    t0: Instant,
    total: u64,
    done: u64,
    progress: &'p mut dyn FnMut(&Progress) -> bool,
}

impl State<'_> {
    fn snapshot(&self) -> Progress {
        Progress { done: self.done, total: self.total, elapsed: self.t0.elapsed().as_secs_f64() }
    }
    /// Count `n` more frames; `Err(Cancelled)` when the callback says stop.
    pub(crate) fn advance(&mut self, n: u64) -> Result<()> {
        self.done += n;
        let p = self.snapshot();
        if (self.progress)(&p) { Ok(()) } else { Err(ExportError::Cancelled) }
    }
}

/// Frames per parallel batch.
pub(crate) fn batch_size() -> u64 {
    rayon::current_num_threads().clamp(2, 16) as u64
}

/// Render output frame `i` of the job at the settings' resolution.
pub(crate) fn render_frame(job: &Job, comp: &Comp, i: u64) -> Image {
    let t: Tick = job.settings.frame_time(comp, i);
    let opts = RenderOpts {
        scale: job.settings.resolution.clamp(0.01, 4.0),
        motion_blur: job.settings.motion_blur,
        guides: false,
        draft: job.settings.quality == RenderQuality::Draft,
        // Output renders always look through the comp's active camera.
        view: None,
        backend: effectcraft_render::Backend::Auto,
        roi: None,
        proxy: job.settings.proxy_use,
    };
    let mut r = Renderer::new(job.project, job.footage, opts);
    r.expr = job.expr;
    r.accel = job.accel;
    r.comp_frame(job.comp, t)
}

/// 8-bit RGBA of the requested channels (RGB: over the comp background, opaque; RGBA: straight),
/// cropped/padded to `w`×`h`.
pub(crate) fn rgba8(img: &Image, comp: &Comp, channels: Channels, w: u32, h: u32) -> Vec<u8> {
    let px = match channels {
        Channels::Rgb => img.to_rgba8_over(comp.background),
        Channels::Rgba => img.to_rgba8(),
    };
    fit(px, img.width, img.height, w, h)
}

fn fit(px: Vec<u8>, iw: u32, ih: u32, w: u32, h: u32) -> Vec<u8> {
    if iw == w && ih == h {
        return px;
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h.min(ih) as usize {
        let n = w.min(iw) as usize * 4;
        out[y * w as usize * 4..y * w as usize * 4 + n].copy_from_slice(&px[y * iw as usize * 4..y * iw as usize * 4 + n]);
    }
    out
}

/// The comp frame number of output frame 0 (sequence numbering follows comp frames, like AE).
fn first_frame_number(job: &Job, comp: &Comp) -> i64 {
    job.settings.first_frame(comp)
}

fn sequence(job: &Job, comp: &Comp, w: u32, h: u32, st: &mut State) -> Result<Report> {
    let f0 = first_frame_number(job, comp);
    let fmt = job.output.format;
    let channels = if fmt.supports_alpha() { job.output.channels } else { Channels::Rgb };
    let batch = batch_size();
    let mut bytes = 0u64;
    let mut i = 0;
    while i < st.total {
        let end = (i + batch).min(st.total);
        let written: Vec<Result<u64>> = (i..end)
            .into_par_iter()
            .map(|k| {
                let path = sequence_path(job.path, f0 + k as i64);
                if job.settings.skip_existing && out::exists(job.sink, &path) {
                    return Ok(0);
                }
                let img = render_frame(job, comp, k);
                let mut f = out::create(job.sink, &path)?;
                write_still(&mut f, fmt, &img, comp, channels, w, h, job.output.quality)?;
                f.finish()
            })
            .collect();
        for r in written {
            bytes += r?;
        }
        st.advance(end - i)?;
        i = end;
    }
    Ok(Report { path: sequence_path(job.path, f0), frames: 0, width: w, height: h, seconds: 0.0, bytes, audio: false })
}

#[allow(clippy::too_many_arguments)]
fn write_still(file: &mut out::Out, fmt: OutputFormat, img: &Image, comp: &Comp, channels: Channels, w: u32, h: u32, quality: u8) -> Result<()> {
    use image::{ExtendedColorType, ImageEncoder, ImageFormat};
    match fmt {
        OutputFormat::ExrSequence => {
            // Linear light, premultiplied (the EXR convention); RGB keeps alpha = 1 over the background.
            let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            let mut data = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    let p = if x < img.width && y < img.height { img.data[img.idx(x, y)] } else { [0.0; 4] };
                    let (c, a) = match channels {
                        Channels::Rgba => {
                            let a = p[3].clamp(0.0, 1.0);
                            let s = |c: f32| if a > 0.0 { lin((c / a).clamp(0.0, 1.0)) * a } else { 0.0 };
                            ([s(p[0]), s(p[1]), s(p[2])], a)
                        }
                        Channels::Rgb => {
                            let k = 1.0 - p[3].clamp(0.0, 1.0);
                            let c = |i: usize| lin((p[i] + comp.background[i] * k).clamp(0.0, 1.0));
                            ([c(0), c(1), c(2)], 1.0)
                        }
                    };
                    data.extend_from_slice(&[c[0], c[1], c[2], a]);
                }
            }
            let buf = image::Rgba32FImage::from_raw(w, h, data).ok_or_else(|| ExportError::Encode("exr buffer".into()))?;
            image::DynamicImage::ImageRgba32F(buf).write_to(file, ImageFormat::OpenExr).map_err(|e| ExportError::Encode(e.to_string()))
        }
        OutputFormat::JpegSequence => {
            let px = rgba8(img, comp, Channels::Rgb, w, h);
            let rgb: Vec<u8> = px.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality.clamp(1, 100))
                .write_image(&rgb, w, h, ExtendedColorType::Rgb8)
                .map_err(|e| ExportError::Encode(e.to_string()))
        }
        OutputFormat::PngSequence | OutputFormat::TiffSequence => {
            let px = rgba8(img, comp, channels, w, h);
            let (data, ct) = match channels {
                Channels::Rgba => (px, ExtendedColorType::Rgba8),
                Channels::Rgb => (px.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect(), ExtendedColorType::Rgb8),
            };
            if fmt == OutputFormat::PngSequence {
                image::codecs::png::PngEncoder::new(file).write_image(&data, w, h, ct).map_err(|e| ExportError::Encode(e.to_string()))
            } else {
                // (the TIFF encoder needs Seek)
                image::codecs::tiff::TiffEncoder::new(file).write_image(&data, w, h, ct).map_err(|e| ExportError::Encode(e.to_string()))
            }
        }
        f => Err(ExportError::Unsupported(format!("{} is not a still format", f.label()))),
    }
}

fn gif_export(job: &Job, comp: &Comp, w: u32, h: u32, st: &mut State) -> Result<Report> {
    if w > u16::MAX as u32 || h > u16::MAX as u32 {
        return Err(ExportError::Unsupported("GIF frames are limited to 65535 px".into()));
    }
    let mut file = out::create(job.sink, job.path)?;
    let mut enc = gif::Encoder::new(&mut file, w as u16, h as u16, &[]).map_err(|e| ExportError::Encode(e.to_string()))?;
    enc.set_repeat(if job.output.gif_loop { gif::Repeat::Infinite } else { gif::Repeat::Finite(0) }).map_err(|e| ExportError::Encode(e.to_string()))?;
    let rate = job.settings.rate(comp);
    // GIF delays are in 1/100 s: carry the rounding error so long animations keep their timing.
    let cs_per_frame = 100.0 * rate.den as f64 / rate.num as f64;
    let mut clock = 0.0f64;
    let mut emitted = 0u64;
    let batch = batch_size();
    let mut i = 0;
    while i < st.total {
        let end = (i + batch).min(st.total);
        let frames: Vec<gif::Frame<'static>> = (i..end)
            .into_par_iter()
            .map(|k| {
                let img = render_frame(job, comp, k);
                let mut px = rgba8(&img, comp, job.output.channels, w, h);
                gif::Frame::from_rgba_speed(w as u16, h as u16, &mut px, 10)
            })
            .collect();
        for mut f in frames {
            clock += cs_per_frame;
            let d = (clock.round() as u64).saturating_sub(emitted).max(1);
            emitted += d;
            f.delay = d.min(u16::MAX as u64) as u16;
            if job.output.channels == Channels::Rgba {
                f.dispose = gif::DisposalMethod::Background;
            }
            enc.write_frame(&f).map_err(|e| ExportError::Encode(e.to_string()))?;
        }
        st.advance(end - i)?;
        i = end;
    }
    drop(enc);
    let bytes = file.finish()?;
    Ok(Report { path: job.path.to_string(), frames: 0, width: w, height: h, seconds: 0.0, bytes, audio: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_crops_and_pads() {
        let px: Vec<u8> = (0..3 * 3 * 4).map(|v| v as u8).collect();
        let c = fit(px.clone(), 3, 3, 2, 2);
        assert_eq!(c.len(), 16);
        assert_eq!(&c[0..8], &px[0..8]);
        assert_eq!(&c[8..16], &px[12..20]);
        let p = fit(px, 3, 3, 4, 4);
        assert_eq!(p.len(), 64);
        assert_eq!(&p[12..16], &[0, 0, 0, 0]);
        assert_eq!(OutputFormat::H264.coded_size(1, 101), (2, 100));
        assert_eq!(OutputFormat::Gif.coded_size(1, 101), (1, 101));
    }
}
