# 未使用の next_encoded_frame を削除して EncoderOutputSender を pub(crate) 化する

- Priority: Low
- Created: 2026-07-09
- Completed: {YYYY-MM-DD}
- Model: Claude Opus 4.7
- Branch: feature/refactor-remove-unused-encoder-api
- Polished: {YYYY-MM-DD}
- Reporter: @sile
- Decision Owner: @sile

## 目的

`VideoEncoder::next_encoded_frame` (`pub async fn next_encoded_frame(&mut self) -> Option<crate::Result<VideoFrame>>`、 実体は `self.rx.recv().await` だけ) は本番コードから 1 箇所も呼ばれていない未使用 public API である。 closed issue 0067 で「Encoder の pull 型直接利用への将来拡張余地」として追加され、 closed issue 0083 の wrap 削除 + rename 完了時点でも本番 caller は 0 件のまま。 本 issue はこの将来拡張余地を放棄して API を削除し、 `VideoEncoder` の出力取得モデルを実際に使われている同期 poll 経路一本に収束させる。

同時に `EncoderOutputSender` type alias の可視性 (`pub type`) を `pub(crate) type` に引き下げる。 encoder 側 `OutputSink` は decoder 側と異なり `pub fn new` を持たず、 `OutputSink { tx, total_output_metric, total_output_keyframe_metric }` の struct literal 構築でしか作られない。 crate 外呼出 (`tests/encoder_tests.rs` および `tests/e2e.rs`) は `OutputSink::new` を通らないため、 `EncoderOutputSender` は現時点でも公開シグネチャに露出しない。 削除に連動して非対称を解消する。

closed/0057 §3 分割表 line 363 で「(未起票) encoder 未使用 API 削除 refactor issue: 使用側移行完了後の dead code 削除 + `EncoderOutputReceiver` 可視性整理」として予告済み。 decoder 系列の対称 precedent は closed/0078 (`feature/refactor-remove-unused-next-decoded-frame`)。

将来拡張余地を放棄する根拠: (1) 本番経路 (compose / vmaf / list_codecs / `create_video_processor(_with_params)`) はすべて同期の `handle_input_sample` + `poll_output` の drain ループで EOS を扱い、 非同期 EOS シグナルを要求する使用側は現時点で見えない (2) 仮に将来必要が生じても、 Sender の `drop` シグナル (現在の `Option<crate::Result<VideoFrame>>` の `None`) より `enum { Ok, EndOfStream, Err }` のような明示型を新たに導入する方が意図が明瞭で、 現状の `Option` 保持は必然性が薄い (closed/0078 と同判断)。

## 優先度根拠

Low。

- 本番挙動は不変 (未使用 API の削除 + 可視性引き下げ)
- 実装コストは軽微 (純削除 3 箇所 + docstring 参照除去 + 可視性引き下げ 1 箇所)
- closed/0067 / closed/0083 の追加・維持判断を覆すため、 PR に対する Decision Owner 承認プロセスが必要 (詳細は §解決方法 §1、 closed/0078 と同型)

## 現状

closed issue 0083 (同期 wrap `VideoEncoder` 削除 + `AsyncVideoEncoder` → `VideoEncoder` リネーム + `_sync` / `_async` サフィックス整理、 2026-07-09 マージ済み、 merge commit `66663c37`) 時点の `src/encoder.rs` を基準とする。

- `VideoEncoder::next_encoded_frame` の本番呼出はゼロ。 本番の映像エンコード出力取得は、 processor 経路 (`VideoEncoder::run`、 compose / vmaf / `create_video_processor_with_params` 内の spawn クロージャから呼び出される) がすべて同期の `handle_input_sample` + `poll_output` (内部 `try_recv`) を経由する
- 唯一の呼出元は「そのメソッド自身を試すためだけのテスト」2 件 (`src/encoder.rs:1330` の `next_encoded_frame_returns_frame_after_emit_ok` / `:1347` の `next_encoded_frame_propagates_error_from_emit_err`)
- struct docstring (`src/encoder.rs:461-472`) は closed/0083 のリネーム時に「`handle_input_sample` / `poll_output` で同期駆動する映像エンコーダー」に更新済みで、 `next_encoded_frame` への対比参照は既に存在しない
- `EncoderOutputSender` は `pub type` (`src/encoder.rs:368`) だが、 crate 外で使う唯一の経路 `OutputSink` の構築は `impl OutputSink` に `pub fn new` を持たず struct literal (`src/encoder/test_helpers.rs:54` および `src/encoder.rs:519`) で行う。 したがって公開シグネチャに `EncoderOutputSender` は露出しない (`rg 'EncoderOutputSender' src/ tests/` の hit は `src/encoder.rs` の定義行 `:368` と `OutputSink` の field 型 `:384` の 2 箇所のみ、 crate 外参照はゼロ)
- `EncoderOutputReceiver` は既に `pub(crate) type` (`src/encoder.rs:371-372`)。 本 issue で追加変更は不要

