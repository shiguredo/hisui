# 音声 sample_entry 要求機構を追加し finalize 失敗（映像トラック空）を直す

- Priority: High
- Created: 2026-06-04
- Completed:
- Model: Opus 4.8
- Branch:
- Polished:

## 目的

短時間録画等で StopRecord 後の出力 MP4 の映像トラックが空になることがある問題（issues/0011）の真因を直す。録画した映像が再生できなくなるデータ破損であり、実害が大きい。

## 優先度根拠

High。録画映像が再生不能になるデータ破損で、CI の e2e でも再発が確認されている（発生率は約 3% 程度）。録画機能の信頼性に直結する。

## 現状

- finalize 時に muxer が「Missing sample entry for first sample of Audio track」で Err を返し、標準 MP4 への変換が失敗する。失敗時は出力が録画中の fMP4 形式のまま残り、録画全体が単一の未 flush フラグメントに収まる短時間録画では映像トラックが空になる。
- 真因: `OpusEncoder`（`src/encoder/opus.rs`）は sample_entry を最初の出力フレームにしか載せない（`self.sample_entry.take()`、`src/encoder/opus.rs:58`）。
- 映像には録画開始時のキーフレーム要求機構（`src/encoder.rs` の `request_upstream_video_keyframe`、`VideoEncoderRpcMessage::RequestKeyframe`）があり、映像エンコーダはキーフレームに sample_entry を常に補完する（`src/encoder.rs:729-736`）ため、録画 writer に確実に sample_entry が届く。
- 一方、音声には同等の「sample_entry 要求」機構が無い。そのため録画 writer が最初の entry 付き音声フレームを取りこぼすと（合流タイミング・起動レース等）、sample_entry が一度も届かず `last_audio_sample_entry` が None のまま finalize に至り失敗する。
- issues/0011 で入れた「writer 入口（`handle_*_message`）での sample_entry 取り込み」は、届いた entry を pause 等の drop で落とさないための hardening であり、「そもそも届かない」本症状は塞げていない（CI の e2e `obsws/test_output.py::test_obsws_srt_inbound_with_stream_id` で再発を確認済み）。

## 設計方針

- 音声側にも「sample_entry 要求」機構を追加する（映像のキーフレーム要求の音声版）。録画 writer（および他の音声出力先）の起動時に上流の音声エンコーダへ要求を送り、音声エンコーダが次の出力フレームに sample_entry を再付与する（`OpusEncoder` の `self.sample_entry` を再セットする等）。
- RPC の形は要検討。候補は次の 2 つ:
  - 映像のキーフレーム要求 RPC（`VideoEncoderRpcMessage::RequestKeyframe`）を音声にも流用する（要求の意味は「次フレームに sample_entry を載せ直す」）。
  - 音声用の新規 RPC（例: `AudioEncoderRpcMessage::RequestSampleEntry`）を追加する。
- mp4 writer 側でコーデック情報から sample_entry を合成する案は採らない（writer の責務外）。
- 実装の参考: 映像のキーフレーム要求送信（`src/encoder.rs` の `request_upstream_video_keyframe`、`src/encoder.rs:379` 付近）と、エンコーダ側 RPC 処理（`src/encoder.rs:665-699` の `RequestKeyframe` ハンドリング・`keyframe_request_pending`）。

## 完了条件

- 短時間 SRT 録画（`obsws/test_output.py::test_obsws_srt_inbound_with_stream_id` および `obsws/test_output.py::test_obsws_srt_inbound_start_record_and_inspect_output`）を CI で十分な回数繰り返しても finalize 失敗が再発しないこと。発生率が約 3% のため、issues/0008 で用いた 100 回相当（10 シャード × 10 回）の一時ワークフローで検証し、検証後にそのワークフローは削除する。
- 検証には観測メトリクス（`hisui_total_finalize_failure_count` / `hisui_total_missing_audio_sample_entry_count` / `hisui_total_received_audio_sample_entry_count`）と warn ログ（`Missing sample entry for first sample of Audio track`）を使う。これらは feature/fix-hybrid-writer-finalize-on-stop-record で追加済み。
- 修正完了後に issues/0011 を close する。

## 関連

- issues/0011（reopen 済み。本 issue で真の修正を行う）
- issues/0008（先行する flaky テストの issue。closed）
