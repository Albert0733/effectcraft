//! A VP9 intra-frame encoder (profile 0, 8-bit 4:2:0), written from the *VP9 Bitstream &
//! Decoding Process Specification* v0.6 by mirroring its decoding process in reverse (see
//! README.md). Every frame is a key frame: one tile row (tile columns as the frame width
//! requires), 64×64 superblocks split to 8×8 blocks, 4×4 transforms (the Walsh–Hadamard
//! transform at quality 100, which is lossless), intra modes DC / V / H / TM chosen per block by
//! prediction error, default probabilities (no probability updates). The encoder reconstructs
//! exactly what a decoder reconstructs (the specification's integer inverse transforms and
//! intra edge rules), so prediction never drifts.

mod bool;
mod tables;

use bool::BoolEncoder;
use tables::*;

const DC_PRED: u8 = 0;
const V_PRED: u8 = 1;
const H_PRED: u8 = 2;
const TM_PRED: u8 = 9;
const MODES: [u8; 4] = [DC_PRED, V_PRED, H_PRED, TM_PRED];

const DCT_DCT: u8 = 0;
const ADST_DCT: u8 = 1;
const DCT_ADST: u8 = 2;
const ADST_ADST: u8 = 3;

/// mode2txfm_map for the modes used here.
fn mode_tx_type(mode: u8) -> u8 {
    match mode {
        V_PRED => ADST_DCT,
        H_PRED => DCT_ADST,
        TM_PRED => ADST_ADST,
        _ => DCT_DCT,
    }
}

const PARTITION_TREE: [i8; 6] = [0, 2, -1, 4, -2, -3];
const INTRA_MODE_TREE: [i8; 18] = [0, 2, -9, 4, -1, 6, 8, 12, -2, 10, -4, -5, -3, 14, -8, 16, -6, -7];

/// extra_bits[token] = (cat, numExtra, base).
const EXTRA_BITS: [(u8, u8, u16); 11] =
    [(0, 0, 0), (0, 0, 1), (0, 0, 2), (0, 0, 3), (0, 0, 4), (1, 1, 5), (2, 2, 7), (3, 3, 11), (4, 4, 19), (5, 5, 35), (6, 14, 67)];
const CAT_PROBS: [&[u8]; 7] = [
    &[],
    &[159],
    &[165, 145],
    &[173, 148, 140],
    &[176, 155, 140, 135],
    &[180, 157, 141, 134, 130],
    &[254, 254, 254, 252, 249, 243, 230, 196, 177, 153, 140, 133, 130, 129],
];

/// Encoder settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    /// 1..=100; 100 is lossless.
    pub quality: u8,
    /// Full-range (0–255) rather than limited (16–235) YUV; only signalled.
    pub full_range: bool,
}

/// Probabilities of the full token tree (9.3.2) for model probability `p`.
fn pareto(p: u8) -> [u8; 8] {
    let p = p.max(1) as usize;
    let x = (p - 1) / 2;
    let mut t = [0u8; 8];
    for n in 0..8 {
        t[n] = if p & 1 == 1 { PARETO_TABLE[x * 8 + n] } else { ((PARETO_TABLE[x * 8 + n] as u32 + PARETO_TABLE[(x + 1) * 8 + n] as u32) >> 1) as u8 };
    }
    t
}

fn scan(tx_type: u8) -> &'static [u16; 16] {
    match tx_type {
        ADST_DCT => &ROW_SCAN_4X4,
        DCT_ADST => &COL_SCAN_4X4,
        _ => &DEFAULT_SCAN_4X4,
    }
}

/// Coefficient context neighbours (9.3.2) of each scan position.
fn neighbors(tx_type: u8) -> [(usize, usize); 16] {
    let sc = scan(tx_type);
    let mut nb = [(0, 0); 16];
    for (c, &pos) in sc.iter().enumerate().skip(1) {
        let pos = pos as usize;
        let (i, j) = (pos / 4, pos % 4);
        nb[c] = if i > 0 && j > 0 {
            let a = (i - 1) * 4 + j;
            let b = i * 4 + j - 1;
            match tx_type {
                DCT_ADST => (a, a),
                ADST_DCT => (b, b),
                _ => (a, b),
            }
        } else if i > 0 {
            ((i - 1) * 4 + j, (i - 1) * 4 + j)
        } else {
            (j - 1, j - 1)
        };
    }
    nb
}

