# `hisui list-codecs` コマンド {#command-list-codecs}

`hisui list-codecs` コマンドは、Hisui で利用可能なコーデックの一覧を表示するためのコマンドです。
このコマンドは、使用可能なエンコーダーやデコーダーの情報を JSON 形式で出力します。

## 使用方法 {#command-list-codecs-usage}

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

## 実行例 {#command-list-codecs-examples}

コマンドを実行すると、利用可能なコーデックの一覧が JSON 形式で出力されます。

```console
$ hisui list-codecs
{
  "codecs": [
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
      "decoders": ["openh264", "video_toolbox"],
      "encoders": ["openh264", "video_toolbox"]
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
  ],
  "engines": [
    {
      "name": "dav1d",
      "repository": "https://github.com/videolan/dav1d.git",
      "build_version": "1.5.1"
    },
    {
      "name": "libvpx",
      "repository": "https://github.com/webmproject/libvpx.git",
      "build_version": "v1.15.2"
    },
    {
      "name": "openh264",
      "repository": "https://github.com/cisco/openh264.git",
      "shared_library_path": "/usr/local/lib/libopenh264.dylib",
      "build_version": "v2.6.0",
      "runtime_version": "v2.6.0"
    },
    {
      "name": "opus",
      "repository": "https://github.com/xiph/opus.git",
      "build_version": "v1.5.2"
    },
    {
      "name": "svt_av1",
      "repository": "https://gitlab.com/AOMediaCodec/SVT-AV1.git",
      "build_version": "v3.1.0"
    }
  ]
}
```

`codecs` には、その環境の Hisui が利用可能なコーデック一覧と、
それぞれのコーデックのデコードおよびエンコードに使用されるエンジン名が表示されます。
あるコーデックのデコード・エンコードに対応するエンジンが複数ある場合には、リストの先頭要素のものが使用されます。

`engines` には、各エンジンの詳細情報が載っています。
