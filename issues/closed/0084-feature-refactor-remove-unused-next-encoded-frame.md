# 未使用の next_encoded_frame を削除して EncoderOutputSender を pub(crate) 化する

- Priority: Low
- Created: 2026-07-09
- Completed: 2026-07-10
- Model: Opus 4.7
- Branch: feature/refactor-remove-unused-next-encoded-frame
- Polished: 2026-07-09
- Reporter: @sile
- Decision Owner: @sile

## 目的

`VideoEncoder::next_encoded_frame` (`pub async fn next_encoded_frame(&mut self) -> Option<crate::Result<VideoFrame>>`、 実体は `self.rx.recv().await` だけ) は本番コードから 1 箇所も呼ばれていない未使用 public API である。 closed issue 0067 で「Encoder の pull 型直接利用への将来拡張余地」として追加され、 closed issue 0083 (wrap 削除 + rename) 完了時点でも本番 caller は 0 件のまま (0083 は rename に伴う追随のみで、 存廃判断は下していない = 暗黙に保持継続)。 本 issue はこの将来拡張余地を放棄して API を削除し、 `VideoEncoder` の出力取得モデルを実際に使われている同期 poll 経路一本に収束させる。

同時に `EncoderOutputSender` type alias の可視性 (`pub type`) を `pub(crate) type` に引き下げ、 encoder 内の Sender (pub) と Receiver (pub(crate)) の可視性非対称を解消する。 encoder 側 `EncoderOutputSender` は crate 外の公開シグネチャに露出する経路が存在しない (根拠詳細は §現状)。 併せて `EncoderOutputSender` の docstring を decoder 側 `DecoderOutputSender` docstring と対称に増強し、 「なぜ decoder 側 Sender は pub 維持で encoder 側 Sender は pub(crate) に引き下げるか」の非対称理由を明示する。

将来拡張余地を放棄する根拠 (closed/0078 と同判断): (1) 本番経路 (compose / vmaf / list_codecs / `create_video_processor(_with_params)`) はすべて同期の `handle_input_sample` + `poll_output` の drain ループで EOS を扱い、 非同期 EOS シグナルを要求する使用側は現時点で見えない (2) 仮に将来必要が生じても、 Sender の `drop` シグナル (現在の `Option<crate::Result<VideoFrame>>` の `None`) より `enum { Ok, EndOfStream, Err }` のような明示型を新たに導入する方が意図が明瞭で、 現状の `Option` 保持は必然性が薄い。

## 優先度根拠

Low。

- 本番挙動は不変 (未使用 API の削除 + 可視性引き下げ + docstring 増強)
- 実装コストは軽微 (純削除 2 項目 + 可視性引き下げ 1 項目 + docstring 増強 1 項目 = 計 4 項目)
- closed/0067 の追加判断を覆すため、 PR に対する Decision Owner 承認プロセスが必要 (詳細は §解決方法 §1)

## 現状

closed issue 0083 (同期 wrap `VideoEncoder` 削除 + `AsyncVideoEncoder` → `VideoEncoder` リネーム + `_sync` / `_async` サフィックス整理、 2026-07-09 マージ済み、 merge commit `66663c37`) 時点の `src/encoder.rs` を基準とする (総行数 1360)。

- `VideoEncoder::next_encoded_frame` の本番呼出はゼロ。 本番の映像エンコード出力取得は、 processor 経路 (`VideoEncoder::run`、 compose / vmaf / `create_video_processor_with_params` 内の spawn クロージャから呼び出される) がすべて同期の `handle_input_sample` + `poll_output` (内部 `try_recv`) を経由する
- 唯一の呼出元は「そのメソッド自身を試すためだけのテスト」2 件 (`src/encoder.rs:1330` の `next_encoded_frame_returns_frame_after_emit_ok` / `:1347` の `next_encoded_frame_propagates_error_from_emit_err`)
- struct docstring (`src/encoder.rs:462-472`) は closed/0083 のリネーム時に「`handle_input_sample` / `poll_output` で同期駆動する映像エンコーダー」に更新済みで、 `next_encoded_frame` への対比参照は既に存在しない
- `OutputSink` docstring (`src/encoder.rs:374-381`) も同じく 0083 で更新済みで、 `next_encoded_frame` への言及は既に存在しない
- `EncoderOutputSender` は `pub type` (`src/encoder.rs:368`) だが、 crate 外で使う経路がゼロ:
  - `impl OutputSink` に `pub fn new` が存在しない (実装は `pub fn emit_ok` / `pub fn emit_err` のみ)
  - `OutputSink` struct field は全て非 pub のため、 crate 外からの struct literal 構築も不可能
  - `tests/encoder_tests.rs` および `tests/e2e.rs` に `hisui::encoder::OutputSink` の touching なし (decoder 側 `hisui::decoder::OutputSink::new` は `tests/e2e.rs:1406, :1630, :1764, :1875` の 4 箇所で呼ぶが、 encoder 側は 0 件)
  - crate 内 struct literal 構築は `src/encoder/test_helpers.rs:54` (test 用 helper) と `src/encoder.rs:519` (`VideoEncoder::new` 内) の 2 箇所のみ、 いずれも crate 内で pub(crate) で成立する
  - `rg 'EncoderOutputSender' src/ tests/` の hit は `src/encoder.rs` の定義行 `:368` と `OutputSink` の field 型 `:384` の 2 箇所のみ
