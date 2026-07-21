# 音声文字起こしパイプライン

hisui は音声 track (`MediaFrame::Audio`) を Whisper で文字起こしし、text track
(`MediaFrame::Text`) に流す処理系を持つ。 本ドキュメントは全体像と登場人物、
処理の流れを整理する。 モデル型分割の判断基準は [ml_models.md](ml_models.md) を、
細部の API は本体コードの doc を参照する。

## 全体像

```
音声 track (AudioFrame, I16Be)
    │
    ▼ subscribe
TranscriptionProcessor (MediaPipeline 上の processor)
    ├─ i16 → f32 正規化、16 kHz mono に resample
    ├─ VadGate (Silero VAD) で発話区間 (SpeechSegment) を切り出す
    └─ 発話区間の PCM を最大 30 秒ずつに分割する
    │
    ▼ submit
TranscriptionService (bounded mpsc queue + 単一 blocking worker)
    │
    ▼ worker が blocking で実行
WhisperPipeline (mel filter + 言語解決の薄いラッパ)
    │
    ▼
WhisperDecoder (candle-transformers の Whisper を保持)
    ├─ PCM → mel スペクトログラム
    ├─ encoder で audio features を得る
    ├─ SOT / 言語 / transcribe / no_timestamps トークンを積む
    ├─ decoder + greedy sampling でトークン列を出す
    └─ text / avg_logprob / no_speech_prob を返す
    │
    ▼ oneshot::Sender で結果を戻す
TranscriptionProcessor
    ├─ oneshot::Receiver を FIFO で待つ (submit した順で publish)
    └─ 幻覚判定を通ったものを TextFrame に組んで publish する
    │
    ▼ publish
text track (MediaFrame::Text)
```

## 登場人物

### `TranscriptionProcessor` (`src/ml/audio/transcription_processor.rs`)

MediaPipeline 上の processor。 1 音声 track → 1 text track の変換を担う。

- I16Be の `AudioFrame` を f32 正規化し、1 秒単位のバッファに貯めてから
  `resample_to_mono` で 16 kHz mono 化する
  - 1 秒単位にする理由は、`resample_to_mono` がバッチ設計 (フィルタ状態を持ち回さない)
    で出力長が ceil 丸めされるため、フレーム単位で呼ぶと丸め誤差が累積するため
- `VadGate` (Silero VAD) に流して発話区間 (`SpeechSegment`) を得る
- 発話区間の PCM を最大 30 秒ずつに分割して `TranscriptionService::submit` に投げる
  - Whisper encoder の入力長上限が 30 秒相当 (mel フレーム 3000) のため
- 返ってきた `oneshot::Receiver` を FIFO で待ち、`TextFrame` に組んで publish する
- 幻覚判定 (`WhisperTranscript::is_likely_no_speech`) を通らないもの・空 text は
  publish しない
- 入力 track の EOS で pending をすべて drain してから出力 text track を閉じる

タイムスタンプ (`TextFrame.start` / `end`) は Whisper 出力ではなく VAD 由来 (発話区間の
サンプル通し番号 + 最初の `AudioFrame.timestamp` を基準とした写像) で埋める。

### `TranscriptionService` (`src/ml/audio/transcription_service.rs`)

async 側 (tokio) から blocking な Whisper 推論を叩くための橋渡し。

- 内部に単一の blocking worker (`spawn_blocking`) と bounded mpsc channel
  (容量 2、backpressure 用) を持つ
- `submit(request)` は `oneshot::Receiver` を返し、呼び出し側が個別に await できる
- 全 `Arc<TranscriptionService>` の drop でキュー sender が閉じ、worker はキュー内の
  リクエストを drain し切ってから終了する

pool 化しない理由: candle は CPU 推論で既定でホスト物理コアまで並列化するので、外側で
worker を複数持つとコア競合で per-decode の並列度が相殺され、実効スループットが伸びない。
worker を絞ることで、代わりに `RAYON_NUM_THREADS` で candle 側の並列度を制御できる。

