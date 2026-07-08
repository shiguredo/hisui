# 同期 VideoEncoder wrap を削除して AsyncVideoEncoder を VideoEncoder にリネームする

- Priority: Medium
- Created: 2026-07-08
- Completed: {YYYY-MM-DD}
- Model: Claude Opus 4.7
- Branch: feature/refactor-remove-sync-video-encoder-and-rename
- Polished: {YYYY-MM-DD}
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0066 で確立され closed issue 0067 で encoder 側に展開された派生方針 (δ) 「`Async*` 新規追加 + 既存を wrap 化 + 段階移行」は、 closed issue 0079 で全使用側 (compose / vmaf / list_codecs / `create_video_processor(_with_params)`) が `AsyncVideoEncoder` に切り替わった時点で「同期 wrap を削除し、 `AsyncVideoEncoder` を `VideoEncoder` にリネームする」ことで最終形に到達する。 本 issue はその最終ステップを扱う。

closed/0079 の完了 (2026-07-08 develop merge) により、 wrap `VideoEncoder` は本番経路での使用ゼロ (参照はテストと wrap 側 helper 経由のみ) の実質 dead code になっている。 closed issue 0057 §3 採用案 C の長所 (v) 「callback friendly 定義 (ホップ数上限 1)」は、 wrap の 2 段ホップ (`VideoEncoder::poll_output` → `AsyncVideoEncoder::poll_output_sync`) が型として残る限り最終達成にならない。 本 issue で wrap 型を消し、 `AsyncVideoEncoder` を `VideoEncoder` にリネームして命名を最終化する。

decoder 系列の closed/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`) を先例として、 encoder 側の RPC 経路 (`recv_video_encoder_rpc_message_or_pending` helper と `handle_rpc_message_sync` 内部 API) を加味して同型で実施する。

## 優先度根拠

Medium。

- closed issue 0057 §3 の 2 系統共存を最終解消する方針 (δ、 §3 備考) との最終整合は本 issue でしか達成できない
- 本 issue 単独では外部挙動は不変。 内部型名の整理のため緊急性は低い
- ただし wrap 状態のまま放置すると「`AsyncVideoEncoder` と `VideoEncoder` のどちらを使うべきか」の API 選択の負債が蓄積する
- 依存先 closed/0079 は 2026-07-08 に develop merge 済みで着手条件成立
- Priority は decoder 系列 closed/0073 と対称 (Medium 維持)

## 現状

closed issue 0079 完了時点 (2026-07-08 develop merge、 merge commit `0943e9d6`) の `src/encoder.rs` の構造:

```rust
// AsyncVideoEncoder (`:474-499`)
pub struct AsyncVideoEncoder { ... }

impl AsyncVideoEncoder {
    pub fn new(...) -> Result<Self>                                                        // :501
    pub fn name(&self) -> Option<EngineName>                                               // :629
    pub fn codec(&self) -> Option<CodecName>                                               // :633
    pub fn get_engines(codec, is_openh264_available) -> Vec<EngineName>                    // :637
    pub(crate) fn handle_rpc_message_sync(&mut self, VideoEncoderRpcMessage)               // :684
    pub(crate) fn handle_input_sample_sync(&mut self, Option<MediaFrame>) -> Result<()>    // :698
    pub(crate) fn poll_output_sync(&mut self) -> Result<EncoderRunOutput>                  // :727
    pub async fn next_encoded_frame_async(&mut self) -> Option<Result<VideoFrame>>         // :751
    pub async fn run(mut self, ProcessorHandle, TrackId, TrackId) -> Result<()>            // :761 (0079 で追加)
}

// 同期 wrap (`:825-919`)。 全メソッドが AsyncVideoEncoder への委譲
pub struct VideoEncoder { inner_encoder: AsyncVideoEncoder }
impl VideoEncoder {
    // new (:830) / name / codec / get_engines / run / handle_rpc_message /
    // handle_input_message / handle_input_sample / poll_output
}

