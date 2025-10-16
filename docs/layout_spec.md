# レイアウト JSON の 仕様

## JSONC (JSON with Comments) 対応

レイアウト JSON ファイルの拡張子が `.jsonc` の場合には、以下の JSONC の仕様が有効になります:

- `//` による行コメント
- `/* ... */` によるブロックコメント
- 配列およびオブジェクトの末尾要素の後ろのカンマを許容

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

## レイアウト JSON で指定可能な項目一覧

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
  "video_encoders": [ $ENCODER_NAME ],
  "video_decoders": [ $DECODER_NAME ],
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
    "z_pos": $INTEGER,
    "border_pixels": $INTEGER
  },
  "frame_rate": $FRAME_RATE,
  "bitrate": $BITRATE_KBPS,
  "libvpx_vp8_encode_params": $PARAMS,
  "libvpx_vp9_encode_params": $PARAMS,
  "openh264_encode_params": $PARAMS,
  "svt_av1_encode_params": $PARAMS,
  "video_toolbox_h264_encode_params": $PARAMS,
  "video_toolbox_h265_encode_params": $PARAMS,
  "nvcodec_h264_encode_params": $PARAMS,
  "nvcodec_h265_encode_params": $PARAMS,
  "nvcodec_av1_encode_params": $PARAMS,
  "nvcodec_h264_decode_params": $PARAMS,
  "nvcodec_h265_decode_params": $PARAMS,
  "nvcodec_vp8_decode_params": $PARAMS,
  "nvcodec_vp9_decode_params": $PARAMS,
  "nvcodec_av1_decode_params": $PARAMS,
  "trim": $BOOLEAN
}
```

トップレベルの項目は全てデフォルト値があり、省略可能です。

`video_layout` を指定する場合、各リージョン内で必須項目は `video_sources` のみで、
それ以外は省略された場合にはデフォルト値が使用されます。

`video_layout` 以下の項目の扱いについては [layout_region.md](./layout_region.md) も参照してください。

## レイアウト JSON の各項目の詳細

### `audio_codec: $AUDIO_CODEC_NAME`

合成後の音声のエンコードに使用するコーデックを指定します。

`$AUDIO_CODEC_NAME` に指定可能な値は以下の通りです：

- `"OPUS"` （デフォルト）
- `"AAC"`

`"AAC"` は、以下のいずれかの条件を満たしている場合にのみ指定可能です:

- macOS 用にビルドされた Hisui（Apple Audio Toolboxの AAC エンコーダーが使用されます）
- FDK-AAC を有効にしてビルドされた Hisui（参考: [build.md](build.md)）

### `audio_bitrate: $BITRATE`

合成後の音声のエンコードビットレートを指定します（bps 単位）。

デフォルト値は `65536` です。

### `audio_sources: [ $SOURCE_FILE_NAME ]`

音声合成のソースとなるファイル（JSON）のパスを配列で指定します。

デフォルト値は `[]` で、音声なしの合成を意味します。

**ソース JSON ファイルについて**

通常、ソース JSON ファイルには、
Sora が録画時に配信者毎に生成する `archive-{ CONNECTION_ID }.json` ファイルを指定します。

Hisui は、この `archive.json` ファイルの中の以下の情報を参照します:

```jsonc
{
  "connection_id": "コネクション ID",
  "format": "webm" | "mp4", // 省略時は "webm" 扱い
  "audio": true | false,
  "video": true | false,
  "start_time_offset": 開始時刻（秒）,
  "stop_time_offset": 終了時刻（秒）
}
```

また、ソース JSON ファイルに対応するメディアファイルが、
ソース JSON ファイルの拡張子を `.mp4` ないし `.webm` に変えたパスに存在する、と想定しています。

**ソース JSON ファイルのパス指定について**

ソース JSON ファイルのパスが相対パスの場合には、
[`hisui compose`](./command_compose.md) コマンドなどの `ROOT_DIR` 引数で指定した値がベースパスとして扱われます。

以下の場合にはエラーとなります:

- `ROOT_DIR` の外のパスが指定された場合
- ソース JSON ファイルに対応するメディアファイルが存在しない場合

**ワイルドカードパターンについて**

ソース JSON ファイルのパス指定では、ファイル名部分でワイルドカード（`*`）を使用できます。

通常の合成では、以下のようにワイルドカードを使って、一括で合成対象を指定するのが便利です:

```json
{
  "audio_sources": ["archive-*.json"],
  "video_layout": {
    "main": {
      "video_sources": ["archive-*.json"],
      "max_columns": 3
    }
  },
  "resolution": "1920x1080"
}
```

なお、対応するメディアファイルが存在しないソースのパスは、ワイルドカードにマッチしたとしても展開結果から除外されます。

### `audio_source_excluded: [ $SOURCE_FILE_NAME ]`

音声合成から除外するソースファイルのパスを配列で指定します。

デフォルト値は `[]` です。

`$SOURCE_FILE_NAME` の詳細については `audio_sources` の説明を参照してください。

### `video_codec: $VIDEO_CODEC_NAME`

合成後の映像のエンコードに使用するコーデックを指定します。

`$VIDEO_CODEC_NAME` に指定可能な値は以下の通りです：

- `"VP8"`
- `"VP9"` （デフォルト）
- `"H264"`
- `"H265"`
- `"AV1"`

`"H264"` は、以下のいずれかの条件を満たしている場合にのみ指定可能です:

- macOS 用にビルドされた Hisui（Apple Video Toolboxのエンコーダーが使用されます）
- [`hisui compose`](command_compose.md)  などのコマンドの引数で `--openh264` オプションが指定された場合
- nvcodec に対応してビルドされた Hisui（NVIDIA Video Codec SDKのエンコーダーが使用されます）

`"H265"` は、以下のいずれかの条件を満たしている場合にのみ指定可能です:

- macOS 用にビルドされた Hisui（Apple Video Toolboxのエンコーダーが使用されます）
- nvcodec に対応してビルドされた Hisui（NVIDIA Video Codec SDKのエンコーダーが使用されます）

また `"AV1"` は常に指定可能ですが nvcodec に対応してビルドされた Hisui の場合には NVIDIA Video Codec SDKのエンコーダーが優先的に使用されます。

なお自前で Hisui のビルドを行う場合には、nvcodec はデフォルトでは無効になっています。
有効にする方法は [ビルド方法](build.md) をご参照ください。
ubuntu-24.04_x86_64 向けのビルド済みバイナリでは nvcodec が有効になっています（CUDA がない環境では実行時に無効になります）。

### `video_bitrate: $BITRATE`

合成後の映像のエンコードビットレートを指定します（bps 単位）。

デフォルト値は `映像ソースの数 * 200 * 1024` です。

**注意**: レガシー版の Hisui との互換性のため、`bitrate` フィールド（kbps単位）も利用可能ですが、両方が指定された場合には `video_bitrate` が優先されます。

### `video_encoders: [ $ENCODER_NAME ]`

映像エンコード時に使用するエンコーダーの候補を配列で指定します。

デフォルト値は環境に依存し、以下の順序で使用可能なエンジンが自動的に設定されます:

1. `"openh264"` (OpenH264 が引数ないし環境変数経由で指定されている場合)
2. `"nvcodec"` (nvcodec feature が有効になっている場合)
3. `"video_toolbox"` (macOS の場合)
4. `"svt_av1"`
5. `"libvpx"`

配列の先頭に近いエンコーダーほど優先度が高くなります。指定されたコーデックに対応していないものは無視されます。

例:
```json
{
  "video_encoders": ["nvcodec", "svt_av1", "libvpx"],
  "video_codec": "AV1"
}
```

この場合、AV1 エンコードに対して、nvcodec が最優先で使用され、利用できない場合は svt_av1 が使用されます。

### `video_decoders: [ $DECODER_NAME ]`

映像デコード時に使用するデコーダーの候補を配列で指定します。

デフォルト値は環境に依存し、以下の順序で使用可能なエンジンが自動的に設定されます:

1. `"openh264"` (OpenH264 が引数ないし環境変数経由で指定されている場合)
2. `"nvcodec"` (nvcodec feature が有効になっている場合)
3. `"video_toolbox"` (macOS の場合)
4. `"dav1d"`
5. `"libvpx"`

配列の先頭に近いデコードほど優先度が高くなります。指定されたコーデックに対応していないものは無視されます。

例:
```json
{
  "video_decoders": ["nvcodec", "dav1d", "libvpx"]
}
```

この場合、各コーデックのデコードに対して、nvcodec が最優先で使用され、利用できない場合や対応していないコーデックの場合は、以降のデコーダーが使用されます。

### `resolution: $RESOLUTION`

合成後の映像の解像度を指定します。

`$RESOLUTION` は `"幅x高さ"` の形式で指定します（例: `"1920x1080"`）。

指定可能な値の範囲は以下の通りです：

- 幅・高さともに 16 ピクセル以上 3840 ピクセル以下
- 幅・高さともに偶数値（奇数が指定された場合は自動的に偶数に丸められます）

この項目が省略された場合には、`video_layout` で定義されたリージョンのサイズと位置から自動的に全体の解像度が計算されます。

詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.video_sources: [ $SOURCE_FILE_NAME ]`

