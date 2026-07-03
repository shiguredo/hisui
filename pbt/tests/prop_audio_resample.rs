//! `src/audio/resample.rs` の `resample_to_mono` に対する PBT。

use hisui::audio::resample::resample_to_mono;
use hisui::audio::{Channels, SampleRate};
use proptest::prelude::*;

/// サポート対象のサンプルレート。
const SUPPORTED_HZ: &[u32] = &[8000, 16000, 22050, 24000, 32000, 44100, 48000];

/// 任意の f32 PCM (絶対値 1.0 以下) を生成する Strategy。
fn arb_pcm(max_len: usize) -> impl Strategy<Value = Vec<f32>> {
    prop::collection::vec(-1.0f32..=1.0f32, 0..max_len)
}

proptest! {
    /// モノラル入力の出力長は `ceil(input_len * dst_hz / src_hz)` に一致する。
    #[test]
    fn resample_mono_output_length_matches_formula(
        src_index in 0usize..SUPPORTED_HZ.len(),
        dst_index in 0usize..SUPPORTED_HZ.len(),
        pcm in arb_pcm(2048),
    ) {
        let src_hz = SUPPORTED_HZ[src_index];
        let dst_hz = SUPPORTED_HZ[dst_index];
        let src = SampleRate::from_u32(src_hz).expect("SUPPORTED_HZ は有効なはず");
        let dst = SampleRate::from_u32(dst_hz).expect("SUPPORTED_HZ は有効なはず");
        let out = resample_to_mono(&pcm, src, dst, Channels::MONO).expect("正常入力は Ok");
        let expected = pcm.len().saturating_mul(dst_hz as usize).div_ceil(src_hz as usize);
        prop_assert_eq!(out.len(), expected, "出力長は ceil(input * dst / src) に一致するはず");
    }

    /// 同一入力を 2 回リサンプルしても出力は完全一致する (決定性)。
    #[test]
    fn resample_is_deterministic(
        src_index in 0usize..SUPPORTED_HZ.len(),
        dst_index in 0usize..SUPPORTED_HZ.len(),
        pcm in arb_pcm(1024),
    ) {
        let src_hz = SUPPORTED_HZ[src_index];
        let dst_hz = SUPPORTED_HZ[dst_index];
        let src = SampleRate::from_u32(src_hz).expect("SUPPORTED_HZ は有効なはず");
        let dst = SampleRate::from_u32(dst_hz).expect("SUPPORTED_HZ は有効なはず");
        let a = resample_to_mono(&pcm, src, dst, Channels::MONO).expect("Ok");
        let b = resample_to_mono(&pcm, src, dst, Channels::MONO).expect("Ok");
        prop_assert_eq!(a, b, "同一入力なら同一出力を返すはず");
    }

    /// ステレオ入力の結果は、対応するモノラルダウンミックス入力の結果と厳密に一致する
    /// (ダウンミックスがリサンプル前に行われる仕様)。
    #[test]
    fn stereo_downmix_matches_precomputed_mono(
        src_index in 0usize..SUPPORTED_HZ.len(),
        dst_index in 0usize..SUPPORTED_HZ.len(),
        pcm_pairs in prop::collection::vec((-1.0f32..=1.0f32, -1.0f32..=1.0f32), 0..512),
    ) {
        let src_hz = SUPPORTED_HZ[src_index];
        let dst_hz = SUPPORTED_HZ[dst_index];
        let src = SampleRate::from_u32(src_hz).expect("SUPPORTED_HZ は有効なはず");
        let dst = SampleRate::from_u32(dst_hz).expect("SUPPORTED_HZ は有効なはず");
        let mut stereo = Vec::with_capacity(pcm_pairs.len() * 2);
        let mut mono = Vec::with_capacity(pcm_pairs.len());
        for (l, r) in &pcm_pairs {
            stereo.push(*l);
            stereo.push(*r);
            mono.push((l + r) * 0.5);
        }
        let out_stereo = resample_to_mono(&stereo, src, dst, Channels::STEREO).expect("Ok");
        let out_mono = resample_to_mono(&mono, src, dst, Channels::MONO).expect("Ok");
        prop_assert_eq!(out_stereo, out_mono, "ステレオ経由とモノラル経由でリサンプル結果は厳密に一致するはず");
    }
}