pub fn drain_video_encoder_output(encoder: &mut VideoEncoder, ...) -> Result<bool>          // :922
async fn recv_video_encoder_rpc_message_or_pending(...) -> Option<VideoEncoderRpcMessage>   // wrap 側と AsyncVideoEncoder::run の両方から使う
```

wrap の本番使用はゼロであることを確認済み:

- `drain_video_encoder_output` の呼出は wrap 側 `run` 内のみ (0079 で使用側移行完了により本番経路の呼出は消滅)
- wrap 側 `handle_input_message` の呼出も wrap 側 `run` 内のみ。 wrap 削除で公開 API から消えるが外部影響なし (`AsyncVideoEncoder::run` は `Message` dispatch を自前展開済み)
- `recv_video_encoder_rpc_message_or_pending` は `AsyncVideoEncoder::run` からも呼ばれるため存置する

残存する wrap `VideoEncoder` への参照は次の 2 群に分類される:

**(a) リネーム後にテキスト無変更で新型に解決される箇所 (実作業なし)**

- `tests/encoder_tests.rs` の 3 テスト (`:56` / `:92` / `:152` の `#[test]` 行、 内部で `VideoEncoder::new` (`:65` / `:109` / `:166`) を使う): closed/0067 で追加された wrap 側 pub API の integration test。 `new` / `handle_input_sample` / `poll_output` はリネーム後 (新 `VideoEncoder` = 旧 `AsyncVideoEncoder`) にサフィックス整理後の `handle_input_sample` / `poll_output` にテキスト無変更で解決される (§決定事項 の可視性 pub 化前提)

**(b) 書換が必要な箇所 (§tests への影響 / §使用側の追随で扱う)**

- 使用側 4 hit の `AsyncVideoEncoder::` prefix (0079 で置換した compose:577 / vmaf:456 / `create_video_processor_with_params:1226` / `list_codecs:88`) と、 use 文 3 箇所 (`compose.rs:15` / `vmaf.rs:15` / `list_codecs.rs:7`)
- `tests/encoder_tests.rs` の 4 番目テスト `video_encoder_run_processes_i420_via_async_pipeline` (0079 で追加) 内の `AsyncVideoEncoder::new` (`:265`) と use 文 (`:6`)
- `src/encoder.rs` 内の unit tests (`#[cfg(test)] mod tests`) 内の `AsyncVideoEncoder::new` (`:1381`) と型注釈 (`new_uninitialized_encoder` の戻り値型 `AsyncVideoEncoder` `:1368`) 、 メソッド呼出 (`poll_output_sync` / `next_encoded_frame_async` )

## 設計方針

closed/0073 (decoder 側 precedent) と同型で実施する。

### 削除対象 (`src/encoder.rs`)

- `pub struct VideoEncoder` (wrap 型、 `:825`) と `impl VideoEncoder` (`:829-919`) の全メソッド (`new` / `name` / `codec` / `get_engines` / `run` / `handle_rpc_message` / `handle_input_message` / `handle_input_sample` / `poll_output`)
- `pub fn drain_video_encoder_output` (`:922`)。 wrap `run` 内以外の呼出はなく、 wrap 削除で不要になる

### 存置対象

- `async fn recv_video_encoder_rpc_message_or_pending`: `AsyncVideoEncoder::run` (リネーム後 `VideoEncoder::run`) から引き続き呼ばれるため存置する。 free fn のままか、 リネーム後の `impl VideoEncoder` の associated fn 化するかは polish で確定させる

### リネーム対象

- `pub struct AsyncVideoEncoder` → `pub struct VideoEncoder` (`:474`)
- 型エイリアス `EncoderOutputSender` / `EncoderOutputReceiver` の名前はそのまま維持
- メソッドの `_sync` / `_async` サフィックス削除 (§決定事項 1):
  - `handle_input_sample_sync` → `handle_input_sample`
  - `poll_output_sync` → `poll_output`
  - `handle_rpc_message_sync` → `handle_rpc_message`
  - `next_encoded_frame_async` → `next_encoded_frame`
- 可視性の整理 (§決定事項 2): wrap 消滅で名前空間分離の必要が消えるため、 現状 `pub(crate)` の 3 API のうち integration test から使うものは `pub` に格上げする
  - `handle_input_sample_sync` (現 `pub(crate)`) → `handle_input_sample` (`pub`)
  - `poll_output_sync` (現 `pub(crate)`) → `poll_output` (`pub`)
  - `handle_rpc_message_sync` (現 `pub(crate)`) → `handle_rpc_message` (`pub(crate)` 維持、 内部利用のみ)
- 型名の rename 対象 (grep で全数把握、 本 issue で全て置換):
  - `src/encoder.rs` 内: `AsyncVideoEncoder` の全参照 (docstring / コメント / impl / use)
  - `src/sora/recording_subcommand_compose.rs` / `recording_subcommand_vmaf.rs` / `src/subcommand_list_codecs.rs` の use 文と call site
  - `src/encoder.rs:1226` (`create_video_processor_with_params` 内)
  - `tests/encoder_tests.rs` の use 文と call site + docstring
