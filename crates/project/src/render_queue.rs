//! The Render Queue: compositions queued for export, each with Render Settings (what to render:
//! quality, resolution, time span, frame rate) and an Output Module (how to write it: format,
//! channels, codec options, output path). Saved with the project, like After Effects.
//!
//! Output paths are templates: `[compName].[fileExtension]` (the default), with the tokens
//! `[compName]`, `[projectName]`, `[fileExtension]`, `[width]`, `[height]`, `[frameRate]`,
//! `[startFrame]`, `[endFrame]`, `[outputModuleName]` and, for image sequences, a run of `#`
//! (`[#####]`) that becomes the zero-padded frame number. A relative result is resolved against the
//! project's folder (or the working directory for unsaved projects).

use effectcraft_time::{FrameRate, Tick};
use serde::{Deserialize, Serialize};

use crate::{Comp, ItemId};

/// The default output path template.
pub const DEFAULT_TEMPLATE: &str = "[compName].[fileExtension]";
/// The default template for image sequences.
pub const DEFAULT_SEQUENCE_TEMPLATE: &str = "[compName]_[#####].[fileExtension]";

/// Render Settings ▸ Quality.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderQuality {
    #[default]
    Best,
    Draft,
}

/// Render Settings ▸ Time Span.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TimeSpan {
    /// The composition's work area.
    #[default]
    WorkArea,
    /// The whole composition.
    LengthOfComp,
    /// An explicit span `[start, end)` in comp time.
    Custom { start: Tick, end: Tick },
}

/// Render Settings ▸ Proxy Use: which proxies a render uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProxyUse {
    /// Each item's Use Proxy switch (what the viewer shows).
    #[default]
    CurrentSettings,
    /// Every item that has a proxy uses it.
    UseAll,
    /// Only compositions' proxies.
    UseCompOnly,
    /// No proxies (full-resolution sources).
    UseNone,
}

impl ProxyUse {
    pub const ALL: [ProxyUse; 4] = [ProxyUse::CurrentSettings, ProxyUse::UseAll, ProxyUse::UseCompOnly, ProxyUse::UseNone];
    pub fn label(self) -> &'static str {
        match self {
            ProxyUse::CurrentSettings => "Current Settings",
            ProxyUse::UseAll => "Use All Proxies",
            ProxyUse::UseCompOnly => "Use Comp Proxies Only",
            ProxyUse::UseNone => "Use No Proxies",
        }
    }
    pub fn parse(s: &str) -> Option<ProxyUse> {
        match s.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
            "current" | "currentsettings" => Some(ProxyUse::CurrentSettings),
            "all" | "useall" | "useallproxies" => Some(ProxyUse::UseAll),
            "comp" | "componly" | "usecompproxiesonly" | "usecomponly" => Some(ProxyUse::UseCompOnly),
            "none" | "usenone" | "usenoproxies" => Some(ProxyUse::UseNone),
            _ => None,
        }
    }
    /// Whether an item's proxy is used: `comp` = the item is a composition, `enabled` = its Use
    /// Proxy switch.
    pub fn uses(self, comp: bool, enabled: bool) -> bool {
        match self {
            ProxyUse::CurrentSettings => enabled,
            ProxyUse::UseAll => true,
            ProxyUse::UseCompOnly => comp,
            ProxyUse::UseNone => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderSettings {
    pub quality: RenderQuality,
    /// Output scale (1 = Full, 0.5 = Half, 1/3 = Third, 0.25 = Quarter).
    pub resolution: f64,
    pub time_span: TimeSpan,
    /// `None` = use the comp's frame rate.
    pub frame_rate: Option<FrameRate>,
    /// Motion blur for layers with the switch on (and the comp's Enable Motion Blur).
    pub motion_blur: bool,
    /// Image sequences: skip frames whose file already exists.
    pub skip_existing: bool,
    /// Proxy Use (Best Settings: Use No Proxies).
    pub proxy_use: ProxyUse,
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings {
            quality: RenderQuality::Best,
            resolution: 1.0,
            time_span: TimeSpan::WorkArea,
            frame_rate: None,
            motion_blur: true,
            skip_existing: false,
            proxy_use: ProxyUse::UseNone,
        }
    }
}

impl RenderSettings {
    /// The comp-time span `[start, end)` to render.
    pub fn span(&self, comp: &Comp) -> (Tick, Tick) {
        let (a, b) = match self.time_span {
            TimeSpan::WorkArea => comp.work_area,
            TimeSpan::LengthOfComp => (Tick::ZERO, comp.duration),
            TimeSpan::Custom { start, end } => (start, end),
        };
        let a = a.clamp(Tick::ZERO, comp.duration);
        let b = b.clamp(a, comp.duration);
        (a, b)
    }
    pub fn rate(&self, comp: &Comp) -> FrameRate {
        self.frame_rate.unwrap_or(comp.frame_rate)
    }
    /// First output-rate frame starting at or after `t`.
    fn ceil_frame(r: FrameRate, t: Tick) -> i64 {
        let f = r.frame_at(t);
        if r.tick_of(f) < t { f + 1 } else { f }
    }
    /// Output-rate frame number of output frame 0 (the first frame starting inside the span).
    pub fn first_frame(&self, comp: &Comp) -> i64 {
        Self::ceil_frame(self.rate(comp), self.span(comp).0)
    }
    /// Number of output frames: those starting in `[start, end)` (at least 1).
    pub fn frame_count(&self, comp: &Comp) -> u64 {
        let (_, b) = self.span(comp);
        let n = Self::ceil_frame(self.rate(comp), b) - self.first_frame(comp);
        n.max(1) as u64
    }
    /// Comp time of output frame `i`.
    pub fn frame_time(&self, comp: &Comp, i: u64) -> Tick {
        self.rate(comp).tick_of(self.first_frame(comp) + i as i64)
    }
    pub fn output_size(&self, comp: &Comp) -> (u32, u32) {
        let s = self.resolution.clamp(0.01, 4.0);
        (((comp.width as f64 * s).round() as u32).max(1), ((comp.height as f64 * s).round() as u32).max(1))
    }
    pub fn resolution_label(&self) -> &'static str {
        match self.resolution {
            r if (r - 1.0).abs() < 1e-6 => "Full",
            r if (r - 0.5).abs() < 1e-6 => "Half",
            r if (r - 1.0 / 3.0).abs() < 1e-3 => "Third",
            r if (r - 0.25).abs() < 1e-6 => "Quarter",
            _ => "Custom",
        }
    }
    /// One-line summary shown next to "Render Settings:" (AE shows the template name).
    pub fn summary(&self) -> String {
        let q = if self.quality == RenderQuality::Best { "Best Settings" } else { "Draft Settings" };
        format!("{q} · {}", self.resolution_label())
    }
}

