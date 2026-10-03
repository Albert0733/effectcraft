//! The VP9 intra encoder (`effectcraft-vp9enc`) round-trips through FilmCraft's VP9 decoder (and
//! ffmpeg as an external oracle when installed): sizes, lossless exactness, PSNR.

use effectcraft_vp9enc::{EncoderConfig, Vp9Encoder};
use filmcraft_vp9::{Decoder, Plane};

fn plane_u8(p: &Plane) -> Vec<u8> {
    match p {
        Plane::U8(v) => v.clone(),
        Plane::U16(v) => v.iter().map(|x| *x as u8).collect(),
    }
}

/// Synthetic planes: gradients, noise and sharp edges.
fn source(w: usize, h: usize, kind: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut s = 99u32 + kind;
    let mut rnd = || {
        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
        (s >> 24) as u8
    };
    let y: Vec<u8> = (0..w * h)
        .map(|i| {
            let (x, yy) = (i % w, i / w);
            match kind {
                0 => ((x * 255) / w.max(1)) as u8 / 2 + ((yy * 255) / h.max(1)) as u8 / 2,
                1 => rnd(),
                _ => {
                    if (x / 7 + yy / 5) % 2 == 0 {
                        30
                    } else {
                        220
                    }
                }
            }
        })
        .collect();
    let u: Vec<u8> = (0..cw * ch).map(|i| (64 + (i % cw) * 128 / cw.max(1)) as u8).collect();
    let v: Vec<u8> = (0..cw * ch).map(|i| (200 - (i / cw) * 100 / ch.max(1)) as u8).collect();
    (y, u, v)
}

fn decode(frame: &[u8]) -> filmcraft_vp9::Picture {
    let mut d = Decoder::new();
    let mut pics = d.decode(frame, 0).expect("decode");
    assert_eq!(pics.len(), 1);
    pics.remove(0)
}

fn crop(p: &[u8], stride: usize, w: usize, h: usize) -> Vec<u8> {
    (0..h).flat_map(|y| p[y * stride..y * stride + w].to_vec()).collect()
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let mse: f64 = a.iter().zip(b).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>() / a.len() as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (255.0f64 * 255.0 / mse).log10() }
}

#[test]
fn lossless_is_exact_for_odd_sizes() {
    for (w, h) in [(1, 1), (33, 17), (64, 64), (200, 120), (7, 70)] {
        for kind in 0..3 {
            let (y, u, v) = source(w, h, kind);
            let mut e = Vp9Encoder::new(EncoderConfig { width: w as u32, height: h as u32, quality: 100, full_range: false });
            let f = e.encode_yuv420(&y, &u, &v);
            let p = decode(&f);
            assert_eq!((p.width, p.height), (w as u32, h as u32));
            let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
            assert_eq!(crop(&plane_u8(&p.y), p.y_stride, w, h), y, "{w}x{h} kind {kind}: luma");
            assert_eq!(crop(&plane_u8(&p.u), p.uv_stride, cw, ch), u, "{w}x{h} kind {kind}: u");
            assert_eq!(crop(&plane_u8(&p.v), p.uv_stride, cw, ch), v, "{w}x{h} kind {kind}: v");
        }
    }
}

#[test]
fn lossy_quality_and_size() {
    let (w, h) = (200, 120);
    for kind in [0, 2] {
        let (y, u, v) = source(w, h, kind);
        let mut last = 0usize;
        for q in [30u8, 60, 80, 95] {
            let mut e = Vp9Encoder::new(EncoderConfig { width: w as u32, height: h as u32, quality: q, full_range: false });
            let f = e.encode_yuv420(&y, &u, &v);
            let p = decode(&f);
            let got = crop(&plane_u8(&p.y), p.y_stride, w, h);
            let db = psnr(&got, &y);
            let min = match q {
                30 => 20.0,
                60 => 28.0,
                80 => 32.0,
                _ => 38.0,
            };
            assert!(db > min, "kind {kind} q{q}: PSNR {db:.1} dB, {} bytes", f.len());
            assert!(f.len() >= last, "higher quality, more bytes");
            last = f.len();
        }
    }
}

#[test]
fn wide_frames_use_tile_columns() {
    // 4160 px > 64 superblocks: two tile columns are required.
    let (w, h) = (4160, 16);
    let (y, u, v) = source(w, h, 0);
    let mut e = Vp9Encoder::new(EncoderConfig { width: w as u32, height: h as u32, quality: 90, full_range: false });
    let f = e.encode_yuv420(&y, &u, &v);
    let p = decode(&f);
    assert!(psnr(&crop(&plane_u8(&p.y), p.y_stride, w, h), &y) > 35.0);
}

#[test]
fn ffmpeg_decodes_our_stream() {
    let (w, h) = (96, 64);
    let (y, u, v) = source(w, h, 0);
    let mut e = Vp9Encoder::new(EncoderConfig { width: w as u32, height: h as u32, quality: 100, full_range: false });
    let frame = e.encode_yuv420(&y, &u, &v);
    // IVF container (one frame).
    let mut ivf = Vec::new();
    ivf.extend_from_slice(b"DKIF");
    ivf.extend_from_slice(&0u16.to_le_bytes());
    ivf.extend_from_slice(&32u16.to_le_bytes());
    ivf.extend_from_slice(b"VP90");
    ivf.extend_from_slice(&(w as u16).to_le_bytes());
    ivf.extend_from_slice(&(h as u16).to_le_bytes());
    ivf.extend_from_slice(&30u32.to_le_bytes());
    ivf.extend_from_slice(&1u32.to_le_bytes());
    ivf.extend_from_slice(&1u32.to_le_bytes());
    ivf.extend_from_slice(&0u32.to_le_bytes());
    ivf.extend_from_slice(&(frame.len() as u32).to_le_bytes());
    ivf.extend_from_slice(&0u64.to_le_bytes());
    ivf.extend_from_slice(&frame);
    let dir = std::env::temp_dir().join(format!("effectcraft-vp9-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.ivf");
    std::fs::write(&path, &ivf).unwrap();
    let out = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(&path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output();
    let Ok(out) = out else {
        eprintln!("ffmpeg not found: skipping oracle");
        return;
    };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(&out.stdout[..w * h], &y[..], "ffmpeg luma matches (lossless)");
    assert_eq!(&out.stdout[w * h..w * h + u.len()], &u[..]);
}
