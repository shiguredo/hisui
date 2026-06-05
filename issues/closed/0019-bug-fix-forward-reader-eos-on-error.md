# 前方読み reader がエラー時に EOS を送らずパイプラインがハングする

- Priority: Medium
- Created: 2026-06-04
- Completed: 2026-06-05
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

## 解決方法 (2026-06-05)

本 issue が対象とする「破損 / truncated な MP4・fMP4 を inspect に渡すとハングする」問題は、後発の issue 0024 (`inspect が映像トラックを含む fMP4 を demux できず、エラー後にハングするのを直す`、Completed: 2026-06-05) で根本対処済みであることが判明した。0024 は本 issue の「設計方針」が挙げた根本対処をそのまま実装しており、本 issue の完了条件はすべて 0024 で満たされている。そのため新規の対応は行わず close する。

### 0024 で解決済みの内容

- TrackPublisher の drop 時に subscriber チャネルを閉じる根本対処が実装済み。`TrackPublisher` に `eos_sent` を持たせ (`src/media_pipeline.rs:1199`)、`send()` 内で EOS 送出を一元記録する (`src/media_pipeline.rs:1255-1259`)。EOS 未送信のまま drop し、かつ再 publish 待ちでない場合は `drain_returned_subscribers` が subscriber を閉じる (`src/media_pipeline.rs:134-153`)。購読側の `MessageReceiver::recv` は切断を `Message::Eos` に変換する (`src/media_pipeline.rs:1305-1312`) ため、`OutputPrinter` はハングしない。これは本 issue が「現状」で「TrackPublisher は EOS 未送出のまま drop されても subscriber 側チャネルを閉じない」と記した点を解消している。
- `run_internal` のエラー握り潰しも解消済み。`MediaPipeline::run()` が異常終了を `bool` で返し (`src/media_pipeline.rs:108`)、inspect はこれを参照して非ゼロ終了する (`src/subcommand_inspect.rs:106-114`)。
- demuxer のベストエフォート読み取りにより、破損フラグメント・EOF 切り詰めは `Ok(None)` (正常終端) として扱われる (`src/mp4/demuxer.rs:163-188`)。この場合 `Mp4SampleReader::run` はループを正常に抜け、末尾の `send_eos` (`src/mp4/sample_reader.rs:147-152`) を通常通り通る。本 issue が懸念したエラーパス自体がほぼ発生しなくなった。
- 回帰テストも追加済み。`publisher_failure_without_eos_closes_subscribers` (`src/media_pipeline.rs`) が、publisher が EOS 未送信で異常終了した際に購読側が有限時間で `Message::Eos` を受信し、`run()` が異常終了を報告することを検証する。demuxer 側にも切り詰め許容と moov 破損エラーのテスト群がある (`src/mp4/demuxer.rs`)。

### 見送った対応

本 issue の「軽量防御」(`Mp4SampleReader::run` のエラー終了前に reader 自身が `send_eos` を送る) は未実装のまま残るが、上記の TrackPublisher drop による根本対処がある今は冗長な二重防御であり、観測可能な不具合を生まない。むしろ EOS 経路が二重になり `eos_sent` 一元管理の設計意図と緊張するため、実装を見送る。

### 関連

- issues/0024 (本 issue の問題を根本対処した issue。closed)
