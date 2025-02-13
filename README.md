# レガシー版 Hisui

新しい Hisui はレガシー版の Hisui と **互換性があります** 。
新しい Hisui のリリースまではレガシー版の Hisui をお使いください。

<https://github.com/shiguredo/hisui-legacy>

# 新しい Hisui を開発中です

- 2025 年の春に公開を予定しています
- 2025 年の夏に正式版リリースを予定しています
- ライセンスは [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0.html) として公開を予定しています

# Recording Composition Tool Hisui

[![GitHub tag (latest SemVer)](https://img.shields.io/github/tag/shiguredo/hisui.svg)](https://github.com/shiguredo/hisui)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read <https://github.com/shiguredo/oss/blob/master/README.en.md> before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に <https://github.com/shiguredo/oss> をお読みください。

## Recording Composition Tool Hisui について

Recording Composition Tool Hisui は WebRTC SFU Sora 向けの録画合成ツールです。

Sora が出力した録画ファイル (MP4 または WebM)を合成し MP4 で出力するツールです。

## 特徴

- Sora が生成する録画ファイルや録画レポートをそのまま利用できます
- 特に設定することなくすぐに使い始められます
- 複雑なレイアウトを JSON で指定することができます

### 新しい Hisui とレガシー版 Hisui の違い

- Rust で実装されています
- macOS の Video Toolbox を利用した H.264/H.265 のハードウェアアクセラレーターの映像デコード/エンコードに対応しています
- macOS の Audio Toolbox を利用した AAC の音声エンコードに対応しています
- MP4 メタデータに対応しています
- 分割録画に対応しています
- 入力形式が MP4 にも対応しています
- 出力形式が MP4 形式のみです
  - WebM での出力形式は非対応です
- AV1 のデコーダに rav1e を利用しています
- Intel VPL に非対応です
  - 将来的に対応予定です

## ファイル形式

- Sora が生成した WebM ファイルに対応しています
- 出力ファイル形式は MP4 に対応しています

## デコーダー/エンコーダー

- [Opus](https://github.com/xiph/opus) のソフトウェアによるデコード/エンコードに対応しています
- [Apple Audio Toolbox](https://developer.apple.com/documentation/audiotoolbox) を利用した AAC のエンコードに対応しています
- [libvpx](https://chromium.googlesource.com/webm/libvpx) を利用した VP8 / VP9 のソフトウェアによるデコード/エンコードに対応しています
- [SVT-AV1](https://gitlab.com/AOMediaCodec/SVT-AV1/) を利用した AV1 のソフトウェアによるエンコードに対応しています
- [dav1d](https://code.videolan.org/videolan/dav1d/) を利用した AV1 のソフトウェアによるデコードに対応しています
- [OpenH264](https://github.com/cisco/openh264) を利用した H.264 のデコード/エンコードに対応しています
- [Apple Video Toolbox](https://developer.apple.com/documentation/videotoolbox) を利用したハードウェアアクセラレーターによる H.264 / H.265 のデコード/エンコードに対応しています

### libfdk-aac-dev

> [!IMPORTANT]  
> Ubuntu を利用する場合、 libfdk-aac-dev を利用した AAC のエンコードに対応しています。
> ただし、自前でビルドする必要があります。

## 動作環境

- Ubuntu 24.04 x86_64
- Ubuntu 24.04 arm64
- Ubuntu 22.04 x86_64
- Ubuntu 22.04 arm64
- macOS 15.0 arm64
- macOS 14.0 arm64

### macOS の対応バージョン

直近の 2 バージョンをサポートします。

### Ubuntu の対応バージョン

直近の LTS 2 バージョンをサポートします。

## 対応 Sora

- WebRTC SFU Sora 2024.1 以降

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
