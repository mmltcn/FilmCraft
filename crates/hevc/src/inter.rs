//! Fractional sample interpolation (8.5.3.3.3) and weighted sample prediction (8.5.3.3.4).

use crate::picture::Frame;
use crate::spec_tables::{CHROMA_FILTER, LUMA_FILTER};

/// Max prediction block size + filter margin.
const WIN: usize = 64 + 7;

/// Reference windows and first-pass rows, reused between blocks (no per-block zeroing).
pub struct McScratch {
    win: Vec<i16>,
    tmp: Vec<i16>,
}

impl Default for McScratch {
    fn default() -> Self {
        McScratch { win: vec![0; WIN * WIN], tmp: vec![0; WIN * WIN] }
    }
}

/// `out[j] = (sum_k f[k] * src[j + k * step]) >> shift` for `W` outputs: taps outer, outputs
/// inner, so each tap is one vector multiply-add across the row (exact integer sums).
#[inline(always)]
fn filt_w<const T: usize, const W: usize>(src: &[i16], step: usize, f: &[i8; T], shift: u32, out: &mut [i16]) {
    let mut acc = [0i32; W];
    for k in 0..T {
        let c = f[k] as i32;
        let Some(p) = src.get(k * step..).and_then(|s| s.first_chunk::<W>()) else {
            return;
        };
        for j in 0..W {
            acc[j] += c * p[j] as i32;
        }
    }
    let Some(o) = out.first_chunk_mut::<W>() else {
        return;
    };
    for j in 0..W {
        o[j] = (acc[j] >> shift) as i16;
    }
}

/// [`filt_w`] in 16-bit lanes, for a first pass over 8-bit samples (`shift` = 0): the filtered
/// value is then at most 88 * 255 and at least -24 * 255 (the positive / negative taps of any
/// luma or chroma filter, 8.5.3.3.3.1 / 8.5.3.3.3.2), so it fits in an `i16`, and wrapping sums
/// are exact whenever the final value fits. Twice the lanes of [`filt_w`] per vector.
#[inline(always)]
fn filt16_w<const T: usize, const W: usize>(src: &[i16], step: usize, f: &[i8; T], out: &mut [i16]) {
    let mut acc = [0i16; W];
    for k in 0..T {
        let c = f[k] as i16;
        let Some(p) = src.get(k * step..).and_then(|s| s.first_chunk::<W>()) else {
            return;
        };
        for j in 0..W {
            acc[j] = acc[j].wrapping_add(c.wrapping_mul(p[j]));
        }
    }
    if let Some(o) = out.first_chunk_mut::<W>() {
        *o = acc;
    }
}

/// [`filt_w`] for a run of `w` outputs (prediction block widths are fixed sizes); in 16-bit lanes
/// ([`filt16_w`]) when `narrow` (a first pass over 8-bit samples).
#[inline(always)]
fn filt<const T: usize>(src: &[i16], step: usize, f: &[i8; T], shift: u32, out: &mut [i16], w: usize, narrow: bool) {
    if narrow {
        match w {
            4 => return filt16_w::<T, 4>(src, step, f, out),
            8 => return filt16_w::<T, 8>(src, step, f, out),
            12 => return filt16_w::<T, 12>(src, step, f, out),
            16 => return filt16_w::<T, 16>(src, step, f, out),
            24 => return filt16_w::<T, 24>(src, step, f, out),
            32 => return filt16_w::<T, 32>(src, step, f, out),
            48 => return filt16_w::<T, 48>(src, step, f, out),
            64 => return filt16_w::<T, 64>(src, step, f, out),
            _ => {}
        }
    }
    match w {
        2 => filt_w::<T, 2>(src, step, f, shift, out),
        4 => filt_w::<T, 4>(src, step, f, shift, out),
        6 => filt_w::<T, 6>(src, step, f, shift, out),
        8 => filt_w::<T, 8>(src, step, f, shift, out),
        12 => filt_w::<T, 12>(src, step, f, shift, out),
        16 => filt_w::<T, 16>(src, step, f, shift, out),
        24 => filt_w::<T, 24>(src, step, f, shift, out),
        32 => filt_w::<T, 32>(src, step, f, shift, out),
        48 => filt_w::<T, 48>(src, step, f, shift, out),
        64 => filt_w::<T, 64>(src, step, f, shift, out),
        _ => {
            for (j, o) in out[..w].iter_mut().enumerate() {
                let s: i32 = (0..T).map(|k| f[k] as i32 * src[j + k * step] as i32).sum();
                *o = (s >> shift) as i16;
            }
        }
    }
}

