//! Intra prediction for 4x4, 8x8, 16x16 and 32x32 blocks (spec 7.11.2).
//!
//! Provides DC, Vertical, and Horizontal prediction modes.

pub const DC_PRED: u8 = 0;
pub const V_PRED: u8 = 1;
pub const H_PRED: u8 = 2;

/// Compute intra prediction for a block of size (w, h).
///
/// `above`: pixel row above the block (length >= w).
/// `left`: pixel column left of the block (length >= h).
/// `mode`: DC_PRED, V_PRED, or H_PRED.
pub fn predict_intra_block(mode: u8, w: usize, h: usize, above: Option<&[u8]>, left: Option<&[u8]>, dst: &mut [u8], dst_stride: usize) {
    match mode {
        V_PRED => {
            if let Some(ab) = above {
                for y in 0..h {
                    let row = &mut dst[y * dst_stride..y * dst_stride + w];
                    row.copy_from_slice(&ab[..w]);
                }
            } else {
                fill_dc(128, w, h, dst, dst_stride);
            }
        }
        H_PRED => {
            if let Some(lf) = left {
                for y in 0..h {
                    let val = lf[y];
                    let row = &mut dst[y * dst_stride..y * dst_stride + w];
                    row.fill(val);
                }
            } else {
                fill_dc(128, w, h, dst, dst_stride);
            }
        }
        _ => {
            // DC_PRED
            let dc_val = match (above, left) {
                (Some(ab), Some(lf)) => {
                    let sum: u32 = ab[..w].iter().map(|&x| x as u32).sum::<u32>() + lf[..h].iter().map(|&x| x as u32).sum::<u32>();
                    ((sum + ((w + h) as u32 >> 1)) / (w + h) as u32) as u8
                }
                (Some(ab), None) => {
                    let sum: u32 = ab[..w].iter().map(|&x| x as u32).sum();
                    ((sum + (w as u32 >> 1)) / w as u32) as u8
                }
                (None, Some(lf)) => {
                    let sum: u32 = lf[..h].iter().map(|&x| x as u32).sum();
                    ((sum + (h as u32 >> 1)) / h as u32) as u8
                }
                (None, None) => 128,
            };
            fill_dc(dc_val, w, h, dst, dst_stride);
        }
    }
}

fn fill_dc(val: u8, w: usize, h: usize, dst: &mut [u8], dst_stride: usize) {
    for y in 0..h {
        let row = &mut dst[y * dst_stride..y * dst_stride + w];
        row.fill(val);
    }
}
