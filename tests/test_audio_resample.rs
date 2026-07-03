//! `src/audio/resample.rs` の integration テスト。

use hisui::audio::resample::resample_to_mono;
use hisui::audio::{Channels, SampleRate};

/// テスト用に `SampleRate` を生成する補助関数。
fn sr(hz: u32) -> SampleRate {
    SampleRate::from_u32(hz).expect("有効なサンプルレート")
}

/// 空スライスは `Ok(vec![])` を返す。
#[test]
fn resample_to_mono_accepts_empty_input() {
    let out = resample_to_mono(&[], sr(48000), sr(16000), Channels::MONO).expect("空スライスは Ok");
    assert!(out.is_empty(), "空入力なら空出力");
}

/// 同一サンプルレート (16 kHz → 16 kHz) は入力をそのまま返す。
#[test]
fn resample_to_mono_passes_through_same_rate() {
    let input: Vec<f32> = (0..100).map(|i| (i as f32) * 0.01).collect();
    let out =
        resample_to_mono(&input, sr(16000), sr(16000), Channels::MONO).expect("16k → 16k は Ok");
    assert_eq!(out, input, "同じサンプルレート同士は入力と一致するはず");
}

/// ステレオ入力はチャンネル平均でモノラルにダウンミックスされる。
#[test]
fn resample_to_mono_downmixes_stereo() {
    // ステレオ 16 kHz を入力し、モノラル 16 kHz を得る。
    let input = vec![0.4f32, 0.6, 0.2, 0.8, 1.0, 0.0];
    let out = resample_to_mono(&input, sr(16000), sr(16000), Channels::STEREO)
        .expect("ステレオ入力は Ok");
    // (0.4+0.6)/2 = 0.5、(0.2+0.8)/2 = 0.5、(1.0+0.0)/2 = 0.5
    assert_eq!(out, vec![0.5, 0.5, 0.5]);
}

/// ステレオ入力のサンプル数が奇数の場合は Err。
#[test]
fn resample_to_mono_rejects_odd_stereo_length() {
    let input = vec![0.1f32, 0.2, 0.3];
    let err = resample_to_mono(&input, sr(16000), sr(16000), Channels::STEREO)
        .expect_err("ステレオで奇数長は Err になる想定");
    let msg = err.display().to_string();
    assert!(
        msg.contains("even"),
        "エラーメッセージに 'even' を含むこと: {msg}"
    );
}

/// サポート外の src サンプルレートは Err。
#[test]
fn resample_to_mono_rejects_unsupported_source_rate() {
    let unsupported = SampleRate::from_u32(11025).expect("11025 は SampleRate 型としては有効");
    let err = resample_to_mono(&[0.0; 1024], unsupported, sr(16000), Channels::MONO)
        .expect_err("11025 Hz は本 API では非サポート");
    let msg = err.display().to_string();
    assert!(
        msg.contains("11025") && msg.contains("source") && msg.contains("unsupported"),
        "エラーメッセージに '11025' と 'source' と 'unsupported' を含むこと: {msg}"
    );
}

/// サポート外の dst サンプルレートは Err。
#[test]
fn resample_to_mono_rejects_unsupported_destination_rate() {
    let unsupported = SampleRate::from_u32(11025).expect("11025 は SampleRate 型としては有効");
    let err = resample_to_mono(&[0.0; 1024], sr(48000), unsupported, Channels::MONO)
        .expect_err("dst=11025 Hz は本 API では非サポート");
    let msg = err.display().to_string();
    assert!(
        msg.contains("11025") && msg.contains("destination") && msg.contains("unsupported"),
        "エラーメッセージに '11025' と 'destination' と 'unsupported' を含むこと: {msg}"
    );
}

/// 48 kHz → 16 kHz の出力長は `ceil(input_len * 16000 / 48000)` に一致する。
#[test]
fn resample_to_mono_48k_to_16k_output_length() {
    let input_len = 480;
    let input = vec![0.0f32; input_len];
    let out =
        resample_to_mono(&input, sr(48000), sr(16000), Channels::MONO).expect("48k → 16k は Ok");
    let expected = input_len.div_ceil(3); // 480 * 16000 / 48000 = 160
    assert_eq!(out.len(), expected);
}

