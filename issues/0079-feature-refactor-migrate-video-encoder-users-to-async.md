# compose / recording / vmaf / obsws / list_codecs を AsyncVideoEncoder 直接利用に移行して AsyncVideoEncoder::run を追加する

- Priority: Medium
- Created: 2026-07-07
- Completed: {YYYY-MM-DD}
- Model: Opus 4.7
- Branch: feature/refactor-migrate-video-encoder-users-to-async
- Polished: {YYYY-MM-DD}
- Reporter: @sile
- Decision Owner: @sile

## 目的

open issue 0067 で確立された派生方針 (δ) 「`AsyncVideoEncoder` 追加 + 既存 `VideoEncoder` は wrap 構造で挙動維持」の後続として、 本番使用側 (compose / recording / vmaf / obsws / list_codecs) を wrap `VideoEncoder` から `AsyncVideoEncoder` 直接利用に移行する。 同時に processor モデル用の駆動 API として `AsyncVideoEncoder::run` を新規追加する。

closed issue 0068 (decoder 側の対称 issue) と同じパターンで実施する。 本 issue 完了で、 wrap `VideoEncoder` の本番使用側がゼロになり、 後続の wrap 削除 + rename issue の下地が整う。 closed/0057 §3 採用案 C の「中途半端な 2 系統共存を残さない」原則の最終達成に向けた段階移行の 1 ステップ。

## 優先度根拠

Medium。

- open issue 0067 で採用案が確定済み、 (δ) 方針の後続として着手する
- Priority は decoder 系列 0068 と対称 (Medium 維持)
- 依存先: 0067 (`AsyncVideoEncoder` 追加 + wrap 化 + inner Sender 化) の PR merge 後に着手可能

## 現状

open issue 0067 完了後の `src/encoder.rs` の構造を基準とする。 wrap `VideoEncoder` の本番使用側は以下の 6 call site:

### `VideoEncoder::new` を直接呼ぶ本番使用側 (2 call site)

- `src/sora/recording_subcommand_compose.rs:577` (`compose` サブコマンド)
- `src/sora/recording_subcommand_vmaf.rs:456` (`vmaf` サブコマンド)

### `create_video_processor` / `create_video_processor_with_params` (`src/encoder.rs:997-1059`) 経由の間接使用側 (3 call site)

- `src/obsws/coordinator/output.rs:718` (obsws output)
- `src/obsws/coordinator/output_dash.rs:894` (obsws DASH)
- `src/obsws/coordinator/output_hls.rs:911` (obsws HLS)

これら 3 call site は `create_video_processor_with_params` 内の `VideoEncoder::new` (`src/encoder.rs:1052`) 経由で間接呼出する。 `create_video_processor` / `create_video_processor_with_params` の pub シグネチャを変えないため、 使用側 3 call site の書き換えは不要。

### `VideoEncoder::get_engines` の使用側 (1 call site)

- `src/subcommand_list_codecs.rs:88` (`list-codecs` サブコマンド)

これらすべての `VideoEncoder` 参照を `AsyncVideoEncoder` に切り替える。 wrap `VideoEncoder` 型自体の削除は本 issue のスコープ外 (後続の wrap 削除 + rename issue で扱う)。

## 設計方針

closed issue 0068 (decoder 側の対称 issue) の設計方針をそのまま encoder 側に移し替える。

### `AsyncVideoEncoder::run` の追加

processor モデル (`ProcessorHandle` + subscribe / publish) 用の駆動 API を新規追加する。 wrap 側 `VideoEncoder::run` (`src/encoder.rs:615-661` の 2 腕 `tokio::select!`: 入力 + RPC) と同じロジックを、 wrap を介さず `AsyncVideoEncoder` 自身のフィールド (`inner`, `sink`, `keyframe_request_pending` 等) と `handle_input_sample_sync` / `poll_output_sync` / `handle_rpc_message_sync` を直接呼び出す形で書き直す。 使用側は wrap 側 helper (`drain_video_encoder_output`) を経由せず、 `AsyncVideoEncoder::run` から `output_tx.send_media()` に直接流す。

### 各使用側の移行

