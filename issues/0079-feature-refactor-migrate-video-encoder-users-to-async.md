# encoder 使用側を AsyncVideoEncoder に移行して AsyncVideoEncoder::run を追加する

- Priority: Medium
- Created: 2026-07-07
- Completed: {YYYY-MM-DD}
- Model: Claude Opus 4.7
- Branch: feature/refactor-migrate-video-encoder-users-to-async
- Polished: 2026-07-08
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0066 で確立され closed issue 0067 で encoder 側に展開された派生方針 (δ) 「`Async*` 新規追加 + 既存を wrap 化 + 段階移行」の後続として、 本番使用側 (compose / vmaf / list_codecs、 および `create_video_processor(_with_params)` 内の呼出) を wrap `VideoEncoder` から `AsyncVideoEncoder` 直接利用に移行する。 同時に processor モデル用の駆動 API として `AsyncVideoEncoder::run` を新規追加する。 obsws 3 site (`output.rs` / `output_dash.rs` / `output_hls.rs`) は `create_video_processor(_with_params)` の pub シグネチャを維持することで無変更で通す (間接経路)。

closed issue 0068 (decoder 側の対称 issue) と同じパターンで実施する。 本 issue 完了で、 wrap `VideoEncoder` の本番使用側 (直呼出) がゼロになり、 後続の wrap 削除 + rename issue の下地が整う。 closed/0057 §3 採用案 C 「中途半端な 2 系統共存を残さない」原則の最終達成に向けた段階移行の 1 ステップ。

なお `AsyncVideoEncoder::run` の追加は closed issue 0057 §3 採用案 C 「callback friendly interface」の一部として closed issue 0067 で意図的に未提供とされた API の補完であり、 refactor スコープ内に収まる (外部公開 API ではなく crate 内内部 API のため、 後方互換性影響なし)。

## 優先度根拠

Medium。

- (δ) 方針の後続として最も影響範囲が小さく、 pattern 確立に適する
- 本 issue 完了で後続 (wrap 削除 + rename refactor issue / 未使用 API 削除 refactor issue) の下地が整う
- Priority は decoder 系列 0068 と対称 (Medium 維持)
- 依存先 closed/0067 は 2026-07-08 に develop merge 済み (詳細は §依存関係)
- open/0080 (`NvcodecEncoder::flush()` 撤廃 + bp 機構) とは相互独立で、 どちらから着手してもよい (0080 側で明言済み、 本 issue 側でも 0080 完了は前提としない)

## 現状

closed issue 0067 完了時点 (2026-07-08 develop merge、 commit `7b5f2740`) の `src/encoder.rs` (1416 行) の構造を基準とする。 主要要素:

- `pub struct AsyncVideoEncoder` (`src/encoder.rs:476`) と `impl AsyncVideoEncoder` (`:502-756`) が存在し、 以下を提供:
  - `pub fn new(options, openh264_lib, compose_stats) -> Result<Self>` (`:503`)
  - `pub fn get_engines(codec, is_openh264_available) -> Vec<EngineName>` (`:639`)
  - `pub async fn next_encoded_frame_async(&mut self) -> Option<Result<VideoFrame>>` (`:753`)
  - `pub(crate) fn handle_rpc_message_sync(&mut self, VideoEncoderRpcMessage)` (`:686`)
  - `pub(crate) fn handle_input_sample_sync(&mut self, Option<MediaFrame>) -> Result<()>` (`:700`)
  - `pub(crate) fn poll_output_sync(&mut self) -> Result<EncoderRunOutput>` (`:729`)
  - `pub fn name(&self) -> Option<EngineName>` / `pub fn codec(&self) -> Option<CodecName>` (`:631, :635`)
- `pub struct VideoEncoder { inner_encoder: AsyncVideoEncoder }` (`:763`) が wrap 型として存在。 全 pub メソッドが `AsyncVideoEncoder` への delegate:
  - `new` (`:768`) / `name` (`:778`) / `codec` (`:782`) / `get_engines` (`:786`) / `run` (`:790-836`) / `handle_input_message` (`:842`) / `handle_input_sample` (`:850`) / `poll_output` (`:854`)
- wrap 側 helper: `drain_video_encoder_output(&mut VideoEncoder, &mut TrackPublisher) -> Result<bool>` (`:859`) と `recv_video_encoder_rpc_message_or_pending(Option<&mut UnboundedReceiver<VideoEncoderRpcMessage>>) -> Option<VideoEncoderRpcMessage>` (`:880`)
- inner (`VideoEncoderInner` enum / `LibvpxEncoder` / `Openh264Encoder` / `SvtAv1Encoder` / `VideoToolboxEncoder` / `NvcodecEncoder`) は 0067 で `OutputSink` (`:382`) を受け取る Sender 化済み。 本 issue では触らない

### 本 issue で書き換える対象 (直呼出 4 hit)

