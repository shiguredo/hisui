//! `src/video/av1.rs` の `parse_av1_codec_private` に対する PBT
//!
//! AV1CodecConfigurationRecord (固定 4 バイトヘッダ + configOBUs) を任意の configOBUs 部分で
//! encode し、`parse_av1_codec_private` で decode したときに configOBUs バイト列が完全一致する
//! ことをラウンドトリップ性質として保証する。
//!
//! 入力は (marker=1, version=1) の妥当な 4 バイトヘッダ + 任意の configOBUs を生成する。
//! byte 1..=3 はパーサが検証も抽出もしないため任意化する。

use hisui::video::av1::parse_av1_codec_private;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1024,
        .. ProptestConfig::default()
    })]

    /// 妥当な 4 バイトヘッダ + 任意の configOBUs バイト列を encode し、
    /// `parse_av1_codec_private` で decode すると configOBUs が完全一致すること。
    #[test]
    fn parse_av1_codec_private_roundtrip(
        // byte 1..=3 (seq_profile / seq_level_idx_0 等) は任意
        header_bytes in proptest::array::uniform3(any::<u8>()),
        // configOBUs バイト列 (空でも parse_av1_codec_private は Ok を返す契約)
        config_obus in prop::collection::vec(any::<u8>(), 0..=4096),
    ) {
        // byte 0 = 0x81 (marker=1, version=1) で固定。byte 1..=3 は任意値。
        let mut data = vec![0x81u8];
        data.extend_from_slice(&header_bytes);
        data.extend_from_slice(&config_obus);
        let parsed = parse_av1_codec_private(&data)
            .expect("妥当なヘッダ + 任意 configOBUs はパース成功");
        prop_assert_eq!(parsed, config_obus.as_slice());
    }
}
