# Mp4Demuxer::open のファイル / moov 二度読みを解消する

- Priority: Low
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

`Mp4Demuxer::open` がファイル種別判定とデマルチプレクサ初期化でファイルを二度開き、moov を二度パースしている。重複を解消して無駄な I/O・パースをなくし、責務の分散による読みにくさを改善する。

## 優先度根拠

Low。性能影響は実用上小さく、主に重複の解消と可読性の向上。

## 現状

- `src/mp4/demuxer.rs` の `Mp4Demuxer::open` は `detect_mp4_file_kind(path)` でファイルを開いて ftyp + moov を読み種別判定し（`src/mp4/file_kind.rs`）、その後あらためて `File::open` し直して `initialize()` で moov を先頭からパースし直す。
- `Mp4FileKindDetector` は moov 全体を読む実装のため、I/O とパースが二重になる。

## 設計方針

- 種別判定と初期化でファイルオープン・先頭読み込みを共有する。例: 判定時に開いた `File` と読み込んだ先頭バッファを引き回す、もしくは判定結果だけ受け取り `open` 内で 1 回のオープンに統一する。
- 依存ライブラリ `shiguredo_mp4` の API 制約（detector と demuxer が別）を踏まえ、現実的な範囲で重複を減らす。

## 完了条件

- 通常 MP4 / fMP4 の前方読みが、ファイルオープンと moov 読み込みを重複なく行うこと。
- 既存の inspect 出力・テストが不変であること。