## 設計方針

closed/0078 (decoder 側 precedent) と同型で実施する。 encoder 特有の差分は `EncoderOutputSender` の可視性引き下げが同 issue に統合される点 (decoder 側は tests/e2e.rs が `OutputSink::new` を呼ぶため `Sender` は pub 維持だった、 encoder 側は非対称 = 引き下げ可能)。

### 削除・書き替え対象

行番号は着手時に `rg 'next_encoded_frame' src/ tests/ pbt/ fuzz/ examples/` で再特定する (以下は 2026-07-09 時点の実測位置)。 純削除項目は削除範囲に末尾空行 1 行を含める (`cargo fmt --check` の空行連続検出を回避)。

1. **[純削除] `next_encoded_frame` メソッド定義** — `src/encoder.rs:753-761` の docstring + signature + body 全体 + 末尾空行 1 行 (実測)
2. **[純削除] `next_encoded_frame` 専用テスト 2 件** — `src/encoder.rs:1319-1361` の label コメントブロック (`---- next_encoded_frame の pub 契約テスト ----` 以下) と 2 テスト (`next_encoded_frame_returns_frame_after_emit_ok` / `next_encoded_frame_propagates_error_from_emit_err`) 全体 + 末尾空行 1 行
3. **[可視性引き下げ] `EncoderOutputSender` の `pub type` → `pub(crate) type`** — `src/encoder.rs:367-368` の 1 行を `pub(crate) type EncoderOutputSender = ...` に変更 (docstring は維持)

### 維持対象 (削除に伴い触らない)

- **`src/encoder.rs:484-490` の drop 順制御コメント** と **`:491-492` のフィールド宣言順** — Nvcodec の worker drop 中に callback が `sink.emit_ok` → `tx.send` した際に `rx` を alive に保つ契約
- **`src/encoder.rs:461-472` の struct docstring** — closed/0083 のリネーム時に「`handle_input_sample` / `poll_output` で同期駆動する映像エンコーダー」形に更新済みで、 `next_encoded_frame` への対比参照は既に存在しない。 本 issue で追加変更不要
- **`src/encoder.rs:374-380` の `OutputSink` docstring** — closed/0083 のリネーム時に「`poll_output` の `Disconnected` 分岐も `unreachable!()` で潰す」の言及を含む形に更新済みで、 `next_encoded_frame` への対比参照は存在しない。 本 issue で追加変更不要
- **`EncoderOutputReceiver` の `pub(crate) type`** (`src/encoder.rs:371-372`) — 既に `pub(crate)` で外部露出なし。 追加変更不要
- **`impl OutputSink` の `pub fn emit_ok` / `pub fn emit_err`** — struct literal 構築される `OutputSink` の pub method で、 crate 外 (integration test 経由の間接呼出可能性) から呼び出される可能性を保つため触らない

### 削除による失われるカバレッジと代替担保

削除対象テスト 2 件は `AsyncVideoEncoder`/`VideoEncoder` (rename 後) 内部 `sink` を直接叩いて `rx.recv().await` で受け取る契約を検証する。 兄弟テスト `poll_output_returns_processed_when_frame_available` / `poll_output_propagates_error_from_rx` (`src/encoder.rs::tests`) が `sink.emit_ok` / `emit_err` → `poll_output` の同期経路を等価にカバーしており、 削除で失われるのは async 経路 (`rx.recv().await`) の直接検証のみ。 `rx.recv().await` は tokio の unbounded channel の標準 API で hisui 側実装なしの薄い透過呼出のため、 hisui 側の regression 検出価値は薄い (closed/0078 と同判断)。

### shiguredo-rust 規約整合

- モック / スタブ不使用
- 新規 trait 追加なし
- `#[non_exhaustive]` 不使用 (enum バリアント変更なし)

## 完了条件

- 削除対象 3 項目がすべて反映されている (§設計方針 §削除・書き替え対象)
- `EncoderOutputSender` の可視性が `pub(crate)` に変更されている
- 変更ファイルは `src/encoder.rs` のみ (`git diff --name-only develop...HEAD -- src tests` で確認)
- grep 検証:
  - `rg 'next_encoded_frame' src/ tests/ pbt/ fuzz/ examples/` の hit が 0 件
  - `rg 'pub\(crate\) type EncoderOutputSender' src/` の hit が 1 件 (新可視性の適用確認)
  - `rg 'pub type EncoderOutputSender' src/` の hit が 0 件 (旧 pub type の残骸検出)
