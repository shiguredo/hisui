# 同期 VideoEncoder wrap を削除して AsyncVideoEncoder を VideoEncoder にリネームする

- Priority: Medium
- Created: 2026-07-08
- Completed: 2026-07-09
- Model: Claude Opus 4.7
- Branch: feature/refactor-remove-sync-video-encoder-and-rename
- Polished: 2026-07-08
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0066 で確立され closed issue 0067 で encoder 側に展開された派生方針 (δ) 「`Async*` 新規追加 + 既存を wrap 化 + 段階移行」は、 closed issue 0079 で全使用側 (compose / vmaf / list_codecs / `create_video_processor(_with_params)`) が `AsyncVideoEncoder` に切り替わった時点で「同期 wrap を削除し、 `AsyncVideoEncoder` を `VideoEncoder` にリネームする」ことで最終形に到達する。 本 issue はその最終ステップを扱う。

closed/0079 の完了 (2026-07-08 develop merge) により、 wrap `VideoEncoder` は本番経路での使用ゼロの実質 dead code になっている。 closed issue 0057 §3 採用案 C の長所 (v) 「callback friendly 定義 (ホップ数上限 1)」は、 wrap の 2 段ホップ (`VideoEncoder::poll_output` → `AsyncVideoEncoder::poll_output_sync`) が型として残る限り最終達成にならない。 本 issue で wrap 型を消し、 `AsyncVideoEncoder` を `VideoEncoder` にリネームして命名を最終化する。

## 優先度根拠

Medium。

- 本 issue 単独では外部挙動は不変。 内部型名の整理のため緊急性は低い
- 依存先 closed/0079 は 2026-07-08 に develop merge 済みで着手条件成立 (§依存関係参照)

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
    // new (:830) / name / codec / get_engines / run (:852) / handle_rpc_message /
    // handle_input_message (:904) / handle_input_sample / poll_output
}

fn drain_video_encoder_output(encoder: &mut VideoEncoder, ...) -> Result<bool>              // :921 (module-private free fn)
async fn recv_video_encoder_rpc_message_or_pending(...) -> Option<VideoEncoderRpcMessage>   // :942 (wrap 側 :885 と AsyncVideoEncoder::run :806 の両方から呼ばれる)
```

wrap の本番使用はゼロであることを確認済み:

- `drain_video_encoder_output` の呼出は wrap 側 `run` 内のみ (0079 で使用側移行完了により本番経路の呼出は消滅)
- wrap 側 `handle_input_message` (`:904`、 private fn) の呼出も wrap 側 `run` 内のみ。 wrap 削除でも元々 module 外に公開されていないため外部影響なし (`AsyncVideoEncoder::run` は `Message` dispatch を自前展開済み)
- `recv_video_encoder_rpc_message_or_pending` (`:942`) は `AsyncVideoEncoder::run` (`:806`) からも呼ばれるため存置する (§決定事項 4)
- `pbt/` / `fuzz/` / `examples/` に `AsyncVideoEncoder` / wrap `VideoEncoder` の参照はない (`rg '\bAsyncVideoEncoder\b|\bVideoEncoder\b' pbt/ fuzz/ examples/` 実測済み。 `examples/obsws_bootstrap` の `VideoEncoderFactory` は libwebrtc 由来の別物)

残存する wrap `VideoEncoder` への参照は次の 2 群に分類される:

**(a) リネーム後にテキスト無変更で新型に解決される箇所 (実作業なし)**

- `tests/encoder_tests.rs` の 3 テスト (`#[test]` 行 `:61` / `:97` / `:157`、 内部で `VideoEncoder::new` (`:65` / `:109` / `:166`) と `handle_input_sample` / `poll_output` を使う): closed/0067 で追加された wrap 側 pub API の integration test。 `new` / `handle_input_sample` / `poll_output` はリネーム後 (新 `VideoEncoder` = 旧 `AsyncVideoEncoder`) にサフィックス整理後の `handle_input_sample` / `poll_output` にテキスト無変更で解決される (§決定事項 2 の可視性 pub 化前提)。 なお同ファイルの use 文 (`:4-13` の `use hisui::{...}` ブロック、 `encoder::{...}` は `:7-10`、 `AsyncVideoEncoder` identifier は `:8`) は §tests への影響 の書換対象 (縮約)

