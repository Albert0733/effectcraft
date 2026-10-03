//! Clean-room pure-Rust Opus encoder (RFC 6716 CELT-only mode; RFC 7845 `OpusHead`).
//!
//! Produces fullband (20 kHz) 48 kHz Opus packets of 20 ms, mono or stereo, at a constant
//! packet size derived from the requested bitrate. Each packet is a single CELT frame (TOC
//! configuration 31, frame-count code 0). Stereo is coded as dual (independent L/R) stereo.
//!
//! Layer L0: no dependencies beyond `std`; no `unsafe`; builds for `wasm32-unknown-unknown`.
//!
//! ```
//! use effectcraft_opusenc::OpusEncoder;
//! let mut enc = OpusEncoder::new(2, 128_000);
//! let pcm = vec![0.0f32; OpusEncoder::FRAME_SIZE * 2];
//! let packet = enc.encode_float(&pcm);
//! assert_eq!(packet[0] >> 3, 31); // CELT-only, fullband, 20 ms
//! assert_eq!(enc.opus_head().len(), 19);
//! ```

// The bit-allocation code mirrors the normative integer arithmetic; keep C-like shift expressions.
#![allow(clippy::precedence, clippy::int_plus_one)]

mod bands;
mod energy;
mod mdct;
mod range;
mod rate;
mod tables;

use std::sync::OnceLock;

use bands::{SPREAD_NORMAL, quant_all_bands};
use energy::{quant_coarse_energy, quant_energy_finalise, quant_fine_energy};
use mdct::Mdct;
use range::{BITRES, RangeEncoder};
use rate::{Mode, compute_allocation};
use tables::*;

/// Samples per channel in one 20 ms frame at 48 kHz.
const N: usize = 960;
/// LM of a 20 ms frame (8 short blocks).
const LM: usize = 3;
/// Smallest CELT payload the encoder produces (bytes).
const MIN_FRAME_BYTES: usize = 16;
/// Largest CELT payload allowed in one frame (RFC 6716 §3.4, R2).
const MAX_FRAME_BYTES: usize = 1275;

fn mode() -> &'static Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    MODE.get_or_init(Mode::new)
}

/// Opus encoder for one mono or stereo stream at 48 kHz.
pub struct OpusEncoder {
    channels: usize,
    /// CELT payload bytes per packet (the packet is one byte longer: the TOC).
    frame_bytes: usize,
    mdct: Mdct,
    /// Last input sample of each channel (pre-emphasis filter memory, ×32768).
    preemph_mem: [f32; 2],
    /// Last `OVERLAP` pre-emphasised samples of each channel.
    hist: Vec<f32>,
    /// Quantised band energies, exactly as the decoder holds them.
    old_band_e: [f32; 2 * NB_EBANDS],
    block: Vec<f32>,
    freq: Vec<f32>,
}

impl OpusEncoder {
    /// Samples per channel per packet (20 ms at 48 kHz).
    pub const FRAME_SIZE: usize = N;

    /// Creates an encoder for `channels` (1 or 2) channels of 48 kHz audio at `bitrate_bps`
    /// (total for all channels; packets are constant-size, clamped to 6.8–510 kb/s).
    pub fn new(channels: u8, bitrate_bps: u32) -> Self {
        assert!(channels == 1 || channels == 2, "Opus family-0 streams have 1 or 2 channels");
        let c = channels as usize;
        let packet_bytes = (bitrate_bps as usize).div_ceil(400);
        let frame_bytes = packet_bytes.saturating_sub(1).clamp(MIN_FRAME_BYTES, MAX_FRAME_BYTES);
        OpusEncoder {
            channels: c,
            frame_bytes,
            mdct: Mdct::new(N),
            preemph_mem: [0.0; 2],
            hist: vec![0.0; c * OVERLAP],
            old_band_e: [-28.0; 2 * NB_EBANDS],
            block: vec![0.0; N + OVERLAP],
            freq: vec![0.0; c * N],
        }
    }

    /// Number of channels.
    pub fn channels(&self) -> u8 {
        self.channels as u8
    }

    /// Actual bitrate in bits per second (packet size × 50 packets per second).
    pub fn bitrate(&self) -> u32 {
        ((self.frame_bytes + 1) * 8 * 50) as u32
    }

    /// Samples at 48 kHz the decoder must discard from the start of the stream (the MDCT
    /// overlap); this is the `OpusHead` pre-skip.
    pub fn pre_skip(&self) -> u16 {
        OVERLAP as u16
    }

