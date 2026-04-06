# Media Engine Hisui

[![GitHub tag (latest SemVer)](https://img.shields.io/github/tag/shiguredo/hisui.svg)](https://github.com/shiguredo/hisui)
[![hisui](https://img.shields.io/crates/v/hisui.svg)](https://crates.io/crates/hisui)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read <https://github.com/shiguredo/oss/blob/master/README.en.md> before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に <https://github.com/shiguredo/oss> をお読みください。

## Media Engine Hisui について

Media Engine Hisui は Rust で実装されたメディアエンジンです。

- OBS WebSocket 互換 API を提供しています
- 複数プロトコルの入出力をリアルタイムに合成・変換します
- ブラウザからリモートでリアルタイムなレイアウト指定ができます
- WebRTC SFU Sora が出力した録画ファイルの合成ツールとしても利用できます

## 特徴

### OBS WebSocket API への対応

Hisui は OBS Studio 互換の WebSocket プロトコルを実装しており、
シーン・入力・出力・録画・配信を RPC で操作できます。

- 50 種類以上の RPC リクエストを実装
  - 一般: `GetVersion` / `GetStats`
  - シーン: `GetSceneList` / `GetCurrentProgramScene` / `CreateScene` / `RemoveScene` ほか
  - シーンアイテム: `GetSceneItemList` / `CreateSceneItem` / `SetSceneItemTransform` ほか
  - 入力: `GetInputList` / `CreateInput` / `RemoveInput` / `SetInputSettings` / `SetInputMute` / `SetInputVolume` ほか
  - 出力・録画・配信: `StartStream` / `StopStream` / `StartOutput` / `StopOutput` / `StartRecord` / `StopRecord` ほか
- Identify ハンドシェイク、RPC バージョンネゴシエーション、SHA256 チャレンジによるパスワード認証に対応
- WebRTC DataChannel (`obsdc`) 経由の制御にも対応
- `hisui obsws` サブコマンドで起動できます

> [!IMPORTANT]
> OBS WebSocket API 対応は現時点では実験的機能です。`--experimental` フラグの指定が必要です。

### 複数プロトコルの入出力

Hisui は複数のメディアプロトコルの入出力に対応しています。

入力:

- WebRTC SFU Sora
  - role: recvonly
- RTMP
- SRT
- RTSP 1.0
- 画像ファイル / 単色 (color source)
- オーディデバイス
- ビデオデバイス
- MP4 ファイル
- WebM ファイル

出力:

- WebRTC SFU Sora
  - role: sendonly
- RTMP
- HLS (ABR 対応)
  - S3 API 互換オブジェクトストレージへ直接アップロードできます (AWS Signature Version 4 対応)
- DASH (ABR 対応)
  - S3 API 互換オブジェクトストレージへ直接アップロードできます (AWS Signature Version 4 対応)
- MP4

HLS / DASH のセグメントや MP4 / WebM ファイルは Amazon S3 へ直接アップロードできます (AWS Signature Version 4 対応)。

### リアルタイムな合成と変換

Hisui は映像・音声のリアルタイムミキサーを搭載しています。

- 複数の入力ソースをレイアウト指定でリアルタイムに合成
- 解像度・フレームレート・コーデックの変換
- レイアウトは JSON / JSONC で記述可能
- レイアウトは OBS WebSocket API over DataChannel (`obsdc`) を経由してブラウザからリアルタイムに指定・変更可能
- 複雑なレイアウトを [JSON で指定](./docs/layout_encode_params.md)できます
- 用途に合わせた[エンコードパラメーターの指定](./docs/layout_encode_params.md)や[自動調整](./docs/command_tune.md)ができます

## サブコマンド

- `hisui inspect`: 録画ファイルの情報を取得します
- `hisui list-codecs`: 利用可能なコーデック一覧を表示します
- `hisui compose`: 複数の MP4 / WebM ファイルをレイアウト指定で合成し MP4 として出力します
- `hisui vmaf`: VMAF スコアを計測します
- `hisui tune`: [Optuna](https://optuna.org/) を利用してエンコーダーパラメーターを自動調整します
- `hisui obsws`: OBS WebSocket サーバーを起動します (実験的機能)

## Sora 録画合成ツールとして利用する

Hisui は WebRTC SFU Sora が出力した録画ファイルの合成ツールとしても利用できます。

- Sora が生成する録画ファイルや録画レポートをそのまま利用できます
- Sora が出力した MP4 / WebM ファイルを合成し MP4 で出力します
- 複雑なレイアウトを JSON で指定できます
- エンコードパラメーターの指定や自動調整ができます

## インストール

Hisui は [uv](https://docs.astral.sh/uv/) を利用して PyPI 経由でインストールできます。

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

- MP4
- WebM

## デコーダー / エンコーダー

- [Opus](https://github.com/xiph/opus) のソフトウェアによるデコード / エンコード
- [Apple Audio Toolbox](https://developer.apple.com/documentation/audiotoolbox) を利用した AAC のデコード / エンコード
- [libvpx](https://chromium.googlesource.com/webm/libvpx) を利用した VP8 / VP9 のソフトウェアによるデコード / エンコード
- [SVT-AV1](https://gitlab.com/AOMediaCodec/SVT-AV1/) を利用した AV1 のソフトウェアによるエンコード
- [dav1d](https://code.videolan.org/videolan/dav1d/) を利用した AV1 のソフトウェアによるデコード
- [OpenH264](https://github.com/cisco/openh264) を利用した H.264 のデコード / エンコード
- [Apple Video Toolbox](https://developer.apple.com/documentation/videotoolbox) を利用したハードウェアアクセラレーターによる H.264 / H.265 のデコード / エンコード
- [NVIDIA Video Codec](https://developer.nvidia.com/nvidia-video-codec-sdk) を利用したハードウェアアクセラレーターによる AV1 / H.264 / H.265 のエンコードと、VP8 / VP9 / AV1 / H.264 / H.265 のデコード

### NVIDIA Video Codec

NVIDIA Video Codec を利用する場合は NVIDIA ドライバー 570.0 以降が必要です。

### OpenH264

> [!IMPORTANT]
> OpenH264 を利用した H.264 のデコード / エンコードを行う場合は、
> OpenH264 の共有ライブラリを別途用意し、`--openh264 <PATH>` オプションまたは
> `HISUI_OPENH264_PATH` 環境変数で共有ライブラリのパスを指定する必要があります。

### FDK-AAC

> [!IMPORTANT]
> Linux 版の Hisui は `fdk-aac` フィーチャーがデフォルトで有効になっています。
> FDK-AAC を利用した AAC のデコード / エンコードを行う場合は、
> FDK-AAC の共有ライブラリを別途用意し、`--fdk-aac <PATH>` オプションまたは
> `HISUI_FDK_AAC_PATH` 環境変数で共有ライブラリのパスを指定してください。

## HTTP エンドポイント

- `/bootstrap`: WebRTC ブートストラップ用のエンドポイントです
- `/metrics`: Prometheus テキスト形式のメトリクスを返します。`?format=json` で JSON 形式も取得できます

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

## 今後の対応予定

- NDI 入出力

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
- macOS の Audio Toolbox を利用した AAC の音声デコード/エンコードに対応しています
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
