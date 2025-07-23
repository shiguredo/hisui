# レイアウトでのエンコードコーデックやパラメーターの指定

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

## 映像エンコーダー固有のパラメーターの指定

Hisui は映像のエンコーダーとして、以下をサポートしています:
- **libvpx**: VP8 / VP9 用のエンコーダー
- **OpenH264**: H.264 用のエンコーダー
- **SVT-AV1**: AV1 用のエンコーダー
- **Apple Video Toolbox**: macOS で利用可能な H.264 / H.265 用のエンコーダー

映像エンコーダーの種類とエンコードコーデックの組み合わせによって、指定可能なパラメーターセット変わってきます。

例えば `libvpx` エンコーダーで `VP8` コーデックでエンコードする場合のパラメーターセットは、
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

- `libvpx_vp8_encode_params`: libvpx で VP8 エンコードを行う際のパラメーター
- `libvpx_vp9_encode_params`: libvpx で VP9 エンコードを行う際のパラメーター
- `openh264_encode_params`: OpenH264 で H.264 エンコードを行う際のパラメーター
- `svt_av1_encode_params`: SVT-AV1 で AV1 エンコードを行う際のパラメーター
- `video_toolbox_h264_encode_params`: Apple Video Toolbox で H.264 エンコードを行う際のパラメーター
- `video_toolbox_h265_encode_params`: Apple Video Toolbox で H.265 エンコードを行う際のパラメーター


TODO: 各パラメーターセットの詳細を書く