指定されたリージョンの映像合成のソースとなるファイル（JSON）のパスを配列で指定します。

この項目は各リージョンで必須です。
映像合成を行う場合、少なくとも一つのリージョンで `video_sources` を指定する必要があります。

`$SOURCE_FILE_NAME` の詳細については `audio_sources` の説明を参照してください。

### `video_layout.$REGION_NAME.video_sources_excluded: [ $SOURCE_FILE_NAME ]`

指定されたリージョンの映像合成から除外するソースファイルのパスを配列で指定します。

デフォルト値は `[]` です。

`$SOURCE_FILE_NAME` の詳細については `audio_sources` の説明を参照してください。

### `video_layout.$REGION_NAME.cells_excluded: [ $CELL_INDEX ]`

指定されたリージョンで、映像ソースの割り当てを除外するセルのインデックスを配列で指定します。

リージョンやセルなどの詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.width: $INTEGER`

指定されたリージョンの幅をピクセル単位で指定します。

指定可能な値の範囲は以下の通りです：

- 16 ピクセル以上、`x_pos + width` の値が合成後の映像の全体解像度の幅以下
- 偶数値（奇数が指定された場合は自動的に偶数に丸められます）

0 が指定されたり省略された場合には、以下のようにして自動で計算されます。

- `video_layout.$REGION_NAME.cell_width` が指定されている場合:
  - `セルの幅 * グリッドの列数` として計算されます（実際にはこれに枠線等の調整が加わります）
- `resolution` が指定されている場合:
  - `全体の解像度の幅 - リージョンの x 座標` として計算されます

なお `width` と `cell_width` を同時に指定した場合にはエラーになります。

リージョンの幅の計算方法の詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.height: $INTEGER`