/// Output Module ▸ Format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OutputFormat {
    /// H.264 video (+ AAC audio) in MP4.
    #[default]
    H264,
    /// Apple ProRes in QuickTime (+ PCM audio). 4444 when channels include alpha.
    ProRes,
    PngSequence,
    JpegSequence,
    TiffSequence,
    /// OpenEXR sequence (32-bit float, linear light).
    ExrSequence,
    /// Animated GIF.
    Gif,
    /// WebM: VP9 video (alpha with RGB + Alpha) + Opus audio.
    WebM,
    /// Audio only: WAV (16-bit PCM).
    Wav,
    /// Audio only: AIFF (16-bit PCM).
    Aiff,
    /// HEVC (H.265) video (+ AAC audio) in MP4 (`hvc1`).
    Hevc,
    /// AV1 video (+ AAC audio) in MP4 (`av01`). WebM with AV1: [`OutputFormat::WebM`] with
    /// [`WebmVideoCodec::Av1`].
    Av1,
}

impl OutputFormat {
    pub const ALL: [OutputFormat; 12] = [
        OutputFormat::H264,
        OutputFormat::Hevc,
        OutputFormat::Av1,
        OutputFormat::ProRes,
        OutputFormat::WebM,
        OutputFormat::PngSequence,
        OutputFormat::JpegSequence,
        OutputFormat::TiffSequence,
        OutputFormat::ExrSequence,
        OutputFormat::Gif,
        OutputFormat::Wav,
        OutputFormat::Aiff,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OutputFormat::H264 => "H.264",
            OutputFormat::ProRes => "QuickTime (ProRes)",
            OutputFormat::PngSequence => "PNG Sequence",
            OutputFormat::JpegSequence => "JPEG Sequence",
            OutputFormat::TiffSequence => "TIFF Sequence",
            OutputFormat::ExrSequence => "OpenEXR Sequence",
            OutputFormat::Gif => "Animated GIF",
            OutputFormat::WebM => "WebM (VP9 + Opus)",
            OutputFormat::Wav => "WAV",
            OutputFormat::Aiff => "AIFF",
            OutputFormat::Hevc => "HEVC (H.265)",
            OutputFormat::Av1 => "AV1",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::H264 => "mp4",
            OutputFormat::ProRes => "mov",
            OutputFormat::PngSequence => "png",
            OutputFormat::JpegSequence => "jpg",
            OutputFormat::TiffSequence => "tif",
            OutputFormat::ExrSequence => "exr",
            OutputFormat::Gif => "gif",
            OutputFormat::WebM => "webm",
            OutputFormat::Wav => "wav",
            OutputFormat::Aiff => "aif",
            OutputFormat::Hevc | OutputFormat::Av1 => "mp4",
        }
    }
    pub fn is_sequence(self) -> bool {
        matches!(self, OutputFormat::PngSequence | OutputFormat::JpegSequence | OutputFormat::TiffSequence | OutputFormat::ExrSequence)
    }
    pub fn is_movie(self) -> bool {
        matches!(self, OutputFormat::H264 | OutputFormat::ProRes | OutputFormat::WebM | OutputFormat::Hevc | OutputFormat::Av1)
    }
    /// Formats whose video codec options live in [`OutputModule::codec`] (HEVC, AV1).
    pub fn has_codec_options(self) -> bool {
        matches!(self, OutputFormat::Hevc | OutputFormat::Av1)
    }
    /// Audio-only outputs (no video is rendered).
    pub fn is_audio_only(self) -> bool {
        matches!(self, OutputFormat::Wav | OutputFormat::Aiff)
    }
    pub fn supports_alpha(self) -> bool {
        !matches!(self, OutputFormat::H264 | OutputFormat::Hevc | OutputFormat::Av1 | OutputFormat::JpegSequence | OutputFormat::Wav | OutputFormat::Aiff)
    }
    pub fn supports_audio(self) -> bool {
        self.is_movie() || self.is_audio_only()
    }
    /// The written frame size for a rendered size: H.264 and HEVC need even dimensions (rounded down,
    /// at least 2).
    pub fn coded_size(self, w: u32, h: u32) -> (u32, u32) {
        let even = |v: u32| if v < 2 { 2 } else { v & !1 };
        if matches!(self, OutputFormat::H264 | OutputFormat::Hevc) { (even(w), even(h)) } else { (w, h) }
    }
    /// Parse a format name: `h264`, `mp4`, `prores`, `mov`, `png`, `jpeg`/`jpg`, `tiff`/`tif`,
    /// `exr`, `gif`, or a label.
    pub fn from_name(s: &str) -> Option<OutputFormat> {
        let n = s.trim().to_ascii_lowercase().replace([' ', '.', '-', '_'], "");
        Some(match n.as_str() {
            "h264" | "mp4" | "avc" => OutputFormat::H264,
            "hevc" | "h265" | "hevch265" | "hvc1" | "x265" => OutputFormat::Hevc,
            "av1" | "av01" | "mp4av1" => OutputFormat::Av1,
            "prores" | "mov" | "quicktime" | "quicktimeprores" => OutputFormat::ProRes,
            "png" | "pngsequence" => OutputFormat::PngSequence,
            "jpg" | "jpeg" | "jpegsequence" => OutputFormat::JpegSequence,
            "tif" | "tiff" | "tiffsequence" => OutputFormat::TiffSequence,
            "exr" | "openexr" | "openexrsequence" => OutputFormat::ExrSequence,
            "gif" | "animatedgif" => OutputFormat::Gif,
            "webm" | "vp9" | "webmvp9opus" => OutputFormat::WebM,
            "wav" | "wave" => OutputFormat::Wav,
            "aif" | "aiff" | "aifc" => OutputFormat::Aiff,
            _ => return None,
        })
    }
    /// Guess from a file name's extension.
    pub fn from_path(path: &str) -> Option<OutputFormat> {
        let ext = std::path::Path::new(path).extension()?.to_string_lossy().to_ascii_lowercase();
        OutputFormat::from_name(&ext)
    }
}

