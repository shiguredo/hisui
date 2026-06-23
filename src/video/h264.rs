use shiguredo_mp4::{
    Uint,
    boxes::{Avc1Box, AvccBox, SampleEntry},
};

use crate::video::{self, VideoFrameSize, bit_reader::BitReader};

// H.264 の NAL ユニット前に付与されるサイズのバイト数
// Sora / Hisui が生成するものは全て 4 バイトなので固定値でいい
pub const NALU_HEADER_LENGTH: usize = 4;

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

        let i = find_next_annexb_start_code(self.data).unwrap_or(self.data.len());
        let data = &self.data[..i];
        self.data = &self.data[i..];
        Ok(Some(H264NalUnit {
            ty: nal_unit_type,
            data,
        }))
    }
}

/// Annex-B 形式の `data` から次の start code (3 バイト `[0, 0, 1]` または
/// 4 バイト `[0, 0, 0, 1]`) の開始位置を返す。
///
/// `windows(4)` ベースの探索だと末尾 3 バイトが `[0, 0, 1]` で終わる場合に検出漏れが
/// 発生する (`windows(4)` は `data.len() - 3` 個の window しか生成しないため、末尾の
/// 3-byte start code を評価する 4-byte window が存在しない) ので、手書きループで
/// `[0, 0, 1]` を探し、直前バイトが 0 なら 4-byte start code として境界を 1 つ手前に
/// する形にする。
pub(crate) fn find_next_annexb_start_code(data: &[u8]) -> Option<usize> {
    let mut j = 0;
    while j + 3 <= data.len() {
        if data[j] == 0 && data[j + 1] == 0 && data[j + 2] == 1 {
            if j > 0 && data[j - 1] == 0 {
                return Some(j - 1);
            }
            return Some(j);
        }
        j += 1;
    }
    None
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

/// SPS / PPS リストから AVC1 サンプルエントリーと cropping 適用後の解像度を構築する
///
/// 入力 `sps_list[i]` / `pps_list[j]` は ISO/IEC 14496-15 §5.3.3.1 で定義された EBSP 形式
/// (emulation prevention byte 込み、NAL ヘッダ 1 バイト含む、start code は含まない)。
/// `H264AnnexBNalUnits::next` が返す `H264NalUnit.data` の形式と `AvccBox.sps_list / pps_list`
/// の格納形式に揃える。emulation prevention byte の除去 (RBSP 抽出) は `parse_sps` 内の
/// `rbsp_from_sps_nalu` でのみ実施し、本関数の入力契約には除去しない。
///
/// `pps_list[i]` の先頭バイトに NAL タイプ検査 (`& 0x1F == H264_NALU_TYPE_PPS`) を実施する。
/// 呼び出し側の事前判定漏れで SPS や IDR が誤って `pps_list` に混入した場合は Err を返す。
///
/// 内部で `parse_sps(sps_list[0])` を 1 回呼んで SPS パラメータを取り出し、avcC フィールドに
/// 反映する。複数 SPS は先頭 SPS のパラメータのみを採用し、`AvccBox.sps_list` には全 SPS を
/// move する (Hisui の入力前提では複数 SPS は同一内容を想定)。
///
/// 戻り値タプルの `VideoFrameSize` は SPS 由来の cropping 適用後解像度。`parse_sps` 内で
/// width / height > 0 / u16::MAX 上限を保証しているため `VideoFrameSize::new` は infallible。
/// 呼び出し側で `VideoFrame.size` を encoder 設定値などから別途構築する場合は、
/// タプルの第 2 要素 `VideoFrameSize` を `_` で捨ててよい (encoder 経路の既存挙動)。
pub fn h264_sample_entry_from_sps_pps_lists(
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
) -> crate::Result<(SampleEntry, VideoFrameSize)> {
    if sps_list.is_empty() {
        return Err(crate::Error::new("missing H.264 SPS"));
    }
    if pps_list.is_empty() {
        return Err(crate::Error::new("missing H.264 PPS"));
    }

    // pps_list の各要素が PPS NAL であることを検査する。SPS は parse_sps 内で検査される。
    // 防御的検査で、呼び出し側 (SRT inbound / RTSP / encoder 3 経路) の事前判定漏れで
    // SPS や IDR が混入した場合に AvccBox.pps_list へ壊れた NAL が move されるのを防ぐ。
    for (i, pps) in pps_list.iter().enumerate() {
        if pps.is_empty() {
            return Err(crate::Error::new(format!(
                "invalid H.264 PPS at index {i}: empty NAL"
            )));
        }
        let nal_unit_type = pps[0] & 0x1F;
        if nal_unit_type != H264_NALU_TYPE_PPS {
            return Err(crate::Error::new(format!(
                "invalid H.264 PPS at index {i}: expected nal_unit_type={H264_NALU_TYPE_PPS}, got {nal_unit_type}"
            )));
        }
    }

    let params = parse_sps(sps_list[0].as_slice())?;

    let frame_size = VideoFrameSize::new(params.width as usize, params.height as usize)
        .expect("infallible: parse_sps validates u16::MAX upper bound and positive width / height");

    let entry = SampleEntry::Avc1(Avc1Box {
        visual: video::sample_entry_visual_fields(params.width as usize, params.height as usize),
        avcc_box: AvccBox {
            avc_profile_indication: params.profile_idc,
            profile_compatibility: params.constraint_set_flags,
            avc_level_indication: params.level_idc,
            chroma_format: params
                .high_profile_params
                .as_ref()
                .map(|h| Uint::new(h.chroma_format_idc)),
            bit_depth_luma_minus8: params
                .high_profile_params
                .as_ref()
                .map(|h| Uint::new(h.bit_depth_luma_minus8)),
            bit_depth_chroma_minus8: params
                .high_profile_params
                .as_ref()
                .map(|h| Uint::new(h.bit_depth_chroma_minus8)),
            length_size_minus_one: Uint::new(NALU_HEADER_LENGTH as u8 - 1),
            sps_ext_list: Vec::new(),
            sps_list,
            pps_list,
        },
        unknown_boxes: Vec::new(),
    });

    Ok((entry, frame_size))
}

/// Annex-B バイト列から AVC1 サンプルエントリーを構築する薄いラッパー
///
/// 内部で `H264AnnexBNalUnits` を 1 回走査して SPS / PPS NAL のみを抽出し、
/// `h264_sample_entry_from_sps_pps_lists` を呼ぶ。SEI / IDR / Filler 等の NAL タイプは無視する。
///
/// 引数として `width` / `height` は受け取らない (SPS 由来の実値を avcC と visual に反映するため)。
pub fn h264_sample_entry_from_annexb(data: &[u8]) -> crate::Result<SampleEntry> {
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
    let (entry, _frame_size) = h264_sample_entry_from_sps_pps_lists(sps_list, pps_list)?;
    Ok(entry)
}

/// WebM CodecPrivate の AVCDecoderConfigurationRecord (avcC) から SPS / PPS リストを抽出する。
///
/// ISO/IEC 14496-15 の AVCDecoderConfigurationRecord に基づき、avcC バイト列内の SPS / PPS
/// NAL バイト列 (NAL ヘッダ 1 バイト含む、start code なし) を `(Vec<Vec<u8>>, Vec<Vec<u8>>)`
/// で返す。各リストは avcC 内の出現順を保持する (`h264_sample_entry_from_sps_pps_lists` が
/// `parse_sps(sps_list[0])` で先頭 SPS のパラメータを採用するため順序保証が必須)。
///
/// 戻り値の SPS / PPS リストはそのまま `h264_sample_entry_from_sps_pps_lists` の入力契約
/// (EBSP 形式、NAL ヘッダ 1 バイト含む) と一致するため、中間変換なしで move できる。
/// NAL タイプ検査は本関数では行わない。後段の `h264_sample_entry_from_sps_pps_lists` 内で
/// `sps_list[0]` の SPS と全 PPS の NAL タイプを検査する。`sps_list[1..]` の NAL タイプは
/// 検査されないため、avcC が壊れていて先頭以外の SPS スロットに非 SPS NAL が混ざっていた
/// 場合はそのまま `AvccBox.sps_list` に move される (Hisui の入力前提では発生しない異常系)。
///
/// byte 1..=3 (`AVCProfileIndication` / `profile_compatibility` / `AVCLevelIndication`) と
/// High 系プロファイル時の末尾追加フィールドは捨てる (`parse_sps` が SPS 由来実値で
/// `AvccBox` を埋めるため avcC ヘッダ値は不要)。
#[allow(clippy::type_complexity)]
pub fn parse_avcc_sps_pps_lists(data: &[u8]) -> crate::Result<(Vec<Vec<u8>>, Vec<Vec<u8>>)> {
    // 固定ヘッダの最小サイズは byte 0..=5 の 6 バイト
    if data.len() < 6 {
        return Err(crate::Error::new(format!(
            "invalid H.264 avcC: too short (expected >= 6 bytes, got {})",
            data.len()
        )));
    }
    if data[0] != 1 {
        return Err(crate::Error::new(format!(
            "invalid H.264 avcC: unsupported configurationVersion {}",
            data[0]
        )));
    }
    // Hisui の MP4 出力は NALU_HEADER_LENGTH = 4 固定で、`AvccBox.length_size_minus_one`
    // との乖離があると下流 muxer 出力後にプレイヤーが NAL を切り出せない。
    // 上位 6 bit (reserved) はマスクで捨てる。
    let length_size_minus_one = data[4] & 0b0000_0011;
    if length_size_minus_one != 3 {
        return Err(crate::Error::new(format!(
            "invalid H.264 avcC: unsupported lengthSizeMinusOne {length_size_minus_one} (expected 3)"
        )));
    }
    // 上位 3 bit (reserved) はマスクで捨てる。下位 5 bit のため上限 31 は構造的に保証される。
    let num_sps = (data[5] & 0b0001_1111) as usize;
    if num_sps == 0 {
        return Err(crate::Error::new(
            "invalid H.264 avcC: numOfSequenceParameterSets == 0",
        ));
    }
    // byte 6 以降を逐次パースする。残バイト不足はすべて同じメッセージで返す。
    let mut offset: usize = 6;
    let mut sps_list: Vec<Vec<u8>> = Vec::new();
    for _ in 0..num_sps {
        let nal_bytes = read_avcc_nal(data, &mut offset)?;
        sps_list.push(nal_bytes);
    }
    // numOfPictureParameterSets (8 bit、最大 255)
    if offset + 1 > data.len() {
        return Err(crate::Error::new(
            "invalid H.264 avcC: SPS/PPS length exceeds remaining data",
        ));
    }
    let num_pps = data[offset] as usize;
    offset += 1;
    if num_pps == 0 {
        return Err(crate::Error::new(
            "invalid H.264 avcC: numOfPictureParameterSets == 0",
        ));
    }
    // shiguredo_mp4::AvccBox::encode は PPS 31 個までしか encode できないため事前に Err 化する。
    if num_pps > 31 {
        return Err(crate::Error::new(format!(
            "invalid H.264 avcC: numOfPictureParameterSets exceeds 31 (got {num_pps})"
        )));
    }
    let mut pps_list: Vec<Vec<u8>> = Vec::new();
    for _ in 0..num_pps {
        let nal_bytes = read_avcc_nal(data, &mut offset)?;
        pps_list.push(nal_bytes);
    }
    // High 系プロファイル時の末尾追加フィールドは残バイトのまま読み捨て (本関数では未利用)。
    Ok((sps_list, pps_list))
}

/// avcC リスト内の単一 NAL を読む内部ヘルパー: 16 bit BE 長フィールド + NAL バイト列。
/// 残バイト不足の場合はすべて統一メッセージで Err を返す。
fn read_avcc_nal(data: &[u8], offset: &mut usize) -> crate::Result<Vec<u8>> {
    if *offset + 2 > data.len() {
        return Err(crate::Error::new(
            "invalid H.264 avcC: SPS/PPS length exceeds remaining data",
        ));
    }
    let len = u16::from_be_bytes([data[*offset], data[*offset + 1]]) as usize;
    *offset += 2;
    if *offset + len > data.len() {
        return Err(crate::Error::new(
            "invalid H.264 avcC: SPS/PPS length exceeds remaining data",
        ));
    }
    let nal = data[*offset..*offset + len].to_vec();
    *offset += len;
    Ok(nal)
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

/// SPS バイト列から取り出した avcC 反映用フィールド群と解像度
///
/// `parse_sps` の戻り値で、`h264_sample_entry_from_sps_pps_lists` 経由で
/// `AvccBox` の各フィールドにマップされる。
#[derive(Debug)]
struct SpsParams {
    /// avcC の `avc_profile_indication` に詰める SPS 1 バイト目
    profile_idc: u8,
    /// avcC の `profile_compatibility` に詰める SPS 2 バイト目
    /// (constraint_set0..5_flag + reserved_zero_2bits の 1 バイト全体)
    constraint_set_flags: u8,
    /// avcC の `avc_level_indication` に詰める SPS 3 バイト目
    level_idc: u8,
    /// High 系プロファイル時のみ Some。それ以外のプロファイルでは None。
    /// avcC の `chroma_format` / `bit_depth_luma_minus8` / `bit_depth_chroma_minus8` の
    /// Some / None をこの Option の有無で 1 対 1 対応させる。
    high_profile_params: Option<HighProfileSpsParams>,
    /// cropping 適用後の最終解像度。`parse_sps` 内で u16::MAX 上限を保証する。
    width: u16,
    height: u16,
}

/// High 系プロファイル時に SPS から取り出すフィールド群
///
/// 各フィールドの値域は ITU-T H.264 仕様 7.4.2.1.1 に従い `parse_sps` で範囲検証済み。
#[derive(Debug)]
struct HighProfileSpsParams {
    chroma_format_idc: u8,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
}

/// SPS NAL ユニットから profile / level / chroma / bit_depth / 解像度を抽出する内部関数
///
/// 入力 `sps` は `H264AnnexBNalUnits` が返す `H264NalUnit.data` をそのまま渡す形式で、
/// 先頭 1 バイトに NAL ヘッダ（forbidden_zero_bit + nal_ref_idc + nal_unit_type = 7）を含む。
/// 内部で `rbsp_from_sps_nalu` を呼んで NAL タイプ検査と RBSP 抽出を行い、
/// ITU-T H.264 仕様 7.3.2.1.1 / 7.4.2.1.1 に従って Exp-Golomb でフィールドを取り出す。
///
/// `profile_idc` が仕様準拠 publisher のプロファイル群
/// (`{66, 77, 88} ∪ H264_HIGH_PROFILES`) に含まれない値、または High 系プロファイル時に
/// `chroma_format_idc > 3` / `bit_depth_luma_minus8 > 6` / `bit_depth_chroma_minus8 > 6` の
/// 仕様値域外を検出した場合は Err を返す。
fn parse_sps(sps: &[u8]) -> crate::Result<SpsParams> {
    let rbsp = rbsp_from_sps_nalu(sps)?;
    let mut reader = BitReader::new(&rbsp);

    let profile_idc = reader.read_u(8)? as u8;
    let constraint_set_flags = reader.read_u(8)? as u8;
    let level_idc = reader.read_u(8)? as u8;
    reader.skip_ue()?; // seq_parameter_set_id

    // 仕様準拠 publisher のプロファイル群 ({66, 77, 88} ∪ H264_HIGH_PROFILES) 以外を弾く
    let is_high = H264_HIGH_PROFILES.contains(&profile_idc);
    let is_baseline_main_extended = matches!(profile_idc, 66 | 77 | 88);
    if !is_high && !is_baseline_main_extended {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: unsupported profile_idc {profile_idc}"
        )));
    }

    // High 系プロファイルは追加フィールドを SPS から取り出し、それ以外は 4:2:0 デフォルトを使う
    let (chroma_array_type, high_profile_params) = if is_high {
        let (chroma_array_type, params) = read_high_profile_sps_fields(&mut reader)?;
        (chroma_array_type, Some(params))
    } else {
        // Baseline / Main / Extended は chroma_format_idc が SPS に含まれず 4:2:0 がデフォルト
        (1, None)
    };

    reader.skip_ue()?; // log2_max_frame_num_minus4
    skip_pic_order_cnt_type_extras(&mut reader)?;
    let (width, height) = read_dimensions_with_cropping(&mut reader, chroma_array_type)?;

    if width == 0 || height == 0 {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: zero dimensions after cropping (width={width}, height={height})"
        )));
    }

    // u16 上限 (65535) を超えると `sample_entry_visual_fields` の `width as u16 / height as u16`
    // で silent truncation してラップした値や 0 が MP4 sample_entry に埋め込まれるため、
    // ここで上限を強制する。H.264 仕様 Level 6.2 の最大解像度 8192x4320 でも
    // u16 に収まるため、実用範囲を狭めることはない。
    if width > u16::MAX as usize || height > u16::MAX as usize {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: dimensions exceed u16::MAX (width={width}, height={height})"
        )));
    }

    Ok(SpsParams {
        profile_idc,
        constraint_set_flags,
        level_idc,
        high_profile_params,
        width: width as u16,
        height: height as u16,
    })
}