**(b) 書換が必要な箇所 (§tests への影響 / §使用側の追随で扱う)**

- 使用側 4 hit の `AsyncVideoEncoder::` prefix (0079 で置換した compose:577 / vmaf:456 / `create_video_processor_with_params:1226` / `list_codecs:88`) と、 use 文 3 箇所 (`compose.rs:15` / `vmaf.rs:15` / `list_codecs.rs:7`)
- `tests/encoder_tests.rs` の 4 番目テスト `video_encoder_run_processes_i420_via_async_pipeline` (0079 で追加。 詳細な書換対象は §tests への影響 で全量列挙)
- `src/encoder.rs` の `#[cfg(test)] mod tests` 内の `AsyncVideoEncoder` 参照、 `poll_output_sync` / `next_encoded_frame_async` 呼出、 コメント内の型名・メソッド名参照 (詳細は §tests への影響 で全 hit 列挙)

## 設計方針

closed/0073 (decoder 側 precedent) と同型で実施する。

### 削除対象 (`src/encoder.rs`)

- `pub struct VideoEncoder` (wrap 型、 `:825`) と `impl VideoEncoder` (`:829-919`) の全メソッド (`new` / `name` / `codec` / `get_engines` / `run` / `handle_rpc_message` / `handle_input_message` / `handle_input_sample` / `poll_output`)
- `fn drain_video_encoder_output` (`:921`、 module-private)。 wrap `run` 内以外の呼出はなく、 wrap 削除で不要になる

**注意**: `run` は同名メソッドが 2 つ存在する。 削除するのは wrap 側 `:852` (`drain_video_encoder_output` 経由の 2 腕 `tokio::select!`)。 `AsyncVideoEncoder::run` (`:761`、 `poll_output_sync` の inline drain + 2 腕 `tokio::select!`) は存続し、 リネーム後の新 `VideoEncoder::run` になる。

### リネーム対象

- `pub struct AsyncVideoEncoder` → `pub struct VideoEncoder` (`:474`)
- メソッドの `_sync` / `_async` サフィックス削除 (§決定事項 1):
  - `handle_input_sample_sync` → `handle_input_sample`
  - `poll_output_sync` → `poll_output`
  - `handle_rpc_message_sync` → `handle_rpc_message`
  - `next_encoded_frame_async` → `next_encoded_frame`
- 可視性の整理 (§決定事項 2): wrap 消滅で名前空間分離の必要が消えるため、 現状 `pub(crate)` の 3 API のうち integration test から使うものは `pub` に格上げする
  - `handle_input_sample_sync` (現 `pub(crate)`) → `handle_input_sample` (`pub`)
  - `poll_output_sync` (現 `pub(crate)`) → `poll_output` (`pub`)
  - `handle_rpc_message_sync` (現 `pub(crate)`) → `handle_rpc_message` (`pub(crate)` 維持、 内部利用のみ)
- 型名の rename 対象:
  - `src/encoder.rs` 内: `AsyncVideoEncoder` の全参照 (struct 定義 / impl / struct field 型 / fn シグネチャ / call site / docstring / コメント / `.expect` メッセージ)
  - `src/sora/recording_subcommand_compose.rs` / `recording_subcommand_vmaf.rs` / `src/subcommand_list_codecs.rs` の use 文と call site。 use 文はアルファベット順を維持する (rustfmt / 既存規約踏襲)。 compose (`:15`) と list_codecs (`:7`) は `encoder::{AsyncVideoEncoder, AudioEncoder}` (`AsyncVideoEncoder` < `AudioEncoder`) の順で、 rename 後は `encoder::{AudioEncoder, VideoEncoder}` (`AudioEncoder` < `VideoEncoder`) に整列が入れ替わる。 vmaf (`:15`) は `encoder::VideoEncoder` の単独 use のためアルファベット整列対象外
  - `src/encoder.rs:1226` (`create_video_processor_with_params` 内)
  - `tests/encoder_tests.rs` の use 文と call site + docstring