/// Output Module ▸ Channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channels {
    /// Opaque, composited over the comp background colour.
    #[default]
    Rgb,
    /// With straight (unmatted) alpha.
    Rgba,
}

/// Output Module ▸ Audio Output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioOutput {
    /// On when the comp has audible layers.
    #[default]
    Auto,
    On,
    Off,
}

/// ProRes flavour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProResProfile {
    Proxy,
    Lt,
    Standard,
    #[default]
    Hq,
    P4444,
    P4444Xq,
}

impl ProResProfile {
    pub const ALL: [ProResProfile; 6] =
        [ProResProfile::Proxy, ProResProfile::Lt, ProResProfile::Standard, ProResProfile::Hq, ProResProfile::P4444, ProResProfile::P4444Xq];
    pub fn label(self) -> &'static str {
        match self {
            ProResProfile::Proxy => "Apple ProRes 422 Proxy",
            ProResProfile::Lt => "Apple ProRes 422 LT",
            ProResProfile::Standard => "Apple ProRes 422",
            ProResProfile::Hq => "Apple ProRes 422 HQ",
            ProResProfile::P4444 => "Apple ProRes 4444",
            ProResProfile::P4444Xq => "Apple ProRes 4444 XQ",
        }
    }
    pub fn is_4444(self) -> bool {
        matches!(self, ProResProfile::P4444 | ProResProfile::P4444Xq)
    }
    pub fn from_name(s: &str) -> Option<ProResProfile> {
        let n = s.to_ascii_lowercase().replace([' ', '-', '_'], "");
        ProResProfile::ALL.into_iter().find(|p| {
            let l = p.label().to_ascii_lowercase().replace(' ', "");
            l == n || l.trim_start_matches("appleprores") == n.trim_start_matches("appleprores") || format!("{p:?}").to_ascii_lowercase() == n
        })
    }
}

