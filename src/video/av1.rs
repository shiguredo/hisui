use shiguredo_mp4::{
    Uint,
    boxes::{Av01Box, Av1cBox, SampleEntry},
};

use crate::{types::EvenUsize, video};

pub fn av1_sample_entry(width: EvenUsize, height: EvenUsize, config_obus: &[u8]) -> SampleEntry {
    SampleEntry::Av01(Av01Box {
        visual: video::sample_entry_visual_fields(width.get(), height.get()),
        av1c_box: Av1cBox {
            seq_profile: Uint::new(0),            // Main profile
            seq_level_idx_0: Uint::new(0),        // Default level (unrestricted)
            seq_tier_0: Uint::new(0),             // Main tier
            high_bitdepth: Uint::new(0),          // false
            twelve_bit: Uint::new(0),             // false
            monochrome: Uint::new(0),             // false
            chroma_subsampling_x: Uint::new(1),   // 4:2:0 subsampling
            chroma_subsampling_y: Uint::new(1),   // 4:2:0 subsampling
            chroma_sample_position: Uint::new(0), // Colocated with luma (0, 0)
            initial_presentation_delay_minus_one: None,
            config_obus: config_obus.to_vec(),
        },
        unknown_boxes: Vec::new(),
    })
}

/// WebM CodecPrivate の AV1CodecConfigurationRecord から configOBUs スライス参照を抽出する。
///
/// AOM Codecs ISO Media File Format Binding §2.3 に基づき、固定 4 バイトヘッダ
/// (byte 0: marker / version、byte 1..=3: seq_profile / seq_level_idx_0 等) を
/// 読み飛ばして byte 4 以降の configOBUs スライス参照を返す。byte 1..=3 の各フィールドは
/// 検証も抽出もしない (av1_sample_entry が Av1cBox の固定値を使うため、ヘッダ実値は不要)。
///
/// configOBUs が空 (data.len() == 4) でも Ok を返す。
pub fn parse_av1_codec_private(data: &[u8]) -> crate::Result<&[u8]> {
    if data.len() < 4 {
        return Err(crate::Error::new(format!(
            "invalid AV1 CodecPrivate: too short (expected >= 4 bytes, got {})",
            data.len()
        )));
    }
    let marker = data[0] >> 7;
    if marker != 1 {
        return Err(crate::Error::new(
            "invalid AV1 CodecPrivate: marker bit is not set",
        ));
    }
    let version = data[0] & 0b0111_1111;
    if version != 1 {
        return Err(crate::Error::new(format!(
            "invalid AV1 CodecPrivate: unsupported version {version}"
        )));
    }
    Ok(&data[4..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_av1_codec_private_extracts_config_obus() {
        // 正常な AV1CodecConfigurationRecord: 4 バイトヘッダ + 5 バイトの configOBUs ダミー列。
        // byte 0 = 0x81 (marker=1, version=1)、byte 1..=3 は任意値。
        let data = [0x81, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        let result = parse_av1_codec_private(&data).expect("正常入力でパースが成功する");
        assert_eq!(
            result,
            &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE],
            "byte 4 以降の configOBUs を返すこと"
        );
    }

    #[test]
    fn parse_av1_codec_private_returns_err_on_too_short() {
        // バイト長が 4 未満の入力は Err を返す。
        let data = [0x81, 0x00, 0x00];
        assert!(
            parse_av1_codec_private(&data).is_err(),
            "3 バイト入力で Err が返ること"
        );
    }

    #[test]
    fn parse_av1_codec_private_returns_err_on_marker_bit_unset() {
        // byte 0 の最上位 bit (marker) が 0 だと Err。
        // byte 0 = 0x01 (marker=0, version=1)
        let data = [0x01, 0x00, 0x00, 0x00];
        assert!(
            parse_av1_codec_private(&data).is_err(),
            "marker bit 不在で Err が返ること"
        );
    }

    #[test]
    fn parse_av1_codec_private_returns_err_on_unsupported_version() {
        // byte 0 の下位 7 bit (version) が 1 以外だと Err。
        // byte 0 = 0x82 (marker=1, version=2)
        let data = [0x82, 0x00, 0x00, 0x00];
        assert!(
            parse_av1_codec_private(&data).is_err(),
            "未サポート version で Err が返ること"
        );
    }
}
