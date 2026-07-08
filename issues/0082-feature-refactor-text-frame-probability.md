# TextFrame の no_speech_prob / avg_logprob を Probability / LogProbability に置き換える

- Priority: Medium
- Created: 2026-07-08
- Completed:
- Model: Opus 4.8
- Branch: feature/refactor-text-frame-probability
- Polished:

## 目的

`TextFrame` の `no_speech_prob: Option<f32>` と `avg_logprob: Option<f32>` を、それぞれ `Option<Probability>` / `Option<LogProbability>` に置き換え、確率値の型による範囲保証を pipeline 全経路に貫通させる。

## 優先度根拠

Medium。実バグではなく型設計の統一。 0062 で decode 境界に導入した `Probability` / `LogProbability` の恩恵が、`TextFrame` まで届かないと分断されるため後追い改修する。緊急度は高くないが、今揃えないと将来 text producer が増えた際に、生の f32 が伝播し続ける負債になる。

## 現状

- `src/probability.rs` に `Probability` (`[0.0, 1.0]`) と `LogProbability` (`(-∞, 0]`) が定義済み。内部型はいずれも f64 。 0062 で Whisper の decode 境界に導入した。
- `src/ml/audio/whisper/decode.rs` の `WhisperDecodedChunk` は `no_speech_prob: Probability` / `avg_logprob: LogProbability` を保持。
- `src/ml/audio/whisper.rs` の `WhisperTranscription` は詰め替え時に `.get() as f32` で素の f32 に落としている。以降 `TranscriptResult` (`src/ml/audio/transcription_service.rs`) → `TextFrame` (`src/text.rs`) までずっと f32 のまま流れる。
- `TextFrame.no_speech_prob: Option<f32>` / `TextFrame.avg_logprob: Option<f32>` は Whisper 以外の text producer も想定した汎用型として `Option` になっているが、`Option` を保ちつつ内側の f32 を型付きに置き換えれば汎用性と型保証を両立できる。

## 設計方針

- `TextFrame` の 2 フィールドを `Option<Probability>` / `Option<LogProbability>` に置き換える。
- 途中のレイヤー (`WhisperTranscription` , `TranscriptResult`) も `Probability` / `LogProbability` に揃え、`.get() as f32` の narrowing を廃止する。
- text producer 側は境界で `Probability::new(...)` / `LogProbability::new(...)` を通す (Whisper 経路は既に通しているため詰め替えの型を差し替えるだけ)。範囲外や NaN は `Err` として返す。
- 外部シリアライズ (JSON など) が必要な箇所は `.get()` で内部の f64 を取り出す。

## 完了条件

- `TextFrame.no_speech_prob` / `TextFrame.avg_logprob` が `Option<Probability>` / `Option<LogProbability>` になっている。
- Whisper 経路 (`WhisperTranscription` → `TranscriptResult` → `TextFrame`) の詰め替えが `.get() as f32` を挟まず型付きのまま伝播する。
- `cargo clippy --features candle --all-targets -p hisui -- --deny warnings` と `cargo test --features candle -p hisui` が green 。
- 既定 feature ビルド (`cargo check -p hisui` , `cargo test --workspace`) も green 。

## 解決方法

- `src/text.rs`: `no_speech_prob: Option<f32>` → `Option<Probability>` 、 `avg_logprob: Option<f32>` → `Option<LogProbability>` に変更。 doc コメントも型に合わせて更新する。
- `src/ml/audio/transcription_service.rs`: `TranscriptResult` の 2 フィールドを同様に置き換える。
- `src/ml/audio/whisper.rs`: `WhisperTranscription` の 2 フィールドを同様に置き換え、詰め替え時の `.get() as f32` を撤去する。
- `src/ml/audio/transcription_processor.rs`: `TranscriptResult` → `TextFrame` への詰め替え箇所を型合わせして `Some(result.no_speech_prob)` / `Some(result.avg_logprob)` に単純化する。
- 他の text producer が現れた場合は境界で `Probability::new(...)` / `LogProbability::new(...)` を通す形に統一する。
