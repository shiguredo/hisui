# HlsWriter の MpegTs 経路で sample_entry None 時の静かな劣化を Err 化する

- Priority: Low
- Created: 2026-06-23
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-hls-mpegts-fail-on-missing-sample-entry
- Polished:

## 目的

issue 0051 で writer 入口の sample_entry fallback 補完経路を全削除した結果、`HlsWriter` の MpegTs 経路だけが「不変条件違反フレーム流入時に Err にもならず、不正な ADTS / AnnexB を静かに出力する」唯一の経路として残った。他の writer と同じく Err 化することで fail-safety を揃え、本 PR 適用後に静かな破壊が起きるリスクを取り除く。

## 優先度根拠

Low。issue 0051 で確立した入力側不変条件のもとでは違反フレームは writer に届かないため、現状の実装でも発火しない経路。ただし将来の入力経路追加で前提が崩れた場合に「静かな破壊」が起きるリスクを取り除く保険として価値が高い。0054（encoder 側の出力保留設計）と対で実施することで、writer 入口の fallback 削除に伴う fail-safety 補強を完成させる。

## 現状

issue 0051 で writer 入口の sample_entry fallback 補完経路を削除した結果、writer 側の違反流入時の挙動は以下のように分かれる:

- `Mp4Writer` / `HybridMp4Writer` 経路: muxer の `MissingSampleEntry` Err でパイプライン fail-fast 停止
- `DashWriter` / `HlsWriter` の fMP4 経路: 同じく muxer Err、上位の `tracing::warn!` で握り潰されて配信は止まらない（不正出力にはならない）
- **`HlsWriter` の MpegTs 経路**: Err にもならず、ハードコードフォールバックで不正な ADTS / AnnexB を静かに出力。運用上はファイル再生時に初めて気づく経路になる

該当ヘルパ（行番号は develop 時点。実装着手時に再特定する）:

- `src/hls/writer.rs::convert_length_prefixed_to_annexb`: `sample_entry` が `Avc1` でない場合は `length_size: 4` （AVC のデフォルト）を使う
- `src/hls/writer.rs::extract_aac_config`: `sample_entry` が `Mp4a` でない場合は AAC LC（object_type=2）/ 48kHz（sampling_frequency_index=3）/ stereo（channel_configuration=2）を返す

issue 0051 以前は writer 入口の fallback 補完経路で違反フレームを `Patched` で救済（直前の正常 sample_entry で差し替え）していたため、このハードコードフォールバックは実質的に発火しない経路だった。issue 0051 で fallback を削除した結果、不変条件違反が起きた場合の唯一の挙動として MpegTs 経路でだけ「静かに不正な出力」が残った。

## 設計方針

### 1. ヘルパ関数で sample_entry None / 期待型と異なる場合に Err を返す

`src/hls/writer.rs` の以下を改修する。

- `convert_length_prefixed_to_annexb`: `sample_entry` が `None` か `Avc1` 以外の場合は `Err` を返す
- `extract_aac_config`: `sample_entry` が `None` か `Mp4a` 以外の場合は `Err` を返す

これにより、`HlsWriter::handle_video_frame` / `handle_audio_frame` の MpegTs 経路で違反フレームが流入した場合は `Err` で上位に伝播し、`run` メソッドの `tracing::warn!("HLS ... error: {e}")` で握り潰されてログに残るが、不正な出力ファイルは生成されない（該当フレームだけスキップされる）。これは `DashWriter` / 他の writer と同じ fail-fast 寄りの設計に揃う。

### 2. テスト追加

`src/hls/writer.rs` の `mod tests`（現状無いため新設）で以下を追加する。

- `convert_length_prefixed_to_annexb` に sample_entry: None を渡すと Err になることを assert
- `convert_length_prefixed_to_annexb` に sample_entry: Mp4a（期待外型）を渡すと Err になることを assert
- `extract_aac_config` に sample_entry: None を渡すと Err になることを assert
- `extract_aac_config` に sample_entry: Avc1（期待外型）を渡すと Err になることを assert

### CHANGES.md

本 issue で変更する範囲は内部実装の fail-safety 補強で外部 API 変更を伴わない。`shiguredo-changelog` の「派生元ブランチとの最終的な差分のみを記載すること」に従って判断する。リリース時に観測可能な挙動変化（Err 発生条件の変化）があるなら記載、無いなら記載なし。実装時に最終判断する。

## スコープ

含むもの:

- `src/hls/writer.rs::convert_length_prefixed_to_annexb` の改修と単体テスト追加
- `src/hls/writer.rs::extract_aac_config` の改修と単体テスト追加

含まないもの:

- `Mp4Writer` / `HybridMp4Writer` 経路（既に muxer Err で fail-fast 化されている）
- `HlsWriter` の fMP4 経路と `DashWriter`（こちらは muxer Err 経路で握り潰されるが、不正出力にはならない）
- writer 入口の fallback 復活（issue 0051 で確立した「責任の所在を入力側に集約する」方針を維持する）

## 完了条件

- `convert_length_prefixed_to_annexb` / `extract_aac_config` で sample_entry None / 期待外型のときに Err を返すことが単体テストで保証されること
- `cargo check && cargo clippy --all-targets -- --deny warnings && cargo test` が通ること
- 既存 e2e テスト（`e2e-tests/obsws/test_output.py` 等の HLS 関連）が引き続き通ること

## 関連

- closed/0051（writer 入口 fallback 削除。本 issue の前提。本 issue 着手時点で closed されている想定）
- 0054（openh264 / VideoToolbox エンコーダ側の出力保留設計。本 issue と性質が一対）
- closed/0034（writer 入口 fallback の導入）
- `docs/internals/sample_entry_invariant.md`
