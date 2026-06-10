# inspect の fMP4 e2e テストを対応する通常 MP4 と実際に突き合わせる

- Priority: Low
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

fMP4 inspect の e2e テストが「通常 MP4 と一致」と謳いながら、実際には通常 MP4 を読んで比較していない。対応する通常 MP4 も inspect して実際に突き合わせる形にし、testdata 再生成時に通常 MP4 側だけが変わった場合の回帰を検出できるようにする。

## 優先度根拠

Low。テストの実効性向上であり、製品機能には影響しない。

## 現状

- `tests/e2e.rs` の `inspect_fragmented_mp4_video_only` / `inspect_fragmented_mp4_audio_only` / `inspect_fragmented_mp4_audio_video` は、fMP4 を inspect した結果のサンプル数をハードコード値（映像 25 / 音声 45）と突き合わせるだけ。
- アサーションメッセージは「映像 / 音声サンプル数が通常 MP4 と一致すること」だが、対応する通常 MP4 を実際に inspect して比較していない。値自体は実測で一致するが、「一致」をテスト自身が担保していないため、testdata 再生成時に通常 MP4 側が変わっても検出できない。
- 各テストと対応する通常 MP4 は次のとおり（いずれも `testdata/` 配下）。

  | テスト | fMP4 | 対応する通常 MP4 |
  | --- | --- | --- |
  | `inspect_fragmented_mp4_video_only` | `archive-red-320x320-h264-fragmented.mp4` | `archive-red-320x320-h264.mp4` |
  | `inspect_fragmented_mp4_audio_only` | `beep-aac-audio-fragmented.mp4` | `beep-aac-audio.mp4` |
  | `inspect_fragmented_mp4_audio_video` | `red-320x320-h264-aac-fragmented.mp4` | `red-320x320-h264-aac.mp4` |

- 実測（両者を `hisui inspect` して diff）では、3 ペアとも `data_size` / `keyframe` / `nalus` 列は完全一致する。一方 `timestamp_us` / `duration_us` は「音声+映像」ペアの映像トラックでのみずれる（先頭フレームの `duration_us` が通常 40000us・fMP4 63281us で、以降の全 `timestamp_us` がずれる。集計値 `video_duration_us` も 960000 対 983281）。「映像のみ」「音声のみ」のペアでは `timestamp_us` も完全一致する。このずれは testdata 生成差によるもので reader のバグではない。

## 設計方針

- `tests/e2e.rs` 内に、inspect 出力 JSON 文字列から比較対象フィールドを抽出する共通ヘルパーを追加し、3 テストから利用する。既存テストと同じく `nojson::RawJson` を使い、`to_member` / `required` / `to_array`、トラックの有無は `optional` で扱う（モック・スタブは使わない）。
- 比較する項目: サンプル数、`data_size`（音声・映像）、`keyframe`（映像）。映像が H264 / H265 の場合は `nalus` も一致するため含めてよい。
- 比較から除外する項目: `timestamp_us` / `duration_us`、および集計値 `video_duration_us` / `audio_duration_us`。これらは「音声+映像」ペアの映像トラックでずれるため、全テスト共通で除外する（許容差は設けない）。
- 各テストは自身が持つトラックに対応するフィールドのみ比較する（`video_only` は映像のみ、`audio_only` は音声のみ、`audio_video` は両方）。トラックが無い場合は該当キーが出力されないため `optional` で扱う。
- 既存の絶対値検証（映像 25 / 音声 45）は、通常 MP4 と fMP4 が同時に同数で変化した場合の回帰を検出するアンカーとして残す。突き合わせ後はアサーションメッセージを実態に合わせる（「通常 MP4 と一致」「期待値どおり」）。

## 完了条件

- 3 テストすべてで、fMP4 と対応する通常 MP4 の inspect 結果が、比較可能な項目（サンプル数・`data_size`・`keyframe`、映像が H264 / H265 なら `nalus`）で実際に突き合わされること。
- `timestamp_us` / `duration_us` および集計値の duration はずれるため、比較対象から除外すること。
- 既存の絶対値検証（映像 25 / 音声 45）が残ること。
- 本変更は `tests/e2e.rs` のみに閉じ、プロダクションコード（`src/`）と inspect 出力仕様を変更しないこと。
- `cargo test --test e2e inspect_fragmented` が通ること。

## 関連

- issues/closed/0023（inspect が通常 MP4 と fMP4 を区別して `format` に報告する。本テストの初出）
- issues/closed/0024（inspect の fMP4 映像トラック読み出しハングを修正し、本テストの `#[ignore]` を解除）
