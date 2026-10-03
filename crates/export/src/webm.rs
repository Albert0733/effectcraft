//! WebM output (VP9 video, optional VP9 alpha, Opus audio) and audio-only WAV / AIFF.
//!
//! The WebM container is written from the Matroska / WebM specifications (IETF RFC 8794 EBML,
//! the Matroska element registry and the WebM container guidelines): EBML header, Segment
//! (size patched at the end), Info, Tracks and one Cluster per render batch. Video frames are
//! SimpleBlocks; with alpha (Channels: RGB + Alpha) each frame is a BlockGroup whose
//! BlockAdditional (BlockAddID 1) carries a second VP9 frame coding the alpha channel as luma,
//! with `AlphaMode` 1 on the track, as WebM defines. VP9 comes from `effectcraft-vp9enc` (every
//! frame a key frame), Opus from `effectcraft-opusenc` (CELT, 48 kHz stereo, 20 ms packets,
//! `CodecDelay` = pre-skip). WAV (RIFF, 16-bit PCM) and AIFF (big-endian 16-bit, 80-bit
//! extended sample rate) follow their published file layouts.

use std::io::{Seek, SeekFrom, Write};

use effectcraft_project::Comp;
use effectcraft_project::render_queue::Channels;
use effectcraft_time::{TICKS_PER_SECOND, Tick};
use rayon::prelude::*;

use crate::{Job, Report, Result, State, batch_size, io, render_frame, rgba8, wants_audio};

const OPUS_RATE: u32 = 48_000;
const OPUS_BITRATE: u32 = 192_000;

// Element ids (with their length markers).
const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const INFO: u32 = 0x1549_A966;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const CLUSTER: u32 = 0x1F43_B675;

fn id_bytes(id: u32) -> Vec<u8> {
    let b = id.to_be_bytes();
    let skip = b.iter().position(|x| *x != 0).unwrap_or(3);
    b[skip..].to_vec()
}

/// EBML variable-size integer for an element data size (shortest form).
fn vint(n: u64) -> Vec<u8> {
    for len in 1..=8u32 {
        let max = (1u64 << (7 * len)) - 2;
        if n <= max {
            let marked = n | (1u64 << (7 * len));
            return marked.to_be_bytes()[8 - len as usize..].to_vec();
        }
    }
    let mut v = vec![0x01];
    v.extend_from_slice(&n.to_be_bytes()[1..]);
    v
}

fn el(out: &mut Vec<u8>, id: u32, data: &[u8]) {
    out.extend(id_bytes(id));
    out.extend(vint(data.len() as u64));
    out.extend_from_slice(data);
}

fn el_uint(out: &mut Vec<u8>, id: u32, v: u64) {
    let b = v.to_be_bytes();
    let skip = b.iter().position(|x| *x != 0).unwrap_or(7);
    el(out, id, &b[skip..]);
}

fn el_float(out: &mut Vec<u8>, id: u32, v: f64) {
    el(out, id, &v.to_be_bytes());
}

fn el_str(out: &mut Vec<u8>, id: u32, s: &str) {
    el(out, id, s.as_bytes());
}

/// A block payload: track number, timestamp relative to the cluster, flags, frame.
fn block(track: u64, rel: i16, flags: u8, data: &[u8]) -> Vec<u8> {
    let mut b = vint(track);
    b.extend_from_slice(&rel.to_be_bytes());
    b.push(flags);
    b.extend_from_slice(data);
    b
}

struct Frame {
    ms: i64,
    track: u64,
    data: Vec<u8>,
    alpha: Option<Vec<u8>>,
}