- closed/0057 §3 分割表 (`issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md`) の line 363 の未起票行を本 issue に対応させる (実装 PR で 5 セル形式に置換、 詳細は §解決方法 §5)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

### 1. 承認と分岐

削除実装は通常の refactor issue フローに従う。 承認プロセスは closed/0078 と同型:

- 削除 branch (`feature/refactor-remove-unused-encoder-api`) を切って §2〜§4 の実装を進め、 PR を開設する
- Decision Owner (@sile) の PR review LGTM が承認確定
- 承認見送りの場合の運用は closed/0078 §解決方法 §1 の precedent を踏襲する (見送り理由を本 issue 末尾に `## 見送り記録` セクションで永続化する)

### 2. 実行番号を再特定

`rg 'next_encoded_frame|EncoderOutputSender' src/ tests/ pbt/ fuzz/ examples/` で削除・書き替え対象の実行番号を再特定する (§設計方針の再掲。 着手時に必ず実施)。

### 3. 削除・可視性引き下げを単一コミットで完成させる

§設計方針 §削除・書き替え対象の 3 項目を単一コミット `0084 未使用の next_encoded_frame を削除して EncoderOutputSender を pub(crate) 化する` (コード変更コミット形式 `{SEQ} {変更内容}`。 issue タイトル原文を流用) に収める。 可視性引き下げは削除と論理的に一体 (`next_encoded_frame` 削除で `EncoderOutputSender` の crate 外用途が完全に消え、 encoder 側の非対称な `pub` を維持する根拠がなくなるため、 削除に付随して整えることで削除後の型面と可視性が一貫する)。

### 4. cargo 検証

§完了条件の cargo コマンドを default + `--no-default-features` の両方で PR 開設前の local で通す。

### 5. closed/0057 §3 分割表の更新

本 issue の実装 PR に closed/0057 §3 分割表更新を含める (別コミット):

- **line 351 の依存順序記述** の更新: 現状 `encoder 系列: 0066 → 0067 → closed/0079 → closed/0083 → 未起票 encoder 未使用 API 削除` の `未起票 encoder 未使用 API 削除` を `closed/0084` に置換
- **line 363 の未起票行を本 issue の 5 セル形式に置換** (推定 LOC は実装完了時点の `git diff --stat develop` からコード限定基準で `+X/-Y` 形式に記入。 提出時点で `closed/0084` として書く。 依存先セルは他行に合わせて数字表記 `0083` に統一):
  ```
  | closed/0084 (`feature/refactor-remove-unused-encoder-api`) | 未使用の `VideoEncoder::next_encoded_frame` 削除 + `EncoderOutputSender` の pub → pub(crate) 引き下げ | <+X/-Y> | 0083 | 内部 API のみ |
  ```

### 6. マージ後の closing

PR merge 完了直後に Reporter (@sile) が `develop` ブランチで単一コミット `0084 closed 未使用の next_encoded_frame を削除して EncoderOutputSender を pub(crate) 化する` (`Completed:` を PR merge 日に更新 + `git mv issues/0084-....md issues/closed/`) で closing する。

## CHANGES.md について

内部リファクタにつき記載不要。 hisui は bin crate として配布され、 `VideoEncoder` 系は外部公開していない。 外部プロトコル / 出力は不変。

## 関連

- closed/0067 (`feature/refactor-add-async-video-encoder`、 2026-07-08 merge): 本 API を追加した親 issue。 本 issue はその追加判断のうち、 将来拡張余地としての `next_encoded_frame` 保持部分を撤回する (Sender 経由の出力統一の骨格は撤回しない)
- closed/0078 (`feature/refactor-remove-unused-next-decoded-frame`、 2026-07-07 merge): decoder 系列の対称 precedent。 本 issue と同型のクリーンアップ (未使用 pull API 削除 + Receiver 可視性整理)。 encoder 側は `Sender` 側の pub 非対称も追加で解消する差分あり
- closed/0079 (`feature/refactor-migrate-video-encoder-users-to-async`、 2026-07-08 merge): 使用側移行。 本 API は触られず保持継続された
- closed/0083 (`feature/refactor-remove-sync-video-encoder-and-rename`、 2026-07-09 merge): 依存先。 wrap 削除 + rename + サフィックス整理を完了。 本 issue の発端 (0083 の `review-diff-code` で削除候補として検出、 かつ closed/0057 §3 分割表 line 363 の予告に該当)
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 採用案 C の親 issue。 本 API は 0067 の派生方針 (δ) で導入された非同期取り出し API で、 削除の影響は 0057 §3 本体判断 (Sender 経由の出力統一) には及ばない。 分割表 line 363 の未起票行を本 issue に対応させる (完了条件 / §解決方法 5 参照)
- open/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`): 独立に着手可能な perf issue。 本 issue の完了は 0080 の前提としない、 逆も同様
