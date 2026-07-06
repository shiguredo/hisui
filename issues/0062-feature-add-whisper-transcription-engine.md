# Whisper 文字起こしエンジンと TranscriptionService/Processor を実装する

- Priority: Medium
- Created: 2026-06-24
- Completed:
- Model: Opus 4.7
- Branch: feature/add-whisper-transcription-engine
- Polished:

## 目的

Whisper モデルによる音声文字起こしのライブラリ層と、複数音声トラックを並列処理できる `TranscriptionService` (ワーカープール) + `TranscriptionProcessor` (MediaPipeline 上の processor) を実装する。本 issue は親 issue 0012 系列の推論基盤層であり、0063 (利用者向けサブコマンド) の前提となる。

## 優先度根拠

本系列の最終層 0063 の前提となる中核 issue。Medium。

## 現状

- hisui には文字起こし機能がない
- PR #246 (お試しブランチ) で `src/ml/audio/{whisper, decode, multilingual}.rs` と PoC processor (`AudioMlProcessor`) が実装済みだが、結果はログ出力のみで MediaPipeline に流れない (publish しない)
- 0059 で candle feature と src/ml/{mod, device}.rs 骨格、0060 で MediaFrame::Text、0061 で Silero VAD と前処理ライブラリが揃っている前提

## 設計方針

### Whisper ライブラリ層

`src/ml/audio/` 配下に以下を新規追加 (PoC からの移植 + 整理):

- `whisper.rs`: WhisperPipeline (モデルロード、PCM → mel → encode → decode → text の一連の処理)
- `decode.rs`: WhisperModel / Decoder (KV cache 管理、decode ループ)
- `multilingual.rs`: 言語自動検出

依存はすべて 0059 で導入済みの candle-core / candle-nn / candle-transformers / tokenizers。

### TranscriptionService (新規)

ワーカープール:

```
TranscriptionService
  ├ M 個の WhisperModel を起動時にロード (デフォルト M = 1、設定可変)
  ├ 推論キュー (mpsc / crossbeam)
  └ M 個の worker (spawn_blocking で常駐、キューから取って推論)
```

- M はコンストラクタで指定
- Whisper モデルは KV cache 内部状態 mutable のため、モデル個別ロード (Arc 共有 + Mutex 排他では意味が薄い)
- 推論本体は `tokio::task::spawn_blocking` で別スレッド (CPU-bound、async ワーカー starve 回避)
- 投入 API: `submit(pcm: Vec<f32>, reply: oneshot::Sender<TranscriptResult>)` 等 (実装時に確定)

### TranscriptionProcessor (新規)

MediaPipeline 上の processor として:

```
TranscriptionProcessor (1 個 = 1 入力 audio track = 1 出力 text track)
  ├ subscribe_track(audio_track_id) で PCM 入力
  ├ リサンプル → Silero VAD で発話区間抽出 (0061 のライブラリを利用)
  ├ TranscriptionService::submit() で推論依頼
  └ 結果を MediaFrame::Text として publish_track で流す
```

- PoC の `AudioMlProcessor` を再設計
- ログ出力固定 → publish_track 経由で MediaPipeline に流す
- マイク入力固定 → 任意の入力 audio track を subscribe
- 結果を受け取る側は subscribe_track(transcript_track_id) で MediaFrame::Text を受信

### TranscriptionService と Processor の関係

- 1 個の Service を複数の Processor が共有する (`Arc<TranscriptionService>`)
- 各 Processor は自身の入力 track ID と出力 track ID を持ち、Service にリクエストを投げて oneshot::Receiver で結果を待つ
- 結果が来たら publish_track で流す

### 順序保証・VAD・終端・エラーの方針

- 順序保証: 複数の入力 track (複数 Processor) 間での結果の順序保証はしない。単一 Processor 内では submit した順に oneshot::Receiver を FIFO で待って publish するため、1 つの text track 内では TextFrame が start 順に流れる (M > 1 でも自然に保証される)
- VAD: Silero VAD あり前提とする。VAD なし (固定長分割等) のパスは本 issue では実装しない
- 終端 (フラッシュ): 入力 audio track の終端を検知したら、VAD バッファに残っている末尾の発話区間を flush して最後の推論を submit し、pending の推論結果をすべて受け取って publish し切ってから出力 text track を終了する
- エラー: 推論失敗 (推論エラー、Service との通信断等) 時は該当 Processor をエラーで終了させる (processor ごと落とす)。区間スキップや backoff retry などの細かいケアは、将来必要になった場合に processor 共通の仕組みとして検討する (本 issue のスコープ外)

### MediaFrame::Text 出力フォーマット

0060 で定義した TextFrame の各フィールド (start / end / text / language / no_speech_prob / avg_logprob) を Whisper の出力から埋める。

### テスト

- 単体テスト: Whisper ライブラリ層の各関数
- integration テスト: testdata の短い実音声 (日本語 / 英語、CC0) を `#[cfg(feature = "candle")]` で実推論。緩い不変条件 (text 非空 + keyword substring + 品質指標範囲 + 言語判定厳密一致) で assert
- in-process pipeline test: TranscriptionProcessor が MediaFrame::Text を publish できる (テスト用 source processor + Whisper + subscribe で結果取得)
- モック禁止 (AGENTS.md)、実モデル使用

## 完了条件

- `src/ml/audio/{whisper, decode, multilingual}.rs` が追加されている
- `TranscriptionService` (ワーカープール) と `TranscriptionProcessor` (MediaPipeline processor) が新規追加されている
- TranscriptionProcessor が MediaFrame::Text を publish できる (in-process pipeline test で確認)
- integration テスト (whisper-tiny + Silero VAD + testdata 実音声) が green
- `cargo test --features candle -p hisui` が test-candle CI job (0059 で骨格追加) で green

## 解決方法

PR #246 の `src/ml/audio/{whisper, decode, multilingual}.rs` と AudioMlProcessor をベースに、本 issue の設計方針 (ワーカープール、publish_track 経由) に合わせて新規実装する。