fn cluster(frames: &mut [Frame]) -> Vec<u8> {
    frames.sort_by_key(|f| (f.ms, f.track));
    let base = frames.first().map_or(0, |f| f.ms);
    let mut body = vec![];
    el_uint(&mut body, 0xE7, base.max(0) as u64);
    for f in frames.iter() {
        let rel = (f.ms - base).clamp(i16::MIN as i64, i16::MAX as i64) as i16;
        match &f.alpha {
            None => el(&mut body, 0xA3, &block(f.track, rel, 0x80, &f.data)),
            Some(a) => {
                let mut g = vec![];
                el(&mut g, 0xA1, &block(f.track, rel, 0x00, &f.data));
                let mut more = vec![];
                el_uint(&mut more, 0xEE, 1);
                el(&mut more, 0xA5, a);
                let mut adds = vec![];
                el(&mut adds, 0xA6, &more);
                el(&mut g, 0x75A1, &adds);
                el(&mut body, 0xA0, &g);
            }
        }
    }
    let mut out = vec![];
    el(&mut out, CLUSTER, &body);
    out
}

pub(crate) fn webm(job: &Job, comp: &Comp, w: u32, h: u32, st: &mut State) -> Result<Report> {
    let rate = job.settings.rate(comp);
    let alpha = job.output.channels == Channels::Rgba;
    let quality = job.output.quality.clamp(1, 100);
    let cfg = effectcraft_vp9enc::EncoderConfig { width: w, height: h, quality, full_range: false };
    let with_audio = wants_audio(job);
    let mut opus = with_audio.then(|| effectcraft_opusenc::OpusEncoder::new(2, OPUS_BITRATE));
    let total = st.total;
    let frame_ms = |k: u64| -> i64 { ((k as i128 * 1000 * rate.den as i128 + rate.num as i128 / 2) / rate.num as i128) as i64 };
    let duration_ms = frame_ms(total) as f64;

    let mut file = crate::out::create(job.sink, job.path)?;
    // EBML header.
    let mut head = vec![];
    let mut eb = vec![];
    el_uint(&mut eb, 0x4286, 1);
    el_uint(&mut eb, 0x42F7, 1);
    el_uint(&mut eb, 0x42F2, 4);
    el_uint(&mut eb, 0x42F3, 8);
    el_str(&mut eb, 0x4282, "webm");
    el_uint(&mut eb, 0x4287, 4);
    el_uint(&mut eb, 0x4285, 2);
    el(&mut head, EBML, &eb);
    // Segment with an 8-byte size patched at the end.
    head.extend(id_bytes(SEGMENT));
    let size_pos = head.len() as u64;
    head.extend_from_slice(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    let seg_start = head.len() as u64;
    let mut info = vec![];
    el_uint(&mut info, 0x2AD7B1, 1_000_000);
    el_float(&mut info, 0x4489, duration_ms);
    el_str(&mut info, 0x4D80, "EffectCraft");
    el_str(&mut info, 0x5741, "EffectCraft");
    el(&mut head, INFO, &info);
    let mut tracks = vec![];
    let mut v = vec![];
    el_uint(&mut v, 0xD7, 1);
    el_uint(&mut v, 0x73C5, 1);
    el_uint(&mut v, 0x83, 1);
    el_uint(&mut v, 0x9C, 0);
    el_str(&mut v, 0x86, "V_VP9");
    el_uint(&mut v, 0x23E383, (1_000_000_000u128 * rate.den as u128 / rate.num.max(1) as u128) as u64);
    if alpha {
        el_uint(&mut v, 0x55EE, 1);
    }
    let mut video = vec![];
    el_uint(&mut video, 0xB0, w as u64);
    el_uint(&mut video, 0xBA, h as u64);
    if alpha {
        el_uint(&mut video, 0x53C0, 1);
    }
    el(&mut v, 0xE0, &video);
    el(&mut tracks, TRACK_ENTRY, &v);
    if let Some(o) = &opus {
        let mut a = vec![];
        el_uint(&mut a, 0xD7, 2);
        el_uint(&mut a, 0x73C5, 2);
        el_uint(&mut a, 0x83, 2);
        el_uint(&mut a, 0x9C, 0);
        el_str(&mut a, 0x86, "A_OPUS");
        el(&mut a, 0x63A2, &o.opus_head());
        el_uint(&mut a, 0x56AA, o.pre_skip() as u64 * 1_000_000_000 / OPUS_RATE as u64);
        el_uint(&mut a, 0x56BB, 80_000_000);
        let mut au = vec![];
        el_float(&mut au, 0xB5, OPUS_RATE as f64);
        el_uint(&mut au, 0x9F, 2);
        el(&mut a, 0xE1, &au);
        el(&mut tracks, TRACK_ENTRY, &a);
    }
    el(&mut head, TRACKS, &tracks);
    file.write_all(&head).map_err(io)?;
    let mut seg_len = head.len() as u64 - seg_start;

    // Audio: mixed in blocks, encoded in 20 ms packets.
    let (span_start, span_end) = job.settings.span(comp);
    let mut cursor = span_start.to_units_floor(OPUS_RATE as i64);
    let mut pcm_left: Vec<f32> = vec![];
    let mut packets = 0u64;
    let mut audio_until = |until: Tick, frames: &mut Vec<Frame>, finish: bool| {
        let Some(enc) = opus.as_mut() else { return };
        let end = until.to_units_floor(OPUS_RATE as i64);
        if end > cursor {
            let n = (end - cursor) as usize;
            let start = Tick(((cursor as i128 * TICKS_PER_SECOND as i128) / OPUS_RATE as i128) as i64);
            pcm_left.extend(effectcraft_render::audio::mix_comp(job.project, job.footage, job.expr, job.comp, start, n, OPUS_RATE));
            cursor = end;
        }
        let fs = effectcraft_opusenc::OpusEncoder::FRAME_SIZE * 2;
        if finish && !pcm_left.is_empty() {
            // Pad the last packet (pre-skip + padding cover the encoder delay).
            let pad = (fs - pcm_left.len() % fs) % fs + fs;
            pcm_left.extend(std::iter::repeat_n(0.0, pad));
        }
        while pcm_left.len() >= fs {
            let pkt = enc.encode_float(&pcm_left[..fs]);
            pcm_left.drain(..fs);
            frames.push(Frame { ms: packets as i64 * 20, track: 2, data: pkt, alpha: None });
            packets += 1;
        }
    };

    let batch = batch_size();
    let mut i = 0;
    while i < total {
        let end = (i + batch).min(total);
        let encoded: Vec<(Vec<u8>, Option<Vec<u8>>)> = (i..end)
            .into_par_iter()
            .map(|k| {
                let px = rgba8(&render_frame(job, comp, k), comp, if alpha { Channels::Rgba } else { Channels::Rgb }, w, h);
                let mut enc = effectcraft_vp9enc::Vp9Encoder::new(cfg.clone());
                let (y, u, vv) = effectcraft_vp9enc::rgba_to_yuv420(&px, w, h, false);
                let color = enc.encode_yuv420(&y, &u, &vv);
                let a = alpha.then(|| {
                    let mut ea = effectcraft_vp9enc::Vp9Encoder::new(cfg.clone());
                    let (y, u, vv) = effectcraft_vp9enc::alpha_to_yuv420(&px, w, h);
                    ea.encode_yuv420(&y, &u, &vv)
                });
                (color, a)
            })
            .collect();
        let mut frames: Vec<Frame> =
            encoded.into_iter().enumerate().map(|(j, (data, alpha))| Frame { ms: frame_ms(i + j as u64), track: 1, data, alpha }).collect();
        let t_end = if end >= total { span_end } else { job.settings.frame_time(comp, end) };
        audio_until(t_end, &mut frames, end >= total);
        let c = cluster(&mut frames);
        file.write_all(&c).map_err(io)?;
        seg_len += c.len() as u64;
        st.advance(end - i)?;
        i = end;
    }
    if total == 0 {
        let mut frames = vec![];
        audio_until(span_end, &mut frames, true);
        if !frames.is_empty() {
            let c = cluster(&mut frames);
            file.write_all(&c).map_err(io)?;
            seg_len += c.len() as u64;
        }
    }
    // Patch the Segment size.
    let mut size = [0u8; 8];
    size[0] = 0x01;
    size[1..].copy_from_slice(&seg_len.to_be_bytes()[1..]);
    file.seek(SeekFrom::Start(size_pos)).map_err(io)?;
    file.write_all(&size).map_err(io)?;
    file.seek(SeekFrom::End(0)).map_err(io)?;
    let bytes = file.finish()?;
    Ok(Report { path: job.path.to_string(), frames: 0, width: w, height: h, seconds: 0.0, bytes, audio: with_audio })
}

// ---------------------------------------------------------------- audio-only outputs

fn ieee_extended(v: f64) -> [u8; 10] {
    // 80-bit IEEE 754 extended precision (AIFF COMM sample rate).
    let mut out = [0u8; 10];
    if v <= 0.0 {
        return out;
    }
    let e = v.log2().floor() as i32;
    let mant = (v / 2f64.powi(e) * (1u64 << 63) as f64) as u64;
    out[..2].copy_from_slice(&((e + 16383) as u16).to_be_bytes());
    out[2..].copy_from_slice(&mant.to_be_bytes());
    out
}

/// WAV or AIFF: the comp's audio over the render span, 16-bit stereo.
pub(crate) fn audio_file(job: &Job, comp: &Comp, aiff: bool, st: &mut State) -> Result<Report> {
    let sr = job.output.audio_sample_rate.clamp(8_000, 192_000);
    let (span_start, span_end) = job.settings.span(comp);
    let start = span_start.to_units_floor(sr as i64);
    let end = span_end.to_units_floor(sr as i64);
    let n = (end - start).max(0) as usize;
    let mut pcm = Vec::with_capacity(n * 4);
    // Mix in one-second blocks, reporting progress in frames.
    let block = sr as usize;
    let mut done = 0usize;
    let total_frames = st.total.max(1);
    let mut reported = 0u64;
    while done < n {
        let k = block.min(n - done);
        let t = Tick((((start + done as i64) as i128 * TICKS_PER_SECOND as i128) / sr as i128) as i64);
        for s in effectcraft_render::audio::mix_comp(job.project, job.footage, job.expr, job.comp, t, k, sr) {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            pcm.extend_from_slice(&if aiff { v.to_be_bytes() } else { v.to_le_bytes() });
        }
        done += k;
        let now = (done as u64 * total_frames / n.max(1) as u64).min(total_frames);
        if now > reported && st.total > 0 {
            st.advance(now - reported)?;
            reported = now;
        }
    }
    if reported < st.total {
        st.advance(st.total - reported)?;
    }
    let mut out = vec![];
    let frames = (pcm.len() / 4) as u32;
    if aiff {
        let mut comm = vec![];
        comm.extend_from_slice(&2u16.to_be_bytes());
        comm.extend_from_slice(&frames.to_be_bytes());
        comm.extend_from_slice(&16u16.to_be_bytes());
        comm.extend_from_slice(&ieee_extended(sr as f64));
        let ssnd_len = 8 + pcm.len();
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&((4 + 8 + comm.len() + 8 + ssnd_len + ssnd_len % 2) as u32).to_be_bytes());
        out.extend_from_slice(b"AIFF");
        out.extend_from_slice(b"COMM");
        out.extend_from_slice(&(comm.len() as u32).to_be_bytes());
        out.extend_from_slice(&comm);
        out.extend_from_slice(b"SSND");
        out.extend_from_slice(&(ssnd_len as u32).to_be_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&pcm);
        if ssnd_len % 2 == 1 {
            out.push(0);
        }
    } else {
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + pcm.len()) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&sr.to_le_bytes());
        out.extend_from_slice(&(sr * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        out.extend_from_slice(&pcm);
    }
    let mut file = crate::out::create(job.sink, job.path)?;
    file.write_all(&out).map_err(io)?;
    let bytes = file.finish()?;
    Ok(Report { path: job.path.to_string(), frames: 0, width: 0, height: 0, seconds: 0.0, bytes, audio: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vints_and_ids() {
        assert_eq!(vint(0), vec![0x80]);
        assert_eq!(vint(126), vec![0xFE]);
        assert_eq!(vint(127), vec![0x40, 0x7F]);
        assert_eq!(id_bytes(0xA3), vec![0xA3]);
        assert_eq!(id_bytes(EBML), vec![0x1A, 0x45, 0xDF, 0xA3]);
        assert_eq!(ieee_extended(44100.0), [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
        assert_eq!(ieee_extended(48000.0)[..4], [0x40, 0x0E, 0xBB, 0x80]);
    }
}
