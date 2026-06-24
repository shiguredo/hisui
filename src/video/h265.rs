use shiguredo_mp4::boxes::{Hvc1Box, HvccBox, HvccNalUintArray, SampleEntry};

use crate::video::{
    self, FrameRate, VideoFrameSize, bit_reader::BitReader, h264::find_next_annexb_start_code,
};

// H.265 の NAL ユニット前に付与されるサイズのバイト数
// Sora / Hisui が生成するものは全て 4 バイトなので固定値でいい（H.264と同様）
pub use crate::video::h264::NALU_HEADER_LENGTH;

// H.265 の NAL ユニットタイプ
pub const H265_NALU_TYPE_VPS: u8 = 32;
pub const H265_NALU_TYPE_SPS: u8 = 33;
pub const H265_NALU_TYPE_PPS: u8 = 34;

// 仕様準拠 publisher が出す general_profile_idc の許容値群（ITU-T H.265 仕様 Annex A.3）。
// Main / Main 10 / Main Still Picture / Format Range Extensions / High Throughput /
// Multiview Main / Scalable Main / Screen Content Coding をカバーする。
// Hisui の入力前提 (video_toolbox / nvcodec) もこの範囲に含まれる。
const H265_ALLOWED_PROFILE_IDCS: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 9];

/// Annex.B 形式の H.265 をパースして、含まれている NAL ユニットを走査するためのイテレーター
///
/// H.265 の NAL ユニットヘッダは 2 バイト構造（ITU-T H.265 仕様 7.3.1.2）で、
/// 第 1 バイトに forbidden_zero_bit (1 bit) と nal_unit_type (6 bit)、
/// 第 2 バイトに nuh_layer_id (6 bit) と nuh_temporal_id_plus1 (3 bit) が分割配置される。
/// 本イテレーターは start code 直後の 1 バイトから nal_unit_type を抽出するのみで、
/// 第 2 バイト以降は呼び出し側に渡す `data` に含めて返す。
#[derive(Debug)]
pub struct H265AnnexBNalUnits<'a> {
    data: &'a [u8],
}

impl<'a> H265AnnexBNalUnits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn next_nal_unit(&mut self) -> crate::Result<Option<H265NalUnit<'a>>> {
        if self.data.is_empty() {
            return Ok(None);
        }

        if self.data.starts_with(&[0, 0, 1]) {
            self.data = &self.data[3..];
        } else if self.data.starts_with(&[0, 0, 0, 1]) {
            self.data = &self.data[4..];
        } else {
            return Err(crate::Error::new("no H.265 start code prefix"));
        };
        if self.data.is_empty() {
            return Err(crate::Error::new("empty H.265 NAL unit"));
        }

        let header = self.data[0];
        if (header >> 7) != 0 {
            return Err(crate::Error::new(
                "invalid H.265 NAL header: forbidden_zero_bit is set",
            ));
        }

        // H.265 の nal_unit_type は NAL ヘッダ第 1 バイトの bit 1-6 (上位ビット側から 2 番目を MSB とする 6 ビット)
        let nal_unit_type = (header >> 1) & 0x3F;

        let i = find_next_annexb_start_code(self.data).unwrap_or(self.data.len());
        let data = &self.data[..i];
        self.data = &self.data[i..];
        Ok(Some(H265NalUnit {
            ty: nal_unit_type,
            data,
        }))
    }
}

impl<'a> Iterator for H265AnnexBNalUnits<'a> {
    type Item = crate::Result<H265NalUnit<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_nal_unit().transpose()
    }
}

/// H.265 の Annex-B NAL ユニット 1 個分
///
/// `ty` は NAL ヘッダ第 1 バイトの bit 1-6 (`(byte[0] >> 1) & 0x3F`) から抽出した
/// nal_unit_type (6 bit、0..=63、`H265_NALU_TYPE_*` と比較する)。
/// `data` は start code を除いた NAL バイト列で、NAL ヘッダ 2 バイトを含む EBSP 形式
/// (emulation prevention byte 込み、未除去)。RBSP への変換 (emulation prevention byte の
/// 除去) は呼び出し側 (例: `rbsp_from_hevc_sps_nalu`) で実施する。
#[derive(Debug)]
pub struct H265NalUnit<'a> {
    pub ty: u8,
    pub data: &'a [u8],
}

/// SPS NAL ユニットから取り出した hvcC 反映用フィールド群
///
/// `parse_hevc_sps` の戻り値で、`h265_sample_entry_from_vps_sps_pps_lists` 経由で
/// `HvccBox` の対応フィールドにマップされる。
#[derive(Debug)]
struct HevcSpsParams {
    /// profile_tier_level の general_profile_space (u(2)、ITU-T H.265 仕様 7.4.4)
    general_profile_space: u8,
    /// profile_tier_level の general_tier_flag (u(1))
    general_tier_flag: u8,
    /// profile_tier_level の general_profile_idc (u(5))
    general_profile_idc: u8,
    /// profile_tier_level の general_profile_compatibility_flag[0..32] を MSB ファーストで詰めた値
    general_profile_compatibility_flags: u32,
    /// profile_tier_level の general_progressive_source_flag 以下の連続 48 bit を u64 の bit 47..0 に詰めた値
    general_constraint_indicator_flags: u64,
    /// profile_tier_level の general_level_idc (u(8))
    general_level_idc: u8,
    /// SPS の sps_max_sub_layers_minus1 (u(3)、仕様 7.4.3.2.1 で 0..=6)
    sps_max_sub_layers_minus1: u8,
    /// SPS の sps_temporal_id_nesting_flag (u(1))
    sps_temporal_id_nesting_flag: u8,
    /// SPS の chroma_format_idc (0..=3、parse_hevc_sps で範囲検証済み)
    chroma_format_idc: u8,
    /// SPS の bit_depth_luma_minus8 (0..=7、HvccBox の Uint<u8, 3> 制約に整合)
    bit_depth_luma_minus8: u8,
    /// SPS の bit_depth_chroma_minus8 (0..=7、HvccBox の Uint<u8, 3> 制約に整合)
    bit_depth_chroma_minus8: u8,
    /// conformance window 適用後の width (parse_hevc_sps 内で > 0 / u16::MAX 上限を保証)
    width: u16,
    /// conformance window 適用後の height
    height: u16,
}

