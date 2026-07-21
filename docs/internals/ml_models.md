# `ml/audio` のモデル型設計

## 概要

`src/ml/audio/` の下には現在 2 種類の ML モデルが載っている: **Silero VAD** (発話区間検出) と
**Whisper** (音声文字起こし)。両者はモデル内部が持つ可変状態の性質が違うため、 Rust 側の型分割方針も
異なる。本ドキュメントは違いと将来 ml モデルを増やすときの判断基準だけをまとめる (型ごとの詳細は
本体コードの module doc / 型 doc を参照する)。

## Silero VAD: 共有ハブ + 派生インスタンス

- 本体 `SileroVadModel` は immutable、推論インスタンス `SileroVad` が LSTM state を持つ
- 1 プロセスで 1 回だけ `load` し、`Arc<SileroVadModel>` から `new_instance()` で track / 話者ごとに
  独立インスタンスを派生させる (state / context が track 境界で混ざらないようにする)
- 詳細は `src/ml/audio/silero_vad.rs` 冒頭の module doc を参照

## Whisper: 単一インスタンス + 非同期橋渡し

- 本体 `WhisperDecoder` は KV cache を持つ mutable な状態機。 `Arc<Mutex<...>>` 越しの共有は
  直列化されて並列度が消えるため、Silero 型の共有ハブは採らない
- `WhisperPipeline` (mel filter + config + language 解決) が decoder を薄くラップし、
  `TranscriptionService` が 1 個の pipeline を blocking worker で保持して async 側と
  channel + oneshot で橋渡しする
- worker を複数持たない理由、KV cache の詳細、`spawn_blocking` の使い方は
  `src/ml/audio/whisper.rs` / `src/ml/audio/whisper/decode.rs` /
  `src/ml/audio/transcription_service.rs` の doc を参照

## 判断基準 (新しい ml モデルを足すとき)

モデル内部の可変状態の有無で決める。

- 本体が immutable 、可変 state はインスタンス側 (stateless matmul 中心のモデル、または ONNX 化
  された RNN 系で state を Rust 側で持ち回す形): Silero 型 (`Arc<Model>` + `new_instance`) を採る。
- 本体が可変 state を内包 (KV cache や encoder-decoder キャッシュ、大規模 Transformer 系):
  Whisper 型 (推論器を単一インスタンスで保持し blocking worker から呼ぶ) を採る。 async な produce
  側との橋渡しは channel + oneshot で行う。
- 折衷 (`Arc<Model>` + `Mutex` 越しの推論) は原則採らない。直列化して並列度が消えるか、境界前後で
  state が汚染されるかのどちらかになる。

## 実装参照

- Silero VAD: `src/ml/audio/silero_vad.rs`
- Whisper: `src/ml/audio/whisper.rs`, `src/ml/audio/whisper/decode.rs`
- 非同期橋渡し (blocking worker): `src/ml/audio/transcription_service.rs`
