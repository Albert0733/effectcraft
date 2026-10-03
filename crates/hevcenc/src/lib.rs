//! Clean-room pure-Rust HEVC (H.265) encoder — API skeleton (implementation in progress).
//!
//! The public API below is the contract `crates/export` codes against; keep it stable.

/// Profile written in the parameter sets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    /// Main: 8-bit 4:2:0.
    #[default]
    Main,
    /// Main 10: 10-bit 4:2:0.
    Main10,
}

impl Profile {
    pub fn bit_depth(self) -> u8 {
        match self {
            Profile::Main => 8,
            Profile::Main10 => 10,
        }
    }
}

/// Rate control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateControl {
    /// Constant QP (0–51; lower is better).
    ConstantQp(u8),
    /// Average bitrate target in kbit/s (frame-level QP adaptation).
    Bitrate { kbps: u32 },
}

/// Encoder settings.
#[derive(Clone, Debug, PartialEq)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    /// Frame rate numerator / denominator (VUI timing and rate control).
    pub fps_num: u32,
    pub fps_den: u32,
    pub profile: Profile,
    /// `general_level_idc` (30 × level, e.g. 93 = level 3.1); `None` picks the lowest level that fits.
    pub level_idc: Option<u8>,
    pub rate: RateControl,
    /// Frames between IDR pictures (1 = all intra).
    pub keyint: u32,
    /// BT.709 limited range is signalled in the VUI unless this is set.
    pub full_range: bool,
}

impl EncoderConfig {
    pub fn new(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Self {
        EncoderConfig {
            width,
            height,
            fps_num,
            fps_den,
            profile: Profile::Main,
            level_idc: None,
            rate: RateControl::ConstantQp(28),
            keyint: 60,
            full_range: false,
        }
    }
}

/// One input picture, 4:2:0, samples at the profile's bit depth (8-bit values 0–255 for Main).
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    pub y: &'a [u16],
    pub u: &'a [u16],
    pub v: &'a [u16],
    pub y_stride: usize,
    pub uv_stride: usize,
}

/// One coded picture (an access unit): NAL units, each prefixed by a 4-byte big-endian length
/// (ISO/IEC 14496-15 sample format; parameter sets are only in [`Encoder::hvcc`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub data: Vec<u8>,
    /// IDR picture.
    pub keyframe: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfig(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidConfig(s) => write!(f, "invalid HEVC encoder config: {s}"),
        }
    }
}

impl std::error::Error for Error {}

/// HEVC encoder. Pictures come out in input order (no reordering: I and P only).
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
    /// VPS, SPS, PPS NAL units (without start codes or length prefixes).
    pub fn parameter_sets(&self) -> [Vec<u8>; 3] {
        [vec![], vec![], vec![]]
    }
    /// HEVCDecoderConfigurationRecord (`hvcC` payload) with the parameter sets.
    pub fn hvcc(&self) -> Vec<u8> {
        vec![]
    }
    /// Encodes one picture.
    pub fn encode(&mut self, _frame: &Frame) -> Packet {
        Packet { data: vec![], keyframe: true }
    }
}