/// SPS NAL ユニットのバイト列から cropping 適用後の width / height を抽出する
///
/// 入力 `sps` は `H264AnnexBNalUnits` が返す `H264NalUnit.data` をそのまま渡す形式で、
/// 先頭 1 バイトに NAL ヘッダ（forbidden_zero_bit + nal_ref_idc + nal_unit_type = 7）を含む。
/// 内部で `parse_sps` を呼ぶ薄いラッパーで、Err 条件は `parse_sps` と同じ
/// (NAL タイプ不一致 / 仕様準拠プロファイル群外の profile_idc / 仕様値域外の chroma_format_idc
/// または bit_depth_*_minus8 / u16 上限超過の解像度 等)。
///
/// 本関数は本番経路からは呼ばれず、`pbt/tests/prop_h264_sps.rs` のクラッシュフリー PBT 専用の
/// 公開 API として残している。本番経路は `h264_sample_entry_from_sps_pps_lists` 経由で
/// `parse_sps` を内部呼び出しする。
pub fn extract_dimensions_from_sps(sps: &[u8]) -> crate::Result<(usize, usize)> {
    parse_sps(sps).map(|p| (p.width as usize, p.height as usize))
}

/// High 系プロファイル時の SPS 追加フィールドを読み取り、`HighProfileSpsParams` と
/// `chroma_array_type` を返す（仕様 7.3.2.1.1 の High 系プロファイル分岐）。
///
/// 各フィールドの値域 (chroma_format_idc は 0..=3、bit_depth_*_minus8 は 0..=6) は
/// 仕様 7.4.2.1.1 を根拠に Err 化する。`seq_scaling_matrix_present_flag` 経路は
/// avcC に反映先がないためビット位置を進めるためにのみ skip する。
fn read_high_profile_sps_fields(
    reader: &mut BitReader<'_>,
) -> crate::Result<(u32, HighProfileSpsParams)> {
    let chroma_format_idc = reader.read_ue()?;
    if chroma_format_idc > 3 {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: chroma_format_idc out of spec range (0..=3): {chroma_format_idc}"
        )));
    }
    let separate_colour_plane_flag = if chroma_format_idc == 3 {
        reader.read_u(1)?
    } else {
        0
    };
    let bit_depth_luma_minus8 = reader.read_ue()?;
    if bit_depth_luma_minus8 > 6 {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: bit_depth_luma_minus8 out of spec range (0..=6): {bit_depth_luma_minus8}"
        )));
    }
    let bit_depth_chroma_minus8 = reader.read_ue()?;
    if bit_depth_chroma_minus8 > 6 {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: bit_depth_chroma_minus8 out of spec range (0..=6): {bit_depth_chroma_minus8}"
        )));
    }
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

    let chroma_array_type = if separate_colour_plane_flag == 1 {
        0
    } else {
        chroma_format_idc
    };
    let params = HighProfileSpsParams {
        chroma_format_idc: chroma_format_idc as u8,
        bit_depth_luma_minus8: bit_depth_luma_minus8 as u8,
        bit_depth_chroma_minus8: bit_depth_chroma_minus8 as u8,
    };
    Ok((chroma_array_type, params))
}