/// Separable sub-sample interpolation of a w x h block from `win` (stride WIN, the block's
/// top-left sample at (m, m) for a T-tap filter with m = T / 2 - 1).
#[allow(clippy::too_many_arguments)]
fn interpolate<const T: usize>(
    win: &[i16],
    tmp: &mut [i16],
    fx: usize,
    fy: usize,
    hf: &[i8; T],
    vf: &[i8; T],
    w: usize,
    h: usize,
    shift1: u32,
    shift3: u32,
    out: &mut [i16],
) {
    let m = T / 2 - 1;
    // 8-bit samples: first passes in 16-bit lanes (see filt16_w)
    let narrow = shift1 == 0 && shift3 == 6;
    if fx == 0 && fy == 0 {
        for r in 0..h {
            for (o, &s) in out[r * w..r * w + w].iter_mut().zip(&win[(r + m) * WIN + m..]) {
                *o = s << shift3;
            }
        }
    } else if fy == 0 {
        for r in 0..h {
            filt(&win[(r + m) * WIN..], 1, hf, shift1, &mut out[r * w..], w, narrow);
        }
    } else if fx == 0 {
        for r in 0..h {
            filt(&win[r * WIN + m..], WIN, vf, shift1, &mut out[r * w..], w, narrow);
        }
    } else {
        for r in 0..h + T - 1 {
            filt(&win[r * WIN..], 1, hf, shift1, &mut tmp[r * WIN..], w, narrow);
        }
        for r in 0..h {
            filt(&tmp[r * WIN..], WIN, vf, 6, &mut out[r * w..], w, false);
        }
    }
}

/// Luma prediction samples (14-bit intermediate precision) of a w x h block at (x, y) displaced by the
/// quarter-sample motion vector `mv`, into `out` (stride w).
#[allow(clippy::too_many_arguments)]
pub fn mc_luma(f: &Frame, x: i32, y: i32, mv: [i16; 2], w: usize, h: usize, out: &mut [i16], sc: &mut McScratch) {
    let bd = f.bit_depth;
    let (fx, fy) = ((mv[0] & 3) as usize, (mv[1] & 3) as usize);
    let xi = x + (mv[0] as i32 >> 2);
    let yi = y + (mv[1] as i32 >> 2);
    let shift1 = bd.min(12) - 8;
    let shift3 = 14 - bd;
    f.luma_window(xi - 3, yi - 3, w + 7, h + 7, &mut sc.win, WIN);
    interpolate::<8>(&sc.win, &mut sc.tmp, fx, fy, &LUMA_FILTER[fx], &LUMA_FILTER[fy], w, h, shift1, shift3, out);
}

/// Chroma prediction samples of a w x h chroma block at chroma position (x, y) with a 1/8-sample
/// vector (4:2:0).
#[allow(clippy::too_many_arguments)]
pub fn mc_chroma(f: &Frame, c: usize, x: i32, y: i32, mv: [i16; 2], w: usize, h: usize, out: &mut [i16], sc: &mut McScratch) {
    let bd = f.bit_depth_c;
    let (fx, fy) = ((mv[0] & 7) as usize, (mv[1] & 7) as usize);
    let xi = x + (mv[0] as i32 >> 3);
    let yi = y + (mv[1] as i32 >> 3);
    let shift1 = bd.min(12) - 8;
    let shift3 = 14 - bd;
    f.chroma_window(c, xi - 1, yi - 1, w + 3, h + 3, &mut sc.win, WIN);
    interpolate::<4>(&sc.win, &mut sc.tmp, fx, fy, &CHROMA_FILTER[fx], &CHROMA_FILTER[fy], w, h, shift1, shift3, out);
}

