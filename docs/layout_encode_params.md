# エンコード設定の指定方法

[レイアウト機能](./layout.md) で指定する JSON ファイルでは、
合成後の映像や音声をエンコードするコーデックやエンコードパラメーターを指定することができます。

エンコードコーデックやパラメーターによって、合成結果の品質やサイズ、
合成の要する時間が大きく変わる可能性があるため、
Hisui を最大限活用するためには、これらを適切に指定することが重要です。

実際に利用可能なコーデックは、Hisui のビルド方法や実行環境で変わりますが、
[`hisui list-codecs`](./command_list_codecs.md) コマンドで一覧を取得することができます。

なお [`hisui tune`](./command_tune.md) コマンドを利用することで、
適切なエンコードパラメーターをある程度自動で調整することができます。

## 音声エンコードコーデックの指定

合成後の音声のエンコードコーデックは、以下のように `audio_codec` フィールドで指定できます。

```json
{
  "audio_codec": "OPUS",
  "audio_sources": ["archive-*.json"]
}
```

`audio_codec` で指定可能な値は以下の通りです:
- `"OPUS"`: Opus音声コーデック（デフォルト）
- `"AAC"`: AAC音声コーデック

### 注意

`"AAC"` は、以下のいずれかの条件を満たしている場合にのみ指定可能です：
- macOS 用にビルドされた Hisui（Apple Audio Toolboxの AAC エンコーダーが使用されます）
- FDK-AAC を有効にしてビルドされた Hisui

公開されているビルド済みバイナリは FDK-AAC には対応していません。
FDK-AAC を利用する場合は、[ビルド方法](build.md) を参考にして、自前でのビルドを行ってください。

## 音声エンコードビットレートの指定

映像とは異なり、音声はエンコード時のパラメーターが少なく、
エンコードビットレートのみが指定可能となっています。

エンコードビットレートは、以下のように `audio_bitrate` フィールドで bps 単位で指定します。

```json
{
  "audio_codec": "OPUS",
  "audio_bitrate": 65536,
  "audio_sources": ["archive-*.json"]
}
```

`audio_bitrate` のデフォルト値は 65536 です。

## 映像エンコードコーデックの指定

合成後の映像のエンコードコーデックは、レイアウト JSON の `video_codec` フィールドで指定できます。

```json
{
  "video_codec": "VP8",
  "video_layout": {
    "main": {
      "video_sources": ["archive-*.json"]
    }
  }
}
```

`video_codec` で指定可能な値は以下の通りです：

- `"VP8"`: VP8 映像コーデック（デフォルト）
- `"VP9"`: VP9 映像コーデック
- `"H264"`: H.264 映像コーデック
- `"H265"`: H.265 映像コーデック
- `"AV1"`: AV1 映像コーデック

### 注意

`"H264"` は、以下のいずれかの条件を満たしている場合にのみ指定可能です：
- **macOS 用にビルドされた Hisui**: Apple Video Toolbox の H.264 エンコーダーが使用されます
- **OpenH264 オプション指定時**: [`hisui compose`](command_compose.md) などのコマンドで `--openh264` オプションや `HISUI_OPENH264_PATH` 環境変数が指定された場合

`"H265"` は、以下の条件を満たしている場合にのみ指定可能です：
- **macOS 用にビルドされた Hisui**: Apple Video Toolbox の H.265 エンコーダーが使用されます

## 映像エンコードビットレートの指定

映像エンコードビットレートは、レイアウト JSON の `video_bitrate` フィールドで bps 単位で指定できます。

```json
{
  "video_codec": "VP8",
  "video_bitrate": 1048576,
  "video_layout": {
    "main": {
      "video_sources": ["archive-*.json"]
    }
  }
}
```

`video_bitrate` のデフォルト値は `映像ソースの数 * 200 * 1024` です。

### 注意

[レガシー版の Hisui](./hisui_legacy.md) との互換性維持のため、`bitrate` フィールド（kbps単位）も利用可能ですが、両方が指定された場合には `video_bitrate` が優先されます。

