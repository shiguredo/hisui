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

// High 系プロファイル群（ITU-T H.264 (2017/06) 仕様 7.3.2.1.1 の `if (profile_idc == ...)` 条件節）
// 該当プロファイルでは SPS に chroma_format_idc 以下の追加フィールド群が含まれる
const H264_HIGH_PROFILES: [u8; 13] = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135];

/// Annex.B 形式の H.264 をパースして、含まれている NAL ユニットを走査するためのイテレーター
#[derive(Debug)]
pub struct H264AnnexBNalUnits<'a> {
    data: &'a [u8],
}

impl<'a> H264AnnexBNalUnits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn next_nal_unit(&mut self) -> crate::Result<Option<H264NalUnit<'a>>> {
        if self.data.is_empty() {
            return Ok(None);
        }

        if self.data.starts_with(&[0, 0, 1]) {
            self.data = &self.data[3..];
        } else if self.data.starts_with(&[0, 0, 0, 1]) {
            self.data = &self.data[4..];
        } else {
            return Err(crate::Error::new("no H.264 start code prefix"));
        };
        if self.data.is_empty() {
            return Err(crate::Error::new("empty H.264 NAL unit"));
        }

        let header = self.data[0];
        if (header >> 7) != 0 {
            return Err(crate::Error::new(
                "invalid H.264 NAL header: forbidden_zero_bit is set",
            ));
        }

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
    type Item = crate::Result<H264NalUnit<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_nal_unit().transpose()
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
) -> crate::Result<SampleEntry> {
    // H.264 ストリームから SPS と PPS と取り出す
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();
    for nalu in H264AnnexBNalUnits::new(data) {
        let nalu = nalu?;
        match nalu.ty {
            H264_NALU_TYPE_SPS => sps_list.push(nalu.data.to_vec()),
            H264_NALU_TYPE_PPS => pps_list.push(nalu.data.to_vec()),
            _ => {}
        }
    }
    if sps_list.is_empty() {
        return Err(crate::Error::new("missing H.264 SPS"));
    }
    if pps_list.is_empty() {
        return Err(crate::Error::new("missing H.264 PPS"));
    }

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
pub fn extract_video_dimensions(entry: &SampleEntry) -> crate::Result<(u32, u32)> {
    match entry {
        SampleEntry::Avc1(avc1) => {
            let width = avc1.visual.width as u32;
            let height = avc1.visual.height as u32;
            Ok((width, height))
        }
        _ => Err(crate::Error::new("Not an H.264 video sample entry")),
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

/// SPS NAL ユニットのバイト列から width / height を抽出する
///
/// 入力 `sps` は `H264AnnexBNalUnits` が返す `H264NalUnit.data` をそのまま渡す形式で、
/// 先頭 1 バイトに NAL ヘッダ（forbidden_zero_bit + nal_ref_idc + nal_unit_type = 7）を含む。
/// 先頭バイトの下位 5 bit が SPS の NAL unit type (7) でない場合は Err を返す。
/// 内部で先頭 1 バイトをスキップしたうえで RBSP 抽出（emulation prevention byte 除去）を行い、
/// ITU-T H.264 仕様 7.3.2.1.1 / 7.4.2.1.1 に従って Exp-Golomb で解像度を抽出する。
pub fn extract_dimensions_from_sps(sps: &[u8]) -> crate::Result<(usize, usize)> {
    let rbsp = rbsp_from_sps_nalu(sps)?;
    let mut reader = H264BitReader::new(&rbsp);

    let profile_idc = reader.read_u(8)? as u8;
    reader.skip_u(8)?; // constraint_set0..5_flag + reserved_zero_2bits
    reader.skip_u(8)?; // level_idc
    reader.skip_ue()?; // seq_parameter_set_id

    let chroma_array_type = read_chroma_array_type(&mut reader, profile_idc)?;
    reader.skip_ue()?; // log2_max_frame_num_minus4
    skip_pic_order_cnt_type_extras(&mut reader)?;
    let (width, height) = read_dimensions_with_cropping(&mut reader, chroma_array_type)?;

    if width == 0 || height == 0 {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: zero dimensions after cropping (width={width}, height={height})"
        )));
    }

    // 戻り値は最終的に `sample_entry_visual_fields` で `width as u16 / height as u16` に渡される。
    // u16 上限 (65535) を超えると silent truncation してラップした値や 0 が MP4 sample_entry に
    // 埋め込まれるため、ここで上限を強制する。H.264 仕様 Level 6.2 の最大解像度 8192x4320 でも
    // u16 に収まるため、実用範囲を狭めることはない。
    if width > u16::MAX as usize || height > u16::MAX as usize {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: dimensions exceed u16::MAX (width={width}, height={height})"
        )));
    }

    Ok((width, height))
}

