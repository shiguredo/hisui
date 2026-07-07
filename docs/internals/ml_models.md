# `ml/audio` のモデル型と worker 配分

## 概要

`src/ml/audio/` の下には現在 2 種類の ML モデルが載っている: **Silero VAD** (発話区間検出) と
**Whisper** (音声文字起こし)。両者はモデル内部が持つ可変状態の性質が違うため、 Rust 側の型分割方針も
異なる。本ドキュメントは、その違いと背景、および将来 ml モデルを増やすときの判断基準を整理する。

## Silero VAD: 共有ハブ + 派生インスタンス

- **`SileroVadModel`** (immutable): ONNX グラフのパース結果と初期テンソルを保持する。 1 プロセスで
  1 回だけ `load` し、 `Arc<SileroVadModel>` で共有する。
- **`SileroVad`** (可変): LSTM state と 64 サンプルの context を持つ推論インスタンス。 track /
  話者ごとに `SileroVadModel::new_instance()` で独立に派生させる。
- モデル本体は immutable なので共有可能。可変状態はインスタンス側に閉じているため、 track 境界で
  state が混ざらない。

## Whisper: worker ごとにフルロード

- **`WhisperDecoder`** (`whisper/decode.rs`): candle の `Whisper` (encoder / decoder / KV cache)、
  tokenizer 、 greedy decode 用トークン ID 群を 1 型にまとめた worker 単位。
- **`WhisperPipeline`** (`whisper.rs`): モデルディレクトリからの `config.json` /
  `tokenizer.json` / `model.safetensors` ロードと mel フィルタ・言語トークン解決を
  `WhisperDecoder` の周りに載せた層。 1 worker = 1 `WhisperPipeline` = 1 `WhisperDecoder` 。
- 共有 (`Arc`) しない。 candle の `Whisper` 型は KV cache が mutable な内部状態のため、
  `Arc<Mutex<...>>` で共有しても直列化されて並列度が上がらない。
- 並列度は `TranscriptionService::new(model_dir, device, workers)` の `workers` で決まり、内部で
  `WhisperDecoder` を個別に workers 個ロードして worker プールに詰める。

## 判断基準 (新しい ml モデルを足すとき)

モデル内部の可変状態の有無で決める。

- 本体が immutable 、可変 state はインスタンス側 (stateless matmul 中心のモデル、または ONNX 化
  された RNN 系で state を Rust 側で持ち回す形): Silero 型 (`Arc<Model>` + `new_instance`) を採る。
- 本体が可変 state を内包 (KV cache や encoder-decoder キャッシュ、大規模 Transformer 系):
  Whisper 型 (worker ごとにフルロード) を採る。並列度はロード時間とメモリを worker 数分だけ払う
  トレードオフになる。
- 折衷 (`Arc<Model>` + `Mutex` 越しの推論) は原則採らない。直列化して並列度が消えるか、境界前後で
  state が汚染されるかのどちらかになる。

## 実装参照

- Silero VAD: `src/ml/audio/silero_vad.rs`
- Whisper: `src/ml/audio/whisper.rs` , `src/ml/audio/whisper/decode.rs`
- worker プール: `src/ml/audio/transcription_service.rs`
