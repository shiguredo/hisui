//! `src/video/h264.rs` の `parse_avcc_sps_pps_lists` に対する PBT
//!
//! 任意の SPS / PPS リスト (件数 1..=31、各バイト列 0..=256) を avcC バイト列で encode し、
//! `parse_avcc_sps_pps_lists` で decode したときに SPS / PPS リストが出現順を含めて完全一致する
//! ことをラウンドトリップ性質として保証する。
//!
//! avcC の構造 (ISO/IEC 14496-15): byte 0..=5 の固定ヘッダ + SPS リスト + numOfPPS + PPS リスト。
//! lengthSizeMinusOne = 3、reserved bit は 1 詰めで encode し、`parse_avcc_sps_pps_lists` の
//! 検証経路 (configurationVersion=1 / lengthSizeMinusOne=3 / numOfSPS in 1..=31 / numOfPPS in 1..=31)
//! を満たすように生成する。

use hisui::video::h264::parse_avcc_sps_pps_lists;
use proptest::prelude::*;

// avcC バイト列を構築するヘルパー (lengthSizeMinusOne = 3 固定、reserved bit は 1 詰め)。
fn build_avcc(sps_list: &[Vec<u8>], pps_list: &[Vec<u8>]) -> Vec<u8> {
    let mut v = vec![
        1u8,                           // configurationVersion
        0x42,                          // AVCProfileIndication (パーサは捨てる)
        0xc0,                          // profile_compatibility (パーサは捨てる)
        0x0d,                          // AVCLevelIndication (パーサは捨てる)
        0xff,                          // reserved (6 bit) + lengthSizeMinusOne (2 bit) = 3
        0xe0 | (sps_list.len() as u8), // reserved (3 bit) + numOfSPS (5 bit)
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

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        .. ProptestConfig::default()
    })]

    /// 任意の SPS / PPS リスト (件数 1..=31、各 0..=256 バイト) を avcC で encode し、
    /// `parse_avcc_sps_pps_lists` で decode すると、SPS / PPS リストが出現順を含めて
    /// 完全一致すること。
    #[test]
    fn parse_avcc_sps_pps_lists_roundtrip(
        sps_list in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..=256), 1..=31),
        pps_list in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..=256), 1..=31),
    ) {
        let avcc = build_avcc(&sps_list, &pps_list);
        let (parsed_sps, parsed_pps) = parse_avcc_sps_pps_lists(&avcc)
            .expect("有効な avcC はパース成功");
        prop_assert_eq!(parsed_sps, sps_list);
        prop_assert_eq!(parsed_pps, pps_list);
    }
}