- `EncoderOutputReceiver` は `pub(crate) type` (`src/encoder.rs:371-372`、 折返し 2 行) と既に非 pub。 本 issue で追加変更は不要
- decoder 側 `DecoderOutputSender` (`src/decoder.rs:329-332`) は `OutputSink::new` の公開シグネチャに引数型として露出するため `pub` 維持 (closed/0078 の判断)。 encoder 側は上記のとおり露出経路がないため非対称に `pub(crate)` へ引き下げできる

### closed/0057 §3 分割表 line 363 予告との対象反転

closed/0057 §3 分割表 line 363 は「`EncoderOutputReceiver` 可視性整理」を予告するが、 Receiver は closed/0067 の実装完了時点で既に `pub(crate) type` 化済みで整理不要。 予告時に想定されていなかった `EncoderOutputSender` 側が `pub type` のまま残り、 上記のとおり pub 保持根拠がない状態が pre-existing になっている。 本 issue は Receiver ではなく Sender 側 (予告未想定) を扱う (§解決方法 §5 で 0057 §3 line 363 差替時にこの意味論変化を反映する)。

## 設計方針

closed/0078 (decoder 側 precedent) と同型で実施する。

### 削除・書き替え対象

行番号は着手時に `rg 'next_encoded_frame|EncoderOutputSender' src/ tests/ pbt/ fuzz/ examples/` で再特定する (以下は 2026-07-09 時点の実測位置)。 純削除 §1 は削除範囲に末尾空行 1 行を含める (`cargo fmt --check` の空行連続検出を回避)。 §2 (テスト 2 件) は削除範囲直後が `mod tests` の閉じ `}` (`:1360`) で末尾空行は元々存在しないため、 末尾空行の追加も削除もしない (§1 と適用が異なる)。

1. **[純削除] `next_encoded_frame` メソッド定義** — `src/encoder.rs:754-762` (docstring 5 行 `:754-758` + signature `:759` + body `:760` + 閉じ `}` `:761` + 末尾空行 `:762` の計 9 行)
2. **[純削除] `next_encoded_frame` 専用テスト 2 件と label コメントブロック** — `src/encoder.rs:1322-1359` (直前 `:1321` は兄弟テスト `poll_output_returns_finished_when_empty_and_eos` の閉じ `}` で保存対象、 直後 `:1360` は `mod tests` の閉じ `}` で保存対象)。 内訳: label コメントブロック `:1323-1327` + テスト 1 (`next_encoded_frame_returns_frame_after_emit_ok`) `:1329-1344` + テスト 2 (`next_encoded_frame_propagates_error_from_emit_err`) `:1346-1359` (`:1322 / :1328 / :1345` の 3 箇所の空行を含む一括削除)
3. **[可視性引き下げ] `EncoderOutputSender` の `pub type` → `pub(crate) type`** — `src/encoder.rs:368` の `pub type EncoderOutputSender = ...` の 1 行を `pub(crate) type EncoderOutputSender = ...` に変更 (docstring 増強は §4 で扱う)
4. **[書き替え] `EncoderOutputSender` の docstring 増強** — `src/encoder.rs:367` の 1 行 docstring を、 decoder 側 `DecoderOutputSender` docstring (`src/decoder.rs:329-332` の 4 行構成: 見出し 1 行 + 空 `///` 1 行 + rationale 2 行) と対称に 4 行に増強する。 対比軸は「同 struct 内の Sender vs Receiver」ではなく「encoder Sender vs decoder Sender」に置く (encoder 側で同 struct 内 Sender vs Receiver の対比を書くと decoder 系列との非対称理由が説明できないため。 decoder 側 docstring の対比軸を意図的に変更する)。 想定文案:

    ```rust
    /// 内部エンコーダーが出力フレーム / エラーを `VideoEncoder` 内の受信側 (`rx`) に流すための送信側の型エイリアス
    ///
    /// crate 外から `OutputSink` インスタンスを構築する経路がないため `pub(crate)`。
    /// (対する decoder 側 `DecoderOutputSender` は `OutputSink::new` の公開シグネチャに引数型として露出するため `pub` 維持)。
    ```

    §3 (可視性引き下げ) と §4 (docstring 増強) は単一 atomic edit で実施し、 現状 `:367-368` の 2 行を新 5 行 (docstring 4 行 + `pub(crate) type` 定義 1 行) に置換する (置換後 `pub(crate) type` 定義行は `:371` に移動)。