// ---------------------------------------------------------------- transforms (8.7)

fn r14(v: i32) -> i32 {
    (v + (1 << 13)) >> 14
}

fn idct4(x: [i32; 4]) -> [i32; 4] {
    let v0 = r14(x[0] * 11585 - x[2] * 11585);
    let v1 = r14(x[0] * 11585 + x[2] * 11585);
    let v2 = r14(x[1] * 6270 - x[3] * 15137);
    let v3 = r14(x[1] * 15137 + x[3] * 6270);
    [v1 + v3, v0 + v2, v0 - v2, v1 - v3]
}

fn iadst4(x: [i32; 4]) -> [i32; 4] {
    let (s19, s29, s39, s49) = (5283i64, 9929i64, 13377i64, 15212i64);
    let x: [i64; 4] = x.map(|v| v as i64);
    let s0 = s19 * x[0];
    let s1 = s29 * x[0];
    let s2 = s39 * x[1];
    let s3 = s49 * x[2];
    let s4 = s19 * x[2];
    let s5 = s29 * x[3];
    let s6 = s49 * x[3];
    let s7 = s39 * (x[0] - x[2] + x[3]);
    let x0 = s0 + s3 + s5;
    let x1 = s1 - s4 - s6;
    let x2 = s7;
    let x3 = s2;
    let r = |v: i64| ((v + (1 << 13)) >> 14) as i32;
    [r(x0 + x3), r(x1 + x3), r(x2), r(x0 + x1 - x3)]
}

/// The decoder's 4×4 inverse transform of dequantised coefficients (row-major) to a residual.
fn inverse4(coefs: &[i32; 16], tx_type: u8) -> [i32; 16] {
    let row_adst = matches!(tx_type, DCT_ADST | ADST_ADST);
    let col_adst = matches!(tx_type, ADST_DCT | ADST_ADST);
    let mut t = [[0i32; 4]; 4];
    for i in 0..4 {
        let r = [coefs[i * 4], coefs[i * 4 + 1], coefs[i * 4 + 2], coefs[i * 4 + 3]];
        t[i] = if row_adst { iadst4(r) } else { idct4(r) };
    }
    let mut out = [0i32; 16];
    for j in 0..4 {
        let c = [t[0][j], t[1][j], t[2][j], t[3][j]];
        let c = if col_adst { iadst4(c) } else { idct4(c) };
        for i in 0..4 {
            out[i * 4 + j] = (c[i] + 8) >> 4;
        }
    }
    out
}

/// Inverse Walsh–Hadamard (8.7.1.10) on one line.
fn iwht(x: [i32; 4], shift: u32) -> [i32; 4] {
    let mut a = x[0] >> shift;
    let mut c = x[1] >> shift;
    let mut d = x[2] >> shift;
    let mut b = x[3] >> shift;
    a += c;
    d -= b;
    let e = (a - d) >> 1;
    b = e - b;
    c = e - c;
    a -= b;
    d += c;
    [a, b, c, d]
}

/// The exact inverse of [`iwht`] (with shift 0): inputs that produce outputs `y`.
fn fwht(y: [i32; 4]) -> [i32; 4] {
    let (aa, bb, cc, dd) = (y[0], y[1], y[2], y[3]);
    let d1 = dd - cc;
    let a1 = aa + bb;
    let e = (a1 - d1) >> 1;
    let b0 = e - bb;
    let c0 = e - cc;
    let a0 = a1 - c0;
    let d0 = d1 + b0;
    [a0, c0, d0, b0]
}

fn inverse_wht(coefs: &[i32; 16]) -> [i32; 16] {
    let mut t = [[0i32; 4]; 4];
    for i in 0..4 {
        t[i] = iwht([coefs[i * 4], coefs[i * 4 + 1], coefs[i * 4 + 2], coefs[i * 4 + 3]], 2);
    }
    let mut out = [0i32; 16];
    for j in 0..4 {
        let r = iwht([t[0][j], t[1][j], t[2][j], t[3][j]], 0);
        for i in 0..4 {
            out[i * 4 + j] = r[i];
        }
    }
    out
}