/// 44.1 kHz → 16 kHz の非整数比リサンプルも出力長式に従う (端数切り上げ)。
#[test]
fn resample_to_mono_44100_to_16k_output_length() {
    let input = vec![0.0f32; 441];
    let out =
        resample_to_mono(&input, sr(44100), sr(16000), Channels::MONO).expect("44.1k → 16k は Ok");
    // ceil(441 * 16000 / 44100) = ceil(160.0) = 160
    assert_eq!(out.len(), 160);
}

/// 16 kHz → 48 kHz (upsample) の出力長は `ceil(input_len * 48000 / 16000)` に一致する。
#[test]
fn resample_to_mono_16k_to_48k_output_length() {
    let input = vec![0.0f32; 160];
    let out =
        resample_to_mono(&input, sr(16000), sr(48000), Channels::MONO).expect("16k → 48k は Ok");
    // ceil(160 * 48000 / 16000) = 480
    assert_eq!(out.len(), 480);
}

/// 単一正弦波の RMS (振幅 1.0 の正弦波は理論値 1/sqrt(2) ≈ 0.7071)。
fn rms(samples: &[f32]) -> f32 {
    let sum_sq: f64 = samples.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
    (sum_sq / samples.len() as f64).sqrt() as f32
}

/// 22050 Hz の 9 kHz 正弦波を 16 kHz にリサンプルすると、9 kHz は 16 kHz 出力の Nyquist (8 kHz)
/// を超えているため anti-aliasing フィルタで大幅に減衰される。
///
/// カットオフが `0.5 / L` のままだと `L=320`, `M=441` で downsample 側の Nyquist を超えたエリアシング
/// 成分を除去できず、折り返し歪みが 7 kHz 付近に発生する。`0.5 / max(L, M)` が正しい設定。
#[test]
fn resample_to_mono_attenuates_above_nyquist_22050_to_16k() {
    let src_hz = 22050;
    let n = 22050 * 2; // 2 秒分
    let signal: Vec<f32> = (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * 9000.0 * i as f32 / src_hz as f32).sin())
        .collect();
    let out = resample_to_mono(&signal, sr(src_hz), sr(16000), Channels::MONO).expect("Ok");
    // 過渡期 (先頭・末尾) を除いた中央部の RMS。
    let start = out.len() / 4;
    let end = out.len() * 3 / 4;
    let out_rms = rms(&out[start..end]);
    // 入力振幅 1.0 の正弦波なら入力 RMS ≈ 0.707。60 dB 減衰なら出力 RMS < 0.707 / 1000 ≈ 0.0007。
    // 十分な減衰を要求 (0.01 以下 = 約 -37 dB 以下) する。カットオフが誤っている場合、
    // エリアシング成分が 7 kHz 付近に折り返して RMS が 0.1 以上になる。
    assert!(
        out_rms < 0.01,
        "22050 Hz の 9 kHz 正弦波は 16 kHz にリサンプル時に反エイリアシングで除去されるべき (out_rms={out_rms})"
    );
}

/// 22050 Hz の 7 kHz 正弦波を 16 kHz にリサンプルすると、通過帯域内なので概ね振幅を保つ。
///
/// このテストは上の反エイリアシングテストの対照として、通過帯域まで削られていないことを確認する。
#[test]
fn resample_to_mono_passes_below_nyquist_22050_to_16k() {
    let src_hz = 22050;
    let n = 22050 * 2;
    let signal: Vec<f32> = (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * 7000.0 * i as f32 / src_hz as f32).sin())
        .collect();
    let out = resample_to_mono(&signal, sr(src_hz), sr(16000), Channels::MONO).expect("Ok");
    let start = out.len() / 4;
    let end = out.len() * 3 / 4;
    let out_rms = rms(&out[start..end]);
    // 入力 RMS ≈ 0.707、通過帯域なので出力もほぼ同じレベル (0.5 以上を要求)。
    assert!(
        out_rms > 0.5,
        "22050 Hz の 7 kHz 正弦波は 16 kHz にリサンプルしても通過帯域で保持されるべき (out_rms={out_rms})"
    );
}