指定されたリージョンの高さをピクセル単位で指定します。

指定可能な値の範囲は以下の通りです：

- 16 ピクセル以上、`y_pos + height` の値が合成後の映像の全体解像度の高さ以下
- 偶数値（奇数が指定された場合は自動的に偶数に丸められます）

0 が指定されたり省略された場合には、以下のようにして自動で計算されます。

- `video_layout.$REGION_NAME.cell_height` が指定されている場合:
  - `セルの高さ * グリッドの行数` として計算されます（実際にはこれに枠線等の調整が加わります）
- `resolution` が指定されている場合:
  - `全体の解像度の高さ - リージョンの y 座標` として計算されます

なお `height` と `cell_height` を同時に指定した場合にはエラーになります。

リージョンの高さの計算方法の詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.cell_width: $INTEGER`

指定されたリージョンのセルの幅をピクセル単位で指定します。

指定可能な値は以下の通りです：

- 偶数値（奇数が指定された場合は自動的に偶数に丸められます）

この項目を指定した場合、リージョンの幅は `セルの幅 * グリッドの列数 + 内側の枠線 + 外側の枠線` として自動計算されます。

**注意**: `width` と `cell_width` を同時に指定することはできません。両方が指定された場合はエラーとなります。

0 が指定されたり省略された場合には、リージョンの `video_layout.$REGION_NAME.width` や `resolution` の設定に基づいて自動で計算されます。

リージョンのサイズ計算の詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.cell_height: $INTEGER`

