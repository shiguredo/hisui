//! 任意サンプルレート・任意チャンネル数の PCM を、指定サンプルレートのモノラル f32 に変換する。
//!
//! polyphase FIR (Kaiser 窓、β = 8.6) で 8000 / 16000 / 22050 / 24000 / 32000 / 44100 / 48000 Hz
//! 間の変換をサポートする。ステレオはチャンネル平均でモノラルにダウンミックスする。
//! Kaiser 窓の設計に必要な第 0 種変形 Bessel 関数 `I0` は Rust std / libm に存在しないため
//! `bessel_i0` に級数展開の自前実装を置く。
//!
//! # `audio::converter` との使い分け
//!
//! `audio::converter::AudioConverter` にも別のリサンプラが同梱されている。以下の性格が異なる。
//!
//! - `converter`: i16 interleaved 前提、線形補間、ストリーミング (フレーム境界で prev 持ち回し)、
//!   同一チャンネル数の interleaved 出力。mixer / encoder への入力揃えに使う。
//! - 本モジュール `resample_to_mono`: f32 前提、polyphase FIR (高品質)、バッチ (1 バッファ完結、
//!   prev 持ち回し不要)、モノラル固定出力。ML 前処理・オフライン高品質変換に使う。
//!
//! リアルタイム / interleaved / i16 が必要なら `converter` を、バッチ / 高品質 / f32 mono が
//! 必要なら本モジュールを選ぶ。

use crate::audio::{Channels, SampleRate};
use crate::error::Error;

/// 対応するサンプルレート一覧 (src / dst 両方に共通)。
const SUPPORTED_HZ: &[u32] = &[8000, 16000, 22050, 24000, 32000, 44100, 48000];

/// polyphase FIR のプロトタイプフィルタ長のベースタップ数。
const BASE_TAPS: usize = 64;

/// Kaiser 窓の β パラメータ。60 dB の阻止帯域減衰を目安に選択。
const KAISER_BETA: f32 = 8.6;

/// 任意サンプルレート・任意チャンネル数の PCM を、指定サンプルレートのモノラル f32 に変換する。
///
/// 空スライスは `Ok(vec![])` を返す。ステレオはチャンネル平均でダウンミックスする。
/// `src_hz` / `dst_hz` はいずれも `SUPPORTED_HZ` に含まれる必要がある。
pub fn resample_to_mono(
    pcm: &[f32],
    src_hz: SampleRate,
    dst_hz: SampleRate,
    channels: Channels,
) -> crate::Result<Vec<f32>> {
    let src = src_hz.get();
    let dst = dst_hz.get();
    if !SUPPORTED_HZ.contains(&src) {
        return Err(Error::new(format!(
            "unsupported source sample rate for resample_to_mono: {src} Hz (supported: {SUPPORTED_HZ:?})"
        )));
    }
    if !SUPPORTED_HZ.contains(&dst) {
        return Err(Error::new(format!(
            "unsupported destination sample rate for resample_to_mono: {dst} Hz (supported: {SUPPORTED_HZ:?})"
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
            "unsupported channel count for resample_to_mono: {}",
            channels.get()
        )));
    };

    if mono.is_empty() {
        return Ok(Vec::new());
    }

    Ok(polyphase_resample(&mono, src, dst))
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
    let proto = prototype_filter(n_taps, l, m);

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
/// - 正規化カットオフ `fc = 0.5 / max(upsample_factor, downsample_factor)`
///   (アップサンプル後の Nyquist とダウンサンプル後の Nyquist のうち低い方に合わせて反エイリアシング)
/// - Kaiser 窓 β = `KAISER_BETA`
/// - polyphase 分解時に downsample で `1/L` になる分を先取りしてゲイン `L` を乗じておく
fn prototype_filter(n_taps: usize, upsample_factor: usize, downsample_factor: usize) -> Vec<f32> {
    let fc = 0.5f32 / upsample_factor.max(downsample_factor) as f32;
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
fn bessel_i0(x: f32) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