/// Lossless forward transform: levels (dequantised by 4) whose inverse is exactly `res`.
fn forward_wht(res: &[i32; 16]) -> [i32; 16] {
    let mut t = [[0i32; 4]; 4];
    for j in 0..4 {
        let col = fwht([res[j], res[4 + j], res[8 + j], res[12 + j]]);
        for i in 0..4 {
            t[i][j] = col[i];
        }
    }
    let mut lv = [0i32; 16];
    for i in 0..4 {
        let r = fwht(t[i]);
        lv[i * 4..i * 4 + 4].copy_from_slice(&r);
    }
    lv
}

/// Forward transform matrices: the inverses of the decoder's (linear part of the) inverse
/// transforms, so `inverse4(forward(res)) ≈ res`.
fn forward_matrices() -> &'static [[[f64; 16]; 16]; 4] {
    use std::sync::OnceLock;
    static M: OnceLock<[[[f64; 16]; 16]; 4]> = OnceLock::new();
    M.get_or_init(|| {
        let mut out = [[[0.0; 16]; 16]; 4];
        for (ty, o) in out.iter_mut().enumerate() {
            // Columns of the inverse: response to a scaled unit coefficient.
            let k = 1 << 16;
            let mut inv = [[0.0f64; 16]; 16];
            for c in 0..16 {
                let mut e = [0i32; 16];
                e[c] = k;
                let r = inverse4(&e, ty as u8);
                for p in 0..16 {
                    inv[p][c] = r[p] as f64 / k as f64;
                }
            }
            *o = invert16(inv);
        }
        out
    })
}

fn invert16(m: [[f64; 16]; 16]) -> [[f64; 16]; 16] {
    let mut a = m;
    let mut inv = [[0.0; 16]; 16];
    for (i, r) in inv.iter_mut().enumerate() {
        r[i] = 1.0;
    }
    for col in 0..16 {
        let piv = (col..16).max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs())).unwrap_or(col);
        a.swap(col, piv);
        inv.swap(col, piv);
        let d = a[col][col];
        for j in 0..16 {
            a[col][j] /= d;
            inv[col][j] /= d;
        }
        for r in 0..16 {
            if r != col {
                let f = a[r][col];
                if f != 0.0 {
                    for j in 0..16 {
                        a[r][j] -= f * a[col][j];
                        inv[r][j] -= f * inv[col][j];
                    }
                }
            }
        }
    }
    inv
}

// ---------------------------------------------------------------- planes and prediction

struct Plane {
    w: usize,
    /// Last valid column / row for prediction edges (the 8-aligned mode-info grid).
    max_x: usize,
    max_y: usize,
    src: Vec<u8>,
    rec: Vec<u8>,
}

impl Plane {
    fn new(data: &[u8], vw: usize, vh: usize, w: usize, h: usize, max_x: usize, max_y: usize) -> Plane {
        let mut src = vec![0u8; w * h];
        for y in 0..h {
            let sy = y.min(vh.saturating_sub(1));
            for x in 0..w {
                let sx = x.min(vw.saturating_sub(1));
                src[y * w + x] = data.get(sy * vw + sx).copied().unwrap_or(128);
            }
        }
        Plane { w, max_x, max_y, src, rec: vec![0; w * h] }
    }
}

/// Intra prediction (8.5.1) of a square block for DC / V / H / TM.
fn predict(p: &Plane, x: usize, y: usize, log2: usize, mode: u8, have_left: bool, have_above: bool) -> [i32; 64] {
    let size = 1usize << log2;
    let base = 128i32;
    let buf = &p.rec;
    let w = p.w;
    let mut above = [0i32; 8];
    let mut left = [0i32; 8];
    if have_above {
        for i in 0..size {
            above[i] = buf[(y - 1) * w + (x + i).min(p.max_x)] as i32;
        }
    } else {
        above[..size].fill(base - 1);
    }
    let top_left = if have_above && have_left {
        buf[(y - 1) * w + (x - 1).min(p.max_x)] as i32
    } else if have_above {
        base + 1
    } else {
        base - 1
    };
    if have_left {
        for i in 0..size {
            left[i] = buf[(y + i).min(p.max_y) * w + x - 1] as i32;
        }
    } else {
        left[..size].fill(base + 1);
    }
    let mut out = [0i32; 64];
    for i in 0..size {
        for j in 0..size {
            out[i * size + j] = match mode {
                V_PRED => above[j],
                H_PRED => left[i],
                TM_PRED => (above[j] + left[i] - top_left).clamp(0, 255),
                _ => 0,
            };
        }
    }
    if mode == DC_PRED {
        let v = match (have_left, have_above) {
            (true, true) => (left[..size].iter().sum::<i32>() + above[..size].iter().sum::<i32>() + size as i32) >> (log2 + 1),
            (true, false) => (left[..size].iter().sum::<i32>() + (1 << (log2 - 1))) >> log2,
            (false, true) => (above[..size].iter().sum::<i32>() + (1 << (log2 - 1))) >> log2,
            (false, false) => base,
        };
        out[..size * size].fill(v);
    }
    out
}

