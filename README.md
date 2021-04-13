# Recording Composition Tool Hisui

[![GitHub tag (latest SemVer)](https://img.shields.io/github/tag/shiguredo/hisui.svg)](https://github.com/shiguredo/hisui)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read https://github.com/shiguredo/oss before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に https://github.com/shiguredo/oss をお読みください。

## Recording Composition Tool Hisui について

Recording Composition Tool Hisui は WebRTC SFU Sora 向けの録画合成ツールです。

Sora が出力した録画ファイルを合成し WebM や MP4 で出力するツールです。

## 特徴

## ファイル形式

- Sora が生成した WebM ファイルに対応しています
- 出力ファイル形式は WebM と MP4 に対応しています
- 生成された VP9/AAC の MP4 ファイルは Safari 最新版で再生が可能です

## デコーダー/エンコーダー

- VP8 / VP9 / H.264 デコードに対応しています
    - H.264 をデコードする場合は OpenH264 を用意する必要があります
- Opus デコードに対応しています
- VP8 / VP9 エンコードに対応しています
- Opus / AAC エンコードに対応しています
    - AAC を利用する場合は自前でのビルドが必要です

## 動作環境

- Ubuntu 20.04 x86_64

## 対応 Sora

- WebRTC SFU Sora 2020.3 以降

## 使ってみる

Hisui を使ってみたい人は [USE.md](doc/USE.md) をお読みください。

## ビルドする

Linux 版 Hisui のビルドしたい人は [BUILD_LINUX.md](doc/BUILD_LINUX.md) をお読みください

## FAQ

Hisui についての FAQ は [FAQ.md](doc/FAQ.md) をお読みください。

## 優先実装

優先実装とは Sora のライセンスを契約頂いているお客様限定で Hisui の実装予定機能を有償にて前倒しで実装することです。

- AV1 出力対応
- AV1 入力対応
- JSON によるレイアウト指定対応
- アイコン埋め込み対応
    - 音声のみの場合は指定したアイコンを埋め込めるようにする
- タイトルの埋め込み対応
    - 会議のタイトルなどを埋め込めるようにする
- 時間の埋め込み対応
    - タイムスタンプを埋め込めるようにする
- 配信情報の埋め込み対応
    - ConnectionID や Metadata 情報を指定して埋め込めるようにする

## 対応予定

- Sora が AV1 録画に対応したら AV1 デコード機能へ対応予定です
- Safari が AV1 再生に対応したら AV1 エンコード機能へ対応予定です

## 廃止予定

- Safari が Opus 再生に対応したら AAC 対応を削除予定です


## ライセンス

Apache License 2.0

```
Copyright 2020-2021, HARUYAMA Seigo (Original Author)
Copyright 2020-2021, Shiguredo Inc.

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

