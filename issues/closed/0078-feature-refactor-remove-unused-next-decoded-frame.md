# 未使用の VideoDecoder::next_decoded_frame を削除する

- Priority: Low
- Created: 2026-07-06
- Completed: 2026-07-07
- Model: Opus 4.8
- Branch: feature/refactor-remove-unused-next-decoded-frame
- Polished: 2026-07-07
- Reporter: @sile
- Decision Owner: @sile

## 目的

`VideoDecoder::next_decoded_frame` (`pub async fn next_decoded_frame(&mut self) -> Option<crate::Result<VideoFrame>>`、 実体は `self.output_rx.recv().await` だけ) は本番コードから 1 箇所も呼ばれていない未使用 public API である。 closed issue 0066 で「将来 EOS を非同期経路で通知する形が必要になった際に `None` を EOS シグナルとして活用できるよう `Option` を維持している」という将来拡張余地として追加された。 本 issue はこの将来拡張余地を放棄して API を削除し、 `VideoDecoder` の出力取得モデルを実際に使われている同期 poll 経路一本に収束させる。 削除に連動して、 crate 外で使用意義が消える `DecoderOutputReceiver` 型エイリアスの可視性を `pub(crate)` に落とし、 struct docstring / `OutputSink` docstring から `next_decoded_frame` を対比参照する記述を除去する。

将来拡張余地を放棄する根拠: (1) 本番経路 (mp4 reader / RTSP / RTMP / SRT / processor 経路) はすべて同期 `handle_input_sample(None)` + `poll_output` の drain ループで EOS を扱い、 非同期 EOS シグナルを要求する使用側は現時点で見えない (2) 仮に将来必要が生じても、 Sender の `drop` シグナル (現在の `Option<crate::Result<VideoFrame>>` の `None`) より `enum { Ok, EndOfStream, Err }` のような明示型を新たに導入する方が意図が明瞭で、 現状の `Option` 保持は必然性が薄い。

## 優先度根拠

Low。

- 本番挙動は不変 (未使用 API の削除)
- 実装コスト自体は軽微 (純削除 2 箇所 + docstring 書き替え 3 箇所 + 可視性引き下げ 1 箇所)
- closed/0066 の追加判断を覆すため PR に対する Decision Owner 承認プロセスが必要 (詳細は §解決方法 §1)

## 現状

closed issue 0073 (同期 wrap `VideoDecoder` 削除 + `AsyncVideoDecoder` → `VideoDecoder` リネーム、 2026-07-06 マージ済み、 commit `423480b7`) 時点の `src/decoder.rs` を基準とする。

- `VideoDecoder::next_decoded_frame` の本番呼出はゼロ。 本番の映像デコード出力取得は、 decoder task loop (RTMP / RTSP / SRT inbound endpoint、 mp4 reader) と processor 経路 (`VideoDecoder::run`。 subcommand_inspect / sora recording subcommand は `VideoDecoder::run` を呼び出す使用側で、 独立した駆動経路ではない) がすべて同期の `handle_input_sample` + `poll_output` (内部 `try_recv`) を経由する
- 唯一の呼出元は「そのメソッド自身を試すためだけのテスト」(`tests/decoder_tests.rs:58` の `video_decoder_processes_real_vp9_frame_via_next_decoded_frame`)
- メソッドの docstring 自身にも拡張余地としての `Option` 保持の意図が明記されている (原文は §目的で引用)

## 設計方針

### 削除・書き替え対象 (全 5 箇所)

行番号は着手時に `grep -rn 'next_decoded_frame' src/ tests/ pbt/ fuzz/ examples/` で再特定する (以下は 2026-07-06 時点の実測位置)。 純削除項目 (1 と 4) は削除範囲に末尾空行 1 行を含める (`cargo fmt --check` の `blank_lines_upper_bound = 1` で空行連続を検出させないため)。 書き替え項目 (2 / 3 / 5) の書き替え後文言は下記の想定文案に authoritative に従う。