/// High 系プロファイルの追加フィールド群を読み、`chroma_array_type` を返す。
///
/// Baseline / Main / Extended プロファイルでは chroma_format_idc が SPS に含まれないため
/// 仕様 7.4.2.1.1 のデフォルトとして 4:2:0 (`chroma_array_type = 1`) を返す。
fn read_chroma_array_type(reader: &mut H264BitReader<'_>, profile_idc: u8) -> crate::Result<u32> {
    if !H264_HIGH_PROFILES.contains(&profile_idc) {
        return Ok(1);
    }

    let chroma_format_idc = reader.read_ue()?;
    let separate_colour_plane_flag = if chroma_format_idc == 3 {
        reader.read_u(1)?
    } else {
        0
    };
    reader.skip_ue()?; // bit_depth_luma_minus8
    reader.skip_ue()?; // bit_depth_chroma_minus8
    reader.skip_u(1)?; // qpprime_y_zero_transform_bypass_flag
    let seq_scaling_matrix_present_flag = reader.read_u(1)?;
    if seq_scaling_matrix_present_flag == 1 {
        // chroma_format_idc が 3 のときは 12 個、それ以外は 8 個の scaling_list を読み飛ばす
        let scaling_list_count = if chroma_format_idc == 3 { 12 } else { 8 };
        for i in 0..scaling_list_count {
            let seq_scaling_list_present_flag = reader.read_u(1)?;
            if seq_scaling_list_present_flag == 1 {
                // 先頭 6 個は 4x4 (size 16)、残りは 8x8 (size 64)
                let size = if i < 6 { 16 } else { 64 };
                skip_scaling_list(reader, size)?;
            }
        }
    }

    // chroma_array_type の決定（仕様 7.4.2.1.1）
    Ok(if separate_colour_plane_flag == 1 {
        0
    } else {
        chroma_format_idc
    })
}

/// pic_order_cnt_type に応じた追加フィールド群を読み飛ばす（仕様 7.3.2.1.1）
fn skip_pic_order_cnt_type_extras(reader: &mut H264BitReader<'_>) -> crate::Result<()> {
    let pic_order_cnt_type = reader.read_ue()?;
    match pic_order_cnt_type {
        0 => {
            reader.skip_ue()?; // log2_max_pic_order_cnt_lsb_minus4
        }
        1 => {
            reader.skip_u(1)?; // delta_pic_order_always_zero_flag
            reader.skip_se()?; // offset_for_non_ref_pic
            reader.skip_se()?; // offset_for_top_to_bottom_field
            let num_ref_frames_in_pic_order_cnt_cycle = reader.read_ue()?;
            // 仕様 7.4.2.1.1 で 0..=255 の範囲。それを超える値は仕様外で、巨大値での無駄な se(v) ループを防ぐ。
            if num_ref_frames_in_pic_order_cnt_cycle > 255 {
                return Err(crate::Error::new(format!(
                    "invalid H.264 SPS: num_ref_frames_in_pic_order_cnt_cycle exceeds 255 ({num_ref_frames_in_pic_order_cnt_cycle})"
                )));
            }
            for _ in 0..num_ref_frames_in_pic_order_cnt_cycle {
                reader.skip_se()?; // offset_for_ref_frame[i]
            }
        }
        // pic_order_cnt_type == 2 のときは追加読み出しなし
        _ => {}
    }
    Ok(())
}

/// pic_width / pic_height / frame_mbs_only_flag / frame_cropping_flag を読み、
/// cropping 適用後の (width, height) を返す（仕様 7.4.2.1.1）。
///
/// `chroma_array_type` は CropUnitX / CropUnitY の決定に使う。
fn read_dimensions_with_cropping(
    reader: &mut H264BitReader<'_>,
    chroma_array_type: u32,
) -> crate::Result<(usize, usize)> {
    reader.skip_ue()?; // max_num_ref_frames
    reader.skip_u(1)?; // gaps_in_frame_num_value_allowed_flag
    let pic_width_in_mbs_minus1 = reader.read_ue()?;
    let pic_height_in_map_units_minus1 = reader.read_ue()?;
    let frame_mbs_only_flag = reader.read_u(1)?;
    if frame_mbs_only_flag == 0 {
        // 仕様 7.3.2.1.1: frame_mbs_only_flag == 0 のとき mb_adaptive_frame_field_flag (u(1)) を読む。
        // 値自体は本実装では使わないが、ビット位置を進めないと後続の direct_8x8_inference_flag /
        // frame_cropping_flag の読み出しが 1 bit ずれて誤動作する。
        reader.skip_u(1)?; // mb_adaptive_frame_field_flag
    }
    reader.skip_u(1)?; // direct_8x8_inference_flag
    let frame_cropping_flag = reader.read_u(1)?;

    // CropUnitX / CropUnitY の決定（仕様 6.2 / 7.4.2.1.1）
    let frame_mbs_factor = 2 - frame_mbs_only_flag as usize;
    let (crop_unit_x, crop_unit_y) = match chroma_array_type {
        0 => (1usize, frame_mbs_factor),
        1 => (2, 2 * frame_mbs_factor),
        2 => (2, frame_mbs_factor),
        3 => (1, frame_mbs_factor),
        _ => {
            return Err(crate::Error::new(format!(
                "invalid H.264 SPS: unexpected chroma_array_type {chroma_array_type}"
            )));
        }
    };

    // raw_width / raw_height の算出（usize に変換してから checked_* で組み立てる）
    let raw_width = (pic_width_in_mbs_minus1 as usize)
        .checked_add(1)
        .and_then(|v| v.checked_mul(16))
        .ok_or_else(|| {
            crate::Error::new("invalid H.264 SPS: pic_width overflow during raw_width calculation")
        })?;
    let raw_height = (pic_height_in_map_units_minus1 as usize)
        .checked_add(1)
        .and_then(|v| v.checked_mul(16))
        .and_then(|v| v.checked_mul(2 - frame_mbs_only_flag as usize))
        .ok_or_else(|| {
            crate::Error::new(
                "invalid H.264 SPS: pic_height overflow during raw_height calculation",
            )
        })?;

    if frame_cropping_flag == 1 {
        let frame_crop_left_offset = reader.read_ue()? as usize;
        let frame_crop_right_offset = reader.read_ue()? as usize;
        let frame_crop_top_offset = reader.read_ue()? as usize;
        let frame_crop_bottom_offset = reader.read_ue()? as usize;

        let crop_x = frame_crop_left_offset
            .checked_add(frame_crop_right_offset)
            .and_then(|v| v.checked_mul(crop_unit_x))
            .ok_or_else(|| {
                crate::Error::new("invalid H.264 SPS: crop_x overflow during width calculation")
            })?;
        let crop_y = frame_crop_top_offset
            .checked_add(frame_crop_bottom_offset)
            .and_then(|v| v.checked_mul(crop_unit_y))
            .ok_or_else(|| {
                crate::Error::new("invalid H.264 SPS: crop_y overflow during height calculation")
            })?;

        let width = raw_width.checked_sub(crop_x).ok_or_else(|| {
            crate::Error::new("invalid H.264 SPS: crop_x exceeds raw_width (underflow)")
        })?;
        let height = raw_height.checked_sub(crop_y).ok_or_else(|| {
            crate::Error::new("invalid H.264 SPS: crop_y exceeds raw_height (underflow)")
        })?;
        Ok((width, height))
    } else {
        Ok((raw_width, raw_height))
    }
}

