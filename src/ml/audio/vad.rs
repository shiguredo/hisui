//! Silero VAD による発話区間検出のゲート。
//!
//! `VadGate::feed` に 16 kHz f32 モノラル PCM を任意長ずつ渡すと、内部の `AudioChunkBuffer` に貯めて
//! 512 サンプル境界で `SileroVad::chunk_probability` を実行し、閾値ゲート + `min_silence` /
//! `min_speech` の集約ロジックで発話区間 `SpeechSegment` (16 kHz サンプル通し番号) を確定して返す。

use std::num::NonZeroUsize;
use std::time::Duration;

use crate::probability::Probability;

use super::buffer::AudioChunkBuffer;
use super::config::VadConfig;
use super::silero_vad::SileroVad;

/// Silero VAD の 1 チャンクのサンプル数。`SileroVad::chunk_probability` が要求する固定値と一致させる。
const CHUNK_SAMPLES: NonZeroUsize = NonZeroUsize::new(512).expect("CHUNK_SAMPLES > 0");

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
/// - `Trailing`: 発話後の無音を数え始めており、`min_silence` 到達で確定候補となる。
#[derive(Debug, PartialEq)]
enum State {
    Idle,
    InSpeech(SpeechInProgress),
    Trailing {
        speech: SpeechInProgress,
        silence_samples: u64,
    },
}

