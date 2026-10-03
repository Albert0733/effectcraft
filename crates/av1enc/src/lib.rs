//! Clean-room pure-Rust AV1 encoder — API skeleton (implementation in progress).
//!
//! The public API below is the contract `crates/export` codes against; keep it stable.

/// Rate control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateControl {
    /// Constant quantizer index (`base_q_idx`, 1–255; lower is better).
    ConstantQ(u8),
    /// Average bitrate target in kbit/s (frame-level quantizer adaptation).
    Bitrate { kbps: u32 },
}

/// Encoder settings. Main profile (seq_profile 0), 4:2:0.
#[derive(Clone, Debug, PartialEq)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// 8 or 10.
    pub bit_depth: u8,
    /// `seq_level_idx` (e.g. 8 = level 4.0); `None` picks the lowest level that fits.
    pub level_idx: Option<u8>,
    pub rate: RateControl,
    /// Frames between key frames (1 = all key frames).
    pub keyint: u32,
    /// BT.709 limited range is signalled unless this is set.
    pub full_range: bool,
}

impl EncoderConfig {
    pub fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Self {
        EncoderConfig { width, height, fps_num, fps_den, bit_depth: 8, level_idx: None, rate: RateControl::ConstantQ(100), keyint: 60, full_range: false }
    }
}

/// One input picture, 4:2:0, samples at the configured bit depth.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    pub y: &'a [u16],
    pub u: &'a [u16],
    pub v: &'a [u16],
    pub y_stride: usize,
    pub uv_stride: usize,
}

/// One temporal unit as stored in MP4 / WebM samples: OBUs with `obu_has_size_field` = 1 and
/// no temporal delimiter (key frames start with the sequence header OBU).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub data: Vec<u8>,
    pub keyframe: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfig(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidConfig(s) => write!(f, "invalid AV1 encoder config: {s}"),
        }
    }
}

impl std::error::Error for Error {}

/// AV1 encoder. Frames come out in input order (no hidden/reordered frames).
pub struct Encoder {
    cfg: EncoderConfig,
}

impl Encoder {
    pub fn new(cfg: EncoderConfig) -> Result<Encoder, Error> {
        if cfg.width == 0 || cfg.height == 0 {
            return Err(Error::InvalidConfig("empty frame".into()));
        }
        Ok(Encoder { cfg })
    }
    pub fn config(&self) -> &EncoderConfig {
        &self.cfg
    }
    /// The sequence header OBU (with size field).
    pub fn sequence_header_obu(&self) -> Vec<u8> {
        vec![]
    }
    /// AV1CodecConfigurationRecord (`av1C` payload, also the WebM CodecPrivate).
    pub fn av1c(&self) -> Vec<u8> {
        vec![]
    }
    /// Encodes one frame.
    pub fn encode(&mut self, _frame: &Frame) -> Packet {
        Packet { data: vec![], keyframe: true }
    }
}
