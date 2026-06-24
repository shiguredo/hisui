# Silero VAD と音声前処理ライブラリを実装する

- Priority: Medium
- Created: 2026-06-24
- Completed:
- Model: Opus 4.7
- Branch: feature/add-silero-vad-and-audio-preprocessing
- Polished:

## 目的

Whisper 推論の前段として、Silero VAD (ONNX) による音声区間検出と、リサンプル / buffer 等の音声前処理ライブラリを実装する。これは親 issue 0012 系列の前処理層であり、0062 (Whisper エンジン) の前提となる。

## 優先度根拠

本系列の中核 0062 の前提となるが、本 issue 単独では利用者向けの機能を提供しない。Medium。

## 現状

- hisui には音声前処理ライブラリ (リサンプル / VAD / buffer 等) がない
- PR #246 (お試しブランチ) で `src/ml/audio/{silero_vad, vad, buffer, decode, config}.rs` 等が実装されているが、マイク入力前提で汎用化が不十分

## 設計方針

### 依存

0059 で追加済みの `candle-onnx` を Silero VAD のロード・推論に使う。

### モジュール構成

`src/ml/audio/` 配下に以下を新規追加 (PoC からの移植 + 汎用化):

- `silero_vad.rs`: Silero VAD モデルのロードと推論 (16 kHz、256 サンプル = 16 ms or 512 サンプル = 32 ms チャンクで発話確率を出力)
- `vad.rs`: VadGate 抽象 (Silero VAD or RMS フォールバック、閾値ゲート)
- `buffer.rs`: AudioChunkBuffer (入力 PCM を任意秒のチャンクに分割、Whisper 用の固定長 30 秒チャンク等)
- `config.rs`: VAD / 前処理の設定構造体
- リサンプル: 任意サンプルレート → 16 kHz モノラル変換 (自前 FIR、PoC のロジック流用)

### Silero VAD の実装ポイント

- ONNX モデルロード (`candle-onnx::simple_eval` 等で推論)
- 入力フォーマット: 16 kHz モノラル f32 PCM、256 / 512 サンプルチャンク
- 出力: 0.0〜1.0 の発話確率スコア
- 閾値ゲート: デフォルト 0.5
- RMS フォールバック: モデルロード失敗時の代替 (PoC と同様)

### 汎用化 (PoC との差)

- マイク入力前提を外す (= 任意の f32 PCM スライスを受け取る形に)
- 入力サンプルレートは引数で受け取り、必要なら内部でリサンプル
- モデルパスは引数で受け取る (環境変数や固定パスにしない)

### テスト

- 単体テスト (各純粋関数: リサンプル、VAD ゲート、buffer 分割)
- PBT (pbt/ に追加): リサンプル出力長 = ceil(input_len * 16000 / src_rate)、buffer.push() の不変条件、無音入力で VAD 全 reject、等
- integration テスト: 実 Silero VAD モデルでの発話 / 無音判定 (`#[cfg(feature = "candle")]`)
- モデルが `ml-models/` にあれば実行、無ければ skip / エラー (実装時に判断)

### モック禁止

実 Silero VAD モデルでテストする (AGENTS.md「モックやスタブは絶対に利用しないこと」)。

## 完了条件

- `src/ml/audio/{silero_vad, vad, buffer, config}.rs` 等が追加されている (リサンプル含む)
- `cargo test --features candle -p hisui` が green
- PBT (pbt/) が green
- Silero VAD が実音声で発話 / 無音を判定できる (integration テスト)
- モデル不在時の挙動が定義され、エラーパスがテストされている

## 解決方法

PR #246 の `src/ml/audio/{silero_vad, vad, buffer, decode, config}.rs` をベースに移植・整理する。マイク入力前提を外して汎用 PCM ライブラリ層として整える。
