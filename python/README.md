# Hisui の Python ラッパーライブラリ

これは、Recording Composition Tool Hisui の Python ラッパーライブラリです。
バインディングではなく Python から Hisui のコマンドラインインターフェースを呼び出す形で実装されています。

## ビルド


```bash
uv tool install maturin
maturin build --release
```

## ツールインストール

```bash
uv tool install hisui
```

これで `hisui` コマンドが利用可能になります。

## ライブラリインストール

ツールとして利用する場合

```bash
uv add hisui
```

これで `hisui` ライブラリがインストールされます。

## ライブラリの使い方

```bash
uv run python3
```

```python
>>> from hisui import Hisui
>>> with Hisui() as h:
...     h.list_codecs()
...
{'codecs': [{'name': 'OPUS', 'type': 'audio', 'decoders': ['opus'], 'encoders': ['opus']}, {'name': 'AAC', 'type': 'audio', 'decoders': [], 'encoders': ['audio_toolbox']}, {'name': 'VP8', 'type': 'video', 'decoders': ['libvpx'], 'encoders': ['libvpx']}, {'name': 'VP9', 'type': 'video', 'decoders': ['libvpx'], 'encoders': ['libvpx']}, {'name': 'H264', 'type': 'video', 'decoders': ['video_toolbox'], 'encoders': ['video_toolbox']}, {'name': 'H265', 'type': 'video', 'decoders': ['video_toolbox'], 'encoders': ['video_toolbox']}, {'name': 'AV1', 'type': 'video', 'decoders': ['dav1d'], 'encoders': ['svt_av1']}], 'engines': [{'name': 'audio_toolbox'}, {'name': 'dav1d', 'repository': 'https://github.com/videolan/dav1d.git', 'build_version': '1.5.1'}, {'name': 'libvpx', 'repository': 'https://github.com/webmproject/libvpx.git', 'build_version': 'v1.15.2'}, {'name': 'opus', 'repository': 'https://github.com/xiph/opus.git', 'build_version': 'v1.5.2'}, {'name': 'svt_av1', 'repository': 'https://gitlab.com/AOMediaCodec/SVT-AV1.git', 'build_version': 'v3.1.2'}, {'name': 'video_toolbox'}]}
```