`rg '\bVideoEncoder::(new|run|get_engines)\b' src/` の hit は現在 4 件 (いずれも本 issue で `AsyncVideoEncoder` へ切り替え):

| # | 対象ファイル | 現状パターン | 位置 |
|---|---------------|--------------|------|
| 1 | `src/sora/recording_subcommand_compose.rs` | `spawn_processor_task` の move クロージャ内で `VideoEncoder::new(&video_encoder_options, openh264_lib_for_encoder, handle.stats())?` → `encoder.run(handle, in, out).await` | `:577` (`VideoEncoder::new`)、 use 文は `:15` |
| 2 | `src/sora/recording_subcommand_vmaf.rs` | 同上パターン | `:456` (`VideoEncoder::new`)、 use 文は `:15` |
| 3 | `src/encoder.rs` (`create_video_processor_with_params` 内) | `spawn_processor` の move クロージャ内で `VideoEncoder::new(&options, h.config().openh264_lib.clone(), h.stats())?` → `encoder.run(h, in, out).await` | `:1164` (定義は `:1137-1171`) |
| 4 | `src/subcommand_list_codecs.rs` | `VideoEncoder::get_engines(name, is_openh264_available)` の 1 行呼出 | `:88`、 use 文は `:7` |

### obsws 経路の間接呼出 3 site (本 issue では **書き換えない**)

以下 3 site は `create_video_processor` / `create_video_processor_with_params` (`src/encoder.rs:1109-1171`) 経由の間接呼出で、 本 issue では pub シグネチャを変えないため使用側は無変更:

- `src/obsws/coordinator/output.rs:718`
- `src/obsws/coordinator/output_dash.rs:894`
- `src/obsws/coordinator/output_hls.rs:911`

上記 3 site は `create_video_processor` / `create_video_processor_with_params` を呼ぶだけで `VideoEncoder` 型を直接名前で参照しない (`rg 'VideoEncoder' src/obsws/coordinator/output*.rs` の hit は 0 件、 `VideoEncoderOptions` などの派生名も未参照)。

### tests/encoder_tests.rs (wrap 型テスト、 本 issue では **書き換えない**)

`tests/encoder_tests.rs` の 3 テスト (`#[test]` 行で `:56` / `:92` / `:152`、 内部で `VideoEncoder::new` を呼ぶ位置は `:60` / `:104` / `:161`) は closed/0067 で追加された wrap 側 pub API の integration test。 本 issue では wrap 型 `VideoEncoder` の定義を残すため、 これらのテストも wrap 型のテストとして据え置く。 wrap 削除 + rename issue (未起票、 closed/0073 相当) で wrap 型と一緒に切替える。 完了条件の grep 検証もこの方針で `src/` に限定する。

## 設計方針

closed issue 0068 (decoder 側の対称 issue) の設計方針を encoder 側の RPC 経路を加味して展開する。

### `AsyncVideoEncoder::run` の実装骨子

wrap 側 `VideoEncoder::run` (`src/encoder.rs:790-836`) の 2 腕 `tokio::select!` (入力 + RPC) を、 wrap を介さず `AsyncVideoEncoder` 自身のフィールドと `_sync` API (`handle_input_sample_sync` / `poll_output_sync` / `handle_rpc_message_sync`) を直接呼び出す形に書き直す。 helper `recv_video_encoder_rpc_message_or_pending` (`:880`) は wrap 側と共有する (再定義しない)。 `drain_video_encoder_output` (`:859`) は `&mut VideoEncoder` シグネチャのため呼べず、 `poll_output_sync` の内側 loop に inline 展開する。

RPC keyframe 経路の下流呼出元 (`request_upstream_video_keyframe` (`src/encoder.rs:416`) → `get_rpc_sender::<UnboundedSender<VideoEncoderRpcMessage>>`) は現時点で 6 site (`sora_publisher.rs:93` / `mp4/hybrid_writer.rs:783` / `mp4/writer.rs:859` / `rtmp/outbound_endpoint.rs:628` / `hls/writer.rs:438` / `dash/writer.rs:377`)。 `register_rpc_sender` を削ると上流の `request_upstream_video_keyframe` が Err を返し、 各下流は `tracing::warn!` でログを残して continue する (`sora_publisher.rs:100` / `rtmp/outbound_endpoint.rs:635` 等)。 挙動としては「警告ログには残るが keyframe は届かない」degrade で、 完全 silent ではないが実運用では警告が埋もれて気付かない可能性がある。

以下は既存 `impl AsyncVideoEncoder` (`:502-756`) の末尾 (`next_encoded_frame_async` (`:753-755`) 直後、 impl 閉じ `}` (`:756`) 直前) に追加する **メソッド本体**。 コピペ時に外側の `impl AsyncVideoEncoder { ... }` を含めないこと (既存 impl を分割してしまう):