    /// The 19-byte `OpusHead` identification header (RFC 7845 §5.1): version 1, channel count,
    /// pre-skip, input sample rate 48000, output gain 0, channel mapping family 0.
    pub fn opus_head(&self) -> Vec<u8> {
        let mut h = Vec::with_capacity(19);
        h.extend_from_slice(b"OpusHead");
        h.push(1);
        h.push(self.channels as u8);
        h.extend_from_slice(&self.pre_skip().to_le_bytes());
        h.extend_from_slice(&48_000u32.to_le_bytes());
        h.extend_from_slice(&0i16.to_le_bytes());
        h.push(0);
        h
    }

    /// Encodes one 20 ms frame of interleaved f32 samples in [-1, 1] (exactly
    /// `FRAME_SIZE * channels` values) and returns one Opus packet (TOC byte + CELT frame).
    pub fn encode_float(&mut self, pcm: &[f32]) -> Vec<u8> {
        let c = self.channels;
        assert_eq!(pcm.len(), N * c, "encode_float takes exactly FRAME_SIZE samples per channel");

        // Pre-emphasis, the MDCT and the band energies.
        let mut band_e = [0f32; 2 * NB_EBANDS];
        let mut log_e = [0f32; 2 * NB_EBANDS];
        let mut peak = 0f32;
        for ch in 0..c {
            self.block[..OVERLAP].copy_from_slice(&self.hist[ch * OVERLAP..(ch + 1) * OVERLAP]);
            let mut mem = self.preemph_mem[ch];
            for j in 0..N {
                let s = pcm[j * c + ch];
                let s = if s.is_finite() { s.clamp(-2.0, 2.0) * 32768.0 } else { 0.0 };
                let v = s - PREEMPH * mem;
                mem = s;
                self.block[OVERLAP + j] = v;
            }
            self.preemph_mem[ch] = mem;
            self.hist[ch * OVERLAP..(ch + 1) * OVERLAP].copy_from_slice(&self.block[N..]);
            peak = self.block.iter().fold(peak, |p, v| p.max(v.abs()));
            let freq = &mut self.freq[ch * N..(ch + 1) * N];
            self.mdct.forward(&self.block, freq);
            for i in 0..NB_EBANDS {
                let lo = (EBANDS[i] as usize) << LM;
                let hi = (EBANDS[i + 1] as usize) << LM;
                let e = (1e-27f32 + freq[lo..hi].iter().map(|v| v * v).sum::<f32>()).sqrt();
                band_e[ch * NB_EBANDS + i] = e;
                log_e[ch * NB_EBANDS + i] = e.log2() - E_MEANS[i];
                let g = 1.0 / e;
                for v in freq[lo..hi].iter_mut() {
                    *v *= g;
                }
            }
        }
        let silence = peak < 1e-4;
        let toc = (31u8 << 3) | if c == 2 { 4 } else { 0 };
        let mut packet = vec![toc];
        packet.extend_from_slice(&self.encode_celt(&log_e, silence));
        packet
    }

