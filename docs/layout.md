# レイアウト機能

レイアウト JSON は、複数の映像・音声ソースを合成する際の配置や設定を定義するための設定ファイルです。

## 表記法について

このドキュメントでは、JSON 構造を説明する際に以下の表記法を使用します。

### `$VARIABLE` 表記について

このドキュメントでは、JSON の値として任意の値を取りうる箇所を `$VARIABLE` という形式で表記しています。
これらは実際の JSON では具体的な値に置き換える必要があります。

例：
- `$AUDIO_CODEC_NAME` → `"OPUS"` や `"AAC"` などの文字列
- `$BITRATE` → `65536` などの数値
- `$SOURCE_FILE_NAME` → `"archive.json"` などのファイルパス
- `$INTEGER` → `640` などの整数値
- `$BOOLEAN` → `true` または `false`

### ネストした JSON オブジェクトのメンバー表記

`root_name.child_name` の形式で、ネストしたオブジェクトのメンバーを示します。

例えば、以下のような JSON の場合、`video_layout.main.max_columns` という表記で、
一番内側の `max_columns` メンバーを参照しているものとします。

```json
{
  "video_layout": {
    "main": {
      "max_columns": 2
    }
  }
}
```

また、オブジェクトのメンバーの名前が可変の場合には、
そこで任意の文字列を取りえることを示すために `$NAME` という形式で記載することがあります。

上の `max_columns` の例の場合には、
具体的な JSON オブジェクトでの `max_columns` の値に言及する時には `video_layout.main.main_max_columns` と記載しますが、
そうではなく、一般的な仕様の説明の際には `video_layout.$REGION_NAME.max_columns` と記載します。

## レイアウト JSON の 仕様

### 指定可能な項目一覧

以下はレイアウトで指定可能な項目を全て記載した JSON です。
各項目の詳細については以降で説明します。

```json
{
  "audio_codec": $AUDIO_CODEC_NAME,
  "audio_bitrate": $BITRATE,
  "audio_sources": [ $SOURCE_FILE_NAME ],
  "audio_source_excluded": [ $SOURCE_FILE_NAME ],
  "video_codec": $VIDEO_CODEC_NAME,
  "video_bitrate": $BITRATE,
  "resolution": $RESOLUTION,
  "video_layout": { $REGION_NAME: {
    "video_sources": [ $SOURCE_FILE_NAME ],
    "video_sources_excluded": [ $SOURCE_FILE_NAME ],
    "cells_excluded": [ $CELL_INDEX ],
    "width": $INTEGER,
    "height": $INTEGER,
    "cell_width": $INTEGER,
    "cell_height": $INTEGER,
    "max_columns": $INTEGER,
    "max_rows": $INTEGER,
    "reuse": $REUSE_KIND,
    "x_pos": $INTEGER,
    "y_pos": $INTEGER,
    "z_pos": $INTEGER
  },
  "frame_rate": $FRAME_RATE,
  "bitrate": $BITRATE_KBPS,
  "libvpx_vp8_encode_params": $PARAMS,
  "libvpx_vp9_encode_params": $PARAMS,
  "openh264_encode_params": $PARAMS,
  "svt_av1_encode_params": $PARAMS,
  "video_toolbox_h264_encode_params": $PARAMS,
  "video_toolbox_h265_encode_params": $PARAMS,
  "trim": $BOOLEAN
}
```

この中で必須項目は `video_sources` のみで、それ以外は省略された場合にはデフォルト値が使用されます。
なお `video_sources` を包含する `video_layout` 自体を省略することは可能です（その場合は映像ストリームが合成対象から外されます）。

### 各項目の詳細

#### `audio_codec: $AUDIO_CODEC_NAME`

合成後の音声のエンコードに使用するコーデックを指定します。

`$AUDIO_CODEC_NAME` に指定可能な値は以下の通りです：
- `"OPUS"` （デフォルト）
- `"AAC"`

`"AAC"` は、以下のいずれかの条件を満たしている場合にのみ指定可能です:
- macOS 用にビルドされた Hisui（Apple Audio Toolboxの AAC エンコーダーが使用されます）
- FDK-AAC を有効にしてビルドされた Hisui（参考: [build.md](build.md)）

#### `audio_bitrate: $BITRATE`

合成後の音声のエンコードビットレートを指定します（bps 単位）。

デフォルト値は `65536` です。

#### `audio_sources: [ $SOURCE_FILE_NAME ]`

音声合成のソースとなるファイル（JSON）のパスを配列で指定します。

デフォルト値は `[]` で、音声なしの合成を意味します。

TODO:
- glob (wildcard) パターン
- ROOT_DIR との関係
- ソースファイルの詳細について書く
  - Sora の録画との関係
  - どういった内容のファイルか
  - メディアファイルとの関係

#### `audio_source_excluded: [ $SOURCE_FILE_NAME ]`

音声合成から除外するソースファイルのパスを配列で指定します。

デフォルト値は `[]` です。

`$SOURCE_FILE_NAME` の詳細については `audio_sources` の説明を参照してください。