/// SPS NAL ユニットから RBSP を抽出する
///
/// 先頭の NAL ヘッダ 1 バイトをスキップし、payload 内の emulation prevention byte
/// （`0x00 0x00 0x03` パターン）を除去した RBSP バイト列を返す。
fn rbsp_from_sps_nalu(nalu: &[u8]) -> crate::Result<Vec<u8>> {
    if nalu.is_empty() {
        return Err(crate::Error::new("invalid H.264 SPS: empty NAL unit"));
    }
    // `extract_dimensions_from_sps` は pub で外部から呼ばれ得るため、release ビルドでも NAL タイプを検査する。
    // ここで検出された場合のエラーメッセージは「NAL タイプの不一致」として、後段のビットリーダで失敗するよりも
    // 早い段階で原因が分かるようにする。
    let nal_unit_type = nalu[0] & 0x1F;
    if nal_unit_type != H264_NALU_TYPE_SPS {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: expected nal_unit_type={H264_NALU_TYPE_SPS}, got {nal_unit_type}"
        )));
    }
    let payload = &nalu[1..];
    let mut rbsp = Vec::with_capacity(payload.len());
    let mut i = 0;
    while i < payload.len() {
        // `0x00 0x00 0x03` パターンを検出したら `0x03` を除去する
        if i + 2 < payload.len()
            && payload[i] == 0x00
            && payload[i + 1] == 0x00
            && payload[i + 2] == 0x03
        {
            rbsp.push(0x00);
            rbsp.push(0x00);
            i += 3;
        } else {
            rbsp.push(payload[i]);
            i += 1;
        }
    }
    Ok(rbsp)
}

/// バイト列を 1 ビット単位で読み出すリーダー
///
/// 全 read メソッド（`read_u` / `read_ue` / `read_se`）はバッファ末尾を超える読み出しで Err を返す。
/// パニックや無限ループは起こらないため、proptest のクラッシュフリー保証はこの構造で担保される。
struct H264BitReader<'a> {
    data: &'a [u8],
    // バイト単位の現在位置
    byte_pos: usize,
    // 現バイト内のビット位置（0 = MSB, 7 = LSB）
    bit_pos: u8,
}

impl<'a> H264BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// n ビット符号なし整数を読み出す（仕様の u(n) に相当）
    ///
    /// n は最大 32 まで対応する。バッファ末尾を超える場合は Err を返す。
    fn read_u(&mut self, n: usize) -> crate::Result<u32> {
        if n > 32 {
            return Err(crate::Error::new(format!(
                "invalid H.264 SPS: read_u with n > 32 (n={n})"
            )));
        }
        let mut value: u32 = 0;
        for _ in 0..n {
            value = (value << 1) | self.read_bit()? as u32;
        }
        Ok(value)
    }

    /// 1 ビットを読み出す（内部ヘルパー）
    fn read_bit(&mut self) -> crate::Result<u8> {
        if self.byte_pos >= self.data.len() {
            return Err(crate::Error::new(
                "invalid H.264 SPS: bit reader exhausted before requested read",
            ));
        }
        let bit = (self.data[self.byte_pos] >> (7 - self.bit_pos)) & 1;
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
        Ok(bit)
    }

    /// 符号なし Exp-Golomb 復号（仕様 9.1 の ue(v) に相当）
    ///
    /// 連続する 0 ビットの数を leading_zeros として数え、続く 1 ビットを読み、
    /// その後 leading_zeros 個のビットを読んで `(1 << leading_zeros) - 1 + bits` を返す。
    fn read_ue(&mut self) -> crate::Result<u32> {
        let mut leading_zeros: u32 = 0;
        loop {
            let bit = self.read_bit()?;
            if bit == 1 {
                break;
            }
            leading_zeros += 1;
            // 仕様 9.1 上 codeNum は最大 2^32 - 2 まで表現可能だが、`1u32 << 32` がシフト範囲外で
            // panic / 未定義動作になるため 31 で制限する。Hisui で扱う SPS フィールド値はすべて 31 bit 以下。
            if leading_zeros > 31 {
                return Err(crate::Error::new(
                    "invalid H.264 SPS: ue(v) leading_zeros exceeds 31 (overflow)",
                ));
            }
        }
        if leading_zeros == 0 {
            return Ok(0);
        }
        let suffix = self.read_u(leading_zeros as usize)?;
        // (1 << leading_zeros) - 1 + suffix が u32 にちょうど収まる範囲
        // leading_zeros == 31 のとき、(1 << 31) - 1 + suffix は最大 (2^31 - 1) + (2^31 - 1) = 2^32 - 2
        let prefix = (1u32 << leading_zeros).wrapping_sub(1);
        prefix
            .checked_add(suffix)
            .ok_or_else(|| crate::Error::new("invalid H.264 SPS: ue(v) value overflow on combine"))
    }

    /// 符号付き Exp-Golomb 復号（仕様 9.1.1 の se(v) に相当）
    ///
    /// 内部で ue(v) を読み、code_num が偶数なら -code_num / 2、奇数なら (code_num + 1) / 2 を返す。
    fn read_se(&mut self) -> crate::Result<i32> {
        let code_num = self.read_ue()?;
        if code_num % 2 == 1 {
            let value = code_num.div_ceil(2);
            i32::try_from(value)
                .map_err(|_| crate::Error::new("invalid H.264 SPS: se(v) positive value overflow"))
        } else {
            let value = code_num / 2;
            let negated = i64::from(value)
                .checked_neg()
                .ok_or_else(|| crate::Error::new("invalid H.264 SPS: se(v) negation overflow"))?;
            i32::try_from(negated)
                .map_err(|_| crate::Error::new("invalid H.264 SPS: se(v) negative value overflow"))
        }
    }

    /// n ビット符号なし整数を読み飛ばす（戻り値を捨てる `read_u` のラッパー）
    fn skip_u(&mut self, n: usize) -> crate::Result<()> {
        self.read_u(n).map(|_| ())
    }

    /// ue(v) を読み飛ばす（戻り値を捨てる `read_ue` のラッパー）
    fn skip_ue(&mut self) -> crate::Result<()> {
        self.read_ue().map(|_| ())
    }

    /// se(v) を読み飛ばす（戻り値を捨てる `read_se` のラッパー）
    fn skip_se(&mut self) -> crate::Result<()> {
        self.read_se().map(|_| ())
    }
}

