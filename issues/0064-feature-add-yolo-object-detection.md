# YOLO 物体検出機能を hisui に正式対応する

- Priority: Low
- Created: 2026-06-24
- Completed:
- Model: Opus 4.7
- Branch: feature/add-yolo-object-detection
- Polished:

## 目的

PR #246 (お試しブランチ) で実装済みの YOLOv8 物体検出機能を hisui に正式対応として取り込む。candle 系基盤 (親 issue 0012 系列で導入予定の `candle` feature) を利用し、映像トラックに対する物体検出を MediaPipeline 上で動作させる。

本 issue は 0012 系列 (Whisper 文字起こし) とは独立した **映像系 ML 機能** であり、優先度・スコープも別に管理する。

## 優先度根拠

- PR #246 で動作確認済みのため実現性は高い (ゼロから研究ではなく PoC を引き上げる作業)
- 一方で字幕オーバーレイ (0012 + 0013) のような明確な応用先が現時点でない
- 業務を止めている課題ではない
- 0012 系列 (Whisper) を優先するため、本 issue は後回し
- 以上から Low

## 現状

- develop には YOLO 機能はない (`src/ml/yolo.rs` は存在せず)
- PR #246 (お試しブランチ) で以下が実装済み:
  - `src/ml/yolo.rs`: YOLOv8 物体検出 (Detect / Pose の 2 モード、safetensors 重みロード、I420 フレームへの推論・描画)
  - `src/ml/mod.rs` の `MlModel` enum (`Detect(YoloV8)` / `Pose(YoloV8Pose)`)
  - `src/subcommand_ml.rs` の `hisui ml` (カメラ入力 → YOLO 推論 → ウィンドウ表示、`player` feature 必須)
  - 重みは `lmz/candle-yolo-v8` (yolov8s.safetensors / yolov8s-pose.safetensors)
  - 前処理は `shiguredo_libyuv` で I420 → RGB 変換 + リサイズ (SIMD)
  - MediaPipeline 上で `VideoRealtimeMixer` に統合 (`Arc<MlModel>` 共有で複数カメラに適用可能)
- candle 系依存と `candle` feature は 0012 系列 (0059) で develop に入る予定。本 issue はその基盤を利用する

## 設計方針 (起票時の骨子、polish 時に詳細化)

### 依存

- 0059 (candle feature 追加) がマージ済みである前提
- `candle-core` / `candle-nn` / `candle-transformers` をそのまま利用 (YOLO は `candle-onnx` を使わないため protoc 依存は実は不要だが、0059 で既に入っている)
- 追加依存なし

### モデル管理

- YOLO の forward は内部状態を更新しない read-only 推論 (Whisper と違って KV cache 等を持たない)
- `Arc<MlModel>` で複数 processor から共有可能 (PoC 既存パターン踏襲)
- 共有しても並列推論可能 (Mutex 不要)

### モジュール構成

PR #246 の構成を踏襲:

- `src/ml/yolo.rs`: YOLOv8 推論本体
- `src/ml/mod.rs` の `MlModel` enum (Detect / Pose) を追加

### 入力 / 出力

- 入力: `MediaFrame::Video(Arc<VideoFrame>)` (I420)
- 前処理: `shiguredo_libyuv` で I420 → RGB 変換 + 入力解像度 (例: 640x640) にリサイズ
- 出力: 検出結果 (バウンディングボックス + クラス + 信頼度) のリスト

### サブコマンドと統合形 (要議論)

PoC の `hisui ml` (カメラ入力 → YOLO → ウィンドウ表示、`player` 必須) を正式化するか、別の形にするかは polish 時に確定する。候補:

- カメラ入力 → ウィンドウ表示の PoC 形を磨く (ライブデモ用途)
- 動画ファイル入力に置き換え (`hisui -x detect <video.mp4>` 等、CI 親和性高い)
- compose / server 系へのオーバーレイ統合 (検出枠を合成映像に焼き込む)
- MediaFrame に Detection バリアントを追加し、processor 経由で結果を流す (0060 の MediaFrame::Text と同様のパターン)

### モデル配布

- `scripts/download_ml_models.py` (0059 で追加) に `yolo` ターゲットを追加
- yolov8s.safetensors / yolov8s-pose.safetensors を `ml-models/yolo/` に取得

### テスト戦略

- 0061-0063 で確立する 4 階層構成 (unit / PBT / integration / e2e) を踏襲
- testdata の静止画 (CC0、人物 / 物体が写っているもの) で実推論し、緩い不変条件 (検出件数が想定範囲、信頼度が閾値以上、bounding box がフレーム内) で assert
- CI への組込み (test-candle job への追加) は polish 時に判断

### CHANGES.md エントリ

- `[ADD] YOLOv8 物体検出機能 / `hisui ml` (or 確定したサブコマンド名) を追加する`
- 担当者: @sile

## 完了条件 (起票時の素案、polish 時に詳細化)

- YOLOv8 物体検出が `candle` feature 配下で hisui に正式対応されている
- 動作確認可能なサブコマンド or MediaPipeline processor として利用できる
- モデル取得スクリプトに yolo ターゲットが追加されている
- テストが green
- CHANGES.md に該当エントリ追記済み

## 解決方法

PR #246 の YOLO 関連コード (src/ml/yolo.rs、src/ml/mod.rs の MlModel enum、src/subcommand_ml.rs) をベースに、本 issue の設計方針に従って develop 向けに整理する。サブコマンドと統合形は `/polish-issue` で確定してから実装に入る。