#### `video_codec: $VIDEO_CODEC_NAME`

合成後の映像のエンコードに使用するコーデックを指定します。

`$VIDEO_CODEC_NAME` に指定可能な値は以下の通りです：

- `"VP8"` （デフォルト）
- `"VP9"`
- `"H264"`
- `"H265"`
- `"AV1"`

`"H264"` は、以下のいずれかの条件を満たしている場合にのみ指定可能です:
- macOS 用にビルドされた Hisui（Apple Video Toolboxのエンコーダーが使用されます）
- [`hisui compose`](command_compose.md)  などのコマンドの引数で `--openh264` オプションが指定された場合

`"H265"` は、以下の条件を満たしている場合にのみ指定可能です:
- macOS 用にビルドされた Hisui（Apple Video Toolboxのエンコーダーが使用されます）

#### `video_bitrate: $BITRATE`

合成後の映像のエンコードビットレートを指定します（bps 単位）。

デフォルト値は `映像ソースの数 * 200 * 1024` です。

**注意**: レガシー版の Hisui との互換性のため、`bitrate` フィールド（kbps単位）も利用可能ですが、両方が指定された場合には `video_bitrate` が優先されます。

#### `resolution: $RESOLUTION`

合成後の映像の解像度を指定します。

`$RESOLUTION` は `"幅x高さ"` の形式で指定します（例: `"1920x1080"`）。

指定可能な値の範囲は以下の通りです：
- 幅・高さともに 16 ピクセル以上 3840 ピクセル以下
- 幅・高さともに偶数値（奇数が指定された場合は自動的に偶数に丸められます）

この項目が省略された場合には、`video_layout` で定義されたリージョンのサイズと位置から自動的に全体の解像度が計算されます。

TODO: 解像度周りは複雑なところなので、専用のセクションを用意する

#### `video_layout.$REGION_NAME.video_sources: [ $SOURCE_FILE_NAME ]`

指定されたリージョンの映像合成に使用するソースファイルのパスを配列で指定します。

#### `video_layout.$REGION_NAME.video_sources_excluded: [ $SOURCE_FILE_NAME ]`

指定されたリージョンの映像合成から除外するソースファイルのパスを配列で指定します。

#### `video_layout.$REGION_NAME.cells_excluded: [ $CELL_INDEX ]`

指定されたリージョンで除外するセルのインデックスを配列で指定します。

#### `video_layout.$REGION_NAME.width: $INTEGER`

指定されたリージョンの幅をピクセル単位で指定します。

#### `video_layout.$REGION_NAME.height: $INTEGER`

指定されたリージョンの高さをピクセル単位で指定します。

#### `video_layout.$REGION_NAME.cell_width: $INTEGER`

指定されたリージョンのセルの幅をピクセル単位で指定します。

#### `video_layout.$REGION_NAME.cell_height: $INTEGER`

指定されたリージョンのセルの高さをピクセル単位で指定します。

#### `video_layout.$REGION_NAME.max_columns: $INTEGER`

指定されたリージョンのグリッドの最大列数を指定します。

#### `video_layout.$REGION_NAME.max_rows: $INTEGER`

指定されたリージョンのグリッドの最大行数を指定します。

#### `video_layout.$REGION_NAME.reuse: $REUSE_KIND`

指定されたリージョンでのセル再利用方法を指定します。

#### `video_layout.$REGION_NAME.x_pos: $INTEGER`

指定されたリージョンのX座標をピクセル単位で指定します。

#### `video_layout.$REGION_NAME.y_pos: $INTEGER`

指定されたリージョンのY座標をピクセル単位で指定します。

#### `video_layout.$REGION_NAME.z_pos: $INTEGER`

指定されたリージョンのZ座標を指定します。

#### `frame_rate: $FRAME_RATE`

出力映像のフレームレートを指定します。

#### `bitrate: $BITRATE_KBPS`

出力映像のビットレートをkbps単位で指定します。

#### `libvpx_vp8_encode_params: $PARAMS`

libvpx で VP8 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [encode.md](./encode.md) を参照してください。

#### `libvpx_vp9_encode_params: $PARAMS`

libvpx で VP9 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [encode.md](./encode.md) を参照してください。

#### `openh264_encode_params: $PARAMS`

OpenH264 で H.264 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [encode.md](./encode.md) を参照してください。

#### `svt_av1_encode_params: $PARAMS`

SVT-AV1 で AV1 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [encode.md](./encode.md) を参照してください。

#### `video_toolbox_h264_encode_params: $PARAMS`

Apple Video Toolbox で H.264 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [encode.md](./encode.md) を参照してください。

#### `video_toolbox_h265_encode_params: $PARAMS`

Apple Video Toolbox で H.265 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [encode.md](./encode.md) を参照してください。

#### `trim: $BOOLEAN`

配信者が存在しない期間の自動トリミング（除去）を有効にするかどうかを指定します。

`true` を指定すると、音声ないし映像ソースが全く存在しない時間帯は、合成結果に含まれなくなります。

デフォルト値は `false` です。