/// scaling_list() サブルーチンの読み飛ばし（仕様 7.3.2.1.1.1）
///
/// 要素ごとに delta_scale (se(v)) を読む。next_scale が 0 になると以降は読まずに進める。
/// 実値は本実装では使わず、ビット位置を進めるだけ。
///
/// H.264 仕様固有のロジックなので `H264BitReader` 本体（汎用ビットリーダ）からは分離する。
fn skip_scaling_list(reader: &mut H264BitReader<'_>, size: usize) -> crate::Result<()> {
    let mut last_scale: i32 = 8;
    let mut next_scale: i32 = 8;
    for _ in 0..size {
        if next_scale != 0 {
            let delta_scale = reader.read_se()?;
            // 仕様 7.3.2.1.1.1: next_scale = (last_scale + delta_scale + 256) % 256
            let sum = last_scale
                .checked_add(delta_scale)
                .and_then(|v| v.checked_add(256))
                .ok_or_else(|| {
                    crate::Error::new(
                        "invalid H.264 SPS: scaling_list next_scale overflow during update",
                    )
                })?;
            next_scale = sum.rem_euclid(256);
        }
        if next_scale != 0 {
            last_scale = next_scale;
        }
    }
    Ok(())
}

/// Annex.B 形式の H.264 を RTMP 用の AVC パケット形式（サイズ付き NALU）に変換
pub fn convert_annexb_to_nalu(data: &[u8], length_size: u8) -> crate::Result<Vec<u8>> {
    let mut result = Vec::new();

    if length_size == 0 || length_size > 4 {
        return Err(crate::Error::new(format!(
            "invalid NALU length size: {length_size}"
        )));
    }

    for nalu in H264AnnexBNalUnits::new(data) {
        let nalu = nalu?;

        // サイズをバイト列に変換
        let size_bytes = match length_size {
            1 => {
                let size = u8::try_from(nalu.data.len())?;
                &[size][..]
            }
            2 => {
                let size = u16::try_from(nalu.data.len())?;
                &size.to_be_bytes()[..]
            }
            3 => {
                let size = u32::try_from(nalu.data.len())?;
                if size > 0x00FF_FFFF {
                    return Err(crate::Error::new(format!(
                        "NALU size does not fit in 3-byte length field: {size}"
                    )));
                }
                &[(size >> 16) as u8, (size >> 8) as u8, size as u8]
            }
            4 => {
                let size = u32::try_from(nalu.data.len())?;
                &size.to_be_bytes()[..]
            }
            _ => unreachable!(),
        };

        result.extend_from_slice(size_bytes);
        result.extend_from_slice(nalu.data);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 以下の SPS バイト列は ffmpeg + libx264 で生成した実機 SPS を抽出したもの。
    // 生成コマンドは `ffmpeg -f lavfi -i testsrc=size=WIDTHxHEIGHT:rate=30 -pix_fmt yuv420p
    // -c:v libx264 -profile:v baseline -frames:v 1 -f h264 out.h264` で、
    // 先頭の SPS NAL を 2 個目の start code 直前まで切り出した。

    // Baseline プロファイル + 320x240 (16 の倍数の解像度、crop なしの最小実機 SPS パターン)
    const SPS_320X240: [u8; 24] = [
        0x67, 0x42, 0xc0, 0x0d, 0xd9, 0x01, 0x41, 0xfb, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10,
        0x00, 0x00, 0x03, 0x03, 0xc0, 0xf1, 0x42, 0xa4, 0x80,
    ];

    // Baseline プロファイル + 1920x1080 (16 倍数でない 1080 のため crop_bottom 経路を踏む実機 SPS)
    const SPS_1920X1080: [u8; 26] = [
        0x67, 0x42, 0xc0, 0x28, 0xd9, 0x00, 0x78, 0x02, 0x27, 0xe5, 0xc0, 0x44, 0x00, 0x00, 0x03,
        0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0, 0x3c, 0x60, 0xc9, 0x20,
    ];

    #[test]
    fn extract_dimensions_from_baseline_no_crop_320x240() {
        // crop なし (16 倍数解像度) のときに raw_width / raw_height をそのまま返すこと
        let (width, height) = extract_dimensions_from_sps(&SPS_320X240).expect("SPS パース成功");
        assert_eq!((width, height), (320, 240));
    }

    #[test]
    fn extract_dimensions_from_baseline_with_crop_1920x1080() {
        // libx264 が 1920x1080 を表現する際の実機パターン (raw 1920x1088 + crop_bottom=4) を
        // 仕様準拠で正しく解釈して 1920x1080 を返すこと
        let (width, height) = extract_dimensions_from_sps(&SPS_1920X1080).expect("SPS パース成功");
        assert_eq!((width, height), (1920, 1080));
    }

    #[test]
    fn extract_dimensions_fails_on_truncated_sps() {
        // SPS 末尾でビット切れになる入力では Err を返すこと
        let truncated = &SPS_1920X1080[..5]; // NAL ヘッダ + 数バイトだけ残す
        let result = extract_dimensions_from_sps(truncated);
        assert!(
            result.is_err(),
            "短すぎる SPS は Err を返すはず: {result:?}"
        );
    }

    #[test]
    fn rbsp_from_sps_nalu_removes_emulation_prevention_bytes() {
        // 0x00 0x00 0x03 の 3 バイトが 0x00 0x00 に縮約されることを直接検証する。
        // 1920x1080 の SPS には emulation prevention byte が 2 箇所含まれる
        let rbsp = rbsp_from_sps_nalu(&SPS_1920X1080).expect("RBSP 抽出成功");
        // 入力（NAL ヘッダ 1 バイト + payload 25 バイト）から emulation prevention byte 2 個分が削れる
        assert_eq!(rbsp.len(), SPS_1920X1080.len() - 1 - 2);
        // RBSP には連続する 3 バイト `0x00 0x00 0x03` が残らない
        for window in rbsp.windows(3) {
            assert!(
                !(window[0] == 0x00 && window[1] == 0x00 && window[2] == 0x03),
                "RBSP に emulation prevention byte が残っている: {window:?}"
            );
        }
    }

    #[test]
    fn rbsp_from_sps_nalu_rejects_empty_input() {
        // 空入力では Err を返すこと
        let result = rbsp_from_sps_nalu(&[]);
        assert!(result.is_err(), "空 NAL は Err を返すはず: {result:?}");
    }

    #[test]
    fn extract_dimensions_from_sps_rejects_non_sps_nal() {
        // SPS 以外の NAL（先頭バイトの下位 5 bit が 7 でないもの）を渡すと Err を返すこと。
        // pub 関数として誤呼出時に release ビルドでも検出できることの回帰防止。
        // 0x68 は PPS の NAL ヘッダ（nal_unit_type = 8）。
        let pps_nal = [0x68, 0xce, 0x06, 0xe2];
        let result = extract_dimensions_from_sps(&pps_nal);
        assert!(
            result.is_err(),
            "SPS 以外の NAL は Err を返すはず: {result:?}"
        );
    }

    #[test]
    fn read_ue_decodes_specification_examples() {
        // 仕様 9.1 表 9-1 の代表的な値を網羅的に検証する
        // codeNum = 0 → "1"
        // codeNum = 1 → "010"
        // codeNum = 2 → "011"
        // codeNum = 3 → "00100"
        // codeNum = 4 → "00101"
        // codeNum = 5 → "00110"
        // codeNum = 6 → "00111"
        let data = [
            // "1 010 011 00100 00101 00110 00111" を 8 bit 単位に詰める
            // MSB から bit 0..27 を順に並べると次の 4 バイトになる:
            //   1010 0110 = 0xa6
            //   0100 0010 = 0x42
            //   1001 1000 = 0x98
            //   1110 0000 = 0xe0（最後の 3 bit は "111"、残り 5 bit は 0 padding）
            0xa6, 0x42, 0x98, 0xe0,
        ];
        let mut reader = H264BitReader::new(&data);
        let expected = [0u32, 1, 2, 3, 4, 5, 6];
        for &want in &expected {
            let got = reader.read_ue().expect("ue(v) 読み出し成功");
            assert_eq!(got, want, "ue(v) のデコード結果が期待値と一致すること");
        }
    }

    #[test]
    fn read_se_decodes_specification_examples() {
        // 仕様 9.1.1 表 9-3 の代表的な値:
        // ue codeNum 0 → se 0
        // ue codeNum 1 → se 1
        // ue codeNum 2 → se -1
        // ue codeNum 3 → se 2
        // ue codeNum 4 → se -2
        // ue を順に並べたバイト列を使う
        // ue: 1, 010, 011, 00100, 00101 = "1 010 011 00100 00101" を 8 bit 単位
        // 1 0 1 0 0 1 1 0 = 0xa6
        // 0 1 0 0 0 0 1 0 = 0x42
        // 1 ... 0 埋め → 0x80
        let data = [0xa6, 0x42, 0x80];
        let mut reader = H264BitReader::new(&data);
        let expected = [0i32, 1, -1, 2, -2];
        for &want in &expected {
            let got = reader.read_se().expect("se(v) 読み出し成功");
            assert_eq!(got, want, "se(v) のデコード結果が期待値と一致すること");
        }
    }

    #[test]
    fn read_u_fails_on_exhausted_buffer() {
        // バッファ末尾を超えた読み出しで Err を返すこと
        let data = [0xff];
        let mut reader = H264BitReader::new(&data);
        // 8 bit 読めるが、9 bit 目で Err
        assert!(reader.read_u(8).is_ok());
        assert!(
            reader.read_u(1).is_err(),
            "exhausted buffer で Err を返すはず"
        );
    }

    #[test]
    fn read_u_rejects_too_large_n() {
        // read_u(n) で n > 32 は Err を返すこと
        let data = [0xff; 8];
        let mut reader = H264BitReader::new(&data);
        assert!(reader.read_u(33).is_err(), "n > 32 は Err を返すはず");
    }

    #[test]
    fn read_ue_rejects_excessive_leading_zeros() {
        // 0 ビットを 32 個以上連続させると `1u32 << 32` がシフト範囲外になるため Err を返すこと
        // 32 個の連続 0 = 4 バイトすべて 0x00 にして、その後を埋める
        let data = [0x00, 0x00, 0x00, 0x00, 0xff, 0xff];
        let mut reader = H264BitReader::new(&data);
        let result = reader.read_ue();
        assert!(
            result.is_err(),
            "leading_zeros が 31 を超えると Err を返すはず: {result:?}"
        );
    }

    #[test]
    fn skip_scaling_list_rejects_next_scale_overflow() {
        // scaling_list の next_scale 計算式 `last_scale + delta_scale + 256` で
        // i32 オーバーフローが起きた場合に Err を返すこと。
        //
        // 仕様 9.1.1 で se(v) = i32::MAX を表現するには ue(v) で codeNum = 2 * i32::MAX - 1
        // (= u32::MAX - 2 = 0xFFFFFFFD) を読ませる。
        // codeNum = 0xFFFFFFFD の ue(v) バイト列:
        //   leading_zeros = 31 (prefix = 2^31 - 1 = 0x7FFFFFFF)
        //   suffix = codeNum - prefix = 0x7FFFFFFE (31 ビット)
        // よって入力バイト列:
        //   "0000_0000 0000_0000 0000_0000 0000_0001 1111_1111 1111_1111 1111_1111 1111_1100"
        //   (31 個の 0 + "1" マーカー + suffix 31 ビット "1111_..._110" + padding 1 bit)
        //   = [0x00, 0x00, 0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xFC]
        // これで delta_scale = i32::MAX となり、8.checked_add(i32::MAX) が None → Err。
        let data = [0x00, 0x00, 0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xFC];
        let mut reader = H264BitReader::new(&data);
        let result = skip_scaling_list(&mut reader, 1);
        assert!(
            result.is_err(),
            "巨大な delta_scale で next_scale 計算が overflow したら Err: {result:?}"
        );
    }

    // ----------------------------------------------------------------
    // 仕様準拠の SPS バイト列ビルダー（テスト専用）
    //
    // 主要分岐 (High profile / scaling_matrix / pic_order_cnt_type=1 / interlaced / cropping) を
    // 確実に踏ませるため、ITU-T H.264 7.3.2.1.1 に従って SPS を組み立てる。
    // libx264 で生成できない経路や、各種 Err 経路 (crop アンダーフロー / 巨大値オーバーフロー)
    // の単体テストにも使う。
    // ----------------------------------------------------------------

    /// ビット単位で値を書き出すライター（仕様 9.1 / 9.1.1 の逆操作）
    struct SpsBitWriter {
        bytes: Vec<u8>,
        bit_count: usize,
    }

    impl SpsBitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                bit_count: 0,
            }
        }

        fn write_bit(&mut self, bit: u8) {
            let byte_idx = self.bit_count / 8;
            let bit_idx = self.bit_count % 8;
            if byte_idx >= self.bytes.len() {
                self.bytes.push(0);
            }
            self.bytes[byte_idx] |= (bit & 1) << (7 - bit_idx);
            self.bit_count += 1;
        }

        /// n ビット符号なし整数を MSB ファーストで書き出す（u(n)）
        fn write_u(&mut self, n: usize, value: u32) {
            for i in (0..n).rev() {
                let bit = ((value >> i) & 1) as u8;
                self.write_bit(bit);
            }
        }

        /// 符号なし Exp-Golomb 復号の逆操作（ue(v)、仕様 9.1）
        fn write_ue(&mut self, value: u32) {
            // codeNum = value
            // value + 1 の bit 表現長を取り、(長さ - 1) 個の先行 0 + value+1 の bit 表現を書く
            let v = value
                .checked_add(1)
                .expect("ue(v) のテスト入力が u32::MAX を越えた場合は意図的なオーバーフロー検証用");
            let bits_needed = 32 - v.leading_zeros() as usize;
            for _ in 0..(bits_needed - 1) {
                self.write_bit(0);
            }
            self.write_u(bits_needed, v);
        }

        /// 符号付き Exp-Golomb 復号の逆操作（se(v)、仕様 9.1.1）
        fn write_se(&mut self, value: i32) {
            let code_num = if value <= 0 {
                (-(i64::from(value))) as u64 * 2
            } else {
                (value as u64) * 2 - 1
            };
            self.write_ue(code_num as u32);
        }

        fn into_bytes(self) -> Vec<u8> {
            self.bytes
        }
    }

    /// テスト用の SPS バイト列ビルダー
    ///
    /// デフォルトは Baseline (profile_idc=66) + 1920x1080 + progressive + crop なしの最小 SPS。
    /// `with_*` メソッドで各分岐を踏ませる。
    struct SpsBuilder {
        profile_idc: u8,
        pic_width_in_mbs_minus1: u32,
        pic_height_in_map_units_minus1: u32,
        frame_mbs_only_flag: bool,
        seq_scaling_matrix_present_flag: bool,
        pic_order_cnt_type: u32,
        frame_cropping_flag: bool,
        crop_offsets: (u32, u32, u32, u32),
    }

    impl SpsBuilder {
        /// raw 解像度 (pic_width_in_mbs_minus1 / pic_height_in_map_units_minus1 から決まる値) を
        /// 直接指定するベースビルダー。
        /// デフォルトは Baseline + progressive + crop なし + pic_order_cnt_type=2。
        fn raw(raw_width: u32, raw_height: u32) -> Self {
            assert!(
                raw_width.is_multiple_of(16),
                "raw_width は 16 の倍数で指定すること"
            );
            assert!(
                raw_height.is_multiple_of(16),
                "raw_height は 16 の倍数で指定すること"
            );
            Self {
                profile_idc: 66, // Baseline
                pic_width_in_mbs_minus1: raw_width / 16 - 1,
                pic_height_in_map_units_minus1: raw_height / 16 - 1,
                frame_mbs_only_flag: true,
                seq_scaling_matrix_present_flag: false,
                pic_order_cnt_type: 2,
                frame_cropping_flag: false,
                crop_offsets: (0, 0, 0, 0),
            }
        }

        fn with_high_profile_and_scaling_matrix(mut self) -> Self {
            self.profile_idc = 100; // High
            self.seq_scaling_matrix_present_flag = true;
            self
        }

        fn with_interlaced(mut self, raw_height_field: u32) -> Self {
            // interlaced では pic_height_in_map_units_minus1 は field 単位（= 半分の高さ）
            assert!(
                raw_height_field.is_multiple_of(16),
                "raw_height_field は 16 の倍数で指定すること"
            );
            self.frame_mbs_only_flag = false;
            self.pic_height_in_map_units_minus1 = raw_height_field / 16 - 1;
            self
        }

        fn with_pic_order_cnt_type_1(mut self) -> Self {
            self.pic_order_cnt_type = 1;
            self
        }

        fn with_cropping(mut self, left: u32, right: u32, top: u32, bottom: u32) -> Self {
            self.frame_cropping_flag = true;
            self.crop_offsets = (left, right, top, bottom);
            self
        }

        fn with_pic_width_in_mbs_minus1(mut self, value: u32) -> Self {
            self.pic_width_in_mbs_minus1 = value;
            self
        }

        fn build(self) -> Vec<u8> {
            let mut w = SpsBitWriter::new();

            // NAL ヘッダ: forbidden_zero_bit=0, nal_ref_idc=3 (binary 011), nal_unit_type=7 (binary 00111)
            // → 0x67
            w.write_u(8, 0x67);

            // profile_idc (u(8))
            w.write_u(8, u32::from(self.profile_idc));
            // constraint_set*_flag (6 bit) + reserved_zero_2bits (2 bit)
            w.write_u(8, 0);
            // level_idc (u(8)): 適当に Level 3.1
            w.write_u(8, 31);
            // seq_parameter_set_id
            w.write_ue(0);

            // High 系プロファイルの追加フィールド
            let is_high = H264_HIGH_PROFILES.contains(&self.profile_idc);
            if is_high {
                w.write_ue(1); // chroma_format_idc = 1 (4:2:0)
                // chroma_format_idc != 3 のため separate_colour_plane_flag は書かない
                w.write_ue(0); // bit_depth_luma_minus8
                w.write_ue(0); // bit_depth_chroma_minus8
                w.write_u(1, 0); // qpprime_y_zero_transform_bypass_flag
                w.write_u(
                    1,
                    if self.seq_scaling_matrix_present_flag {
                        1
                    } else {
                        0
                    },
                );
                if self.seq_scaling_matrix_present_flag {
                    // 全 seq_scaling_list_present_flag を 0 にして scaling_list 本体を読まない経路で進める
                    for _ in 0..8 {
                        w.write_u(1, 0);
                    }
                }
            }

            // log2_max_frame_num_minus4
            w.write_ue(0);
            // pic_order_cnt_type
            w.write_ue(self.pic_order_cnt_type);
            match self.pic_order_cnt_type {
                0 => {
                    w.write_ue(0); // log2_max_pic_order_cnt_lsb_minus4
                }
                1 => {
                    w.write_u(1, 0); // delta_pic_order_always_zero_flag
                    w.write_se(0); // offset_for_non_ref_pic
                    w.write_se(0); // offset_for_top_to_bottom_field
                    w.write_ue(0); // num_ref_frames_in_pic_order_cnt_cycle (要素数 0)
                }
                _ => {}
            }
            w.write_ue(1); // max_num_ref_frames
            w.write_u(1, 0); // gaps_in_frame_num_value_allowed_flag
            w.write_ue(self.pic_width_in_mbs_minus1);
            w.write_ue(self.pic_height_in_map_units_minus1);
            w.write_u(1, if self.frame_mbs_only_flag { 1 } else { 0 });
            if !self.frame_mbs_only_flag {
                w.write_u(1, 0); // mb_adaptive_frame_field_flag
            }
            w.write_u(1, 0); // direct_8x8_inference_flag
            w.write_u(1, if self.frame_cropping_flag { 1 } else { 0 });
            if self.frame_cropping_flag {
                w.write_ue(self.crop_offsets.0);
                w.write_ue(self.crop_offsets.1);
                w.write_ue(self.crop_offsets.2);
                w.write_ue(self.crop_offsets.3);
            }
            // RBSP trailing bits は本実装では `pic_width_in_mbs_minus1` 以降まで読めれば不要なため省略する

            w.into_bytes()
        }
    }

    #[test]
    fn extract_dimensions_handles_high_profile_with_scaling_matrix() {
        // High profile かつ seq_scaling_matrix_present_flag=1（全 list_present_flag=0）で
        // scaling_list 本体を読まずに pic_width / pic_height まで正しく到達できること
        let sps = SpsBuilder::raw(1920, 1088)
            .with_high_profile_and_scaling_matrix()
            .build();
        let (width, height) = extract_dimensions_from_sps(&sps).expect("SPS パース成功");
        assert_eq!((width, height), (1920, 1088));
    }

    #[test]
    fn extract_dimensions_handles_pic_order_cnt_type_1() {
        // pic_order_cnt_type=1 の経路（delta_pic_order_always_zero_flag / offset_for_*）を踏んでも
        // pic_width / pic_height まで正しく到達できること
        let sps = SpsBuilder::raw(1920, 1088)
            .with_pic_order_cnt_type_1()
            .build();
        let (width, height) = extract_dimensions_from_sps(&sps).expect("SPS パース成功");
        assert_eq!((width, height), (1920, 1088));
    }

    #[test]
    fn extract_dimensions_handles_interlaced_frame_mbs_only_flag_zero() {
        // frame_mbs_only_flag=0 のとき mb_adaptive_frame_field_flag を読む経路を踏み、
        // raw_height = (pic_height_in_map_units_minus1 + 1) * 16 * 2 と算出されること。
        // field 単位の高さ 544 を渡すと frame 高さ 1088 が得られる。
        let sps = SpsBuilder::raw(1920, 1088).with_interlaced(544).build();
        let (width, height) = extract_dimensions_from_sps(&sps).expect("SPS パース成功");
        assert_eq!((width, height), (1920, 1088));
    }

    #[test]
    fn extract_dimensions_handles_frame_cropping_to_1080() {
        // ffmpeg / libx264 が 1920x1080 を表現する際の典型パターン:
        //   raw 1920x1088 + frame_cropping_flag=1 + crop_bottom=4
        //   (CropUnitY=2, 2*(0+4)=8 を 1088 から削って 1080)
        let sps = SpsBuilder::raw(1920, 1088)
            .with_cropping(0, 0, 0, 4)
            .build();
        let (width, height) = extract_dimensions_from_sps(&sps).expect("SPS パース成功");
        assert_eq!((width, height), (1920, 1080));
    }

    #[test]
    fn extract_dimensions_rejects_crop_underflow() {
        // crop 値が raw 解像度を超える場合は Err を返すこと（checked_sub アンダーフロー）。
        // 1 MB ( = 16x16) の raw 解像度に対し、crop で横を 400 = 2*(100+100) 削ろうとする。
        let sps = SpsBuilder::raw(16, 16)
            .with_cropping(100, 100, 0, 0)
            .build();
        let result = extract_dimensions_from_sps(&sps);
        assert!(
            result.is_err(),
            "crop アンダーフローは Err を返すはず: {result:?}"
        );
    }

    #[test]
    fn extract_dimensions_rejects_zero_dimensions_after_cropping() {
        // crop 適用後に width / height が 0 になる場合は Err を返すこと。
        // raw 1MB x 1MB (16x16) から crop で 16 横 削るとちょうど 0 になる。
        // CropUnitX=2 のため crop_left=4, crop_right=4 で 2*(4+4)=16 削る。
        let sps = SpsBuilder::raw(16, 32).with_cropping(4, 4, 0, 0).build();
        let result = extract_dimensions_from_sps(&sps);
        assert!(
            result.is_err(),
            "crop 後に width が 0 になる場合は Err を返すはず: {result:?}"
        );
    }

    #[test]
    fn extract_dimensions_does_not_panic_on_huge_pic_width() {
        // pic_width_in_mbs_minus1 が巨大値の場合、checked_mul で Err になることがある。
        // 32 bit 環境ではオーバーフローで Err、64 bit 環境では巨大値で Ok になることがあるため、
        // ここではパニックしないこと（Ok か Err のどちらかが返ること）だけ確認する。
        let sps = SpsBuilder::raw(16, 16)
            .with_pic_width_in_mbs_minus1(u32::MAX / 2)
            .build();
        let _ = extract_dimensions_from_sps(&sps);
    }

    #[test]
    fn extract_dimensions_rejects_width_exceeding_u16_max() {
        // pic_width_in_mbs_minus1=4095 で raw_width=65536 (= u16::MAX + 1) になり、
        // `sample_entry_visual_fields` の `width as u16` で 0 にラップする入力。
        // u16 上限を超える解像度は Err で弾くこと（外部入力で MP4 sample_entry に 0 が埋まらないため）。
        let sps = SpsBuilder::raw(16, 16)
            .with_pic_width_in_mbs_minus1(4095)
            .build();
        let result = extract_dimensions_from_sps(&sps);
        assert!(
            result.is_err(),
            "u16::MAX を超える width は Err を返すはず: {result:?}"
        );
    }
}
