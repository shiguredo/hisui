# Media Pipeline Tool Hisui

[![GitHub tag (latest SemVer)](https://img.shields.io/github/tag/shiguredo/hisui.svg)](https://github.com/shiguredo/hisui)
[![hisui](https://img.shields.io/crates/v/hisui.svg)](https://crates.io/crates/hisui)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read <https://github.com/shiguredo/oss/blob/master/README.en.md> before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に <https://github.com/shiguredo/oss> をお読みください。

## Media Pipeline Tool Hisui について

Media Pipeline Tool Hisui は複数のプロトコルやファイルに対応したメディアパイプラインツールです。

RTMP と SRTP からの音声や映像を一つの映像に合成して WebRTC SFU Sora と YouTube に同時配信したり、
複数の MP4 ファイルを合成して一つの MP4 ファイルに出力したりすることができます。

## 特徴

- 様々なメディアプロトコルに対応
  - RTMP または RTMPS の入出力に対応
  - RTSP の入出力に対応
    - RTSP over HTTPS
  - SRT 入出力に対応
  - WebRTC 入出力に対応
  - RTP 入出力に対応
  - NDI 入出力に対応
  - HLS 出力に対応
- 様々な映像・音声コーデックに対応
  - 音声コーデック: Opus / AAC
  - 映像コーデック: VP9 / VP8 / AV1 / H.265 / H.264
- パイプライン処理
  - JSON-RPC 2.0 ベースのプロトコルで柔軟にパイプラインを構築
  - Python などで JSON-RPC 2.0 サーバーを実装するだけで機能拡張が可能
  - Hisui SDK を使う事で簡単に Python で JSON-RPC 2.0 サーバーを実装できます

### Sora 録画ファイルの合成に最適化

- Sora が生成する録画ファイルや録画レポートをそのまま利用できます
- 特に設定することなくすぐに使い始められます
- 複雑なレイアウトを JSON で指定することができます
- 用途に合わせた[エンコードパラメーターの指定](./docs/layout_encode_params.md)や[自動調整](./docs/command_tune.md)ができます

## インストール

Hisui は [uv](https://docs.astral.sh/uv/) を利用して PyPI 経由でインストールすることができます。

```bash
uv tool install hisui
```

## Python ライブラリとして利用する

Hisui は Python でコマンドラインのラッパーライブラリを提供しています。

```bash
uv add hisui
```

```python
from hisui import Hisui

with Hisui() as hisui:
    # List available codecs
    codecs = hisui.list_codecs()
    print(codecs)
```

## ファイル形式

- MP4 または WebM ファイルに対応しています
- 出力ファイル形式は MP4 または Fragmented MP4 に対応しています

## デコーダー/エンコーダー

- [Opus](https://github.com/xiph/opus) のソフトウェアによるデコード/エンコードに対応しています
- [Apple Audio Toolbox](https://developer.apple.com/documentation/audiotoolbox) を利用した AAC のエンコードに対応しています
- [libvpx](https://chromium.googlesource.com/webm/libvpx) を利用した VP8 / VP9 のソフトウェアによるデコード/エンコードに対応しています
- [SVT-AV1](https://gitlab.com/AOMediaCodec/SVT-AV1/) を利用した AV1 のソフトウェアによるエンコードに対応しています
- [dav1d](https://code.videolan.org/videolan/dav1d/) を利用した AV1 のソフトウェアによるデコードに対応しています
- [OpenH264](https://github.com/cisco/openh264) を利用した H.264 のデコード/エンコードに対応しています
- [Apple Video Toolbox](https://developer.apple.com/documentation/videotoolbox) を利用したハードウェアアクセラレーターによる H.264 / H.265 のデコード/エンコードに対応しています
- [NVIDIA Video Codec](https://developer.nvidia.com/nvidia-video-codec-sdk) を利用したハードウェアアクセラレーターによる AV1 / H.264 / H.265 のエンコードと、VP8 / VP9 / AV1 / H.264 / H.265 のデコードに対応しています

### NVIDIA Video Codec

NVIDIA Video Codec を利用する場合は NVIDIA ドライバー 570.0 以降が必要です。

### FDK-AAC

> [!IMPORTANT]
> Ubuntu を利用する場合、 FDK-AAC を利用した AAC のエンコードに対応しています。
> ただし、 `libfdk-aac-dev` パッケージをシステムにインストールした上で、 `--features fdk-aac` を指定して Hisui を自前でビルドする必要があります。

##

## 動作環境

- Ubuntu 24.04 x86_64
- Ubuntu 24.04 arm64
- Ubuntu 22.04 x86_64
- Ubuntu 22.04 arm64
- macOS 26 arm64
- macOS 15 arm64
- macOS 14 arm64

### macOS の対応バージョン

直近の 2 バージョンをサポートします。

### Ubuntu の対応バージョン

直近の LTS 2 バージョンをサポートします。

## 対応 Sora

- WebRTC SFU Sora 2025.1 以降

## ドキュメント

Hisui の利用方法は [usage.md](docs/usage.md) をご確認ください。

## ビルド

Hisui のビルド方法は [build.md](docs/build.md) をご確認ください。

## サポートについて

## 優先実装

優先実装とは Sora のライセンスを契約頂いているお客様向けに Sora Python SDK の実装予定機能を有償にて前倒しで実装することです。

**詳細は Discord やメールなどでお気軽にお問い合わせください**

### 優先実装が可能な機能一覧

- Intel VPL 対応
- AMD AMF 対応
- NETINT Quadra 対応

### Discord

- **サポートしません**
- アドバイスします
- フィードバック歓迎します

最新の状況などは Discord で共有しています。質問や相談も Discord でのみ受け付けています。

<https://discord.gg/shiguredo>

### バグ報告

Discord の `#sora-tool-faq` チャンネルへお願いします。

## ライセンス

Apache License 2.0

```text
Copyright 2025-2025, Takeru Ohta (Original Author)
Copyright 2025-2025, Shiguredo Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

## OpenH264

<https://www.openh264.org/BINARY_LICENSE.txt>

```text
"OpenH264 Video Codec provided by Cisco Systems, Inc."
```

## NVIDIA Video Codec SDK

<https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/index.html>

```text
“This software contains source code provided by NVIDIA Corporation.”
```

## H.264 (AVC) と H.265 (HEVC) のライセンスについて

**時雨堂が提供する Hisui のビルド済みバイナリには H.264 と H.265 のコーデックは含まれていません**

### H.264

H.264 対応は [Via LA Licensing](https://www.via-la.com/) (旧 MPEG-LA) に連絡を取り、ロイヤリティの対象にならないことを確認しています。

> 時雨堂がエンドユーザーの PC /デバイスに既に存在する AVC / H.264 エンコーダー/デコーダーに依存する製品を提供する場合は、
> ソフトウェア製品は AVC ライセンスの対象外となり、ロイヤリティの対象にもなりません。

### H.265

H.265 対応は以下の二つの団体に連絡を取り、H.265 ハードウェアアクセラレーターのみを利用し、
H.265 が利用可能なバイナリを配布する事は、ライセンスが不要であることを確認しています。

また、H.265 のハードウェアアクセラレーターのみを利用した H.265 対応の SDK を OSS で公開し、
ビルド済みバイナリを配布する事は、ライセンスが不要であることも確認しています。

- [Access Advance](https://accessadvance.com/ja/)
- [Via Licensing Alliance](https://www.via-la.com/)

## Hisui レガシー機能

> [!IMPORTANT]
> Hisui レガシー機能は 2025.1.x でのみ利用できます。

新しい Hisui のレガシー機能は [レガシー版の Hisui](<https://github.com/shiguredo/hisui-legacy>) とほぼ互換性があります。
レガシー版の Hisui は新しい Hisui が正式リリースしたタイミングで非推奨となります。
Hisui レガシー機能は Hisui 2025.1.x でのみ利用できます。

### 新しい Hisui とレガシー版 Hisui の違い

- Rust で実装されています
- macOS の Audio Toolbox を利用した AAC の音声エンコードに対応しています
- macOS の Video Toolbox を利用した H.264/H.265 のハードウェアアクセラレーターの映像デコード/エンコードに対応しています
- MP4 と WebM の入力形式に対応しています
- 分割録画機能が出力するファイル形式に対応しています
- 出力形式が MP4 形式のみです
  - WebM での出力形式は非対応です
- AV1 のデコーダに [dav1d](https://code.videolan.org/videolan/dav1d/) を利用しています
- Intel VPL に非対応です
- NVIDIA Video Codec に対応しています
- [Optuna](https://optuna.org/) を利用したエンコーダーパラメータの自動調整機能を利用できます

詳細は [migrate_hisui_legacy\.md](docs/migrate_hisui_legacy.md) をご覧ください。
