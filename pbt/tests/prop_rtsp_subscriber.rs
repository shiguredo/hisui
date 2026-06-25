//! `AacRtpDepacketizer` 経路に対する PBT
//!
//! - ラウンドトリップ: `build_aac_rtp_payload_for_pbt` → `depacketize_aac_payload_for_pbt`
//!   で任意の (size_length, index_length, index_delta_length, au_data) を往復させて元の
//!   AU データが取り出せることを担保する。
//! - クラッシュフリー: 任意の (fmtp パラメータ, payload バイト列) で
//!   `depacketize_aac_payload_for_pbt` が panic しないことを担保する。
//!
//! 単体テストは Err 文言 assert や境界値受理など、PBT で表現しづらいケースに絞って
//! `src/rtsp/subscriber.rs::tests` に残している。

use hisui::rtsp::subscriber::{build_aac_rtp_payload_for_pbt, depacketize_aac_payload_for_pbt};
use proptest::prelude::*;

/// AAC fmtp パラメータと AU データの組
#[derive(Debug, Clone)]
struct AacParams {
    size_length: u8,
    index_length: u8,
    index_delta_length: u8,
    au_data: Vec<Vec<u8>>,
}

/// AU 1 個あたりのバイト長上限。Strategy が `size_length` ビットで表現可能な範囲に収まるよう絞る。
const MAX_AU_SIZE: usize = 64;

/// AU 個数の上限 (1 RTP packet 内に詰める典型レンジ)
const MAX_AU_COUNT: usize = 8;

/// `size_length` が表現可能な最大 AU バイト長を返す (Strategy で AU サイズを切り詰めるため)。
/// Strategy は `size_length: 1..=32` に絞っているため `size_length == 0` には対応しない。
fn au_size_cap(size_length: u8) -> usize {
    if size_length >= 32 {
        return MAX_AU_SIZE;
    }
    std::cmp::min(MAX_AU_SIZE, ((1u64 << size_length) - 1) as usize)
}

/// AacParams を生成する Strategy。
///
/// - `size_length`: 1..=32 (`AacRtpDepacketizer::new` の `debug_assert!(size_length > 0)` と
///   `select_audio_track` の `> 32` 検査を尊重した範囲)
/// - `index_length` / `index_delta_length`: 0..=32 (RFC 3640 §3.3.6 で 0 も合法)
/// - `au_data`: 1..=8 個の AU、各 AU は `size_length` ビットで表現できるバイト長以下
fn aac_params_strategy() -> impl Strategy<Value = AacParams> {
    (1u8..=32, 0u8..=32, 0u8..=32).prop_flat_map(
        |(size_length, index_length, index_delta_length)| {
            let cap = au_size_cap(size_length);
            proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 0..=cap),
                1..=MAX_AU_COUNT,
            )
            .prop_map(move |au_data| AacParams {
                size_length,
                index_length,
                index_delta_length,
                au_data,
            })
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// `build_aac_rtp_payload_for_pbt` で組み立てた payload を `depacketize_aac_payload_for_pbt`
    /// に流し込むと元の au_data 列が完全一致で取り出せること (ラウンドトリップ)。
    #[test]
    fn prop_depacketize_aac_roundtrips_au_data(params in aac_params_strategy()) {
        let payload = build_aac_rtp_payload_for_pbt(
            params.size_length,
            params.index_length,
            params.index_delta_length,
            &params.au_data,
        );
        let actual = depacketize_aac_payload_for_pbt(
            params.size_length,
            params.index_length,
            params.index_delta_length,
            &payload,
        )
        .map_err(|e| TestCaseError::fail(format!(
            "正常 payload が depacketize で Err になった: {}",
            e.display()
        )))?;
        prop_assert_eq!(
            actual.len(),
            params.au_data.len(),
            "取り出した AU 個数が組み立て時の入力と一致しないこと"
        );
        for (i, (got, expected)) in actual.iter().zip(params.au_data.iter()).enumerate() {
            prop_assert_eq!(
                got,
                expected,
                "AU#{} の data が組み立て時の入力と一致しないこと",
                i
            );
        }
    }

    /// 任意の fmtp パラメータと任意の payload バイト列で `depacketize_aac_payload_for_pbt`
    /// が panic しないこと (Result で表現される、クラッシュフリー)。
    ///
    /// `payload` の上限 8200 byte は AU-headers-length (u16) 最大値 65535 bit =
    /// `div_ceil(8) = 8192 byte` を超えるよう設定し、`BitReader` ループ内部の境界処理も
    /// 含めてカバーする。`size_length == 0` や `> 32` は helper 入口の
    /// `validate_aac_fmtp_lengths` で Err 化されるため、`AacRtpDepacketizer::new` の
    /// `debug_assert!(size_length > 0)` には到達しない。
    #[test]
    fn prop_depacketize_aac_does_not_panic(
        size_length in 0u8..=u8::MAX,
        index_length in 0u8..=u8::MAX,
        index_delta_length in 0u8..=u8::MAX,
        payload in proptest::collection::vec(any::<u8>(), 0..=8200),
    ) {
        let _ = depacketize_aac_payload_for_pbt(
            size_length,
            index_length,
            index_delta_length,
            &payload,
        );
    }
}