1. **[純削除] `next_decoded_frame` メソッド定義** — `src/decoder.rs:466-478` の docstring + signature + body 全体 + 末尾空行 1 行 (計 13 行、 実測)
2. **[書き替え] `VideoDecoder` struct docstring** — `src/decoder.rs:378-380` の 3 行を想定文案 (§書き替え項目 2) 通りに置換。 `src/decoder.rs:376-377` の見出しと空行、 および `src/decoder.rs:381-385` の空行と「注意」ブロックは無変更
3. **[書き替え] `OutputSink` docstring** — `src/decoder.rs:342-344` の 3 行全体 (line 342 の「同じ理由で」「は」の削除 + `next_decoded_frame` parenthetical の除去) を想定文案 (§書き替え項目 3) 通りに置換。 `src/decoder.rs:335-341` の他の部分は無変更
4. **[純削除] `next_decoded_frame` 専用テスト定義** — `tests/decoder_tests.rs:51-87` の `video_decoder_processes_real_vp9_frame_via_next_decoded_frame` テスト全体 (docstring + `#[tokio::test(flavor = "multi_thread")]` + `async fn` シグネチャ + body + `}` + 末尾空行 1 行、 計 37 行、 実測)
5. **[書き替え] 兄弟テスト `video_decoder_poll_output_returns_processed` の docstring** — `tests/decoder_tests.rs:88-93` を想定文案 (§書き替え項目 5) 通りに置換

### 書き替え項目 2 想定文案

現行 `src/decoder.rs:376-385` を次に置換する (`:382-385` の「注意」ブロックはそのまま維持)。

```rust
/// 内部チャンネルベースの映像デコーダー
///
/// decoder task loop (mp4 reader / RTSP / RTMP / SRT) および `run` (processor 経路) から
/// `handle_input_sample` / `poll_output` 経由で同期的に駆動する。
///
/// **注意**: 非同期な内部デコーダー (Nvcodec 等) 使用時、 `VideoDecoder` を drop する前に
/// EOS + drain (`handle_input_sample(None)` + `poll_output` ループ) を完走させないと、
/// コールバックが drop 中に emit した残物とメトリクス (`total_output_video_frame_count`) が
/// 乖離する可能性がある (エラー時の warm-up 中止経路等で発生し得る)。
```

### 書き替え項目 3 想定文案

現行 `src/decoder.rs:335-344` を次に置換する。

```rust
/// 内部デコーダーが出力フレーム / エラーを `VideoDecoder` 内の受信側 (`output_rx`) に流すためのシンク。
///
/// 出力フレーム (`emit_ok`) 送信時に `total_output_metric` の増分を物理的に強制ペアリングする。
/// エラー (`emit_err`) 送信時はメトリクスを増分しない (出力フレーム数の意味論を汚さないため)。
///
/// `unreachable!()` 検出契約: シンクと `output_rx` は `VideoDecoder` 内で同居するため、
/// 送信失敗 (受信側 drop) は構造上到達不能な不変条件違反 = バグ。 通常運用では起こらない。
/// `poll_output` の `Disconnected` 分岐も、 シンクと `output_rx` の同居不変条件が破れない限り
/// 到達不能なため `unreachable!()` で潰す。
```

### 書き替え項目 5 想定文案

現行 `tests/decoder_tests.rs:88-93` を次に置換する。

```rust
/// `VideoDecoder::poll_output` の同期取り出し経路を実 VP9 フィクスチャで踏破する
///
/// 検証対象パス: `VideoDecoder::handle_input_sample` → `VideoDecoderInner::decode`
/// → `sink.emit_ok` → 内部チャンネル → `VideoDecoder::poll_output` の全段を
/// `VideoDecoder` の公開 API 呼び出しだけで踏破する回帰テスト。
```

### 維持対象 (削除に伴い触らない)

- **`src/decoder.rs:393-396` の drop 順制御コメント** と **`:397-398` のフィールド宣言順** — Nvcodec の worker drop 中に callback が `sink.emit_ok` → `tx.send` した際に `output_rx` を alive に保つ契約
- **`DecoderOutputSender` の `pub`** (`src/decoder.rs:330`) — `tests/e2e.rs` (別 crate) が `hisui::decoder::OutputSink::new(tx, counter)` を呼び、 `OutputSink::new` の公開シグネチャに `DecoderOutputSender` が引数型として露出しているため `pub(crate)` に落とすと E0446 (private type in public interface) で compile error。 `DecoderOutputReceiver` (crate 外参照ゼロ、 シグネチャ露出なし) との非対称は Sender/Receiver の役割非対称の自然な帰結
- **`src/decoder.rs:999-1003` の tests mod 内コメント** — `output_rx` フィールドを言及するがフィールド自体は削除後も維持されるため文言変更不要