/// SPS NAL ユニットから profile_tier_level / chroma / bit_depth / 解像度を抽出する内部関数
///
/// 入力 `sps` は `H265AnnexBNalUnits` が返す `H265NalUnit.data` をそのまま渡す形式で、
/// 先頭 2 バイトに NAL ヘッダ（forbidden_zero_bit + nal_unit_type + nuh_layer_id +
/// nuh_temporal_id_plus1）を含む。内部で `rbsp_from_hevc_sps_nalu` を呼んで NAL タイプ
/// 検査と RBSP 抽出（emulation prevention byte 除去）を行い、ITU-T H.265 仕様 7.3.2.2.1 /
/// 7.3.3 / 7.4.3 に従って Exp-Golomb（ue(v)）と固定長ビットフィールド（u(n)）でフィールドを
/// 読み取る。
///
/// 仕様準拠 publisher のプロファイル群 (`H265_ALLOWED_PROFILE_IDCS`) 以外、または仕様値域外の
/// `chroma_format_idc` / `bit_depth_*_minus8` / `sps_max_sub_layers_minus1`、conformance
/// window 適用後の解像度ゼロや u16::MAX 超過を検出した場合は Err を返す。
fn parse_hevc_sps(sps: &[u8]) -> crate::Result<HevcSpsParams> {
    let rbsp = rbsp_from_hevc_sps_nalu(sps)?;
    let mut reader = BitReader::new(&rbsp);

    // sps_video_parameter_set_id (u(4))
    reader.skip_u(4)?;
    // sps_max_sub_layers_minus1 (u(3))
    let sps_max_sub_layers_minus1 = reader.read_u(3)? as u8;
    if sps_max_sub_layers_minus1 > 6 {
        return Err(crate::Error::new(format!(
            "invalid H.265 SPS: sps_max_sub_layers_minus1 out of spec range (0..=6): {sps_max_sub_layers_minus1}"
        )));
    }
    // sps_temporal_id_nesting_flag (u(1))
    let sps_temporal_id_nesting_flag = reader.read_u(1)? as u8;

    // profile_tier_level(1, sps_max_sub_layers_minus1) （仕様 7.3.3）
    let general_profile_space = reader.read_u(2)? as u8;
    let general_tier_flag = reader.read_u(1)? as u8;
    let general_profile_idc = reader.read_u(5)? as u8;
    if !H265_ALLOWED_PROFILE_IDCS.contains(&general_profile_idc) {
        return Err(crate::Error::new(format!(
            "invalid H.265 SPS: unsupported general_profile_idc {general_profile_idc}"
        )));
    }
    let general_profile_compatibility_flags = reader.read_u(32)?;
    // 48 bit の constraint indicator flags は BitReader::read_u が n > 32 で Err を返す
    // 制約のため、上位 32 bit + 下位 16 bit に分けて読んでから u64 に組み立てる。
    let constraint_upper = reader.read_u(32)? as u64;
    let constraint_lower = reader.read_u(16)? as u64;
    let general_constraint_indicator_flags = (constraint_upper << 16) | constraint_lower;
    let general_level_idc = reader.read_u(8)? as u8;

    // サブレイヤー毎の present flag を読みつつ保持する。後段の sub_layer profile / level
    // 領域 skip は各 flag の値に依存するため一旦保持する必要があるが、
    // sps_max_sub_layers_minus1 は parse_hevc_sps で 0..=6 に制限済みなので
    // 最大 6 要素の固定配列で十分 (Hisui の Single layer 前提 sps_max_sub_layers_minus1 == 0 では
    // ループは 1 回も実行されない)。
    let mut sub_layer_present_flags: [(u8, u8); 6] = [(0, 0); 6];
    let sub_layer_count = sps_max_sub_layers_minus1 as usize;
    for slot in sub_layer_present_flags.iter_mut().take(sub_layer_count) {
        let prof = reader.read_u(1)? as u8;
        let lvl = reader.read_u(1)? as u8;
        *slot = (prof, lvl);
    }
    // sps_max_sub_layers_minus1 > 0 のとき reserved_zero_2bits[i] を i = sps_max_sub_layers_minus1..8 個読む
    // （合計 (8 - sps_max_sub_layers_minus1) * 2 bit）。sps_max_sub_layers_minus1 == 0 のときは読まない。
    if sps_max_sub_layers_minus1 > 0 {
        let reserved_bits = (8 - sps_max_sub_layers_minus1 as usize) * 2;
        reader.skip_u(reserved_bits)?;
    }
    // 各サブレイヤーの profile / level 領域を flag に応じて skip する。
    // sub_layer profile 88 bit = profile_space(2) + tier_flag(1) + profile_idc(5) +
    // profile_compatibility(32) + constraint_indicator_flags(48)。
    // sub_layer_level_idc は 8 bit。
    for &(prof, lvl) in sub_layer_present_flags.iter().take(sub_layer_count) {
        if prof == 1 {
            reader.skip_u(2)?;
            reader.skip_u(1)?;
            reader.skip_u(5)?;
            reader.skip_u(32)?;
            reader.skip_u(32)?;
            reader.skip_u(16)?;
        }
        if lvl == 1 {
            reader.skip_u(8)?;
        }
    }

    // sps_seq_parameter_set_id (ue(v))
    reader.skip_ue()?;

    // chroma_format_idc (ue(v))
    let chroma_format_idc = read_ue_as_u8_bounded(&mut reader, 3, "chroma_format_idc")?;
    // separate_colour_plane_flag (u(1))（chroma_format_idc == 3 のときのみ）
    let separate_colour_plane_flag = if chroma_format_idc == 3 {
        reader.read_u(1)?
    } else {
        0
    };

    // pic_width_in_luma_samples / pic_height_in_luma_samples (ue(v))
    let pic_width_in_luma_samples = reader.read_ue()? as usize;
    let pic_height_in_luma_samples = reader.read_ue()? as usize;

    // conformance_window_flag (u(1))
    let conformance_window_flag = reader.read_u(1)?;
    let (conf_win_left, conf_win_right, conf_win_top, conf_win_bottom) =
        if conformance_window_flag == 1 {
            let l = reader.read_ue()? as usize;
            let r = reader.read_ue()? as usize;
            let t = reader.read_ue()? as usize;
            let b = reader.read_ue()? as usize;
            (l, r, t, b)
        } else {
            (0, 0, 0, 0)
        };

    // bit_depth_luma_minus8 / bit_depth_chroma_minus8 (ue(v))
    let bit_depth_luma_minus8 = read_ue_as_u8_bounded(&mut reader, 7, "bit_depth_luma_minus8")?;
    let bit_depth_chroma_minus8 = read_ue_as_u8_bounded(&mut reader, 7, "bit_depth_chroma_minus8")?;

    // conformance window 適用後の解像度を計算する（仕様 6.2 / 7.4.3.2.1 / Table 6-1）。
    let (sub_width_c, sub_height_c) =
        chroma_subsampling_factors(chroma_format_idc, separate_colour_plane_flag);
    let crop_x = conf_win_left
        .checked_add(conf_win_right)
        .and_then(|v| v.checked_mul(sub_width_c))
        .ok_or_else(|| {
            crate::Error::new(
                "invalid H.265 SPS: conformance window crop_x overflow during width calculation",
            )
        })?;
    let crop_y = conf_win_top
        .checked_add(conf_win_bottom)
        .and_then(|v| v.checked_mul(sub_height_c))
        .ok_or_else(|| {
            crate::Error::new(
                "invalid H.265 SPS: conformance window crop_y overflow during height calculation",
            )
        })?;
    let width = pic_width_in_luma_samples
        .checked_sub(crop_x)
        .ok_or_else(|| {
            crate::Error::new(
                "invalid H.265 SPS: conformance window crop_x exceeds pic_width_in_luma_samples (underflow)",
            )
        })?;
    let height = pic_height_in_luma_samples
        .checked_sub(crop_y)
        .ok_or_else(|| {
            crate::Error::new(
                "invalid H.265 SPS: conformance window crop_y exceeds pic_height_in_luma_samples (underflow)",
            )
        })?;

    if width == 0 || height == 0 {
        return Err(crate::Error::new(format!(
            "invalid H.265 SPS: zero dimensions after conformance window crop (width={width}, height={height})"
        )));
    }
    if width > u16::MAX as usize || height > u16::MAX as usize {
        return Err(crate::Error::new(format!(
            "invalid H.265 SPS: dimensions exceed u16::MAX (width={width}, height={height})"
        )));
    }

    Ok(HevcSpsParams {
        general_profile_space,
        general_tier_flag,
        general_profile_idc,
        general_profile_compatibility_flags,
        general_constraint_indicator_flags,
        general_level_idc,
        sps_max_sub_layers_minus1,
        sps_temporal_id_nesting_flag,
        chroma_format_idc,
        bit_depth_luma_minus8,
        bit_depth_chroma_minus8,
        width: width as u16,
        height: height as u16,
    })
}

/// ue(v) を読み出し、値が `max` 以下であることを検証してから u8 にキャストする内部ヘルパー
///
/// 仕様値域 (例: chroma_format_idc は 0..=3、bit_depth_*_minus8 は 0..=7) を超えた場合は
/// 統一フォーマットの `invalid H.265 SPS: <field_name> out of spec range (0..=<max>): <value>` Err を返す。
fn read_ue_as_u8_bounded(
    reader: &mut BitReader<'_>,
    max: u32,
    field_name: &str,
) -> crate::Result<u8> {
    let v = reader.read_ue()?;
    if v > max {
        return Err(crate::Error::new(format!(
            "invalid H.265 SPS: {field_name} out of spec range (0..={max}): {v}"
        )));
    }
    Ok(v as u8)
}

/// chroma_format_idc / separate_colour_plane_flag から (SubWidthC, SubHeightC) を返す
///
/// 仕様 6.2 / Table 6-1。conformance window cropping の単位として使う。
fn chroma_subsampling_factors(
    chroma_format_idc: u8,
    separate_colour_plane_flag: u32,
) -> (usize, usize) {
    // chroma_format_idc == 3 + separate_colour_plane_flag == 1 は ChromaArrayType = 0 として扱う
    if separate_colour_plane_flag == 1 {
        return (1, 1);
    }
    match chroma_format_idc {
        0 => (1, 1), // monochrome
        1 => (2, 2), // 4:2:0
        2 => (2, 1), // 4:2:2
        3 => (1, 1), // 4:4:4
        _ => unreachable!("chroma_format_idc は parse_hevc_sps で 0..=3 に絞られている"),
    }
}