```rust
/// processor モデル (`ProcessorHandle` + subscribe / publish) で `AsyncVideoEncoder` を
/// 駆動する。 wrap 側 `VideoEncoder::run` の 2 腕 `tokio::select!` (入力 + RPC) を
/// wrap を介さず自身の `_sync` API (`handle_input_sample_sync` / `poll_output_sync` /
/// `handle_rpc_message_sync`) を直接呼ぶ形に書き直したもの。 挙動は wrap 側と完全一致。
pub async fn run(
    mut self,
    handle: ProcessorHandle,
    input_track_id: TrackId,
    output_track_id: TrackId,
) -> Result<()> {
    let mut input_rx = handle.subscribe_track(input_track_id);
    let mut output_tx = handle.publish_track(output_track_id).await?;
    // register_rpc_sender は subscribe / publish の後、 notify_ready / wait_subscribers_ready
    // の前に呼ぶ (wrap 側 :798-804 と同順序)。 削ると上流 `request_upstream_video_keyframe` が
    // Err を返して各下流 (sora_publisher / mp4/hybrid_writer / mp4/writer / rtmp/outbound_endpoint
    // / hls/writer / dash/writer の 6 site) で warn ログ + keyframe 未到達 degrade になる。
    // RegisterProcessorRpcSenderError は crate::Error への From 実装がないため .map_err で
    // 明示的に変換する (PublishTrackError / PipelineTerminated は From 実装があるので ? のみで足りる)。
    let (rpc_tx, mut rpc_rx) = tokio::sync::mpsc::unbounded_channel();
    handle
        .register_rpc_sender(rpc_tx)
        .await
        .map_err(|e| Error::new(format!("failed to register video encoder RPC sender: {e}")))?;
    handle.notify_ready();
    handle.wait_subscribers_ready().await?;
    let mut rpc_rx_enabled = true;

    loop {
        tokio::select! {
            message = input_rx.recv() => {
                let is_eos = matches!(message, Message::Eos);
                match message {
                    Message::Media(sample) => self.handle_input_sample_sync(Some(sample))?,
                    Message::Eos => self.handle_input_sample_sync(None)?,
                    // Syn は末端到達確認用の制御メッセージ (`media_pipeline.rs::Syn`)。
                    // encoder は track 同期に関与しないため、 何もせず drop するのみ (下流に転送しない)。
                    // drop 契機で `Syn` 内部の `Sender<()>` が破棄され、 送信側の `Ack.rx.recv()` が
                    // 完了する。 wrap 側 :846 と同挙動。
                    Message::Syn(_) => {}
                }
                // 1 サンプル入力で 0〜N frame 出力する inner に対応するため Pending / Finished
                // まで drain する。 N は inner ごとに異なる (Openh264 / VideoToolbox は 0〜2、
                // Nvcodec は callback 経路で任意数)。
                loop {
                    match self.poll_output_sync()? {
                        EncoderRunOutput::Processed(sample) => {
                            if !output_tx.send_media(sample) {
                                output_tx.send_eos();
                                return Ok(());
                            }
                        }
                        EncoderRunOutput::Pending => break,
                        EncoderRunOutput::Finished => {
                            output_tx.send_eos();
                            return Ok(());
                        }
                    }
                }
                // wrap 側 :820 と挙動一致の防御コード。 handle_input_sample_sync(None) で
                // self.eos = true にした後、 上の内側 loop で poll_output_sync の Empty 分岐
                // (:733-738) が self.eos を見て Finished を返す実装のため実行時到達不能だが、
                // wrap 側と挙動を揃えて残す。
                if is_eos {
                    return Err(Error::new("video encoder still pending after EOS"));
                }
            }
            rpc_message = recv_video_encoder_rpc_message_or_pending(
                rpc_rx_enabled.then_some(&mut rpc_rx)
            ) => {
                // rpc_rx の disconnect (None 受信) は wrap 側 :826-828 と同じく flag off で
                // 吸収し、 std::future::pending() 化することで tokio::select! の RPC 腕を
                // ロックしなくする。 break にすると入力腕まで抜けて eos 未処理で return
                // する経路が増えるため flag off + continue を維持する。
                let Some(rpc_message) = rpc_message else {
                    rpc_rx_enabled = false;
                    continue;
                };
                self.handle_rpc_message_sync(rpc_message);
            }
        }
    }
}
```

骨子コード内で参照する型 (`Error`, `Message`, `ProcessorHandle`, `TrackId`, `Result`, `EncoderRunOutput`) と関数 (`recv_video_encoder_rpc_message_or_pending`) はいずれも `src/encoder.rs` の既存 use 文 (`:34-41`) と同ファイル内定義でカバー済みのため、 追加 use は不要。

### 骨子と wrap 版の行動等価性

上記骨子は wrap 版 `VideoEncoder::run` (`:790-836`) + `drain_video_encoder_output` (`:859-878`) + `recv_video_encoder_rpc_message_or_pending` (`:880-888`) の統合挙動と行動等価である。 根拠:

1. `subscribe_track` / `publish_track` / `register_rpc_sender` / `notify_ready` / `wait_subscribers_ready` の呼出順は wrap 側 (`:796-804`) と同一
2. `register_rpc_sender` のエラー文面 `"failed to register video encoder RPC sender: {e}"` は wrap 側 (`:802`) と一致
3. `Message` の 3 variant 処理 (Media / Eos / Syn) は wrap 側 `handle_input_message` (`:842-848`) と等価。 Syn は encoder が track 同期に関与しないため無視して drop する (drop 契機で送信側の `Ack` が完了する)
4. drain 内 `Processed` の `send_media` false 分岐で `send_eos` + return する挙動は、 wrap 版全体で「`drain_video_encoder_output` の PipelineClosed 経路 (`:865-868` で `Ok(true)`) → `run` 外側 (`:814-816`) の `if finished { send_eos; break; }` → 関数の `Ok(())` return」と発火するのと同一。 骨子側は drain 展開時に `send_eos` の呼出点だけを内側 loop 内に移した (制御フローの経路は違うが外部観測 = `output_tx` への `send_media` false 後の `send_eos` 送出 + 関数終了は完全一致)
5. drain 内 `Finished` の `send_eos` + return も、 wrap 版全体で「`drain_video_encoder_output` の Finished 経路 (`:873-875` で `Ok(true)`) → `run` 外側 (`:814-816`) の `if finished { send_eos; break; }` → 関数の `Ok(())` return」と発火するのと同一 (制御フローの構造は 4 番と対称、 発火する分岐だけが `:873-875` 側)
6. EOS 後 Pending の Err 文言 `"video encoder still pending after EOS"` は wrap 側 (`:820`) と完全一致
7. RPC 腕の disconnect 処理 (`rpc_rx_enabled = false; continue;`) は wrap 側 (`:826-828`) と一致
8. Err 経路は `?` propagation で wrap 側と同一 (エラー時に `send_eos()` は明示的に呼ばず、 `output_tx: TrackPublisher` の Drop 側で subscriber を閉じる契機に任せる。 `media_pipeline.rs:1213-1231`)

### 各使用側の移行

いずれも `VideoEncoder` の他利用箇所はなく (`rg '\bVideoEncoder\b' <各対象ファイル>` で確認)、 型置換のみで完結する。 use 文はアルファベット順に整列する (rustfmt / 既存規約踏襲)。

- `src/sora/recording_subcommand_compose.rs`:
  - `:15` の use 文: `encoder::{AudioEncoder, VideoEncoder}` → `encoder::{AsyncVideoEncoder, AudioEncoder}`
  - `:577` の `VideoEncoder::new(...)` → `AsyncVideoEncoder::new(...)`。 直後の `encoder.run(handle, in, out).await` はメソッド呼出構文で変数 `encoder` の型追随のためリテラルの書換不要
- `src/sora/recording_subcommand_vmaf.rs`:
  - `:15` の use 文: `encoder::VideoEncoder` → `encoder::AsyncVideoEncoder`
  - `:456` の `VideoEncoder::new(...)` → `AsyncVideoEncoder::new(...)` (compose と同パターン)
- `src/encoder.rs`:
  - `:1164` の `VideoEncoder::new(...)` → `AsyncVideoEncoder::new(...)`。 `create_video_processor` / `create_video_processor_with_params` の pub シグネチャは不変 (obsws 使用側 3 call site は無変更で通る)。 encoder.rs 内なので use 文追加は不要
- `src/subcommand_list_codecs.rs`:
  - `:7` の use 文: `encoder::{AudioEncoder, VideoEncoder}` → `encoder::{AsyncVideoEncoder, AudioEncoder}`
  - `:88` の `VideoEncoder::get_engines(...)` → `AsyncVideoEncoder::get_engines(...)` (`AsyncVideoEncoder::get_engines` は 0067 で `AsyncVideoEncoder` 側に移植済み、 wrap 側は薄い委譲)

### wrap `VideoEncoder` と helper の存置

本 issue では wrap 型自体は削除しない。 移行完了時点で wrap 側の本番呼出はゼロになるが、 以下は残る:

- `pub struct VideoEncoder` (`:763`) と `impl VideoEncoder` (`:767-857`) 全体
- helper `drain_video_encoder_output` (`:859`) と `recv_video_encoder_rpc_message_or_pending` (`:880`)。 後者は `AsyncVideoEncoder::run` からも呼ぶため引き続き存置 (再定義しない)
- `tests/encoder_tests.rs` の 3 テスト (`#[test]` 行 `:56` / `:92` / `:152`) が呼ぶ wrap 側 pub API (`VideoEncoder::new` / `handle_input_sample` / `poll_output`)