/// HEVC / AV1 profile: 8-bit (HEVC Main, AV1 Main 8-bit) or 10-bit (HEVC Main 10, AV1 Main 10-bit).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodecProfile {
    #[default]
    Main,
    Main10,
}

impl CodecProfile {
    pub const ALL: [CodecProfile; 2] = [CodecProfile::Main, CodecProfile::Main10];
    pub fn label(self) -> &'static str {
        match self {
            CodecProfile::Main => "Main",
            CodecProfile::Main10 => "Main 10",
        }
    }
    pub fn bit_depth(self) -> u8 {
        match self {
            CodecProfile::Main => 8,
            CodecProfile::Main10 => 10,
        }
    }
    pub fn from_name(s: &str) -> Option<CodecProfile> {
        match s.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "main" | "main8" | "8" | "8bit" => Some(CodecProfile::Main),
            "main10" | "10" | "10bit" => Some(CodecProfile::Main10),
            _ => None,
        }
    }
}

/// HEVC / AV1 rate control: a target bitrate ([`OutputModule::bitrate_kbps`]) or constant
/// quality ([`VideoCodecOptions::quality`], CRF-like).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RateControlMode {
    #[default]
    Bitrate,
    Quality,
}

impl RateControlMode {
    pub fn label(self) -> &'static str {
        match self {
            RateControlMode::Bitrate => "Target Bitrate",
            RateControlMode::Quality => "Constant Quality",
        }
    }
    pub fn from_name(s: &str) -> Option<RateControlMode> {
        match s.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "bitrate" | "vbr" | "abr" | "targetbitrate" => Some(RateControlMode::Bitrate),
            "quality" | "crf" | "cq" | "constantquality" | "qp" => Some(RateControlMode::Quality),
            _ => None,
        }
    }
}

/// Video codec options of the HEVC and AV1 formats (and AV1 in WebM).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoCodecOptions {
    pub profile: CodecProfile,
    /// Level × 10 (`41` = level 4.1); `None` picks the lowest level that fits the frame size and rate.
    pub level: Option<u8>,
    pub rate_control: RateControlMode,
    /// Constant-quality setting 1–100 (100 ≈ visually lossless), used with [`RateControlMode::Quality`].
    pub quality: u8,
    /// Frames between key frames; 0 = automatic (about 2 seconds).
    pub keyframe_interval: u32,
}

impl Default for VideoCodecOptions {
    fn default() -> Self {
        VideoCodecOptions { profile: CodecProfile::Main, level: None, rate_control: RateControlMode::Bitrate, quality: 70, keyframe_interval: 0 }
    }
}