## 映像エンコーダー固有のパラメーターセットの指定

Hisui は映像のエンコーダーとして、以下をサポートしています:
- **libvpx**: VP8 / VP9 用のエンコーダー
- **OpenH264**: H.264 用のエンコーダー
- **SVT-AV1**: AV1 用のエンコーダー
- **Apple Video Toolbox**: macOS で利用可能な H.264 / H.265 用のエンコーダー

映像エンコーダーの種類とエンコードコーデックの組み合わせによって、指定可能なパラメーターセットは変わります。

例えば `libvpx` エンコーダーで `VP8` コーデックでエンコードを行う場合のパラメーターセットは、
以下のように、`libvpx_vp8_encode_params` をキーとした JSON オブジェクトを使って指定します:
```json
{
  "video_codec": "VP8",
  "libvpx_vp8_encode_params": {
    "cpu_used": 4,
    "min_quantizer": 4,
    "max_quantizer": 56
  }
}
```

エンコーダー固有パラメーターセット用のキーの一覧は以下の通りです：

- `libvpx_vp8_encode_params`: libvpx で VP8 エンコードを行う際のパラメーターセット
- `libvpx_vp9_encode_params`: libvpx で VP9 エンコードを行う際のパラメーターセット
- `openh264_encode_params`: OpenH264 で H.264 エンコードを行う際のパラメーターセット
- `svt_av1_encode_params`: SVT-AV1 で AV1 エンコードを行う際のパラメーターセット
- `video_toolbox_h264_encode_params`: Apple Video Toolbox で H.264 エンコードを行う際のパラメーターセット
- `video_toolbox_h265_encode_params`: Apple Video Toolbox で H.265 エンコードを行う際のパラメーターセット


### `libvpx_vp8_encode_params` で指定可能なパラメーターセット

`libvpx` で VP8 エンコードを行う際に指定可能なパラメーターは以下の通りです.

TODO: デフォルトおよび範囲の記述は暫定なので後でちゃんとする

#### 基本的なエンコーダーパラメーター

- `min_quantizer` (整数値): 最小量子化パラメーター値
  - デフォルト値: TBD
  - 指定可能な範囲: 4 〜 29

- `max_quantizer` (整数値): 最大量子化パラメーター値
  - デフォルト値: 50
  - 指定可能な範囲: 30 〜 58

- `cq_level` (整数値): 固定品質エンコード時の品質レベル
  - デフォルト値: 30
  - 指定可能な範囲: 0 〜 63

- `cpu_used` (整数値): エンコード速度と品質のバランス調整
  - デフォルト値: なし（必須項目）
  - 指定可能な範囲: 0 〜 16
  - 値が大きいほど高速だが品質が低下

#### エンコード制御パラメーター

- `deadline` (文字列): エンコード期限設定
  - デフォルト値: `"good"`
  - 指定可能な値: `"best"`, `"good"`, `"realtime"`

- `rate_control` (文字列): レート制御モード
  - デフォルト値: `"vbr"`
  - 指定可能な値: `"vbr"`, `"cbr"`, `"cq"`

- `lag_in_frames` (整数値): 先読みフレーム数
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 1 〜 25

- `threads` (整数値): エンコードに使用するスレッド数
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 1 〜 16

- `error_resilient` (真偽値): エラー耐性モードの有効化
  - デフォルト値: `false`

- `keyframe_interval` (整数値): キーフレーム間隔
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 1 〜 600

- `frame_drop_threshold` (整数値): フレームドロップ閾値
  - デフォルト値: なし（省略可能）

#### VP8 固有のパラメーター

- `noise_sensitivity` (整数値): ノイズ感度設定
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 0 〜 6

- `static_threshold` (整数値): 静的領域検出閾値
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 0 〜 10000

- `token_partitions` (整数値): トークン分割数
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 0 〜 3

- `max_intra_bitrate_pct` (整数値): イントラフレームの最大ビットレート（%）
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 0 〜 10000

