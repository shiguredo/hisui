use shiguredo_mp4::boxes::{DopsBox, OpusBox, SampleEntry};

use crate::audio::{self, Channels, SampleRate};

/// Opus 用の sample_entry を構築する。
///
/// 引数 `pre_skip` は OpusHead (RFC 7845 §5.1) から取得した値。
/// channels / sample_rate / output_gain は Hisui の固定値 (Stereo / 48kHz / 0) を使う。
pub fn opus_sample_entry(pre_skip: u16) -> SampleEntry {
    SampleEntry::Opus(OpusBox {
        audio: audio::sample_entry_audio_fields(),
        dops_box: DopsBox {
            output_channel_count: Channels::STEREO.get(),
            pre_skip,
            input_sample_rate: SampleRate::HZ_48000.get(),
            output_gain: 0,
        },
        unknown_boxes: Vec::new(),
    })
}

/// OpusHead (RFC 7845 §5.1) から `pre_skip` を抽出する。
///
/// オフセット 0-7: マジック `OpusHead`、8: Version (MUST be 1)、10-11: Pre-Skip (LE u16)、
/// 18: ChannelMappingFamily。Sora 録画は ChannelMappingFamily = 0 (stereo) 固定で
/// OpusHead は 19 バイト固定のため、Version != 1 / ChannelMappingFamily != 0 は
/// 仕様逸脱として `Err` を返す (silent degradation 回避)。
pub fn parse_opus_head_pre_skip(data: &[u8]) -> crate::Result<u16> {
    if data.len() < 19 {
        return Err(crate::Error::new(format!(
            "OpusHead too short: {} bytes (expected at least 19)",
            data.len()
        )));
    }
    if &data[0..8] != b"OpusHead" {
        return Err(crate::Error::new("OpusHead magic mismatch"));
    }
    if data[8] != 1 {
        return Err(crate::Error::new(format!(
            "unsupported OpusHead version: {} (expected 1)",
            data[8]
        )));
    }
    if data[18] != 0 {
        return Err(crate::Error::new(format!(
            "unsupported OpusHead ChannelMappingFamily: {} (expected 0, stereo)",
            data[18]
        )));
    }
    Ok(u16::from_le_bytes([data[10], data[11]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_opus_head_pre_skip_returns_err_on_too_short_data() {
        // RFC 7845 §5.1 で定める ChannelMappingFamily=0 の OpusHead は最短 19 バイト。
        // それ未満は不正として Err を返すこと。
        let too_short = vec![0u8; 18];
        let result = parse_opus_head_pre_skip(&too_short);
        assert!(result.is_err(), "19 バイト未満は Err を返すこと");
    }

    #[test]
    fn parse_opus_head_pre_skip_returns_err_on_magic_mismatch() {
        // 先頭 8 バイトが b"OpusHead" でない場合は Err を返すこと。
        let mut data = vec![0u8; 19];
        data[0..8].copy_from_slice(b"NotOpusH");
        data[8] = 1;
        let result = parse_opus_head_pre_skip(&data);
        assert!(result.is_err(), "マジック不一致は Err を返すこと");
    }

    #[test]
    fn parse_opus_head_pre_skip_returns_err_on_unsupported_version() {
        // Version (オフセット 8) は RFC 7845 で MUST be 1。それ以外は Err を返すこと。
        let mut data = vec![0u8; 19];
        data[0..8].copy_from_slice(b"OpusHead");
        data[8] = 2; // 仕様違反の Version
        let result = parse_opus_head_pre_skip(&data);
        assert!(result.is_err(), "Version != 1 は Err を返すこと");
    }

    #[test]
    fn parse_opus_head_pre_skip_returns_err_on_unsupported_channel_mapping_family() {
        // ChannelMappingFamily (オフセット 18) は Sora 録画では 0 (stereo) 固定。
        // それ以外は Sora 前提が崩れたとして Err を返すこと。
        let mut data = vec![0u8; 19];
        data[0..8].copy_from_slice(b"OpusHead");
        data[8] = 1;
        data[18] = 1; // surround など
        let result = parse_opus_head_pre_skip(&data);
        assert!(
            result.is_err(),
            "ChannelMappingFamily != 0 は Err を返すこと"
        );
    }

    #[test]
    fn parse_opus_head_pre_skip_extracts_value_in_little_endian() {
        // 正常な OpusHead からは pre_skip をオフセット 10-11 から LE u16 で取り出せること。
        let mut data = vec![0u8; 19];
        data[0..8].copy_from_slice(b"OpusHead");
        data[8] = 1; // Version
        data[9] = 2; // OutputChannelCount
        data[10] = 0x34; // pre_skip LE 下位バイト
        data[11] = 0x12; // pre_skip LE 上位バイト
        // data[18] = 0; (デフォルトで 0 = stereo)
        let pre_skip = parse_opus_head_pre_skip(&data).expect("正常な OpusHead は pre_skip を返す");
        assert_eq!(
            pre_skip, 0x1234,
            "pre_skip が LE u16 として正しく抽出されること"
        );
    }
}
