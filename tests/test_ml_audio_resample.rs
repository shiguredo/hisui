//! `src/ml/audio/resample.rs` の integration テスト。

#![cfg(feature = "candle")]

use hisui::audio::{Channels, SampleRate};
use hisui::ml::audio::resample::{bessel_i0, resample_to_16k_mono};

/// Bessel I0 の代表値を SciPy / WolframAlpha の参照値と相対誤差 1e-6 未満で一致することを確認する。
#[test]
fn bessel_i0_matches_reference_values() {
    // I0(0) = 1 (定義から厳密)
    assert_eq!(bessel_i0(0.0), 1.0);

    // I0(1) ≈ 1.2660658 (WolframAlpha: BesselI[0, 1] を f32 精度に丸めた値)
    let expected_at_1 = 1.266_065_8_f32;
    let got = bessel_i0(1.0);
    let rel_err = ((got - expected_at_1) / expected_at_1).abs();
    assert!(
        rel_err < 1e-6,
        "bessel_i0(1.0) の相対誤差 {rel_err} が 1e-6 を超えた (got={got}, expected={expected_at_1})"
    );

    // I0(8.6) ≈ 750.4612 (級数展開 sum (x/2)^(2n)/(n!)^2 の Python 手計算値、f32 精度に丸め)
    let expected_at_86 = 750.461_2_f32;
    let got = bessel_i0(8.6);
    let rel_err = ((got - expected_at_86) / expected_at_86).abs();
    assert!(
        rel_err < 1e-6,
        "bessel_i0(8.6) の相対誤差 {rel_err} が 1e-6 を超えた (got={got}, expected={expected_at_86})"
    );
}

/// 空スライスは `Ok(vec![])` を返す。
#[test]
fn resample_to_16k_mono_accepts_empty_input() {
    let src = SampleRate::from_u32(48000).expect("48000 は有効なサンプルレート");
    let out = resample_to_16k_mono(&[], src, Channels::MONO).expect("空スライスは Ok");
    assert!(out.is_empty(), "空入力なら空出力");
}

/// 同一サンプルレート (16 kHz → 16 kHz) は入力をそのまま返す。
#[test]
fn resample_to_16k_mono_passes_through_16k() {
    let input: Vec<f32> = (0..100).map(|i| (i as f32) * 0.01).collect();
    let src = SampleRate::from_u32(16000).expect("16000 は有効なサンプルレート");
    let out = resample_to_16k_mono(&input, src, Channels::MONO).expect("16k → 16k は Ok");
    assert_eq!(out, input, "16 kHz → 16 kHz は入力と一致するはず");
}

/// ステレオ入力はチャンネル平均でモノラルにダウンミックスされる。
#[test]
fn resample_to_16k_mono_downmixes_stereo() {
    // ステレオ 16 kHz を入力し、モノラル 16 kHz を得る。
    let input = vec![0.4f32, 0.6, 0.2, 0.8, 1.0, 0.0];
    let src = SampleRate::from_u32(16000).expect("16000 は有効なサンプルレート");
    let out = resample_to_16k_mono(&input, src, Channels::STEREO).expect("ステレオ入力は Ok");
    // (0.4+0.6)/2 = 0.5、(0.2+0.8)/2 = 0.5、(1.0+0.0)/2 = 0.5
    assert_eq!(out, vec![0.5, 0.5, 0.5]);
}

/// ステレオ入力のサンプル数が奇数の場合は Err。
#[test]
fn resample_to_16k_mono_rejects_odd_stereo_length() {
    let input = vec![0.1f32, 0.2, 0.3];
    let src = SampleRate::from_u32(16000).expect("16000 は有効なサンプルレート");
    let err = resample_to_16k_mono(&input, src, Channels::STEREO)
        .expect_err("ステレオで奇数長は Err になる想定");
    let msg = err.display().to_string();
    assert!(
        msg.contains("even"),
        "エラーメッセージに 'even' を含むこと: {msg}"
    );
}

/// サポート外のサンプルレートは Err。
#[test]
fn resample_to_16k_mono_rejects_unsupported_rate() {
    let src = SampleRate::from_u32(11025).expect("11025 は SampleRate 型としては有効");
    let err = resample_to_16k_mono(&[0.0; 1024], src, Channels::MONO)
        .expect_err("11025 Hz は本 API では非サポート");
    let msg = err.display().to_string();
    assert!(
        msg.contains("11025") && msg.contains("unsupported"),
        "エラーメッセージに '11025' と 'unsupported' を含むこと: {msg}"
    );
}

/// 48 kHz → 16 kHz の出力長は `ceil(input_len * 16000 / 48000)` に一致する。
#[test]
fn resample_to_16k_mono_48k_output_length() {
    let src = SampleRate::from_u32(48000).expect("48000 は有効なサンプルレート");
    let input_len = 480;
    let input = vec![0.0f32; input_len];
    let out = resample_to_16k_mono(&input, src, Channels::MONO).expect("48k → 16k は Ok");
    let expected = input_len.div_ceil(3); // 480 * 16000 / 48000 = 160
    assert_eq!(out.len(), expected);
}

/// 44.1 kHz → 16 kHz の非整数比リサンプルも出力長式に従う (端数切り上げ)。
#[test]
fn resample_to_16k_mono_44100_output_length() {
    let src = SampleRate::from_u32(44100).expect("44100 は有効なサンプルレート");
    let input = vec![0.0f32; 441];
    let out = resample_to_16k_mono(&input, src, Channels::MONO).expect("44.1k → 16k は Ok");
    // ceil(441 * 16000 / 44100) = ceil(160.0) = 160
    assert_eq!(out.len(), 160);
}
