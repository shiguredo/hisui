//! Silero VAD による発話区間検出のゲート。
//!
//! `VadGate::feed` に 16 kHz f32 モノラル PCM を任意長ずつ渡すと、内部の `AudioChunkBuffer` に貯めて
//! 512 サンプル境界で `SileroVad::chunk_probability` を実行し、閾値ゲート + `min_silence_ms` /
//! `min_speech_ms` の集約ロジックで発話区間 `SpeechSegment` (16 kHz サンプル通し番号) を確定して返す。

use std::num::NonZeroUsize;
use std::time::Duration;

use super::buffer::AudioChunkBuffer;
use super::config::VadConfig;
use super::silero_vad::SileroVad;

/// Silero VAD の 1 チャンクのサンプル数。`SileroVad::chunk_probability` が要求する固定値と一致させる。
const CHUNK_SAMPLES: NonZeroUsize = NonZeroUsize::new(512).expect("CHUNK_SAMPLES > 0");

/// VadGate が動作するサンプルレート (Silero VAD v5 の 16 kHz)。
const SAMPLE_RATE_HZ: u64 = 16000;

/// 検出された発話区間。
///
/// `start_sample` / `end_sample` は `VadGate::new` / `reset` からの 16 kHz サンプル通し番号
/// (inclusive start、exclusive end、Rust Range 慣習)。呼び出し側が `feed` に流し込んだ 16 kHz PCM を
/// 全区間保持し、`start_sample..end_sample` で slice する責務を負う (VadGate は PCM を保持しない)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechSegment {
    pub start_sample: u64,
    pub end_sample: u64,
    pub max_probability: f32,
}

impl SpeechSegment {
    /// 16 kHz 換算での発話開始時刻を `Duration` で返す。
    ///
    /// 1 サンプル = 62_500 ns (16000 は 1_000_000_000 の約数) なので丸め誤差ゼロ。
    /// `u64::MAX / 62_500 ≈ 9370 年`ぶんまでオーバーフロー無しに扱える。
    pub fn start_time(&self) -> Duration {
        Duration::from_nanos(self.start_sample * 62_500)
    }

    /// 16 kHz 換算での発話終了時刻を `Duration` で返す。
    pub fn end_time(&self) -> Duration {
        Duration::from_nanos(self.end_sample * 62_500)
    }
}

/// VadGate の内部状態機。
///
/// - `Idle`: 開始直後 or 直前 SpeechSegment 確定直後 (発話・無音のいずれもカウントしていない)。
/// - `InSpeech`: 直近チャンクが閾値超えで、発話区間として集約中。
/// - `Trailing`: 発話後の無音を数え始めており、`min_silence_ms` 到達で確定候補となる。
enum State {
    Idle,
    InSpeech(SpeechInProgress),
    Trailing {
        speech: SpeechInProgress,
        silence_samples: u64,
    },
}

/// 集約中の発話状態。
#[derive(Debug, Clone, Copy)]
struct SpeechInProgress {
    start_sample: u64,
    last_speech_end_sample: u64,
    max_probability: f32,
}

/// Silero VAD ラッパー。発話区間の集約を担う。
///
/// 1 つの `VadGate` は 1 つの track に紐付ける (`SileroVad` の内部 state / context と `VadGate` の
/// 通し番号を別 track と混ぜると意味を失うため)。
pub struct VadGate {
    silero: SileroVad,
    buffer: AudioChunkBuffer,
    config: VadConfig,
    /// `VadGate::new` / `reset` 以降に処理した 16 kHz サンプル数の累計。
    sample_count: u64,
    state: State,
}

impl VadGate {
    pub fn new(silero: SileroVad, config: VadConfig) -> Self {
        Self {
            silero,
            buffer: AudioChunkBuffer::new(CHUNK_SAMPLES),
            config,
            sample_count: 0,
            state: State::Idle,
        }
    }