- 単なる型名 / メソッド名 rename では拾えない **semantic な docstring / コメント** も同時に書き換える (行番号昇順で列挙。 具体的な書換文面案は §tests への影響 と以下に明示):
  - `src/encoder.rs:367` (`EncoderOutputSender` 型エイリアス docstring): 「内部エンコーダーが出力フレーム / エラーを `AsyncVideoEncoder` 内の受信側 (`rx`) に流す」の `AsyncVideoEncoder` を新型名 `VideoEncoder` に追従
  - `src/encoder.rs:370` (`EncoderOutputReceiver` 型エイリアス docstring): 同上
  - `src/encoder.rs:374` (`OutputSink` docstring): 同上
  - `src/encoder.rs:379` (`OutputSink` の `unreachable!()` 契約 docstring): 同上
  - `src/encoder.rs:461-472` (`AsyncVideoEncoder` 型 docstring): 「processor 経路 (`run`) からは `handle_input_sample_sync` / `poll_output_sync` / `handle_rpc_message_sync` 等の `_sync` 付き内部 API 経由で同期駆動する。 pull 型で直接利用するときは `next_encoded_frame_async` で非同期に取得する」→ rename 後の実態 (「processor 経路 (`run`) からは `handle_input_sample` / `poll_output` / `handle_rpc_message` 経由で同期駆動する。 pull 型で直接利用するときは `next_encoded_frame` で非同期に取得する」) に更新。 `_sync` 付き / 非同期版の対比構造は wrap 消滅で不要になる
  - `src/encoder.rs:678-683` (`handle_rpc_message_sync` docstring): 「processor 経路 (`run`) の RPC 腕から呼び出される同期 RPC ハンドラ」の後段「実際の keyframe 要求適用は次の `handle_input_sample_sync` 呼び出し時に inner へ伝播する」の `_sync` 付きメソッド名を rename に追従
  - `src/encoder.rs:697` (`/// wrap から呼ぶ同期入力 API`): 「processor 経路 (`run`) から呼ぶ同期入力 API」に更新 (wrap 消滅で虚偽になる)
  - `src/encoder.rs:726` (`/// wrap から呼ぶ同期 poll`): 「processor 経路 (`run`) から呼ぶ同期 poll」に更新 (同上)
  - `src/encoder.rs:757-760` (`AsyncVideoEncoder::run` docstring): 「processor モデル (`ProcessorHandle` + subscribe / publish) 用の駆動 API」の後段「入力トラックを subscribe し、 `_sync` API の drain ループでエンコード結果を出力トラックへ流す」の `_sync` API 表現を rename に追従 (`_sync` サフィックス消滅を反映)
  - `src/encoder.rs:820-823` (wrap `VideoEncoder` 型 docstring 「同期 API を保つ VideoEncoder は `AsyncVideoEncoder` の wrap として動作する」): wrap 削除で全て消滅

### tests への影響

- `tests/encoder_tests.rs`:
  - use 文 (`:4-13` の `use hisui::{...}` ブロック、 `encoder::{...}` は `:7-10`): `encoder::{AsyncVideoEncoder, EncoderRunOutput, VideoEncoder, VideoEncoderOptions, default_video_encode_config_for_rpc}` を `encoder::{EncoderRunOutput, VideoEncoder, VideoEncoderOptions, default_video_encode_config_for_rpc}` に縮約 (`AsyncVideoEncoder` の import 削除、 `VideoEncoder` は残す)
  - 4 番目テスト `video_encoder_run_processes_i420_via_async_pipeline` (`#[test]` `:204`、 fn `:205`):
    - テスト名の `_via_async_pipeline` サフィックスを `_via_pipeline` に整理し、 内部 `AsyncVideoEncoder::new` (`:265`) を `VideoEncoder::new` に置換 (`AsyncVideoEncoder::` の `::` 参照がなくなるため wrap `VideoEncoder` との名前空間衝突なし)
    - 4 番目テストが呼ぶ helper fn `encode_video_frames_with_async_pipeline` (定義 `:229`、 呼出 `:209`) を `encode_video_frames_with_pipeline` にリネーム。 decoder 対称先例 `tests/decoder_tests.rs::decode_video_frames_with_pipeline` (0073 で一本化済み) と同名になる。 テスト名だけ整理して helper 名に `_async_` を残すと「同期版が存在しないのに `_async_` サフィックス」が残る負債になる
    - docstring (`:198-203`) 内の 2 箇所の `AsyncVideoEncoder::run` (`:198` / `:200`) と「async pipeline」表現 (`:200`) を新型名・新表現 (`VideoEncoder::run` / 「pipeline」) に書換
  - 手順は順不同 (rustc はコンパイル通せる中間状態を規定しないため、 use 縮約・型名置換・fn 名 rename・docstring 更新のいずれから始めても最終的に全部揃えば成立)
