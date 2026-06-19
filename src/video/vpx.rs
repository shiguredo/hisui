use shiguredo_mp4::{
    Uint,
    boxes::{SampleEntry, Vp08Box, Vp09Box, VpccBox},
};

use crate::video;

// Hisui 固有の固定値 (VP8 / VP9 共通)
const CHROMA_SUBSAMPLING_I420: Uint<u8, 3, 1> = Uint::new(1); // 4:2:0 colocated with luma (0,0)
const BIT_DEPTH: Uint<u8, 4, 4> = Uint::new(8);
const LEGAL_RANGE: Uint<u8, 1> = Uint::new(0); // 典型的な値。必要に応じて調整する
const BT_709: u8 = 1; // 典型的な値。必要に応じて調整する

/// VP8 用の sample_entry を構築する。
///
/// profile / level / bit_depth 等は Hisui の固定値を使う。
pub fn vp8_sample_entry(width: usize, height: usize) -> SampleEntry {
    SampleEntry::Vp08(Vp08Box {
        visual: video::sample_entry_visual_fields(width, height),
        vpcc_box: VpccBox {
            // Hisui 固有の固定値 (VP8 / VP9 共通)
            bit_depth: BIT_DEPTH,
            chroma_subsampling: CHROMA_SUBSAMPLING_I420,
            video_full_range_flag: LEGAL_RANGE,
            colour_primaries: BT_709,
            transfer_characteristics: BT_709,
            matrix_coefficients: BT_709,

            // VP8 では以下の値は常に固定値
            profile: 0,
            level: 0,
            codec_initialization_data: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    })
}

/// VP9 用の sample_entry を構築する。
///
/// profile / level / bit_depth 等は Hisui の固定値を使う。
pub fn vp9_sample_entry(width: usize, height: usize) -> SampleEntry {
    SampleEntry::Vp09(Vp09Box {
        visual: video::sample_entry_visual_fields(width, height),
        vpcc_box: VpccBox {
            profile: 0, // 0 は "8bit color depth, chroma-subsampling-4:2:0" を意味する
            level: 0,   // 適切な値を指定するのは大変なので undefined 扱いにしておく

            // Hisui 固有の固定値 (VP8 / VP9 共通)
            bit_depth: BIT_DEPTH,
            chroma_subsampling: CHROMA_SUBSAMPLING_I420,
            video_full_range_flag: LEGAL_RANGE,
            colour_primaries: BT_709,
            transfer_characteristics: BT_709,
            matrix_coefficients: BT_709,

            // VP9 では以下の値は常に固定値
            codec_initialization_data: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    })
}
