# fMP4 reader 周りの紛らわしい命名とエラー文言の表記を統一する

- Priority: Low
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

自前型とライブラリ型で近接した紛らわしい命名があり、またエラー文言の fMP4 表記が不統一。可読性と一貫性を改善する。

## 優先度根拠

Low。可読性・一貫性の改善で機能には影響しない。

## 現状

- 命名の近接衝突（`src/mp4/demuxer.rs`, `src/mp4/sample_reader.rs`）:
  - 自前 `Mp4Demuxer` vs ライブラリ `Mp4FileDemuxer`
  - 自前 `DemuxerKind` vs ライブラリ `Mp4FileKind`
  - `Mp4SampleReader`（前方読み専用）vs `Mp4FileReader`（再生制御つき）
  - いずれも役割差が名前から読み取りにくい。
- エラー文言の表記揺れ: `src/mp4/reader.rs` の `Mp4FileReader::new` のエラー `"Fmp4 is not supported by Mp4FileReader yet"` の `Fmp4` が、コード / コメント / CHANGES の `fMP4` 表記と不統一。テスト `new_rejects_fragmented_mp4` が `contains("Fmp4")` で検証しているため、連動修正が必要。

## 設計方針

- 型名の役割が分かるよう改名するか、最低限 doc コメントで `Mp4FileReader` との違い等を明示する。
- エラー文言を `fragmented MP4` または `fMP4` に統一し、対応するテストのアサーションも合わせる。

## 完了条件

- 命名から役割が読み取れる、もしくは doc で明示されること。
- fMP4 のエラー文言表記がコードベースで一貫し、テストも追従すること。