これらの整理と wrap 型の物理削除は後続の wrap 削除 + rename issue (未起票、 closed/0073 相当) で扱う。 `_sync` サフィックス整理も同時に扱う (closed/0057 §3 line 362 の wrap 削除 + rename 行に「`_sync` / `_async` サフィックス整理」として記載済み。 本 issue では wrap `VideoEncoder` の同名 pub API (`handle_input_sample` / `poll_output`) との名前空間分離のため `_sync` サフィックスを保持する)。

### 現存 docstring の更新

wrap 削除前提だが、 本 issue の完了時点で「wrap の run から呼ばれる」の記述が現実と乖離する箇所は書き直す。 対象は `AsyncVideoEncoder` 型 docstring (`src/encoder.rs:461-474`) と `handle_rpc_message_sync` docstring (`:680-685`) の 2 箇所。 `VideoEncoder` 型 docstring (`:758-761`) の「同期 API を保つ VideoEncoder は AsyncVideoEncoder の wrap として動作する。 (中略) 将来 AsyncVideoEncoder 直接利用への段階移行が完了した時点で本 wrap 型は削除される」は wrap 削除完了までは現実と一致するため更新不要。

`AsyncVideoEncoder` 型 docstring (`:461-474`) は 14 行の複数段落構成。 「エンコーダー本体で、 `VideoEncoder` (wrap) の `run` (processor 経路) から (中略) 同期駆動される」 (`:463-465`) の 1 文を「エンコーダー本体で、 processor 経路 (`AsyncVideoEncoder::run` および wrap `VideoEncoder::run`) から `handle_input_sample_sync` / `poll_output_sync` / `handle_rpc_message_sync` 等の `_sync` 付き内部 API 経由で同期駆動される」に置換する。 続く「wrap 側は同名の非 `_sync` API (`handle_input_sample` / `poll_output`) を露出し、 内部で本 struct の `_sync` 版に delegate する。 直接利用するときは `next_encoded_frame_async` で非同期に取得する」 (`:465-467`) は wrap 側の delegate 挙動として引き続き正確なので据え置く。 `:469-474` の「注意」以下 (Nvcodec 等の drop 順注意) も本 issue のスコープ外で据え置く。

`handle_rpc_message_sync` docstring (`:680`) の「wrap (`VideoEncoder`) の `run` 内 RPC 腕から delegate される同期 RPC ハンドラ」を「processor 経路 (`AsyncVideoEncoder::run` および wrap `VideoEncoder::run`) の RPC 腕から呼び出される同期 RPC ハンドラ」に置換する。 `:682-685` (RPC の意味論説明) は据え置く。

### 決定事項 (実装で覆さない)

