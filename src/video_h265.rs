use orfail::OrFail;
use shiguredo_mp4::boxes::{
    Hev1Box, HvccBox, HvccNalUintArray, SampleEntry, VisualSampleEntryFields,
};

use crate::{
    types::EvenUsize,
    video::{self, FrameRate},
    video_h264::{H265_NALU_TYPE_PPS, H265_NALU_TYPE_SPS, H265_NALU_TYPE_VPS, NALU_HEADER_LENGTH},
};

/// H.265 sample entry を生成する
pub fn h265_sample_entry(
    width: EvenUsize,
    height: EvenUsize,
    fps: FrameRate,
    vps_list: Vec<Vec<u8>>,
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
) -> orfail::Result<SampleEntry> {
    Ok(SampleEntry::Hev1(Hev1Box {
        visual: video::sample_entry_visual_fields(width.get(), height.get()),
        hvcc_box: HvccBox {
            // 以下はSora の録画ファイルに合わせた値（必要に応じて調整すること）
            general_profile_compatibility_flags: 0x60000000,
            general_constraint_indicator_flags: shiguredo_mp4::Uint::new(0xb00000000000),
            general_level_idc: 123,
            general_profile_space: shiguredo_mp4::Uint::new(0),
            general_tier_flag: shiguredo_mp4::Uint::new(0),
            num_temporal_layers: shiguredo_mp4::Uint::new(0),
            temporal_id_nested: shiguredo_mp4::Uint::new(0),
            min_spatial_segmentation_idc: shiguredo_mp4::Uint::new(0),
            parallelism_type: shiguredo_mp4::Uint::new(0),

            // Hisui ではフレームレートは固定（整数にならない場合は切り上げ）
            avg_frame_rate: (fps.numerator.get().div_ceil(fps.denumerator.get())) as u16,
            constant_frame_rate: shiguredo_mp4::Uint::new(1), // CFR (固定フレームレート)

            // Hisui ではヘッダサイズが固定であることが前提
            length_size_minus_one: shiguredo_mp4::Uint::new(NALU_HEADER_LENGTH as u8 - 1),

            // 以下は実際のストリームから取得した値
            nalu_arrays: vec![
                hvcc_nalu_array(H265_NALU_TYPE_VPS, vps_list),
                hvcc_nalu_array(H265_NALU_TYPE_SPS, sps_list),
                hvcc_nalu_array(H265_NALU_TYPE_PPS, pps_list),
            ],

            // これ以降はエンコーダーへの指定に対応する値を設定している

            // 色空間 (4:2:0)
            chroma_format_idc: shiguredo_mp4::Uint::new(1),

            // kVTProfileLevel_HEVC_Main_AutoLevel に対応する値
            general_profile_idc: shiguredo_mp4::Uint::new(1), // Main
            bit_depth_luma_minus8: shiguredo_mp4::Uint::new(0), // 8 ビット深度
            bit_depth_chroma_minus8: shiguredo_mp4::Uint::new(0), // 8 ビット深度
        },
        unknown_boxes: Vec::new(),
    }))
}

fn hvcc_nalu_array(nalu_type: u8, nalus: Vec<Vec<u8>>) -> HvccNalUintArray {
    HvccNalUintArray {
        array_completeness: shiguredo_mp4::Uint::new(1), // true
        nal_unit_type: shiguredo_mp4::Uint::new(nalu_type),
        nalus,
    }
}

/// MP4 形式の H.265 データから VPS, SPS, PPS を抽出する
///
/// MP4 形式: サイズ (4バイト) + NALU データ
pub fn extract_h265_parameter_sets(
    mp4_data: &[u8],
) -> orfail::Result<(Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>)> {
    let mut vps_list = Vec::new();
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();
    let mut pos = 0;

    while pos + 4 <= mp4_data.len() {
        let size = u32::from_be_bytes([
            mp4_data[pos],
            mp4_data[pos + 1],
            mp4_data[pos + 2],
            mp4_data[pos + 3],
        ]) as usize;
        pos += 4;

        if pos + size > mp4_data.len() {
            break;
        }

        let nalu = &mp4_data[pos..pos + size];
        if !nalu.is_empty() {
            // H.265 NAL unit type は最初のバイトの上位6ビット（bit 1-6）
            let nalu_type = (nalu[0] >> 1) & 0x3F;

            match nalu_type {
                H265_NALU_TYPE_VPS => vps_list.push(nalu.to_vec()),
                H265_NALU_TYPE_SPS => sps_list.push(nalu.to_vec()),
                H265_NALU_TYPE_PPS => pps_list.push(nalu.to_vec()),
                _ => {}
            }
        }
        pos += size;
    }

    Ok((vps_list, sps_list, pps_list))
}
