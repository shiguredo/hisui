# 同期 VideoDecoder wrap を削除して AsyncVideoDecoder を VideoDecoder にリネームする

- Priority: Medium
- Created: 2026-07-02
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-remove-sync-video-decoder-and-rename
- Polished:

## 目的

closed issue 0066 で導入された「AsyncVideoDecoder 新規追加 + 同期 VideoDecoder は wrap 構造で挙動維持」 の段階的移行方針 (δ) は、 全使用側が `AsyncVideoDecoder` に切り替わった時点で「同期 wrap を削除し、 `AsyncVideoDecoder` を `VideoDecoder` にリネームする」ことで最終形に到達する。 本 issue はその最終ステップを扱う。

closed issue 0057 §3 採用案 C の長所 (v) 「callback friendly 定義 (ホップ数上限 1) を真に満たす」は、 wrap 経由の 2 段ホップ (`VideoDecoder::poll_output` → `AsyncVideoDecoder::poll_output_sync`) を残したままでは達成できない。 本 issue で wrap 型を消し、 `AsyncVideoDecoder` を `VideoDecoder` にリネームすることで、 使用側の型名も混乱なく最終形に落ち着く。

## 優先度根拠

Medium。

- closed issue 0057 §3 の 「中途半端な 2 系統共存を残さない」原則との最終整合は本 issue でしか達成できない
- 本 issue 単独では外部挙動は不変。 内部型名の整理のため緊急性は低い
- ただし wrap 状態のまま長期間放置すると「AsyncVideoDecoder と VideoDecoder のどちらを使うべきか」の API 選択の負債が蓄積する
- 依存する 3 issue (open/0068, open/0071, open/0072) がすべて完了しない限り本 issue は実施できない

## 現状

closed issue 0066 完了時点 (2026-07-01 close、 develop merge 済み) の `src/decoder.rs` の構造:

```rust
// AsyncVideoDecoder: 新規追加 (0066)
pub struct AsyncVideoDecoder {
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    eos: bool,
    inner: VideoDecoderInner,
    output_rx: DecoderOutputReceiver,
}

impl AsyncVideoDecoder {
    pub fn new(options: VideoDecoderOptions, mut compose_stats: crate::stats::Stats) -> Self { ... }
    pub fn handle_input_sample_sync(&mut self, sample: Option<MediaFrame>) -> Result<()> { ... }
    pub fn poll_output_sync(&mut self) -> Result<DecoderRunOutput> { ... }
    pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>> { ... }
    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> { ... }
}

// VideoDecoder: 既存の同期 wrap (0066 で内部を AsyncVideoDecoder ベースに移行、 外部 API 不変)
pub struct VideoDecoder {
    inner_decoder: AsyncVideoDecoder,
}

impl VideoDecoder {
    pub fn new(options, compose_stats) -> Self { ... }
    pub async fn run(mut self, handle, input_track_id, output_track_id) -> Result<()> { ... }
    pub fn handle_input_message(&mut self, message: Message) -> Result<()> { ... }
    pub fn handle_input_sample(&mut self, sample: Option<MediaFrame>) -> Result<()> { ... }
    pub fn poll_output(&mut self) -> Result<DecoderRunOutput> { ... }
    pub fn get_engines(...) -> Vec<EngineName> { ... }
}

pub fn drain_video_decoder_output(decoder: &mut VideoDecoder, output_tx: &mut TrackPublisher) -> Result<DrainResult> { ... }
```

本 issue 着手時点では、 0068 / 0071 / 0072 の完了により、 crate 内すべての使用側が `AsyncVideoDecoder` ベース (もしくは 0068 で `AsyncVideoDecoder::run` などの新 API を経由) に切り替わっている状態を前提とする。

## 設計方針

### 削除対象 (`src/decoder.rs`)

- `pub struct VideoDecoder` (wrap 型、 `:542` 付近)
- `impl VideoDecoder` の全メソッド:
  - `new` (`:547`)
  - `run` (`:553`)
  - `handle_input_message` (`:586`)
  - `handle_input_sample` (`:594`)
  - `poll_output` (`:598`)
  - `get_engines` (`:602`)
- `pub fn drain_video_decoder_output` (`:628`)

### 削除対象 (`src/mp4/reader.rs`)

- `Mp4FileReader::set_video_decoder` (`:318`)
- `fn discard_video_decoder_output` (`:1388` の module-private helper)

これらは open/0071 で削除される予定だが、 本 issue 着手時点で残っていれば同時削除する。

### リネーム対象

- `pub struct AsyncVideoDecoder` → `pub struct VideoDecoder` (`src/decoder.rs:385` 付近)
- 関連メソッドのサフィックス整理:
  - `handle_input_sample_sync` → `handle_input_sample` (`_sync` サフィックス削除、 元の wrap 側 API 名を継承)
  - `poll_output_sync` → `poll_output` (同上)
  - `next_decoded_frame_async` → `next_decoded_frame` (`_async` サフィックス削除)
