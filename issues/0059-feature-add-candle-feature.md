# candle feature と ML モデル取得スクリプトを追加する

- Priority: Medium
- Created: 2026-06-24
- Completed:
- Model: Opus 4.7
- Branch: feature/add-candle-feature
- Polished:

## 目的

ML 推論機能 (Whisper / Silero VAD) の基盤として candle (Rust 製 ML 推論フレームワーク) を hisui のオプション依存として追加する。あわせて ML モデル取得スクリプト (`scripts/download_ml_models.py`) と `src/ml/{mod,device}.rs` の骨格を整備する。本 issue は親 issue 0012 系列の最初の層であり、後続の子 issue (0061-0063) の前提となる。

## 優先度根拠

本系列全体の前提となるが、本 issue 単独では利用者向けの機能を提供しない。後続 issue がマージされて初めて利用者から見える機能が完成するため、Medium。

## 現状

- hisui には ML 推論機能がない (`src/ml/` は存在しない)
- PR #246 (お試しブランチ) で candle 系依存とモデル取得スクリプト (.sh) が実装済みだが develop には入っていない
- 既存 feature: default = ["player"]、optional に nvcodec / fdk-aac / player

## 設計方針

### Cargo.toml への candle feature 追加

PoC の 3 段構成を踏襲:

```
candle = ["dep:candle-core", "dep:candle-nn", "dep:candle-transformers", "dep:candle-onnx", "dep:tokenizers"]
candle-metal = ["candle", "candle-core/metal"]
candle-cuda = ["candle", "candle-core/cuda"]
```

依存バージョン (PoC 踏襲、exact pin、各依存に用途コメント):

- `candle-core = "=0.10.2"`: テンソル計算
- `candle-nn = "=0.10.2"`: ニューラルネット building block
- `candle-transformers = "=0.10.2"`: Whisper モデル実装
- `candle-onnx = "=0.10.2"`: Silero VAD (ONNX) ロード・推論
- `tokenizers = "=0.22.0"` (`default-features = false`, `features = ["onig"]`): Whisper の text tokenizer

### システム依存

- `protoc` (candle-onnx のビルドに必須)
- CUDA toolkit (`candle-cuda` 時のみ)
- Xcode 標準 SDK (`candle-metal` 時、macOS で自動)

### src/ml/{mod,device}.rs

- `src/ml/mod.rs` を新規作成 (空 or 最小)
- `src/ml/device.rs` を新規作成: candle-metal feature 有効時に `Device::new_metal(0)` を試行、candle-cuda 有効時に `Device::cuda_if_available(0)` を試行、いずれも失敗時 CPU fallback
- 0059 では device.rs のロジックだけ。Whisper / VAD 本体は後続 issue で実装

### scripts/download_ml_models.py

- 標準ライブラリのみ (urllib.request / hashlib / argparse / pathlib) で実装
- huggingface_hub 等の追加依存はしない
- 起動: `uv run scripts/download_ml_models.py <target>`
- ターゲット: `whisper-tiny` / `silero-vad` (0059 範囲)。将来 `whisper-small` / `yolo` 等を追加する設計
- 取得先: Hugging Face (PoC と同じ URL)
- 保存先: `ML_MODELS_DIR` 環境変数 (デフォルト `./ml-models/`)
- ファイルは `.gitignore` 済み

### ci.yml への test-candle job 追加

`test-fdk-aac` / `test-openh264` のパターンに沿って独立 job 追加:

```yaml
test-candle:
  runs-on: ubuntu-24.04
  timeout-minutes: 20
  steps:
    - rustup update + apt install protobuf-compiler 等
    - Rust cache + ML モデル cache (actions/cache@v4 で ml-models/)
    - uv run scripts/download_ml_models.py whisper-tiny silero-vad
    - cargo test --features candle -p hisui (0059 では smoke 程度、実推論は 0062 / 0063 で追加)
- slack_notify の needs に test-candle を追加
```

### CHANGES.md エントリ

- `[ADD] candle (ML 推論フレームワーク) を candle feature 配下にオプション依存として追加する`
- `[ADD] ML モデル取得スクリプト scripts/download_ml_models.py を追加する`
- 担当者: @sile

## 完了条件

- `cargo build --features candle` が成功する
- `cargo build --features candle,candle-metal` (macOS) / `cargo build --features candle,candle-cuda` (CUDA 環境) のビルドパスが整っている (CI ではビルドのみ確認)
- `uv run scripts/download_ml_models.py whisper-tiny silero-vad` で `ml-models/` 配下にモデルが取得できる
- `src/ml/device.rs` の auto-detect が呼べる (空ロジック OK、後続 issue で使う)
- ci.yml に test-candle job が追加され green
- CHANGES.md に該当エントリ追記済み

## 解決方法

PR #246 ブランチの該当部分 (Cargo.toml の features セクション、依存追加、src/ml/{mod,device}.rs、scripts/download_ml_models.sh) をベースに、本 issue の設計方針に合わせて整理して develop 向けに移植する。.sh は .py に書き換える。
