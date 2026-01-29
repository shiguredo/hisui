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

/// FLV の AVCDecoderConfigurationRecord (シーケンスヘッダ) を解析・変換する構造体
#[derive(Debug, Clone)]
pub struct FlvAvcSequenceHeader {
    pub configuration_version: u8,
    pub avc_profile_indication: u8,
    pub profile_compatibility: u8,
    pub avc_level_indication: u8,
    pub length_size_minus_one: u8,
    pub sps_list: Vec<Vec<u8>>,
    pub pps_list: Vec<Vec<u8>>,
}

impl FlvAvcSequenceHeader {
    /// FLV バイト列から AVCDecoderConfigurationRecord をパースする
    pub fn from_bytes(data: &[u8]) -> orfail::Result<Self> {
        (data.len() >= 7)
            .or_fail_with(|()| "AVCDecoderConfigurationRecord too short".to_owned())?;

        let configuration_version = data[0];
        (configuration_version == 1).or_fail_with(|()| {
            format!(
                "unsupported configuration version: {}",
                configuration_version
            )
        })?;

        let avc_profile_indication = data[1];
        let profile_compatibility = data[2];
        let avc_level_indication = data[3];
        let length_size_minus_one = data[4] & 0x03;

        let mut offset = 5;
        let mut sps_list = Vec::new();
        let mut pps_list = Vec::new();

        // SPS ユニット群をパース
        let num_sps = (data[offset] & 0x1F) as usize;
        offset += 1;

        for _ in 0..num_sps {
            (offset + 2 <= data.len()).or_fail()?;
            let sps_length = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;

            (offset + sps_length <= data.len()).or_fail()?;
            sps_list.push(data[offset..offset + sps_length].to_vec());
            offset += sps_length;
        }

        // PPS ユニット群をパース
        (offset < data.len()).or_fail()?;
        let num_pps = data[offset] as usize;
        offset += 1;

        for _ in 0..num_pps {
            (offset + 2 <= data.len()).or_fail()?;
            let pps_length = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;

            (offset + pps_length <= data.len()).or_fail()?;
            pps_list.push(data[offset..offset + pps_length].to_vec());
            offset += pps_length;
        }

        Ok(Self {
            configuration_version,
            avc_profile_indication,
            profile_compatibility,
            avc_level_indication,
            length_size_minus_one,
            sps_list,
            pps_list,
        })
    }

    /// Annex B 形式のバイト列に変換する
    pub fn to_annexb(&self) -> Vec<u8> {
        create_sequence_header_annexb(&self.sps_list, &self.pps_list)
    }

    /// SampleEntry を生成する
    pub fn to_sample_entry(&self, width: usize, height: usize) -> orfail::Result<SampleEntry> {
        Ok(SampleEntry::Avc1(Avc1Box {
            visual: crate::video::sample_entry_visual_fields(width, height),
            avcc_box: AvccBox {
                sps_list: self.sps_list.clone(),
                pps_list: self.pps_list.clone(),
                avc_profile_indication: H264_PROFILE_BASELINE, // TODO: 実際の値を使う (ただし profile によっては chroma format とかの指定が必要になる）: self.avc_profile_indication,
                avc_level_indication: self.avc_level_indication,
                profile_compatibility: self.profile_compatibility,
                length_size_minus_one: Uint::new(self.length_size_minus_one),
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext_list: Vec::new(),
            },
            unknown_boxes: Vec::new(),
        }))
    }
}

