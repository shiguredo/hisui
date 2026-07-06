# 同期 VideoDecoder wrap を削除して AsyncVideoDecoder を VideoDecoder にリネームする

- Priority: Medium
- Created: 2026-07-02
- Completed: 2026-07-06
- Model: Claude Opus 4.7
- Branch: feature/refactor-remove-sync-video-decoder-and-rename
- Polished: 2026-07-06
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0066 で導入された「AsyncVideoDecoder 新規追加 + 同期 VideoDecoder は wrap 構造で挙動維持」の段階的移行方針 (δ) は、 全使用側が `AsyncVideoDecoder` に切り替わった時点で「同期 wrap を削除し、 `AsyncVideoDecoder` を `VideoDecoder` にリネームする」ことで最終形に到達する。 本 issue はその最終ステップを扱う。

closed/0068 / closed/0071 / closed/0072 の完了により、 本番経路 (processor 経路 / mp4 reader / RTSP / RTMP / SRT) はすべて `AsyncVideoDecoder` ベースに移行済みで、 同期 wrap `VideoDecoder` は本番経路での使用ゼロ (参照はテストと `get_engines` 委譲のみ) の実質 dead code になっている。 closed issue 0057 §3 採用案 C の長所 (v) 「callback friendly 定義 (ホップ数上限 1)」は、 wrap の 2 段ホップ (`VideoDecoder::poll_output` → `AsyncVideoDecoder::poll_output_sync`) が型として残る限り最終達成にならない。 本 issue で wrap 型を消し、 `AsyncVideoDecoder` を `VideoDecoder` にリネームして命名を最終化する。

## 優先度根拠

Medium。

- closed issue 0057 §3 の 2 系統共存を最終解消する方針 (δ、 同 §3 備考) との最終整合は本 issue でしか達成できない
- 本 issue 単独では外部挙動は不変。 内部型名の整理のため緊急性は低い
- ただし wrap 状態のまま放置すると「AsyncVideoDecoder と VideoDecoder のどちらを使うべきか」の API 選択の負債が蓄積する
- 依存 3 issue はすべて closed 済みで、 本 issue が移行系列の残る唯一のステップ (§依存関係)

## 現状

2026-07-06 (0072 close、 develop merge 済み) 時点の `src/decoder.rs` の構造:

```rust
// AsyncVideoDecoder (`:385`)
pub struct AsyncVideoDecoder { ... }

impl AsyncVideoDecoder {
    pub fn new(options: VideoDecoderOptions, mut compose_stats: crate::stats::Stats) -> Self  // :400
    pub fn handle_input_sample_sync(&mut self, sample: Option<MediaFrame>) -> Result<()>      // :424
    pub fn poll_output_sync(&mut self) -> Result<DecoderRunOutput>                            // :441
    pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>>     // :472
    pub async fn run(self, handle, input_track_id, output_track_id) -> Result<()>             // :476 (0068 で追加、 processor モデル用)
    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName>      // :520
}

// 同期 wrap (`:585`)。 全メソッドが AsyncVideoDecoder への委譲
pub struct VideoDecoder { inner_decoder: AsyncVideoDecoder }
impl VideoDecoder {
    // new (:590) / run (:596) / handle_input_message (:629) /
    // handle_input_sample (:637) / poll_output (:641) / get_engines (:645)
}

pub fn drain_video_decoder_output(decoder: &mut VideoDecoder, ...) -> Result<DrainResult>     // :671
```

wrap の本番使用はゼロであることを確認済み:

- `drain_video_decoder_output` の呼出は wrap 側 `run` 内 (`:613`) のみ (0066 時点の「4 ファイル / 5 call site 利用」は 0071 / 0072 で解消済み)
- wrap 側 `handle_input_message` (video) の呼出も wrap 側 `run` 内 (`:611`) のみ。 wrap 削除で公開 API から消えるが外部影響なし (`AsyncVideoDecoder::run` は `Message` dispatch を自前展開済み)

残存する wrap `VideoDecoder` への参照は次の 2 群に分類される:

**(a) リネーム後にテキスト無変更で新型に解決される箇所 (実作業なし)**