    /// 16 kHz f32 モノラル PCM を任意長受け取り、確定した SpeechSegment を start_sample 昇順で返す。
    ///
    /// - 発話継続中や `min_silence_ms` 未達の区間は Self 内に保持し、次の `feed` または `flush` で確定する。
    /// - 512 サンプル境界に満たない残余は Self 内 buffer に貯めて次の feed に持ち越す。
    /// - 1 回の feed で複数 SpeechSegment を返し得る (feed 内で発話 → 無音 → 発話が複数回起きた場合)。
    ///
    /// `SileroVad::chunk_probability` が Err を返した場合、内部 buffer から drain 済みの 1 チャンクぶんの
    /// PCM は失われ、通し番号 (`sample_count`) も進まない。この状態のまま `feed` / `flush` を続けると
    /// SpeechSegment の通し番号が本来より 512 サンプルぶん前にずれる。Err を受け取った呼び出し側は
    /// `reset` して作り直すこと (candle 内部エラーは通常運用では発生しない前提)。
    pub fn feed(&mut self, samples: &[f32]) -> crate::Result<Vec<SpeechSegment>> {
        self.buffer.push(samples);
        let mut results = Vec::new();
        while let Some(chunk) = self.buffer.take_chunk() {
            let probability = self.silero.chunk_probability(&chunk)?;
            let chunk_start = self.sample_count;
            let chunk_end = self.sample_count + CHUNK_SAMPLES.get() as u64;
            self.sample_count = chunk_end;
            self.advance_state(probability, chunk_start, chunk_end, &mut results);
        }
        Ok(results)
    }

    /// 現在確定していない segment を強制確定して返す (ストリーム終端で呼ぶ)。
    ///
    /// 発話中の場合、`min_speech_ms` を満たしていれば SpeechSegment として確定、満たしていなければ破棄する。
    pub fn flush(&mut self) -> crate::Result<Vec<SpeechSegment>> {
        let mut results = Vec::new();
        let state = std::mem::replace(&mut self.state, State::Idle);
        match state {
            State::Idle => {}
            State::InSpeech(speech) | State::Trailing { speech, .. } => {
                if let Some(segment) = self.finalize_speech(&speech) {
                    results.push(segment);
                }
            }
        }
        Ok(results)
    }

    /// 通し番号を 0 に戻し、`SileroVad::reset` を呼ぶ。別 track / 別ストリーム切り替え時に使う。
    ///
    /// buffer の残余サンプルは破棄する (別 track の PCM と混ぜないため)。
    pub fn reset(&mut self) {
        self.silero.reset();
        self.buffer = AudioChunkBuffer::new(CHUNK_SAMPLES);
        self.sample_count = 0;
        self.state = State::Idle;
    }

    /// 1 チャンクぶんの確率を受けて内部状態を進める。
    fn advance_state(
        &mut self,
        probability: f32,
        chunk_start: u64,
        chunk_end: u64,
        results: &mut Vec<SpeechSegment>,
    ) {
        let is_speech = probability >= self.config.threshold;
        let min_silence_samples = ms_to_samples(self.config.min_silence_ms);

        let current = std::mem::replace(&mut self.state, State::Idle);
        self.state = match current {
            State::Idle => {
                if is_speech {
                    State::InSpeech(SpeechInProgress {
                        start_sample: chunk_start,
                        last_speech_end_sample: chunk_end,
                        max_probability: probability,
                    })
                } else {
                    State::Idle
                }
            }
            State::InSpeech(mut speech) => {
                if is_speech {
                    speech.last_speech_end_sample = chunk_end;
                    if probability > speech.max_probability {
                        speech.max_probability = probability;
                    }
                    State::InSpeech(speech)
                } else {
                    State::Trailing {
                        speech,
                        silence_samples: CHUNK_SAMPLES.get() as u64,
                    }
                }
            }
            State::Trailing {
                mut speech,
                silence_samples,
            } => {
                if is_speech {
                    speech.last_speech_end_sample = chunk_end;
                    if probability > speech.max_probability {
                        speech.max_probability = probability;
                    }
                    State::InSpeech(speech)
                } else {
                    let silence_samples = silence_samples + CHUNK_SAMPLES.get() as u64;
                    if silence_samples >= min_silence_samples {
                        if let Some(segment) = self.finalize_speech(&speech) {
                            results.push(segment);
                        }
                        State::Idle
                    } else {
                        State::Trailing {
                            speech,
                            silence_samples,
                        }
                    }
                }
            }
        };
    }

    /// 集約中の発話状態から `min_speech_ms` 到達分だけを SpeechSegment として確定する。
    ///
    /// 未達なら None を返す (破棄)。
    fn finalize_speech(&self, speech: &SpeechInProgress) -> Option<SpeechSegment> {
        let min_speech_samples = ms_to_samples(self.config.min_speech_ms);
        let length = speech.last_speech_end_sample - speech.start_sample;
        if length < min_speech_samples {
            return None;
        }
        Some(SpeechSegment {
            start_sample: speech.start_sample,
            end_sample: speech.last_speech_end_sample,
            max_probability: speech.max_probability,
        })
    }
}

/// ミリ秒を 16 kHz サンプル数に変換する。
fn ms_to_samples(ms: u32) -> u64 {
    u64::from(ms) * SAMPLE_RATE_HZ / 1000
}
