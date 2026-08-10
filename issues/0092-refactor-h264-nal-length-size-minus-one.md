# H.264 サンプルデータの NAL 区切りバイトサイズを lengthSizeMinusOne に基づいて汎用化する

- Created: 2026-08-04
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-h264-nal-length-size-minus-one
- Polished: {YYYY-MM-DD}

## 目的

inspect コマンドの H.264 NAL 情報取得が「区切りバイトサイズ = 4 固定」を仮定しており、`lengthSizeMinusOne` が 3 以外の MP4 ファイルで正しくパースできない。AVCC の仕様に基づいて可変長の NAL 長フィールドに対応する。

## 現状

- `src/subcommand_inspect.rs` の `VideoCodecSpecificInfo::H264` 生成 (サンプルデータの NAL パース) が、NALU の区切りバイトサイズを 4 バイト固定と仮定している。この仮定は Sora 録画由来の名残であり、コメントも Sora 前提の表記だった (本 issue 起票時に一般化済み)
- 元々 `src/video/h264.rs` に存在した avcC の `length_size_minus_one` 検証 (`parse_avcc_sps_pps_lists`) は、録画機能 (WebM リーダー) の削除に伴い削除された
- hisui の MP4 出力は `NALU_HEADER_LENGTH = 4` 固定 (`src/video/h264.rs` の `AvccBox` 構築) のため、自分の出力を読む分には問題ないが、inspect が読み込む外部 MP4 では `lengthSizeMinusOne` が 0〜2 (1〜3 バイト長) のファイルも存在し得る

## 設計方針

- AVCC ではサンプルデータの NAL 長フィールドは avcC ボックスの `lengthSizeMinusOne` (0〜3 = 1〜4 バイト) で決まる
- サンプルデータの NAL パース時に、対応トラックの avcC から取得した `lengthSizeMinusOne` を使って長フィールドを可変長で読み取る
- 具体的な変更対象は polish で確定する。候補:
  - `src/subcommand_inspect.rs` の H.264 NAL パースに `lengthSizeMinusOne` を渡す
  - サンプルエントリー (`SharedSampleEntry` / `AvccBox`) から `length_size_minus_one` を取り出す経路の整備

## 完了条件

- `lengthSizeMinusOne` が 3 以外の MP4 を入力に与えても、inspect が H.264 NAL 情報 (type / nri) を正しく出力できること
- `lengthSizeMinusOne` が 3 の既存 MP4 の読み込みに回帰がないこと