// ---------------------------------------------------------------- the encoder

pub struct Vp9Encoder {
    cfg: EncoderConfig,
    qidx: u8,
    lossless: bool,
}

/// Per-tile coding state.
struct Tile<'a> {
    enc: &'a Vp9Encoder,
    bc: BoolEncoder,
    planes: &'a mut [Plane; 3],
    mi_cols: usize,
    mi_rows: usize,
    col_start: usize,
    above_nz: [Vec<u8>; 3],
    left_nz: [Vec<u8>; 3],
    above_part: Vec<u8>,
    left_part: Vec<u8>,
    /// y mode of every 8×8 block (for mode contexts).
    modes: &'a mut [u8],
}

impl Vp9Encoder {
    pub fn new(cfg: EncoderConfig) -> Self {
        let q = cfg.quality.clamp(1, 100);
        let lossless = q >= 100;
        // Quality → base_q_idx: 99 ≈ 3, 90 ≈ 30, 50 ≈ 150, 1 ≈ 255.
        let qidx = if lossless { 0 } else { ((100 - q as i32) * 255 / 85).clamp(1, 255) as u8 };
        Vp9Encoder { cfg, qidx, lossless }
    }

    pub fn config(&self) -> &EncoderConfig {
        &self.cfg
    }

    /// Encode one key frame from planar 8-bit 4:2:0 (`y`: width×height, `u`/`v`:
    /// ((w+1)/2)×((h+1)/2), tightly packed).
    pub fn encode_yuv420(&mut self, y: &[u8], u: &[u8], v: &[u8]) -> Vec<u8> {
        let (w, h) = (self.cfg.width.max(1) as usize, self.cfg.height.max(1) as usize);
        let mi_cols = w.div_ceil(8);
        let mi_rows = h.div_ceil(8);
        let sb_cols = mi_cols.div_ceil(8);
        let sb_rows = mi_rows.div_ceil(8);
        let (pw, ph) = (sb_cols * 64, sb_rows * 64);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let mut planes = [
            Plane::new(y, w, h, pw, ph, mi_cols * 8 - 1, mi_rows * 8 - 1),
            Plane::new(u, cw, ch, pw / 2, ph / 2, mi_cols * 4 - 1, mi_rows * 4 - 1),
            Plane::new(v, cw, ch, pw / 2, ph / 2, mi_cols * 4 - 1, mi_rows * 4 - 1),
        ];
        // Tile columns: as few as the width allows (tile_info, 6.2.13).
        let mut min_log2 = 0;
        while (64usize << min_log2) < sb_cols {
            min_log2 += 1;
        }
        let mut max_log2 = 1;
        while (sb_cols >> max_log2) >= 4 {
            max_log2 += 1;
        }
        max_log2 -= 1;
        let tile_cols = 1usize << min_log2;
        let mut modes = vec![0u8; mi_cols * mi_rows];
        let mut tiles = vec![];
        for t in 0..tile_cols {
            let start = (((t * sb_cols) >> min_log2) * 8).min(mi_cols);
            let end = ((((t + 1) * sb_cols) >> min_log2) * 8).min(mi_cols);
            let mut tile = Tile {
                enc: self,
                bc: BoolEncoder::new(),
                planes: &mut planes,
                mi_cols,
                mi_rows,
                col_start: start,
                above_nz: [vec![0; mi_cols * 2 + 16], vec![0; mi_cols * 2 + 16], vec![0; mi_cols * 2 + 16]],
                left_nz: [vec![0; 16], vec![0; 16], vec![0; 16]],
                above_part: vec![0; mi_cols + 8],
                left_part: vec![0; 8],
                modes: &mut modes,
            };
            tile.bc.write(false, 128);
            for r in (0..mi_rows).step_by(8) {
                for p in 0..3 {
                    tile.left_nz[p].fill(0);
                }
                tile.left_part.fill(0);
                for c in (start..end).step_by(8) {
                    tile.partition(r, c, 12);
                }
            }
            tiles.push(tile.bc.finish());
        }
        // Compressed header.
        let mut ch_enc = BoolEncoder::new();
        ch_enc.write(false, 128);
        if !self.lossless {
            ch_enc.literal(0, 2); // tx_mode ONLY_4X4
        }
        ch_enc.literal(0, 1); // no coefficient probability updates (TX_4X4)
        for _ in 0..3 {
            ch_enc.write(false, 252); // skip probabilities unchanged
        }
        let compressed = ch_enc.finish();
        // Uncompressed header.
        let mut bw = BitWriter::default();
        bw.put(2, 2); // frame_marker
        bw.put(0, 1); // profile low bit
        bw.put(0, 1); // profile high bit
        bw.put(0, 1); // show_existing_frame
        bw.put(0, 1); // frame_type = KEY_FRAME
        bw.put(1, 1); // show_frame
        bw.put(0, 1); // error_resilient_mode
        bw.put(0x49, 8);
        bw.put(0x83, 8);
        bw.put(0x42, 8);
        bw.put(2, 3); // color_space CS_BT_709
        bw.put(self.cfg.full_range as u32, 1);
        bw.put(w as u32 - 1, 16);
        bw.put(h as u32 - 1, 16);
        bw.put(0, 1); // render_and_frame_size_different
        bw.put(0, 1); // refresh_frame_context
        bw.put(1, 1); // frame_parallel_decoding_mode
        bw.put(0, 2); // frame_context_idx
        bw.put(0, 6); // loop_filter_level
        bw.put(0, 3); // sharpness
        bw.put(0, 1); // mode_ref_delta_enabled
        bw.put(self.qidx as u32, 8);
        bw.put(0, 1); // delta_q_y_dc
        bw.put(0, 1); // delta_q_uv_dc
        bw.put(0, 1); // delta_q_uv_ac
        bw.put(0, 1); // segmentation_enabled
        if min_log2 < max_log2 {
            bw.put(0, 1); // no extra tile columns
        }
        bw.put(0, 1); // tile_rows_log2 = 0
        bw.put(compressed.len() as u32, 16);
        let mut out = bw.finish();
        out.extend_from_slice(&compressed);
        let n = tiles.len();
        for (i, t) in tiles.into_iter().enumerate() {
            if i + 1 < n {
                out.extend_from_slice(&(t.len() as u32).to_be_bytes());
            }
            out.extend_from_slice(&t);
        }
        out
    }
}