/// pic_order_cnt_type に応じた追加フィールド群を読み飛ばす（仕様 7.3.2.1.1）
fn skip_pic_order_cnt_type_extras(reader: &mut BitReader<'_>) -> crate::Result<()> {
    let pic_order_cnt_type = reader.read_ue()?;
    // 仕様 7.4.2.1.1 で 0 / 1 / 2 のいずれかと規定されているため、それ以外は仕様外として弾く。
    if pic_order_cnt_type > 2 {
        return Err(crate::Error::new(format!(
            "invalid H.264 SPS: pic_order_cnt_type out of spec range (0..=2): {pic_order_cnt_type}"
        )));
    }
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
    reader: &mut BitReader<'_>,
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

/// scaling_list() サブルーチンの読み飛ばし（仕様 7.3.2.1.1.1）
///
/// 要素ごとに delta_scale (se(v)) を読む。next_scale が 0 になると以降は読まずに進める。
/// 実値は本実装では使わず、ビット位置を進めるだけ。
///
/// H.264 仕様固有のロジックなので `BitReader` 本体（汎用ビットリーダ）からは分離する。
fn skip_scaling_list(reader: &mut BitReader<'_>, size: usize) -> crate::Result<()> {
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
pub(crate) mod tests {
    use std::sync::LazyLock;

    use super::*;

    // 以下の SPS バイト列は ffmpeg + libx264 で生成した実機 SPS を抽出したもの。
    // 生成コマンドは `ffmpeg -f lavfi -i testsrc=size=WIDTHxHEIGHT:rate=30 -pix_fmt yuv420p
    // -c:v libx264 -profile:v baseline -frames:v 1 -f h264 out.h264` で、
    // 先頭の SPS NAL を 2 個目の start code 直前まで切り出した。
    //
    // 各 SPS は本モジュール外のテスト (decoder/openh264.rs::tests, rtsp/subscriber.rs::tests,
    // srt/inbound_endpoint.rs::tests) からも参照されるため `pub(crate)` で公開する。

    // Baseline プロファイル + 320x240 (16 の倍数の解像度、crop なしの最小実機 SPS パターン)。
    // NAL ヘッダ (0x67) 直後の 3 バイトは profile_idc=66 (Baseline) / constraint_set_flags=0xc0 /
    // level_idc=13 (Level 1.3)。
    pub(crate) const SPS_320X240: [u8; 24] = [
        0x67, 0x42, 0xc0, 0x0d, 0xd9, 0x01, 0x41, 0xfb, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10,
        0x00, 0x00, 0x03, 0x03, 0xc0, 0xf1, 0x42, 0xa4, 0x80,
    ];

    // Baseline プロファイル + 1920x1080 (16 倍数でない 1080 のため crop_bottom 経路を踏む実機 SPS)。
    // NAL ヘッダ (0x67) 直後の 3 バイトは profile_idc=66 (Baseline) / constraint_set_flags=0xc0 /
    // level_idc=40 (Level 4.0)。
    pub(crate) const SPS_1920X1080: [u8; 26] = [
        0x67, 0x42, 0xc0, 0x28, 0xd9, 0x00, 0x78, 0x02, 0x27, 0xe5, 0xc0, 0x44, 0x00, 0x00, 0x03,
        0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0, 0x3c, 0x60, 0xc9, 0x20,
    ];

    // SPS バイト列の Annex-B 形式 (先頭 4 バイト start code + NAL バイト列) を遅延構築する。
    // const レベルで配列連結ができないため `LazyLock<Vec<u8>>` で初期化する。
    // 利用側は `&*SPS_320X240_ANNEXB` または `&SPS_320X240_ANNEXB[..]` で `&[u8]` として参照する。
    pub(crate) static SPS_320X240_ANNEXB: LazyLock<Vec<u8>> =
        LazyLock::new(|| [&[0u8, 0, 0, 1][..], &SPS_320X240].concat());
    pub(crate) static SPS_1920X1080_ANNEXB: LazyLock<Vec<u8>> =
        LazyLock::new(|| [&[0u8, 0, 0, 1][..], &SPS_1920X1080].concat());

    #[test]
    fn extract_dimensions_from_baseline_no_crop_320x240() {
        // crop なし (16 倍数解像度) のときに raw_width / raw_height をそのまま返すこと
        let (width, height) = extract_dimensions_from_sps(&SPS_320X240).expect("SPS パース成功");
        assert_eq!((width, height), (320, 240));
    }

    #[test]
    fn find_next_annexb_start_code_detects_trailing_3byte_start_code() {
        // バッファ末尾が 3 バイト start code `[0, 0, 1]` で終わる場合に検出できること。
        // バッファ境界で Annex-B を分割して受信するストリーミング経路で発生し得る回帰を防ぐ。
        let data = [0xaa, 0x00, 0x00, 0x01];
        assert_eq!(find_next_annexb_start_code(&data), Some(1));
    }

    #[test]
    fn find_next_annexb_start_code_detects_4byte_start_code_when_preceded_by_zero() {
        // 4 バイト start code `[0, 0, 0, 1]` の場合、先頭 0 バイトの位置を返すこと
        let data = [0xaa, 0x00, 0x00, 0x00, 0x01, 0xbb];
        assert_eq!(find_next_annexb_start_code(&data), Some(1));
    }

    #[test]
    fn find_next_annexb_start_code_returns_none_when_no_start_code() {
        // start code が無いバッファでは None を返すこと
        let data = [0xaa, 0xbb, 0xcc, 0xdd];
        assert_eq!(find_next_annexb_start_code(&data), None);
    }

    #[test]
    fn find_next_annexb_start_code_returns_none_for_short_buffer() {
        // 3 バイト未満は start code に成り得ないので None を返すこと
        assert_eq!(find_next_annexb_start_code(&[]), None);
        assert_eq!(find_next_annexb_start_code(&[0]), None);
        assert_eq!(find_next_annexb_start_code(&[0, 0]), None);
    }

    #[test]
    fn h264_annexb_iterator_handles_trailing_3byte_start_code() {
        // バッファ末尾が次の NAL の 3 バイト start code で終わる場合、現在の NAL に
        // start code が混入しないこと (windows(4) ベースだと末尾 3 バイトを検出できず
        // 現在 NAL の末尾に start code が混入する回帰を防ぐ)。
        let mut data = Vec::new();
        data.extend_from_slice(&[0, 0, 0, 1]);
        data.extend_from_slice(&[0x67, 0xaa]); // SPS NAL ヘッダ + ペイロード
        data.extend_from_slice(&[0, 0, 1]); // 末尾 3 バイト start code (NAL ボディなし)

        let mut iter = H264AnnexBNalUnits::new(&data);
        let nalu = iter
            .next()
            .expect("最初の NAL がある")
            .expect("最初の NAL のパース成功");
        // 現在 NAL の data には末尾 3 バイト start code が混入しない
        assert_eq!(nalu.data, &[0x67, 0xaa]);
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
        let mut reader = BitReader::new(&data);
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
        constraint_set_flags: u8,
        chroma_format_idc: u32,
        bit_depth_luma_minus8: u32,
        bit_depth_chroma_minus8: u32,
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
        /// デフォルトは Baseline + progressive + crop なし + pic_order_cnt_type=2 +
        /// constraint_set_flags=0 + High 系プロファイル分岐用の chroma_format_idc=1 / bit_depth_*=0。
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
                constraint_set_flags: 0,
                chroma_format_idc: 1, // 4:2:0
                bit_depth_luma_minus8: 0,
                bit_depth_chroma_minus8: 0,
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

        fn with_pic_order_cnt_type(mut self, value: u32) -> Self {
            self.pic_order_cnt_type = value;
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

        fn with_profile_idc(mut self, profile_idc: u32) -> Self {
            // u8 範囲に丸める。仕様外プロファイル値テストのために u32 シグネチャを保持する。
            self.profile_idc = profile_idc as u8;
            self
        }

        fn with_constraint_set_flags(mut self, flags: u32) -> Self {
            // 他の with_* メソッドと u32 シグネチャを揃える。内部の write_u(8) は u32 で渡るため
            // フィールドへの格納時に u8 に丸める。
            self.constraint_set_flags = flags as u8;
            self
        }

        fn with_chroma_format_idc(mut self, value: u32) -> Self {
            self.chroma_format_idc = value;
            self
        }

        fn with_bit_depth_luma_minus8(mut self, value: u32) -> Self {
            self.bit_depth_luma_minus8 = value;
            self
        }

        fn with_bit_depth_chroma_minus8(mut self, value: u32) -> Self {
            self.bit_depth_chroma_minus8 = value;
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
            w.write_u(8, u32::from(self.constraint_set_flags));
            // level_idc (u(8)): 適当に Level 3.1
            w.write_u(8, 31);
            // seq_parameter_set_id
            w.write_ue(0);

            // High 系プロファイルの追加フィールド
            let is_high = H264_HIGH_PROFILES.contains(&self.profile_idc);
            if is_high {
                w.write_ue(self.chroma_format_idc);
                if self.chroma_format_idc == 3 {
                    // separate_colour_plane_flag (u(1)) = 0
                    w.write_u(1, 0);
                }
                w.write_ue(self.bit_depth_luma_minus8);
                w.write_ue(self.bit_depth_chroma_minus8);
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
                    // chroma_format_idc が 3 のときは 12 個、それ以外は 8 個
                    let count = if self.chroma_format_idc == 3 { 12 } else { 8 };
                    for _ in 0..count {
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
            .with_pic_order_cnt_type(1)
            .build();
        let (width, height) = extract_dimensions_from_sps(&sps).expect("SPS パース成功");
        assert_eq!((width, height), (1920, 1088));
    }

    #[test]
    fn extract_dimensions_handles_pic_order_cnt_type_0() {
        // pic_order_cnt_type=0 の経路（log2_max_pic_order_cnt_lsb_minus4 の読み飛ばし）を踏んでも
        // pic_width / pic_height まで正しく到達できること
        let sps = SpsBuilder::raw(1920, 1088)
            .with_pic_order_cnt_type(0)
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

    // ----------------------------------------------------------------
    // parse_sps の単体テスト群
    //
    // `extract_dimensions_from_sps` は `parse_sps` の薄いラッパーなので、
    // 上記既存テスト群が parse_sps の (width, height) パス検証を兼ねる。
    // ここでは parse_sps が返す avcC 反映用フィールド (profile_idc /
    // constraint_set_flags / level_idc / high_profile_params) と
    // 新規追加した仕様外値 Err 化の挙動を検証する。
    // ----------------------------------------------------------------

    #[test]
    fn parse_sps_baseline_returns_no_high_profile_params() {
        // Baseline (profile_idc=66) では high_profile_params が None になり、
        // profile_idc / level_idc / constraint_set_flags が SPS から取り出されること
        let sps = SpsBuilder::raw(1920, 1088)
            .with_constraint_set_flags(0xc0)
            .build();
        let params = parse_sps(&sps).expect("Baseline SPS のパース成功");
        assert_eq!(params.profile_idc, 66, "Baseline の profile_idc");
        assert_eq!(
            params.constraint_set_flags, 0xc0,
            "constraint_set_flags は SPS の RBSP byte[1] と一致"
        );
        assert_eq!(params.level_idc, 31, "SpsBuilder のデフォルト level_idc");
        assert!(
            params.high_profile_params.is_none(),
            "Baseline では high_profile_params は None"
        );
    }

    #[test]
    fn parse_sps_main_returns_no_high_profile_params() {
        // Main (profile_idc=77) でも high_profile_params が None になること
        let sps = SpsBuilder::raw(1920, 1088).with_profile_idc(77).build();
        let params = parse_sps(&sps).expect("Main SPS のパース成功");
        assert_eq!(params.profile_idc, 77);
        assert!(
            params.high_profile_params.is_none(),
            "Main では high_profile_params は None"
        );
    }

    #[test]
    fn parse_sps_high_returns_high_profile_params() {
        // High (profile_idc=100) では high_profile_params に SPS 由来の実値が入ること
        let sps = SpsBuilder::raw(1920, 1088).with_profile_idc(100).build();
        let params = parse_sps(&sps).expect("High SPS のパース成功");
        assert_eq!(params.profile_idc, 100);
        let high = params
            .high_profile_params
            .expect("High では high_profile_params は Some");
        assert_eq!(
            high.chroma_format_idc, 1,
            "SpsBuilder のデフォルト chroma_format_idc=1"
        );
        assert_eq!(high.bit_depth_luma_minus8, 0);
        assert_eq!(high.bit_depth_chroma_minus8, 0);
    }

    #[test]
    fn parse_sps_high10_returns_bit_depth_luma_minus8_2() {
        // High 10 (profile_idc=110) で bit_depth_luma_minus8=2 を指定したときに
        // SpsParams.high_profile_params.bit_depth_luma_minus8 にそのまま反映されること
        let sps = SpsBuilder::raw(1920, 1088)
            .with_profile_idc(110)
            .with_bit_depth_luma_minus8(2)
            .with_bit_depth_chroma_minus8(2)
            .build();
        let params = parse_sps(&sps).expect("High 10 SPS のパース成功");
        let high = params.high_profile_params.expect("High 10 では Some");
        assert_eq!(high.bit_depth_luma_minus8, 2);
        assert_eq!(high.bit_depth_chroma_minus8, 2);
    }

    #[test]
    fn parse_sps_rejects_unsupported_profile_idc() {
        // profile_idc=99 は {66, 77, 88} (Baseline / Main / Extended) にも
        // H264_HIGH_PROFILES (100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135) にも
        // 含まれない最小値で、仕様準拠 publisher が出さない値の代表値として選んでいる。
        let sps = SpsBuilder::raw(1920, 1088).with_profile_idc(99).build();
        let result = parse_sps(&sps);
        assert!(
            result.is_err(),
            "仕様準拠 publisher のプロファイル群以外の profile_idc は Err: {result:?}"
        );
    }

    #[test]
    fn parse_sps_rejects_chroma_format_idc_out_of_range() {
        // High プロファイル時の chroma_format_idc > 3 (仕様 7.4.2.1.1 の値域外) は Err
        let sps = SpsBuilder::raw(1920, 1088)
            .with_profile_idc(100)
            .with_chroma_format_idc(4)
            .build();
        let result = parse_sps(&sps);
        assert!(
            result.is_err(),
            "chroma_format_idc=4 は仕様値域外で Err: {result:?}"
        );
    }

    #[test]
    fn parse_sps_rejects_bit_depth_luma_minus8_out_of_range() {
        // High プロファイル時の bit_depth_luma_minus8 > 6 (仕様 7.4.2.1.1 の値域外) は Err
        let sps = SpsBuilder::raw(1920, 1088)
            .with_profile_idc(100)
            .with_bit_depth_luma_minus8(7)
            .build();
        let result = parse_sps(&sps);
        assert!(
            result.is_err(),
            "bit_depth_luma_minus8=7 は仕様値域外で Err: {result:?}"
        );
    }

    #[test]
    fn parse_sps_rejects_bit_depth_chroma_minus8_out_of_range() {
        // High プロファイル時の bit_depth_chroma_minus8 > 6 (仕様 7.4.2.1.1 の値域外) は Err
        let sps = SpsBuilder::raw(1920, 1088)
            .with_profile_idc(100)
            .with_bit_depth_chroma_minus8(7)
            .build();
        let result = parse_sps(&sps);
        assert!(
            result.is_err(),
            "bit_depth_chroma_minus8=7 は仕様値域外で Err: {result:?}"
        );
    }

    #[test]
    fn parse_sps_rejects_pic_order_cnt_type_out_of_range() {
        // pic_order_cnt_type=3 (仕様 7.4.2.1.1 の {0,1,2} 値域外) は Err
        let sps = SpsBuilder::raw(1920, 1088)
            .with_pic_order_cnt_type(3)
            .build();
        let result = parse_sps(&sps);
        assert!(
            result.is_err(),
            "pic_order_cnt_type=3 は仕様値域外で Err: {result:?}"
        );
    }

    // ----------------------------------------------------------------
    // h264_sample_entry_from_sps_pps_lists の単体テスト群
    //
    // SpsParams のフィールドが AvccBox の対応フィールドに正しくマップされること、
    // 空 sps_list / pps_list で適切な Err を返すこと、
    // 戻り値タプルの VideoFrameSize が cropping 適用後の値と一致することを直接検証する。
    // ----------------------------------------------------------------

    // PPS バイト列 (NAL ヘッダ 0x68 + 任意 payload)。テスト全体で共有する。
    pub(crate) const PPS_NAL: &[u8] = &[0x68, 0xce, 0x06, 0xe2];

    #[test]
    fn h264_sample_entry_from_sps_pps_lists_returns_err_on_empty_sps_list() {
        // sps_list が空のときは `missing H.264 SPS` Err を返す
        let result = h264_sample_entry_from_sps_pps_lists(vec![], vec![PPS_NAL.to_vec()]);
        let err = result.expect_err("sps_list 空は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("missing H.264 SPS"),
            "エラーメッセージに `missing H.264 SPS` が含まれること (実際: {display})"
        );
    }

    #[test]
    fn h264_sample_entry_from_sps_pps_lists_returns_err_on_empty_pps_list() {
        // pps_list が空のときは `missing H.264 PPS` Err を返す
        let sps = SpsBuilder::raw(1920, 1088).build();
        let result = h264_sample_entry_from_sps_pps_lists(vec![sps], vec![]);
        let err = result.expect_err("pps_list 空は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("missing H.264 PPS"),
            "エラーメッセージに `missing H.264 PPS` が含まれること (実際: {display})"
        );
    }

    #[test]
    fn h264_sample_entry_from_sps_pps_lists_returns_err_on_pps_list_with_non_pps_nal() {
        // pps_list に PPS 以外の NAL タイプ (例: SPS の 0x67) が混入した場合は Err を返す。
        // 呼び出し側の事前判定漏れで AvccBox.pps_list に壊れた NAL が move されるのを防ぐ。
        let sps = SpsBuilder::raw(1920, 1088).build();
        let non_pps_nal = vec![0x67, 0x42, 0xc0, 0x0d]; // SPS NAL ヘッダで始まる
        let result = h264_sample_entry_from_sps_pps_lists(vec![sps], vec![non_pps_nal]);
        let err = result.expect_err("PPS 以外の NAL は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("invalid H.264 PPS"),
            "エラーメッセージに `invalid H.264 PPS` が含まれること (実際: {display})"
        );
    }

    #[test]
    fn h264_sample_entry_from_sps_pps_lists_maps_baseline_sps_to_avcc() {
        // Baseline SPS の profile_idc / constraint_set_flags / level_idc が AvccBox に
        // 1:1 で反映され、chroma_format / bit_depth_* が None になることを直接検証する。
        // constraint_set_flags = 0xc0 を指定して `profile_compatibility` が RBSP byte[1] と
        // 一致することも併せて担保する。
        let sps = SpsBuilder::raw(1920, 1088)
            .with_constraint_set_flags(0xc0)
            .build();
        let (entry, _frame_size) =
            h264_sample_entry_from_sps_pps_lists(vec![sps], vec![PPS_NAL.to_vec()])
                .expect("Baseline SPS のパース成功");
        let SampleEntry::Avc1(avc1) = entry else {
            panic!("Avc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };
        assert_eq!(avc1.avcc_box.avc_profile_indication, 66);
        assert_eq!(avc1.avcc_box.profile_compatibility, 0xc0);
        assert_eq!(avc1.avcc_box.avc_level_indication, 31);
        assert!(avc1.avcc_box.chroma_format.is_none());
        assert!(avc1.avcc_box.bit_depth_luma_minus8.is_none());
        assert!(avc1.avcc_box.bit_depth_chroma_minus8.is_none());
    }

    #[test]
    fn h264_sample_entry_from_sps_pps_lists_maps_high_sps_to_avcc() {
        // High SPS の high_profile_params が AvccBox の chroma_format / bit_depth_* に
        // 1:1 で反映されることを直接検証する。luma / chroma を異なる値に設定して
        // フィールド取り違えのバグを検出可能にする。
        let sps = SpsBuilder::raw(1920, 1088)
            .with_profile_idc(100)
            .with_chroma_format_idc(1)
            .with_bit_depth_luma_minus8(2)
            .with_bit_depth_chroma_minus8(4)
            .build();
        let (entry, _frame_size) =
            h264_sample_entry_from_sps_pps_lists(vec![sps], vec![PPS_NAL.to_vec()])
                .expect("High SPS のパース成功");
        let SampleEntry::Avc1(avc1) = entry else {
            panic!("Avc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };
        assert_eq!(avc1.avcc_box.avc_profile_indication, 100);
        assert_eq!(
            avc1.avcc_box.chroma_format.expect("High では Some").get(),
            1
        );
        assert_eq!(
            avc1.avcc_box
                .bit_depth_luma_minus8
                .expect("High では Some")
                .get(),
            2
        );
        assert_eq!(
            avc1.avcc_box
                .bit_depth_chroma_minus8
                .expect("High では Some")
                .get(),
            4
        );
    }

    #[test]
    fn h264_sample_entry_from_sps_pps_lists_preserves_all_sps_pps_in_avcc() {
        // sps_list / pps_list に複数の NAL を渡したとき、`AvccBox.sps_list / pps_list` に
        // 全 NAL がそのままの順序で move されることを直接検証する。
        // 先頭 SPS のパラメータのみが avcC フィールドに反映される (Hisui の入力前提) ことの
        // 確認も兼ねる。
        let sps_a = SpsBuilder::raw(320, 240).build();
        let sps_b = SpsBuilder::raw(1920, 1088).build();
        let pps_a = PPS_NAL.to_vec();
        let pps_b = vec![0x68, 0x01, 0x02, 0x03];
        let (entry, _frame_size) = h264_sample_entry_from_sps_pps_lists(
            vec![sps_a.clone(), sps_b.clone()],
            vec![pps_a.clone(), pps_b.clone()],
        )
        .expect("複数 SPS / PPS でもパース成功");
        let SampleEntry::Avc1(avc1) = entry else {
            panic!("Avc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };
        assert_eq!(avc1.avcc_box.sps_list, vec![sps_a, sps_b]);
        assert_eq!(avc1.avcc_box.pps_list, vec![pps_a, pps_b]);
    }

    #[test]
    fn h264_sample_entry_from_sps_pps_lists_returns_frame_size_from_cropping() {
        // 戻り値タプルの第 2 要素 `VideoFrameSize` が SPS の cropping 適用後解像度と一致し、
        // `Avc1Box.visual.width / height` にも同じ値が反映されることを直接検証する。
        // libx264 が 1920x1080 を表現する典型パターン (raw 1920x1088 + crop_bottom=4)。
        let sps = SpsBuilder::raw(1920, 1088)
            .with_cropping(0, 0, 0, 4)
            .build();
        let (entry, frame_size) =
            h264_sample_entry_from_sps_pps_lists(vec![sps], vec![PPS_NAL.to_vec()])
                .expect("crop SPS のパース成功");
        assert_eq!(frame_size.width, 1920);
        assert_eq!(frame_size.height, 1080);
        let SampleEntry::Avc1(avc1) = entry else {
            panic!("Avc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };
        assert_eq!(avc1.visual.width, 1920);
        assert_eq!(avc1.visual.height, 1080);
    }

    // avcC バイト列をテスト用に構築するヘルパー (lengthSizeMinusOne = 3 固定)。
    fn build_avcc(sps_list: &[&[u8]], pps_list: &[&[u8]]) -> Vec<u8> {
        assert!(
            sps_list.len() <= 31,
            "numOfSequenceParameterSets は 5 bit のため最大 31"
        );
        let mut v = vec![
            1u8,                           // configurationVersion
            0x42,                          // AVCProfileIndication (パーサは捨てる)
            0xc0,                          // profile_compatibility (パーサは捨てる)
            0x0d,                          // AVCLevelIndication (パーサは捨てる)
            0xff, // reserved (6 bit, 全 1) + lengthSizeMinusOne (2 bit) = 3
            0xe0 | (sps_list.len() as u8), // reserved (3 bit, 全 1) + numOfSPS (5 bit)
        ];
        for sps in sps_list {
            v.extend_from_slice(&(sps.len() as u16).to_be_bytes());
            v.extend_from_slice(sps);
        }
        v.push(pps_list.len() as u8); // numOfPPS (8 bit)
        for pps in pps_list {
            v.extend_from_slice(&(pps.len() as u16).to_be_bytes());
            v.extend_from_slice(pps);
        }
        v
    }

    #[test]
    fn parse_avcc_sps_pps_lists_returns_err_on_invalid_configuration_version() {
        // byte 0 (configurationVersion) が 1 以外だと Err
        let mut avcc = build_avcc(&[&SPS_320X240[..]], &[PPS_NAL]);
        avcc[0] = 2;
        let result = parse_avcc_sps_pps_lists(&avcc);
        assert!(
            result.is_err(),
            "未サポート configurationVersion で Err が返ること: {result:?}"
        );
    }

    #[test]
    fn parse_avcc_sps_pps_lists_returns_err_on_invalid_length_size() {
        // byte 4 下位 2 bit (lengthSizeMinusOne) が 3 以外だと Err
        // 0xfc = 0b1111_1100 で lengthSizeMinusOne = 0
        let mut avcc = build_avcc(&[&SPS_320X240[..]], &[PPS_NAL]);
        avcc[4] = 0xfc;
        let result = parse_avcc_sps_pps_lists(&avcc);
        assert!(
            result.is_err(),
            "未サポート lengthSizeMinusOne で Err が返ること: {result:?}"
        );
    }

    #[test]
    fn parse_avcc_sps_pps_lists_returns_err_on_zero_sps_count() {
        // numOfSequenceParameterSets = 0 だと Err
        // build_avcc に空 SPS リストを渡すと byte 5 = 0xe0 となり numOfSPS = 0 になる
        let avcc = build_avcc(&[], &[PPS_NAL]);
        let result = parse_avcc_sps_pps_lists(&avcc);
        assert!(
            result.is_err(),
            "numOfSequenceParameterSets = 0 で Err が返ること: {result:?}"
        );
    }

    #[test]
    fn parse_avcc_sps_pps_lists_returns_err_on_zero_pps_count() {
        // numOfPictureParameterSets = 0 だと Err
        let avcc = build_avcc(&[&SPS_320X240[..]], &[]);
        let result = parse_avcc_sps_pps_lists(&avcc);
        assert!(
            result.is_err(),
            "numOfPictureParameterSets = 0 で Err が返ること: {result:?}"
        );
    }

    #[test]
    fn parse_avcc_sps_pps_lists_returns_err_on_too_many_pps() {
        // numOfPictureParameterSets > 31 だと Err (shiguredo_mp4::AvccBox::encode の制約)。
        // build_avcc は実際の PPS 数しか書かないため、numOfPPS = 32 の avcC を手動構築する。
        let mut avcc = Vec::new();
        avcc.extend_from_slice(&[1, 0x42, 0xc0, 0x0d, 0xff, 0xe1]); // configVer / profile / compat / level / len_size=3 / numSps=1
        avcc.extend_from_slice(&(SPS_320X240.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&SPS_320X240);
        avcc.push(32); // numOfPPS = 32
        for _ in 0..32 {
            avcc.extend_from_slice(&(PPS_NAL.len() as u16).to_be_bytes());
            avcc.extend_from_slice(PPS_NAL);
        }
        let result = parse_avcc_sps_pps_lists(&avcc);
        assert!(
            result.is_err(),
            "numOfPictureParameterSets > 31 で Err が返ること: {result:?}"
        );
    }

    #[test]
    fn parse_avcc_sps_pps_lists_returns_err_on_truncated_sps_length() {
        // SPS 長フィールドが残りバイトを超える avcC は Err
        let mut avcc = build_avcc(&[&SPS_320X240[..]], &[PPS_NAL]);
        // byte 6..=7 が SPS 長フィールド。残りバイト数を超える 0xFFFF に書き換える。
        avcc[6] = 0xff;
        avcc[7] = 0xff;
        let result = parse_avcc_sps_pps_lists(&avcc);
        assert!(
            result.is_err(),
            "SPS 長が残りバイトを超える avcC で Err が返ること: {result:?}"
        );
    }

    #[test]
    fn parse_avcc_sps_pps_lists_returns_err_on_too_short() {
        // バイト長が 6 未満だと Err (byte 0..=5 の固定ヘッダが揃わない)
        let data = &[1u8, 0x42, 0xc0, 0x0d, 0xff]; // 5 バイト
        let result = parse_avcc_sps_pps_lists(data);
        assert!(result.is_err(), "5 バイト入力で Err が返ること: {result:?}");
    }
}