### 維持対象 (削除に伴い触らない)

- **`src/encoder.rs:482-488` の drop 順制御コメント** と **`:489-490` のフィールド宣言 `inner: Option<VideoEncoderInner>,` / `rx: EncoderOutputReceiver,`** — Nvcodec の worker drop 中に callback が `sink.emit_ok` → `tx.send` した際に `rx` を alive に保つ契約
- **`src/encoder.rs:462-472` の struct docstring** — closed/0083 のリネーム時に「`handle_input_sample` / `poll_output` で同期駆動する映像エンコーダー」形に更新済みで、 `next_encoded_frame` への対比参照は既に存在しない。 本 issue で追加変更不要 (残骸検出は §完了条件 grep 参照)
- **`src/encoder.rs:374-381` の `OutputSink` docstring** — closed/0083 のリネーム時に「`poll_output` の `Disconnected` 分岐も `unreachable!()` で潰す」の言及を含む形に更新済みで、 `next_encoded_frame` への対比参照は存在しない
- **`EncoderOutputReceiver` の `pub(crate) type`** (`src/encoder.rs:371-372`) — 既に `pub(crate)` で外部露出なし。 追加変更不要

### 本 issue のスコープ外

- **`OutputSink` 型自体の `pub struct` → `pub(crate) struct` 引き下げ** および **`impl OutputSink` の `pub fn emit_ok` / `pub fn emit_err` の `pub(crate) fn` 引き下げ** — crate 外の型露出ゼロ + method 呼出ゼロなので技術的には可能だが、 本 issue のスコープは「未使用 API (`next_encoded_frame`) 削除 + `EncoderOutputSender` の pub 非対称解消」に限定。 本 issue 完了後に別 refactor issue で扱う (未起票)

### 削除による失われるカバレッジと代替担保

削除対象テスト 2 件は `VideoEncoder` 内部 `sink` を直接叩いて `rx.recv().await` で受け取る契約を検証する。 兄弟テスト `poll_output_returns_processed_when_frame_available` (`src/encoder.rs:1276`) / `poll_output_propagates_error_from_rx` (`src/encoder.rs:1288`) が `sink.emit_ok` / `emit_err` → `poll_output` の同期経路を等価にカバーしており、 削除で失われるのは async 経路 (`rx.recv().await`) の直接検証のみ。 `rx.recv().await` は tokio の unbounded channel の標準 API で hisui 側実装なしの薄い透過呼出のため、 hisui 側の regression 検出価値は薄い (closed/0078 と同判断)。

### shiguredo-rust 規約整合

- モック / スタブ不使用
- 新規 trait 追加なし
- `#[non_exhaustive]` 不使用 (enum バリアント変更なし)

## 完了条件

