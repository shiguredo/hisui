# `hisui ml` コマンド

[Candle](https://github.com/huggingface/candle) を使った ML 推論の PoC 用サブコマンドです。本番運用を前提としたものではありません。

| コマンド | 内容 | 必要な feature |
|----------|------|----------------|
| `hisui ml` | ビデオデバイス → YOLOv8 → ウィンドウ表示 | `candle`, `player` |
| `hisui ml audio` | マイク → VAD → Whisper 文字起こし（ログ出力） | `candle` |

---

## ビルド

### 映像（`hisui ml`）

```bash
cargo build --release --features candle,player
```

GPU を使う場合はプラットフォームに応じて feature を追加します。

| プラットフォーム | feature |
|------------------|---------|
| macOS (Apple Silicon) | `candle-metal` |
| Linux (NVIDIA GPU) | `candle-cuda` |

```bash
# macOS の例
cargo build --release --features candle,player,candle-metal

# Linux + CUDA の例
cargo build --release --features candle,player,candle-cuda
```

`candle` / `player` を有効にせず実行すると、エラーメッセージを出して終了します。

### 音声（`hisui ml audio`）

`player` は不要です。

```bash
cargo build --release --features candle
```

Silero VAD 用の `candle-onnx` をビルドするには **`protoc`** が必要です。

```bash
# macOS
brew install protobuf
```

---

## モデルデータの準備

初回は Hugging Face から重みを取得します。付属スクリプトで `ml-models/` にまとめて保存できます（`.gitignore` 済み）。

```bash
chmod +x ./scripts/download_ml_models.sh
./scripts/download_ml_models.sh          # whisper-tiny + silero-vad + YOLOv8s（既定）
./scripts/download_ml_models.sh whisper  # Whisper のみ
./scripts/download_ml_models.sh vad      # Silero VAD のみ
./scripts/download_ml_models.sh yolo     # YOLO のみ
```

保存先を変える場合:

```bash
ML_MODELS_DIR=/path/to/models ./scripts/download_ml_models.sh
```

| 用途 | 保存先（既定） | コマンドでの指定 |
|------|----------------|------------------|
| Whisper 転写 | `ml-models/whisper-tiny/` | `--model-dir ml-models/whisper-tiny` |
| Silero VAD | `ml-models/silero-vad/onnx/model.onnx` | 省略可（`--vad-kind auto` で自動検出） |
| YOLO 物体検出 | `ml-models/yolo/yolov8s.safetensors` | `--model-path ml-models/yolo/yolov8s.safetensors` |
| YOLO ポーズ | `ml-models/yolo/yolov8s-pose.safetensors` | `--model-path ... --model pose` |

Whisper を `huggingface-cli` で取得する例:

```bash
huggingface-cli download openai/whisper-tiny --local-dir ./ml-models/whisper-tiny
```

---

## 映像: `hisui ml`

### モデル

[lmz/candle-yolo-v8](https://huggingface.co/lmz/candle-yolo-v8) の safetensors が必要です。**YOLOv8 Small のみ**対応しています。

| `--model` | 重みファイル |
|-----------|--------------|
| `detect`（既定） | `yolov8s.safetensors` |
| `pose` | `yolov8s-pose.safetensors` |

`yolov8n` / `yolov8m` など他サイズはロードできません。

### デバイス一覧

```console
$ hisui ml --list-devices
  <device-id>  <device-name>
  ...
```

`--device-id` 省略時はデフォルトのビデオデバイスを使います。

### オプション

| オプション | 既定値 | 説明 |
|------------|--------|------|
| `--model-path` * | — | safetensors モデルファイル |
| `--device-id` | （デフォルト） | ビデオデバイス ID（複数指定可） |
| `--width` | `320` | キャプチャ幅（px） |
| `--height` | `240` | キャプチャ高さ（px） |
| `--fps` | `30` | キャプチャ FPS |
| `--device` | `auto` | `auto` / `cpu` / `metal` / `cuda` |
| `--model` | `detect` | `detect` / `pose` |
| `--model-size` | `320` | 推論入力の長辺（32 の倍数） |
| `--confidence` | `0.25` | 検出の信頼度しきい値 |
| `--nms` | `0.45` | NMS の IoU しきい値 |
| `--list-devices` | — | デバイス一覧を表示して終了 |
| `--verbose` | — | 詳細ログ（グローバルオプション） |

\* `--model-path` は必須。

`--device auto` は Metal → CUDA → CPU の順で利用可能なデバイスを選びます。

### 実行例

単一カメラ（物体検出）:

```console
$ hisui ml \
    --model-path ./ml-models/yolo/yolov8s.safetensors \
    --device-id "<device-id>" \
    --width 640 --height 480 \
    --verbose
```

ポーズ推定:

```console
$ hisui ml \
    --model-path ./ml-models/yolo/yolov8s-pose.safetensors \
    --model pose \
    --device metal
```

デュアルカメラ（横並び）:

`--device-id` を 2 回指定すると、モデルを `Arc` 共有して 2 路に適用し、`VideoRealtimeMixer` で横並び合成します。表示幅は `width × 2` です。

```console
$ hisui ml \
    --model-path ./ml-models/yolo/yolov8s.safetensors \
    --device-id "<camera-a>" \
    --device-id "<camera-b>" \
    --width 320 --height 240
```

ウィンドウを閉じるとパイプラインが停止します。

### 処理の流れ

```text
VideoDeviceSource (I420)
  → MlProcessor (YOLOv8 推論・描画)
  → DisplaySink → raw_player ウィンドウ
  （2 路の場合: VideoRealtimeMixer で横並び合成）
```

- `detect`: バウンディングボックスを I420 上に描画
- `pose`: キーポイントと骨格を描画
- 検出が無いフレームは元映像をそのまま出力

### Tips（映像）

- **速度**: `--model-size` を小さく、`--width` / `--height` を下げる（`yolov8n` 等は未対応）
- **感度**: 検出が少ない → `--confidence` を下げる。誤検出が多い → 上げる
- **重複枠**: `--nms` を調整する
- **CPU のみ**: `--device cpu`

---

## 音声: `hisui ml audio`

### モデル

`--model-dir` に次の 3 ファイルが同一ディレクトリにある必要があります。

- `config.json`
- `tokenizer.json`
- `model.safetensors`

80 mel bin の Whisper（tiny / base / small 等）を想定しています。

### デバイス一覧

```console
$ hisui ml audio --list-audio-devices
  <device-id>  <device-name>
  ...
```

### オプション

#### Whisper

| オプション | 既定値 | 説明 |
|------------|--------|------|
| `--model-dir` * | — | Whisper モデルディレクトリ |
| `--device-id` | （デフォルト） | オーディオ入力デバイス ID |
| `--chunk-secs` | `10` | 転写チャンク長（秒） |
| `--device` | `auto` | `auto` / `cpu` / `metal` / `cuda` |
| `--language` | （自動） | 言語コード（`en`, `ja` 等）。省略時は多言語モデルで初回チャンクから推定 |
| `--task` | `transcribe` | `transcribe` / `translate` |
| `--list-audio-devices` | — | 入力デバイス一覧を表示して終了 |
| `--verbose` | — | 詳細ログ（グローバルオプション） |

\* `--model-dir` は必須。

#### VAD（発話検出）

| オプション | 既定値 | 説明 |
|------------|--------|------|
| `--vad-kind` | `auto` | `auto` / `silero` / `energy` / `off` |
| `--vad` | 無効 | Silero が使えないとき RMS VAD を有効化 |
| `--vad-model` | （自動） | Silero VAD ONNX のパス |
| `--vad-probability` | `0.35` | Silero: チャンク平均発話確率の下限 |
| `--vad-trim` | 無効 | Silero: 発話フレームのみ Whisper に渡す |
| `--vad-min-speech-ratio` | `0.05` | RMS: 発話フレーム比率の下限 |
| `--vad-rms-threshold` | `0.01` | RMS: フレーム RMS の下限 |

**VAD の動作（`--vad-kind`）**

| 値 | 条件 | 動作 |
|----|------|------|
| `auto` | `ml-models/silero-vad/onnx/model.onnx` あり | Silero VAD |
| `auto` | Silero なし + `--vad` | RMS VAD |
| `auto` | どちらもなし | VAD 無効 |
| `silero` | ONNX 必須 | Silero VAD |
| `energy` | — | RMS VAD |
| `off` | — | 全チャンクを Whisper に渡す |

Silero は `candle-onnx` で [onnx-community/silero-vad](https://huggingface.co/onnx-community/silero-vad) を推論します。無音チャンクは Whisper をスキップします。`--vad-trim` はチャンク内の発話区間だけを切り出してから転写します。

### 実行例

```console
$ hisui ml audio \
    --model-dir ./ml-models/whisper-tiny \
    --vad-trim \
    --language ja \
    --verbose
```

`download_ml_models.sh` で Silero を取得済みなら、`--vad` なしでも `--vad-kind auto`（既定）で Silero が有効になります。

転写結果は `transcript: ...` としてログに出力されます。Ctrl+C で終了します。

### 処理の流れ

```text
AudioDeviceSource (I16Be, 48 kHz)
  → 48 kHz → 16 kHz（自前 FIR + 3:1 間引き）
  → VAD（Silero または RMS）
  → Whisper 転写
  → tracing ログ
```

入力サンプルレートは **48 kHz のみ** 対応です（マイクキャプチャの既定）。それ以外はエラーになります。

### Tips（音声）

- **初回セットアップ**: `./scripts/download_ml_models.sh` で Whisper + Silero を取得
- **無音が多い**: `--vad-trim` と `--vad-probability` の調整
- **Silero なしで試す**: `--vad --vad-kind energy`
- **言語固定**: 多言語モデルで `--language ja` などを指定すると自動推定をスキップ
- **CPU のみ**: `--device cpu`

---

## 制限事項

### 共通

- Candle 統合は実験段階です。CI の通常ビルドでは `candle` feature は有効になっていません。
- macOS 以外での動作は十分に検証されていない可能性があります。

### 映像

- 対応モデルは YOLOv8 Small の detect / pose のみ。
- 録画ファイルや Sora 配信など、カメラ以外のソース入力は `ml` サブコマンドでは提供していません（MediaPipeline の `MlProcessor` として他ソースの後段接続は可能）。

### 音声

- 入力は 48 kHz mono（ステレオ入力は左右平均で mono 化）に限定。
