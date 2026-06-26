//! `AacRtpDepacketizer` 経路に対する PBT
//!
//! - ラウンドトリップ: `build_aac_rtp_payload` で組み立てた payload を
//!   `AacRtpDepacketizer::depacketize` に流して元の AU データ列が取り出せることを担保する。
//! - クラッシュフリー: 任意の fmtp パラメータと任意の payload バイト列で
//!   `AacRtpDepacketizer::depacketize` 経路が panic しないことを担保する。
//!
//! 単体テストは Err 文言 assert や境界値受理など、PBT で表現しづらいケースに絞って
//! `src/rtsp/subscriber.rs::tests` に残している。

use hisui::audio::aac::{AacRtpDepacketizer, validate_aac_fmtp_lengths};
use proptest::prelude::*;
use shiguredo_rtsp::{RtpPacket, rtp::RtpHeader};

/// AAC fmtp パラメータと AU データの組
#[derive(Debug, Clone)]
struct AacParams {
    size_length: u8,
    index_length: u8,
    index_delta_length: u8,
    au_data: Vec<Vec<u8>>,
}

/// AU 1 個あたりのバイト長上限。実機の AAC AU (典型 100〜1500 byte) より小さく
/// 設定し、proptest シュリンク時間を実用域に保つ。
const MAX_AU_SIZE: usize = 64;

/// 1 RTP packet に詰める AU 個数の上限。`size_length=32` × `index_delta_length=32` ×
/// `au_count` の最大値で AU-headers-length (u16) が overflow しないよう抑える
/// (32 + 32 + 7 * 64 = 512 bit、u16::MAX = 65535 に十分余裕)。
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
///   RFC 3640 §3.3.6 の値域上限を尊重した範囲)
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

/// PBT 内で RFC 3640 §3.3.6 の AU-headers-length + au_headers + au_data を組み立てる。
///
/// `size_length` は 1..=32、`index_length` / `index_delta_length` は 0..=32 を取る。
/// `au_data` の各 AU バイト長は `(1 << size_length) - 1` 以下である必要がある (溢れると assert)。
fn build_aac_rtp_payload(
    size_length: u8,
    index_length: u8,
    index_delta_length: u8,
    au_data: &[Vec<u8>],
) -> Vec<u8> {
    assert!(
        (1..=32).contains(&size_length),
        "size_length must be in 1..=32"
    );
    assert!(index_length <= 32, "index_length must be <= 32");
    assert!(index_delta_length <= 32, "index_delta_length must be <= 32");
    assert!(!au_data.is_empty(), "au_data must not be empty");
    let size_cap = if size_length >= 32 {
        u64::from(u32::MAX)
    } else {
        (1u64 << size_length) - 1
    };
    for au in au_data {
        assert!(
            (au.len() as u64) <= size_cap,
            "AU size exceeds size_length bit width"
        );
    }

    let au_headers_length_bits = (size_length as usize)
        + (index_length as usize)
        + (au_data.len() - 1) * ((size_length as usize) + (index_delta_length as usize));
    let au_headers_length_bytes = au_headers_length_bits.div_ceil(8);

    let mut payload = Vec::new();
    payload.extend_from_slice(
        &u16::try_from(au_headers_length_bits)
            .expect("AU headers length exceeds u16 range")
            .to_be_bytes(),
    );

    let mut au_headers = vec![0u8; au_headers_length_bytes];
    let mut bit_offset = 0usize;
    for (i, au) in au_data.iter().enumerate() {
        write_bits_msb_first(
            &mut au_headers,
            &mut bit_offset,
            size_length,
            au.len() as u64,
        );
        let index_bits = if i == 0 {
            index_length
        } else {
            index_delta_length
        };
        write_bits_msb_first(&mut au_headers, &mut bit_offset, index_bits, 0);
    }
    payload.extend_from_slice(&au_headers);

    for au in au_data {
        payload.extend_from_slice(au);
    }

    payload
}

/// `build_aac_rtp_payload` 用のビット書き込みヘルパー (MSB ファースト)。
/// `n` は最大 64 を想定 (`n > 64` で `value >> 64` が panic する)。
fn write_bits_msb_first(buf: &mut [u8], bit_offset: &mut usize, n: u8, value: u64) {
    debug_assert!(n <= 64, "write_bits_msb_first does not support n > 64");
    for i in 0..n {
        let bit = ((value >> (n - 1 - i)) & 1) as u8;
        let byte_index = *bit_offset / 8;
        let bit_index = 7 - (*bit_offset % 8);
        buf[byte_index] |= bit << bit_index;
        *bit_offset += 1;
    }
}

/// PBT は `data` バイト列のみ検証するため、RTP ヘッダ値は depacketize 戻り値に
/// 影響しない範囲のダミー値で組み立てる。
fn build_test_packet(payload: Vec<u8>) -> RtpPacket {
    RtpPacket {
        header: RtpHeader::new(0, 0, 0, 0),
        extension: None,
        payload,
        padding_size: 0,
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// `build_aac_rtp_payload` で組み立てた payload を `AacRtpDepacketizer::depacketize`
    /// に流すと元の au_data 列が完全一致で取り出せること (ラウンドトリップ)。
    #[test]
    fn prop_aac_rtp_depacketize_roundtrips_au_data(params in aac_params_strategy()) {
        let payload = build_aac_rtp_payload(
            params.size_length,
            params.index_length,
            params.index_delta_length,
            &params.au_data,
        );
        let depacketizer = AacRtpDepacketizer::new(
            params.size_length,
            params.index_length,
            params.index_delta_length,
        );
        let aus = depacketizer
            .depacketize(&build_test_packet(payload))
            .map_err(|e| TestCaseError::fail(format!(
                "正常 payload が depacketize で Err になった: {}",
                e.display()
            )))?;
        let actual: Vec<Vec<u8>> = aus.into_iter().map(|au| au.data).collect();
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

    /// 任意の fmtp パラメータと任意の payload バイト列で `AacRtpDepacketizer::depacketize`
    /// 経路が panic しないこと (Result で表現される、クラッシュフリー)。
    ///
    /// `payload` の上限 8200 byte は AU-headers-length (u16) 最大値 65535 bit =
    /// `div_ceil(8) = 8192 byte` を超えるよう設定し、`BitReader` ループ内部の境界処理も
    /// 含めてカバーする。`size_length == 0` や `> 32` は `validate_aac_fmtp_lengths` で
    /// Err 化されるため、`AacRtpDepacketizer::new` の `debug_assert!(size_length > 0)` には
    /// 到達しない。
    #[test]
    fn prop_aac_rtp_depacketize_does_not_panic(
        size_length in 0u8..=u8::MAX,
        index_length in 0u8..=u8::MAX,
        index_delta_length in 0u8..=u8::MAX,
        payload in proptest::collection::vec(any::<u8>(), 0..=8200),
    ) {
        if validate_aac_fmtp_lengths(size_length, index_length, index_delta_length).is_err() {
            return Ok(());
        }
        let depacketizer = AacRtpDepacketizer::new(size_length, index_length, index_delta_length);
        let _ = depacketizer.depacketize(&build_test_packet(payload));
    }
}