#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, v: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.acc = (self.acc << 1) | ((v >> i) & 1);
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc << (8 - self.n)) as u8);
        }
        self.out
    }
}

const BLOCK_8X8: u8 = 3;
const NUM_8X8: [usize; 13] = [1, 1, 1, 1, 1, 2, 2, 2, 4, 4, 4, 8, 8];
const MI_WIDTH_LOG2: [usize; 13] = [0, 0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3];

impl Tile<'_> {
    fn partition(&mut self, r: usize, c: usize, bsize: u8) {
        if r >= self.mi_rows || c >= self.mi_cols {
            return;
        }
        let num8 = NUM_8X8[bsize as usize];
        let half = num8 >> 1;
        let has_rows = (r + half) < self.mi_rows;
        let has_cols = (c + half) < self.mi_cols;
        let bsl = MI_WIDTH_LOG2[bsize as usize];
        let boffset = 3 - bsl;
        let mut above = 0u8;
        let mut left = 0u8;
        for i in 0..num8 {
            above |= self.above_part[c + i];
            left |= self.left_part[(r & 7) + i];
        }
        let above = (above >> boffset) & 1;
        let left = (left >> boffset) & 1;
        let ctx = bsl * 4 + left as usize * 2 + above as usize;
        let probs = &KF_PARTITION_PROBS[ctx * 3..ctx * 3 + 3];
        if bsize == BLOCK_8X8 {
            // PARTITION_NONE.
            self.bc.write(false, probs[0]);
            self.block(r, c);
            self.above_part[c] = 15 >> 1;
            self.left_part[r & 7] = 15 >> 1;
            return;
        }
        // PARTITION_SPLIT.
        if has_rows && has_cols {
            self.bc.tree(&PARTITION_TREE, probs, 3);
        } else if has_cols {
            self.bc.write(true, probs[1]);
        } else if has_rows {
            self.bc.write(true, probs[2]);
        }
        let sub = bsize - 3;
        self.partition(r, c, sub);
        self.partition(r, c + half, sub);
        self.partition(r + half, c, sub);
        self.partition(r + half, c + half, sub);
    }

    fn block(&mut self, r: usize, c: usize) {
        let avail_u = r > 0;
        let avail_l = c > self.col_start;
        // Mode decision on the 8×8 luma block and the 4×4 chroma blocks.
        let y_mode = self.choose(0, c * 8, r * 8, 3, avail_l, avail_u);
        let uv_mode = {
            let mut best = (i64::MAX, DC_PRED);
            for m in MODES {
                let cost: i64 = (1..3).map(|p| self.cost(p, c * 4, r * 4, 2, m, avail_l, avail_u)).sum();
                if cost < best.0 {
                    best = (cost, m);
                }
            }
            best.1
        };
        // skip = 0 (context: neighbours' skip, always 0 here); tx_size implied (4×4).
        self.bc.write(false, DEFAULT_SKIP_PROB[0]);
        let above_mode = if avail_u { self.modes[(r - 1) * self.mi_cols + c] } else { DC_PRED };
        let left_mode = if avail_l { self.modes[r * self.mi_cols + c - 1] } else { DC_PRED };
        let o = (above_mode as usize * 10 + left_mode as usize) * 9;
        self.bc.tree(&INTRA_MODE_TREE, &KF_Y_MODE_PROBS[o..o + 9], y_mode);
        let o = y_mode as usize * 9;
        self.bc.tree(&INTRA_MODE_TREE, &KF_UV_MODE_PROBS[o..o + 9], uv_mode);
        self.modes[r * self.mi_cols + c] = y_mode;
        // Residual: luma 2×2 transform blocks, then one 4×4 block per chroma plane.
        for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let tx = if self.enc.lossless { DCT_DCT } else { mode_tx_type(y_mode) };
            self.tx_block(0, c * 8 + dx * 4, r * 8 + dy * 4, y_mode, tx, avail_l || dx > 0, avail_u || dy > 0);
        }
        for p in 1..3 {
            self.tx_block(p, c * 4, r * 4, uv_mode, DCT_DCT, avail_l, avail_u);
        }
    }

    fn cost(&self, plane: usize, x: usize, y: usize, log2: usize, mode: u8, have_left: bool, have_above: bool) -> i64 {
        let p = &self.planes[plane];
        let pred = predict(p, x, y, log2, mode, have_left, have_above);
        let size = 1 << log2;
        let mut s = 0i64;
        for i in 0..size {
            for j in 0..size {
                s += (p.src[(y + i) * p.w + x + j] as i32 - pred[i * size + j]).abs() as i64;
            }
        }
        s
    }

    fn choose(&self, plane: usize, x: usize, y: usize, log2: usize, have_left: bool, have_above: bool) -> u8 {
        let mut best = (i64::MAX, DC_PRED);
        for m in MODES {
            let cost = self.cost(plane, x, y, log2, m, have_left, have_above);
            if cost < best.0 {
                best = (cost, m);
            }
        }
        best.1
    }

    #[allow(clippy::too_many_arguments)]
    fn tx_block(&mut self, plane: usize, x: usize, y: usize, mode: u8, tx_type: u8, have_left: bool, have_above: bool) {
        let ss = (plane > 0) as usize;
        let max_x = (self.mi_cols * 8) >> ss;
        let max_y = (self.mi_rows * 8) >> ss;
        let (x4, y4) = (x >> 2, y >> 2);
        let ly = y4 & (15 >> ss);
        if x >= max_x || y >= max_y {
            self.above_nz[plane][x4] = 0;
            self.left_nz[plane][ly] = 0;
            return;
        }
        let p = &self.planes[plane];
        let pred = predict(p, x, y, 2, mode, have_left, have_above);
        let mut res = [0i32; 16];
        for i in 0..4 {
            for j in 0..4 {
                res[i * 4 + j] = p.src[(y + i) * p.w + x + j] as i32 - pred[i * 4 + j];
            }
        }
        let (dcq, acq) = (DC_QLOOKUP[self.enc.qidx as usize], AC_QLOOKUP[self.enc.qidx as usize]);
        // Quantised levels (raster order).
        let levels: [i32; 16] = if self.enc.lossless {
            forward_wht(&res)
        } else {
            let f = &forward_matrices()[tx_type as usize];
            let mut lv = [0i32; 16];
            for k in 0..16 {
                let coef: f64 = (0..16).map(|q| f[k][q] * res[q] as f64).sum();
                let qs = if k == 0 { dcq } else { acq } as f64;
                let a = coef.abs() / qs;
                // Rounding with a small dead zone for AC coefficients.
                let l = if k == 0 { (a + 0.5).floor() } else { (a + 0.38).floor() };
                lv[k] = if coef < 0.0 { -(l as i32) } else { l as i32 };
            }
            lv
        };
        let sc = scan(tx_type);
        let eob = (0..16).rev().find(|&i| levels[sc[i] as usize] != 0).map_or(0, |i| i + 1);
        // Tokens.
        let ctx0 = (self.above_nz[plane][x4] + self.left_nz[plane][ly]) as usize;
        self.tokens(plane, tx_type, &levels, eob, ctx0);
        // Reconstruct exactly as the decoder does.
        let mut deq = [0i32; 16];
        for k in 0..16 {
            deq[k] = levels[k] * if k == 0 { dcq } else { acq };
        }
        let resid = if eob == 0 {
            [0i32; 16]
        } else if self.enc.lossless {
            inverse_wht(&deq)
        } else {
            inverse4(&deq, tx_type)
        };
        let p = &mut self.planes[plane];
        for i in 0..4 {
            for j in 0..4 {
                p.rec[(y + i) * p.w + x + j] = (pred[i * 4 + j] + resid[i * 4 + j]).clamp(0, 255) as u8;
            }
        }
        let nz = (eob > 0) as u8;
        self.above_nz[plane][x4] = nz;
        self.left_nz[plane][ly] = nz;
    }

    fn tokens(&mut self, plane: usize, tx_type: u8, levels: &[i32; 16], eob: usize, ctx0: usize) {
        let sc = scan(tx_type);
        let nb = neighbors(tx_type);
        let ptype = (plane > 0) as usize;
        // coef_probs[TX_4X4][ptype][intra]: 6 bands × 6 contexts × 3.
        let base = (ptype * 2) * 6 * 6 * 3;
        let mut tc = [0u8; 16];
        let mut check_eob = true;
        let mut c = 0;
        while c < 16 {
            let pos = sc[c] as usize;
            let band = COEFBAND_4X4[c] as usize;
            let ctx = if c == 0 { ctx0 } else { ((1 + tc[nb[c].0] + tc[nb[c].1]) >> 1) as usize };
            let o = base + (band * 6 + ctx) * 3;
            let p = [DEFAULT_COEF_PROBS[o], DEFAULT_COEF_PROBS[o + 1], DEFAULT_COEF_PROBS[o + 2]];
            if check_eob {
                let more = c < eob;
                self.bc.write(more, p[0]);
                if !more {
                    break;
                }
            }
            let v = levels[pos].unsigned_abs();
            if v == 0 {
                self.bc.write(false, p[1]);
                tc[pos] = 0;
                check_eob = false;
                c += 1;
                continue;
            }
            self.bc.write(true, p[1]);
            check_eob = true;
            let token = match v {
                1 => 1,
                2 => 2,
                3 => 3,
                4 => 4,
                5..=6 => 5,
                7..=10 => 6,
                11..=18 => 7,
                19..=34 => 8,
                35..=66 => 9,
                _ => 10,
            };
            if token == 1 {
                self.bc.write(false, p[2]);
            } else {
                self.bc.write(true, p[2]);
                let pp = pareto(p[2]);
                let path: &[(usize, bool)] = match token {
                    2 => &[(0, false), (1, false)],
                    3 => &[(0, false), (1, true), (2, false)],
                    4 => &[(0, false), (1, true), (2, true)],
                    5 => &[(0, true), (3, false), (4, false)],
                    6 => &[(0, true), (3, false), (4, true)],
                    7 => &[(0, true), (3, true), (5, false), (6, false)],
                    8 => &[(0, true), (3, true), (5, false), (6, true)],
                    9 => &[(0, true), (3, true), (5, true), (7, false)],
                    _ => &[(0, true), (3, true), (5, true), (7, true)],
                };
                for &(n, b) in path {
                    self.bc.write(b, pp[n]);
                }
                let (cat, num_extra, base_v) = EXTRA_BITS[token];
                let extra = (v - base_v as u32).min((1 << num_extra) - 1);
                let cp = CAT_PROBS[cat as usize];
                for e in 0..num_extra as usize {
                    self.bc.write((extra >> (num_extra as usize - 1 - e)) & 1 != 0, cp[e]);
                }
            }
            tc[pos] = ENERGY_CLASS[token];
            self.bc.write(levels[pos] < 0, 128);
            c += 1;
        }
    }
}

