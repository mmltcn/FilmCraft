//! Frame and tile header encoding (AV1 spec 5.9 / 5.11).

use crate::obu::SequenceConfig;
use filmcraft_bitstream::BitWriter;

pub const KEY_FRAME: u8 = 0;
pub const INTER_FRAME: u8 = 1;

/// Parameters for encoding an uncompressed frame header.
pub struct FrameHeaderConfig {
    pub frame_type: u8,
    pub show_frame: bool,
    pub error_resilient_mode: bool,
    pub disable_cdf_update: bool,
    pub qindex: u8,
}

/// Write uncompressed frame header (spec 5.9).
pub fn write_frame_header(cfg: &FrameHeaderConfig, seq: &SequenceConfig) -> Vec<u8> {
    let mut w = BitWriter::new();
    // 1. show_existing_frame = 0
    w.write_bit(false);
    // 2. frame_type: KEY_FRAME (0) or INTER_FRAME (1)
    w.write_bits(cfg.frame_type as u32, 2);
    // 3. show_frame
    w.write_bit(cfg.show_frame);

    let is_key = cfg.frame_type == KEY_FRAME;
    if !is_key || !cfg.show_frame {
        w.write_bit(cfg.error_resilient_mode);
    }

    // 4. disable_cdf_update
    w.write_bit(cfg.disable_cdf_update);

    // allow_screen_content_tools is inferred (0) from sequence header
    // force_integer_mv is inferred (1) for intra frame

    // 5. frame_size_override_flag = 0
    w.write_bit(false);
    // 6. order_hint = 0 (7 bits, matching order_hint_bits)
    w.write_bits(0, 7);

    if is_key {
        // render_and_frame_size_different = 0
        w.write_bit(false);
    }

    if !cfg.disable_cdf_update {
        // disable_frame_end_update_cdf
        w.write_bit(false);
    }

    // 8. tile_info (spec 5.9.15):
    // uniform_tile_spacing_flag = 1
    w.write_bit(true);
    let sb_cols = seq.width.div_ceil(64);
    let sb_rows = seq.height.div_ceil(64);
    let max_tile_cols_log2 = tile_log2(1, sb_cols.min(64));
    let max_tile_rows_log2 = tile_log2(1, sb_rows.min(64));
    if max_tile_cols_log2 > 0 {
        w.write_bit(false);
    }
    if max_tile_rows_log2 > 0 {
        w.write_bit(false);
    }

    // 9. Quantization params (spec 5.9.12):
    // base_q_idx (8 bits)
    w.write_bits(cfg.qindex as u32, 8);
    // delta_q_y_dc = 0, delta_q_u_dc = 0, delta_q_u_ac = 0
    w.write_bit(false);
    w.write_bit(false);
    w.write_bit(false);
    // using_qmatrix = 0
    w.write_bit(false);

    // 10. Segmentation:
    // segmentation_enabled = 0
    w.write_bit(false);

    // 11. Delta Q params:
    if cfg.qindex > 0 {
        // delta_q_present = 0
        w.write_bit(false);
    }

    // 12. Loop filter:
    let coded_lossless = cfg.qindex == 0;
    if !coded_lossless {
        // loop_filter_level[0] = 0, [1] = 0
        w.write_bits(0, 6);
        w.write_bits(0, 6);
        // loop_filter_sharpness = 0
        w.write_bits(0, 3);
        // loop_filter_delta_enabled = 0
        w.write_bit(false);
    }

    // 13. TX mode:
    if !coded_lossless {
        // tx_mode_select = 0 (TX_MODE_LARGEST)
        w.write_bit(false);
    }

    // 14. reduced_tx_set = 1
    w.write_bit(true);

    w.finish()
}

fn tile_log2(blk: u32, target: u32) -> u32 {
    let mut k = 0;
    while (blk << k) < target {
        k += 1;
    }
    k
}
