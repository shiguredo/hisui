# Mp4Demuxer::open のファイル / moov 二度読みを解消する

- Priority: Low
- Created: 2026-06-04
- Completed: 2026-06-09
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

## 結論: 実装せず close

検討の結果、本 issue は実装せず close する。

### 理由

本 issue の主目的は「重複を解消して無駄な I/O・パースをなくす」ことだが、このうち
moov のパース重複 (`MoovBox::decode` の二度実行) は `shiguredo_mp4` の API 制約により
hisui 単独では解消できない。`Mp4FileDemuxer` / `Fmp4FileDemuxer` はどちらも moov を
内部でパースする実装で、外部からパース済み moov を渡す口も、初期化の副産物として
種別 (mvex の有無) を得る口も公開されていない。先に種別を判定してから適切な
デマルチプレクサを選ぶという構造自体がライブラリ都合で強制されている。

hisui 側で解消できるのはファイルオープンと moov の I/O 重複だけだが、moov は通常
数 KB から数 MB 程度と小さく、ファイル全体を読んでデコード・エンコード・合成する
hisui の処理全体から見れば誤差であり、効果は限定的で優先度 Low の想定の範囲に
とどまる。一方で I/O を共有するには `File` と読み込みバッファを判定と初期化で
引き回す新機構が必要になり、現状の責務分離 (`file_kind.rs` が判定し、`demuxer.rs`
が初期化する。`detect_mp4_file_kind` は `reader.rs` / `subcommand_inspect.rs` からも
独立して使われている) を崩して、可読性をむしろ下げる方向にも働く。

主目的が達成できず、残る I/O 削減も実利が小さく複雑化のリスクを伴うため、費用対
効果で見送る。根本的に重複を解消するなら、`shiguredo_mp4` 側にデマルチプレクサ
初期化の副産物として `file_kind` を返す API、または moov を一度だけ読んで
MP4 / fMP4 を両対応する統合デマルチプレクサを追加することが前提となる。

## 完了条件

- 通常 MP4 / fMP4 の前方読みが、ファイルオープンと moov 読み込みを重複なく行うこと。
- 既存の inspect 出力・テストが不変であること。