- 型エイリアス:
  - `DecoderOutputSender` / `DecoderOutputReceiver`: そのまま維持 (すでに整理された型名)
  - `OutputSink`: そのまま維持
- 全参照箇所の型名更新: `src/` / `tests/` / `pbt/` / `examples/` / `fuzz/` 配下すべて。 `grep -rn '\bAsyncVideoDecoder\b'` で検出できる範囲

### tests への影響

- `tests/decoder_tests.rs` (0066 で追加された 3 テスト):
  - `async_video_decoder_processes_real_vp9_frame_via_next_decoded_frame_async`: 名前から `async_video_decoder_` prefix と `_async` サフィックスを削除し、 内部呼出 API 名も追従
  - `video_decoder_poll_output_returns_processed_via_wrap_delegation`: wrap 廃止に伴い `_via_wrap_delegation` サフィックスを削除、 純粋な `VideoDecoder::poll_output` 検証テストに書換
  - `video_decoder_metrics_increment_by_input_count_via_wrap_delegation`: 同上、 `_via_wrap_delegation` サフィックス削除
- `src/decoder.rs` の `#[cfg(test)] mod tests`:
  - `poll_output_sync_returns_finished_when_eos_and_channel_empty` などの `_sync` サフィックスを削除
  - `poll_output_sync_returns_pending_when_not_eos_and_channel_empty` 等も同様
  - `poll_output_sync_returns_err_when_emit_err_received` も同様
- `tests/e2e.rs`: 0066 で `LibvpxDecoder` 直叩き経路に整理済み。 `AsyncVideoDecoder` 参照が残っていれば追従
- `pbt/` / `fuzz/` / `examples/` 配下は要 grep 確認

### 未確定論点 (polish で確定させる想定)

1. **メソッド命名の詳細**
    - `_sync` / `_async` サフィックスをすべて削除するか、 一部残すか
    - 例: `next_decoded_frame_async` は async であることをシグネチャ (`async fn`) から読み取れるがメソッド名にも残す慣習もある
    - 例: `handle_input_sample` は元の wrap 側 API 名と同一なので `_sync` 削除は妥当
2. **リネーム実施順序**
    - option A: 旧 `VideoDecoder` (wrap) を先に削除 → 名前空間解放 → `AsyncVideoDecoder` → `VideoDecoder` にリネーム
    - option B: `AsyncVideoDecoder` を別名 (例: `VideoDecoderV2`) に一時退避 → 旧削除 → `VideoDecoderV2` → `VideoDecoder` にリネーム
    - option A は 1 段階、 option B は 2 段階だが中間状態で cargo check を通せる
3. **完了検証手段**
    - `grep -rn '\bAsyncVideoDecoder\b' src/ tests/ pbt/ fuzz/ examples/` の 0 件を CI にどう組み込むか (現状 CI にこの種の grep チェックはない)
4. **spawn pattern 抽象の共通化**
    - 0068 (subcommand_inspect / sora)、 0071 (mp4 reader)、 0072 (inbound endpoint) の 3 使用側で、 spawn pattern (task 生成 / input channel / shutdown / TrackSender 回収 / panic 伝搬) の共通化余地が生じている
    - 0071 実装で `struct VideoDecoderTask` + `enum DecoderInput { Media, Eos }` + `spawn_video_decoder_task(options, stats, sender)` + `video_decoder_loop` を mp4 reader 内 module-private で導入 (`src/mp4/reader.rs`)。 shutdown 経由の `(TrackSender, Result<()>)` 回収 / discard_mode 制御 / panic の error! log + Err 伝搬まで実装済み
    - option a: 本 issue のリネームと合わせて **汎用 `VideoDecoderTask` core** を `crate::decoder` mod 直下に切り出す。 mp4 特有の `discard_mode_tx` (warm-up 経路制御) は上位 layer (`Mp4VideoDecoderTask` 相当) で扱い、 inbound endpoint / 0068 の subcommand_inspect 系は core をそのまま利用。 その際 0068 で追加した `VideoDecoder::run` (旧 `AsyncVideoDecoder::run`、 processor モデル用) は Task ベースに書き直して廃止する検討をする
    - option b: 共通化しない。 0068 の `run` は残し、 mp4/reader.rs / 0072 inbound endpoint はそれぞれ独自実装のまま維持する
    - option c: decoder を別 processor として MediaPipeline に登録する形 (0066 以前および 0068 の processor モデル相当) に戻す。 使用側の spawn pattern 重複が解消されて全体設計の一貫性が上がる (`tokio::spawn` は MediaPipeline が担い、 lifecycle 管理も pipeline 側に集約される)。 ただし現状の pipeline は subscribe / `notify_ready` の multi-hop 伝播 (mp4 reader → decoder processor → mixer などの chain で subscribe_ready が段階的に伝わる仕組み) を想定しておらず、 race condition が煩雑だった経緯がある。 別 issue で pipeline 側の multi-hop 対応 (subscribe chain の ready 伝播、 shutdown 順序保証) を先に実装する必要があり、 本 issue 単独では完結しない
    - 判定材料: (1) 0072 完了時点で mp4/reader.rs (`VideoDecoderTask` + `video_decoder_loop`) と inbound endpoint 側の実装を突き合わせて「本当に共通で使える範囲」を確認する、 (2) MediaPipeline の multi-hop 対応の実現可能性と実装コストを見積もる (option c を選ぶ前提条件)
    - 実装コスト: option a は core 抽出 + mp4 layer 化 + 0068 の run 書換の 3 段階、 差分規模は core 約 100 行 + 使用側書換。 option c は pipeline 側の改修が別 issue として先行するため本 issue の範囲では実質困難

