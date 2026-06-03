# OBSWS メディア再生 (Mp4FileReader) が fMP4 ファイルに対応する

- Priority: Low
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: feature/add-mp4-reader-fmp4-support
- Polished: 2026-06-03

## pending の理由

OBSWS のメディア再生 (`Mp4FileReader`) は seek / prev_sample / warm-up に依存している。これらを fMP4 で実現するには、依存ライブラリ `shiguredo_mp4` の `Fmp4FileDemuxer` に `seek()` / `prev_sample()` が追加される必要があるが、2026.3.0 時点では提供されていない。ライブラリ側の対応を待つ必要があるため pending とする。それまでは issue 0001 で導入した fail-fast (`Mp4FileReader::new` が fMP4 を明示エラーで拒否する) を維持する。

## 目的

issue 0001 で inspect コマンドの fMP4 読み込みに対応したが、OBSWS のメディア入力 (`Mp4FileReader` 経由のメディア再生) は通常 MP4 のみに対応している。OBSWS のメディア入力で fMP4 ファイルを再生できるようにする。

## 優先度根拠

- OBSWS のメディア入力で fMP4 を扱う需要は限定的であり、現状は fail-fast で明示エラーを返すため不可解な挙動にはならない。
- 実現には依存ライブラリの拡張が前提となり、hisui 単独では対応できない。
- 以上から Low が妥当。

## 現状

- `src/mp4/reader.rs` の `Mp4FileReader` は seek / prev_sample / warm-up を用いた OBSWS メディア再生 (一時停止・シーク・ループ) に依存する。
- issue 0001 で `Mp4FileReader::new` に fMP4 fail-fast を追加済み (`detect_mp4_file_kind` が `FragmentedMp4` を返したら明示エラー)。
- `shiguredo_mp4` 2026.3.0 の `Fmp4FileDemuxer` は `seek()` / `prev_sample()` を提供していない。

## 設計方針

- `shiguredo_mp4` の `Fmp4FileDemuxer` に `seek()` / `prev_sample()` が追加された後、`Mp4FileReader` の `ReaderState` が持つ demuxer を fMP4 対応に拡張する。
- fMP4 でのシーク方式 (非キーフレームへのシーク時に直前キーフレームまで遡る warm-up が fMP4 でどう成立するか) を併せて整理する。

## 完了条件

- OBSWS のメディア入力で fMP4 ファイルを再生・シーク・一時停止できること。
- issue 0001 で追加した fail-fast を解除すること。
- cargo test がすべて成功すること。

## 解決方法

- 依存ライブラリ (`shiguredo_mp4` の `Fmp4FileDemuxer`) の seek 対応後に詳細を詰める。

## 補足

- 録画合成の入力 fMP4 対応 (issue 0001 の段階 2a) は単独 issue としては設けない。対象は `src/sora/recording_mp4_reader.rs` の `Mp4VideoReader` / `Mp4AudioReader`。Sora は通常 fMP4 を出力せず需要が不明確なため。前方読みのみで実装コストは小さいので、需要が出たら本 issue の対応に合わせてついでに行う。
