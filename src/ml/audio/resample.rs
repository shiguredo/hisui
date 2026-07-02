//! 任意サンプルレート・任意チャンネル数の PCM を 16 kHz モノラル f32 に変換する。
//!
//! polyphase FIR (Kaiser 窓、β = 8.6) で 8000 / 16000 / 22050 / 24000 / 32000 / 44100 / 48000 Hz
//! から 16 kHz に変換する。ステレオはチャンネル平均でモノラルにダウンミックスする。
//! Kaiser 窓の設計に必要な第 0 種変形 Bessel 関数 `I0` は Rust std / libm に存在しないため
//! `bessel_i0` に級数展開の自前実装を置く。

use crate::audio::{AudioFormat, AudioFrame, Channels, SampleRate};
use crate::error::Error;

/// 変換先サンプルレート (Hz)。
const DST_HZ: u32 = 16000;

/// 対応する変換元サンプルレート一覧。
const SUPPORTED_SRC_HZ: &[u32] = &[8000, 16000, 22050, 24000, 32000, 44100, 48000];

/// polyphase FIR のプロトタイプフィルタ長のベースタップ数。
const BASE_TAPS: usize = 64;

/// Kaiser 窓の β パラメータ。60 dB の阻止帯域減衰を目安に選択。
const KAISER_BETA: f32 = 8.6;

/// 任意サンプルレート・任意チャンネル数の PCM を 16 kHz モノラル f32 に変換する。
///
/// 空スライスは `Ok(vec![])` を返す。ステレオはチャンネル平均でダウンミックスする。
pub fn resample_to_16k_mono(
    pcm: &[f32],
    src_hz: SampleRate,
    channels: Channels,
) -> crate::Result<Vec<f32>> {
    let src = src_hz.get();
    if !SUPPORTED_SRC_HZ.contains(&src) {
        return Err(Error::new(format!(
            "unsupported source sample rate for resample_to_16k_mono: {src} Hz (supported: {SUPPORTED_SRC_HZ:?})"
        )));
    }

    // 先にチャンネル平均でモノラル化する。
    let mono: Vec<f32> = if channels.is_mono() {
        pcm.to_vec()
    } else if channels.is_stereo() {
        if !pcm.len().is_multiple_of(2) {
            return Err(Error::new(format!(
                "stereo PCM length must be even, got {}",
                pcm.len()
            )));
        }
        pcm.chunks_exact(2).map(|c| (c[0] + c[1]) * 0.5).collect()
    } else {
        return Err(Error::new(format!(
            "unsupported channel count for resample_to_16k_mono: {}",
            channels.get()
        )));
    };

    if mono.is_empty() {
        return Ok(Vec::new());
    }

    Ok(polyphase_resample(&mono, src, DST_HZ))
}

/// `AudioFrame` (`AudioFormat::I16Be` 前提) を 16 kHz モノラル f32 に変換する。
///
/// I16Be のバイト列を `i16::from_be_bytes` で復号し、`/ 32768.0` で `[-1.0, 1.0)` に正規化してから
/// `resample_to_16k_mono` に渡す。`I16Be` 以外のフォーマットは `Err`。
pub fn audio_frame_to_16k_mono(frame: &AudioFrame) -> crate::Result<Vec<f32>> {
    if frame.format != AudioFormat::I16Be {
        return Err(Error::new(format!(
            "audio_frame_to_16k_mono expects I16Be format, got {}",
            frame.format
        )));
    }
    if !frame.data.len().is_multiple_of(2) {
        return Err(Error::new(format!(
            "I16Be data length must be even, got {}",
            frame.data.len()
        )));
    }
    let samples: Vec<f32> = frame
        .data
        .chunks_exact(2)
        .map(|c| f32::from(i16::from_be_bytes([c[0], c[1]])) / 32768.0)
        .collect();
    resample_to_16k_mono(&samples, frame.sample_rate, frame.channels)
}