### 削除による失われるカバレッジと代替担保

削除対象テスト `video_decoder_processes_real_vp9_frame_via_next_decoded_frame` は「`Some(Ok(frame))` の pattern match」と「`frame.size().width == 640` / `height == 480`」を assert する。 兄弟テスト `video_decoder_poll_output_returns_processed` (存続) は `DecoderRunOutput::Processed(_)` の pattern match のみで size 検証は持たない。 削除で直接失われる担保は実 VP9 fixture (blue-640x480-vp9.mp4) の解像度検証で、 これは存続する `video_decoder_metrics_increment_by_input_count` (同一 fixture でフレーム数検証) と `vp9_multi_resolutions` / `multi_resolutions_test` (`tests/decoder_tests.rs:236-250` で 640x480 / 320x320 の Y/U/V 平面値まで検証) が別途担保する。

### DecoderOutputReceiver の可視性整理

`DecoderOutputReceiver` は現時点でも公開シグネチャには現れず (`next_decoded_frame` の戻り値型は `Option<crate::Result<VideoFrame>>` で `Receiver` 型は露出しない、 `output_rx` フィールドも非 `pub`)、 crate 外からの参照実測もゼロ (`src/decoder.rs:333` の型定義と `:398` のフィールド型の 2 箇所のみ)。 削除後は crate 内でも `poll_output` の `try_recv` 経路 1 箇所からしか使われなくなり、 crate 外に対する型エイリアス露出の意義が実質失われる。 `pub type DecoderOutputReceiver = tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>;` (`src/decoder.rs:333`) を `pub(crate) type DecoderOutputReceiver = ...` に変更する。 副次影響として `cargo doc` 生成物から `DecoderOutputReceiver` 項が消える (hisui は crates.io 未 publish のため docs.rs 影響なし)。

### shiguredo-rust 規約整合

- モック / スタブ不使用
- 新規 trait 追加なし
- `#[non_exhaustive]` 不使用 (enum バリアント変更なし)

## 完了条件

- 削除対象 (全 5 箇所) がすべて反映されている
- `DecoderOutputReceiver` の可視性が `pub(crate)` に変更されている
- 変更ファイルは `src/decoder.rs` と `tests/decoder_tests.rs` の 2 ファイルのみ (`git diff --name-only develop...HEAD -- src tests` で確認)
- grep 検証:
  - `grep -rn 'next_decoded_frame' src/ tests/ pbt/ fuzz/ examples/` の hit が 0 件
  - `grep -rn 'pub(crate) type DecoderOutputReceiver' src/` の hit が 1 件 (新可視性の適用確認。 旧 `pub type` の残骸は `cargo check` の重複定義エラーで検出されるためこの 1 件検証で両方担保)
  - `grep -n '非同期に取得' src/decoder.rs` の hit が 0 件 (struct docstring および削除対象メソッド docstring の残骸検出)
  - `grep -n '別テストに対し\|実際に踏む\|上位 API 呼び出し' tests/decoder_tests.rs` の hit が 0 件 (兄弟テスト docstring の書き替え漏れ検出。 削除項目 5 は line 91 の「実際に踏む。」および line 93 の「上位 API 呼び出しだけで」も置換対象で、 line 92 の「別テストに対し」1 語だけを消して他 2 語を残す部分反映を検出する)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

### 1. 承認と分岐

削除実装は通常の refactor issue フローに従う。 承認プロセスは以下:

- 削除 branch (`feature/refactor-remove-unused-next-decoded-frame`) を切って §2〜§4 の実装を進め、 PR を開設する。 PR タイトルは commit タイトルと同一、 PR 本文は §目的の要旨 + §完了条件のチェックリストを載せる (shiguredo-git 規約に PR 形式規定なし。 本 issue が実装者への指示として明示)
- Decision Owner (@sile) の PR review LGTM が承認確定。 別途 issue ファイルに承認記録セクションは追加しない (承認履歴は PR review に残る)
- 承認見送りの場合: `develop` ブランチで以下を **別々のコミット** に分けて実施する (shiguredo-git §「issue ファイル単体のコミット」は「他の issue と混ぜないこと」を規定するため 2 コミットに分ける)。 見送り理由は本 issue ファイル末尾に永続化する (PR コメントは GitHub 外部依存で長期永続性が薄いため):
  1. `0078 closed/0066 §関連節に 0078 の削除見送り判断を追記する` (コード変更コミット形式 `{SEQ} {変更内容}`。 target 明示のため冒頭 SEQ を単一 `0078` で扱う。 `issues/closed/0066-feature-refactor-add-async-video-decoder.md` §関連節末尾に次の 1 行を追加): `` - closed/0078 (`feature/refactor-remove-unused-next-decoded-frame`): `next_decoded_frame` の削除を検討したが保持判断維持 ``
  2. `0078 closed 未使用の VideoDecoder::next_decoded_frame を削除する` (状態遷移コミット形式 `{SEQ} {STATE} {TITLE}`。 §5 の成功時 closing と commit タイトルが完全一致するため、 判別は commit body 1 行目に「削除見送りで close (詳細は本 issue の `## 見送り記録` セクションおよび参照 PR)」を記す形で担保する)。 このコミットで以下を単一 atomic edit にまとめる:
     - `Completed:` を見送り決定日に更新
     - 本 issue 末尾に `## 見送り記録` セクション (見送り決定日 + 見送り理由 + 参照 PR 番号) を追加
     - `git mv issues/0078-....md issues/closed/`

### 2. 実行番号を再特定

`grep -rn 'next_decoded_frame' src/ tests/ pbt/ fuzz/ examples/` で削除・書き替え対象の実行番号を再特定する (§設計方針の再掲。 着手時に必ず実施)。

### 3. 削除・書き替えと可視性整理を単一コミットで完成させる

§設計方針 §削除・書き替え対象の 5 項目 + §DecoderOutputReceiver の可視性整理 (`pub type` → `pub(crate) type`) を単一コミット `0078 未使用の VideoDecoder::next_decoded_frame を削除する` (コード変更コミット形式 `{SEQ} {変更内容}`。 issue タイトル原文を流用) に収める。 書き替え項目 2 / 3 / 5 の書き替え後文案は §設計方針の想定文案に authoritative に従う。 可視性引き下げは削除と論理的に一体 (削除後は `DecoderOutputReceiver` が crate 内でも 1 箇所からしか使われず、 crate 外露出の意義が消えるため、 削除に付随して整えることで削除後の型面と可視性が一貫する)。

### 4. cargo 検証

§完了条件の cargo コマンドを default + `--no-default-features` の両方で PR 開設前の local で通す (shiguredo-git §「全てのテストが通らない限りコミットしないこと」)。

### 5. マージ後の closing

PR merge 完了直後に Reporter (@sile) が `develop` ブランチで単一コミット `0078 closed 未使用の VideoDecoder::next_decoded_frame を削除する` (`Completed:` を PR merge 日に更新 + `git mv issues/0078-....md issues/closed/`) で closing する。

## CHANGES.md について

内部リファクタにつき記載不要。 `VideoDecoder` 系は hisui の lib target が現状 crates.io に未 publish で workspace 内の bin / tests 専用のため、 外部への影響は生じない。

## 関連

- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 採用案 C の親 issue。 本 API は 0066 の派生方針 (δ) で導入された非同期取り出し API で、 削除の影響は 0057 §3 本体判断 (Sender 経由の出力統一) には及ばない。 0057 §3 分割表は移行系列の実績を扱う構造で、 移行完了後の削除は表対象外のため本 issue では追記しない
- closed/0066 (`feature/refactor-add-async-video-decoder`、 2026-07-01 close): 本 API を追加した親 issue (完了条件で `pub async fn next_decoded_frame_async(...)` 提供を明記)。 本 issue はその追加判断のうち、 将来拡張余地としての `next_decoded_frame` 保持部分を撤回する (async 化の骨格は撤回しない)
- closed/0068 (`feature/refactor-migrate-video-decoder-users-to-async`、 2026-07-03 close) / closed/0071 (`feature/refactor-mp4-reader-async-video-decoder`、 2026-07-02 close) / closed/0072 (`feature/refactor-inbound-endpoint-async-video-decoder`、 2026-07-06 close): 移行系列。 本 API は触られず保持継続された
- closed/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`、 2026-07-06 マージ): 本 API を rename のみ扱った (0073 の scope は「サフィックス削除」)。 本 issue の発端 (0073 の `review-diff-code` で削除候補として検出)。 0073 のマージで移行系列 (0068 / 0071 / 0072) の依存条件も暗黙に含意されるため、 本 issue の着手条件は既に成立している
