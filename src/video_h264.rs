use orfail::OrFail;
use shiguredo_mp4::{
    Uint,
    boxes::{Avc1Box, AvccBox, SampleEntry},
};

use crate::video;

// H.264 の NAL ユニット前に付与されるサイズのバイト数
// Sora / Hisui が生成するものは全て 4 バイトなので固定値でいい
pub const NALU_HEADER_LENGTH: usize = 4;

// H.264 のプロファイルとレベル（Hisui では固定）
pub const H264_PROFILE_BASELINE: u8 = 66;
pub const H264_LEVEL_3_1: u8 = 31;

// H.264 の NAL ユニットタイプ
pub const H264_NALU_TYPE_IDR: u8 = 5;
pub const H264_NALU_TYPE_SEI: u8 = 6;
pub const H264_NALU_TYPE_SPS: u8 = 7;
pub const H264_NALU_TYPE_PPS: u8 = 8;

/// Annex.B 形式の H.264 をパースして、含まれている NAL ユニットを走査するためのイテレーター
#[derive(Debug)]
pub struct H264AnnexBNalUnits<'a> {
    data: &'a [u8],
}

impl<'a> H264AnnexBNalUnits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn next_nal_unit(&mut self) -> orfail::Result<Option<H264NalUnit<'a>>> {
        if self.data.is_empty() {
            return Ok(None);
        }

        if self.data.starts_with(&[0, 0, 1]) {
            self.data = &self.data[3..];
        } else if self.data.starts_with(&[0, 0, 0, 1]) {
            self.data = &self.data[4..];
        } else {
            return Err(orfail::Failure::new("no H.264 start code prefix"));
        };
        (!self.data.is_empty()).or_fail()?;

        let header = self.data[0];
        ((header >> 7) == 0).or_fail()?;

        let _nal_ref_idc = header >> 5;
        let nal_unit_type = header & 0b0001_1111;

        let i = self
            .data
            .windows(4)
            .position(|w| matches!(w, [0, 0, 1, _] | [0, 0, 0, 1]))
            .unwrap_or(self.data.len());
        let data = &self.data[..i];
        self.data = &self.data[i..];
        Ok(Some(H264NalUnit {
            ty: nal_unit_type,
            data,
        }))
    }
}

impl<'a> Iterator for H264AnnexBNalUnits<'a> {
    type Item = orfail::Result<H264NalUnit<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_nal_unit().or_fail().transpose()
    }
}

#[derive(Debug)]
pub struct H264NalUnit<'a> {
    pub ty: u8,
    pub data: &'a [u8],
}

pub fn h264_sample_entry_from_annexb(
    width: usize,
    height: usize,
    data: &[u8],
) -> orfail::Result<SampleEntry> {
    // H.264 ストリームから SPS と PPS と取り出す
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();
    for nalu in H264AnnexBNalUnits::new(data) {
        let nalu = nalu.or_fail()?;
        match nalu.ty {
            H264_NALU_TYPE_SPS => sps_list.push(nalu.data.to_vec()),
            H264_NALU_TYPE_PPS => pps_list.push(nalu.data.to_vec()),
            _ => {}
        }
    }
    (!sps_list.is_empty()).or_fail()?;
    (!pps_list.is_empty()).or_fail()?;

    Ok(SampleEntry::Avc1(Avc1Box {
        visual: video::sample_entry_visual_fields(width, height),
        avcc_box: AvccBox {
            // 実際のエンコードストリームに合わせた値
            sps_list,
            pps_list,

            // 以下は Hisui では固定値
            avc_profile_indication: H264_PROFILE_BASELINE, // TODO: 実際の値に合わせる
            avc_level_indication: H264_LEVEL_3_1,          // TODO: 実際の値に合わせる
            profile_compatibility: 0, // いったん 0 を指定しているが、もし支障があれば調整する
            length_size_minus_one: Uint::new(NALU_HEADER_LENGTH as u8 - 1),
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext_list: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    }))
}

/// AVC1 サンプルエントリーから width, height を抽出
pub fn extract_video_dimensions(entry: &SampleEntry) -> orfail::Result<(u32, u32)> {
    match entry {
        SampleEntry::Avc1(avc1) => {
            let width = avc1.visual.width as u32;
            let height = avc1.visual.height as u32;
            Ok((width, height))
        }
        _ => Err(orfail::Failure::new("Not an H.264 video sample entry")),
    }
}

/// H.264 のシーケンスヘッダを Annex B 形式で作成する
///
/// SPS (Sequence Parameter Set) と PPS (Picture Parameter Set) を
/// Annex B 形式で連結してシーケンスヘッダを生成します。
/// 各NALユニットの前には開始コード `0x00 0x00 0x00 0x01` が付与されます。
pub fn create_sequence_header_annexb(sps_list: &[Vec<u8>], pps_list: &[Vec<u8>]) -> Vec<u8> {
    let mut result = Vec::new();

    // 全ての SPS を追加
    for sps in sps_list {
        result.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        result.extend_from_slice(sps);
    }

    // 全ての PPS を追加
    for pps in pps_list {
        result.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        result.extend_from_slice(pps);
    }

    result
}

/// Annex.B 形式の H.264 を RTMP 用の AVC パケット形式（サイズ付き NALU）に変換
pub fn convert_annexb_to_nalu(data: &[u8], length_size: u8) -> orfail::Result<Vec<u8>> {
    let mut result = Vec::new();

    (length_size > 0 && length_size <= 4)
        .or_fail_with(|()| format!("invalid NALU length size: {length_size}"))?;

    for nalu in H264AnnexBNalUnits::new(data) {
        let nalu = nalu.or_fail()?;

        // サイズをバイト列に変換
        let size_bytes = match length_size {
            1 => {
                let size = u8::try_from(nalu.data.len()).or_fail()?;
                &[size][..]
            }
            2 => {
                let size = u16::try_from(nalu.data.len()).or_fail()?;
                &size.to_be_bytes()[..]
            }
            3 => {
                let size = u32::try_from(nalu.data.len()).or_fail()?;
                (size <= 0x00FF_FFFF).or_fail()?;
                &[(size >> 16) as u8, (size >> 8) as u8, size as u8]
            }
            4 => {
                let size = u32::try_from(nalu.data.len()).or_fail()?;
                &size.to_be_bytes()[..]
            }
            _ => unreachable!(),
        };

        result.extend_from_slice(size_bytes);
        result.extend_from_slice(nalu.data);
    }

    Ok(result)
}