- §削除・書き替え対象 4 項目がすべて反映されている
- `EncoderOutputSender` の可視性が `pub(crate)` に変更され、 docstring が decoder 側 `DecoderOutputSender` docstring と対称の 4 行に増強されている
- 変更ファイルは `src/encoder.rs` のみ (`git diff --name-only develop...HEAD` の hit が `src/encoder.rs` のみ、 obsws / decoder / sora / subcommand / tests は無変更)
- grep 検証:
  - `rg 'next_encoded_frame' src/ tests/ pbt/ fuzz/ examples/` の hit が 0 件
  - `rg 'pub type EncoderOutputSender' src/` の hit が 0 件 (旧 `pub type` の残骸検出。 新可視性 `pub(crate) type` の適用は `cargo check` の重複定義エラーで両方担保、 closed/0078 と対称アプローチで 1 件検証に絞る (0078 は「新の 1 件検証」で担保、 本 issue は「旧の 0 件検証」で担保、 論理的には同じ 1 件検証))
  - `rg 'エンコード済みフレームを非同期に取得' src/encoder.rs` の hit が 0 件 (削除対象メソッド docstring 残骸検出)
- closed/0057 §3 分割表 (`issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md`) の 2 行 (line 351 / 363) を本 issue の実装 PR に含める (詳細は §解決方法 §5)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 依存関係

依存先 closed/0083 (`feature/refactor-remove-sync-video-encoder-and-rename`、 2026-07-09 develop merge、 merge commit `66663c37`) は完了済み。

着手時の再確認 grep:

```
rg 'next_encoded_frame|EncoderOutputSender' src/ tests/ pbt/ fuzz/ examples/
```

期待 hit の内訳 (2026-07-09 時点、 計 15 件):

| 分類 | 件数 | 位置 |
|---|---|---|
| `EncoderOutputSender` type alias 定義 | 1 | `:368` |
| `OutputSink` field 型 `EncoderOutputSender` | 1 | `:384` |
| `next_encoded_frame` メソッド定義 | 1 | `:759` |
| label コメント (`next_encoded_frame` 参照) | 2 | `:1323, :1324` |
| テスト 1 (`next_encoded_frame_returns_frame_after_emit_ok`) 内言及 | 5 | `:1330, :1332, :1336, :1338, :1339` |
| テスト 2 (`next_encoded_frame_propagates_error_from_emit_err`) 内言及 | 5 | `:1347, :1348, :1352, :1354, :1357` |
| 計 | 15 | |

これ以外の新規 hit があれば、 その使用側の処理を先に扱うか本 issue のスコープを拡張するかを Decision Owner が判断する。

## 解決方法

### 1. 承認と分岐

削除実装は通常の refactor issue フローに従う。 承認プロセスは closed/0078 と同型:

- 削除 branch (`feature/refactor-remove-unused-next-encoded-frame`) を切って §2〜§5 の実装を進め、 PR を開設する。 PR タイトルは commit タイトルと同一、 PR 本文は §目的の要旨 + §完了条件のチェックリストを載せる
- Decision Owner (@sile) の PR review LGTM が承認確定
- 承認見送りの場合: `develop` ブランチで以下を **別々のコミット** に分けて実施する (shiguredo-git §「issue ファイル単体のコミット」規約に従う。 追記先が `closed/0067` である根拠は「0067 が `next_encoded_frame` の追加判断を下した親 issue のため。 0083 は rename のみで存廃判断を下していない」):
  1. `0084 closed/0067 §関連節に 0084 の削除見送り判断を追記する` (コード変更コミット形式 `{SEQ} {変更内容}`。 `issues/closed/0067-feature-refactor-add-async-video-encoder.md` §関連節末尾に次の 1 行を追加): `- closed/0084 (`feature/refactor-remove-unused-next-encoded-frame`): `next_encoded_frame` の削除を検討したが保持判断維持`
  2. `0084 closed 未使用の next_encoded_frame を削除して EncoderOutputSender を pub(crate) 化する` (状態遷移コミット形式 `{SEQ} {STATE} {TITLE}`。 §6 の成功時 closing と commit タイトル形式は共通、 判別は commit body 1 行目に「削除見送りで close (詳細は本 issue の `## 見送り記録` セクションおよび参照 PR)」を記す形で担保する)。 このコミットで以下を単一 atomic edit にまとめる:
     - `Completed:` を見送り決定日に更新
     - 本 issue 末尾に `## 見送り記録` セクション (見送り決定日 + 見送り理由 + 参照 PR 番号) を追加
     - `git mv issues/0084-....md issues/closed/`

### 2. 実行番号を再特定

`rg 'next_encoded_frame|EncoderOutputSender' src/ tests/ pbt/ fuzz/ examples/` で削除・可視性引き下げ・docstring 増強対象の実行番号を再特定する (§設計方針の再掲。 着手時に必ず実施)。

### 3. 削除・可視性引き下げ・docstring 増強を単一コミットで完成させる