指定されたリージョンのセルの高さをピクセル単位で指定します。

指定可能な値の範囲は以下の通りです：

- 偶数値（奇数が指定された場合は自動的に偶数に丸められます）

この項目を指定した場合、リージョンの高さは `セルの高さ * グリッドの行数 + 内側の枠線 + 外側の枠線` として自動計算されます。

**注意**: `height` と `cell_height` を同時に指定することはできません。両方が指定された場合はエラーとなります。

0 が指定されたり省略された場合には、リージョンの `video_layout.$REGION_NAME.height` や `resolution` の設定に基づいて自動で計算されます。

リージョンのサイズ計算の詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.max_columns: $INTEGER`

指定されたリージョンのグリッドの最大列数を指定します。

指定可能な値の範囲は 0 以上の整数です。
0 を指定するか、項目が省略された場合には未指定扱いになります。
デフォルト値は未指定（制限なし）で、この場合は映像ソースの数に基づいて自動的に列数が決定されます。

**使用例:**

```json
{
  "video_layout": {
    "main": {
      "video_sources": ["*.json"],
      "max_columns": 3
    }
  }
}
```

上記の例では、映像ソースが何個あっても、メインリージョンのグリッドは最大3列までに制限されます。

同時に表示する必要がある映像ソースの数が `max_rows * max_columns` よりも多い場合には、
`video_layout.$REGION_NAME.reuse` の設定に従って、表示する映像ソースが決定されます。

グリッドの行列数の決定方法の詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.max_rows: $INTEGER`

指定されたリージョンのグリッドの最大行数を指定します。

指定可能な値の範囲は 0 以上の整数です。
0 を指定するか、項目が省略された場合には未指定扱いになります。
デフォルト値は未指定（制限なし）で、この場合は映像ソースの数に基づいて自動的に行数が決定されます。

**使用例:**

```json
{
  "video_layout": {
    "main": {
      "video_sources": ["*.json"],
      "max_rows": 2
    }
  }
}
```

上記の例では、映像ソースが何個あっても、メインリージョンのグリッドは最大 2 行までに制限されます。

同時に表示する必要がある映像ソースの数が `max_rows * max_columns` よりも多い場合には、
`video_layout.$REGION_NAME.reuse` の設定に従って、表示する映像ソースが決定されます。

グリッドの行列数の決定方法の詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.reuse: $REUSE_KIND`

指定されたリージョンでのセル再利用方法を指定します。

同時に表示する必要がある映像ソース数がリージョンのセル数（`max_rows * max_columns`）を超える場合に、どのソースを優先して表示するかを決定します。

`$REUSE_KIND` に指定可能な値は以下の通りです：

- `"none"`: セルを再利用しません。競合が発生した場合、開始時刻が遅い映像ソースは合成対象から完全に除外されます。
- `"show_oldest"` （デフォルト）: セルを再利用します。競合が発生した場合、開始時刻が早い映像ソースが優先されます。
- `"show_newest"`: セルを再利用します。競合が発生した場合、開始時刻が遅い映像ソースが優先されます。

**使用例:**

```json
{
  "video_layout": {
    "main": {
      "video_sources": ["*.json"],
      "max_rows": 2,
      "max_columns": 2,
      "reuse": "show_newest"
    }
  }
}
```

上記の例では、メインリージョンは最大4つのセル（2行×2列）を持ちますが、映像ソースが4つを超える場合、新しく開始された映像ソースが古い映像ソースより優先して表示されます。

詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.x_pos: $INTEGER`

