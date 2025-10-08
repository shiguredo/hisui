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

- [CHANGE] legacy サブコマンドを削除する
  - Hisui 2025.1.x で提供されていた `hisui legacy` サブコマンドを削除
  - 代わりに `hisui compose` サブコマンドを使用すること
  - 詳細は [マイグレーションガイド](./docs/migrate_hisui_legacy.md) を参照
  - @sile
- [UPDATE] shiguredo_libyuv のバージョンを 2025.2.0 に更新する
  - nv12 と i420 の相互変換関数が追加された
  - @sile
- [ADD] shiguredo_nvcodec を依存に追加する
  - @sile
- [ADD] NVIDIA Video Codec 対応を追加する
  - NVIDIA Video Codec SDK 13 を利用したデコードおよびエンコードに対応
    - 対応コーデックは H.264 / H.265 / AV1
  - 動作には Linux の NVIDIA GPU 環境が必要 (CUDA ドライバー必須)
  - デフォルトでは無効で feature フラグで `nvcodec` を指定してビルドすると有効になる
  - 合わせてレイアウトファイルに以下の項目を追加（詳細はドキュメントを参照）
    - nvcodec_h264_encode_params
    - nvcodec_h265_encode_params
    - nvcodec_av1_encode_params
    - nvcodec_h264_decode_params
    - nvcodec_h265_decode_params
    - nvcodec_av1_decode_params
  - @sile
- [ADD] macos-26 向けのリリースを追加する
  - @voluntas

## 2025.1.0

**祝リリース**
