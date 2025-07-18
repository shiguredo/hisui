# レイアウト機能

レイアウト JSON は、複数の映像・音声ソースを合成する際の配置や設定を定義するための設定ファイルです。

## レイアウト JSONの 仕様

TODO: Add content

```json
{
}
```

### 基本構造

```json
{
  "audio_sources": ["source1.json", "source2.json"],
  "video_layout": {
    "region_name": {
      "video_sources": ["source1.json", "source2.json"],
      "max_columns": 2,
      "width": 640,
      "height": 480
    }
  },
  "resolution": "1920x1080",
  "trim": false,
  "audio_codec": "opus",
  "video_codec": "vp8",
  "frame_rate": "25"
}
```

### フィールド詳細

#### 音声関連

- **`audio_sources`** (配列): 音声合成に使用するソースファイルのパス
- **`audio_sources_excluded`** (配列): 除外する音声ソースのパス（ワイルドカード対応）
- **`audio_codec`** (文字列): 音声コーデック（`opus`, `aac` など）
- **`audio_bitrate`** (数値): 音声ビットレート（bps）

#### 映像関連

- **`video_layout`** (オブジェクト): 映像リージョンの定義
  - **`video_sources`** (配列, 必須): 映像ソースファイルのパス
  - **`video_sources_excluded`** (配列): 除外する映像ソースのパス
  - **`max_columns`** (数値): グリッドの最大列数
  - **`max_rows`** (数値): グリッドの最大行数
  - **`width`** (数値): リージョンの幅（ピクセル）
  - **`height`** (数値): リージョンの高さ（ピクセル）
  - **`cell_width`** (数値): セルの幅（`width`と併用不可）
  - **`cell_height`** (数値): セルの高さ（`height`と併用不可）
  - **`x_pos`** (数値): リージョンのX座標
  - **`y_pos`** (数値): リージョンのY座標
  - **`z_pos`** (数値): リージョンのZ座標（-99〜99）
  - **`cells_excluded`** (配列): 除外するセルのインデックス
  - **`reuse`** (文字列): セル再利用方法（`none`, `show_oldest`, `show_newest`）

#### 全体設定

- **`resolution`** (文字列): 出力解像度（例: `"1920x1080"`）
- **`trim`** (真偽値): 無音部分の自動トリミング
- **`video_codec`** (文字列): 映像コーデック（`vp8`, `vp9`, `h264`, `h265`, `av1`）
- **`video_bitrate`** (数値): 映像ビットレート（bps）
- **`frame_rate`** (文字列): フレームレート（例: `"25"`, `"30"`）

#### エンコーダー固有パラメータ

- **`libvpx_vp8_encode_params`** (オブジェクト): VP8エンコーダーパラメータ
- **`libvpx_vp9_encode_params`** (オブジェクト): VP9エンコーダーパラメータ
- **`openh264_encode_params`** (オブジェクト): OpenH264エンコーダーパラメータ
- **`svt_av1_encode_params`** (オブジェクト): SVT-AV1エンコーダーパラメータ
- **`video_toolbox_h264_encode_params`** (オブジェクト): VideoToolbox H.264パラメータ（macOS）
- **`video_toolbox_h265_encode_params`** (オブジェクト): VideoToolbox H.265パラメータ（macOS）

### グリッドレイアウト

映像ソースは自動的にグリッド状に配置されます。グリッドのサイズは以下の要素で決定されます：

1. **最大同時ソース数**: 時間的に重複するソースの最大数
2. **制約**: `max_columns` / `max_rows` の指定
3. **除外セル**: `cells_excluded` で指定されたセル

### ソース指定

ソースファイルはワイルドカード（`*`）を使用できます：

```json
{
  "video_sources": ["recordings/*.json"],
  "video_sources_excluded": ["recordings/test_*.json"]
}
```

### 使用例

#### 基本的な2x2グリッド

```json
{
  "audio_sources": ["*.json"],
  "video_layout": {
    "main": {
      "video_sources": ["*.json"],
      "max_columns": 2,
      "width": 640,
      "height": 480
    }
  },
  "resolution": "640x480"
}
```

#### 複数リージョンの配置

```json
{
  "video_layout": {
    "main": {
      "video_sources": ["participant_*.json"],
      "width": 1280,
      "height": 720,
      "x_pos": 0,
      "y_pos": 0,
      "z_pos": 0
    },
    "overlay": {
      "video_sources": ["screen_share.json"],
      "width": 320,
      "height": 180,
      "x_pos": 1600,
      "y_pos": 900,
      "z_pos": 1
    }
  },
  "resolution": "1920x1080"
}
```