    /// Codes one CELT frame (RFC 6716 §4.3, in bitstream order).
    fn encode_celt(&mut self, log_e: &[f32; 2 * NB_EBANDS], silence: bool) -> Vec<u8> {
        let m = mode();
        let c = self.channels;
        let (start, end) = (0usize, NB_EBANDS);
        let mut enc = RangeEncoder::new();
        if c == 1 {
            for i in 0..NB_EBANDS {
                self.old_band_e[i] = self.old_band_e[i].max(self.old_band_e[NB_EBANDS + i]);
            }
        }
        if silence {
            // The silence flag; the decoder treats every remaining bit as consumed.
            enc.bit_logp(true, 15);
            self.old_band_e = [-28.0; 2 * NB_EBANDS];
            return enc.done(2);
        }
        let len = self.frame_bytes;
        let total_bits_raw = len as i32 * 8;
        enc.bit_logp(false, 15);
        // No pitch post-filter.
        if enc.tell() + 16 <= total_bits_raw {
            enc.bit_logp(false, 1);
        }
        // Long blocks only.
        if enc.tell() + 3 <= total_bits_raw {
            enc.bit_logp(false, 3);
        }
        // Coarse energy, choosing intra or inter prediction by trial (whichever is cheaper).
        if enc.tell() + 3 <= total_bits_raw {
            let mut enc_intra = enc.clone();
            let mut old_intra = self.old_band_e;
            enc_intra.bit_logp(true, 3);
            quant_coarse_energy(start, end, log_e, &mut old_intra, true, &mut enc_intra, c, LM, total_bits_raw);
            enc.bit_logp(false, 3);
            let mut old_inter = self.old_band_e;
            quant_coarse_energy(start, end, log_e, &mut old_inter, false, &mut enc, c, LM, total_bits_raw);
            if enc_intra.tell_frac() < enc.tell_frac() {
                enc = enc_intra;
                self.old_band_e = old_intra;
            } else {
                self.old_band_e = old_inter;
            }
        } else {
            quant_coarse_energy(start, end, log_e, &mut self.old_band_e, false, &mut enc, c, LM, total_bits_raw);
        }
        // TF resolution: no changes.
        {
            let mut budget = total_bits_raw;
            let mut tell = enc.tell();
            let mut logp = 4;
            let tf_select_rsv = LM > 0 && tell + logp + 1 <= budget;
            budget -= tf_select_rsv as i32;
            for _ in start..end {
                if tell + logp <= budget {
                    enc.bit_logp(false, logp as u32);
                    tell = enc.tell();
                }
                logp = 5;
            }
            if tf_select_rsv && TF_SELECT_TABLE[LM][0] != TF_SELECT_TABLE[LM][2] {
                enc.bit_logp(false, 1);
            }
        }
        if enc.tell() + 4 <= total_bits_raw {
            enc.icdf(SPREAD_NORMAL, &SPREAD_ICDF, 5);
        }
        // Dynamic allocation: no boosts.
        let cap = m.init_caps(LM, c);
        let offsets = [0i32; NB_EBANDS];
        let total_bits = total_bits_raw << BITRES;
        let mut tellf = enc.tell_frac();
        for &cp in cap.iter().take(end).skip(start) {
            if tellf + (6 << BITRES) < total_bits && 0 < cp {
                enc.bit_logp(false, 6);
                tellf = enc.tell_frac();
            }
        }
        let alloc_trim = 5;
        if tellf + (6 << BITRES) <= total_bits {
            enc.icdf(alloc_trim as usize, &TRIM_ICDF, 7);
        }
        let bits = (total_bits_raw << BITRES) - enc.tell_frac() - 1;
        let alloc = compute_allocation(m, start, end, &offsets, &cap, alloc_trim, bits, c, LM, &mut enc);
        quant_fine_energy(start, end, log_e, &mut self.old_band_e, &alloc.fine_quant, &mut enc, c);
        quant_all_bands(
            m,
            start,
            end,
            &self.freq,
            N,
            c,
            &alloc.pulses,
            SPREAD_NORMAL,
            alloc.dual_stereo,
            alloc.intensity,
            total_bits_raw << BITRES,
            alloc.balance,
            &mut enc,
            LM,
            alloc.coded_bands,
        );
        let bits_left = total_bits_raw - enc.tell();
        quant_energy_finalise(start, end, log_e, &mut self.old_band_e, &alloc.fine_quant, &alloc.fine_priority, bits_left, &mut enc, c);
        debug_assert!(enc.tell() <= total_bits_raw, "frame overflow: {} > {}", enc.tell(), total_bits_raw);
        if c == 1 {
            let (a, b) = self.old_band_e.split_at_mut(NB_EBANDS);
            b.copy_from_slice(a);
        }
        enc.done(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_head_layout() {
        let e = OpusEncoder::new(2, 96_000);
        let h = e.opus_head();
        assert_eq!(&h[..8], b"OpusHead");
        assert_eq!(h[8], 1);
        assert_eq!(h[9], 2);
        assert_eq!(u16::from_le_bytes([h[10], h[11]]), e.pre_skip());
        assert_eq!(u32::from_le_bytes([h[12], h[13], h[14], h[15]]), 48_000);
        assert_eq!(&h[16..], &[0, 0, 0]);
    }

    #[test]
    fn packet_sizes_follow_bitrate() {
        for (ch, rate) in [(1u8, 64_000u32), (2, 128_000), (2, 256_000)] {
            let mut e = OpusEncoder::new(ch, rate);
            let pcm: Vec<f32> = (0..N * ch as usize).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
            let p = e.encode_float(&pcm);
            assert_eq!(p.len(), rate as usize / 400);
            assert_eq!(p[0], 0xF8 | if ch == 2 { 4 } else { 0 });
            assert_eq!(e.bitrate(), rate);
        }
    }

    #[test]
    fn silence_is_a_tiny_packet() {
        let mut e = OpusEncoder::new(1, 64_000);
        let p = e.encode_float(&vec![0.0; N]);
        assert_eq!(p.len(), 3);
    }
}