// ---------------------------------------------------------------- colour conversion

/// BT.709 RGB(A) 8-bit → planar YUV 4:2:0 (limited or full range).
pub fn rgba_to_yuv420(rgba: &[u8], w: u32, h: u32, full_range: bool) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (w, h) = (w as usize, h as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut y = vec![0u8; w * h];
    let mut u = vec![0u8; cw * ch];
    let mut v = vec![0u8; cw * ch];
    let (ys, yo, cs) = if full_range { (255.0, 0.0, 255.0) } else { (219.0, 16.0, 224.0) };
    let px = |x: usize, yy: usize| -> [f32; 3] {
        let i = (yy.min(h - 1) * w + x.min(w - 1)) * 4;
        [rgba[i] as f32 / 255.0, rgba[i + 1] as f32 / 255.0, rgba[i + 2] as f32 / 255.0]
    };
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    for yy in 0..h {
        for x in 0..w {
            y[yy * w + x] = (luma(px(x, yy)) * ys + yo).round().clamp(0.0, 255.0) as u8;
        }
    }
    for cy in 0..ch {
        for cx in 0..cw {
            let mut acc = [0.0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let c = px(cx * 2 + dx, cy * 2 + dy);
                for k in 0..3 {
                    acc[k] += c[k] / 4.0;
                }
            }
            let l = luma(acc);
            let cb = (acc[2] - l) / 1.8556;
            let cr = (acc[0] - l) / 1.5748;
            u[cy * cw + cx] = (cb * cs + 128.0).round().clamp(0.0, 255.0) as u8;
            v[cy * cw + cx] = (cr * cs + 128.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    (y, u, v)
}

/// The alpha channel as a luma plane (full range) with neutral chroma — the second VP9 stream of
/// WebM alpha.
pub fn alpha_to_yuv420(rgba: &[u8], w: u32, h: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (w, h) = (w as usize, h as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let y: Vec<u8> = rgba.chunks_exact(4).take(w * h).map(|p| p[3]).collect();
    (y, vec![128; cw * ch], vec![128; cw * ch])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wht_is_exactly_invertible() {
        let mut s = 7u32;
        for _ in 0..2000 {
            let mut r = [0i32; 16];
            for v in &mut r {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                *v = (s >> 16) as i32 % 511 - 255;
            }
            let lv = forward_wht(&r);
            let deq = lv.map(|l| l * 4);
            assert_eq!(inverse_wht(&deq), r);
        }
    }

    #[test]
    fn forward_transforms_invert_the_decoder() {
        for ty in 0..4u8 {
            let f = &forward_matrices()[ty as usize];
            let r: [i32; 16] = std::array::from_fn(|i| (i as i32 * 37 % 61) - 30);
            let c: [i32; 16] = std::array::from_fn(|k| (0..16).map(|q| f[k][q] * r[q] as f64).sum::<f64>().round() as i32);
            let back = inverse4(&c, ty);
            for i in 0..16 {
                assert!((back[i] - r[i]).abs() <= 1, "type {ty}: {back:?} vs {r:?}");
            }
        }
    }
}