impl VideoCodecOptions {
    /// HEVC levels (× 10) offered in the UI (Main tier).
    pub const HEVC_LEVELS: [u8; 13] = [10, 20, 21, 30, 31, 40, 41, 50, 51, 52, 60, 61, 62];
    /// AV1 levels (× 10) offered in the UI.
    pub const AV1_LEVELS: [u8; 14] = [20, 21, 30, 31, 40, 41, 50, 51, 52, 53, 60, 61, 62, 63];
    /// The key-frame interval in frames at `fps` (automatic: 2 s).
    pub fn keyint(&self, fps: f64) -> u32 {
        if self.keyframe_interval > 0 { self.keyframe_interval } else { (fps * 2.0).round().max(1.0) as u32 }
    }
    /// HEVC QP (0–51) for the constant-quality setting.
    pub fn hevc_qp(&self) -> u8 {
        (4.0 + (100 - self.quality.clamp(1, 100)) as f64 * 0.45).round() as u8
    }
    /// AV1 `base_q_idx` (1–255) for the constant-quality setting.
    pub fn av1_qindex(&self) -> u8 {
        (4.0 + (100 - self.quality.clamp(1, 100)) as f64 * 2.4).round() as u8
    }
    /// `general_level_idc` (30 × level).
    pub fn hevc_level_idc(&self) -> Option<u8> {
        self.level.map(|l| l.saturating_mul(3))
    }
    /// AV1 `seq_level_idx`: (major − 2) × 4 + minor.
    pub fn av1_level_idx(&self) -> Option<u8> {
        self.level.map(|l| ((l / 10).clamp(2, 7) - 2) * 4 + (l % 10).min(3))
    }
    pub fn level_label(&self) -> String {
        match self.level {
            None => "Auto".into(),
            Some(l) => format!("{}.{}", l / 10, l % 10),
        }
    }
    /// Parse `"auto"`, `"4.1"`, `"41"` (level × 10). `None` when unparsable; `Some(None)` = auto.
    pub fn parse_level(s: &str) -> Option<Option<u8>> {
        let s = s.trim().to_ascii_lowercase();
        if s == "auto" || s.is_empty() {
            return Some(None);
        }
        let v: f64 = s.trim_start_matches('l').parse().ok()?;
        let l = if v >= 10.0 { v.round() } else { (v * 10.0).round() };
        (10.0..=73.0).contains(&l).then_some(Some(l as u8))
    }
}

/// The video codec of a WebM output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebmVideoCodec {
    #[default]
    Vp9,
    /// AV1 (`V_AV1`) with the [`OutputModule::codec`] options; no alpha.
    Av1,
}

impl WebmVideoCodec {
    pub fn label(self) -> &'static str {
        match self {
            WebmVideoCodec::Vp9 => "VP9",
            WebmVideoCodec::Av1 => "AV1",
        }
    }
}

/// Opus coding application: general audio (music and mixed content) or voice (speech-tuned
/// SILK / hybrid coding at low bitrates).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpusApplication {
    #[default]
    Audio,
    Voip,
}

