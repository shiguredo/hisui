# inspect にペイロード出力を追加する

- Priority: Low
- Created: 2026-09-11
- Completed:
- Branch: feature/add-inspect-payload
- Polished:

## 目的

調査時にサンプルの実バイト列 (ペイロード) まで確認したいことがある。inspect にペイロード出力機能を追加する。ただし全フレームを出力すると膨大になるため、何らかの絞り込みが必要。

## pending とした理由

- 出力形式 (hex / base64 など) と絞り込み方法 (サンプル範囲・先頭 N バイト・NALU 単位など) の設計が必要。
- 既存の JSON 出力との整合をどう取るかを決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/subcommand_inspect.rs` の `OutputPrinter` は `path` / `format` / `audio_codec` / `video_codec` / duration / sample count / `audio_samples` / `video_samples` を出力する。
- 各サンプルは `timestamp_us` / `duration_us` / `data_size` / `keyframe` / `nalus` (H.264 の NAL type と nri) / `decoded_data_size` / `width` / `height` を持ち、ペイロードのバイト列は含まない。
- CLI オプションは `--decode` / `--openh264` / `--fdk-aac` のみ。

## 設計方針 (要検討・未確定)

- ペイロード出力オプションを追加し、hex などで出す。
- 出力量を抑える絞り込みオプション (対象サンプル・バイト数上限) を用意する。
- `--decode` 時のデコード結果との関係を整理する。

## 完了条件

- 任意のサンプルのペイロードを確認できること。
- 大量出力を避けられる絞り込みができること。

(解決方法は pending のため、設計確定後に記載する。)