§設計方針 §削除・書き替え対象の 4 項目を単一コミット `0084 未使用の next_encoded_frame を削除して EncoderOutputSender を pub(crate) 化する` (コード変更コミット形式 `{SEQ} {変更内容}`。 issue タイトル原文を流用) に収める。 可視性引き下げと docstring 増強は削除と論理的に一体 (`next_encoded_frame` 削除で `EncoderOutputSender` の crate 外用途が完全に消え、 encoder 側の非対称な `pub` を維持する根拠がなくなるため、 削除に付随して整えることで削除後の型面と可視性と docstring が一貫する)。

### 4. cargo 検証

§完了条件の cargo コマンドを default + `--no-default-features` の両方で PR 開設前の local で通す。

### 5. closed/0057 §3 分割表の更新

本 issue の実装 PR に closed/0057 §3 分割表更新を含める (別コミット `0084 closed/0057 §3 分割表を 0084 完了に合わせて更新する` で 2 行更新を単一コミットにまとめる):

- **line 351 の依存順序記述** の更新: 現状 `encoder 系列: 0066 → 0067 → closed/0079 → closed/0083 → 未起票 encoder 未使用 API 削除` の `未起票 encoder 未使用 API 削除` を `closed/0084` に置換
- **line 363 の未起票行を本 issue の 5 セル形式に置換**: 推定 LOC は実装完了時点の `git diff --stat develop` からコード限定基準で `+X/-Y` 形式に記入。 提出時点で `closed/0084` として書く。 依存先セルは他行に合わせて数字表記 `0083` に統一。 現状の予告文言 (`EncoderOutputReceiver` 可視性整理) は §現状 の反転説明に従い、 本 issue の実スコープ (`EncoderOutputSender` の pub → pub(crate) 引き下げ) に修正する。 置換文言 (0057 分割表の table 行内にそのまま貼り付け):

    ```
    | closed/0084 (`feature/refactor-remove-unused-next-encoded-frame`) | 未使用の `VideoEncoder::next_encoded_frame` 削除 + `EncoderOutputSender` の pub → pub(crate) 引き下げ | <+X/-Y> | 0083 | 内部 API のみ |
    ```

### 6. マージ後の closing

PR merge 完了直後に Reporter (@sile) が `develop` ブランチで単一コミット `0084 closed 未使用の next_encoded_frame を削除して EncoderOutputSender を pub(crate) 化する` (`Completed:` を PR merge 日に更新 + `git mv issues/0084-....md issues/closed/`) で closing する。

## CHANGES.md について

内部リファクタにつき記載不要。 hisui は bin crate として配布され、 `VideoEncoder` 系は外部公開していない。 `cargo doc` 生成物から `EncoderOutputSender` 項が消える副次影響はあるが、 hisui は crates.io 未 publish のため docs.rs 影響なし。

## 関連

- closed/0067 (`feature/refactor-add-async-video-encoder`、 2026-07-08 merge): 本 API を追加した親 issue。 本 issue はその追加判断のうち、 将来拡張余地としての `next_encoded_frame` 保持部分を撤回する
- closed/0078 (`feature/refactor-remove-unused-next-decoded-frame`、 2026-07-07 merge): decoder 系列の対称 precedent。 本 issue と同型のクリーンアップ (未使用 pull API 削除 + 型エイリアス可視性整理)。 encoder 側は `Sender` 側の pub 非対称も追加で解消する差分あり
- closed/0079 (`feature/refactor-migrate-video-encoder-users-to-async`、 2026-07-08 merge): encoder 系列の移行系列 issue。 使用側 4 hit を `AsyncVideoEncoder` に移行 + `AsyncVideoEncoder::run` 追加を完了。 本 API は触られず保持継続された。 encoder 系列の系譜は `0067 → 0079 → 0083 → 0084` で decoder 系列の `0066 → {0068 / 0071 / 0072} → 0073 → 0078` と対称
- closed/0083 (`feature/refactor-remove-sync-video-encoder-and-rename`、 2026-07-09 merge): 依存先。 wrap 削除 + rename + サフィックス整理を完了。 本 issue の発端 (0083 の review-diff-code で削除候補として検出、 かつ closed/0057 §3 分割表 line 363 の予告に該当)
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 採用案 C の親 issue。 §3 分割表 line 363 の未起票行を本 issue に対応させる (§解決方法 §5 参照)
