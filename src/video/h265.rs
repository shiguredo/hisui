use shiguredo_mp4::boxes::{Hvc1Box, HvccBox, HvccNalUintArray, SampleEntry};

use crate::{
    types::EvenUsize,
    video::{self, FrameRate},
};

pub type NalUnitArray = Vec<Vec<u8>>;

// H.265 の NAL ユニット前に付与されるサイズのバイト数
// Sora / Hisui が生成するものは全て 4 バイトなので固定値でいい（H.264と同様）
pub use crate::video::h264::NALU_HEADER_LENGTH;

// H.265 の NAL ユニットタイプ
pub const H265_NALU_TYPE_VPS: u8 = 32;
pub const H265_NALU_TYPE_SPS: u8 = 33;
pub const H265_NALU_TYPE_PPS: u8 = 34;

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

        let i = self
            .data
            .windows(4)
            .position(|w| matches!(w, [0, 0, 1, _] | [0, 0, 0, 1]))
            .unwrap_or(self.data.len());
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

#[derive(Debug)]
pub struct H265NalUnit<'a> {
    pub ty: u8,
    pub data: &'a [u8],
}

/// H.265 サンプルエントリーを生成する
pub fn h265_sample_entry(
    width: EvenUsize,
    height: EvenUsize,
    fps: FrameRate,
    vps_list: NalUnitArray,
    sps_list: NalUnitArray,
    pps_list: NalUnitArray,
) -> crate::Result<SampleEntry> {
    // [NOTE]
    // H.265 を表現するためのボックスには hev1 もあり、機能的には hev1 と hvc1 は
    // ほぼ同様（後者の場合にはキーフレームのサンプルデータ本体に SPS などの情報を付与することが必須なのが異なる）だが、
    // hev1 は Apple 系の動画プレイヤーでサポートされていないため、ここでは hvc1 を使用している
    Ok(SampleEntry::Hvc1(Hvc1Box {
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

/// Annex B 形式の H.265 データから VPS, SPS, PPS を抽出してサンプルエントリーを生成する
pub fn h265_sample_entry_from_annexb(
    width: usize,
    height: usize,
    fps: FrameRate,
    data: &[u8],
) -> crate::Result<SampleEntry> {
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

    if vps_list.is_empty() {
        return Err(crate::Error::new("missing H.265 VPS"));
    }
    if sps_list.is_empty() {
        return Err(crate::Error::new("missing H.265 SPS"));
    }
    if pps_list.is_empty() {
        return Err(crate::Error::new("missing H.265 PPS"));
    }

    let width = EvenUsize::new(width)
        .ok_or_else(|| crate::Error::new(format!("H.265 width must be even, got {width}")))?;
    let height = EvenUsize::new(height)
        .ok_or_else(|| crate::Error::new(format!("H.265 height must be even, got {height}")))?;

    h265_sample_entry(width, height, fps, vps_list, sps_list, pps_list)
}

#[cfg(test)]
mod tests {
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
}