- `AsyncVideoEncoder::run` のシグネチャは wrap 側 `VideoEncoder::run` と同一 `pub async fn run(mut self, handle: ProcessorHandle, input_track_id: TrackId, output_track_id: TrackId) -> Result<()>`
- 追加場所は `src/encoder.rs` の既存 `impl AsyncVideoEncoder` ブロック (`:502-756`) 内で、 `next_encoded_frame_async` 関数閉じ `}` (`:755`) の直後、 impl 閉じ `}` (`:756`) の直前。 新規 `impl AsyncVideoEncoder { ... }` ブロックを増やさない。 属性 (`#[allow]` 等) は付けない (wrap 側 `impl VideoEncoder` と対称)
- `AsyncVideoEncoder::run` の docstring は骨子コード先頭に掲載した 4 行のもの (processor モデル駆動の用途 / wrap `VideoEncoder::run` との等価性 / `_sync` API 直呼びの実装派生 の 3 要素) を最低ラインとしてそのまま採用する。 docstring 内に issue 番号を含めない (src/ 内の既存 docstring は `issue 00XX` 形式のプレフィックスなし参照が慣習、 `closed/00XX` 形式は 0 件)
- `_sync` API はいずれも `pub(crate)` (`:686, :700, :729`) のまま維持し、 可視性は変更しない (同一 crate 内呼出のため十分)。 `_sync` サフィックスは wrap 存置期間中の名前空間分離のため保持 (wrap 削除 + rename issue で最終的に整理される、 §「wrap `VideoEncoder` と helper の存置」参照)
- `AsyncVideoEncoder::handle_input_message` / `handle_rpc_message` は追加しない (`run` の内側で `Message` の 3 variant を自前 dispatch する形にする)
- `recv_video_encoder_rpc_message_or_pending` (`:880`) は wrap 側と共有する (再定義しない)
- Nvcodec / VideoToolbox / openh264 feature 有効時と無効時で `AsyncVideoEncoder::run` の挙動差分はない (`poll_output_sync` が同期・非同期 inner の両方を透過的に扱う)
- `AsyncVideoEncoder::name()` / `codec()` は `Option<...>` を返すが、 本 issue の 4 hit + obsws 3 site のいずれも `.name()` / `.codec()` を呼び出さないため書き換え不要
- `AsyncVideoEncoder` の drop 順制約 (`inner` を `rx` より先に drop) は `AsyncVideoEncoder` 定義側 (`:484-490`) のコメントで担保済み、 `run` 側で追加対応不要
- wrap `VideoEncoder::run` (`:790-836`) と挙動完全一致を維持する (§「骨子と wrap 版の行動等価性」の 8 項目)
- wrap `VideoEncoder` 型と helper 2 種は本 issue では削除しない (wrap 削除 issue で扱う)
- 各 inner (`src/encoder/*.rs`) と obsws 使用側 (`src/obsws/`) は一切変更しない (機械検証は §「## 完了条件」の grep および `git diff --name-only` に依る)
- `tests/encoder_tests.rs` の 3 テストは wrap 型のテストとして残す (wrap 削除 issue で `AsyncVideoEncoder` へ切替、 または wrap と一緒に削除)

### テスト戦略

`AsyncVideoEncoder::run` は wrap 側 `VideoEncoder::run` に既存単体 test がなく (実装が実 pipeline 前提)、 本 issue でも e2e で担保する方針を継続する。

- `tests/e2e.rs` の compose シナリオのうち実サンプル入力を伴うテスト (`compose_stdout_summary_has_required_fields` (`:850`) / `compose_stats_file_has_required_top_level_and_processor_entries` (`:902`)、 いずれも `testdata/e2e/simple_single_source_vp9/` を `--layout-file` + input として使う) が `src/sora/recording_subcommand_compose.rs:577` の call site (`AsyncVideoEncoder::new` + `run`) を実行して回帰検出する。 `compose_empty_source_summary_omits_media_specific_fields` (`:960`) は空 source で `Finished` 経路のみ通り実データエンコード経路は踏まない
- vmaf シナリオが `src/sora/recording_subcommand_vmaf.rs:456` の call site を実行
- obsws HLS / DASH / output の 3 経路は `create_video_processor_with_params` (`src/encoder.rs:1137`) 経由で本 issue の切替の影響を受ける。 対応する e2e / integration test の有無は着手時に `rg '#\[(?:tokio::)?test\]' tests/` で再確認する

`poll_output_sync` の 4 分岐 (`Processed` / `Pending` / `Finished` / Err) は既に unit test 済み (`src/encoder.rs:1298-1375`)、 `emit_err` 経由の Err 分岐と `next_encoded_frame_async` の pub 契約テスト 2 件も 0067 で追加済み (`:1385-1414`)。 `AsyncVideoEncoder::run` は分岐選択の薄いラッパとみなせるため新規 unit test は追加しない。 doctest も不要 (call site そのものが `spawn_processor` の使用側 example)。

RPC keyframe 経路 (`register_rpc_sender` → `handle_rpc_message_sync` → 次入力で `inner.request_keyframe()`) の回帰検出は、 6 site (`sora_publisher.rs:93` / `mp4/hybrid_writer.rs:783` / `mp4/writer.rs:859` / `rtmp/outbound_endpoint.rs:628` / `hls/writer.rs:438` / `dash/writer.rs:377`) のいずれかで `request_upstream_video_keyframe` (`src/encoder.rs:416`) が発火する e2e シナリオが必要。 着手時に以下の順序で試みる:

1. **RUST_LOG 目視で発火経路の存在を確認**: `RUST_LOG=hisui::encoder=debug` で e2e (compose / vmaf) を回して `"requested keyframe"` 相当の tracing::debug が出るかを目視。 発火しなければ RPC 経路そのものが e2e シナリオでは触られていないため、 手段 2 / 3 の検討は不要 (残懸念に直接落ちる)
2. **1 で発火する場合、 既存 e2e の stats assert が発火を捉えるかを確認**: `tests/e2e.rs` の compose / vmaf シナリオで pipeline 完了後の `total_video_keyframe_request_count` メトリクスを assert していないかを grep。 存在すれば既存 e2e が回帰検出をカバー。 assert が無いが stats file を dump しているシナリオがあれば、 1 assert 追加で対応 (追加 assert が本 issue の refactor スコープ内に収まるか要判断)
3. **1 で発火するが 2 でも既存 e2e の枠内で捉えられない場合の最終手段として新規 unit test 追加**: `AsyncVideoEncoder::run` に対して直接 RPC を送るテスト。 `register_rpc_sender` が実 pipeline 依存のため実質 integration test 相当になる (本 issue のスコープを広げる判断が要る)

上記 3 手段いずれも成立せず、 かつ本 issue のスコープでは対応が難しいと判断した場合は、 本 issue の実装 PR 完了時に issue 本文 §テスト戦略末尾に「残懸念: RPC 経路の回帰は既存 e2e では検出できない (根拠 = 手段 1 で発火せず / 手段 2 で assert 追加のスコープ超え / 手段 3 は integration test 化コスト超過、 のいずれか)」を追記して close する (残懸念は close 時に必ず 1 文で明記、 別 issue 化はしない)。 wrap 削除 + rename issue の起票時 (別 Decision Owner の可能性あり) に、 起票者が本 issue の残懸念記述を確認し、 必要と判断すれば起票 issue の完了条件に組み込む。 本 issue から起票 issue の完了条件を先に指定しない。

## 完了条件

- `pub async fn run(mut self, handle: ProcessorHandle, input_track_id: TrackId, output_track_id: TrackId) -> Result<()>` が `src/encoder.rs` の既存 `impl AsyncVideoEncoder` (`:502-756`) 内に追加され、 §「### 決定事項 (実装で覆さない)」に列挙した 3 要素を含む docstring が付与されている (docstring 内に issue 番号を含めない)
- 本 issue で書き換える対象表 (§「### 本 issue で書き換える対象 (直呼出 4 hit)」) の 4 hit がすべて `AsyncVideoEncoder` に置換され、 use 文 (`compose:15` / `vmaf:15` / `list_codecs:7`) も追随している
- `create_video_processor` / `create_video_processor_with_params` の pub シグネチャは不変 (obsws 使用側 3 call site は無変更で通る)
- wrap `VideoEncoder` 型と `impl VideoEncoder` ブロック、 wrap 側 helper 2 種 (`drain_video_encoder_output` / `recv_video_encoder_rpc_message_or_pending`) は削除せず残す (wrap 削除 issue で扱う)
- `AsyncVideoEncoder` 型 docstring (`src/encoder.rs:461-474`) と `handle_rpc_message_sync` docstring (`:680-685`) が §「### 現存 docstring の更新」の記述に更新されている
- grep / diff 検証 (BSD grep の `\b` サポートが安定しないため `rg` (ripgrep) を推奨。 `grep -E '\bVideoEncoder::(new|run|get_engines)\b'` は GNU grep 前提):
  - `rg '\bVideoEncoder::(new|run|get_engines)\b' src/` の hit が 0 件 (`AsyncVideoEncoder::` は前置に単語文字 `c` が続くため単語境界不成立で hit しない)
  - `tests/encoder_tests.rs` 内の `VideoEncoder::new` 3 hit は本 issue のスコープ外として意図的に残す (対象パスから `tests/` を除外)
  - `git diff --name-only develop -- src/encoder/ src/obsws/` が空 (各 inner ファイル / obsws 使用側は一切変更しない)
- closed/0057 §3 分割表 (`issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md`) の 4 行 (line 351 / 360 / 361 / 364) を本 issue の実装 PR に含める。 いずれも同じ line 351-364 領域の局所編集で行数変動なし。 なお表内の推定 LOC は line 356-359 の precedent と揃えて `+insertions/-deletions` 形式 (括弧なし、 例 `+147/-9`) で記入する:
  - line 361 の未起票行を closed/0068 (line 356) 対称の 5 セル形式に置換 (推定 LOC は実装完了時点の `git diff --stat develop` からの実績値。 提出時点は `open/0079`、 マージ後に `closed/0079` に切替):
    ```
    | open/0079 (`feature/refactor-migrate-video-encoder-users-to-async`) | encoder 使用側 4 hit を `AsyncVideoEncoder` に移行 + `AsyncVideoEncoder::run` 追加 | <+X/-Y> | 0067 | 内部 API のみ |
    ```
  - line 360 の 0067 行の表記を `| open/0067 (...) | ... | 千行前後 | 0066 | 内部 API のみ |` から `| closed/0067 (...) | ... | +X/-Y | 0066 | 内部 API のみ |` に更新 (表記 open→closed + 推定 LOC を実績値に置換、 broken windows 修正)。 0067 は commit `7b5f2740` で develop merge 済みのため、 実測は本 issue 着手時に `git diff --stat 7b5f2740~1..7b5f2740` から insertions/deletions を確定できる (0079 進行度と独立)
  - line 351 の依存順序記述内の「encoder 使用側移行」を「open/0079」に、 「Nvcodec flush 撤廃 perf issue」を「open/0080」に置換 (0079 起票と 0080 起票の反映、 broken windows)
  - line 364 の 0080 行 (`(未起票) Nvcodec flush() 撤廃 + bp 機構 perf issue ...`) を line 360-361 と同型の 5 セル形式 `| open/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`) | NVENC 非同期パイプライン並列性回復 (flush() 撤廃 + bp 機構)、 wall-clock 短縮 15% / p99 改善 5ms 等の実機計測を完了条件に据える | 未推定 | 0067 | 内部 API のみ (perf カテゴリ) |` に置換 (broken windows)。 1 st cell の「Nvcodec flush() 撤廃 + bp 機構 perf issue」の情報を 2 nd cell の括弧に移し、 現物 2 nd cell の `(本 §3 中核動機)。 ` (0080 起票済み以降は §3 内部参照不要のため不要) は落とす
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 依存関係

依存先 closed/0067 (`feature/refactor-add-async-video-encoder`、 2026-07-08 develop merge、 commit `7b5f2740`) は完了済み。

着手時の再確認 grep (本番経路に想定外の使用が復活していないことの確認、 `rg` 推奨):

```
rg '\bVideoEncoder::(new|run|get_engines)\b' src/
```

期待 hit は §「### 本 issue で書き換える対象 (直呼出 4 hit)」の 4 件のみ (compose:577 / vmaf:456 / encoder.rs:1164 / list_codecs:88)。 `tests/` は本 issue でスコープ外 (wrap 削除 issue で扱う) のため対象から除外する。 これ以外の hit があれば、 その使用側の移行を先に扱うか本 issue のスコープを拡張するかを Decision Owner が判断する。

## 解決方法

各 step で wrap 側 `VideoEncoder::run` と helper は残るため、 いずれの中間状態でも `cargo check` は通る。 順序制約は「Step 1 (`AsyncVideoEncoder::run` 追加) は Step 2 / 3 / 4 (`VideoEncoder::new` → `AsyncVideoEncoder::new` 置換) より先に完了させる」 (Step 2 / 3 / 4 の直後で `encoder.run(...)` が新型の `run` に解決されるため)。 Step 5 (`get_engines` 置換) は `AsyncVideoEncoder::get_engines` が 0067 で既に定義済み (`:639`) のため Step 1 に非依存で独立実施可能。

1. `src/encoder.rs` の既存 `impl AsyncVideoEncoder` (`:502-756`) 内、 `next_encoded_frame_async` 関数閉じ `}` (`:755`) の直後に `AsyncVideoEncoder::run` を docstring 付きで追加する (§「### `AsyncVideoEncoder::run` の実装骨子」のコードブロックの中身をそのまま貼り付け)。 `recv_video_encoder_rpc_message_or_pending` (`:880`) は wrap 側と共有する。 この時点で wrap 側 `VideoEncoder::run` は残っているため `cargo check` は通る
2. `src/sora/recording_subcommand_compose.rs` の use 文 (`:15`) と `:577` の `VideoEncoder::new` を `AsyncVideoEncoder::new` に置換する (アルファベット順を維持)。 `encoder.run(...)` はメソッド呼出構文のため変数型の追随だけで書換不要
3. `src/sora/recording_subcommand_vmaf.rs` の use 文 (`:15`) と `:456` を同様に置換 (step 2 と同パターン)
4. `src/encoder.rs:1164` の `VideoEncoder::new` を `AsyncVideoEncoder::new` に切り替える (`create_video_processor_with_params` の pub シグネチャは不変。 encoder.rs 内なので use 文追加は不要)
5. `src/subcommand_list_codecs.rs` の use 文 (`:7`) と `:88` の `VideoEncoder::get_engines` を `AsyncVideoEncoder::get_engines` に置換する
6. `AsyncVideoEncoder` 型 docstring (`src/encoder.rs:461-474`) と `handle_rpc_message_sync` docstring (`:680-685`) を §「### 現存 docstring の更新」の記述に更新
7. closed/0057 §3 分割表 (`issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md`) の 4 行更新 (line 351 / 360 / 361 / 364) を本 issue の実装 PR に含める。 全て局所編集で行数変動なし
8. 完了条件の全 cargo コマンド (`fmt --check` / `check` (default + `--no-default-features`) / `clippy` (default + `--no-default-features`) / `test` (default + `--no-default-features`)) を通す

## CHANGES.md について

内部リファクタにつき記載不要。 hisui は bin crate として配布され、 `VideoEncoder` 系は外部公開していない。 外部プロトコル / 出力は不変。

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): decoder 側の親 issue、 派生方針 (δ) を確立
- closed/0067 (`feature/refactor-add-async-video-encoder`): 依存先。 `AsyncVideoEncoder` 追加 + wrap 化 + inner Sender 化。 2026-07-08 develop merge 済み
- closed/0068 (`feature/refactor-migrate-video-decoder-users-to-async`): decoder 側の対称 issue。 本 issue と同じパターン (`AsyncVideoDecoder::run` 追加 + processor 経路移行) を encoder 側に移し替える
- closed/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`): decoder 側の最終クリーンアップ (wrap 削除 + rename)。 encoder 側でも本 issue の PR merge 後に対応する wrap 削除 + rename refactor issue (未起票) を起票する。 起票時に `rg '\bVideoEncoder\b' src/` の結果が wrap 定義箇所と `drain_video_encoder_output` 引数型のみになっていることを確認する
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 採用案 C の親 issue。 §3 分割表 line 361 の「(未起票) encoder 使用側移行 refactor issue」行を本 issue に対応させる (完了条件の 0057 §3 分割表更新を参照)
- open/0080 (`feature/refactor-nvcodec-encoder-flush-and-backpressure`): 独立に着手可能な perf issue。 本 issue の完了は 0080 の前提としない、 逆も同様 (0080 側で明言済み)