/// SPS NAL ユニットから RBSP を抽出する
///
/// 先頭 2 バイトの NAL ヘッダ（H.265 の forbidden_zero_bit + nal_unit_type + nuh_layer_id +
/// nuh_temporal_id_plus1）を skip し、payload 内の emulation prevention byte
/// (`0x00 0x00 0x03`) を除去した RBSP バイト列を返す。
fn rbsp_from_hevc_sps_nalu(nalu: &[u8]) -> crate::Result<Vec<u8>> {
    if nalu.len() < 2 {
        return Err(crate::Error::new(
            "invalid H.265 SPS: NAL unit too short (expected >= 2 bytes)",
        ));
    }
    let nal_unit_type = (nalu[0] >> 1) & 0x3F;
    if nal_unit_type != H265_NALU_TYPE_SPS {
        return Err(crate::Error::new(format!(
            "invalid H.265 SPS: expected nal_unit_type={H265_NALU_TYPE_SPS}, got {nal_unit_type}"
        )));
    }
    let payload = &nalu[2..];
    let mut rbsp = Vec::with_capacity(payload.len());
    let mut i = 0;
    while i < payload.len() {
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

/// VPS / SPS / PPS リストから HVC1 サンプルエントリーと cropping 適用後の解像度を構築する
///
/// 入力 `vps_list[i]` / `sps_list[i]` / `pps_list[j]` は ISO/IEC 14496-15 §8.3.3.1 で
/// 定義された EBSP 形式 (emulation prevention byte 込み、NAL ヘッダ 2 バイト含む、start code
/// なし)。`H265AnnexBNalUnits::next` が返す `H265NalUnit.data` の形式と
/// `HvccBox.nalu_arrays[*].nalus[*]` の格納形式に揃える。
///
/// 各 list の全要素に対して先頭バイトの上位ビット側 nal_unit_type
/// (`(byte[0] >> 1) & 0x3F`) を検査し、対応する NAL タイプ
/// (`H265_NALU_TYPE_VPS` / `SPS` / `PPS`) と一致しない場合は Err を返す。
/// 呼び出し側の事前判定漏れで VPS や IDR が誤って `sps_list` / `pps_list` に混入した場合に
/// `HvccBox.nalu_arrays` へ壊れた NAL が move されるのを防ぐ。
///
/// 内部で `parse_hevc_sps(sps_list[0])` を 1 回呼んで SPS パラメータを取り出し、hvcC の
/// 各フィールドに反映する。複数 SPS は先頭 SPS のパラメータのみを採用し、
/// `HvccBox.nalu_arrays[SPS]` には全 SPS を move する (Hisui の入力前提では複数 SPS は
/// 同一内容を想定)。VPS / PPS リストも `HvccBox.nalu_arrays` にそれぞれ move する。
///
/// 戻り値タプルの `VideoFrameSize` は SPS 由来の cropping 適用後解像度。
/// `parse_hevc_sps` 内で width / height > 0 と u16::MAX 上限を保証しているため
/// `VideoFrameSize::new` は infallible。呼び出し側で `VideoFrame.size` を encoder 設定値
/// などから別途構築する場合は、タプルの第 2 要素 `VideoFrameSize` を `_` で捨ててよい
/// (encoder 経路の既存挙動)。
pub fn h265_sample_entry_from_vps_sps_pps_lists(
    vps_list: Vec<Vec<u8>>,
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
    fps: FrameRate,
) -> crate::Result<(SampleEntry, VideoFrameSize)> {
    if vps_list.is_empty() {
        return Err(crate::Error::new("missing H.265 VPS"));
    }
    if sps_list.is_empty() {
        return Err(crate::Error::new("missing H.265 SPS"));
    }
    if pps_list.is_empty() {
        return Err(crate::Error::new("missing H.265 PPS"));
    }

    // VPS / SPS / PPS の全要素に対して NAL タイプ検査を実施する。
    // 呼び出し側 (video_toolbox / nvcodec) の事前判定漏れで HvccBox.nalu_arrays に
    // 壊れた NAL が move されるのを防ぐ防御的検査。
    check_nal_unit_types(&vps_list, H265_NALU_TYPE_VPS, "VPS")?;
    check_nal_unit_types(&sps_list, H265_NALU_TYPE_SPS, "SPS")?;
    check_nal_unit_types(&pps_list, H265_NALU_TYPE_PPS, "PPS")?;

    let params = parse_hevc_sps(&sps_list[0])?;

    let frame_size = VideoFrameSize::new(params.width as usize, params.height as usize).expect(
        "infallible: parse_hevc_sps validates positive width / height and u16::MAX upper bound",
    );

    // Hisui ではフレームレートは固定（整数にならない場合は切り上げ）
    let avg_frame_rate = (fps.numerator.get().div_ceil(fps.denumerator.get())) as u16;

    let entry = SampleEntry::Hvc1(Hvc1Box {
        visual: video::sample_entry_visual_fields(params.width as usize, params.height as usize),
        hvcc_box: HvccBox {
            general_profile_space: shiguredo_mp4::Uint::new(params.general_profile_space),
            general_tier_flag: shiguredo_mp4::Uint::new(params.general_tier_flag),
            general_profile_idc: shiguredo_mp4::Uint::new(params.general_profile_idc),
            general_profile_compatibility_flags: params.general_profile_compatibility_flags,
            general_constraint_indicator_flags: shiguredo_mp4::Uint::new(
                params.general_constraint_indicator_flags,
            ),
            general_level_idc: params.general_level_idc,
            num_temporal_layers: shiguredo_mp4::Uint::new(params.sps_max_sub_layers_minus1 + 1),
            temporal_id_nested: shiguredo_mp4::Uint::new(params.sps_temporal_id_nesting_flag),

            // SPS VUI / PPS から正確な値を抽出する処理は未実装で、固定値 0 を維持する。
            min_spatial_segmentation_idc: shiguredo_mp4::Uint::new(0),
            parallelism_type: shiguredo_mp4::Uint::new(0),

            avg_frame_rate,
            // Hisui は CFR (固定フレームレート) 前提
            constant_frame_rate: shiguredo_mp4::Uint::new(1),
            // Hisui ではヘッダサイズが固定であることが前提
            length_size_minus_one: shiguredo_mp4::Uint::new(NALU_HEADER_LENGTH as u8 - 1),

            chroma_format_idc: shiguredo_mp4::Uint::new(params.chroma_format_idc),
            bit_depth_luma_minus8: shiguredo_mp4::Uint::new(params.bit_depth_luma_minus8),
            bit_depth_chroma_minus8: shiguredo_mp4::Uint::new(params.bit_depth_chroma_minus8),

            nalu_arrays: vec![
                hvcc_nalu_array(H265_NALU_TYPE_VPS, vps_list),
                hvcc_nalu_array(H265_NALU_TYPE_SPS, sps_list),
                hvcc_nalu_array(H265_NALU_TYPE_PPS, pps_list),
            ],
        },
        unknown_boxes: Vec::new(),
    });

    Ok((entry, frame_size))
}

/// `vps_list` / `sps_list` / `pps_list` の各要素に対して NAL タイプ検査を実施する内部ヘルパー
///
/// 先頭バイトから `(byte >> 1) & 0x3F` で nal_unit_type を抽出し、期待する `expected_ty` と
/// 一致するかを確認する。空 NAL や型不一致を検出した場合は Err を返す。
fn check_nal_unit_types(nalus: &[Vec<u8>], expected_ty: u8, label: &str) -> crate::Result<()> {
    for (i, nalu) in nalus.iter().enumerate() {
        if nalu.is_empty() {
            return Err(crate::Error::new(format!(
                "invalid H.265 {label} at index {i}: empty NAL"
            )));
        }
        let nal_unit_type = (nalu[0] >> 1) & 0x3F;
        if nal_unit_type != expected_ty {
            return Err(crate::Error::new(format!(
                "invalid H.265 {label} at index {i}: expected nal_unit_type={expected_ty}, got {nal_unit_type}"
            )));
        }
    }
    Ok(())
}

fn hvcc_nalu_array(nalu_type: u8, nalus: Vec<Vec<u8>>) -> HvccNalUintArray {
    HvccNalUintArray {
        array_completeness: shiguredo_mp4::Uint::new(1), // true
        nal_unit_type: shiguredo_mp4::Uint::new(nalu_type),
        nalus,
    }
}

/// Annex-B バイト列から HVC1 サンプルエントリーを構築する薄いラッパー
///
/// 内部で `H265AnnexBNalUnits` を 1 回走査して VPS / SPS / PPS NAL のみを抽出し、
/// `h265_sample_entry_from_vps_sps_pps_lists` を呼ぶ。VCL / SEI / EOS / EOB 等の
/// 他の NAL タイプは無視する。
///
/// 引数として `width` / `height` は受け取らない (SPS 由来の実値を hvcC と visual に反映するため)。
pub fn h265_sample_entry_from_annexb(data: &[u8], fps: FrameRate) -> crate::Result<SampleEntry> {
    let mut vps_list = Vec::new();
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();
    for nalu in H265AnnexBNalUnits::new(data) {
        let nalu = nalu?;
        match nalu.ty {
            H265_NALU_TYPE_VPS => vps_list.push(nalu.data.to_vec()),
            H265_NALU_TYPE_SPS => sps_list.push(nalu.data.to_vec()),
            H265_NALU_TYPE_PPS => pps_list.push(nalu.data.to_vec()),
            _ => {}
        }
    }
    let (entry, _frame_size) =
        h265_sample_entry_from_vps_sps_pps_lists(vps_list, sps_list, pps_list, fps)?;
    Ok(entry)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // テスト用に VPS / SPS / PPS の NAL ヘッダ 2 バイトを定数化する。
    // 第 1 バイト: forbidden_zero_bit (1 bit) = 0 / nal_unit_type (6 bit) / nuh_layer_id 最上位 1 bit = 0
    // 第 2 バイト: nuh_layer_id 下位 5 bit = 0 / nuh_temporal_id_plus1 (3 bit) = 1
    // VPS (nal_unit_type=32): (32 << 1) | 0 = 0x40
    const VPS_HEADER: [u8; 2] = [0x40, 0x01];
    // SPS (nal_unit_type=33): (33 << 1) | 0 = 0x42
    const SPS_HEADER: [u8; 2] = [0x42, 0x01];
    // PPS (nal_unit_type=34): (34 << 1) | 0 = 0x44
    const PPS_HEADER: [u8; 2] = [0x44, 0x01];

    // 以下の SPS バイト列は ffmpeg + libx265 で生成した実機 SPS を抽出したもの。
    // 生成コマンドは `ffmpeg -f lavfi -i testsrc=size=WIDTHxHEIGHT:rate=30 -pix_fmt yuv420p
    // -c:v libx265 -frames:v 1 -f hevc out.h265` で、Annex-B 形式の出力から SPS NAL
    // (nal_unit_type=33、start code 直後の 1 バイト目が 0x42 で始まる NAL) を
    // 次の start code 直前まで切り出した。emulation prevention byte (`0x00 0x00 0x03`) も
    // そのまま含まれており、`rbsp_from_hevc_sps_nalu` 経路での RBSP 抽出を実機データで担保する。
    //
    // 各 SPS は本モジュール外のテスト (将来追加される decoder / rtmp / srt 等の H.265 テスト) からも
    // 参照可能なように `pub(crate)` で公開する。

    // x265 4.1 が `testsrc=640x480` Main profile / Level-3 で出力する SPS。
    // 16 の倍数解像度で conformance window 不要。emulation prevention byte が 5 箇所含まれる。
    pub(crate) const HEVC_SPS_640X480: [u8; 42] = [
        0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00, 0x00,
        0x03, 0x00, 0x5a, 0xa0, 0x05, 0x02, 0x01, 0xe1, 0x65, 0x95, 0x9a, 0x49, 0x32, 0xbc, 0x05,
        0xa0, 0x20, 0x00, 0x00, 0x03, 0x00, 0x20, 0x00, 0x00, 0x03, 0x03, 0xc1,
    ];

    // x265 4.1 が `testsrc=1920x1080` Main profile / Level-4 で出力する SPS。
    // raw 1920x1088 + conformance_window で 1920x1080 を表現する典型パターン。
    // emulation prevention byte が 4 箇所含まれる。
    pub(crate) const HEVC_SPS_1920X1080: [u8; 42] = [
        0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00, 0x00,
        0x03, 0x00, 0x78, 0xa0, 0x03, 0xc0, 0x80, 0x10, 0xe5, 0x96, 0x56, 0x69, 0x24, 0xca, 0xf0,
        0x16, 0x80, 0x80, 0x00, 0x00, 0x03, 0x00, 0x80, 0x00, 0x00, 0x0f, 0x04,
    ];

    #[test]
    fn h265_annexb_iterator_parses_vps_sps_pps_with_4byte_start_code() {
        // 4 バイト start code [0, 0, 0, 1] で区切られた VPS / SPS / PPS を順に取り出せること
        let mut data = Vec::new();
        data.extend_from_slice(&[0, 0, 0, 1]);
        data.extend_from_slice(&VPS_HEADER);
        data.push(0xaa);
        data.extend_from_slice(&[0, 0, 0, 1]);
        data.extend_from_slice(&SPS_HEADER);
        data.push(0xbb);
        data.extend_from_slice(&[0, 0, 0, 1]);
        data.extend_from_slice(&PPS_HEADER);
        data.push(0xcc);

        let nalus: Vec<_> = H265AnnexBNalUnits::new(&data)
            .collect::<crate::Result<Vec<_>>>()
            .expect("3 個の NAL ユニットを取り出せること");
        assert_eq!(nalus.len(), 3);
        assert_eq!(nalus[0].ty, H265_NALU_TYPE_VPS);
        assert_eq!(nalus[0].data, &[0x40, 0x01, 0xaa]);
        assert_eq!(nalus[1].ty, H265_NALU_TYPE_SPS);
        assert_eq!(nalus[1].data, &[0x42, 0x01, 0xbb]);
        assert_eq!(nalus[2].ty, H265_NALU_TYPE_PPS);
        assert_eq!(nalus[2].data, &[0x44, 0x01, 0xcc]);
    }

    #[test]
    fn h265_annexb_iterator_parses_with_3byte_start_code() {
        // 3 バイト start code [0, 0, 1] でも NAL タイプを取り出せること
        let mut data = Vec::new();
        data.extend_from_slice(&[0, 0, 1]);
        data.extend_from_slice(&VPS_HEADER);
        data.push(0x55);

        let nalus: Vec<_> = H265AnnexBNalUnits::new(&data)
            .collect::<crate::Result<Vec<_>>>()
            .expect("3 バイト start code でも 1 個取り出せること");
        assert_eq!(nalus.len(), 1);
        assert_eq!(nalus[0].ty, H265_NALU_TYPE_VPS);
    }

    #[test]
    fn h265_annexb_iterator_handles_mixed_3byte_and_4byte_start_codes() {
        // 実機エンコーダ出力で起こり得る 4 バイト → 3 バイト → 4 バイト の start code 混在を
        // 正しく 3 個の NAL に切り分けられること。
        let mut data = Vec::new();
        data.extend_from_slice(&[0, 0, 0, 1]); // 4 バイト start code
        data.extend_from_slice(&VPS_HEADER);
        data.push(0xaa);
        data.extend_from_slice(&[0, 0, 1]); // 3 バイト start code
        data.extend_from_slice(&SPS_HEADER);
        data.push(0xbb);
        data.extend_from_slice(&[0, 0, 0, 1]); // 4 バイト start code
        data.extend_from_slice(&PPS_HEADER);
        data.push(0xcc);

        let nalus: Vec<_> = H265AnnexBNalUnits::new(&data)
            .collect::<crate::Result<Vec<_>>>()
            .expect("start code 混在でも 3 個取り出せること");
        assert_eq!(nalus.len(), 3);
        assert_eq!(nalus[0].ty, H265_NALU_TYPE_VPS);
        assert_eq!(nalus[0].data, &[0x40, 0x01, 0xaa]);
        assert_eq!(nalus[1].ty, H265_NALU_TYPE_SPS);
        assert_eq!(nalus[1].data, &[0x42, 0x01, 0xbb]);
        assert_eq!(nalus[2].ty, H265_NALU_TYPE_PPS);
        assert_eq!(nalus[2].data, &[0x44, 0x01, 0xcc]);
    }

    #[test]
    fn h265_annexb_iterator_returns_none_for_empty_input() {
        // 空入力ではイテレーターが None を返すこと
        let mut iter = H265AnnexBNalUnits::new(&[]);
        assert!(iter.next().is_none());
    }

    #[test]
    fn h265_annexb_iterator_rejects_missing_start_code_prefix() {
        // start code 無しでイテレートすると Err になること
        let data = [0x40, 0x01, 0xaa];
        let mut iter = H265AnnexBNalUnits::new(&data);
        let result = iter
            .next()
            .expect("start code 無しの先頭 NAL は Err を返す");
        assert!(result.is_err(), "start code 無しは Err: {result:?}");
    }

    #[test]
    fn h265_annexb_iterator_rejects_empty_nal_unit() {
        // start code 直後がデータ終端だと「empty H.265 NAL unit」 Err になること
        let data = [0, 0, 0, 1];
        let mut iter = H265AnnexBNalUnits::new(&data);
        let result = iter.next().expect("空 NAL ユニットは Err を返す");
        assert!(result.is_err(), "空 NAL は Err: {result:?}");
    }

    #[test]
    fn h265_annexb_iterator_rejects_forbidden_zero_bit_set() {
        // forbidden_zero_bit (NAL ヘッダ第 1 バイトの MSB) が 1 だと Err になること
        let mut data = Vec::new();
        data.extend_from_slice(&[0, 0, 0, 1]);
        data.push(0x80 | 0x40); // forbidden_zero_bit = 1, nal_unit_type = 32
        data.push(0x01);

        let mut iter = H265AnnexBNalUnits::new(&data);
        let result = iter
            .next()
            .expect("forbidden_zero_bit 立ちの NAL は Err を返す");
        assert!(result.is_err(), "forbidden_zero_bit 立ちは Err: {result:?}");
    }

    #[test]
    fn h265_annexb_iterator_handles_trailing_3byte_start_code() {
        // バッファ末尾が次の NAL の 3 バイト start code で終わる場合、現在の NAL に
        // start code が混入しないこと (windows(4) ベースの探索では末尾 3 バイトを検出できず
        // 現在 NAL の末尾に start code が混入する回帰を防ぐ)。
        let mut data = Vec::new();
        data.extend_from_slice(&[0, 0, 0, 1]);
        data.extend_from_slice(&VPS_HEADER);
        data.push(0xaa);
        data.extend_from_slice(&[0, 0, 1]); // 末尾 3 バイト start code (NAL ボディなし)

        let mut iter = H265AnnexBNalUnits::new(&data);
        let nalu = iter
            .next()
            .expect("最初の NAL がある")
            .expect("最初の NAL のパース成功");
        // 現在 NAL の data には末尾 3 バイト start code が混入しない
        assert_eq!(nalu.ty, H265_NALU_TYPE_VPS);
        assert_eq!(nalu.data, &[0x40, 0x01, 0xaa]);
    }

    #[test]
    fn h265_annexb_iterator_extracts_nal_type_from_upper_6_bits() {
        // NAL タイプ抽出が `(byte >> 1) & 0x3F` で行われること
        // (H.264 の `& 0x1F` との違いを担保する)
        // nal_unit_type = 32 (VPS) のとき byte 0 = (32 << 1) | 0 = 0x40 で取り出せる
        let mut data = Vec::new();
        data.extend_from_slice(&[0, 0, 0, 1]);
        data.push(0x40);
        data.push(0x01);
        data.push(0xff);

        let nalus: Vec<_> = H265AnnexBNalUnits::new(&data)
            .collect::<crate::Result<Vec<_>>>()
            .expect("VPS NAL を取り出せること");
        assert_eq!(
            nalus[0].ty, 32,
            "nal_unit_type = 32 (VPS) として抽出されること"
        );
    }

    // ----------------------------------------------------------------
    // 仕様準拠の H.265 SPS バイト列ビルダー（テスト専用）
    //
    // ITU-T H.265 仕様 7.3.2.2.1 / 7.3.3 / 7.4.3 に従って SPS を組み立てる。
    // Main / Main 10 / Main Still Picture の正常系と、各 Err 経路 (chroma_format_idc / bit_depth /
    // sps_max_sub_layers_minus1 / general_profile_idc / cropping アンダーフロー等) を
    // 確実に踏ませるためのビルダー。Hisui の入力前提は単一レイヤー (sps_max_sub_layers_minus1 == 0)。
    // ----------------------------------------------------------------

    /// ビット単位で値を書き出すライター（仕様 9.x の逆操作）
    struct HevcSpsBitWriter {
        bytes: Vec<u8>,
        bit_count: usize,
    }

    impl HevcSpsBitWriter {
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
        fn write_u(&mut self, n: usize, value: u64) {
            for i in (0..n).rev() {
                let bit = ((value >> i) & 1) as u8;
                self.write_bit(bit);
            }
        }

        /// 符号なし Exp-Golomb 復号の逆操作（ue(v)、仕様 9.x）
        fn write_ue(&mut self, value: u32) {
            // value + 1 の bit 表現長を取り、(長さ - 1) 個の先行 0 + value+1 の bit 表現を書く
            let v = value
                .checked_add(1)
                .expect("ue(v) のテスト入力が u32::MAX を越えた");
            let bits_needed = 32 - v.leading_zeros() as usize;
            for _ in 0..(bits_needed - 1) {
                self.write_bit(0);
            }
            self.write_u(bits_needed, v as u64);
        }

        fn into_bytes(self) -> Vec<u8> {
            self.bytes
        }
    }

    /// テスト用の H.265 SPS バイト列ビルダー
    ///
    /// デフォルト: Main プロファイル + sps_max_sub_layers_minus1=0 (Single layer) +
    /// 4:2:0 + 8-bit + 解像度引数指定 + conformance window 無し。
    pub(crate) struct HevcSpsBuilder {
        general_profile_space: u32,
        general_tier_flag: u32,
        general_profile_idc: u32,
        general_profile_compatibility_flags: u32,
        general_constraint_indicator_flags: u64,
        general_level_idc: u32,
        sps_max_sub_layers_minus1: u32,
        sps_temporal_id_nesting_flag: u32,
        chroma_format_idc: u32,
        bit_depth_luma_minus8: u32,
        bit_depth_chroma_minus8: u32,
        pic_width_in_luma_samples: u32,
        pic_height_in_luma_samples: u32,
        conformance_window_flag: bool,
        conf_win_offsets: (u32, u32, u32, u32),
    }

    impl HevcSpsBuilder {
        /// 解像度を直接指定するベースビルダー。デフォルトは Main + Level 3.1 + 4:2:0 +
        /// 8-bit + Single layer + crop なし。
        pub(crate) fn raw(pic_width: u32, pic_height: u32) -> Self {
            Self {
                general_profile_space: 0,
                general_tier_flag: 0,
                general_profile_idc: 1, // Main
                general_profile_compatibility_flags: 0x60000000,
                general_constraint_indicator_flags: 0xb00000000000,
                general_level_idc: 93, // Level 3.1
                sps_max_sub_layers_minus1: 0,
                sps_temporal_id_nesting_flag: 1,
                chroma_format_idc: 1, // 4:2:0
                bit_depth_luma_minus8: 0,
                bit_depth_chroma_minus8: 0,
                pic_width_in_luma_samples: pic_width,
                pic_height_in_luma_samples: pic_height,
                conformance_window_flag: false,
                conf_win_offsets: (0, 0, 0, 0),
            }
        }

        pub(crate) fn with_general_profile_idc(mut self, v: u32) -> Self {
            self.general_profile_idc = v;
            self
        }

        pub(crate) fn with_general_level_idc(mut self, v: u32) -> Self {
            self.general_level_idc = v;
            self
        }

        pub(crate) fn with_general_profile_compatibility_flags(mut self, v: u32) -> Self {
            self.general_profile_compatibility_flags = v;
            self
        }

        pub(crate) fn with_general_constraint_indicator_flags(mut self, v: u64) -> Self {
            self.general_constraint_indicator_flags = v;
            self
        }

        pub(crate) fn with_general_profile_space(mut self, v: u32) -> Self {
            self.general_profile_space = v;
            self
        }

        pub(crate) fn with_general_tier_flag(mut self, v: u32) -> Self {
            self.general_tier_flag = v;
            self
        }

        pub(crate) fn with_sps_max_sub_layers_minus1(mut self, v: u32) -> Self {
            self.sps_max_sub_layers_minus1 = v;
            self
        }

        pub(crate) fn with_chroma_format_idc(mut self, v: u32) -> Self {
            self.chroma_format_idc = v;
            self
        }

        pub(crate) fn with_bit_depth_luma_minus8(mut self, v: u32) -> Self {
            self.bit_depth_luma_minus8 = v;
            self
        }

        pub(crate) fn with_bit_depth_chroma_minus8(mut self, v: u32) -> Self {
            self.bit_depth_chroma_minus8 = v;
            self
        }

        pub(crate) fn with_conformance_window(
            mut self,
            left: u32,
            right: u32,
            top: u32,
            bottom: u32,
        ) -> Self {
            self.conformance_window_flag = true;
            self.conf_win_offsets = (left, right, top, bottom);
            self
        }

        pub(crate) fn build(self) -> Vec<u8> {
            let mut w = HevcSpsBitWriter::new();

            // NAL ヘッダ 2 バイト (SPS_HEADER 定数経由で本モジュール内の VPS/SPS/PPS 定数と DRY を保つ)
            for &b in &SPS_HEADER {
                w.write_u(8, u64::from(b));
            }

            // sps_video_parameter_set_id (u(4))
            w.write_u(4, 0);
            // sps_max_sub_layers_minus1 (u(3))
            w.write_u(3, self.sps_max_sub_layers_minus1 as u64);
            // sps_temporal_id_nesting_flag (u(1))
            w.write_u(1, self.sps_temporal_id_nesting_flag as u64);

            // profile_tier_level (1, sps_max_sub_layers_minus1)
            w.write_u(2, self.general_profile_space as u64);
            w.write_u(1, self.general_tier_flag as u64);
            w.write_u(5, self.general_profile_idc as u64);
            w.write_u(32, self.general_profile_compatibility_flags as u64);
            // 48 bit の constraint indicator flags
            w.write_u(
                32,
                (self.general_constraint_indicator_flags >> 16) & 0xFFFFFFFF,
            );
            w.write_u(16, self.general_constraint_indicator_flags & 0xFFFF);
            w.write_u(8, self.general_level_idc as u64);

            // sps_max_sub_layers_minus1 == 0 のとき sub_layer present flag ループと reserved_zero_2bits は無し
            // sps_max_sub_layers_minus1 > 0 のときは sub_layer_*_present_flag を 0 / reserved_zero_2bits を埋める
            for _ in 0..self.sps_max_sub_layers_minus1 {
                w.write_u(1, 0); // sub_layer_profile_present_flag[i] = 0
                w.write_u(1, 0); // sub_layer_level_present_flag[i] = 0
            }
            if self.sps_max_sub_layers_minus1 > 0 {
                let reserved_bits = (8 - self.sps_max_sub_layers_minus1 as usize) * 2;
                for _ in 0..reserved_bits {
                    w.write_u(1, 0);
                }
            }

            // sps_seq_parameter_set_id (ue(v))
            w.write_ue(0);

            // chroma_format_idc (ue(v))
            w.write_ue(self.chroma_format_idc);
            if self.chroma_format_idc == 3 {
                // separate_colour_plane_flag (u(1)) = 0
                w.write_u(1, 0);
            }

            // pic_width_in_luma_samples / pic_height_in_luma_samples (ue(v))
            w.write_ue(self.pic_width_in_luma_samples);
            w.write_ue(self.pic_height_in_luma_samples);

            // conformance_window_flag (u(1))
            w.write_u(1, if self.conformance_window_flag { 1 } else { 0 });
            if self.conformance_window_flag {
                w.write_ue(self.conf_win_offsets.0);
                w.write_ue(self.conf_win_offsets.1);
                w.write_ue(self.conf_win_offsets.2);
                w.write_ue(self.conf_win_offsets.3);
            }

            // bit_depth_luma_minus8 / bit_depth_chroma_minus8 (ue(v))
            w.write_ue(self.bit_depth_luma_minus8);
            w.write_ue(self.bit_depth_chroma_minus8);

            // 残りのフィールド (log2_max_pic_order_cnt_lsb_minus4 等) は本実装では読まないため省略する。
            // RBSP trailing bits も省略する。

            w.into_bytes()
        }
    }

    #[test]
    fn parse_hevc_sps_main_profile_returns_field_values() {
        // Main プロファイル (general_profile_idc=1) のデフォルトビルダーで profile / level /
        // compat / constraint / chroma / bit_depth が SPS 由来の値で返ること
        let sps = HevcSpsBuilder::raw(1920, 1080).build();
        let params = parse_hevc_sps(&sps).expect("Main SPS のパース成功");
        assert_eq!(params.general_profile_idc, 1);
        assert_eq!(params.general_level_idc, 93);
        assert_eq!(params.general_profile_space, 0);
        assert_eq!(params.general_tier_flag, 0);
        assert_eq!(params.general_profile_compatibility_flags, 0x60000000);
        assert_eq!(params.general_constraint_indicator_flags, 0xb00000000000);
        assert_eq!(params.chroma_format_idc, 1);
        assert_eq!(params.bit_depth_luma_minus8, 0);
        assert_eq!(params.bit_depth_chroma_minus8, 0);
        assert_eq!(params.sps_max_sub_layers_minus1, 0);
        assert_eq!(params.sps_temporal_id_nesting_flag, 1);
        assert_eq!(params.width, 1920);
        assert_eq!(params.height, 1080);
    }

    #[test]
    fn parse_hevc_sps_main10_profile_returns_bit_depth_2() {
        // Main 10 プロファイル (general_profile_idc=2 + bit_depth_*_minus8=2) を反映できること
        let sps = HevcSpsBuilder::raw(1920, 1080)
            .with_general_profile_idc(2)
            .with_bit_depth_luma_minus8(2)
            .with_bit_depth_chroma_minus8(2)
            .build();
        let params = parse_hevc_sps(&sps).expect("Main 10 SPS のパース成功");
        assert_eq!(params.general_profile_idc, 2);
        assert_eq!(params.bit_depth_luma_minus8, 2);
        assert_eq!(params.bit_depth_chroma_minus8, 2);
    }

    #[test]
    fn parse_hevc_sps_applies_conformance_window_cropping() {
        // conformance_window_flag=1 + crop_right=8 + crop_bottom=8 の場合、
        // 4:2:0 ストリームでは SubWidthC=2, SubHeightC=2 で
        // width = 1920 - 2*(0+8) = 1904 / height = 1080 - 2*(0+8) = 1064 になること
        let sps = HevcSpsBuilder::raw(1920, 1080)
            .with_conformance_window(0, 8, 0, 8)
            .build();
        let params = parse_hevc_sps(&sps).expect("crop SPS のパース成功");
        assert_eq!(params.width, 1904);
        assert_eq!(params.height, 1064);
    }

    #[test]
    fn parse_hevc_sps_rejects_unsupported_profile_idc() {
        // general_profile_idc = 10 (H265_ALLOWED_PROFILE_IDCS にも入っていない値) で Err
        let sps = HevcSpsBuilder::raw(1920, 1080)
            .with_general_profile_idc(10)
            .build();
        let result = parse_hevc_sps(&sps);
        assert!(
            result.is_err(),
            "許容リスト外の general_profile_idc は Err: {result:?}"
        );
    }

    #[test]
    fn parse_hevc_sps_rejects_profile_idc_in_unsupported_hole() {
        // 許容リスト {1, 2, 3, 4, 5, 6, 7, 9} の穴 (= 8) で Err になること。
        // 単なる「上限超過」ではなく「許容リストの穴」を検出することの担保。
        let sps = HevcSpsBuilder::raw(1920, 1080)
            .with_general_profile_idc(8)
            .build();
        let result = parse_hevc_sps(&sps);
        assert!(
            result.is_err(),
            "許容リストの穴 (profile_idc=8) は Err: {result:?}"
        );
    }

    #[test]
    fn parse_hevc_sps_accepts_each_allowed_profile_idc() {
        // 許容リスト {1, 2, 3, 4, 5, 6, 7, 9} の全 8 値が parse 成功し、
        // 取り出された profile_idc が入力値と一致すること。
        // 許容リストを誤って一部だけ受理する実装ミスの回帰を防ぐ。
        for &profile in &[1u32, 2, 3, 4, 5, 6, 7, 9] {
            let sps = HevcSpsBuilder::raw(1920, 1080)
                .with_general_profile_idc(profile)
                .build();
            let params = parse_hevc_sps(&sps).unwrap_or_else(|e| {
                panic!("profile_idc={profile} は許容リスト内なので Ok を返すべき: {e:?}")
            });
            assert_eq!(params.general_profile_idc, profile as u8);
        }
    }

    #[test]
    fn parse_hevc_sps_rejects_chroma_format_idc_out_of_range() {
        // chroma_format_idc = 4 (仕様 7.4.3.2.1 で 0..=3) は Err
        let sps = HevcSpsBuilder::raw(1920, 1080)
            .with_chroma_format_idc(4)
            .build();
        let result = parse_hevc_sps(&sps);
        assert!(
            result.is_err(),
            "chroma_format_idc=4 は仕様値域外で Err: {result:?}"
        );
    }

    #[test]
    fn parse_hevc_sps_rejects_bit_depth_luma_minus8_out_of_range() {
        // bit_depth_luma_minus8 = 8 (HvccBox の Uint<u8, 3> 制約 0..=7 を超える) は Err
        let sps = HevcSpsBuilder::raw(1920, 1080)
            .with_bit_depth_luma_minus8(8)
            .build();
        let result = parse_hevc_sps(&sps);
        assert!(
            result.is_err(),
            "bit_depth_luma_minus8=8 は HvccBox 値域外で Err: {result:?}"
        );
    }

    #[test]
    fn parse_hevc_sps_rejects_bit_depth_chroma_minus8_out_of_range() {
        // bit_depth_chroma_minus8 = 8 (HvccBox の Uint<u8, 3> 制約 0..=7 を超える) は Err
        let sps = HevcSpsBuilder::raw(1920, 1080)
            .with_bit_depth_chroma_minus8(8)
            .build();
        let result = parse_hevc_sps(&sps);
        assert!(
            result.is_err(),
            "bit_depth_chroma_minus8=8 は HvccBox 値域外で Err: {result:?}"
        );
    }

    #[test]
    fn parse_hevc_sps_rejects_sps_max_sub_layers_minus1_out_of_range() {
        // sps_max_sub_layers_minus1 = 7 (仕様 7.4.3.2.1 で 0..=6) は Err
        let sps = HevcSpsBuilder::raw(1920, 1080)
            .with_sps_max_sub_layers_minus1(7)
            .build();
        let result = parse_hevc_sps(&sps);
        assert!(
            result.is_err(),
            "sps_max_sub_layers_minus1=7 は仕様値域外で Err: {result:?}"
        );
    }

    #[test]
    fn parse_hevc_sps_rejects_zero_dimensions_after_cropping() {
        // crop 適用後に width が 0 になるケース。pic_width=16 / SubWidthC=2 / crop_left=4 + crop_right=4
        // → 16 - 2*(4+4) = 0 で Err
        let sps = HevcSpsBuilder::raw(16, 32)
            .with_conformance_window(4, 4, 0, 0)
            .build();
        let result = parse_hevc_sps(&sps);
        assert!(
            result.is_err(),
            "crop 後に width が 0 になる場合は Err: {result:?}"
        );
    }

    #[test]
    fn parse_hevc_sps_rejects_crop_underflow() {
        // crop 値が pic_width を超えるケース。pic_width=16 / SubWidthC=2 / crop_left=100 + crop_right=100
        // → 2*(100+100) = 400 を 16 から引けず Err
        let sps = HevcSpsBuilder::raw(16, 32)
            .with_conformance_window(100, 100, 0, 0)
            .build();
        let result = parse_hevc_sps(&sps);
        assert!(result.is_err(), "crop アンダーフローは Err: {result:?}");
    }

    #[test]
    fn parse_hevc_sps_round_trips_profile_tier_level_fields_from_builder() {
        // HevcSpsBuilder の profile_tier_level 系 with_* メソッドが parse_hevc_sps の戻り値に
        // そのまま反映されることを担保する round-trip テスト。デフォルト値以外を全部設定する。
        let sps = HevcSpsBuilder::raw(1920, 1080)
            .with_general_profile_space(2)
            .with_general_tier_flag(1)
            .with_general_profile_idc(2) // Main 10
            .with_general_profile_compatibility_flags(0x40000000)
            .with_general_constraint_indicator_flags(0x123456789abc)
            .with_general_level_idc(120)
            .build();
        let params = parse_hevc_sps(&sps).expect("round-trip SPS のパース成功");
        assert_eq!(params.general_profile_space, 2);
        assert_eq!(params.general_tier_flag, 1);
        assert_eq!(params.general_profile_idc, 2);
        assert_eq!(params.general_profile_compatibility_flags, 0x40000000);
        assert_eq!(params.general_constraint_indicator_flags, 0x123456789abc);
        assert_eq!(params.general_level_idc, 120);
    }

    #[test]
    fn rbsp_from_hevc_sps_nalu_rejects_non_sps_nal_type() {
        // VPS の NAL ヘッダ (nal_unit_type=32) を渡すと Err になること
        let nalu = [0x40, 0x01, 0x00];
        let result = rbsp_from_hevc_sps_nalu(&nalu);
        assert!(result.is_err(), "SPS 以外の NAL は Err: {result:?}");
    }

    #[test]
    fn rbsp_from_hevc_sps_nalu_rejects_short_input() {
        // 1 バイト入力 (NAL ヘッダ 2 バイトに満たない) は Err
        let nalu = [0x42];
        let result = rbsp_from_hevc_sps_nalu(&nalu);
        assert!(result.is_err(), "短い NAL は Err: {result:?}");
    }

    #[test]
    fn rbsp_from_hevc_sps_nalu_removes_emulation_prevention_bytes() {
        // 0x00 0x00 0x03 パターンを RBSP 抽出時に 0x00 0x00 に縮約すること
        // NAL ヘッダ 2 バイト (0x42 0x01) + payload (0x00 0x00 0x03 0xff) を入れる
        let nalu = [0x42, 0x01, 0x00, 0x00, 0x03, 0xff];
        let rbsp = rbsp_from_hevc_sps_nalu(&nalu).expect("RBSP 抽出成功");
        // payload 4 バイトから emulation prevention byte 1 個分が削れる
        assert_eq!(rbsp.len(), 3);
        assert_eq!(rbsp, vec![0x00, 0x00, 0xff]);
    }

    // ----------------------------------------------------------------
    // h265_sample_entry_from_vps_sps_pps_lists / h265_sample_entry_from_annexb の単体テスト群
    //
    // 新ヘルパー関数の空入力 Err / NAL タイプ全要素検査 Err / HvccBox フィールドの
    // SPS 由来値反映 / 複数 VPS / SPS / PPS の順序保持 / conformance window 適用後の
    // VideoFrameSize / Annex-B 経由の薄いラッパー統合経路を直接検証する。
    // ----------------------------------------------------------------

    /// テスト用のダミー VPS NAL バイト列 (NAL タイプ検査のみ通る最小サイズ)
    fn dummy_vps_nal() -> Vec<u8> {
        VPS_HEADER.to_vec()
    }

    /// テスト用のダミー PPS NAL バイト列 (NAL タイプ検査のみ通る最小サイズ)
    fn dummy_pps_nal() -> Vec<u8> {
        PPS_HEADER.to_vec()
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_returns_err_on_empty_vps_list() {
        // vps_list が空のときは `missing H.265 VPS` Err を返す
        let sps = HevcSpsBuilder::raw(1920, 1080).build();
        let result = h265_sample_entry_from_vps_sps_pps_lists(
            vec![],
            vec![sps],
            vec![dummy_pps_nal()],
            FrameRate::FPS_30,
        );
        let err = result.expect_err("vps_list 空は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("missing H.265 VPS"),
            "エラーメッセージに `missing H.265 VPS` が含まれること (実際: {display})"
        );
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_returns_err_on_empty_sps_list() {
        // sps_list が空のときは `missing H.265 SPS` Err を返す
        let result = h265_sample_entry_from_vps_sps_pps_lists(
            vec![dummy_vps_nal()],
            vec![],
            vec![dummy_pps_nal()],
            FrameRate::FPS_30,
        );
        let err = result.expect_err("sps_list 空は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("missing H.265 SPS"),
            "エラーメッセージに `missing H.265 SPS` が含まれること (実際: {display})"
        );
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_returns_err_on_empty_pps_list() {
        // pps_list が空のときは `missing H.265 PPS` Err を返す
        let sps = HevcSpsBuilder::raw(1920, 1080).build();
        let result = h265_sample_entry_from_vps_sps_pps_lists(
            vec![dummy_vps_nal()],
            vec![sps],
            vec![],
            FrameRate::FPS_30,
        );
        let err = result.expect_err("pps_list 空は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("missing H.265 PPS"),
            "エラーメッセージに `missing H.265 PPS` が含まれること (実際: {display})"
        );
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_returns_err_on_vps_list_with_non_vps_nal_at_index_1()
     {
        // vps_list の index 1 に SPS NAL を混入させた場合に Err を返す (全要素検査の担保)
        let sps = HevcSpsBuilder::raw(1920, 1080).build();
        let non_vps_nal = vec![0x42, 0x01]; // SPS NAL ヘッダ
        let result = h265_sample_entry_from_vps_sps_pps_lists(
            vec![dummy_vps_nal(), non_vps_nal],
            vec![sps],
            vec![dummy_pps_nal()],
            FrameRate::FPS_30,
        );
        let err = result.expect_err("VPS 以外の NAL は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("invalid H.265 VPS"),
            "エラーメッセージに `invalid H.265 VPS` が含まれること (実際: {display})"
        );
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_returns_err_on_sps_list_with_non_sps_nal_at_index_1()
     {
        // sps_list の index 1 に VPS NAL を混入させた場合に Err を返す
        let sps = HevcSpsBuilder::raw(1920, 1080).build();
        let non_sps_nal = vec![0x40, 0x01]; // VPS NAL ヘッダ
        let result = h265_sample_entry_from_vps_sps_pps_lists(
            vec![dummy_vps_nal()],
            vec![sps, non_sps_nal],
            vec![dummy_pps_nal()],
            FrameRate::FPS_30,
        );
        let err = result.expect_err("SPS 以外の NAL は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("invalid H.265 SPS"),
            "エラーメッセージに `invalid H.265 SPS` が含まれること (実際: {display})"
        );
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_returns_err_on_pps_list_with_non_pps_nal_at_index_1()
     {
        // pps_list の index 1 に SPS NAL を混入させた場合に Err を返す
        let sps = HevcSpsBuilder::raw(1920, 1080).build();
        let non_pps_nal = vec![0x42, 0x01]; // SPS NAL ヘッダ
        let result = h265_sample_entry_from_vps_sps_pps_lists(
            vec![dummy_vps_nal()],
            vec![sps],
            vec![dummy_pps_nal(), non_pps_nal],
            FrameRate::FPS_30,
        );
        let err = result.expect_err("PPS 以外の NAL は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("invalid H.265 PPS"),
            "エラーメッセージに `invalid H.265 PPS` が含まれること (実際: {display})"
        );
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_maps_main_sps_to_hvcc() {
        // Main プロファイル + Level 3.1 / Single layer の SPS の各フィールドが HvccBox に
        // 1:1 で反映されることを直接検証する。Sora 録画固定値 (general_level_idc: 123 等) で
        // 埋まる旧挙動の回帰防止。
        let sps = HevcSpsBuilder::raw(1920, 1080).build();
        let (entry, _frame_size) = h265_sample_entry_from_vps_sps_pps_lists(
            vec![dummy_vps_nal()],
            vec![sps],
            vec![dummy_pps_nal()],
            FrameRate::FPS_30,
        )
        .expect("Main SPS のパース成功");
        let SampleEntry::Hvc1(hvc1) = entry else {
            panic!("Hvc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };
        assert_eq!(hvc1.hvcc_box.general_profile_idc.get(), 1);
        assert_eq!(hvc1.hvcc_box.general_level_idc, 93);
        assert_eq!(hvc1.hvcc_box.general_profile_space.get(), 0);
        assert_eq!(hvc1.hvcc_box.general_tier_flag.get(), 0);
        assert_eq!(
            hvc1.hvcc_box.general_profile_compatibility_flags,
            0x60000000
        );
        assert_eq!(
            hvc1.hvcc_box.general_constraint_indicator_flags.get(),
            0xb00000000000
        );
        assert_eq!(hvc1.hvcc_box.chroma_format_idc.get(), 1);
        assert_eq!(hvc1.hvcc_box.bit_depth_luma_minus8.get(), 0);
        assert_eq!(hvc1.hvcc_box.bit_depth_chroma_minus8.get(), 0);
        // Single layer (sps_max_sub_layers_minus1=0 → num_temporal_layers=1)
        assert_eq!(hvc1.hvcc_box.num_temporal_layers.get(), 1);
        // sps_temporal_id_nesting_flag のデフォルト 1 を反映
        assert_eq!(hvc1.hvcc_box.temporal_id_nested.get(), 1);
        // visual.width / .height が SPS 由来実値
        assert_eq!(hvc1.visual.width, 1920);
        assert_eq!(hvc1.visual.height, 1080);
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_maps_main10_sps_to_hvcc() {
        // Main 10 プロファイル (general_profile_idc=2 + bit_depth_*_minus8=2) の SPS が
        // HvccBox の bit_depth_*_minus8 に 2 として反映されることを検証する。
        // luma と chroma に異なる値を入れてフィールド取り違えを検出可能にする。
        let sps = HevcSpsBuilder::raw(1920, 1080)
            .with_general_profile_idc(2)
            .with_bit_depth_luma_minus8(2)
            .with_bit_depth_chroma_minus8(4)
            .build();
        let (entry, _frame_size) = h265_sample_entry_from_vps_sps_pps_lists(
            vec![dummy_vps_nal()],
            vec![sps],
            vec![dummy_pps_nal()],
            FrameRate::FPS_30,
        )
        .expect("Main 10 SPS のパース成功");
        let SampleEntry::Hvc1(hvc1) = entry else {
            panic!("Hvc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };
        assert_eq!(hvc1.hvcc_box.general_profile_idc.get(), 2);
        assert_eq!(hvc1.hvcc_box.bit_depth_luma_minus8.get(), 2);
        assert_eq!(hvc1.hvcc_box.bit_depth_chroma_minus8.get(), 4);
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_preserves_all_vps_sps_pps_in_nalu_arrays() {
        // vps_list / sps_list / pps_list に複数 NAL を渡したとき、HvccBox.nalu_arrays の
        // VPS / SPS / PPS スロットに全 NAL がそのままの順序で move されることを直接検証する。
        // 先頭 SPS のパラメータのみが hvcC フィールドに反映される (Hisui の入力前提) ことの確認も兼ねる。
        let sps_a = HevcSpsBuilder::raw(1920, 1080).build();
        let sps_b = HevcSpsBuilder::raw(320, 240).build();
        let vps_a = dummy_vps_nal();
        let vps_b = vec![0x40, 0x02]; // 別の VPS NAL ヘッダ (nuh_temporal_id_plus1=2 違い)
        let pps_a = dummy_pps_nal();
        let pps_b = vec![0x44, 0x02];

        let (entry, _frame_size) = h265_sample_entry_from_vps_sps_pps_lists(
            vec![vps_a.clone(), vps_b.clone()],
            vec![sps_a.clone(), sps_b.clone()],
            vec![pps_a.clone(), pps_b.clone()],
            FrameRate::FPS_30,
        )
        .expect("複数 VPS / SPS / PPS でもパース成功");
        let SampleEntry::Hvc1(hvc1) = entry else {
            panic!("Hvc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };
        // nalu_arrays は VPS / SPS / PPS の 3 配列をこの順で持つ
        assert_eq!(hvc1.hvcc_box.nalu_arrays.len(), 3);
        assert_eq!(hvc1.hvcc_box.nalu_arrays[0].nalus, vec![vps_a, vps_b]);
        assert_eq!(hvc1.hvcc_box.nalu_arrays[1].nalus, vec![sps_a, sps_b]);
        assert_eq!(hvc1.hvcc_box.nalu_arrays[2].nalus, vec![pps_a, pps_b]);
        // 先頭 SPS (1920x1080) のパラメータが反映されている
        assert_eq!(hvc1.visual.width, 1920);
        assert_eq!(hvc1.visual.height, 1080);
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_returns_frame_size_from_cropping() {
        // 戻り値タプルの第 2 要素 `VideoFrameSize` が SPS の conformance window 適用後の
        // 解像度と一致し、`Hvc1Box.visual.width / .height` にも同じ値が反映されることを
        // 直接検証する。1920x1088 raw + crop_bottom=4 で 1920x1080 になる典型パターン
        // (SubHeightC=2 で 2 * (0 + 4) = 8 削って 1080)。
        let sps = HevcSpsBuilder::raw(1920, 1088)
            .with_conformance_window(0, 0, 0, 4)
            .build();
        let (entry, frame_size) = h265_sample_entry_from_vps_sps_pps_lists(
            vec![dummy_vps_nal()],
            vec![sps],
            vec![dummy_pps_nal()],
            FrameRate::FPS_30,
        )
        .expect("crop SPS のパース成功");
        assert_eq!(frame_size.width, 1920);
        assert_eq!(frame_size.height, 1080);
        let SampleEntry::Hvc1(hvc1) = entry else {
            panic!("Hvc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };
        assert_eq!(hvc1.visual.width, 1920);
        assert_eq!(hvc1.visual.height, 1080);
    }

    #[test]
    fn parse_hevc_sps_parses_real_x265_640x480_sps() {
        // x265 が出力する実機 640x480 SPS を parse_hevc_sps に通し、
        // profile_tier_level / chroma / bit_depth / 解像度が仕様準拠の実値で取り出せること、
        // および 5 箇所の emulation prevention byte が `rbsp_from_hevc_sps_nalu` で
        // 正しく除去されてビット位置を進められることを担保する。
        // x265 の出力ログでは "Main profile, Level-3 (Main tier)" なので
        // general_profile_idc=1 / general_level_idc=90 / general_tier_flag=0 を期待する。
        let params = parse_hevc_sps(&HEVC_SPS_640X480).expect("実機 640x480 SPS のパース成功");
        assert_eq!(params.general_profile_idc, 1, "Main プロファイル");
        assert_eq!(
            params.general_level_idc, 90,
            "Level 3.0 の level_idc は仕様 Annex A で 90"
        );
        assert_eq!(params.general_tier_flag, 0, "Main tier");
        assert_eq!(params.general_profile_space, 0);
        assert_eq!(params.chroma_format_idc, 1, "yuv420p で 4:2:0");
        assert_eq!(params.bit_depth_luma_minus8, 0, "8-bit");
        assert_eq!(params.bit_depth_chroma_minus8, 0);
        assert_eq!(params.sps_max_sub_layers_minus1, 0, "Single layer");
        assert_eq!(params.width, 640);
        assert_eq!(params.height, 480);
    }

    #[test]
    fn parse_hevc_sps_parses_real_x265_1920x1080_sps_with_conformance_window() {
        // x265 が出力する実機 1920x1080 SPS を parse_hevc_sps に通し、
        // raw 1920x1088 + conformance window 適用後の 1920x1080 として正しく解釈されることを
        // 担保する。x265 の出力ログでは "Main profile, Level-4 (Main tier)" なので
        // general_level_idc=120 を期待する。
        let params = parse_hevc_sps(&HEVC_SPS_1920X1080).expect("実機 1920x1080 SPS のパース成功");
        assert_eq!(params.general_profile_idc, 1);
        assert_eq!(
            params.general_level_idc, 120,
            "Level 4.0 の level_idc は仕様 Annex A で 120"
        );
        assert_eq!(params.chroma_format_idc, 1);
        assert_eq!(params.bit_depth_luma_minus8, 0);
        assert_eq!(params.bit_depth_chroma_minus8, 0);
        assert_eq!(
            params.width, 1920,
            "raw 1920 から conformance window 適用後も 1920"
        );
        assert_eq!(
            params.height, 1080,
            "raw 1088 から conformance window crop_bottom で 1080"
        );
    }

    #[test]
    fn h265_sample_entry_from_vps_sps_pps_lists_with_real_x265_1920x1080_sps_maps_to_hvcc() {
        // 実機 1920x1080 SPS を `h265_sample_entry_from_vps_sps_pps_lists` 経由で渡し、
        // emulation prevention byte 込みの SPS から HvccBox の各フィールドが
        // SPS 由来実値で埋まることの結合担保を行う。Sora 録画固定値
        // (general_level_idc=123 等) で埋まる旧挙動の回帰防止。
        let (entry, frame_size) = h265_sample_entry_from_vps_sps_pps_lists(
            vec![dummy_vps_nal()],
            vec![HEVC_SPS_1920X1080.to_vec()],
            vec![dummy_pps_nal()],
            FrameRate::FPS_30,
        )
        .expect("実機 SPS で Hvc1 SampleEntry を構築できること");
        assert_eq!(frame_size.width, 1920);
        assert_eq!(frame_size.height, 1080);
        let SampleEntry::Hvc1(hvc1) = entry else {
            panic!("Hvc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };
        assert_eq!(hvc1.hvcc_box.general_profile_idc.get(), 1);
        assert_eq!(hvc1.hvcc_box.general_level_idc, 120);
        assert_eq!(hvc1.hvcc_box.chroma_format_idc.get(), 1);
        assert_eq!(hvc1.hvcc_box.bit_depth_luma_minus8.get(), 0);
        assert_eq!(hvc1.hvcc_box.bit_depth_chroma_minus8.get(), 0);
        assert_eq!(hvc1.hvcc_box.num_temporal_layers.get(), 1);
        assert_eq!(hvc1.visual.width, 1920);
        assert_eq!(hvc1.visual.height, 1080);
    }

    #[test]
    fn h265_sample_entry_from_annexb_builds_hvc1_sample_entry_from_concatenated_annexb() {
        // VPS / SPS / PPS を 4 バイト start code で連結した Annex-B バイト列を
        // `h265_sample_entry_from_annexb` に渡し、薄いラッパーが
        // `H265AnnexBNalUnits` 走査 → 新ヘルパー呼び出しを経て Hvc1 SampleEntry を構築できる
        // ことを検証する。HvccBox.nalu_arrays に VPS / SPS / PPS が正しく詰まることも担保。
        let sps = HevcSpsBuilder::raw(1920, 1080).build();
        let vps = dummy_vps_nal();
        let pps = dummy_pps_nal();

        let mut annexb = Vec::new();
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(&vps);
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(&sps);
        annexb.extend_from_slice(&[0, 0, 0, 1]);
        annexb.extend_from_slice(&pps);

        let entry = h265_sample_entry_from_annexb(&annexb, FrameRate::FPS_30)
            .expect("Annex-B 経由で Hvc1 SampleEntry を構築できること");
        let SampleEntry::Hvc1(hvc1) = entry else {
            panic!("Hvc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };
        // 解像度が SPS 由来実値で反映される
        assert_eq!(hvc1.visual.width, 1920);
        assert_eq!(hvc1.visual.height, 1080);
        // VPS / SPS / PPS が nalu_arrays に詰まる
        assert_eq!(hvc1.hvcc_box.nalu_arrays[0].nalus, vec![vps]);
        assert_eq!(hvc1.hvcc_box.nalu_arrays[1].nalus, vec![sps]);
        assert_eq!(hvc1.hvcc_box.nalu_arrays[2].nalus, vec![pps]);
    }

    #[test]
    fn chroma_subsampling_factors_returns_subwidthc_subheightc_per_chroma_format_idc() {
        // 仕様 6.2 / Table 6-1 の全 chroma_format_idc 値 (0..=3) と
        // separate_colour_plane_flag=1 の特例で (SubWidthC, SubHeightC) が
        // 正しく返ることを直接検証する。Table 6-1 マッピングの誤実装を防ぐ。
        assert_eq!(chroma_subsampling_factors(0, 0), (1, 1), "monochrome");
        assert_eq!(chroma_subsampling_factors(1, 0), (2, 2), "4:2:0");
        assert_eq!(chroma_subsampling_factors(2, 0), (2, 1), "4:2:2");
        assert_eq!(chroma_subsampling_factors(3, 0), (1, 1), "4:4:4");
        // chroma_format_idc=3 + separate_colour_plane_flag=1 は ChromaArrayType=0 として扱う
        assert_eq!(
            chroma_subsampling_factors(3, 1),
            (1, 1),
            "separate_colour_plane_flag=1 で ChromaArrayType=0 扱い"
        );
    }
}
