# subcommand_inspect と sora の VideoDecoder 使用側を AsyncVideoDecoder に移行する

- Priority: Medium
- Created: 2026-06-29
- Completed: 2026-07-03
- Model: Claude Opus 4.7
- Branch: feature/refactor-migrate-video-decoder-users-to-async
- Polished: 2026-07-02
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0066 で導入された `AsyncVideoDecoder` へ、 使用側を段階的に移行する 4 分割 issue の 1 件目。 本 issue は **`spawn_processor` (もしくは `spawn_processor_task`) 経由で `VideoDecoder::run(handle, in, out)` を呼ぶ 3 ファイル 4 call site の単純置換**を扱う。 pattern が最小規模かつ pattern 確立に適しているため、 他 3 issue (0071 / 0072 / 0073) より先行して着手可能。

なお `AsyncVideoDecoder::run` の追加は closed issue 0057 §3 採用案 C 「callback friendly interface」の一部として closed issue 0066 で意図的に未提供とされた API の補完であり、 refactor スコープ内に収まる (外部公開 API ではなく crate 内内部 API のため、 後方互換性影響なし)。

## 優先度根拠

Medium。

- closed issue 0066 の wrap 段階的移行方針 (δ) を、 closed issue 0057 §3 「中途半端な 2 系統共存を残さない」原則と整合させるには全使用側移行が必要。 本 issue はその出発点 (最も影響範囲が小さく pattern 確立に適する)
- 本 issue で `AsyncVideoDecoder::run` を先行追加する必要があり、 これは open issue 0072 (inbound endpoint) と open issue 0071 (mp4 reader) が spawn クロージャ内で自前構築する場合の参照実装にもなる
- 後続の open issue 0073 (最終クリーンアップ) は本 issue と 0071 / 0072 の全完了を待つ

## 現状

closed issue 0066 完了時点 (2026-07-01 close 済み) の状態:

- `AsyncVideoDecoder` が `src/decoder.rs:385` に新規追加されており、 `pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>>` (`:472`) で非同期にフレームを受け取れる。 同期版として `handle_input_sample_sync` (`:424`) と `poll_output_sync` (`:441`) も提供
- 既存 `VideoDecoder` は内部に `AsyncVideoDecoder` を保持する wrap 構造 (`:542`) に切り替わっており、 出力は内部 channel 経由で受け取るが、 外部 API (`poll_output()` `:598` / `handle_input_sample` `:594` / `handle_input_message` `:586` / `run` `:553` / `get_engines` `:602`) は挙動不変で全使用側が引き続き同期 pull または `run` 経由で動いている
- 各 inner (`Libvpx` / `Openh264` / `Dav1d` / `VideoToolbox` / `Nvcodec`) は 0066 で `OutputSink` (`UnboundedSender<crate::Result<VideoFrame>>` と `total_output_metric: StatsCounter` のペアリング構造体) を内包する形に変更済み

### 本 issue で書き換える対象 (3 ファイル 4 call site)

| # | 対象ファイル | 現状パターン | 位置 | 備考 |
|---|---------------|--------------|------|------|
| 1 | `src/subcommand_inspect.rs` | `spawn_processor` クロージャの**外側**で `let video_decoder = VideoDecoder::new(options, Stats::new())` を作り、 クロージャ内で `video_decoder.run(handle, in, out)` を move キャプチャで呼ぶ | `:215` (`VideoDecoder::new`) / `:228` (`decoder.run(handle, ...)`) | 3 call site と異なり第 2 引数が `handle.stats()` ではなく `crate::stats::Stats::new()`、 クロージャ外構築。 型置換 1 語 (`VideoDecoder` → `AsyncVideoDecoder`) のみで移行可能 |
| 2 | `src/sora/recording_subcommand_compose.rs` | `spawn_processor_task` の move クロージャ内で `let decoder = VideoDecoder::new(decoder_options_for_decoder, handle.stats()); decoder.run(...)` | `:463` (`VideoDecoder::new`) / `:464` (`decoder.run(...)`) | 骨子通り |
| 3-a | `src/sora/recording_subcommand_vmaf.rs` | 同上 | `:362` (`VideoDecoder::new`) / `:363` (`decoder.run(...)`) | 骨子通り |
| 3-b | `src/sora/recording_subcommand_vmaf.rs` | 同上 (2 call site 目、 `decoder_options` は move で単発利用) | `:480` (`VideoDecoder::new`) / `:481` (`decoder.run(...)`) | 骨子通り |

いずれも `spawn_processor` / `spawn_processor_task` の closure 内で `decoder.run(handle, ...)` を返す processor モデル。 processor 抽象 (`MediaPipeline` + `ProcessorHandle` 経由でパイプラインに登録) を維持しつつ decoder 型を差し替える。

