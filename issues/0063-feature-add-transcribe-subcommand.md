# hisui -x transcribe 実験的サブコマンドを追加する

- Priority: Medium
- Created: 2026-06-24
- Completed:
- Model: Opus 4.7
- Branch: feature/add-transcribe-subcommand
- Polished:

## 目的

利用者が音声 / 動画ファイルを文字起こしできる実験的サブコマンド `hisui -x transcribe <input>` を提供する。これは親 issue 0012 系列の利用者向け統合層であり、本系列のリリース対象。

## 優先度根拠

本系列の最終層であり、ここまでの基盤を利用者から見える機能として完成させる。Medium。

## 現状

- hisui には文字起こし系のサブコマンドがない
- 0062 までで MediaPipeline 上での文字起こしが動く状態だが、外向きの CLI 露出はない
- 過去に `pipeline` サブコマンドが `--experimental(-x)` フラグを使うパターンを採用していたが、現状の hisui には残っていない (`pipeline` サブコマンドも `-x` フラグも src/ にない)
- 現サブコマンド: inspect / list-codecs / compose / vmaf / tune / server
- PoC の `hisui ml audio` (マイク入力) は PoC 残骸で本系列とは別 (YOLO スピンアウト issue で扱う)

## 設計方針

### グローバルフラグ --experimental(-x) の復活

main.rs に `noargs::flag("experimental").short('x')` を追加し、各サブコマンドに伝播する。`transcribe` サブコマンドはこのフラグが立っている場合のみ受け付ける。立っていない場合は「実験的機能です。`--experimental` (`-x`) フラグを付けて起動してください」のエラーで終了。

### サブコマンド: `hisui -x transcribe <input>`

`src/subcommand_transcribe.rs` を新規追加。

CLI:

- 位置引数: `<input>` (WAV または MP4 ファイル)
- `--model-dir <path>` (必須): Whisper モデルディレクトリ (例: `./ml-models/whisper-tiny/`)
- `--language <code>`: Whisper 言語指定 (省略時 auto detect、`ja` / `en` 等)
- `--workers <N>`: TranscriptionService のワーカープール並列数 (デフォルト 1)
- `--vad <kind>`: silero / off (デフォルト silero)

### 入力ファイル形式

- WAV: 最小の自前 reader (フォーマット単純) で対応
- MP4: hisui 既存の mp4 reader + 音声デコーダー (Opus / AAC) を流用 (compose 系のコードに既にある資産を再利用)
- 他形式 (WebM / Opus 等) は本 issue では対応せず、別 issue (compose 系統合) で扱う

### 内部実装の流れ

```
subcommand_transcribe::try_run
  ├ MediaPipeline 組み立て
  ├ source processor (WAV/MP4 → MediaFrame::Audio publish)
  ├ TranscriptionProcessor (audio subscribe → MediaFrame::Text publish、0062 で実装)
  └ text subscriber (MediaFrame::Text subscribe → 標準出力に JSON LINE)
```

### 出力フォーマット: JSON LINE

1 行 1 セグメント、TextFrame の各フィールドをそのまま:

```jsonl
{"start": 0.5, "end": 2.3, "text": "こんにちは", "language": "ja", "no_speech_prob": 0.02, "avg_logprob": -0.15}
```

利用者が機械処理できる簡易出力。SRT / WebVTT / WebRTC データチャネル等の本格的な外部出力経路は 0014 範囲で別 issue で扱う。

### ドキュメント

- `docs/command_transcribe.md` を新規作成
- 使い方 / CLI オプション / 出力例 / モデル取得手順 / 制約 (実験的機能、対応フォーマット等) を記載

### e2e テスト

- `e2e-tests/` (pytest) で `hisui -x transcribe` を起動し、JSON LINE 出力をパース、構造検証
- 行ごとに必須キー (start / end / text / language / no_speech_prob / avg_logprob) が存在
- start < end、time 単調増加、language が指定値と一致
- 緩い不変条件 (text 非空 + keyword substring + 品質指標範囲)

### CI

- 0059 で test-candle job の骨格が追加済み
- 本 issue で `cargo test --features candle -p hisui` の integration テストと e2e テストに実推論を含める形で組込
- testdata の音声サンプル (日本語 / 英語の短発話 WAV / MP4、CC0) を追加

### CHANGES.md エントリ

- `[ADD] hisui -x transcribe 実験的サブコマンドを追加する`
- 担当者: @sile

## 完了条件

- `hisui -x transcribe <input.wav>` および `hisui -x transcribe <input.mp4>` が動作する
- `--experimental` (`-x`) フラグ無しで呼ぶと有効なエラーメッセージで終了する
- 結果が JSON LINE で標準出力に出力される
- `docs/command_transcribe.md` が整備されている
- `e2e-tests/` の test が green (CI で実推論)
- `cargo test --features candle -p hisui` が test-candle CI job で green
- CHANGES.md に `[ADD] hisui -x transcribe` エントリが追記されている

## 解決方法

0062 で実装された TranscriptionService / TranscriptionProcessor / MediaFrame::Text の基盤を組み合わせて subcommand_transcribe を実装する。`--experimental(-x)` フラグは過去 `pipeline` サブコマンドが使っていたパターンを git log から確認しつつ復活させる。
