# 録画合成の入力に fMP4 ファイルを対応する

- Priority: Low
- Created: 2026-06-03
- Completed:
- Model: Opus 4.8
- Branch: feature/add-recording-fmp4-support
- Polished: 2026-06-03

## 目的

issue 0001 で inspect コマンドの fMP4 読み込みに対応したが、Sora 録画合成パイプライン (`src/sora/recording_reader.rs` / `src/sora/recording_mp4_reader.rs`) は依然として通常 MP4 のみに対応している。外部ツールが生成した fMP4 を録画合成の入力として扱えるようにする。

## 優先度根拠

- 録画合成の入力は通常 Sora が生成した MP4 / WebM であり、Sora 自身は fMP4 を生成しない。外部ツールが生成した fMP4 を録画合成の入力にするユースケースが現状あるかは不明確。
- 一方、`Mp4VideoReader` / `Mp4AudioReader` は前方読み (Iterator) のみで seek を使わないため、issue 0001 で導入した `Mp4Demuxer` enum を流用すれば実装コストは小さい。
- 需要が確認できてから着手すべきであり、Low が妥当。

## 現状

- `src/sora/recording_mp4_reader.rs` の `Mp4VideoReader` / `Mp4AudioReader` は `Mp4FileDemuxer` を直接生成し、`Iterator` (`next_sample()` のみ、前方読み) でサンプルを返す。seek / prev_sample は使わない。
- `src/sora/recording_reader.rs` は `ContainerFormat` で Mp4 / Webm を分岐し、実体 reader を enum で保持する。
- issue 0001 で `src/mp4/demuxer.rs` に `Mp4Demuxer` enum (通常 MP4 / fMP4 を前方読みで統一し、`InputRequired` を内部解決する) を追加済み。

## 設計方針

- `Mp4VideoReader` / `Mp4AudioReader` の内部 demuxer を `Mp4FileDemuxer` から `Mp4Demuxer` enum に差し替える。データ読み出し・format 判定は issue 0001 で `pub(crate)` 化した共通ヘルパー (`read_sample_data_at` / `audio_format_from_entry` / `video_format_from_entry` / `calculate_timestamps`) を流用する。
- `recording_reader.rs` は `ContainerFormat::Mp4` ブランチのまま無変更で済む見込み (拡張子は .mp4 のまま、実体判定は `Mp4Demuxer::open` 内の `detect_mp4_file_kind` が行う)。
- composition_time_offset (B フレーム) は issue 0001 と同様に非対応とする。

## 完了条件

- 録画合成の入力に fMP4 ファイルを指定でき、通常 MP4 と整合的に合成できること。
- cargo test がすべて成功すること。

## 解決方法

- `Mp4VideoReader` / `Mp4AudioReader` の `demuxer` フィールドを `Mp4Demuxer` に変更し、`next_sample` 周辺を `Mp4Demuxer::next_sample` (`SampleContext` ベース) に置き換える。
- 録画合成の E2E テストに fMP4 入力ケースを追加する。
