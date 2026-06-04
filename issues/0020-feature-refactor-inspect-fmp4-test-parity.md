# inspect の fMP4 e2e テストを対応する通常 MP4 と実際に突き合わせる

- Priority: Low
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

fMP4 inspect の e2e テストが「通常 MP4 と一致」と謳いながら、実際には通常 MP4 を読んで比較していない。実際に突き合わせる形にして、回帰検出力を上げる。

## 優先度根拠

Low。テストの実効性向上であり、製品機能には影響しない。

## 現状

- `tests/e2e.rs` の `inspect_fragmented_mp4_video_only` / `inspect_fragmented_mp4_audio_only` / `inspect_fragmented_mp4_audio_video` は、fMP4 のサンプル数をハードコード値（映像 25 / 音声 45）と突き合わせるだけ。
- アサーションメッセージは「映像 / 音声サンプル数が通常 MP4 と一致すること」だが、対応する通常 MP4（`testdata/red-320x320-h264-aac.mp4` / `testdata/beep-aac-audio.mp4`）を実際に inspect して比較していない。
- 値自体は実測で一致するが、「一致」をテスト自身が担保していない。testdata 再生成時に通常 MP4 側が変わっても検出できない。
- 補足: フル出力では timestamp が testdata 生成差で一致しない（reader のバグではない）。

## 設計方針

- 通常 MP4 と fMP4 の両方を inspect し、サンプル数・`data_size`・`keyframe` 列など比較可能な項目を実際に突き合わせるヘルパーを用意する。
- timestamp は testdata 由来で差が出るため、比較対象から外すか許容差を設ける。
- 大掛かりにしないなら、最低限コメント / メッセージを実態に合わせ「期待値（25 / 45）と一致」に修正する。

## 完了条件

- fMP4 と対応する通常 MP4 の inspect 結果が、比較可能な項目で実際に突き合わされること。もしくは、比較していない項目について誤解を招くメッセージを排除すること。
