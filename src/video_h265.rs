use orfail::OrFail;
use shiguredo_mp4::boxes::{Hev1Box, HvccBox, HvccNalUintArray, SampleEntry};

use crate::{
    types::EvenUsize,
    video::{self, FrameRate},
};

pub type NalUnitArray = Vec<Vec<u8>>;

// H.265 の NAL ユニット前に付与されるサイズのバイト数
// Sora / Hisui が生成するものは全て 4 バイトなので固定値でいい（H.264と同様）
pub use crate::video_h264::NALU_HEADER_LENGTH;

// H.265 の NAL ユニットタイプ
pub const H265_NALU_TYPE_VPS: u8 = 32;
pub const H265_NALU_TYPE_SPS: u8 = 33;
pub const H265_NALU_TYPE_PPS: u8 = 34;

/// H.265 sample entry を生成する
pub fn h265_sample_entry(
    width: EvenUsize,
    height: EvenUsize,
    fps: FrameRate,
    vps_list: NalUnitArray,
    sps_list: NalUnitArray,
    pps_list: NalUnitArray,
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

fn hvcc_nalu_array(nalu_type: u8, nalus: NalUnitArray) -> HvccNalUintArray {
    HvccNalUintArray {
        array_completeness: shiguredo_mp4::Uint::new(1), // true
        nal_unit_type: shiguredo_mp4::Uint::new(nalu_type),
        nalus,
    }
}

/// Annex B 形式の H.265 データから VPS, SPS, PPS を抽出して sample entry を生成する
pub fn h265_sample_entry_from_annexb(
    width: usize,
    height: usize,
    fps: FrameRate,
    data: &[u8],
) -> orfail::Result<SampleEntry> {
    // H.265 ストリームから VPS, SPS, PPS を取り出す
    let mut vps_list = Vec::new();
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();

    let mut pos = 0;
    while pos < data.len() {
        // スタートコードを探す (0x00000001 or 0x000001)
        let start_code_len = if pos + 4 <= data.len() && data[pos..pos + 4] == [0, 0, 0, 1] {
            4
        } else if pos + 3 <= data.len() && data[pos..pos + 3] == [0, 0, 1] {
            3
        } else if pos == 0 {
            return Err(orfail::Failure::new(
                "No H.265 start code found at beginning",
            ));
        } else {
            break;
        };

        pos += start_code_len;

        if pos >= data.len() {
            break;
        }

        // 次のスタートコードまたはデータ終端を探す
        let nalu_start = pos;
        let mut nalu_end = data.len();

        for i in (pos + 3)..data.len() {
            if i + 4 <= data.len() && data[i..i + 4] == [0, 0, 0, 1] {
                nalu_end = i;
                break;
            }
            if i + 3 <= data.len() && data[i..i + 3] == [0, 0, 1] {
                nalu_end = i;
                break;
            }
        }

        let nalu = &data[nalu_start..nalu_end];
        if !nalu.is_empty() {
            // H.265 NAL unit type は最初のバイトの上位6ビット（bit 1-6）
            let nal_unit_type = (nalu[0] >> 1) & 0x3F;

            match nal_unit_type {
                H265_NALU_TYPE_VPS => vps_list.push(nalu.to_vec()),
                H265_NALU_TYPE_SPS => sps_list.push(nalu.to_vec()),
                H265_NALU_TYPE_PPS => pps_list.push(nalu.to_vec()),
                _ => {}
            }
        }

        pos = nalu_end;
    }

    (!vps_list.is_empty()).or_fail()?;
    (!sps_list.is_empty()).or_fail()?;
    (!pps_list.is_empty()).or_fail()?;

    let width = EvenUsize::new(width).or_fail()?;
    let height = EvenUsize::new(height).or_fail()?;

    h265_sample_entry(width, height, fps, vps_list, sps_list, pps_list)
}
