# `hisui list-codecs` コマンド

`hisui list-codecs` コマンドは、Hisui で利用可能なコーデックの一覧を表示するためのコマンドです。
このコマンドは、使用可能なエンコーダーやデコーダーの情報を JSON 形式で出力します。

## 使用方法

```console
$ hisui list-codecs -h
Recording Composition Tool Hisui

Usage: hisui ... list-codecs [OPTIONS]

Options:
  -h, --help            このヘルプメッセージを表示します ('--help' なら詳細、'-h' なら簡易版を表示)
      --version         バージョン番号を表示します
      --verbose         警告未満のログメッセージも出力します
      --openh264 <PATH> OpenH264 の共有ライブラリのパス [env: HISUI_OPENH264_PATH]
```

## 実行例

コマンドを実行すると、利用可能なコーデックの一覧が JSON 形式で出力されます。

```console
$ hisui list-codecs
[
  {
    "name": "OPUS",
    "type": "audio",
    "decoders": ["opus"],
    "encoders": ["opus"]
  },
  {
    "name": "AAC",
    "type": "audio",
    "decoders": [],
    "encoders": ["audio_toolbox"]
  },
  {
    "name": "VP8",
    "type": "video",
    "decoders": ["libvpx"],
    "encoders": ["libvpx"]
  },
  {
    "name": "VP9",
    "type": "video",
    "decoders": ["libvpx"],
    "encoders": ["libvpx"]
  },
  {
    "name": "H264",
    "type": "video",
    "decoders": ["video_toolbox"],
    "encoders": ["video_toolbox"]
  },
  {
    "name": "H265",
    "type": "video",
    "decoders": ["video_toolbox"],
    "encoders": ["video_toolbox"]
  },
  {
    "name": "AV1",
    "type": "video",
    "decoders": ["dav1d"],
    "encoders": ["svt_av1"]
  }
]
```