### AsyncVideoDecoder に不足している API

closed issue 0066 の完了条件で明記されている通り、 `AsyncVideoDecoder::run` は 0066 では意図的に **未提供**。 本 issue の processor 経路移行には `run` メソッドが不可欠なため、 本 issue で `AsyncVideoDecoder::run` を追加する。 実装骨子は §「AsyncVideoDecoder::run の実装骨子」参照。

なお `AsyncVideoDecoder::handle_input_message` は本 issue では **追加しない**。 wrap 側 `VideoDecoder::handle_input_message` (`:586`) は 0073 で削除される予定で、 `AsyncVideoDecoder::run` の内側で `Message` の 3 variant を自前 dispatch する形にする (実装骨子参照)。 0071 / 0072 の spawn 経路も同様に自前 dispatch で回避可能。

### スコープ外 (別 issue)

- `src/mp4/reader.rs` + `src/obsws/source/file_mp4.rs`: open issue 0071 (`feature/change-mp4-reader-async-video-decoder`)
- `src/rtmp/inbound_endpoint.rs` / `src/rtsp/subscriber.rs` / `src/srt/inbound_endpoint.rs`: open issue 0072 (`feature/refactor-inbound-endpoint-async-video-decoder`)
- 同期 `VideoDecoder` 削除 + `AsyncVideoDecoder` → `VideoDecoder` リネーム: open issue 0073 (`feature/refactor-remove-sync-video-decoder-and-rename`)
- `src/subcommand_list_codecs.rs` の `VideoDecoder::get_engines` 呼出: 本 issue で触らない (0066 で `VideoDecoder::get_engines` は `AsyncVideoDecoder::get_engines` に委譲する薄いラッパになっており、 wrap 削除は 0073)

## 設計方針

### 移行パターン (骨子)

3 ファイル 4 call site すべて、 使用側での変更は **型名の 1 語置換** (`VideoDecoder` → `AsyncVideoDecoder`) のみ。 `AsyncVideoDecoder::new` のシグネチャを wrap 側 `VideoDecoder::new` と同一 (`fn new(options: VideoDecoderOptions, stats: Stats) -> Self`) に保つことで実現する。

3 ファイルとも既存の import (`decoder::{..., VideoDecoder, ...}`) の `VideoDecoder` を `AsyncVideoDecoder` に置換すれば追加 import は不要:

- `src/subcommand_inspect.rs:11`: `use crate::{... decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions} ...}` の `VideoDecoder` を置換
- `src/sora/recording_subcommand_compose.rs:14`: 同上
- `src/sora/recording_subcommand_vmaf.rs:14`: 同上

いずれのファイルも `VideoDecoder` の他利用箇所はなく (`grep -n '\bVideoDecoder\b'` で確認済み)、 型置換のみで完結する。

### AsyncVideoDecoder::run の実装骨子

現状 wrap 側 `VideoDecoder::run` (`src/decoder.rs:553-584`) の骨子:

```rust
pub async fn run(
    mut self,
    handle: ProcessorHandle,
    input_track_id: TrackId,
    output_track_id: TrackId,
) -> Result<()> {
    let mut input_rx = handle.subscribe_track(input_track_id);
    let mut output_tx = handle.publish_track(output_track_id).await?;
    handle.notify_ready();
    handle.wait_subscribers_ready().await?;

    loop {
        let message = input_rx.recv().await;
        let is_eos = matches!(message, Message::Eos);

        // 入力を inner に渡す
        self.handle_input_message(message)?;

        // 出力を drain
        match drain_video_decoder_output(&mut self, &mut output_tx)? {
            DrainResult::PipelineClosed | DrainResult::Finished => {
                output_tx.send_eos();
                break;
            }
            DrainResult::Pending => {}
        }

        if is_eos {
            return Err(Error::new("video decoder still pending after EOS"));
        }
    }

    Ok(())
}
```

`AsyncVideoDecoder::run` はこれを移植し、 wrap 側 `handle_input_message` (0073 で削除予定) を自前 dispatch に、 `drain_video_decoder_output` (0073 で削除予定) を `poll_output_sync` の内側 loop に置き換える。 骨子:

```rust
impl AsyncVideoDecoder {
    pub async fn run(
        mut self,
        handle: ProcessorHandle,
        input_track_id: TrackId,
        output_track_id: TrackId,
    ) -> Result<()> {
        let mut input_rx = handle.subscribe_track(input_track_id);
        let mut output_tx = handle.publish_track(output_track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        loop {
            let message = input_rx.recv().await;
            let is_eos = matches!(message, Message::Eos);

            // wrap 側 VideoDecoder::handle_input_message (:586) と同等の dispatch を自前展開する。
            // AsyncVideoDecoder::handle_input_message は本 issue では追加しない (0073 で wrap 側が消える)。
            // Syn は decoder が track 同期に関与しないため無視する (wrap 側と同挙動)。
            match message {
                Message::Media(sample) => self.handle_input_sample_sync(Some(sample))?,
                Message::Eos => self.handle_input_sample_sync(None)?,
                Message::Syn(_) => {}
            }

            // 内部 channel に溜まった分をすべて吐き出す (try_recv ベースで非ブロッキング)。
            // Openh264 は 1 サンプル入力で 0〜2 frame 出力する (closed/0066 §現状 参照) ため、
            // Pending / Finished に達するまで内側で loop する必要がある。 1 回だけ
            // next_decoded_frame_async().await 相当を呼ぶ形にすると keyframe 前後の flush 由来
            // frame がロスする。
            loop {
                match self.poll_output_sync()? {
                    DecoderRunOutput::Processed(sample) => {
                        // send_media が false = subscriber 全 drop = pipeline closed。 続く send_eos は
                        // 実質 no-op (subscriber がいない) で wrap 側 DrainResult::PipelineClosed 経路と等価。
                        if !output_tx.send_media(sample) {
                            output_tx.send_eos();
                            return Ok(());
                        }
                    }
                    DecoderRunOutput::Pending => break,
                    DecoderRunOutput::Finished => {
                        output_tx.send_eos();
                        return Ok(());
                    }
                }
            }

            // wrap 側と同じ防御コード。 handle_input_sample_sync(None) で self.eos = true にした後、
            // 上の内側 loop で poll_output_sync は必ず Finished を返して return する不変条件 (Nvcodec
            // でも finish() が全 callback を同期完了する契約、 nvcodec.rs:228-234 参照) のため、
            // この Err 分岐は AsyncVideoDecoder の構造上到達不能。 実装者が意味を誤解して
            // 削除しないよう、 wrap 側 (:579) と挙動を揃えて残す。
            if is_eos {
                return Err(Error::new("video decoder still pending after EOS"));
            }
        }
    }
}
```

`poll_output_sync` は 0066 で `try_recv` ベースの実装として存在するので、 これを流用する (`decoder.rs:441-461`)。 内側 loop は非ブロッキングで、 tokio yield は wrap 側 `drain_video_decoder_output` と同じく挟まない (毎 iteration の `output_tx.send_media` が実質的な yield 機会となる)。

### 骨子と wrap 版の行動等価性

上記骨子は wrap 版 `VideoDecoder::run` (`:553-584`) + `drain_video_decoder_output` (`:628-647`) と行動等価である。 根拠:

1. `Message` の 3 variant 処理は wrap 側 `handle_input_message` (`:586-592`) の dispatch と等価
2. `Processed` の `send_media` false 分岐は wrap 側 `DrainResult::PipelineClosed` 経路と同じ `send_eos` を呼ぶ (実質 no-op) + return
3. `Finished` は wrap 側と同じく `send_eos` + return
4. Err 経路は `?` propagation で wrap 側と同一 (エラー時に `send_eos()` は呼ばない = 下流の pipeline 側で error 伝播により察知)
5. EOS 後 Pending の Err 文言 (`"video decoder still pending after EOS"`) は wrap 側 (`:579`) と完全一致

### 決定事項 (実装で覆さない)

- 各 call site の processor モデル (`spawn_processor` / `spawn_processor_task` + move クロージャ) は維持
- `AsyncVideoDecoder::run` のシグネチャは wrap 側 `VideoDecoder::run` と同一 (`(self, ProcessorHandle, TrackId, TrackId) -> Result<()>`)
- 内部で `poll_output_sync` を利用する (`try_recv` ベース、 tokio runtime 依存最小)
- `AsyncVideoDecoder::handle_input_message` は追加しない (`run` の内側で自前 dispatch)
- `AsyncVideoDecoder` の discard 系ヘルパは本 issue では追加しない (0071 で必要になった時点で追加する)
- Nvcodec feature 有効時と無効時で `AsyncVideoDecoder::run` の挙動差分はない (`poll_output_sync` が同期・非同期 inner の両方を透過的に扱う)
- 0071 / 0072 完了まで wrap 経由 (mp4 reader / inbound) と直接経路 (本 issue の 4 call site) が並走するため、 `AsyncVideoDecoder::run` は wrap 側 `VideoDecoder::run` と挙動完全一致を維持する (Err 時の `send_eos` 有無 / Finished 到達で `send_eos` + return / EOS 後 Pending で "still pending" Err、 いずれも wrap と同一)

### テスト戦略

`AsyncVideoDecoder::run` は wrap 側 `VideoDecoder::run` に既存単体 test がなく (実装が実 pipeline 前提)、 本 issue でも e2e で担保する方針を継続する。 具体的には:

- `tests/e2e.rs` の `inspect --decode` シナリオが `src/subcommand_inspect.rs:215` の call site を実行して回帰検出
- `tests/e2e.rs` の `compose` シナリオが `src/sora/recording_subcommand_compose.rs:463` の call site を実行
- `tests/e2e.rs` の `vmaf` シナリオが `src/sora/recording_subcommand_vmaf.rs:362, :480` の 2 call site を実行

`poll_output_sync` の 3 分岐 (`Processed` / `Pending` / `Finished`) と `emit_err` 経由の Err 分岐は既に unit test 済み (`decoder.rs:1082-1143`) なので、 `AsyncVideoDecoder::run` は分岐選択の薄いラッパとみなせる。 doctest は不要 (call site そのものが `spawn_processor` の使用側 example)。

## 完了条件

- `AsyncVideoDecoder::run(self, ProcessorHandle, TrackId, TrackId) -> impl Future<Output = Result<()>>` が `src/decoder.rs` に追加されている
- `src/subcommand_inspect.rs:215` の `VideoDecoder::new` が `AsyncVideoDecoder::new` に置換され、 use 文の `VideoDecoder` が `AsyncVideoDecoder` に置換されている (call site `:228` の `decoder.run(...)` はそのまま動く)
- `src/sora/recording_subcommand_compose.rs:463` の `VideoDecoder::new` が `AsyncVideoDecoder::new` に置換され、 use 文も置換されている
- `src/sora/recording_subcommand_vmaf.rs:362, :480` の 2 箇所とも `VideoDecoder::new` が `AsyncVideoDecoder::new` に置換され、 use 文も置換されている
- 既存 e2e テスト (`tests/e2e.rs` の `inspect --decode` / `compose` / `vmaf` シナリオ) が本 issue 実装後も通る (`AsyncVideoDecoder::run` の 3 経路: 正常 Finished、 PipelineClosed、 Err のいずれかを既存 e2e が走行させる想定)
- `git diff --stat develop -- src/mp4/ src/rtsp/ src/srt/ src/rtmp/ src/obsws/ src/subcommand_list_codecs.rs src/decoder/` がゼロ行 (本 issue のスコープ外ファイルには一切触れない、 各 inner ファイルも変更しない)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

実装着手時の推奨手順:

1. `src/decoder.rs` に `AsyncVideoDecoder::run` を追加 (設計方針 §「AsyncVideoDecoder::run の実装骨子」参照)。 新規 `impl AsyncVideoDecoder` ブロックを既存 (`:399-536`) の直後に追加、 または既存ブロックに追記のいずれでも可 (Rust は複数 `impl` ブロック並立を許容)。 この時点で wrap 側 `VideoDecoder::run` は残っているため `cargo check` は通る
2. `src/subcommand_inspect.rs` の use 文と `:215` の `VideoDecoder::new` を `AsyncVideoDecoder::new` に置換
3. `src/sora/recording_subcommand_compose.rs` の use 文と `:463` を同様に置換
4. `src/sora/recording_subcommand_vmaf.rs` の use 文と `:362, :480` を同様に置換 (2 箇所)
5. 完了条件の全 cargo コマンド (`fmt --check` / `check` (default + `--no-default-features`) / `clippy` (default + `--no-default-features`) / `test` (default + `--no-default-features`)) を通す

各 step で `cargo check` を通せる中間状態を保つ。

## CHANGES.md について

内部リファクタにつき記載不要。 `VideoDecoder` 系は library として外部公開していない (hisui は bin crate)。 API 変更の後方互換影響は crate 内の subcommand_inspect / sora recording subcommand のみで、 外部プロトコル / 出力は不変。

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): 親 issue。 `AsyncVideoDecoder` を導入し `VideoDecoder` を wrap 構造に切り替えた
- open/0071 (`feature/change-mp4-reader-async-video-decoder`): 兄弟 issue。 `src/mp4/reader.rs` + `src/obsws/source/file_mp4.rs` の async 化を扱う。 本 issue と互いに独立で並行実施可能
- open/0072 (`feature/refactor-inbound-endpoint-async-video-decoder`): 兄弟 issue。 rtmp / rtsp / srt inbound endpoint の spawn pattern 化を扱う。 本 issue と互いに独立で並行実施可能。 本 issue で確立する `AsyncVideoDecoder::run` は 0072 の spawn クロージャの参照実装になる
- open/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`): 最終クリーンアップ。 同期 `VideoDecoder` 削除と `AsyncVideoDecoder` → `VideoDecoder` リネーム。 本 issue と 0071 / 0072 の全完了を待つ
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 設計判断の親 issue。 採用案 C 「中途半端な 2 系統共存を残さない」原則との整合は 0073 で最終達成される
