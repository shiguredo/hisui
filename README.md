# Recording Composition Tool Hisui

[![GitHub tag (latest SemVer)](https://img.shields.io/github/tag/shiguredo/hisui.svg)](https://github.com/shiguredo/hisui)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read https://github.com/shiguredo/oss/blob/master/README.en.md before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に https://github.com/shiguredo/oss をお読みください。

## Recording Composition Tool Hisui について

Recording Composition Tool Hisui は WebRTC SFU Sora 向けの録画合成ツールです。

Sora が出力した録画ファイルを合成し WebM や MP4 で出力するツールです。

## 特徴

- Sora が生成する録画ファイルや録画レポートをそのまま利用できます
- 特に設定することなくすぐに使い始められます
- 細かいチューニングも可能です
- 複雑なレイアウトを JSON で指定することができます

## ファイル形式

- Sora が生成した WebM ファイルに対応しています
- 出力ファイル形式は WebM と MP4 に対応しています
- 生成された VP9/AAC の MP4 ファイルは Safari 最新版で再生が可能です

## デコーダー/エンコーダー

- Opus 音声コーデックデコードに対応しています
- VP8 / VP9 / AV1 / H.264 の映像コーデックデコードに対応しています
  - H.264 をデコードする場合は OpenH264 を用意する必要があります
- Opus / AAC の音声エンコードに対応しています
- VP8 / VP9 / AV1/ H.264 の映像エンコードに対応しています
  - AAC を利用する場合は自前でのビルドが必要です

## 動作環境

- Ubuntu 20.04 x86_64
- Ubuntu 20.04 arm64
- Ubuntu 22.04 x86_64
- Ubuntu 22.04 arm64

## 対応 Sora

- WebRTC SFU Sora 2023.1 以降

## 使ってみる

Hisui を使ってみたい人は [USE.md](doc/USE.md) をお読みください。

レイアウト機能については [LAYOUT.md](doc/LAYOUT.md) をお読みください。

## ビルドする

Linux 版 Hisui をビルドしたい人は [BUILD_LINUX.md](doc/BUILD_LINUX.md) をお読みください。

## FAQ

Hisui についての FAQ は [FAQ.md](doc/FAQ.md) をお読みください。

## 既知の問題について

既知の問題については [既知の問題](doc/KNOWN_ISSUES.md) をお読みください。

## 優先実装

優先実装とは Sora のライセンスを契約頂いているお客様限定で Hisui の実装予定機能を有償にて前倒しで実装することです。

- レイアウト指定機能
  - [ダイキン工業株式会社](https://www.daikin.co.jp/) 様

### 優先実装が可能な機能一覧

**詳細は Discord やメールなどでお気軽にお問い合わせください**

- 分割録画対応
- Lyra V2 MP4 対応
- アイコン埋め込み対応
  - 音声のみの場合は指定したアイコンを埋め込めるようにする
- タイトルの埋め込み対応
  - 会議のタイトルなどを埋め込めるようにする
- 時間の埋め込み対応
  - タイムスタンプを埋め込めるようにする
- 配信情報の埋め込み対応
  - ConnectionID や Metadata 情報を指定して埋め込めるようにする
- [whisper](https://github.com/openai/whisper) を利用した文字起こしと字幕機能
  - 合成時に文字起こしも同時に行います
  - 合成時に字幕を追加します
  - https://github.com/ggerganov/whisper.cpp を利用します
- ハードウェアアクセラレーター対応
  - NVIDIA / AMD への対応
- EME 対応
  - https://www.w3.org/TR/encrypted-media/

## 廃止予定

- Mobile Safari が Opus 再生に対応したら AAC 対応を削除予定です

## ヘルプ

```console
$ hisui -h
hisui
Usage: /usr/local/bin/hisui [OPTIONS]

Options:
-h,--help                   Print this help message and exit
-f,--in-metadata-file       Metadata filename (REQUIRED)
--version                   Print version and exit
--out-container             Output container type (WebM/MP4). default: WebM
--out-video-codec           Video codec (VP8/VP9/H264/AV1). default: VP9
--out-audio-codec           Audio codec (Opus/AAC). default: Opus (hisui supports AAC only in MP4)
--out-video-frame-rate      Video frame rate (INTEGER/RATIONAL). default: 25
--out-file                  Output filename
--max-columns               Max columns (POSITIVE INTEGER). default: 3
--libvpx-cq-level           libvpx Constrained Quality level (NON NEGATIVE INTEGER). default: 30
--libvpx-min-q              libvpx minimum (best) quantizer (NON NEGATIVE INTEGER). default: 10
--libvpx-max-q              libvpx maximum (worst) quantizer (NON NEGATIVE INTEGER). default: 50
--out-opus-bit-rate         Opus bit rate (kbps, POSITIVE INTEGER). default: 65536
--out-aac-bit-rate          AAC bit rate (kbps, POSITIVE INTEGER). default: 64000
--mp4-muxer                 MP4 muxer (Faststart/Simple). default: Faststart
--dir-for-faststart         Directory for intermediate files of faststart muxer. default: metadata directory
--openh264                  OpenH264 dynamic library path
--verbose                   Verbose mode
--audio-only                Audio only mode
--video-codec-engines       Show video codec engines and exit.
--h264-encoder              H264 encoder (OneVPL/OpenH264). default: OneVPL
--show-progress-bar         Toggle to show progress bar. default: true
--layout                    Layout Metadata File


Experimental Options:
--screen-capture-report     Screen capture metadata filename
--screen-capture-connection-id
Screen capture connection id
--screen-capture-width      Width for screen-capture (NON NEGATIVE multiple of 4). default: 960
--screen-capture-height     Height for screen-capture (NON NEGATIVE multiple of 4). default: 640
--screen-capture-bit-rate   Bit rate for screen-capture (kbps). default: 1000
--mix-screen-capture-audio  Mix screen-capture audio. default: false
--success-report            Directory for success report
--failure-report            Directory for failure report
```

## ライセンス

Apache License 2.0

```
Copyright 2020-2023, HARUYAMA Seigo (Original Author)
Copyright 2020-2023, Shiguredo Inc.

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

https://www.openh264.org/BINARY_LICENSE.txt

```
"OpenH264 Video Codec provided by Cisco Systems, Inc."
```