- `arnr_config` (オブジェクト): Altref Noise Reduction 設定
  - `max_frames` (整数値): 最大フレーム数（デフォルト: 0, 範囲: 0 〜 15）
  - `strength` (整数値): フィルター強度（デフォルト: 3, 範囲: 0 〜 6）
  - `filter_type` (整数値): フィルタータイプ（デフォルト: 1, 指定可能な値: 1, 2, 3）

#### 指定例

```json
{
  "video_codec": "VP8",
  "libvpx_vp8_encode_params": {
    "cpu_used": 4,
    "min_quantizer": 4,
    "max_quantizer": 56,
    "cq_level": 30,
    "deadline": "good",
    "rate_control": "vbr",
    "threads": 8,
    "keyframe_interval": 120,
    "noise_sensitivity": 1,
    "arnr_config": {
      "max_frames": 7,
      "strength": 5,
      "filter_type": 1
    }
  }
}
```

### `libvpx_vp9_encode_params` で指定可能なパラメーターセット

`libvpx_vp9_encode_params` で指定可能なパラメーターセットは以下の通りです。

TODO: デフォルトおよび範囲の記述は暫定なので後でちゃんとする

## 基本的なエンコーダーパラメーター

- `min_quantizer` (整数値): 最小量子化パラメーター値
  - デフォルト値: 10
  - 指定可能な範囲: 4 〜 29

- `max_quantizer` (整数値): 最大量子化パラメーター値
  - デフォルト値: 50
  - 指定可能な範囲: 30 〜 58

- `cq_level` (整数値): 固定品質エンコード時の品質レベル
  - デフォルト値: 30
  - 指定可能な範囲: 0 〜 63

- `cpu_used` (整数値): エンコード速度と品質のバランス調整
  - デフォルト値: なし（必須項目）
  - 指定可能な範囲: 0 〜 9
  - 値が大きいほど高速だが品質が低下

## エンコード制御パラメーター

- `deadline` (文字列): エンコード期限設定
  - デフォルト値: `"good"`
  - 指定可能な値: `"best"`, `"good"`, `"realtime"`

- `rate_control` (文字列): レート制御モード
  - デフォルト値: `"vbr"`
  - 指定可能な値: `"vbr"`, `"cbr"`, `"cq"`

- `lag_in_frames` (整数値): 先読みフレーム数
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 1 〜 25

- `threads` (整数値): エンコードに使用するスレッド数
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 1 〜 16

- `error_resilient` (真偽値): エラー耐性モードの有効化
  - デフォルト値: `false`

- `keyframe_interval` (整数値): キーフレーム間隔
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 1 〜 600

- `frame_drop_threshold` (整数値): フレームドロップ閾値
  - デフォルト値: なし（省略可能）

## VP9 固有のパラメーター

- `aq_mode` (整数値): 適応量子化モード
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 0 〜 3

- `noise_sensitivity` (整数値): ノイズ感度設定
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 0 〜 6

- `tile_columns` (整数値): タイル分割の列数（log2値）
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 0 〜 6

- `tile_rows` (整数値): タイル分割の行数（log2値）
  - デフォルト値: なし（省略可能）
  - 指定可能な範囲: 0 〜 2

- `row_mt` (真偽値): 行単位マルチスレッドの有効化
  - デフォルト値: `false`

- `frame_parallel_decoding` (真偽値): フレーム並列デコーディングの有効化
  - デフォルト値: `false`

- `tune_content` (文字列): コンテンツタイプ最適化設定
  - デフォルト値: なし（省略可能）
  - 指定可能な値: `"default"`, `"screen"`

## 指定例

```json
{
  "video_codec": "VP9",
  "libvpx_vp9_encode_params": {
    "cpu_used": 2,
    "min_quantizer": 4,
    "max_quantizer": 56,
    "cq_level": 30,
    "deadline": "good",
    "rate_control": "vbr",
    "threads": 8,
    "keyframe_interval": 120,
    "aq_mode": 3,
    "noise_sensitivity": 1,
    "tile_columns": 1,
    "tile_rows": 0,
    "row_mt": true,
    "frame_parallel_decoding": false,
    "tune_content": "default"
  }
}
```
