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

## 2025.2.2

- [UPDATE] エラーメッセージを改善する
  - CUDA および NVENC のエラーコードに対応する詳細情報を表示するようにする
  - @sile

## 2025.2.1

**リリース日**: 2025-10-21

- [FIX] ビルドに必要なヘッダファイルを含んだ third_party/ ディレクトリを crate 内に移動する
  - 今までは hisui リポジトリのルートに配置していたが、これだと shiguredo_nvcodec の crates.io への publish 時に third_party/ がパッケージに含まれない
  - そのため cargo 経由でビルドする際に必要なファイルが見つからずに失敗してしまっていた
  - third_party/ ディレクトリを hisui/crates/shiguredo_nvcodec/ 以下に移動することで、crates.io に登録したパッケージにもこのディレクトリが含まれるようにした
  - @sile
