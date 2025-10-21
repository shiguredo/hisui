# 変更履歴

- UPDATE
  - 後方互換がある変更
- ADD
  - 後方互換がある追加
- CHANGE
  - 後方互換のない変更
- FIX
  - バグ修正

## develop

- [UPDATE] shiguredo_nvcodec のバージョンを 2025.2.1 に更新する
  - 2025.2.0 には crates.io 経由でのビルドに失敗する問題があった
  - @sile

## 2025.2.0

**リリース日**: 2025-10-20

- [CHANGE] デコーダーおよびエンコーダーの名前を JSON に載せる際のキー名を "..._engine" 形式に統一する
  - compose コマンドの出力 JSON の `output_audio_encoder_name` を `output_audio_encode_engine` に変更する
  - compose コマンドの出力 JSON の `output_video_encoder_name` を `output_video_encode_engine` に変更する
  - vmaf コマンドの出力 JSON の `encoder_name` を `encode_engine` に変更する
  - @sile
- [UPDATE] shiguredo_libyuv のバージョンを 2025.2.0 に更新する
  - nv12 と i420 の相互変換関数が追加された
  - @sile
- [ADD] libvpx feature を追加する
  - デフォルトで有効
  - 無効にした場合には libvpx を用いた VP8 / VP9 のエンコードおよびデコードが行えなくなる
  - 主に CI 環境で、 libvpx が不要なテストのビルド時間短縮用に使用する目的
  - 内部利用前提であるため、ユーザーが無効化する必要はない（そのため公開ドキュメントにもこの feature は記載していない）
  - @sile
- [ADD] shiguredo_nvcodec を依存に追加する
  - @sile
- [ADD] NVIDIA Video Codec 対応を追加する
  - NVIDIA Video Codec SDK 13 を利用したデコードおよびエンコードに対応
    - 対応エンコードコーデックは H.264 / H.265 / AV1
    - 対応デコードコーデックは H.264 / H.265 / VP8 / VP9 / AV1
  - 動作には Linux の NVIDIA GPU 環境が必要 (CUDA ドライバー必須)
  - デフォルトでは無効で feature フラグで `nvcodec` を指定してビルドすると有効になる
    - ただし Ubuntu 24.04 (x86_64) 向けのビルド済みバイナリでは nvcodec 対応が有効になっている
  - 合わせてレイアウトファイルに以下の項目を追加（詳細はドキュメントを参照）
    - nvcodec_h264_encode_params
    - nvcodec_h265_encode_params
    - nvcodec_av1_encode_params
    - nvcodec_h264_decode_params
    - nvcodec_h265_decode_params
    - nvcodec_vp8_decode_params
    - nvcodec_vp9_decode_params
    - nvcodec_av1_decode_params
  - @sile
- [ADD] レイアウトファイルに video_encode_engines と video_decode_engines を追加する
  - 合成に使用するビデオエンコーダーとデコーダーを明示的に指定できるようにする
  - video_encode_engines: 映像エンコード時に使用するエンコーダーの候補を配列で指定（先頭のものほど優先される）
  - video_decode_engines: 映像デコード時に使用するデコーダーの候補を配列で指定（先頭のものほど優先される）
  - 指定可能な値（特定の features が有効な場合にのみ指定可能なものも含む）:
    - エンコーダー: "libvpx", "nvcodec", "openh264", "svt_av1", "video_toolbox"
    - デコーダー: "libvpx", "nvcodec", "openh264", "dav1d", "video_toolbox"
  - 未指定の場合は、その環境で利用可能なエンコーダーおよびデコーダーが全て候補となる（今まで通りの挙動）
  - @sile
- [ADD] macos-26 向けのリリースを追加する
  - @voluntas
- [ADD] macos-14 向けのリリースを追加する
  - @voluntas
- [CHANGE] legacy サブコマンドを削除する
  - Hisui 2025.1.x で提供されていた `hisui legacy` サブコマンドを削除
  - 代わりに `hisui compose` サブコマンドを使用すること
  - 詳細は [マイグレーションガイド](./docs/migrate_hisui_legacy.md) を参照
  - @sile
- [CHANGE] ビルド用 CUDA Toolkit のバージョンを 13.0.2 にする
  - @voluntas
- [FIX] Ubuntu 22.04 向けリリースビルドを追加する
  - x86_64 および arm64 アーキテクチャ向け
  - @voluntas
- [FIX] レイアウトファイルで `"audio_codec": "OPUS"` を指定するとエラーになるのを修正する
  - 値として "Opus" を期待する実装になっていたが、全て大文字が正しいので修正する
  - @sile

### misc

- [ADD] ci.yml にビルドバイナリを artifact としてアップロードするステップを追加する
  - @voluntas

## 2025.1.0

**祝リリース**