- `src/subcommand_list_codecs.rs:6` (brace import) / `:87` (`VideoDecoder::get_engines`。 closed/0068 スコープ外節で本 issue 送りと明記されていた箇所)
- `src/decoder.rs:723` (`initialize_decoder` 内の `VideoDecoder::get_engines`)
- `tests/e2e.rs:7` (brace import) / `:455` (`VideoDecoder::new` + `run` の pipeline harness)

`get_engines` / `new` / `run` は新 `VideoDecoder` (旧 `AsyncVideoDecoder`) にも同名同シグネチャで存在するため、 削除 + リネーム後は同一テキストのまま新型に解決される。 旧 wrap `run` (drain 関数経由) から新 `run` (inline drain) への差し替えは行動等価 (closed/0068 §「骨子と wrap 版の行動等価性」で確定済み)

**(b) 書換が必要な箇所 (§tests への影響で扱う)**

- `src/decoder.rs` の `#[cfg(test)] mod tests`
- `tests/decoder_tests.rs`

`pbt/` / `fuzz/` / `examples/` に `AsyncVideoDecoder` / wrap `VideoDecoder` の参照はない (grep 確認済み。 `examples/obsws_bootstrap` の `VideoDecoderFactory` は libwebrtc 由来の別物)。

## 設計方針

### 削除対象 (`src/decoder.rs`)

- `pub struct VideoDecoder` (wrap 型、 `:585`) と `impl VideoDecoder` の全メソッド (`new` / `run` / `handle_input_message` / `handle_input_sample` / `poll_output` / `get_engines`)
- `pub fn drain_video_decoder_output` (`:671`)。 `drain_audio_decoder_output` (`:650`) と `DrainResult` は audio 側で使用中のため存続する
- **注意**: `run` は同名メソッドが 2 つ存在する。 削除するのは wrap 側 `:596` (`drain_video_decoder_output` 経由)。 `AsyncVideoDecoder::run` (`:476`、 `poll_output_sync` の inline drain) は存続し、 リネーム後の `VideoDecoder::run` になる

### リネーム対象

- `pub struct AsyncVideoDecoder` → `pub struct VideoDecoder` (定義側は `src/decoder.rs:385` (struct) と `:399` (impl) の 2 箇所)
- メソッドの `_sync` / `_async` サフィックス削除 (§決定事項 1):
  - `handle_input_sample_sync` → `handle_input_sample`
  - `poll_output_sync` → `poll_output`
  - `next_decoded_frame_async` → `next_decoded_frame`
- 型名のコード参照 (コンパイルエラーで全数検出されるが列挙): `src/subcommand_inspect.rs:11, :215` / `src/sora/recording_subcommand_vmaf.rs:14, :362, :480` / `src/sora/recording_subcommand_compose.rs:14, :463` / `src/mp4/reader.rs:1595, :1602` / `src/rtsp/subscriber.rs:1551` / `src/rtmp/inbound_endpoint.rs:532` / `src/srt/inbound_endpoint.rs:1118` (いずれも import / `AsyncVideoDecoder::new` / 型注釈)。 tests 側は §tests への影響で扱う
- メソッド名の呼出箇所 (同上): `src/decoder.rs:492, :493, :498` (`run` 内の self 呼出) / `src/mp4/reader.rs:1615, :1616, :1620` / `src/rtsp/subscriber.rs:1560, :1561, :1565` / `src/rtmp/inbound_endpoint.rs:538, :541` / `src/srt/inbound_endpoint.rs:1124, :1127`
- コメント / docstring 内の型名・メソッド名参照も同時に書き換える:
  - `src/decoder.rs:329-343`: 型エイリアスと `OutputSink` の docstring (`AsyncVideoDecoder` / `poll_output_sync` / `next_decoded_frame_async` を参照)
  - `src/decoder.rs:375-383` / `:419-423` / `:437-440` / `:447`: 「同期ラッパー (`VideoDecoder`) から呼ぶ」前提の説明は wrap 消滅後に虚偽になるため、 「decoder task loop (mp4 reader / RTSP / RTMP / SRT) および `run` (processor 経路) から呼ぶ同期 API」の実態に書き直す。 struct docstring (`:375-383`) は同期利用 (task loop / `run` 内部) と非同期利用 (`next_decoded_frame`) の両経路に触れる文面にする
  - `src/decoder.rs` のテスト側コメント: `:1065` / `:1080` / `:1089` / `:1100` / `:1105` / `:1113` / `:1116` (`:1116` の「`VideoDecoder::run` の drain ループ」は wrap 側 run を指している)
  - `src/decoder/nvcodec.rs:234` / `src/mp4/reader.rs:1638` / `src/rtsp/subscriber.rs:1578` / `src/decoder/video_toolbox.rs:137`