/// polyphase FIR による rational resampler。
///
/// `L / M` を gcd で簡約してから、長さ `L * BASE_TAPS` のプロトタイプフィルタを Kaiser 窓で設計し、
/// `L` 個のサブフィルタに分解して出力を生成する。
fn polyphase_resample(input: &[f32], src_hz: u32, dst_hz: u32) -> Vec<f32> {
    let g = gcd(dst_hz, src_hz);
    let l = (dst_hz / g) as usize;
    let m = (src_hz / g) as usize;

    // src_hz == dst_hz なら l = m = 1、フィルタ不要でそのままコピーする。
    if l == 1 && m == 1 {
        return input.to_vec();
    }

    let n_taps_per_phase = BASE_TAPS;
    let n_taps = l * n_taps_per_phase;
    let proto = prototype_filter(n_taps, l);

    // サブフィルタに分解: subfilters[phase][k] = proto[k * l + phase]。
    let mut subfilters: Vec<Vec<f32>> = Vec::with_capacity(l);
    for phase in 0..l {
        let sub: Vec<f32> = (0..n_taps_per_phase)
            .map(|k| proto[k * l + phase])
            .collect();
        subfilters.push(sub);
    }

    // 出力長 = ceil(input_len * dst_hz / src_hz)。
    let output_len = input
        .len()
        .saturating_mul(dst_hz as usize)
        .div_ceil(src_hz as usize);
    let mut output = Vec::with_capacity(output_len);

    for n in 0..output_len {
        // upsampled index = m * n、その中の position = idx / l、subfilter phase = idx % l。
        let idx = m * n;
        let input_idx = idx / l;
        let phase = idx % l;
        let sub = &subfilters[phase];
        let mut acc = 0.0f32;
        for (k, coef) in sub.iter().enumerate() {
            if k > input_idx {
                break;
            }
            let src_idx = input_idx - k;
            if src_idx >= input.len() {
                continue;
            }
            acc += coef * input[src_idx];
        }
        output.push(acc);
    }

    output
}

/// Kaiser 窓を掛けた低域通過 FIR のプロトタイプフィルタを生成する。
///
/// - タップ数 `n_taps`
/// - 正規化カットオフ `fc = 0.5 / upsample_factor` (アップサンプル後の Nyquist から見た 16 kHz 相当の 8 kHz)
/// - Kaiser 窓 β = `KAISER_BETA`
/// - polyphase 分解時に downsample で `1/L` になる分を先取りしてゲイン `L` を乗じておく
fn prototype_filter(n_taps: usize, upsample_factor: usize) -> Vec<f32> {
    let fc = 0.5f32 / upsample_factor as f32;
    let window = kaiser_window(n_taps);
    let center = (n_taps as f32 - 1.0) * 0.5;

    (0..n_taps)
        .map(|i| {
            let n = i as f32 - center;
            let sinc = if n == 0.0 {
                2.0 * fc
            } else {
                let x = 2.0 * std::f32::consts::PI * fc * n;
                (2.0 * fc * x.sin()) / x
            };
            sinc * window[i] * upsample_factor as f32
        })
        .collect()
}

/// Kaiser 窓を生成する (β = `KAISER_BETA`)。
fn kaiser_window(n_taps: usize) -> Vec<f32> {
    if n_taps == 0 {
        return Vec::new();
    }
    if n_taps == 1 {
        return vec![1.0];
    }
    let m = (n_taps as f32 - 1.0) * 0.5;
    let denom = bessel_i0(KAISER_BETA);
    (0..n_taps)
        .map(|i| {
            let x = (i as f32 - m) / m;
            let arg = KAISER_BETA * (1.0 - x * x).max(0.0).sqrt();
            bessel_i0(arg) / denom
        })
        .collect()
}

/// 第 0 種変形 Bessel 関数 `I0(x)` を級数展開で計算する。
///
/// 逐次更新 `t_{n+1} = t_n * (x/2)^2 / (n+1)^2` (t_0 = 1) で総和を進め、
/// 前項比が `1e-14` を下回ったら打ち切る。安全上限 100 項。
///
/// f32 で計算すると `I0(8.6) ≈ 895` オーダーでの相対精度が仮数部長 (~1e-7) に律速されて
/// SciPy 等の参照値と一致しない。内部を f64 で計算してから f32 に丸めることで、Kaiser 窓
/// 設計時の相対誤差を 1e-6 未満に抑える。
pub fn bessel_i0(x: f32) -> f32 {
    let x = f64::from(x);
    let half_sq = (x * 0.5).powi(2);
    let mut sum = 1.0f64;
    let mut term = 1.0f64;
    for n in 1..=100 {
        term = term * half_sq / (f64::from(n) * f64::from(n));
        sum += term;
        if term.abs() < 1e-14 * sum.abs() {
            break;
        }
    }
    sum as f32
}

/// 32 bit 符号無し整数の最大公約数。
fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}
