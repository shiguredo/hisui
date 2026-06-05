# fMP4 reader の命名 doc を補い、エラー文言の fMP4 表記を統一する

- Priority: Low
- Created: 2026-06-04
- Completed: 2026-06-05
- Model: Opus 4.8
- Branch: feature/refactor-fmp4-reader-naming
- Polished: 2026-06-05

## 目的

`Mp4FileReader` だけ doc コメントが無く、`Mp4SampleReader` との役割差が型名から読み取りにくい。また `Mp4FileReader::new` のエラー文言だけが `Fmp4` 表記で、コードベース全体の `fMP4` 表記から外れている。この 2 点を解消し、可読性と表記の一貫性を改善する。

## 優先度根拠

Low。機能には影響しない可読性・一貫性の改善。変更対象は doc コメントとエラー文言の表記のみで、挙動は変えない。

## 現状

### エラー文言の表記揺れ

- 散文表記として `Fmp4` を使っているのは `src/mp4/reader.rs:261` の `Mp4FileReader::new` のエラー `"Fmp4 is not supported by Mp4FileReader yet: {}"` 1 箇所のみ。
- このエラーは OBSWS の MP4 ソース経路（`src/obsws/source/file_mp4.rs:32` の `Mp4FileReader::new(...)?`）でユーザーに表出する。
- 連動して、テスト `new_rejects_fragmented_mp4`（`src/mp4/reader.rs:1687`）が `message.contains("Fmp4")`（`:1696`）とアサーションメッセージ（`:1697`「エラーメッセージに Fmp4 が含まれること」）で `Fmp4` を検証している。
- 一方、他のエラー文字列リテラル（`src/hls/writer.rs:372,797,803` / `src/dash/writer.rs:340,658,664`）・コメント・`CHANGES.md` は一貫して `fMP4` 表記。識別子（型名・enum バリアント `DemuxerKind::Fmp4`・ライブラリ型 `Fmp4FileDemuxer` / `Fmp4SegmentMuxer`・`HlsSegmentFormat::Fmp4` 等）は Rust の命名規則上 `Fmp4` が正規であり、表記統一の対象外。

### doc コメントの欠落

- doc コメントが無いのは `src/mp4/reader.rs:212` の `Mp4FileReader` struct のみ（直前は `#[derive(Debug)]`）。
- 役割差を示す説明自体は `src/mp4/reader.rs:256-257` のコメント（「OBSWS のメディア再生 (seek / prev_sample) に依存」「inspect は fMP4 を `Mp4SampleReader` 経由で扱う」）に既にある。
- 近接して紛らわしい他の自前型は doc 整備済みで追加対応は不要:
  - `Mp4Demuxer`（`src/mp4/demuxer.rs:96-99`）・`DemuxerKind`（`:64-67`）・`Mp4SampleReader`（`src/mp4/sample_reader.rs:1-5,26`）はいずれも doc / module doc で役割明示済み。
- なお名前空間上の衝突（コンパイルエラー）は起きていない。型名が似ていて誤読しやすいだけ。

## 設計方針

- 改名はしない。`Mp4FileReader` / `Mp4SampleReader` は外部 API（`src/lib.rs:17` の `pub mod mp4` と `src/mp4.rs:4-5` の `pub mod reader` / `pub mod sample_reader` で crate 外へ到達する）であり、改名は後方互換を壊す（`feature/change-` 相当）。Priority Low に見合わないため doc 追記で対応する。
- `Mp4FileReader`（`src/mp4/reader.rs:212`）に doc コメントを追加し、`Mp4SampleReader`（前方読み専用・inspect 用）との役割差を明示する。文面は `src/mp4/reader.rs:256-257` のコメントを要約して使う。
- エラー文言を `fMP4` に統一する（エラー文字列リテラルの既存主流表記）。変更対象は次の 3 箇所:
  - `src/mp4/reader.rs:261` のエラー文字列 `"Fmp4 ..."` を `"fMP4 ..."` にする。
  - `src/mp4/reader.rs:1696` の `message.contains("Fmp4")` を `contains("fMP4")` にする。
  - `src/mp4/reader.rs:1697` のアサーションメッセージ `"...に Fmp4 が含まれること"` を `"...に fMP4 が含まれること"` にする。
- 識別子（型名・enum バリアント・ライブラリ型）は変更しない。

## 完了条件

- `Mp4FileReader` に `Mp4SampleReader` との役割差を示す doc コメントが付くこと。
- `Mp4FileReader::new` のエラー文言が `fMP4` 表記になり、テスト `new_rejects_fragmented_mp4`（`src/mp4/reader.rs:1696-1697` の `contains` とアサーションメッセージ）が追従すること。`cargo test -p hisui --lib new_rejects_fragmented_mp4` で確認できる（`testdata/red-320x320-h264-aac-fragmented.mp4` に依存）。
- 識別子の `Fmp4`（型名・enum バリアント・ライブラリ型）に手を入れていないこと。

## 関連

- issues/0021（`src/mp4/demuxer.rs` の `Mp4Demuxer::open` を変更する）。本 issue は `src/mp4/reader.rs` のみ変更し、`demuxer.rs` は doc の手本として参照するだけなので、ファイル変更は重複しない。改名もしないため型名参照への波及も無い。

## 解決方法 (2026-06-05)

`feature/refactor-fmp4-reader-naming` で次を実装した。

- `Mp4FileReader`（`src/mp4/reader.rs`）に doc コメントを追加し、再生制御つき reader であること・seek 依存で fMP4 非対応・前方読み専用は `Mp4SampleReader` を使うことを明示した。
- `Mp4FileReader::new` のエラー文言を `"Fmp4 is not supported ..."` から `"fMP4 is not supported ..."` に統一した。
- 連動するテスト `new_rejects_fragmented_mp4` の `contains` とアサーションメッセージを `fMP4` に追従させた。`cargo test -p hisui --lib new_rejects_fragmented_mp4` で通過を確認した。
- 識別子（型名・enum バリアント・ライブラリ型）は変更していない。
- `CHANGES.md` への記載は、doc 追加とエラー文言の表記統一のみで機能・互換性に影響しないため、方針判断により行わなかった。