- `AsyncVideoDecoder::run` にはリネーム時に docstring を付与する (processor モデル (`ProcessorHandle` + subscribe / publish) 用の駆動 API である旨)
- 型エイリアス `DecoderOutputSender` / `DecoderOutputReceiver` と `OutputSink` の名前はそのまま維持
- 型名更新の対象範囲は `src/` と `tests/` のみ (§現状のとおり `pbt/` / `fuzz/` / `examples/` は参照なし)

### tests への影響

- `tests/decoder_tests.rs`:
  - import (`:3`): `decoder::{AsyncVideoDecoder, VideoDecoder, VideoDecoderOptions}` → `decoder::{VideoDecoder, VideoDecoderOptions}` に縮約し、 関数内 `use hisui::decoder::AsyncVideoDecoder;` (`:61`) も削除
  - `async_video_decoder_processes_real_vp9_frame_via_next_decoded_frame_async` → `video_decoder_processes_real_vp9_frame_via_next_decoded_frame` (隣接テストと同じ `video_decoder_` prefix に付け替え、 内部呼出 API 名も追従)。 非同期 recv 経路 (`next_decoded_frame`) の検証として、 同期 poll 経路の次テストとの区別は維持される
  - `video_decoder_poll_output_returns_processed_via_wrap_delegation` → `video_decoder_poll_output_returns_processed` (wrap 廃止に伴い純粋な `VideoDecoder::poll_output` (同期 poll 経路) の検証テストに書換)
  - `video_decoder_metrics_increment_by_input_count_via_wrap_delegation` → `video_decoder_metrics_increment_by_input_count` (同上)
  - リネーム 3 テストの doc / inline コメント (「同期ラップ経路」「wrap 経路」の記述、 特に `:56` の `_via_wrap_delegation` への相互参照) も単一経路の記述に書き直す
  - `multi_resolutions_test` (`:201`) の wrap / async 二重 decode + byte-wise 等価性検証 (`:233-254`): 等価性検証は 0066 移行期の「wrap == async」担保であり、 wrap 削除で比較対象が消滅して検証命題が成立しなくなるため削除する。 `decode_video_frames_with_pipeline` (`:305`、 wrap 版) を削除し、 `decode_video_frames_with_async_pipeline` (`:378`) を `decode_video_frames_with_pipeline` にリネームして一本化する。 呼出側は比較ブロック削除に伴い `input_frames.clone()` / `options.clone()` の除去と `wrap_output` 変数名の付け替えも行う。 デコード出力の正しさは既存の解像度・単色 (青 / 赤 YUV 値) 検査 (`:258-286`) が単一経路で引き続き担保する。 なお wrap 版 harness はシグネチャ互換のためコンパイルエラーにならず、 対応漏れは「同一経路を 2 回走らせて自己比較する無意味な重複」として静かに残る (`wrap_output` / `wrap 版` の残存は完了条件の grep 3 で検出できるが、 一本化の完全性はレビューでも確認する)