/// 集約中の発話状態。
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// - 発話継続中や `min_silence` 未達の区間は Self 内に保持し、次の `feed` または `flush` で確定する。
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
        let transition = TransitionConfig {
            threshold: self.config.threshold,
            min_silence_samples: duration_to_samples(self.config.min_silence),
            min_speech_samples: duration_to_samples(self.config.min_speech),
            chunk_samples: CHUNK_SAMPLES.get() as u64,
        };
        while let Some(chunk) = self.buffer.take_chunk() {
            let probability = self.silero.chunk_probability(&chunk)?;
            let chunk_start = self.sample_count;
            let chunk_end = self.sample_count + transition.chunk_samples;
            self.sample_count = chunk_end;
            let current = std::mem::replace(&mut self.state, State::Idle);
            self.state = advance_state(
                current,
                probability,
                chunk_start,
                chunk_end,
                &transition,
                &mut results,
            );
        }
        Ok(results)
    }

    /// 現在確定していない segment を強制確定して返す (ストリーム終端で呼ぶ)。
    ///
    /// 発話中の場合、`min_speech` を満たしていれば SpeechSegment として確定、満たしていなければ破棄する。
    pub fn flush(&mut self) -> crate::Result<Vec<SpeechSegment>> {
        let mut results = Vec::new();
        let min_speech_samples = duration_to_samples(self.config.min_speech);
        let state = std::mem::replace(&mut self.state, State::Idle);
        match state {
            State::Idle => {}
            State::InSpeech(speech) | State::Trailing { speech, .. } => {
                if let Some(segment) = finalize_speech(&speech, min_speech_samples) {
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
}

/// `advance_state` に渡す閾値・サンプル数のパラメタ束。
///
/// `advance_state` の引数を減らして clippy::too_many_arguments を回避する目的で導入している。
/// VadGate::feed から `VadConfig` と `CHUNK_SAMPLES` を展開して組み立てる。
struct TransitionConfig {
    threshold: Probability,
    min_silence_samples: u64,
    min_speech_samples: u64,
    chunk_samples: u64,
}

/// 1 チャンクぶんの確率と現在状態から次状態を計算する pure function。
///
/// SileroVad 非依存 (probability を数値として受け取る) にすることで、状態遷移の単体テストを
/// 決定論的に行えるようにしている。
fn advance_state(
    state: State,
    probability: f32,
    chunk_start: u64,
    chunk_end: u64,
    config: &TransitionConfig,
    results: &mut Vec<SpeechSegment>,
) -> State {
    let is_speech = probability >= config.threshold.get();

    match state {
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
                    silence_samples: config.chunk_samples,
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
                let silence_samples = silence_samples + config.chunk_samples;
                if silence_samples >= config.min_silence_samples {
                    if let Some(segment) = finalize_speech(&speech, config.min_speech_samples) {
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
    }
}

/// 集約中の発話状態から `min_speech_samples` 以上の長さのぶんだけを SpeechSegment として確定する。
///
/// 未達なら None を返す (破棄)。
fn finalize_speech(speech: &SpeechInProgress, min_speech_samples: u64) -> Option<SpeechSegment> {
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

/// Duration を 16 kHz サンプル数に変換する。
///
/// 1 サンプル = 62_500 ns (16 kHz は 1_000_000_000 の約数) なので丸め誤差はない。
/// Duration が `u64::MAX / 1000 秒` (実用上の想定音声長を遥かに超える) を超えると
/// `as u64` cast で切り捨てが発生するが、通常運用では発生しない前提。
fn duration_to_samples(d: Duration) -> u64 {
    (d.as_nanos() as u64) / 62_500
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用 1 チャンクのサンプル数 (実装の CHUNK_SAMPLES と同じ)。
    const CHUNK: u64 = 512;

    /// テスト用の閾値。
    const THRESHOLD: f32 = 0.5;

    /// `THRESHOLD` を `Probability` にラップした値。`Probability::new` が const fn なので
    /// const 文脈で組み立てられる。
    const THRESHOLD_PROB: Probability = match Probability::new(THRESHOLD) {
        Some(p) => p,
        None => unreachable!(),
    };

    /// テスト用の min_silence_samples (3 チャンク分)。
    /// 「shorter than 3 chunks は未達 / 3 チャンク以上で到達」の境界を試せる大きさに設定。
    const MIN_SILENCE: u64 = 3 * CHUNK;

    /// テスト用の min_speech_samples (3 チャンク分)。
    /// 「1 チャンクだけの発話は min_speech 未達 / 3 チャンクぶんは到達」を試せる大きさに設定。
    const MIN_SPEECH: u64 = 3 * CHUNK;

    /// SpeechInProgress を組み立てる補助関数。
    fn speech(start: u64, last_end: u64, max_prob: f32) -> SpeechInProgress {
        SpeechInProgress {
            start_sample: start,
            last_speech_end_sample: last_end,
            max_probability: max_prob,
        }
    }

    /// advance_state を固定パラメタで呼び出す補助関数。
    fn step(state: State, probability: f32, chunk_start: u64) -> (State, Vec<SpeechSegment>) {
        let chunk_end = chunk_start + CHUNK;
        let mut results = Vec::new();
        let config = TransitionConfig {
            threshold: THRESHOLD_PROB,
            min_silence_samples: MIN_SILENCE,
            min_speech_samples: MIN_SPEECH,
            chunk_samples: CHUNK,
        };
        let next = advance_state(
            state,
            probability,
            chunk_start,
            chunk_end,
            &config,
            &mut results,
        );
        (next, results)
    }

    /// Idle 状態で probability < threshold なら Idle のまま、確定 segment は無し。
    #[test]
    fn idle_stays_idle_on_silence() {
        let (next, results) = step(State::Idle, 0.2, 0);
        assert_eq!(next, State::Idle);
        assert!(results.is_empty(), "無音のみでは segment は生成されない");
    }

    /// Idle 状態で probability >= threshold なら InSpeech に遷移し、開始位置と max_prob が記録される。
    #[test]
    fn idle_transitions_to_in_speech_on_speech_start() {
        let (next, results) = step(State::Idle, 0.9, 0);
        assert_eq!(next, State::InSpeech(speech(0, CHUNK, 0.9)));
        assert!(
            results.is_empty(),
            "発話開始直後は segment がまだ確定しない"
        );
    }

    /// InSpeech 継続で last_speech_end_sample と max_probability が正しく更新される。
    #[test]
    fn in_speech_updates_last_end_and_tracks_max_probability() {
        // 1 チャンク目: max = 0.7、last_end = CHUNK
        let state = State::InSpeech(speech(0, CHUNK, 0.7));
        // 2 チャンク目 (probability 0.85 > 0.7) で max 更新
        let (next, results) = step(state, 0.85, CHUNK);
        assert_eq!(next, State::InSpeech(speech(0, 2 * CHUNK, 0.85)));
        assert!(results.is_empty());

        // 3 チャンク目 (probability 0.6 < 0.85) は last_end のみ更新、max はそのまま
        let (next, results) = step(next, 0.6, 2 * CHUNK);
        assert_eq!(next, State::InSpeech(speech(0, 3 * CHUNK, 0.85)));
        assert!(results.is_empty());
    }

    /// InSpeech から probability < threshold で Trailing に遷移する (silence_samples = 1 chunk)。
    #[test]
    fn in_speech_transitions_to_trailing_on_silence_chunk() {
        let state = State::InSpeech(speech(0, CHUNK, 0.9));
        let (next, results) = step(state, 0.2, CHUNK);
        assert_eq!(
            next,
            State::Trailing {
                speech: speech(0, CHUNK, 0.9),
                silence_samples: CHUNK,
            }
        );
        assert!(
            results.is_empty(),
            "min_silence 未達では segment は確定しない"
        );
    }

    /// Trailing 中に probability >= threshold で InSpeech に復帰し、silence カウントはリセットされる。
    #[test]
    fn trailing_returns_to_in_speech_on_speech_resume() {
        let state = State::Trailing {
            speech: speech(0, CHUNK, 0.7),
            silence_samples: CHUNK,
        };
        // probability = 0.8 > 0.7 で max 更新、last_end も更新
        let (next, results) = step(state, 0.8, 2 * CHUNK);
        assert_eq!(next, State::InSpeech(speech(0, 3 * CHUNK, 0.8)));
        assert!(results.is_empty());
    }

    /// Trailing 中に無音を追加しても min_silence 未達なら Trailing 継続 (silence_samples が加算)。
    #[test]
    fn trailing_accumulates_silence_when_min_silence_not_reached() {
        let state = State::Trailing {
            speech: speech(0, CHUNK, 0.9),
            silence_samples: CHUNK, // 1 chunk 蓄積済み
        };
        // 追加 1 chunk = 2 * CHUNK < MIN_SILENCE(3 * CHUNK) → Trailing 継続
        let (next, results) = step(state, 0.2, 2 * CHUNK);
        assert_eq!(
            next,
            State::Trailing {
                speech: speech(0, CHUNK, 0.9),
                silence_samples: 2 * CHUNK,
            }
        );
        assert!(results.is_empty());
    }

    /// Trailing 中に min_silence 到達 + 発話長が min_speech 到達で SpeechSegment が確定し Idle に戻る。
    #[test]
    fn trailing_finalizes_segment_when_min_silence_reached_and_min_speech_met() {
        // 発話は 3 chunks 分あり: length = 3 * CHUNK = MIN_SPEECH → 到達
        let state = State::Trailing {
            speech: speech(0, 3 * CHUNK, 0.85),
            silence_samples: 2 * CHUNK, // 2 chunks 蓄積済み
        };
        // 追加 1 chunk = 3 * CHUNK = MIN_SILENCE → 到達
        let (next, results) = step(state, 0.2, 5 * CHUNK);
        assert_eq!(next, State::Idle);
        assert_eq!(results.len(), 1, "segment が 1 つ確定するはず");
        assert_eq!(
            results[0],
            SpeechSegment {
                start_sample: 0,
                end_sample: 3 * CHUNK,
                max_probability: 0.85,
            }
        );
    }

    /// Trailing 中に min_silence 到達だが発話長が min_speech 未達なら segment は破棄され Idle に戻る。
    #[test]
    fn trailing_drops_segment_when_min_silence_reached_but_min_speech_not_met() {
        // 発話は 1 chunk 分のみ: length = CHUNK < MIN_SPEECH(3 * CHUNK) → 未達で破棄
        let state = State::Trailing {
            speech: speech(0, CHUNK, 0.9),
            silence_samples: 2 * CHUNK,
        };
        let (next, results) = step(state, 0.2, 3 * CHUNK);
        assert_eq!(next, State::Idle);
        assert!(
            results.is_empty(),
            "短すぎる発話は min_speech 未達で破棄される"
        );
    }

    /// finalize_speech は length < min_speech_samples なら None を返す (破棄)。
    #[test]
    fn finalize_speech_returns_none_when_min_speech_not_met() {
        let s = speech(0, CHUNK, 0.9);
        assert_eq!(finalize_speech(&s, MIN_SPEECH), None);
    }

    /// finalize_speech は length >= min_speech_samples なら SpeechSegment を返す。
    #[test]
    fn finalize_speech_returns_segment_when_min_speech_met() {
        let s = speech(0, 3 * CHUNK, 0.85);
        assert_eq!(
            finalize_speech(&s, MIN_SPEECH),
            Some(SpeechSegment {
                start_sample: 0,
                end_sample: 3 * CHUNK,
                max_probability: 0.85,
            })
        );
    }
}