- `src/sora/recording_subcommand_compose.rs:577` / `src/sora/recording_subcommand_vmaf.rs:456`: `VideoEncoder::new(...)` を `AsyncVideoEncoder::new(...)` に置換し、 `encoder.run(...)` を `AsyncVideoEncoder::run(...)` に切り替える
- `src/encoder.rs:1025-1059` `create_video_processor_with_params`: 内部の `VideoEncoder::new` を `AsyncVideoEncoder::new` に、 `encoder.run(...)` を `AsyncVideoEncoder::run` に切り替える。 pub シグネチャは維持する (obsws 使用側 3 call site は無変更)
- `src/subcommand_list_codecs.rs:88`: `VideoEncoder::get_engines(...)` を `AsyncVideoEncoder::get_engines(...)` に置換する (0067 で移植済み)

### wrap `VideoEncoder` の存置

本 issue では wrap 型自体は削除しない。 移行完了時点で wrap 側の本番呼出はゼロになるが、 テストや `drain_video_encoder_output` helper 経由の参照が残る可能性がある。 これらの整理と wrap 型の物理削除は closed/0073 相当の後続 issue (wrap 削除 + rename) で扱う。

### shiguredo-rust 規約整合

- モック / スタブ不使用 (テストは実 encoder + tokio channel + 実 pipeline)
- 新規 trait 追加なし
- `#[non_exhaustive]` 不使用

## 完了条件

- `AsyncVideoEncoder::run` が新規追加され、 processor モデル用の駆動 API を提供する
- 上記 6 call site の `VideoEncoder` 参照 (`::new` / `::run` / `::get_engines`) がすべて `AsyncVideoEncoder` に置換されている
- `create_video_processor` / `create_video_processor_with_params` の pub シグネチャは不変 (obsws 使用側は無変更で通る)
- grep 検証:
  - `grep -rn '\bVideoEncoder::new\b\|\bVideoEncoder::run\b\|\bVideoEncoder::get_engines\b' src/` の hit が 0 件 (本番使用側からの `VideoEncoder` 直接呼出がすべて消えていることの確認)
  - wrap 型 `pub struct VideoEncoder` の定義自体は残る (本 issue のスコープ外)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

1. `AsyncVideoEncoder::run` を `src/encoder.rs` に新規追加する (wrap 側 `VideoEncoder::run` の 2 腕 `tokio::select!` ロジックを移植し、 wrap を介さず自身のフィールドと `_sync` API を直接呼び出す形に書き直す)
2. `src/sora/recording_subcommand_compose.rs:577` の `VideoEncoder::new` → `AsyncVideoEncoder::new`、 `encoder.run(...)` → `AsyncVideoEncoder::run` に置換する
3. `src/sora/recording_subcommand_vmaf.rs:456` の同 (2 と同じパターン)
4. `src/encoder.rs:1025-1059` `create_video_processor_with_params` 内の `VideoEncoder::new` を `AsyncVideoEncoder::new`、 `encoder.run(...)` を `AsyncVideoEncoder::run` に切り替える (pub シグネチャは維持)
5. `src/subcommand_list_codecs.rs:88` の `VideoEncoder::get_engines` を `AsyncVideoEncoder::get_engines` に置換する
6. 完了条件の cargo コマンドを default + `--no-default-features` の両方で通す

## CHANGES.md について

内部リファクタにつき記載不要。 `VideoEncoder` 系は library として外部公開していない (hisui の lib target は crates.io 未 publish で workspace 内 bin / tests 専用)。

## 関連

- open/0067 (`feature/refactor-add-async-video-encoder`): 依存先。 本 issue は 0067 の PR merge 後に着手する
- closed/0068 (`feature/refactor-migrate-video-decoder-users-to-async`): decoder 側の対称 issue。 本 issue と同じパターン (`AsyncVideoDecoder::run` 追加 + processor 経路移行) を encoder 側に移し替える
- closed/0071 (`feature/refactor-mp4-reader-async-video-decoder`) / closed/0072 (`feature/refactor-inbound-endpoint-async-video-decoder`): decoder 側の追加移行実例 (mp4 reader / inbound endpoint)。 encoder 側は outbound で該当なし、 本 issue 1 件で使用側移行が完結する
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 採用案 C の親 issue。 §3 分割表の「encoder 使用側移行 refactor issue」行に本 issue を対応させる
- (未起票) encoder wrap 削除 + rename refactor issue (closed/0073 相当): 本 issue の PR merge 後に起票 (同期 wrap `VideoEncoder` 削除 + `AsyncVideoEncoder` → `VideoEncoder` リネーム)