### shiguredo-rust 規約整合

- モック / スタブ不使用 (テストは実 decoder + tokio channel + 実 pipeline)
- `#[non_exhaustive]` 不使用
- 新規 trait 追加なし

## 完了条件

- `pub struct VideoDecoder` (wrap 型) が `src/decoder.rs` から削除されている
- `drain_video_decoder_output` / `discard_video_decoder_output` / `Mp4FileReader::set_video_decoder` が (残っていれば) 削除されている
- `pub struct AsyncVideoDecoder` が `pub struct VideoDecoder` にリネームされている
- メソッドの `_sync` / `_async` サフィックスが確定方針 (§未確定論点 1) で整理されている
- `grep -rn '\bAsyncVideoDecoder\b' src/ tests/ pbt/ fuzz/ examples/` の hit が 0 件
- `grep -rn '\bVideoDecoder\b' src/ tests/ pbt/ fuzz/ examples/` の hit が新 `VideoDecoder` (旧 `AsyncVideoDecoder`) の参照のみ
- closed issue 0057 §3 採用案 C の長所 (v) 「callback friendly 定義 (ホップ数上限 1)」が全使用側で達成されている (wrap 経由の 2 段ホップが grep で残っていないことを確認)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 依存関係

本 issue は以下の 3 issue すべてが完了 (`closed/` に移動) してから着手する:

- open/0068 (`feature/refactor-migrate-video-decoder-users-to-async`): subcommand_inspect / sora の 2 ファイル
- open/0071 (`feature/change-mp4-reader-async-video-decoder`): mp4/reader.rs + obsws/source/file_mp4.rs
- open/0072 (`feature/refactor-inbound-endpoint-async-video-decoder`): rtmp / rtsp / srt inbound endpoint

本 issue 着手時点で同期 `VideoDecoder` (wrap) の使用箇所がゼロであることを次のコマンドで確認する:

```
grep -rn 'crate::decoder::VideoDecoder\b\|decoder::VideoDecoder\b' src/ tests/ pbt/ fuzz/ examples/
```

hit が `AsyncVideoDecoder` の参照のみに絞られていれば着手条件を満たす。

## 解決方法

実装着手時の推奨手順:

1. 着手条件確認: 上記 grep コマンドで同期 `VideoDecoder` (wrap) の参照が 0 件であることを確認
2. 設計方針 §「未確定論点」の 3 論点を polish で確定
3. リネーム順序 (§未確定論点 2) に従って:
    - option A の場合: 旧 wrap 型と関連 API を削除 (`cargo check` が壊れる) → `AsyncVideoDecoder` → `VideoDecoder` にリネーム (`cargo check` 復活)
    - option B の場合: `AsyncVideoDecoder` → 一時名にリネーム → 旧削除 → 一時名 → `VideoDecoder` にリネーム
4. メソッドのサフィックス整理 (`_sync` / `_async` 削除)
5. tests の名称と検証意図の整理
6. grep 検証: `AsyncVideoDecoder` の hit が 0 件、 `VideoDecoder` の hit が新型のみ
7. `cargo fmt` / `cargo check` (default + `--no-default-features`) / `cargo clippy` / `cargo test` 全通過

## CHANGES.md について

内部リファクタにつき記載不要。 `VideoDecoder` 系は library として外部公開していない (hisui は bin crate)。 型名変更の影響は crate 内のみ。

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): 親 issue、 `AsyncVideoDecoder` を導入し wrap 段階的移行方針 (δ) を確定した
- open/0068 (`feature/refactor-migrate-video-decoder-users-to-async`): 単純 call site 置換 3 ファイル。 本 issue の依存
- open/0071 (`feature/change-mp4-reader-async-video-decoder`): mp4 reader async 化。 本 issue の依存
- open/0072 (`feature/refactor-inbound-endpoint-async-video-decoder`): inbound endpoint spawn pattern 化。 本 issue の依存
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 設計判断の親 issue。 本 issue 完了で採用案 C の長所 (v) が最終達成される