- `src/encoder.rs` の `#[cfg(test)] mod tests`:
  - コメント内 `AsyncVideoEncoder` 参照全 hit (`rg '\bAsyncVideoEncoder\b' src/encoder.rs` 実測の mod tests 内 hit 8 箇所のうち、 コメント hit は 4 箇所。 他は :1360 label / :1368 fn 戻り値型 / :1381 call site / :1382 `.expect` message で以下の該当項目で扱う): `:1352` (「rx が sink より先に drop されるのは AsyncVideoEncoder の drop 順制御下では」) / `:1363` (「sink と rx が AsyncVideoEncoder 内で同居する構造上」) / `:1444` (「AsyncVideoEncoder 内で field 所有される」) / `:1449` (「AsyncVideoEncoder が保持する sink 経由で emit_ok したフレームが」)。 いずれも新型名 `VideoEncoder` に書換
  - コメント / label 内メソッド名参照全 hit (`rg 'poll_output_sync|next_encoded_frame_async' src/encoder.rs` の mod tests 内コメント / label hit): `:1244` (「OutputSink / poll_output_sync の契約テストでは keyframe フラグ以外の値」) / `:1360` (`---- R-3: AsyncVideoEncoder::poll_output_sync 分岐テスト ----` label) / `:1362-1363` (label 説明) / `:1439` (`---- I-12: next_encoded_frame_async の pub 契約テスト ----` label) / `:1440-1441` (label 説明) / `:1450` (「`next_encoded_frame_async` の await で受信できる」テスト内コメント) / `:1466` (テスト内コメント)。 いずれも新名 (`poll_output` / `next_encoded_frame`) に追従
  - `.expect` / `assert!` メッセージ内文字列全 hit (`rg '"AsyncVideoEncoder|"next_encoded_frame_async|poll_output_sync' src/encoder.rs` の mod tests 内 string literal hit): `:1382` (`"AsyncVideoEncoder::new が失敗した"`) / `:1392` (`.expect("poll_output_sync が失敗した")`) / `:1407` (`"sink.emit_err の Err が poll_output_sync で伝播されていない"` assert! メッセージ) / `:1418` (`.expect("poll_output_sync が失敗した")`) / `:1432` (`.expect("poll_output_sync が失敗した")`) / `:1456-1457` / `:1472` / `:1475` の `next_encoded_frame_async` 文字列。 いずれも新名に追従
  - `new_uninitialized_encoder` fn (`:1368`) の戻り値型 `AsyncVideoEncoder` → `VideoEncoder`、 内部 `AsyncVideoEncoder::new` (`:1381`) → `VideoEncoder::new`
  - `poll_output_sync` / `next_encoded_frame_async` 呼出を `poll_output` / `next_encoded_frame` に追随
  - テスト名のサフィックス整理 (`_sync` / `_async` 削除): `poll_output_sync_returns_*` (3 テスト: `_processed_when_frame_available` / `_pending_when_empty_and_not_eos` / `_finished_when_empty_and_eos`) と `poll_output_sync_propagates_error_from_rx` (`:1400`) と `next_encoded_frame_async_returns_frame_after_emit_ok` / `next_encoded_frame_async_propagates_error_from_emit_err` の全てから `_sync` / `_async` を削除。 テスト名衝突の実測確認: `tests/encoder_tests.rs::video_encoder_poll_output_returns_processed` (integration test binary) と `src/encoder.rs::tests::poll_output_returns_processed_when_frame_available` (crate 内 module test) は module path が異なりコンパイル単位も別のため衝突しない

### 決定事項 (実装で覆さない)