- コメント / docstring 内の型名・メソッド名参照も同時に書き換える (特に `AsyncVideoEncoder` 型 docstring (`:461-472`) / `handle_rpc_message_sync` docstring (`:681-682`) は 0079 で更新済み。 リネームに追随)

### tests への影響

- `tests/encoder_tests.rs`:
  - use 文 (`:6`): `encoder::{AsyncVideoEncoder, EncoderRunOutput, VideoEncoder, VideoEncoderOptions, default_video_encode_config_for_rpc}` を `encoder::{EncoderRunOutput, VideoEncoder, VideoEncoderOptions, default_video_encode_config_for_rpc}` に縮約 (`AsyncVideoEncoder` の import 削除、 `VideoEncoder` は残す)
  - 4 番目テスト `video_encoder_run_processes_i420_via_async_pipeline`: テスト名の `_via_async_pipeline` を `_via_pipeline` に整理し、 内部 `AsyncVideoEncoder::new` (`:265`) を `VideoEncoder::new` に置換 (`AsyncVideoEncoder::` の :: がなくなり VideoEncoder wrap との名前空間衝突なし)
  - docstring `/// `AsyncVideoEncoder::run` (processor 経路) の end-to-end 契約` (`:198`) の `AsyncVideoEncoder` を `VideoEncoder` に書き換え
  - 1 〜 3 番目のテスト (wrap 経路の pub API `handle_input_sample` / `poll_output` を叩く) は §現状 (a) のとおりリネーム後にテキスト無変更で解決される
- `src/encoder.rs` の `#[cfg(test)] mod tests`:
  - `AsyncVideoEncoder::poll_output_sync 分岐テスト` (`:1360`) 等のコメント内型名・メソッド名を新名 `VideoEncoder::poll_output` に書き換え
  - `new_uninitialized_encoder` fn (`:1368`) の戻り値型 `AsyncVideoEncoder` → `VideoEncoder`、 内部 `AsyncVideoEncoder::new` (`:1381`) → `VideoEncoder::new`、 `.expect("AsyncVideoEncoder::new が失敗した")` の文言更新
  - `poll_output_sync` / `next_encoded_frame_async` 呼出を `poll_output` / `next_encoded_frame` に追随
  - テスト名の `poll_output_sync_returns_*` / `next_encoded_frame_async_*` のサフィックス整理 (`_sync` / `_async` 削除)

### 決定事項 (実装で覆さない)

1. **メソッド命名**: `_sync` / `_async` サフィックスは全削除する。 wrap 削除で `handle_input_sample` / `poll_output` / `handle_rpc_message` の名前は空き、 `next_encoded_frame` にも衝突はない。 `next_encoded_frame` の非同期性は `async fn` シグネチャから自明で、 同一型内に同期版が存在しなくなるため区別サフィックスは不要 (closed/0073 と同判断)
2. **可視性**: integration test (crate 外) から呼ぶ `handle_input_sample` / `poll_output` は `pub` に格上げする。 `handle_rpc_message` は内部利用のみで `pub(crate)` 維持
3. **リネーム実施順序**: option A (wrap 削除 → `AsyncVideoEncoder` を `VideoEncoder` にリネーム)。 wrap 削除・型リネーム・サフィックス整理・tests 追従は単一コミットに収める (途中で区切ると使用側の `AsyncVideoEncoder::new` 参照が未解決の cargo 不通コミットが履歴に残る)
4. **`recv_video_encoder_rpc_message_or_pending` の扱い**: free fn 存置。 associated fn 化は本 issue のスコープ外 (polish で判断)
5. **RPC 経路 e2e 検証**: closed/0079 §テスト戦略末尾に残懸念として明記された「RPC 経路の回帰は既存 e2e では検出できない」項目は、 本 issue のスコープ (削除 + rename) では取り扱わない (挙動不変で、 テスト追加は refactor カテゴリの本旨から外れる)。 起票時に確認済み

### shiguredo-rust 規約整合

- モック / スタブ不使用 (テストは実 encoder + tokio channel + 実 pipeline)
- `#[non_exhaustive]` 不使用
- 新規 trait 追加なし

## 完了条件