/// Calls `$f::<W>(args)` with `W` the prediction block width `$w` as a constant (block widths are
/// fixed sizes; a constant width lets the row loops vectorise even for 4- and 8-sample rows), or
/// `$f::<0>` for any other width.
macro_rules! by_width {
    ($w:expr, $f:ident($($a:expr),*)) => {
        match $w {
            2 => $f::<2>($($a),*),
            4 => $f::<4>($($a),*),
            6 => $f::<6>($($a),*),
            8 => $f::<8>($($a),*),
            12 => $f::<12>($($a),*),
            16 => $f::<16>($($a),*),
            24 => $f::<24>($($a),*),
            32 => $f::<32>($($a),*),
            48 => $f::<48>($($a),*),
            64 => $f::<64>($($a),*),
            _ => $f::<0>($($a),*),
        }
    };
}

/// Default weighted prediction, one list (8-262).
pub fn put_uni(p: &[i16], w: usize, h: usize, bd: u32, dst: &mut [u16], ds: usize) {
    by_width!(w, put_uni_w(p, w, h, bd, dst, ds))
}

/// [`put_uni`] for rows of `W` samples (`W` = 0: rows of `w`).
///
/// In 16-bit lanes: prediction samples are 14-bit values (8.5.3.3.3: at most about +/-27000 for any
/// bit depth, as the reference samples are within `0..2^bd`), so `s + off` and the clipped result fit
/// in an `i16` and the sums are exact. SSE2 clips 16-bit lanes in one instruction each, 32-bit lanes
/// in four.
#[inline(always)]
fn put_uni_w<const W: usize>(p: &[i16], w: usize, h: usize, bd: u32, dst: &mut [u16], ds: usize) {
    let w = if W > 0 { W } else { w };
    let shift = 14 - bd;
    let off: i16 = if shift > 0 { 1 << (shift - 1) } else { 0 };
    let max = ((1i32 << bd) - 1) as i16;
    // Rows as slices and min / max clipping (`clamp` asserts its bounds) so the loop vectorises.
    for (d, s) in dst.chunks_mut(ds).zip(p[..w * h].chunks(w)) {
        for (d, &s) in d[..w].iter_mut().zip(s) {
            *d = (s.wrapping_add(off) >> shift).max(0).min(max) as u16;
        }
    }
}

/// Default weighted prediction, bi-prediction (8-263).
pub fn put_bi(p0: &[i16], p1: &[i16], w: usize, h: usize, bd: u32, dst: &mut [u16], ds: usize) {
    by_width!(w, put_bi_w(p0, p1, w, h, bd, dst, ds))
}

/// [`put_bi`] for rows of `W` samples (`W` = 0: rows of `w`).
///
/// In 16-bit lanes, exactly: with `off = 2^(shift - 1)` even, `(a + b + off) >> shift` equals
/// `((a >> 1) + (b >> 1) + (a & b & 1) + off / 2) >> (shift - 1)` (the two dropped low bits only
/// carry when both are set), and `(a >> 1) + (b >> 1)` fits in an `i16` (see [`put_uni_w`]).
#[inline(always)]
fn put_bi_w<const W: usize>(p0: &[i16], p1: &[i16], w: usize, h: usize, bd: u32, dst: &mut [u16], ds: usize) {
    let w = if W > 0 { W } else { w };
    let shift = 15 - bd; // >= 3 (bd <= 12)
    let half_off: i16 = 1 << (shift - 2);
    let max = ((1i32 << bd) - 1) as i16;
    for ((d, a), b) in dst.chunks_mut(ds).zip(p0[..w * h].chunks(w)).zip(p1[..w * h].chunks(w)) {
        for ((d, &a), &b) in d[..w].iter_mut().zip(a).zip(b) {
            let v = (a >> 1).wrapping_add(b >> 1).wrapping_add(a & b & 1).wrapping_add(half_off) >> (shift - 1);
            *d = v.max(0).min(max) as u16;
        }
    }
}