- `src/decoder.rs` の `#[cfg(test)] mod tests`:
  - `poll_output_sync_returns_finished_when_eos_and_channel_empty` / `poll_output_sync_returns_pending_when_not_eos_and_channel_empty` / `poll_output_sync_returns_err_when_emit_err_received`: テスト名の `_sync` を削除し、 本体の `AsyncVideoDecoder::new` / `handle_input_sample_sync` / `poll_output_sync` 呼出と doc / inline コメントも追従
  - エンジン選択テスト 3 本 (`vp9_without_size_skips_video_toolbox` / `av1_without_size_skips_video_toolbox` / `vp9_with_size_selects_available_engine`): 内部状態参照 `decoder.inner_decoder.inner` (計 8 箇所: `:901, :903, :925, :927, :955, :957, :961, :965`。 うち 4 箇所は `vp9_with_size_selects_available_engine` の macOS cfg 分岐込み) を `decoder.inner` に書換。 構築の `VideoDecoder::new` (`:896, :921, :948`) は新型にテキスト無変更で解決され、 `decoder.handle_input_sample(...)` (`:898, :922, :949`) も `_sync` サフィックス削除後の新メソッドに無変更で解決される (削除 + リネーム + サフィックス整理を単一コミットで行う §決定事項 2 が前提)。 テスト名・検証意図は不変
- `tests/e2e.rs:455` の pipeline harness は無変更で新型に解決される (§現状 (a))

### 決定事項 (実装で覆さない)

起票時の未確定論点 1〜4 は 2026-07-06 にすべて確定した (closed/0072 からは「0073 の未確定論点 4」の名で参照されている)。

1. **メソッド命名**: `_sync` / `_async` サフィックスは全削除する。 wrap 削除で `handle_input_sample` / `poll_output` の名前は空き、 `next_decoded_frame` にも衝突はない。 `next_decoded_frame` の非同期性は `async fn` シグネチャから自明で、 同一型内に同期版が存在しなくなるため区別サフィックスは不要
2. **リネーム実施順序**: option A (wrap 削除 → `AsyncVideoDecoder` を `VideoDecoder` にリネーム)。 wrap 削除・型リネーム・サフィックス整理・tests 追従 (§解決方法の手順 2〜4) は単一コミットに収める。 途中で区切ると §現状 (a) の参照・`decoder.handle_input_sample` 呼出・`inner_decoder` 参照が未解決の「`cargo clippy --all-targets` / `cargo test` 不通コミット」が履歴に残るため、 全ターゲットが通る単位でコミットする
3. **完了検証手段**: CI への恒久 grep チェックは追加しない。 型の消滅後は旧名の再流入がコンパイルエラーで検出されるため恒久チェックは負債にしかならず、 完了条件の一回限り grep (型名 + メソッド名 + wrap 固有アーティファクト) と PR レビューで担保する
4. **spawn pattern 抽象の共通化**: option b (共通化しない) を採用。 共通化の新規 issue も現時点では起票しない
    - 根拠: 4 使用側の実装は 3 系統に分かれており (mp4 reader = `TrackSender` async send + sender 回収 + discard_mode / RTSP = graceful stop + `TrackPublisher` / RTMP / SRT = 直送 + Drop abort の簡素版ペア)、 真に同一なのは中央の drain loop 約 20 行のみ。 sink 型 / shutdown 契約 / discard hook の 3 軸を吸収する抽象は除去対象の重複より大きくなる。 drain loop の契約本体は `poll_output_sync` (リネーム後 `poll_output`) 側にあり、 各 loop は薄い消費者にすぎない。 各実装の inline コメント (RTSP の Fatal 経路 flush 理由、 mp4 の sender 回収理由) は使用側の文脈に結び付いており、 core へ吸い上げると文脈が失われる
    - closed/0072 残懸念 §5 の `handle_input(input: DecoderInput)` 相当の helper 追加も見送る (`DecoderInput` は各使用側の module-private 型であり、 helper 化は共通型の導入 = 共通化そのもののため option b に含める)
    - 再検討トリガー: (1) 5 箇所目の使用側が現れたとき (特に RTMP / SRT と同型の簡素版が 3 つ目になったら 60 行ペアの共通化は自明に元が取れる)、 (2) drain loop の契約変更で 4 箇所同時修正が実際に発生したとき

### shiguredo-rust 規約整合

- モック / スタブ不使用 (テストは実 decoder + tokio channel + 実 pipeline)
- `#[non_exhaustive]` 不使用
- 新規 trait 追加なし