- `pub struct VideoEncoder` (wrap 型) と `drain_video_encoder_output` が `src/encoder.rs` から削除されている
- `pub struct AsyncVideoEncoder` が `pub struct VideoEncoder` にリネームされ、 メソッドの `_sync` / `_async` サフィックスが削除されている (§決定事項 1)
- `handle_input_sample` / `poll_output` が `pub` に格上げされ、 `handle_rpc_message` は `pub(crate)` を維持している (§決定事項 2)
- grep 1: `rg '\bAsyncVideoEncoder\b' src/ tests/ pbt/ fuzz/ examples/` の hit が 0 件 (コメント / docstring 含む)
- grep 2: `rg 'handle_input_sample_sync|poll_output_sync|handle_rpc_message_sync|next_encoded_frame_async' src/ tests/ pbt/ fuzz/ examples/` の hit が 0 件 (同上)
- grep 3: `rg 'inner_encoder|drain_video_encoder_output' src/ tests/` の hit が 0 件 (wrap 固有アーティファクトの不在)
- closed/0057 §3 分割表 (`issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md`) の line 362 の未起票行を本 issue に対応させる (推定 LOC は実装完了時点の `git diff --stat develop` からコード限定基準で `+X/-Y` 形式に記入)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 依存関係

依存先 closed/0079 (`feature/refactor-migrate-video-encoder-users-to-async`、 2026-07-08 develop merge、 merge commit `0943e9d6`) は完了済み。

着手時の再確認 grep (本番経路に wrap 使用が復活していないことの確認):

```
rg '\bVideoEncoder\b' src/ tests/ pbt/ fuzz/ examples/
```

`\b` 境界により `AsyncVideoEncoder` / `VideoEncoderOptions` / `VideoEncoderInner` / `VideoEncoderRpcMessage` は hit しない (前後いずれかが単語文字に連続し、 境界が成立しないため)。 期待 hit は §現状 (a) (b) 群、 `src/encoder.rs` の wrap 定義本体と `drain_video_encoder_output` のシグネチャ。 これ以外の新規 hit があれば、 その使用側の移行を先に行う。

## 解決方法

実装手順:

1. 着手条件確認: §依存関係の grep を実施し、 hit が期待どおりであることを確認
2. wrap 型 (`VideoEncoder` + `impl VideoEncoder` + `drain_video_encoder_output`) を削除し、 `AsyncVideoEncoder` → `VideoEncoder` にリネーム
3. メソッドのサフィックス整理 (`_sync` / `_async` 削除) と可視性の格上げ (`pub` へ)、 呼出箇所・コメント / docstring の追従
4. 使用側 (`compose.rs:15,577` / `vmaf.rs:15,456` / `list_codecs.rs:7,88` / `encoder.rs:1226`) の `AsyncVideoEncoder::` を `VideoEncoder::` に置換 (rename に追従。 use 文の縮約含む)
5. tests の追従 (`tests/encoder_tests.rs` の use 文縮約、 4 番目テストの `_via_async_pipeline` サフィックス整理、 docstring 追随。 `src/encoder.rs` 内 unit tests の型名・メソッド名・テスト名追随)。 手順 2〜5 は単一コミットに収める (§決定事項 3)
6. closed/0057 §3 分割表 (`:362`) の未起票行を本 issue の 5 セル形式に置換 (提出時点は `open/0083`、 マージ後に `closed/0083` に切替)。 実装 PR に含める
7. §完了条件の grep 検証 (1〜3) がすべて 0 件であることを確認
8. §完了条件のビルド / テストコマンド (cargo fmt / check / clippy / test、 default + `--no-default-features`) を全通過させる

## CHANGES.md について

内部リファクタにつき記載不要。 hisui は bin crate として配布され、 `VideoEncoder` 系は外部公開していない。 外部プロトコル / 出力は不変。

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): decoder 側の親 issue、 派生方針 (δ) を確立
- closed/0067 (`feature/refactor-add-async-video-encoder`): encoder 側の親 issue、 派生方針 (δ) を encoder 側に展開
- closed/0079 (`feature/refactor-migrate-video-encoder-users-to-async`): 依存先。 使用側 4 hit を `AsyncVideoEncoder` に移行 + `AsyncVideoEncoder::run` 追加を完了。 本 issue のテキスト書換対象は 0079 で確立された使用側 4 hit + tests 4 件が全量
- closed/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`): decoder 系列の precedent。 本 issue と同型のクリーンアップ (wrap 削除 + rename + サフィックス整理)
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 採用案 C の親 issue。 本 issue 完了で採用案 C の長所 (v)「ホップ数上限 1」が encoder 側で最終達成される。 §3 分割表 line 362 の未起票行を本 issue に対応させる (完了条件参照)
- open/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`): 独立に着手可能な perf issue。 本 issue の完了は 0080 の前提としない、 逆も同様
- (未起票) encoder 未使用 API 削除 refactor issue: 本 issue 完了後に起票候補。 リネーム完了後に dead code になった public API の削除 + `EncoderOutputReceiver` 可視性整理 (closed/0057 §3 分割表 line 363 参照)