impl OpusApplication {
    pub fn label(self) -> &'static str {
        match self {
            OpusApplication::Audio => "Audio",
            OpusApplication::Voip => "Voice",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputModule {
    pub format: OutputFormat,
    pub channels: Channels,
    /// Output path or template (see the module docs).
    pub output: String,
    /// JPEG quality 1–100.
    pub quality: u8,
    /// H.264 target bitrate (also HEVC / AV1 with [`RateControlMode::Bitrate`]).
    pub bitrate_kbps: u32,
    pub prores_profile: ProResProfile,
    pub audio: AudioOutput,
    pub audio_sample_rate: u32,
    /// GIF: loop forever.
    pub gif_loop: bool,
    /// HEVC / AV1 options.
    pub codec: VideoCodecOptions,
    /// WebM video codec.
    pub webm_codec: WebmVideoCodec,
    /// WebM Opus audio bitrate (stereo total), kbit/s.
    pub opus_bitrate_kbps: u32,
    pub opus_application: OpusApplication,
}

impl Default for OutputModule {
    fn default() -> Self {
        OutputModule {
            format: OutputFormat::H264,
            channels: Channels::Rgb,
            output: DEFAULT_TEMPLATE.into(),
            quality: 90,
            bitrate_kbps: 10_000,
            prores_profile: ProResProfile::Hq,
            audio: AudioOutput::Auto,
            audio_sample_rate: 48_000,
            gif_loop: true,
            codec: VideoCodecOptions::default(),
            webm_codec: WebmVideoCodec::Vp9,
            opus_bitrate_kbps: 192,
            opus_application: OpusApplication::Audio,
        }
    }
}

impl OutputModule {
    pub fn for_format(format: OutputFormat) -> OutputModule {
        let mut m = OutputModule { format, ..Default::default() };
        if format.is_sequence() {
            m.output = DEFAULT_SEQUENCE_TEMPLATE.into();
        }
        m
    }
    /// Change the format, keeping the output name but fixing its extension / frame-number token.
    pub fn set_format(&mut self, format: OutputFormat) {
        let old = self.format;
        self.format = format;
        if !self.supports_alpha() {
            self.channels = Channels::Rgb;
        }
        if self.output.contains("[fileExtension]") {
            if format.is_sequence() && !self.output.contains('#') {
                self.output = self.output.replace(".[fileExtension]", "_[#####].[fileExtension]");
            } else if !format.is_sequence() && old.is_sequence() {
                self.output = self.output.replace("_[#####]", "").replace("[#####]", "");
            }
            return;
        }
        let p = std::path::Path::new(&self.output);
        let mut stem = p.with_extension("").to_string_lossy().to_string();
        if format.is_sequence() && !stem.contains('#') {
            stem.push_str("_[#####]");
        } else if !format.is_sequence() {
            stem = stem.replace("_[#####]", "").replace("[#####]", "");
        }
        self.output = format!("{stem}.{}", format.extension());
    }
    /// Whether this module can write alpha (the format can, and WebM uses VP9).
    pub fn supports_alpha(&self) -> bool {
        self.format.supports_alpha() && !(self.format == OutputFormat::WebM && self.webm_codec == WebmVideoCodec::Av1)
    }
    /// Codec options summary: `Main · 10000 kbps · Level Auto · Key every auto`.
    fn codec_summary(&self) -> String {
        let c = &self.codec;
        let rate = match c.rate_control {
            RateControlMode::Bitrate => format!("{} kbps", self.bitrate_kbps),
            RateControlMode::Quality => format!("Quality {}", c.quality),
        };
        let gop = if c.keyframe_interval == 0 { "auto".to_string() } else { c.keyframe_interval.to_string() };
        format!("{} · {rate} · Level {} · Key every {gop}", c.profile.label(), c.level_label())
    }
    /// One-line summary shown next to "Output Module:".
    pub fn summary(&self) -> String {
        let ch = match self.channels {
            Channels::Rgb => "RGB",
            Channels::Rgba => "RGB + Alpha",
        };
        match self.format {
            OutputFormat::H264 => format!("H.264 · {} kbps", self.bitrate_kbps),
            OutputFormat::ProRes => format!(
                "{} · {ch}",
                if self.channels == Channels::Rgba && !self.prores_profile.is_4444() { "Apple ProRes 4444" } else { self.prores_profile.label() }
            ),
            OutputFormat::Hevc | OutputFormat::Av1 => format!("{} · {}", self.format.label(), self.codec_summary()),
            OutputFormat::WebM if self.webm_codec == WebmVideoCodec::Av1 => {
                format!("WebM (AV1 + Opus {} kbps) · {}", self.opus_bitrate_kbps, self.codec_summary())
            }
            f => format!("{} · {ch}", f.label()),
        }
    }
}

/// Render Queue item status (the Status column).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum RenderStatus {
    /// Render checkbox off.
    Unqueued,
    #[default]
    Queued,
    NeedsOutput,
    Rendering,
    Done,
    UserStopped,
    Failed(String),
}

impl RenderStatus {
    pub fn label(&self) -> &str {
        match self {
            RenderStatus::Unqueued => "Unqueued",
            RenderStatus::Queued => "Queued",
            RenderStatus::NeedsOutput => "Needs Output",
            RenderStatus::Rendering => "Rendering",
            RenderStatus::Done => "Done",
            RenderStatus::UserStopped => "User Stopped",
            RenderStatus::Failed(_) => "Failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderQueueItem {
    /// Stable id (unique within the queue).
    pub id: u64,
    pub comp: ItemId,
    /// The Render checkbox.
    pub render: bool,
    #[serde(default)]
    pub status: RenderStatus,
    #[serde(default)]
    pub settings: RenderSettings,
    #[serde(default)]
    pub output: OutputModule,
    /// Unix seconds when the last render of this item started.
    #[serde(default)]
    pub started: Option<u64>,
    /// Seconds the last render took.
    #[serde(default)]
    pub render_time: Option<f64>,
    /// Resolved path of the last render (first file for sequences).
    #[serde(default)]
    pub last_output: Option<String>,
    /// Further Output Modules (Composition ▸ Add Output Module): the same frames are encoded
    /// once per module.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_outputs: Vec<OutputModule>,
    /// Post-Render Action of the first output module.
    #[serde(default, skip_serializing_if = "PostRenderAction::is_none")]
    pub post_render: PostRenderAction,
}

/// What happens after an item renders (Output Module ▸ Post-Render Action).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostRenderAction {
    #[default]
    None,
    /// Import the rendered file into the project.
    Import,
    /// Import it and replace every use of the rendered composition (Composition ▸ Pre-render).
    ImportAndReplace,
    /// Set the rendered file as the composition's proxy (File ▸ Create Proxy).
    SetProxy,
}

impl PostRenderAction {
    pub fn is_none(&self) -> bool {
        *self == PostRenderAction::None
    }
    pub fn label(self) -> &'static str {
        match self {
            PostRenderAction::None => "None",
            PostRenderAction::Import => "Import",
            PostRenderAction::ImportAndReplace => "Import & Replace Usage",
            PostRenderAction::SetProxy => "Set Proxy",
        }
    }
}

impl RenderQueueItem {
    pub fn new(id: u64, comp: ItemId) -> RenderQueueItem {
        RenderQueueItem {
            id,
            comp,
            render: true,
            status: RenderStatus::Queued,
            settings: RenderSettings::default(),
            output: OutputModule::default(),
            started: None,
            render_time: None,
            last_output: None,
            extra_outputs: vec![],
            post_render: PostRenderAction::None,
        }
    }
    /// Every output module, the first one first.
    pub fn output_modules(&self) -> impl Iterator<Item = &OutputModule> {
        std::iter::once(&self.output).chain(self.extra_outputs.iter())
    }
    /// Will be rendered by the next Render.
    pub fn is_queued(&self) -> bool {
        self.render && matches!(self.status, RenderStatus::Queued)
    }
}

/// Values for template tokens.
pub struct TemplateVars<'a> {
    pub comp_name: &'a str,
    pub project_name: &'a str,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub start_frame: i64,
    pub end_frame: i64,
}