## 完了条件

- `pub struct VideoDecoder` (wrap 型) と `drain_video_decoder_output` が `src/decoder.rs` から削除されている
- `pub struct AsyncVideoDecoder` が `pub struct VideoDecoder` にリネームされ、 メソッドの `_sync` / `_async` サフィックスが削除されている (§決定事項 1)
- grep 1: `grep -rn '\bAsyncVideoDecoder\b' src/ tests/ pbt/ fuzz/ examples/` の hit が 0 件 (コメント / docstring 含む)
- grep 2: `grep -rn 'handle_input_sample_sync\|poll_output_sync\|next_decoded_frame_async' src/ tests/ pbt/ fuzz/ examples/` の hit が 0 件 (同上)
- grep 3: `grep -rn 'inner_decoder\|drain_video_decoder_output\|同期ラッパー\|同期ラップ\|wrap 経路\|wrap 版\|wrap 側\|wrap_delegation\|wrap_output' src/ tests/` の hit が 0 件 (wrap 固有アーティファクトの不在 = closed issue 0057 §3 採用案 C の長所 (v) 「ホップ数上限 1」達成の機械検証。 現状の全 hit が本 issue の削除・書換対象に含まれることは確認済みで、 0 件に到達可能)
- `tests/decoder_tests.rs` の wrap / async 二重 harness が一本化されている (§tests への影響)
- `issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md` §3 分割表が §解決方法 5 の全編集どおりに更新されている: 0071 / 0072 / 0073 の 3 行追加 (closed/0071 関連節で「0073 完了時にまとめて対応」と確定していた宿題) / 0068 行の実績修正と方針 (δ) 注記 2 文の更新 (本 issue で追加した宿題) / 分割表直上の依存順序行の更新 / 0067 行の依存先列補完 (broken windows) / `open/` → `closed/` 表記更新。 0057 側の整形は本 issue の grep / build コマンドでは検出されないため、 §解決方法 5 との突き合わせで確認する
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 依存関係

依存 3 issue はすべて closed 済みで、 着手条件は成立している:

- closed/0068 (`feature/refactor-migrate-video-decoder-users-to-async`、 2026-07-03 close): subcommand_inspect / sora の processor 経路移行 + `AsyncVideoDecoder::run` 追加
- closed/0071 (`feature/refactor-mp4-reader-async-video-decoder`、 2026-07-02 close): mp4 reader の spawn pattern 化 + `set_video_decoder` / `discard_video_decoder_output` 削除
- closed/0072 (`feature/refactor-inbound-endpoint-async-video-decoder`、 2026-07-06 close): rtmp / rtsp / srt inbound endpoint の spawn pattern 化

着手時の再確認 grep (本番経路に wrap 使用が復活していないことの確認):

```
grep -rn '\bVideoDecoder\b' src/ tests/ pbt/ fuzz/ examples/
```

`\b` 境界により `AsyncVideoDecoder` / `VideoDecoderOptions` / `VideoDecoderInner` / `VideoDecoderTask` / `VideoDecoderFactory` は hit しない (前後いずれかが単語文字に連続し、 境界が成立しないため)。 期待 hit は次のみ: §現状 (a) (b) 群、 `src/decoder.rs` の wrap 定義本体 (`:585-648`) と `drain_video_decoder_output` のシグネチャ (`:672`)、 コメント (`src/decoder.rs:377, :419, :437, :1116` と `src/decoder/video_toolbox.rs:122`。 後者はリネーム後も新 `VideoDecoder` を指す記述として意味が保たれるため書き換え不要)。 これ以外の新規 hit があれば、 その使用側の移行を先に行う。

## 解決方法

実装手順:

1. 着手条件確認: §依存関係の grep を実施し、 hit が期待どおりであることを確認
2. wrap 型 (`VideoDecoder` + `drain_video_decoder_output`) を削除し、 `AsyncVideoDecoder` → `VideoDecoder` にリネーム
3. メソッドのサフィックス整理 (`_sync` / `_async` 削除) と、 呼出箇所・コメント / docstring の追従 (§リネーム対象)
4. tests の名称と検証意図の整理 (§tests への影響。 二重 harness の一本化を含む)。 手順 2〜4 は単一コミットに収める (§決定事項 2)
5. closed/0057 §3 分割表 (`:350-354` 付近) の更新:
    - 0071 / 0072 / 0073 の 3 行を追加する。 行の形式は既存行に合わせ、 「推定 LOC」列には実績の diff 規模 (各 PR の changed lines) を記載する (0073 自身の行は自 PR 未マージのため working-tree の `git diff --stat` による概算でよい)。 文面案:
      - `| closed/0071 (\`feature/refactor-mp4-reader-async-video-decoder\`) | mp4 reader の video decoder 経路を decoder task (spawn pattern) 化、 \`set_video_decoder\` / \`discard_video_decoder_output\` 削除 | (実績) | 0066 | 内部 API のみ |`
      - `| closed/0072 (\`feature/refactor-inbound-endpoint-async-video-decoder\`) | RTMP / RTSP / SRT inbound endpoint の video decoder 経路を spawn pattern 化 | (実績) | 0066 | 内部 API のみ |`
      - `| closed/0073 (\`feature/refactor-remove-sync-video-decoder-and-rename\`) | 同期 wrap \`VideoDecoder\` 削除 + \`AsyncVideoDecoder\` を \`VideoDecoder\` にリネーム | (実績) | 0068 / 0071 / 0072 | 内部 API のみ |`
    - 0068 行の範囲記述「0066 完了後に各使用側を `AsyncVideoDecoder` に移行 + 最終クリーンアップ (同期 `VideoDecoder` 削除 + `AsyncVideoDecoder` を `VideoDecoder` にリネーム)」を実績 (subcommand_inspect / sora の call site 移行 + `AsyncVideoDecoder::run` 追加) に修正する
    - 既存行・備考の `open/0066` / `open/0068` 表記は `closed/` に更新する (0067 は open のまま維持)。 備考の方針 (δ) 注記 (`:362`) は 2 文とも更新する: (1)「0068 で最終解消する派生」→「0068 / 0071 / 0072 / 0073 で最終解消する派生」、 (2)「0066 + 0068 で採用案 C の長所 5 項目を分担達成」→「0066 + 0068 で長所の大半を分担達成し、 残る (v)『ホップ数上限 1』は 0071 / 0072 / 0073 のクリーンアップ (drain 経路・wrap 型の除去) で最終達成」 (長所 (v) は wrap 削除まで未達のため、 第 2 文をそのまま残すと第 1 文と矛盾する)
    - 分割表直上の「依存順序: `0066 → 0067`」(`:348`) を、 decoder 移行系列を反映する形に更新する。 文面案: `依存順序: 0066 → {0068 / 0071 / 0072} → 0073 (encoder 系列は 0066 → 0067 で独立)`
    - 0067 行 (`:354`) は「依存先」列のセルが欠落した不正な行 (他行は 5 セル、 0067 行のみ 4 セル) になっているため、 備考 (`:360`「0067 は 0066 完了後に着手」) と整合する依存先 `0066` を補って表を整える (0067 は open のままだが、 分割表を触るついでの broken windows 修正)
6. grep 検証: §完了条件の grep 1〜3 がすべて 0 件であることを確認
7. §完了条件のビルド / テストコマンド (cargo fmt / check / clippy / test、 default + `--no-default-features`) を全通過させる

## CHANGES.md について

内部リファクタにつき記載不要。 `VideoDecoder` 系は外部公開していない (hisui の lib target は crates.io 未公開で、 workspace 内の bin / tests 専用)。 型名変更の影響は crate 内のみ。

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): 親 issue、 `AsyncVideoDecoder` を導入し wrap 段階的移行方針 (δ) を確定した。 依存 3 issue (0068 / 0071 / 0072) は §依存関係を参照
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 設計判断の親 issue。 本 issue 完了で採用案 C の長所 (v) が最終達成され、 §3 分割表の更新 (§解決方法 5) で移行系列の記録が閉じる
