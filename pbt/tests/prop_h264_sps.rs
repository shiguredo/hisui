//! `src/video/h264.rs` の `extract_dimensions_from_sps` に対する PBT
//!
//! 任意のバイト列を SPS の payload として投入したとき、`extract_dimensions_from_sps` は
//! パニックや無限ループを起こさず必ず `Ok((width, height))` か `Err` を返すことを
//! クラッシュフリー性質として保証する。
//!
//! `extract_dimensions_from_sps` は先頭 1 バイトに NAL ヘッダ（下位 5 bit が 7）を期待する
//! 入力契約のため、入力先頭バイトは SPS の NAL ヘッダに固定し、それ以降のバイトを任意化する。

use hisui::video::h264::extract_dimensions_from_sps;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1024,
        .. ProptestConfig::default()
    })]

    /// 入力に何が来てもパニックや無限ループを起こさず Ok か Err を返すこと
    /// （クラッシュフリーの保証）
    #[test]
    fn extract_dimensions_from_sps_does_not_panic(payload in prop::collection::vec(any::<u8>(), 0..=4096)) {
        // 先頭バイトは SPS NAL ヘッダ固定（0x67 = forbidden_zero_bit=0 + nal_ref_idc=3 + nal_unit_type=7）
        let mut sps = Vec::with_capacity(payload.len() + 1);
        sps.push(0x67);
        sps.extend_from_slice(&payload);
        // パース結果は問わない（Ok でも Err でもよい）。重要なのは panic / 無限ループしないこと。
        let _ = extract_dimensions_from_sps(&sps);
    }
}