1. **メソッド命名**: `_sync` / `_async` サフィックスは全削除する。 wrap 削除で `handle_input_sample` / `poll_output` / `handle_rpc_message` の名前は空き、 `next_encoded_frame` にも衝突はない。 `next_encoded_frame` の非同期性は `async fn` シグネチャから自明で、 同一型内に同期版が存在しなくなるため区別サフィックスは不要 (closed/0073 と同判断)
2. **可視性**: integration test (crate 外) から呼ぶ `handle_input_sample` / `poll_output` は `pub` に格上げする。 `handle_rpc_message` は内部利用のみで `pub(crate)` 維持
3. **リネーム実施順序**: option A (wrap 削除 → `AsyncVideoEncoder` を `VideoEncoder` にリネーム)。 wrap 削除・型リネーム・サフィックス整理・tests 追従は単一コミットに収める (途中で区切ると使用側の `AsyncVideoEncoder::new` 参照が未解決の cargo 不通コミットが履歴に残る)
4. **`recv_video_encoder_rpc_message_or_pending` の扱い**: `src/encoder.rs:942` の free fn を存置する (触らない)。 rename 後の呼出元は `VideoEncoder::run` (旧 `AsyncVideoEncoder::run`) の 1 箇所のみで、 associated fn 化しても短縮効果が薄い + 関数シグネチャが `Option<&mut UnboundedReceiver<...>>` を引数に取る形で self に持たせられない (rpc_rx_enabled フラグを外部で保持する構造) ため、 free fn のまま保持する方が構造的に自然。 associated fn 化は本 issue のスコープ外 (将来の未使用 API 削除 refactor issue で再判断可)
5. **RPC 経路 e2e 検証**: closed/0079 §テスト戦略末尾に「残懸念: RPC 経路の回帰は既存 e2e では検出できない」が明記されている。 本 issue のスコープは wrap 削除 + rename で挙動不変。 テスト追加は refactor カテゴリの本旨から外れるため取り扱わない。 起票者 (@sile) は本項を確認したうえで「本 issue には組み込まない」判断を確定させた。 独立 test 追加 issue の起票有無は本 issue の範疇外 (0079 残懸念のフォローアップとして別途 Decision Owner が判断)

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
- grep 3: `rg 'inner_encoder|drain_video_encoder_output|_via_async_pipeline|_with_async_pipeline' src/ tests/` の hit が 0 件 (wrap 固有アーティファクトの不在 = closed issue 0057 §3 採用案 C の長所 (v) 「ホップ数上限 1」達成の機械検証)
- closed/0057 §3 分割表 (`issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md`) の 3 行 (line 351 / 361 / 362) を本 issue の実装 PR に含める。 詳細は §解決方法 6 を参照
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

`\b` 境界により `AsyncVideoEncoder` / `VideoEncoderOptions` / `VideoEncoderInner` / `VideoEncoderRpcMessage` は hit しない (前後いずれかが単語文字に連続し、 境界が成立しないため)。 期待 hit は §現状 (a) (b) 群、 `src/encoder.rs` の wrap 定義本体 (`:825-919`) と `drain_video_encoder_output` の引数型 `VideoEncoder` (`:922`)、 wrap docstring 内の `VideoEncoder` 参照 (`:820-823`)。 これ以外の新規 hit があれば、 その使用側の移行を先に行う。

## 解決方法

実装手順:

1. 着手条件確認: §依存関係の grep を実施し、 hit が期待どおりであることを確認
2. wrap 型 (`VideoEncoder` + `impl VideoEncoder` + `drain_video_encoder_output`) を削除し、 `AsyncVideoEncoder` → `VideoEncoder` にリネーム
3. メソッドのサフィックス整理 (`_sync` / `_async` 削除) と可視性の格上げ (`pub` へ)、 呼出箇所・コメント / docstring の追従 (§リネーム対象 の semantic 表現書換リストを網羅)
4. 使用側 (`compose.rs:15,577` / `vmaf.rs:15,456` / `list_codecs.rs:7,88` / `encoder.rs:1226`) の `AsyncVideoEncoder::` を `VideoEncoder::` に置換 (rename に追従。 use 文はアルファベット順で integrity を保つ。 詳細は §リネーム対象 参照)
5. tests の追従 (`tests/encoder_tests.rs` の use 文縮約、 4 番目テストの `_via_async_pipeline` サフィックス整理、 helper fn `encode_video_frames_with_async_pipeline` → `encode_video_frames_with_pipeline` の rename、 docstring 追随。 `src/encoder.rs` 内 unit tests の型名・メソッド名・テスト名追随)。 手順 2〜5 は単一コミットに収める (§決定事項 3)
6. closed/0057 §3 分割表 (`issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md`) の 3 行更新を本 issue の実装 PR に含める (手順 2〜5 とは別コミットで、 3 行更新は同一コミットにまとめる):
    - **line 351 の依存順序記述** の更新: 現状 `encoder 系列: **0066 → 0067 → {open/0079 / encoder wrap 削除 rename / encoder 未使用 API 削除}**` の `open/0079` を `closed/0079` に、 `encoder wrap 削除 rename` を `closed/0083` に置換 (0079 は 2026-07-08 に closed 移動済み、 本 issue も PR merge で closed 化する前提の実装 PR 内表記)
    - **line 361 の 0079 行の表記更新**: 現状 `| open/0079 (...) | ... | +75/-13 | 0067 | 内部 API のみ |` を `| closed/0079 (...) | ... | +75/-13 | 0067 | 内部 API のみ |` に置換 (open → closed の broken windows 修正)
    - **line 362 の未起票行を本 issue の 5 セル形式に置換**: closed/0079 (line 361) 対称の 5 セル形式に置換 (推定 LOC は実装完了時点の `git diff --stat develop` からコード限定基準で `+X/-Y` 形式に記入。 提出時点で `closed/0083` として書く。 依存先セルも他行に合わせて数字表記 `0079` に統一):
    ```
    | closed/0083 (`feature/refactor-remove-sync-video-encoder-and-rename`) | 同期 wrap `VideoEncoder` 削除 + `AsyncVideoEncoder` を `VideoEncoder` にリネーム + `_sync` / `_async` サフィックス整理 | <+X/-Y> | 0079 | 内部 API のみ |
    ```
    - line 363 (未起票 encoder 未使用 API 削除 refactor issue 行) は本 issue のスコープ外として現状維持 (依存先セルの `encoder wrap 削除 + rename issue` は将来 encoder 未使用 API 削除 issue が起票された際にその PR で `closed/0083` に置換する)
    - line 364 (0080 行) は 0079 の実装 PR で更新済み (現状維持)
7. §完了条件の grep 検証 (1〜3) がすべて 0 件であることを確認
8. §完了条件のビルド / テストコマンド (cargo fmt / check / clippy / test、 default + `--no-default-features`) を全通過させる

## CHANGES.md について

内部リファクタにつき記載不要。 hisui は bin crate として配布され、 `VideoEncoder` 系は外部公開していない。 外部プロトコル / 出力は不変。

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): decoder 側の親 issue、 派生方針 (δ) を確立
- closed/0067 (`feature/refactor-add-async-video-encoder`): encoder 側の親 issue、 派生方針 (δ) を encoder 側に展開
- closed/0079 (`feature/refactor-migrate-video-encoder-users-to-async`): 依存先。 使用側 4 hit を `AsyncVideoEncoder` に移行 + `AsyncVideoEncoder::run` 追加を完了 (2026-07-08 develop merge、 実績 +75/-13)
- closed/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`): decoder 系列の precedent。 本 issue と同型のクリーンアップ (wrap 削除 + rename + サフィックス整理)
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 採用案 C の親 issue。 本 issue 完了で採用案 C の長所 (v)「ホップ数上限 1」が encoder 側で最終達成される。 §3 分割表 line 362 の未起票行を本 issue に対応させる (完了条件 / §解決方法 6 参照)
- open/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`): 独立に着手可能な perf issue。 本 issue の完了は 0080 の前提としない、 逆も同様
- (未起票) encoder 未使用 API 削除 refactor issue: 本 issue 完了後に起票候補。 リネーム完了後に dead code になった public API の削除 + `EncoderOutputReceiver` 可視性整理 (closed/0057 §3 分割表 line 363 参照)
- (未起票) 起票候補: closed/0079 §テスト戦略末尾の RPC 経路 e2e 未検出残懸念のフォローアップ (起票有無は 0079 の残懸念として別途 Decision Owner が判断。 §決定事項 5 参照)
