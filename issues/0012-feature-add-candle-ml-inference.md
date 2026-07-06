# candle を用いた ML 推論機能 (文字起こし) の正式対応 (索引 issue)

- Priority: Medium
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: -
- Polished:

## 本 issue の位置づけ

本 issue は元々 PoC ブランチ (PR #246) の成果から candle (Rust 製 ML 推論フレームワーク) 系機能を hisui に正式対応として取り込むためのものとして起票された。議論の結果、対応範囲が **依存追加 / MediaPipeline 拡張 / 音声前処理ライブラリ / 推論基盤 / 利用者向けサブコマンド** の独立した層にまたがるため、5 つの子 issue に分割した。

本 issue 自体は実装作業を伴わず、**子 issue の索引・全体方針の記録** としてのみ機能する。実装は子 issue で行う。

## 子 issue 一覧

| # | issue | 内容 | 依存 |
|---|---|---|---|
| 0059 | candle feature 追加と ML モデル取得スクリプト | Cargo.toml の candle feature 追加、device 検出骨格、scripts/download_ml_models.py、ci.yml に test-candle 骨格 | - |
| 0060 | MediaFrame::Text バリアントを追加する | MediaFrame::Text(Arc<TextFrame>) 新設、既存 match 箇所への Text ブランチ追加 | - |
| 0061 | Silero VAD と音声前処理ライブラリを実装する | Silero VAD ライブラリ (candle-onnx)、リサンプル、buffer、PBT | 0059 |
| 0062 | Whisper 文字起こしエンジンと TranscriptionService/Processor を実装する | Whisper ライブラリ、ワーカープール、MediaPipeline processor、testdata 実音声追加、test-candle CI へ whisper-tiny 追加 (integration テスト実推論) | 0059, 0060, 0061 |
| 0063 | hisui -x transcribe 実験的サブコマンドを追加する | subcommand_transcribe (WAV/MP4 入力 → JSON LINE 出力)、--experimental(-x) フラグ復活、ドキュメント、CHANGES.md エントリ、e2e テストへの実推論組込 | 0062 |

### 依存関係

```
0059 ──→ 0061 ─┐
               ├──→ 0062 ──→ 0063
0060 ──────────┘
```

- 0059 と 0060 は互いに独立で並行可能
- 0061 は 0059 (candle 依存) に依存
- 0062 は 0059, 0060, 0061 全部に依存
- 0063 は 0062 に依存し、本系列の利用者向け統合層となる

## 確定方針 (議論成果)

### スコープ

- Whisper 文字起こし + Silero VAD のみを 0061-0063 で正式対応する
- YOLO 物体検出は別 issue にスピンアウトする (PoC PR #246 で実装済みだが本系列とは分離)
- candle 依存追加 (0059) はスコープに関係なく一律発生する

### VAD

- Silero VAD (ONNX) を採用する
- 代替候補 (WebRTC VAD / RNNoise / nnnoiseless) より精度・多言語汎化で優位
- candle-onnx + ビルド時 protoc 依存を許容する

### Whisper モデル管理

- ワーカープール方式: TranscriptionService が M 個の WhisperPipeline を保持し、N 個の TranscriptionProcessor が推論キューに発話区間を投入する
- デフォルト M = 1、CLI / 設定で可変
- モデルは差し替え可能 (CLI 引数で tiny / base / small / medium / large 等を指定)
- 実用想定は small 以上 (ウェブ会議用途、日本語含む)。tiny は動作確認・デモ用
- candle の Whisper 実装は KV cache を持つため、Arc 共有では Mutex 排他で直列化される。スループットを上げるには M を増やしてモデル個別ロードが必要

### 推論実行

- 推論本体は tokio::task::spawn_blocking で別スレッドに逃がす (CPU-bound のため async ワーカー starve を回避)
- GPU 利用時 (candle-metal / candle-cuda) も同期待ちで拘束されるため同様

### 結果通知

- MediaPipeline 流儀準拠で MediaFrame::Text(Arc<TextFrame>) を新設し、TranscriptionProcessor は publish_track で結果を流す
- TextFrame は start / end / text / language / no_speech_prob / avg_logprob の 6 フィールド
- input_track_id は持たない (subscribe した TrackId で判別)
- 0014 (外部出力経路) の議論はこの上に乗る

### 入力源とサブコマンド

- 0012 系列ではファイル入力 (WAV + MP4) のみを最小動作確認入り口とする
- MP4 は hisui 既存の mp4 reader + 音声デコーダー (Opus / AAC) を流用する
- WAV は最小の自前 reader で対応する
- `hisui -x transcribe <input>` 実験的サブコマンドを新設する
- `--experimental(-x)` フラグ必須 (過去 pipeline サブコマンドのパターンを復活)
- 標準出力に JSON LINE で結果書き出し (動作確認用の簡易出力)
- compose / server (obsws) 統合は本系列の範囲外。別 issue で TranscriptionService / TranscriptionProcessor を再利用する

### モデル取得・配布

- `scripts/download_ml_models.py` を新設する (PoC の .sh は廃止)
- リポジトリで既に Python が first-class (uv / pyproject.toml / canary.py) なので Python ツールに統一
- 標準ライブラリのみ (huggingface_hub 等の追加依存なし)
- `uv run scripts/download_ml_models.py <target>` で起動
- モデルファイルはリポジトリに含めない (.gitignore 済み)
- CLI は `--model-dir <path>` 必須 (デフォルトパスを持たない)

### テスト戦略

- 4 階層構成: unit / PBT / integration / e2e
- モック禁止規約に従い実モデル・実音声で integration テスト
- Whisper 非決定性は緩い不変条件 (text 非空 + keyword substring + 品質指標範囲 + 言語判定厳密一致) で吸収
- CI に test-candle job 追加 (whisper-tiny + silero-vad、GitHub Actions cache)
- testdata の音声サンプルは CC0 / パブリックドメインを別途調達
- 詳細 (cache key 設計、env var 命名、testdata 具体調達、許容閾値、PBT 不変条件、e2e 検証項目) は実装着手時に再検討

### device feature 構成

- PoC の 3 段踏襲: candle / candle-metal / candle-cuda
- src/ml/device.rs の自動検出ロジック踏襲 (Metal/CUDA feature 有効時に試行、失敗時 CPU fallback)
- 0012 系列の CI は CPU only。Metal / CUDA の CI 組込は別 issue

### CHANGES.md

- 利用者から見える変更のみ [ADD] エントリ。内部実装 (MediaFrame バリアント追加、ライブラリ、Service / Processor) はエントリ書かない
- 想定 3 エントリ: candle 依存追加 (0059) / download_ml_models.py (0059) / hisui -x transcribe (0063)
- 担当者は @sile
- 文体は shiguredo-changelog 規約準拠

## 将来検討事項

### 本系列の延長

- compose 系統合: Sora アーカイブから字幕付き mp4 を生成する issue
- server (obsws) 系統合: リアルタイム文字起こし + obsws API
- 多形式入力対応 (WebM / Opus 等)
- 0014 (ML 結果出力経路) との接続点設計

### モデル・推論基盤

- distil-whisper / kotoba-whisper など派生モデル対応
- ML ワーカープール基盤化 (Whisper 専用 → ML 全般の共通基盤に格上げ)
- candle-metal / candle-cuda の CI 組込

### MediaPipeline 拡張

- メディアトラックの「ストリーム / セッション帰属」メタ情報 (映像と音声を同一ソースとして紐付ける仕組み)
- MediaFrame::Text を活用した他機能 (字幕重畳ブリッジ等は 0014 / 別 issue)

### スピンアウト

- YOLO 物体検出 (映像系 ML、PoC PR #246 で実装済み、別 issue 化予定)