/// シンプルなビットリーダー（Exp-Golomb デコード用）
struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn read(&mut self, bits: usize) -> orfail::Result<u32> {
        let mut result: u32 = 0;
        for _ in 0..bits {
            let byte_pos = self.bit_pos / 8;
            let bit_offset = 7 - (self.bit_pos % 8);
            (byte_pos < self.data.len()).or_fail()?;
            let bit = (self.data[byte_pos] >> bit_offset) & 1;
            result = (result << 1) | (bit as u32);
            self.bit_pos += 1;
        }
        Ok(result)
    }

    fn read_ue(&mut self) -> orfail::Result<u32> {
        // Exp-Golomb デコード
        let mut leading_zeros = 0;
        while self.read(1)? == 0 {
            leading_zeros += 1;
        }
        if leading_zeros == 0 {
            Ok(0)
        } else {
            let info = self.read(leading_zeros)?;
            Ok((1 << leading_zeros) - 1 + info)
        }
    }

    fn read_se(&mut self) -> orfail::Result<i32> {
        let ue = self.read_ue()?;
        Ok(if ue & 1 == 0 {
            -((ue >> 1) as i32)
        } else {
            ((ue + 1) >> 1) as i32
        })
    }

    fn skip(&mut self, bits: usize) {
        self.bit_pos += bits;
    }

    fn skip_ue(&mut self) -> orfail::Result<()> {
        let _ue = self.read_ue()?;
        Ok(())
    }

    fn skip_se(&mut self) -> orfail::Result<()> {
        let _se = self.read_se()?;
        Ok(())
    }
}

/// H.264 の SPS から width と height を抽出する
pub fn extract_dimensions_from_sps(sps: &[u8]) -> orfail::Result<(usize, usize)> {
    (sps.len() >= 4).or_fail()?;

    // ビット位置を追跡するための構造体
    let mut bit_reader = BitReader::new(sps);

    // profile_idc
    bit_reader.skip(8);
    // constraint flags
    bit_reader.skip(8);
    // level_idc
    bit_reader.skip(8);
    // seq_parameter_set_id
    bit_reader.skip_ue()?;

    // log2_max_frame_num_minus4
    bit_reader.skip_ue()?;
    // pic_order_cnt_type
    let pic_order_cnt_type = bit_reader.read_ue()?;

    if pic_order_cnt_type == 0 {
        // log2_max_pic_order_cnt_lsb_minus4
        bit_reader.skip_ue()?;
    } else if pic_order_cnt_type == 1 {
        // delta_pic_order_always_zero_flag
        bit_reader.skip(1);
        // offset_for_non_ref_pic
        bit_reader.skip_se()?;
        // offset_for_top_to_bottom_field
        bit_reader.skip_se()?;
        // num_ref_frames_in_pic_order_cnt_cycle
        let num_ref_frames = bit_reader.read_ue()?;
        for _ in 0..num_ref_frames {
            bit_reader.skip_se()?;
        }
    }

    // num_ref_frames
    bit_reader.skip_ue()?;
    // gaps_in_frame_num_value_allowed_flag
    bit_reader.skip(1);

    // pic_width_in_mbs_minus1
    let pic_width = bit_reader.read_ue()? + 1;
    // pic_height_in_map_units_minus1
    let pic_height = bit_reader.read_ue()? + 1;

    // frame_mbs_only_flag
    let frame_mbs_only_flag = bit_reader.read(1)?;
    if frame_mbs_only_flag == 0 {
        // mb_adaptive_frame_field_flag
        bit_reader.skip(1);
    }

    let width = (pic_width * 16) as usize;
    let height = (pic_height * 16 * (if frame_mbs_only_flag == 0 { 2 } else { 1 })) as usize;

    Ok((width, height))
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

/// MP4 ファイルの H.264 映像フレームの形式を RTMP がサポートしている Annex B 形式に変換する
pub fn convert_nalu_to_annexb(data: &[u8], length_size: u8) -> orfail::Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut offset = 0;
    let length_size = length_size as usize;

    (length_size > 0 && length_size <= 4)
        .or_fail_with(|()| format!("invalid NALU length size: {}", length_size))?;

    while offset < data.len() {
        if offset + length_size > data.len() {
            break;
        }

        // MP4 ファイル形式で H.264 の NALU 長を読み取る
        let length = match length_size {
            1 => data[offset] as usize,
            2 => u16::from_be_bytes([data[offset], data[offset + 1]]) as usize,
            3 => u32::from_be_bytes([0, data[offset], data[offset + 1], data[offset + 2]]) as usize,
            4 => u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize,
            _ => unreachable!(),
        };

        offset += length_size;

        (offset + length <= data.len())
            .or_fail_with(|()| "NALU data exceeds buffer length".to_owned())?;

        // Annex B の形式（先頭に固定の区切りバイト列が付与される）に変換する
        result.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        result.extend_from_slice(&data[offset..offset + length]);

        offset += length;
    }

    Ok(result)
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