/// Expand template tokens (all but the `#` frame-number run).
pub fn expand_template(template: &str, format: OutputFormat, v: &TemplateVars) -> String {
    let fps = if (v.frame_rate - v.frame_rate.round()).abs() < 1e-6 { format!("{}", v.frame_rate.round() as i64) } else { format!("{:.2}", v.frame_rate) };
    template
        .replace("[compName]", &sanitize(v.comp_name))
        .replace("[projectName]", &sanitize(v.project_name))
        .replace("[fileExtension]", format.extension())
        .replace("[width]", &v.width.to_string())
        .replace("[height]", &v.height.to_string())
        .replace("[frameRate]", &fps)
        .replace("[startFrame]", &v.start_frame.to_string())
        .replace("[endFrame]", &v.end_frame.to_string())
        .replace("[outputModuleName]", format.label())
}

/// Replace the first run of `#` (optionally in brackets, `[####]`) with the zero-padded frame
/// number. Paths without a run get `_NNNNN` before the extension.
pub fn sequence_path(path: &str, frame: i64) -> String {
    if let Some(start) = path.find('#') {
        let len = path[start..].chars().take_while(|c| *c == '#').count();
        let (mut a, mut b) = (start, start + len);
        if a > 0 && path.as_bytes()[a - 1] == b'[' && path.as_bytes().get(b) == Some(&b']') {
            a -= 1;
            b += 1;
        }
        return format!("{}{:0width$}{}", &path[..a], frame, &path[b..], width = len);
    }
    let p = std::path::Path::new(path);
    match p.extension() {
        Some(ext) => format!("{}_{frame:05}.{}", p.with_extension("").to_string_lossy(), ext.to_string_lossy()),
        None => format!("{path}_{frame:05}"),
    }
}