### `WhisperPipeline` (`src/ml/audio/whisper.rs`)

`WhisperDecoder` の周りに以下を載せた薄いラッパ。

- モデルディレクトリからのロード (`config.json` / `tokenizer.json` /
  `model.safetensors` の 3 ファイルを検証してからロード)
- mel filter bank の同梱 (`melfilters.bytes`、80-bin 固定)
- 指定言語コード → 言語トークン変換 (`multilingual::language_token_from_code`)
- mel を Whisper encoder の入力長 (最大 3000 frames) に narrow する

呼び出し側の API は「PCM (16 kHz mono f32) と言語コードを渡すと `WhisperTranscript`
が返る」。 128-bin (large-v3 系) は非対応で、tiny / base / small の 80-bin モデルを
対象とする。

### `WhisperDecoder` (`src/ml/audio/whisper/decode.rs`)

candle-transformers の `Whisper` (encoder + decoder + KV cache) と tokenizer を
保持する低レベル推論器。

- mel テンソルと言語トークンを受け取り、greedy decode (温度 0 固定) でトークン列を吐く
- text / avg_logprob / no_speech_prob を返す
- KV cache はリクエスト間で必ずリセットし、state が漏れないようにする
- 言語トークンも state に持たず引数で毎回受ける (リクエスト単位で完結)
- タスクは文字起こし (`transcribe`) 固定、タイムスタンプトークンは出力しない
  (時刻は VAD 由来で埋めるため)
- `avg_logprob` は openai/whisper の定義に合わせて「プレフィックス除去後の生成トークン数
  (EOT 含む) で平均」する

### `WhisperTranscript` / `TextFrame`

- `WhisperTranscript` (`src/ml/audio/whisper.rs`): decoder 層の型。 f64 の
  `Probability` / `LogProbability` で品質指標を持つ (candle 内部と同じ精度)。
- `TextFrame` (`src/text.rs`): MediaPipeline 上を流れる型。 `Option<f32>` で
  品質指標を持つ (下流の JSON 出力等に合わせて narrowing)。
- 両者のブリッジは `TranscriptionProcessor` が担う (幻覚判定 + f32 化 + track 時刻付与)。

### 補助モジュール

- `src/ml/audio/whisper/multilingual.rs`: 多言語モデル判定 (`is_multilingual_config`)
  と言語コード → Whisper 言語トークン変換 (`language_token_from_code`)。 言語自動検出は
  実装しない (whisper-tiny の検出精度が低く、誤検出がトラック全体を劣化させるため。
  必要になれば別途追加する)。
- `src/ml/audio/vad.rs`: Silero VAD ラッパ (`VadGate` / `SpeechSegment`)。 保持が必要な
  最小サンプル番号を返す `min_required_sample` は Processor 側の PCM 破棄判定に使う。
- `src/audio/resample.rs`: `resample_to_mono` (f32、polyphase FIR、バッチ設計)。
  `SUPPORTED_HZ` (8k / 16k / 22.05k / 24k / 32k / 44.1k / 48k) 間の変換をサポートする。

## 型設計の背景

Silero VAD と Whisper で型分割方針が異なる (共有ハブ + 派生インスタンス vs
単一インスタンス + 非同期橋渡し) 理由は [ml_models.md](ml_models.md) を参照。

## 実装参照

- Processor: `src/ml/audio/transcription_processor.rs`
- Service: `src/ml/audio/transcription_service.rs`
- Pipeline / Decoder: `src/ml/audio/whisper.rs`, `src/ml/audio/whisper/decode.rs`
- 多言語モデル判定 / 言語トークン解決: `src/ml/audio/whisper/multilingual.rs`
- Silero VAD: `src/ml/audio/silero_vad.rs`, `src/ml/audio/vad.rs`
- resample: `src/audio/resample.rs`
- 型 (`WhisperTranscript` / `TextFrame` / `LanguageCode`): `src/ml/audio/whisper.rs`,
  `src/text.rs`