指定されたリージョンを配置する X 座標をピクセル単位で指定します。

指定可能な値の範囲は以下の通りです：

- 0 以上、合成後の映像の全体解像度の幅未満の値
- 偶数値（奇数が指定された場合は自動的に偶数に丸められます）

デフォルト値は `0` で、この場合リージョンは左端に配置されます。

**使用例:**

```json
{
  "video_layout": {
    "main": {
      "video_sources": ["*.json"],
      "x_pos": 100,
      "width": 640
    },
    "sidebar": {
      "video_sources": ["sidebar.json"],
      "x_pos": 740,
      "width": 200
    }
  }
}
```

上記の例では `main` リージョンが X 座標 100 の位置に、`sidebar` リージョンが X 座標 740 の位置に配置されます。

リージョンの位置とサイズの詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.y_pos: $INTEGER`

指定されたリージョンを配置する Y 座標をピクセル単位で指定します。

指定可能な値の範囲は以下の通りです：

- 0 以上、合成後の映像の全体解像度の高さ未満の値
- 偶数値（奇数が指定された場合は自動的に偶数に丸められます）

デフォルト値は `0` で、この場合リージョンは上端に配置されます。

**使用例:**

```json
{
  "video_layout": {
    "main": {
      "video_sources": ["*.json"],
      "y_pos": 50,
      "height": 480
    },
    "bottom_bar": {
      "video_sources": ["bottom.json"],
      "y_pos": 530,
      "height": 100
    }
  }
}
```

上記の例では `main` リージョンが Y 座標 50 の位置に、`bottom_bar` リージョンが Y 座標 530 の位置に配置されます。

リージョンの位置とサイズの詳細については [layout_region.md](./layout_region.md) を参照してください。

### `video_layout.$REGION_NAME.z_pos: $INTEGER`

指定されたリージョンの Z 座標（重ね合わせ順序）を指定します。

Z 座標は、複数のリージョンが重なり合う場合の描画順序を決定します。値が小さいリージョンほど奥（背景側）に、値が大きいリージョンほど手前（前景側）に描画されます。

指定可能な値の範囲は -99 から 99 までの整数で、デフォルト値は 0 です。

なお、同じ Z 座標を持つリージョン同士の描画順序は未定義です。

**使用例:**

```json
{
  "video_layout": {
    "background": {
      "video_sources": [...],
      "z_pos": -10
    },
    "main": {
      "video_sources": [...],
      "z_pos": 0
    },
    "overlay": {
      "video_sources": [...],
      "z_pos": 10
    }
  }
}
```

上記の例では、`background` リージョンが最も奥に、`overlay` リージョンが最も手前に描画されます。

### `video_layout.$REGION_NAME.border_pixels: $INTEGER`

指定されたリージョンのセル間および外周の枠線の幅をピクセル単位で指定します。

指定可能な値の範囲は以下の通りです：

- 0 ピクセル以上
- 偶数値（奇数が指定された場合はエラーになります）

デフォルト値は `2` ピクセルです。

この設定は以下の枠線に影響します：

- **内側の枠線**: セル間に挿入される枠線
- **外側の枠線**: リージョンの外周に挿入される枠線（ただし、リージョンが全体解像度と同じサイズの場合は挿入されません）

**使用例:**

```json
{
  "video_layout": {
    "main": {
      "video_sources": ["archive-*.json"],
      "border_pixels": 4,
      "max_columns": 2,
      "max_rows": 2
    }
  }
}
```

上記の例では、セル間および外周に 4 ピクセルの枠線が挿入されます。

なお、外周については、リージョンやセルのサイズ関連の関係で
枠線のサイズが指定値からわずかに変わることがあります。

### `frame_rate: $FRAME_RATE`

出力映像のフレームレートを指定します。

`$FRAME_RATE` は以下の形式で指定可能です：

- 整数値（例：`25`, `30`, `60`）
- 分数表記（例：`"30/1"`, `"60000/1001"`）

整数値で指定した場合は、その値を分子として分母は 1 となります。
分数表記で指定する場合は、文字列として `"分子/分母"` の形式で記述します。

デフォルト値は `25` です。

### `bitrate: $BITRATE_KBPS`

**非推奨**: この項目はレガシー版の Hisui との互換性維持のために残されています。新しい Hisui では `video_bitrate` の使用を推奨します。

合成後の映像のエンコードビットレートを kbps 単位で指定します。

デフォルト値は `映像ソースの数 * 200` です。

`video_bitrate` フィールドと `bitrate` フィールドの両方が指定された場合、`video_bitrate` が優先されます。

### `libvpx_vp8_encode_params: $PARAMS`

libvpx で VP8 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [layout_encode_params.md](./layout_encode_params.md) を参照してください。

### `libvpx_vp9_encode_params: $PARAMS`

libvpx で VP9 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [layout_encode_params.md](./layout_encode_params.md) を参照してください。

### `openh264_encode_params: $PARAMS`

OpenH264 で H.264 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [layout_encode_params.md](./layout_encode_params.md) を参照してください。

### `svt_av1_encode_params: $PARAMS`

SVT-AV1 で AV1 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [layout_encode_params.md](./layout_encode_params.md) を参照してください。

### `video_toolbox_h264_encode_params: $PARAMS`

Apple Video Toolbox で H.264 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [layout_encode_params.md](./layout_encode_params.md) を参照してください。

### `video_toolbox_h265_encode_params: $PARAMS`

Apple Video Toolbox で H.265 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [layout_encode_params.md](./layout_encode_params.md) を参照してください。

### `nvcodec_h264_encode_params: $PARAMS`

NVIDIA Video Codec SDK で H.264 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [layout_encode_params.md](./layout_encode_params.md) を参照してください。

### `nvcodec_h265_encode_params: $PARAMS`

NVIDIA Video Codec SDK で H.265 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [layout_encode_params.md](./layout_encode_params.md) を参照してください。

### `nvcodec_av1_encode_params: $PARAMS`

NVIDIA Video Codec SDK で AV1 エンコードを行う際のエンコードパラメーターを指定します。
詳細は [layout_encode_params.md](./layout_encode_params.md) を参照してください。

### `nvcodec_h264_decode_params: $PARAMS`

NVIDIA Video Codec SDK で H.264 デコードを行う際のデコードパラメーターを指定します。
詳細は [layout_decode_params.md](./layout_decode_params.md) を参照してください。

### `nvcodec_h265_decode_params: $PARAMS`

NVIDIA Video Codec SDK で H.265 デコードを行う際のデコードパラメーターを指定します。
詳細は [layout_decode_params.md](./layout_decode_params.md) を参照してください。

### `nvcodec_vp8_decode_params: $PARAMS`

NVIDIA Video Codec SDK で VP8 デコードを行う際のデコードパラメーターを指定します。
詳細は [layout_decode_params.md](./layout_decode_params.md) を参照してください。

### `nvcodec_vp9_decode_params: $PARAMS`

NVIDIA Video Codec SDK で VP9 デコードを行う際のデコードパラメーターを指定します。
詳細は [layout_decode_params.md](./layout_decode_params.md) を参照してください。

### `nvcodec_av1_decode_params: $PARAMS`

NVIDIA Video Codec SDK で AV1 デコードを行う際のデコードパラメーターを指定します。
詳細は [layout_decode_params.md](./layout_decode_params.md) を参照してください。

### `trim: $BOOLEAN`

配信者が存在しない期間の自動トリミング（除去）を有効にするかどうかを指定します。

`true` を指定すると、音声ないし映像ソースが全く存在しない時間帯は、合成結果に含まれなくなります。

デフォルト値は `true` です。