/// Make a name safe as a file name component.
pub fn sanitize(name: &str) -> String {
    name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_and_sequences() {
        let v = TemplateVars { comp_name: "Main/Title", project_name: "P", width: 1920, height: 1080, frame_rate: 29.97, start_frame: 0, end_frame: 9 };
        assert_eq!(expand_template(DEFAULT_TEMPLATE, OutputFormat::H264, &v), "Main_Title.mp4");
        assert_eq!(expand_template("[compName]_[width]x[height]@[frameRate].[fileExtension]", OutputFormat::Gif, &v), "Main_Title_1920x1080@29.97.gif");
        assert_eq!(sequence_path("out/a_[#####].png", 42), "out/a_00042.png");
        assert_eq!(sequence_path("a###.tif", 7), "a007.tif");
        assert_eq!(sequence_path("a.png", 3), "a_00003.png");
    }

    #[test]
    fn set_format_fixes_names() {
        let mut m = OutputModule::default();
        m.set_format(OutputFormat::PngSequence);
        assert_eq!(m.output, "[compName]_[#####].[fileExtension]");
        m.set_format(OutputFormat::ProRes);
        assert_eq!(m.output, DEFAULT_TEMPLATE);
        m.output = "/tmp/x.mov".into();
        m.set_format(OutputFormat::JpegSequence);
        assert_eq!(m.output, "/tmp/x_[#####].jpg");
        assert_eq!(OutputFormat::from_name("TIFF"), Some(OutputFormat::TiffSequence));
        assert_eq!(ProResProfile::from_name("4444"), Some(ProResProfile::P4444));
        assert_eq!(ProResProfile::from_name("hq"), Some(ProResProfile::Hq));
    }

    #[test]
    fn spans() {
        let mut c = Comp::new(100, 50, FrameRate::new(25, 1), Tick::from_seconds_f64(4.0));
        c.work_area = (Tick::from_seconds_f64(1.0), Tick::from_seconds_f64(2.0));
        let mut s = RenderSettings::default();
        assert_eq!(s.frame_count(&c), 25);
        assert_eq!(s.frame_time(&c, 0), Tick::from_seconds_f64(1.0));
        s.time_span = TimeSpan::LengthOfComp;
        assert_eq!(s.frame_count(&c), 100);
        s.frame_rate = Some(FrameRate::new(10, 1));
        assert_eq!(s.frame_count(&c), 40);
        s.resolution = 0.5;
        assert_eq!(s.output_size(&c), (50, 25));
        // NTSC: 1 s – 2 s holds the 30 frames that start inside it.
        let c = Comp::new(100, 50, FrameRate::new(30000, 1001), Tick::from_seconds_f64(4.0));
        let s = RenderSettings { time_span: TimeSpan::Custom { start: Tick::from_seconds_f64(1.0), end: Tick::from_seconds_f64(2.0) }, ..Default::default() };
        assert_eq!(s.frame_count(&c), 30);
        assert_eq!(s.first_frame(&c), 30);
        assert!(s.frame_time(&c, 0) >= Tick::from_seconds_f64(1.0));
    }

    #[test]
    fn hevc_av1_codec_options() {
        assert_eq!(OutputFormat::from_name("H.265"), Some(OutputFormat::Hevc));
        assert_eq!(OutputFormat::from_name("hevc"), Some(OutputFormat::Hevc));
        assert_eq!(OutputFormat::from_name("AV1"), Some(OutputFormat::Av1));
        assert_eq!(OutputFormat::Hevc.coded_size(33, 17), (32, 16));
        assert_eq!(OutputFormat::Av1.coded_size(33, 17), (33, 17));
        assert!(!OutputFormat::Hevc.supports_alpha() && OutputFormat::Av1.is_movie() && OutputFormat::Av1.supports_audio());
        assert_eq!(VideoCodecOptions::parse_level("4.1"), Some(Some(41)));
        assert_eq!(VideoCodecOptions::parse_level("51"), Some(Some(51)));
        assert_eq!(VideoCodecOptions::parse_level("auto"), Some(None));
        assert_eq!(VideoCodecOptions::parse_level("9"), None);
        let mut o = VideoCodecOptions { level: Some(41), ..Default::default() };
        assert_eq!((o.hevc_level_idc(), o.av1_level_idx()), (Some(123), Some(9)));
        o.level = Some(20);
        assert_eq!(o.av1_level_idx(), Some(0));
        o.quality = 100;
        assert_eq!((o.hevc_qp(), o.av1_qindex()), (4, 4));
        o.quality = 1;
        assert!(o.hevc_qp() <= 51 && o.av1_qindex() >= 240);
        assert_eq!(o.keyint(29.97), 60);
        o.keyframe_interval = 1;
        assert_eq!(o.keyint(29.97), 1);
        // WebM with AV1 has no alpha; switching drops RGB + Alpha.
        let mut m = OutputModule::for_format(OutputFormat::WebM);
        m.channels = Channels::Rgba;
        assert!(m.supports_alpha());
        m.webm_codec = WebmVideoCodec::Av1;
        assert!(!m.supports_alpha());
        assert!(m.summary().starts_with("WebM (AV1 + Opus 192 kbps)"), "{}", m.summary());
        let h = OutputModule::for_format(OutputFormat::Hevc);
        assert_eq!(h.summary(), "HEVC (H.265) · Main · 10000 kbps · Level Auto · Key every auto");
        // Older projects (no codec fields) load with the defaults.
        let old: OutputModule = serde_json::from_str(r#"{"format":"H264","bitrate_kbps":5000}"#).unwrap();
        assert_eq!((old.codec, old.webm_codec, old.opus_bitrate_kbps), (VideoCodecOptions::default(), WebmVideoCodec::Vp9, 192));
    }
}