/// Explicit weighted prediction, one list (8-265). `o` is the offset already scaled to the bit depth.
pub fn put_weighted_uni(p: &[i16], w: usize, h: usize, bd: u32, log2wd: u32, wt: i32, o: i32, dst: &mut [u16], ds: usize) {
    let max = (1i32 << bd) - 1;
    for r in 0..h {
        for c in 0..w {
            let v = p[r * w + c] as i32 * wt;
            let v = if log2wd >= 1 { ((v + (1 << (log2wd - 1))) >> log2wd) + o } else { v + o };
            dst[r * ds + c] = v.max(0).min(max) as u16;
        }
    }
}

/// Explicit weighted bi-prediction (8-266).
pub fn put_weighted_bi(p0: &[i16], p1: &[i16], w: usize, h: usize, bd: u32, log2wd: u32, w0: i32, w1: i32, o0: i32, o1: i32, dst: &mut [u16], ds: usize) {
    let max = (1i32 << bd) - 1;
    for r in 0..h {
        for c in 0..w {
            let i = r * w + c;
            let v = (p0[i] as i32 * w0 + p1[i] as i32 * w1 + ((o0 + o1 + 1) << log2wd)) >> (log2wd + 1);
            dst[r * ds + c] = v.max(0).min(max) as u16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo + 1) as u64) as i32
        }
    }

    const WIDTHS: [usize; 11] = [2, 4, 6, 8, 12, 16, 24, 32, 48, 64, 3];

    /// The 16-bit-lane `put_uni` / `put_bi` give exactly the 32-bit formulas of 8-262 / 8-263 over
    /// the whole range of 14-bit prediction samples, for every block width and bit depth.
    #[test]
    fn default_weighted_prediction_matches_the_32_bit_formulas() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for bd in [8u32, 10, 12] {
            let max = (1i32 << bd) - 1;
            for &w in &WIDTHS {
                let h = 3;
                let sample = |rng: &mut Rng| -> i16 {
                    match rng.next() % 6 {
                        0 => -10_240,
                        1 => 26_624,
                        2 => rng.range(-1, 1) as i16,
                        _ => rng.range(-10_240, 26_624) as i16,
                    }
                };
                let p0: Vec<i16> = (0..w * h).map(|_| sample(&mut rng)).collect();
                let p1: Vec<i16> = (0..w * h).map(|_| sample(&mut rng)).collect();
                let ds = w + 5;
                let (mut uni, mut bi) = (vec![0u16; ds * h], vec![0u16; ds * h]);
                put_uni(&p0, w, h, bd, &mut uni, ds);
                put_bi(&p0, &p1, w, h, bd, &mut bi, ds);
                for r in 0..h {
                    for c in 0..w {
                        let (a, b) = (p0[r * w + c] as i32, p1[r * w + c] as i32);
                        let s1 = 14 - bd;
                        let o1 = if s1 > 0 { 1 << (s1 - 1) } else { 0 };
                        assert_eq!(uni[r * ds + c] as i32, ((a + o1) >> s1).clamp(0, max), "uni bd {bd} w {w} a {a}");
                        let s2 = 15 - bd;
                        assert_eq!(bi[r * ds + c] as i32, ((a + b + (1 << (s2 - 1))) >> s2).clamp(0, max), "bi bd {bd} w {w} a {a} b {b}");
                    }
                }
            }
        }
    }

    /// First passes over 8-bit samples in 16-bit lanes equal the 32-bit sums, including the
    /// extreme sample patterns of every filter.
    #[test]
    fn narrow_first_pass_matches_the_32_bit_filter() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        fn check<const T: usize>(f: &[i8; T], src: &[i16], step: usize, w: usize) {
            let (mut a, mut b) = ([0i16; 64], [0i16; 64]);
            filt(src, step, f, 0, &mut a, w, true);
            filt(src, step, f, 0, &mut b, w, false);
            assert_eq!(a[..w], b[..w], "filter {f:?} step {step} w {w}");
        }
        for &w in &WIDTHS {
            for step in [1usize, WIN] {
                let len = w + 7 * step + 8;
                let mut patterns: Vec<Vec<i16>> = (0..20).map(|_| (0..len).map(|_| rng.range(0, 255) as i16).collect()).collect();
                patterns.push(vec![255; len]);
                patterns.push(vec![0; len]);
                for f in LUMA_FILTER.iter() {
                    // 255 under positive taps, 0 under negative ones (maximum), and the reverse
                    for sign in [1, -1] {
                        let mut v = vec![0i16; len];
                        for (k, &c) in f.iter().enumerate() {
                            for j in 0..w {
                                v[j + k * step] = if (c as i32) * sign > 0 { 255 } else { 0 };
                            }
                        }
                        check(f, &v, step, w);
                    }
                    for p in &patterns {
                        check(f, p, step, w);
                    }
                }
                for f in CHROMA_FILTER.iter() {
                    for p in &patterns {
                        check(f, p, step, w);
                    }
                }
            }
        }
    }

    fn frame(rng: &mut Rng, w: usize, h: usize, bd: u32) -> Frame {
        let like = Frame::new(0, 0, w, h, 4, bd, bd);
        let max = (1i32 << bd) - 1;
        let y: Vec<u16> = (0..w * h).map(|_| rng.range(0, max) as u16).collect();
        let c: Vec<u16> = (0..w * h / 4).map(|_| rng.range(0, max) as u16).collect();
        let c2: Vec<u16> = (0..w * h / 4).map(|_| rng.range(0, max) as u16).collect();
        Frame::from_planes(1, 0, &like, &y, &c, &c2)
    }

    /// The full-sample uni-prediction shortcut (`Frame::copy_block`) gives what interpolation and
    /// 8-262 give, inside the picture and across every edge.
    #[test]
    fn full_sample_copy_matches_interpolation() {
        let mut rng = Rng(0x0123_4567_89ab_cdef);
        let mut sc = McScratch::default();
        for bd in [8u32, 10] {
            let f = frame(&mut rng, 48, 40, bd);
            for _ in 0..300 {
                let (w, h) = ([4usize, 8, 12, 16, 24, 32][rng.range(0, 5) as usize], [4usize, 8, 16][rng.range(0, 2) as usize]);
                let (x, y) = (rng.range(-40, 80), rng.range(-40, 70));
                let mut pred = vec![0i16; w * h];
                let (mut a, mut b) = (vec![0u16; w * h], vec![0u16; w * h]);
                inter_luma(&f, x, y, w, h, &mut pred, &mut sc);
                put_uni(&pred, w, h, bd, &mut a, w);
                f.copy_block(0, x, y, w, h, &mut b, w);
                assert_eq!(a, b, "luma bd {bd} at ({x}, {y}) {w}x{h}");
                for c in 1..3 {
                    let (cw, ch) = (w / 2, h / 2);
                    mc_chroma(&f, c - 1, 0, 0, [(x * 8) as i16, (y * 8) as i16], cw, ch, &mut pred, &mut sc);
                    let (mut a, mut b) = (vec![0u16; cw * ch], vec![0u16; cw * ch]);
                    put_uni(&pred, cw, ch, bd, &mut a, cw);
                    f.copy_block(c, x, y, cw, ch, &mut b, cw);
                    assert_eq!(a, b, "chroma {c} bd {bd} at ({x}, {y}) {cw}x{ch}");
                }
            }
        }
    }

    fn inter_luma(f: &Frame, x: i32, y: i32, w: usize, h: usize, out: &mut [i16], sc: &mut McScratch) {
        mc_luma(f, 0, 0, [(x * 4) as i16, (y * 4) as i16], w, h, out, sc);
    }
}
