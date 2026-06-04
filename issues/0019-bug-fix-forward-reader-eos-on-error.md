# 前方読み reader がエラー時に EOS を送らずパイプラインがハングする

- Priority: Medium
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

破損 / truncated な MP4・fMP4 を inspect に渡すと、reader がエラーで停止しても EOS を送らないため、出力側 (`OutputPrinter`) が終了せずプロセスがハングする。エラー時でも確実に終了するようにし、堅牢性を確保する。

## 優先度根拠

Medium。データ破損で詰まる類で正常ファイルでは発生しないが、ハングは利用者にとって分かりにくい挙動であり実害がある。issue 0001 で inspect が fMP4 を受理するようになり、このコードパスを踏む入力の幅が広がった。

## 現状

- `src/mp4/sample_reader.rs` の `Mp4SampleReader::run` は正常終了時のみ末尾で `send_eos()` する。途中の `?` / 早期 `return Err`（`Mp4Demuxer::open`、`next_sample`、`composition_time_offset` 非対応、`audio_format_from_entry` / `video_format_from_entry`、`read_sample_data` など）はいずれも末尾の `send_eos` を通らずに `TrackSender`（内部の `TrackPublisher`）を drop する。
- `src/media_pipeline.rs` の `TrackPublisher` は EOS 未送出のまま drop されても subscriber 側チャネルを閉じない。そのため inspect の `OutputPrinter`（`src/subcommand_inspect.rs`）の受信ループが終端を検知できず、プロセスがハングする。
- 実測: truncated / 破損 MP4・fMP4 を inspect に与えるとタイムアウト（ハング）する。develop でも同様に再現する既存問題で、fMP4 対応とは独立した堅牢性課題。
- `src/subcommand_inspect.rs` の `run_internal` は `setup_pipeline` / `pipeline.run` のエラーを `tracing::error!` で出すだけで握り潰し、常に `Ok(())` を返す。そのためハング以外にも、部分的・空の結果を終了コード 0 で返しうる。

## 設計方針

- 軽量防御: `Mp4SampleReader::run`（および同じ前方読みパターンを持つ reader）がエラー終了する前に、生成済みの sender へ必ず `send_eos` を送る。結果を一度受けてからエラーでも EOS を送る構造にするか、スコープガード / Drop で EOS 送出を保証する。
- 根本対処: `TrackPublisher` の drop 時に subscriber チャネルを閉じて終端を通知することを検討する。これは全 reader / processor に効く。影響範囲が広いため慎重に行う。
- `run_internal` がエラーを握り潰す点も見直す（少なくともハングを招かないこと、必要に応じて異常終了を表すこと）。

## 完了条件

- 破損 / truncated な MP4・fMP4 を inspect に渡してもハングせず、明示エラーで終了すること。
- 正常ファイルの inspect 出力が不変であること。
- 破損入力でハングしないことを検証する回帰テストを追加すること。
