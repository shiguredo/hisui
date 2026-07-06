# 未使用の VideoDecoder::next_decoded_frame を撤去する

- Priority: Low
- Created: 2026-07-06
- Completed: {YYYY-MM-DD}
- Model: Opus 4.8
- Branch: feature/refactor-remove-unused-next-decoded-frame
- Polished: {YYYY-MM-DD}
- Reporter: @sile
- Decision Owner: @sile

## 目的

`VideoDecoder::next_decoded_frame` (`pub async fn next_decoded_frame(&mut self) -> Option<crate::Result<VideoFrame>>`、 実体は `self.output_rx.recv().await` だけ) は本番コードから 1 箇所も呼ばれていない未使用 public API である。 YAGNI (CLAUDE.md「Premature Optimization is the Root of All Evil」) の観点で、 投機的に保持された未使用 API を撤去し、 `VideoDecoder` の消費モデルを実際に使われている同期 poll 一本に収束させる。

## 優先度根拠

Low。

- 本番挙動には一切影響しない (未使用 API の削除)。 緊急性はない
- ただし後述のとおり closed issue 0066 で Decision Owner が意図的に残した API であり、 撤去は設計判断の反転を伴う。 **着手前に Decision Owner (@sile) の承認が必須**
- 承認が得られない場合は本 issue を close する (保持を継続する判断もあり得る)

## 現状

closed issue 0073 (同期 wrap `VideoDecoder` 削除 + `AsyncVideoDecoder` → `VideoDecoder` リネーム) 完了後の `src/decoder.rs` 時点を基準とする。

- `VideoDecoder::next_decoded_frame` の本番呼出はゼロ。 本番の映像デコード駆動経路 (RTMP / RTSP / SRT inbound endpoint、 mp4 reader、 `VideoDecoder::run`、 subcommand_inspect / sora recording subcommand) はすべて同期の `handle_input_sample` + `poll_output` (内部 `try_recv`) を使っており、 非同期 `recv().await` 版の `next_decoded_frame` は誰も使わない
- 唯一の呼出元は「そのメソッド自身を試すためだけのテスト」(`tests/decoder_tests.rs` の `video_decoder_processes_real_vp9_frame_via_next_decoded_frame`)
- メソッドの docstring 自身が投機的保持を明記している: 「現状の実装では EOS 経路で sink を drop しないため `None` は構造上到達しないが、 将来 EOS を非同期経路で通知する形が必要になった際に `None` を EOS シグナルとして活用できるよう `Option` を維持している」

### 重要: 事故的な死にコードではない

本 API は **closed issue 0066 で Decision Owner (@sile) が意図的に追加・保持した public API** である。 closed/0066 の完了条件に「`AsyncVideoDecoder` が `pub async fn next_decoded_frame_async(...)` を提供する」と明記され、 closed/0057 §3 採用案 C の「channel ベースの非同期取り出しインターフェース」に対応する。 したがって本 issue の撤去は 0066 の明文の保持判断を覆すことになる。 判断者は @sile であり、 承認なしに撤去してはならない。

本 API は closed issue 0073 の際に `next_decoded_frame_async` → `next_decoded_frame` へリネームだけされ、 撤去は明示的にスコープ外とされた。 0073 のレビュー (review-diff-code) で削除候補として検出されたのが本 issue の発端である。

## 設計方針

### 撤去対象 (全 6 箇所)

行番号は 0073 マージ後にずれる想定のため、 着手時に `grep -rn 'next_decoded_frame' src/ tests/` で再特定する。

1. `src/decoder.rs` の `next_decoded_frame` メソッド本体
2. 同メソッドの docstring (投機的保持理由を含む節)
3. `VideoDecoder` の struct docstring の「直接利用するときは `next_decoded_frame` で非同期に取得する」節。 撤去に連動して struct docstring の同期 / 非同期の言い回しを同期利用のみに簡潔化する
4. `OutputSink` の docstring の `next_decoded_frame` 言及
5. `tests/decoder_tests.rs` のテスト `video_decoder_processes_real_vp9_frame_via_next_decoded_frame` (このメソッドを踏むためだけのテスト。 同期 poll 経路は別テスト `video_decoder_poll_output_returns_processed` が担保済みで、 撤去してもカバレッジは実質低下しない)
6. `tests/decoder_tests.rs` の兄弟テスト docstring 内の `next_decoded_frame` への相互参照

### 付随して検討する論点

- `next_decoded_frame` 撤去後、 `output_rx` (`DecoderOutputReceiver`) は `poll_output` の `try_recv` からのみ使われる (`recv().await` 経路が消える)。 `DecoderOutputReceiver` は非公開フィールド `output_rx` のみで使われ公開シグネチャに現れないため、 `pub` を落とす (もしくはインライン化する) 余地がある。 ただし `DecoderOutputSender` は `OutputSink::new` の公開引数なので `pub` を維持する
- 撤去で `VideoDecoder` の消費モデルが同期 poll 一本に収束する。 struct docstring をそれに合わせて簡潔化する

### shiguredo-rust 規約整合

- モック / スタブ不使用
- 新規 trait 追加なし

## 完了条件

- 上記 6 箇所が削除され、 `grep -rn 'next_decoded_frame' src/ tests/ pbt/ fuzz/ examples/` が 0 件
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

1. Decision Owner (@sile) に撤去の承認を得る (0066 の保持判断を覆すため)。 承認が得られなければ本 issue を close
2. 着手時 grep で撤去対象 6 箇所を再特定する
3. メソッド本体・docstring・テストを削除し、 struct docstring / OutputSink docstring の言及を除去して同期 poll 一本の記述に整える
4. `DecoderOutputReceiver` の `pub` 可視性を落とせるか確認して対応する
5. 完了条件の cargo コマンドを default + `--no-default-features` の両方で通す

## 依存関係

- closed issue 0073 (`feature/refactor-remove-sync-video-decoder-and-rename`) のマージ後に着手する。 0073 でリネーム済みの `next_decoded_frame` を対象とするため

## CHANGES.md について

内部リファクタにつき記載不要。 `VideoDecoder` 系は外部公開していない (hisui の lib target は crates.io 未公開)。

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): 本 API を意図的に追加・保持した親 issue。 本 issue はその保持判断を覆す
- closed/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`): 本 API をリネームのみし撤去はスコープ外とした。 本 issue の発端
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 採用案 C の非同期取り出しインターフェースの設計判断元
