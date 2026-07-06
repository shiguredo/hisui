# RTMP / RTSP / SRT inbound endpoint の video decoder 経路を spawn pattern に再設計する

- Priority: Medium
- Created: 2026-07-02
- Completed: 2026-07-06
- Model: Claude Opus 4.7
- Branch: feature/refactor-inbound-endpoint-async-video-decoder
- Polished: 2026-07-03
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0068 (2026-07-03 close) と closed issue 0071 (2026-07-02 close) の完了により、 crate 内で **同期 wrap `VideoDecoder` を保持する call site は RTMP / RTSP / SRT の 3 inbound endpoint のみ** となった。 本 issue は 3 endpoint を `AsyncVideoDecoder` ベースの spawn pattern に切り替え、 open issue 0073 (最終クリーンアップ、 同期 wrap 削除 + `AsyncVideoDecoder` → `VideoDecoder` リネーム) の着手条件を満たす。

3 endpoint は decoder の所有と受信ループへの組み込みが 3 者三様のため、 単純な call site 置換ではなく **受信ループの再設計 (refactor)** が主眼になる:

- **rtmp** (`src/rtmp/inbound_endpoint.rs`): `RtmpInboundEndpoint::run` 内のローカル変数を、 接続ごとに `RtmpPublisherHandler` に `take()` で move し、 切断時に `handler.into_parts()` で回収して次接続に使い回す (接続跨ぎで decoder 長寿命)
- **rtsp** (`src/rtsp/subscriber.rs`): `RtspSubscriber::run` 内のローカル変数を `RtspOutputContext<'a>` に `&mut Option<VideoDecoder>` として借用で渡す。 `SessionError::Retryable` 経由の再接続では decoder が生存を続ける
- **srt** (`src/srt/inbound_endpoint.rs`): `SrtInboundEndpoint::run` 内のローカル変数を receive path に `&mut Option<VideoDecoder>` として借用で渡す。 `process_polled_events` は **同期クロージャ** で、 内側から同期 fn `publish_samples` を呼ぶ

closed issue 0068 の polish 過程で本 issue が分離された。 分離理由は「実装難所 (接続跨ぎパターン、 再接続時ライフサイクル、 SRT 同期クロージャ) が集中しており、 0068 の単純 call site 置換とは性質が異なる」。

### スコープ外 (本 issue で触らない)

- **audio decoder**: `AsyncAudioDecoder` は未整備。 3 endpoint の audio 経路 (`audio_decoder: Option<AudioDecoder>` field、 `handle_input_sample` / `drain_audio_decoder_output` の直呼出) は同期のまま維持する
- **同期 wrap `VideoDecoder` / `drain_video_decoder_output` の削除、 `AsyncVideoDecoder` → `VideoDecoder` リネーム**: open issue 0073 で扱う
- **decoder task 抽象の共通化 (mp4 reader / 3 endpoint 共通の `VideoDecoderTask` module 切り出し)**: open issue 0073 の未確定論点 4 で最終判断。 本 issue は各 endpoint 内 module-private helper として **写経** する (0071 の `src/mp4/reader.rs:1528-1643` を参照実装として利用)

## 優先度根拠

Medium。

- closed issue 0066 の wrap 段階的移行方針 (δ) を、 closed issue 0057 §3 「中途半端な 2 系統共存を残さない」原則と整合させるには全使用側移行が必要。 本 issue はその最後の 1 パート
- 本 issue 単独では外部挙動 (受信 → publish のセマンティクス) は不変。 内部 refactor で緊急性なし
- open issue 0073 (最終クリーンアップ) は 0068 / 0071 / 本 issue の全完了を待つ。 0068 / 0071 は既に closed のため、 本 issue が 0073 着手の唯一のブロッカー

## 現状

### 3 endpoint の decoder 保持と受信ループ

#### RTMP (`src/rtmp/inbound_endpoint.rs`)

| 位置 | 内容 |
|------|------|
| `:164-176` | `VideoDecoder::new(...)` (`RtmpInboundEndpoint::run` 内、 `Some(...)` に包む) |
| `:177-187` | `AudioDecoder::new(...)` (本 issue スコープ外) |
| `:220-238` | `RtmpPublisherHandler::new` 呼出。 引数として `video_track_tx.take()` (`:225`)、 `audio_track_tx.take()` (`:226`)、 `video_decoder.take()` (`:227`)、 `audio_decoder.take()` (`:228`) を渡す |
| `:243-252` | `handler.into_parts()` (`:248`) で 4-tuple (`video_track_tx`, `audio_track_tx`, `video_decoder`, `audio_decoder`) を回収 → ローカル変数に restore |
| `:295-307` | `struct RtmpPublisherHandler`。 field: `video_track_tx: Option<TrackPublisher>` (`:302`)、 `audio_track_tx: Option<TrackPublisher>` (`:303`)、 `video_decoder: Option<VideoDecoder>` (`:304`)、 `audio_decoder: Option<AudioDecoder>` (`:305`) |
| `:309-338` | `RtmpPublisherHandler::new` コンストラクタ (9 引数、 `#[expect(clippy::too_many_arguments)]`)。 `RtmpIncomingFrameHandler::new(timestamp_offset)?` (`:331`) が唯一の Err 経路 |
| `:367-381` | `into_parts` (4-tuple 戻り値) |
| `:462-484` | `handle_video_frame` (async fn)。 `:473` で `decoder.handle_input_sample(Some(MediaFrame::Video(Arc::new(video_frame))))?`、 `:477` で `drain_video_decoder_output(decoder, tx)` |
| `:435-457` | `handle_audio_frame` (audio 側、 本 issue スコープ外) |

現状 decoder は endpoint 寿命で保持され、 接続ごとに `handler` に move/回収で使い回される。 接続跨ぎ再利用の明示的要件は文書化されていないが、 `TrackPublisher::Drop` が `publisher_processor_id` を clear しない設計 (`src/media_pipeline.rs:391-395`、 `handle_unpublish_track` 経由のみ clear) のため、 **同じ `track_id` に対する再 `publish_track` は `DuplicateTrackId` で失敗する**。 接続ごとに `TrackPublisher` を作り直せない構造制約が endpoint 寿命の decoder 保持を裏付けている。

#### RTSP (`src/rtsp/subscriber.rs`)

| 位置 | 内容 |
|------|------|
| `:82-169` | `RtspSubscriber::run` 全体 |
| `:104-116` | `VideoDecoder::new(...)` (`Some(...)` に包む) |
| `:117-127` | `AudioDecoder::new(...)` (本 issue スコープ外) |
| `:137-142` | `RtspOutputContext { audio_track_tx, video_track_tx, audio_decoder, video_decoder }` を組み立てて `run_rtsp_session` に渡す |
| `:275-282` | `struct RtspOutputContext<'a>`。 4 field すべて `&'a mut Option<...>`。 `video_decoder: &'a mut Option<VideoDecoder>` (`:281`)、 `video_track_tx: &'a mut Option<TrackPublisher>` (`:279`)、 audio 側 (`:278`, `:280`) |
| `:657-763` | `handle_rtp_packet` (同期 fn)。 `:696-713` で `output.video_decoder.as_mut()` + `output.video_track_tx.as_mut()` 経由で `decoder.handle_input_sample(...)?` (`:699`)、 `drain_video_decoder_output(decoder, tx)?` (`:705`) を呼ぶ |
| `:153-164` | `session_result` の 3 経路: (a) `Ok(())` → 再接続、 (b) `Err(SessionError::Fatal(e))` → endpoint 停止 `return Err(e)`、 (c) `Err(SessionError::Retryable(e))` → 再接続。 いずれの経路でも decoder ローカル変数は継続保持 |

`RtspOutputContext` を組み立てる箇所は `:137` (run 内) と、 unit test 3 箇所 (`:1676`, `:1715`, `:1760`、 いずれも decoder 経路を触らない URL parse / depacketizer 系のテスト内で空 context を要求するため `&mut None` を渡すだけ) の合計 4 箇所。

#### SRT (`src/srt/inbound_endpoint.rs`)

| 位置 | 内容 |
|------|------|
| `:164-323` | `SrtInboundEndpoint::run` 全体 |
| `:201-213` | `VideoDecoder::new(...)` (`Some(...)` に包む) |
| `:214-224` | `AudioDecoder::new(...)` (本 issue スコープ外) |
| `:229-274` | `process_polled_events` **同期クロージャ** `\|conn, peer_addr\| -> Result<()>`。 `&mut audio_track_tx`, `&mut video_track_tx`, `&mut audio_decoder`, `&mut video_decoder`, `&stats`, `&mut demuxer`, `&mut connection_timestamp_offset`, `base_time`, `&endpoint_config` などを capture |
| `:277`, `:312` | `process_polled_events(&mut conn, &mut peer_addr)?` 呼出 (1 iteration あたり 2 回) |
| `:468-522` | `publish_samples` 同期 fn (`&mut Option<AudioDecoder>`, `&mut Option<VideoDecoder>`, `&mut Option<TrackPublisher>` × 2 を引数)。 `:488` (audio) / `:508` (video) で `handle_input_sample`、 `:492` (audio) / `:512` (video) で `drain_*_decoder_output` |
| `:376-422` | `handle_connection_event` 同期 fn。 `:401-411` で切断時に `reset_connection_state` を呼ぶ |
| `:524-534` | `reset_connection_state` 同期 fn。 切断時 (`ConnectionEvent::Disconnected` / `StateChanged(Disconnected)`) に呼ばれ demuxer と connection をリセット。 現状 decoder は reset しない (endpoint 寿命で生存) |

### `AsyncVideoDecoder` の現状 API (`src/decoder.rs`)

| メソッド | 位置 | 用途 |
|---------|------|------|
| `pub fn new(options, stats) -> Self` | `:400` | コンストラクタ (Result を返さず必ず成功) |
| `pub fn handle_input_sample_sync(&mut self, Option<MediaFrame>) -> Result<()>` | `:424` | 同期入力 (task 内で使う) |
| `pub fn poll_output_sync(&mut self) -> Result<DecoderRunOutput>` | `:441` | 同期 poll (task 内で使う、 `Processed` / `Pending` / `Finished` の 3 分岐) |
| `pub async fn next_decoded_frame_async(&mut self) -> Option<Result<VideoFrame>>` | `:472` | 非同期取得 (本 issue の task では使わない、 EOS を `None` で表現しないため終端検知が組めない) |
| `pub async fn run(self, handle, input_track_id, output_track_id) -> Result<()>` | `:476` | processor モデル用 (0068 で追加、 本 issue 不採用、 §「決定事項」3) |
| `pub fn get_engines(codec, is_openh264_available) -> Vec<EngineName>` | `:520` | エンジン列挙 |

`handle_input_message` は同期 wrap `VideoDecoder` のみに存在し (`:629`)、 0068 で `AsyncVideoDecoder` 側には追加しないことが確定 (closed/0068 §「AsyncVideoDecoder に不足している API」)。

なお docstring `:380-383` に「非同期な内部デコーダー (Nvcodec 等) 使用時、 `AsyncVideoDecoder` を drop する前に EOS + drain を完走させないと、 コールバックが drop 中に emit した残物とメトリクス (`total_output_video_frame_count`) が乖離する可能性」が明記されている。 本 issue の task lifecycle 設計はこれを踏まえて endpoint 寿命保持を選ぶ (§「推奨案 §1」根拠参照)。

### 0071 で確立された spawn pattern (`src/mp4/reader.rs`、 module-private、 本 issue の参照実装)

| 型/関数 | 位置 | 概要 |
|--------|------|------|
| `enum DecoderInput { Media(MediaFrame), Eos }` | `:1528` | task 入力メッセージ (Syn なし) |
| `struct VideoDecoderTask { input_tx, discard_mode_tx, join_handle }` | `:1533` | `discard_mode_tx` は mp4 reader 固有の warm-up 制御、 `join_handle` は `(TrackSender, Result<()>)` を回収 |
| `spawn_video_decoder_task(options, stats, sender: TrackSender) -> VideoDecoderTask` | `:1565` | task 生成、 内部で `stats.set_default_label("component", "video_decoder")` |
| `video_decoder_loop` + `run_video_decoder_loop` | `:1586`, `:1601` | `AsyncVideoDecoder::new` → `poll_output_sync` の 3 分岐 drain |
| `pub(crate) struct TrackSender` | `:1470` | mp4 reader 内で SYN/ACK 背圧付き sender (`send_media(sample).await -> bool`) |
| `impl Drop for Mp4FileReader` | `:1400-1409` (相当) | `task.join_handle.abort()` で panic path / early return path での task leak を防ぐ |

本 issue の 3 endpoint は mp4 reader と用途が異なるため、 `VideoDecoderTask` から `discard_mode_tx` を落とし、 `TrackSender` (背圧付き) ではなく `TrackPublisher` を直接 task に move し、 `JoinHandle` の戻り値は `crate::Result<()>` のみとする (task と `output_tx` を endpoint 寿命で保持するため回収不要)。

### 現状の背圧と `unbounded` channel 採用

3 endpoint は現状同期 pull ループで、 decoder 内部 (`AsyncVideoDecoder::output_rx` は unbounded、 0066 確定) にキューイングが完結。 上流 (TCP / UDP recv buffer) から見ると、 decode が遅ければ receive loop 全体が同期的に詰まり、 OS レベルで peer 側に TCP/RTP 背圧が波及する。

spawn pattern に切り替えると、 receive loop は `UnboundedSender::send` (同期 fn、 `Result<(), SendError<T>>` 返却、 `.await` なし) で input_tx に投げるため、 receive loop が decode 完了を待たない = 上流の recv buffer は詰まらない。 代わりに decoder task 入力 channel が unbounded で成長する可能性がある (decoder が上流速度に追いつかない極端なケースで OOM)。 意図的な perf 改善ではなく、 spawn pattern 化に伴う副作用として粒度が移るだけである。 実測回帰は残懸念 §1 で扱う。

### 既存テスト

- **integration test**: `tests/rtmp_inbound_endpoint_tests.rs` (125 行)、 `tests/rtsp_subscriber_tests.rs` (56 行)、 `tests/srt_inbound_endpoint_tests.rs` (127 行)。 いずれも `new()` バリデーション網羅のみで `run()` 経路を触らない
- **unit test**:
  - `src/rtsp/subscriber.rs:1676, :1715, :1760` の 3 箇所で `RtspOutputContext` を組み立て (URL parse / depacketizer 系のテスト内で空 context を要求)。 各テストは `let mut video_track_tx = None; let mut video_decoder = None; ...` のように事前宣言してから `&mut` を渡している
  - `src/srt/inbound_endpoint.rs:1091-` の unit test は MPEG-TS parse 系で decoder 経路を触らない
- **e2e** (`tests/e2e.rs`): 3 endpoint はカバーしない (subprocess で `hisui` CLI サブコマンドを叩く形式で inspect / compose / vmaf のみ)

## 設計方針

### 決定事項 (実装で覆さない)

1. **`AsyncVideoDecoder::run` (`src/decoder.rs:476`) は本 issue の decoder task では再利用しない**: 3 endpoint の受信ループは `handle.subscribe_track` 経由の processor モデルではなく外部 IO から直接受信するため `AsyncVideoDecoder::run` のインタフェースに合わない。 各 endpoint 内で `handle_input_sample_sync` + `poll_output_sync` の drain ループを自前で組む (0071 と同型)
2. **warm-up 機構は導入しない**: 3 endpoint に seek / restart / loop 継続 (mp4 reader の warm-up 対象) 相当の状態がないため `discard_mode_tx` は持たない
3. **task 入力 channel は `unbounded`、 型は `enum DecoderInput { Media(MediaFrame), Eos }`**: 0071 と統一。 Syn は含めない (endpoint は publish 側)
4. **task 出力は `TrackPublisher` 直接**: `TrackPublisher::send_media(sample) -> bool` (`src/media_pipeline.rs:1276`) を task 内で呼ぶ (`.await` なし、 SYN/ACK 背圧なし)。 `TrackSender` (`src/mp4/reader.rs:1470`) は使わない (リアルタイム受信で上流 recv buffer を詰まらせる意味論が mp4 reader と異なる)
5. **decoder task 型群は各 endpoint 内 module-private として写経**: `enum DecoderInput` / `struct VideoDecoderTask` / `spawn_video_decoder_task` / `video_decoder_loop` を 3 endpoint それぞれで module-private 定義する。 共通化 (`src/decoder/task.rs` 等への切り出し) は 0073 の未確定論点 4 で最終判断
6. **task と `TrackPublisher` は endpoint 寿命で保持**: `RtmpInboundEndpoint::run` / `RtspSubscriber::run` / `SrtInboundEndpoint::run` の冒頭で 1 回だけ `spawn_video_decoder_task` を呼び、 endpoint 寿命の間 `Option<VideoDecoderTask>` として保持する。 `TrackPublisher` は spawn 時に task 内に move されるため、 endpoint 側からは `input_tx.clone()` (Clone 可能) のみ handler / context に渡す。 これにより (a) 同一 `track_id` への再 `publish_track` (`DuplicateTrackId` エラー) を回避、 (b) 接続ごとの `send_eos` 発火による下流 mixer での track 終了誤検知を回避、 (c) 現状同期版の「decoder を接続跨ぎで保持」挙動と一致
7. **エラー・panic 伝搬とメッセージ**:
    - `VideoDecoderTask::shutdown` は `input_tx.send(Eos)` → `join_handle.await` を実施
    - panic は `tracing::error!("video decoder task panicked: {e}")` + `Err(crate::Error::new(format!("video decoder task panicked: {e}")))` で fatal 化
    - join failure (非 panic) は `Err(crate::Error::new(format!("video decoder task join failed: {e}")))`
    - `output_tx.send_media(sample)` false (subscriber 全 drop) → task を `Ok(())` で正常終了
    - `poll_output_sync` Err → `?` で task が Err で終了
    - Err 経路で `send_eos` は呼ばない (`TrackPublisher::Drop` は `eos_sent=false` として subscribers を pipeline に返却し、 pipeline 側は `marked_for_republish=true` で republish 待ちにする。 本 issue の 3 endpoint は endpoint 寿命 = pipeline 生存中は republish しないため、 endpoint 停止経路のみこの状態に落ちる想定)
    - `input_tx.send` の Err (task 死亡) を receive loop 側で検出した場合は fatal error として endpoint / session を停止させる。 3 endpoint 統一メッセージ: `crate::Error::new("video decoder task terminated unexpectedly")`
8. **`Drop` impl による task leak 防止**: `impl Drop for VideoDecoderTask` を骨子側に組み込み、 `join_handle` を `Option<JoinHandle<...>>` として保持して `Option::take` パターンで shutdown と Drop の共存を実現する。 Rust の drop check は Drop 実装型の field を partial move できないため、 `shutdown(self)` 内で `self.join_handle.take().expect(...).await` の形にする必要がある (`join_handle: JoinHandle<...>` 直接保持だと `self.join_handle.await` が E0509 でコンパイル不能)。 これにより (a) 関数正常経路の drop、 (b) 早期 `?` return 経路の drop、 (c) panic unwind 経路の drop、 (d) shutdown() 経路の drop、 いずれでも `Option::take` の semantics で二重 abort が回避される。 3 endpoint とも task を struct field ではなく `run` 関数内ローカル変数として保持するため、 task 自身の Drop impl 方式を採る (mp4 reader の `Mp4FileReader::Drop` は保持側 struct 経由で解決しているが、 3 endpoint の `RtmpInboundEndpoint` / `RtspSubscriber` / `SrtInboundEndpoint` は `#[derive(Clone)]` (RTSP) 等の制約や `run(self)` consume の設計から task を struct field 化しない)

### decoder task 骨子 (3 endpoint 共通の写経元)

各 endpoint 内 module-private として次を持たせる (0071 の `src/mp4/reader.rs:1528-1643` から `discard_mode_tx` と `TrackSender` を落とし、 `TrackPublisher` 直接に置換した形):

```rust
enum DecoderInput {
    Media(crate::MediaFrame),
    Eos,
}

#[derive(Debug)]
struct VideoDecoderTask {
    input_tx: tokio::sync::mpsc::UnboundedSender<DecoderInput>,
    // Drop trait と shutdown(self) を共存させるため Option で保持し take() で move する。
    // 直接 JoinHandle を持つと Drop 実装型の partial move が E0509 で禁止される。
    join_handle: Option<tokio::task::JoinHandle<crate::Result<()>>>,
}

impl VideoDecoderTask {
    async fn shutdown(mut self) -> crate::Result<()> {
        let _ = self.input_tx.send(DecoderInput::Eos);
        let handle = self
            .join_handle
            .take()
            .expect("join_handle is Some until shutdown/Drop consumes it");
        match handle.await {
            Ok(result) => result,
            Err(e) if e.is_panic() => {
                tracing::error!("video decoder task panicked: {e}");
                Err(crate::Error::new(format!(
                    "video decoder task panicked: {e}"
                )))
            }
            Err(e) => Err(crate::Error::new(format!(
                "video decoder task join failed: {e}"
            ))),
        }
    }
}

impl Drop for VideoDecoderTask {
    fn drop(&mut self) {
        // 早期 return / panic unwind 経路で task が leak しないよう abort する。
        // shutdown() が先に呼ばれていれば take 済みで None のため何もしない (二重 abort 回避)。
        if let Some(handle) = self.join_handle.take() {
            handle.abort();
        }
    }
}

fn spawn_video_decoder_task(
    options: crate::decoder::VideoDecoderOptions,
    mut stats: crate::stats::Stats,
    output_tx: crate::TrackPublisher,
) -> VideoDecoderTask {
    let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel::<DecoderInput>();
    // stats label は task 側で設定する責務に統一する (現状 3 endpoint の呼出側で
    // decoder_stats.set_default_label(...) を呼んでいる箇所は移行時に削除する)
    stats.set_default_label("component", "video_decoder");
    let join_handle = tokio::spawn(async move {
        video_decoder_loop(options, stats, input_rx, output_tx).await
    });
    VideoDecoderTask {
        input_tx,
        join_handle: Some(join_handle),
    }
}

async fn video_decoder_loop(
    options: crate::decoder::VideoDecoderOptions,
    stats: crate::stats::Stats,
    mut input_rx: tokio::sync::mpsc::UnboundedReceiver<DecoderInput>,
    mut output_tx: crate::TrackPublisher,
) -> crate::Result<()> {
    let mut decoder = crate::decoder::AsyncVideoDecoder::new(options, stats);
    loop {
        let input = match input_rx.recv().await {
            Some(input) => input,
            None => return Ok(()),
        };
        let is_eos = matches!(input, DecoderInput::Eos);
        match input {
            DecoderInput::Media(sample) => decoder.handle_input_sample_sync(Some(sample))?,
            DecoderInput::Eos => decoder.handle_input_sample_sync(None)?,
        }
        loop {
            match decoder.poll_output_sync()? {
                crate::decoder::DecoderRunOutput::Processed(sample) => {
                    if !output_tx.send_media(sample) {
                        return Ok(());
                    }
                }
                crate::decoder::DecoderRunOutput::Pending => break,
                crate::decoder::DecoderRunOutput::Finished => {
                    let _ = output_tx.send_eos();
                    return Ok(());
                }
            }
        }
        if is_eos {
            unreachable!("video decoder still pending after EOS");
        }
    }
}
```

`output_tx.send_media(sample)` false (pipeline closed) 経路で drain 未完了のまま return する挙動は現状同期版の `drain_video_decoder_output` = `DrainResult::PipelineClosed` 経路と同じ (現状も `send_media` false 相当で drain を中断)。 Nvcodec 使用時のメトリクス乖離リスク (`AsyncVideoDecoder` docstring 参照) は pipeline closed = 全体停止直前でしか発生せず、 実運用の観測性への影響は最小限とみなす。

なお `TrackPublisher::send_media` (`src/media_pipeline.rs:1276` → `send` `:1255-1266`) は pipeline が生きていて subscribers が空の場合でも `true` を返す (`drain_new_subscribers` `:1242-1244` の Empty 分岐)。 subscribe 未確定で decoder task から `send_media(sample)` を呼ぶとフレームは subscribers Vec に追加されないまま **静かに drop される**。 通常運用では endpoint 冒頭の `handle.wait_subscribers_ready().await?` で subscriber 準備完了を待ってから receive loop に進むため、 最初のフレームが decoder task 経由で送られる時点で subscribers は既に確定している (実運用で顕在化しない)。

decoder への入力構築は `crate::MediaFrame::new_video(video_frame)` を使う (現状 3 endpoint は `crate::MediaFrame::Video(std::sync::Arc::new(video_frame))` で構築しているが、 0071 / mp4 reader の慣習に揃えて `new_video` に統一する)。 両者は等価だが可読性向上のため統一する。

### 推奨案 §1 (RTMP: task と `TrackPublisher` の endpoint 寿命保持)

- `RtmpInboundEndpoint::run` 冒頭 (accept ループの外、 `handle.notify_ready()` / `handle.wait_subscribers_ready().await?` **より前** に順序変更する) で 1 回だけ:
    - `handle.publish_track(video_track_id.clone()).await?` で `TrackPublisher` を取得
    - `spawn_video_decoder_task(options, handle.stats(), video_track_tx)` を呼び `VideoDecoderTask` を run 内ローカル変数として保持
    - `handle.publish_track(audio_track_id.clone()).await?` (`audio_track_tx`) も同時に前倒して endpoint 側で保持

    現状 RTMP は `notify_ready` → `wait_subscribers_ready` → `publish_track` の順 (`:189-202`) だが、 spawn 時に `TrackPublisher` が必要なため RTSP / SRT と同じ `publish_track` → `notify_ready` → `wait_subscribers_ready` の順に前倒す。 pipeline 側 `handle_publish_track` (`src/media_pipeline.rs:373-422`) / `handle_notify_ready` / `handle_wait_subscribers_ready` は相互のガードを持たず順序独立で安全 (RTSP / SRT が既にこの順序で運用中)。
- `RtmpPublisherHandler` の field 変更:
    - **削除**: `video_track_tx: Option<TrackPublisher>` (task に move されているため handler は保持しない)
    - **削除**: `video_decoder: Option<VideoDecoder>`
    - **追加**: `video_decoder_input_tx: Option<tokio::sync::mpsc::UnboundedSender<DecoderInput>>` (task の `input_tx.clone()` を保持。 `UnboundedSender` は `Clone` 実装あり)
    - audio 側 (`audio_track_tx: Option<TrackPublisher>`, `audio_decoder: Option<AudioDecoder>`) は無変更
- `RtmpPublisherHandler::new` のコンストラクタ引数: `video_track_tx` と `video_decoder` の 2 引数を `video_decoder_input_tx: Option<UnboundedSender<DecoderInput>>` の 1 引数に置換 (9 → 8 引数、 `#[expect(clippy::too_many_arguments)]` allow は 8 でも clippy default (7) を超えるため維持)
- `RtmpPublisherHandler::into_parts` 戻り値: 4-tuple → 2-tuple。 tuple 順序ミスによる audio / video 誤配線を避けるため named struct を新設して名前解決する:
    ```rust
    struct RtmpPublisherHandlerAudioParts {
        audio_track_tx: Option<crate::TrackPublisher>,
        audio_decoder: Option<crate::decoder::AudioDecoder>,
    }
    ```
    into_parts はこの struct を返す。 呼出側 (`:243-252`) の 4-tuple destructuring も struct 分解パターンに書き換える:
    ```rust
    let RtmpPublisherHandlerAudioParts {
        audio_track_tx: restored_audio_track_tx,
        audio_decoder: restored_audio_decoder,
    } = handler.into_parts();
    audio_track_tx = restored_audio_track_tx;
    audio_decoder = restored_audio_decoder;
    ```
    video 側の `input_tx` は endpoint 側の task から `.input_tx.clone()` で毎接続作り直すため handler から回収しない。 コンストラクタ側の named struct 化 (`RtmpPublisherHandlerInputs`) は本 issue のスコープ外 (Round 2 で `into_parts` 側のみに絞った判断)
- 各接続 (`handler_new`) で `video_decoder_task.as_ref().map(|t| t.input_tx.clone())` を handler に渡す (`output_video_track_id.is_none()` の場合は `video_decoder_task` 自体が `None` で `None` を渡す。 `input_tx` field は module-private のまま、 同一 module 内 (endpoint の `run` 関数) から `task.input_tx.clone()` で直接アクセス可能)
- `handle_video_frame` (`:462-484`) の decoder 直呼出を次に置換:
    ```rust
    if let Some(tx) = self.video_decoder_input_tx.as_ref() {
        tx.send(DecoderInput::Media(crate::MediaFrame::new_video(video_frame)))
            .map_err(|_| crate::Error::new("video decoder task terminated unexpectedly"))?;
    }
    ```
- `RtmpInboundEndpoint::run` の accept ループを抜ける経路 (`listener.accept().await?` の Err で `return Err(e.into())`) の直前で `task.shutdown().await` を呼ぶ実装は tricky (`return Err(...)` 前に await を挟む必要)。 代わりに `impl Drop for RtmpInboundEndpoint` を追加して `task.join_handle.abort()` で強制終了 (現状 accept ループを抜ける経路は Err で endpoint 全体停止のため、 shutdown 完走で得られる benefit (metric 完走) は限定的)

**根拠 (endpoint 寿命保持を選ぶ理由)**:

- **`DuplicateTrackId` 制約**: `TrackPublisher::Drop` (`src/media_pipeline.rs:1213`) は `publisher_processor_id` を clear しない。 clear は `handle_unpublish_track` (`:424-445`) または `handle_deregister_processor` (`:469-489`) 経由のみ。 接続ごとに `TrackPublisher` を drop して次接続で `handle.publish_track(video_track_id)` を再呼出すると `DuplicateTrackId` (`:391-395`) で失敗する。 pipeline 側の unpublish + republish 対応まで拡張すると本 issue のスコープを超える
- **`send_eos` の下流影響**: `output_tx.send_eos()` は `Message::Eos` を pipeline に流し、 下流 mixer (`src/mixer/audio.rs:828`、 `src/mixer/video.rs:780`) が該当トラック終了として処理する。 接続ごとに task を spawn して shutdown で `send_eos` を発火する形にすると、 最初の RTMP 切断で pipeline が停止する。 endpoint 寿命保持なら shutdown = endpoint 停止時のみ発火するため、 mixer 側の挙動が現状と一致する
- **現状同期版との挙動一致**: 現状は decoder を接続跨ぎで保持しており、 SPS/PPS の切り替わりは decoder 内部で吸収されている (0068 / 0071 close 時点で運用問題報告なし)。 task 寿命を endpoint 寿命に揃えることで現状挙動を保つ (0071 の「seek / loop 継続経路で残 buffer 漏出を防ぐ」教訓は mp4 reader 固有の状態遷移で、 endpoint の接続切断 → 再接続とは semantics が異なる)

### 推奨案 §2 (RTSP: `SessionError` 3 経路別の decoder ライフサイクル)

`RtspSubscriber::run` の `session_result` 3 経路の扱い:

| 経路 | decoder task 扱い |
|------|------------------|
| `Ok(())` (session closed、 再接続) | **継続** (次 iteration でそのまま使う) |
| `Err(SessionError::Retryable(e))` (再接続) | **継続** (同上) |
| `Err(SessionError::Fatal(e))` (endpoint 停止) | `task.shutdown().await` (Err は `tracing::warn!` で握り潰し進行)、 その後 `return Err(e)` |

- `RtspSubscriber::run` 冒頭 (session ループの外、 現状の `publish_track` (`:89-98`) 位置はそのまま維持) で `spawn_video_decoder_task` を追加し、 `video_decoder_task: Option<VideoDecoderTask>` を run 内ローカル変数として保持
- `RtspSubscriber` struct 自身は `#[derive(Debug, Clone)]` (`:35`) を持つため `VideoDecoderTask` を struct field 化できない (`JoinHandle` は Clone 未実装)。 task は run 関数内ローカル変数として保持し、 task leak は §「決定事項」8 の `VideoDecoderTask::Drop` で担保する
- session ループの外側で `let mut video_decoder_input_tx: Option<UnboundedSender<DecoderInput>> = video_decoder_task.as_ref().map(|t| t.input_tx.clone());` を宣言し、 毎 iteration の `RtspOutputContext` に `&mut video_decoder_input_tx` を渡す (毎 iteration で clone を作り直さない)
- `RtspOutputContext<'a>` の field 変更:
    - **削除**: `video_decoder: &'a mut Option<VideoDecoder>`
    - **削除**: `video_track_tx: &'a mut Option<TrackPublisher>`
    - **追加**: `video_decoder_input_tx: &'a mut Option<tokio::sync::mpsc::UnboundedSender<DecoderInput>>`
    - audio 側 field は無変更
- `handle_rtp_packet` (`:657-763`) の video 側直呼出を次に置換:
    ```rust
    if let Some(tx) = output.video_decoder_input_tx.as_ref() {
        tx.send(DecoderInput::Media(crate::MediaFrame::new_video(video_frame)))
            .map_err(|_| SessionError::Fatal(Error::new("video decoder task terminated unexpectedly")))?;
    }
    ```
- unit test 3 箇所 (`:1676, :1715, :1760`) は `RtspOutputContext` 組み立てを新 field に置換。 事前宣言していた `let mut video_track_tx = None;` / `let mut video_decoder = None;` は不要になるため削除 (`unused_variables` 警告を避ける)。 代わりに `let mut video_decoder_input_tx = None;` を宣言して `&mut video_decoder_input_tx` を渡す

**Fatal 判断の根拠**: `input_tx.send` の Err は task 内で `poll_output_sync` の Err (Nvcodec / VideoToolbox 等の非同期 callback エラー、 openh264 の decode エラー) で task が終了した場合に生じる。 現状同期版は `drain_video_decoder_output` の Err を `SessionError::Fatal` に map (`:706`) しており挙動一致。 task の再 spawn 経路を持たない現設計では Retryable にすると次 session でも同じ Err で無限ループするため Fatal で endpoint 停止が妥当

### 推奨案 §3 (SRT: 同期クロージャ `process_polled_events` の async 対応) → **同期のまま維持 (unbounded_channel の同期 send を利用)**

- `process_polled_events` クロージャ (`:229-274`) と `publish_samples` fn (`:468-522`) は **同期のまま維持** する
- `publish_samples` の引数変更:
    - **削除**: `video_decoder: &mut Option<VideoDecoder>`、 `video_track_tx: &mut Option<TrackPublisher>`
    - **追加**: `video_decoder_input_tx: &mut Option<tokio::sync::mpsc::UnboundedSender<DecoderInput>>`
    - audio 側は無変更
- `publish_samples` の video 側直呼出 (`:508-517`) を次に置換:
    ```rust
    if let Some(tx) = video_decoder_input_tx.as_ref() {
        tx.send(DecoderInput::Media(crate::MediaFrame::new_video(frame)))
            .map_err(|_| crate::Error::new("video decoder task terminated unexpectedly"))?;
    }
    ```
- `SrtInboundEndpoint::run` の spawn は現状の `publish_track` 位置 (`:185-194`) を維持したまま、 その直後 (現状の stats 生成 `:196` より前、 少なくとも `handle.wait_subscribers_ready().await?` (`:227`) より **前**) に `spawn_video_decoder_task(...)` を追加する。 現状 SRT は `publish_track` (`:185-194`) → `notify_ready` (`:226`) → `wait_subscribers_ready` (`:227`) の順で既に整合しているため、 publish_track の位置移動は不要 (spawn 追加のみ)
- endpoint 停止経路 (`?` 早期 return) では `impl Drop for SrtInboundEndpoint` の `task.join_handle.abort()` で強制終了

**根拠**: option A (async 化) は clean だが波及範囲が広い。 option B (`try_send` + フレームドロップ) はフレームロスで change カテゴリ。 option C (`blocking_send`) は tokio runtime handle 問題。 `unbounded_channel::send` の同期非ブロッキング特性を使えば波及範囲ゼロで SRT の現状同期構造をそのまま維持できる

### 推奨案 §4 (SRT: `reset_connection_state` 経路の decoder task 扱い) → **継続保持 (RTSP と同型)**

- SRT の切断 (`ConnectionEvent::Disconnected` / `StateChanged(Disconnected)`) 時に `reset_connection_state` (`:524-534`) が呼ばれるが、 **decoder task は継続保持** する
- SRT は再接続時に MPEG-TS PMT / PES を demuxer 側 (`SrtTsDemuxer` 内 `last_video_sample_entry`) で再取得するため decoder 内部の SPS/PPS / keyframe 検出とは独立

## テスト戦略

### 既存テストへの影響

- **integration test 3 種** (`tests/rtmp_inbound_endpoint_tests.rs`, `tests/rtsp_subscriber_tests.rs`, `tests/srt_inbound_endpoint_tests.rs`): `new()` バリデーション網羅のみで decoder 経路を触らないため **影響なし**
- **RTSP unit test 3 箇所** (`src/rtsp/subscriber.rs:1676, :1715, :1760`): `RtspOutputContext` 組み立ての mechanical な更新 (`video_track_tx` / `video_decoder` field 削除 → `video_decoder_input_tx` field 追加、 事前宣言変数 (`video_track_tx`, `video_decoder`) の削除と新規宣言 (`video_decoder_input_tx = None`) の追加)。 テスト意図 (URL parse / depacketizer) は無変更
- **SRT unit test** (`src/srt/inbound_endpoint.rs:1091-`): MPEG-TS parse 系で decoder 経路を触らないため影響なし
- **e2e** (`tests/e2e.rs`): 3 endpoint はカバーしないため影響なし

### 新規テスト

`spawn_video_decoder_task` の task ライフサイクルを検証する unit test を各 endpoint の `#[cfg(test)] mod tests` に追加する。 モック / スタブ禁止 (shiguredo-rust) のため、 実 `AsyncVideoDecoder` (テスト用 `VideoDecoderOptions::default()`) + 実 `tokio::sync::mpsc::unbounded_channel` + 実 `TrackPublisher` (下流 subscriber なしで `send_media` false 経路が発火する状態、 または実 subscriber を組んで正常経路を通す) で組む:

1. `spawn_video_decoder_task` 直後の `shutdown().await` が Ok (`input_tx.send(Eos)` → `Finished` 到達 → `send_eos` → task Ok return) で完了する
2. 下流 subscriber なしの `TrackPublisher` に対して `Media` 投入時、 task が `Ok(())` で早期終了する (`send_media` false → return)

`poll_output_sync` の Err パス (Nvcodec / openh264 の実 codec エラー触発) の unit test は「モック禁止 + 実 codec が unit test スコープで異常入力を扱う保証がない」ため実装可能性が低い。 pipeline レベルの Err 挙動は integration test の追加ではなく `cargo test --workspace` の統合実行と実運用回帰で担保する (残懸念 §3)。

3 endpoint の受信ループ全体 (実 TCP/UDP 経由の `run()` の一連動作) の integration test は本 issue のスコープ外。

## 完了条件

### 3 endpoint 共通
- 各 endpoint 内 module-private として §「decoder task 骨子」の `enum DecoderInput` / `struct VideoDecoderTask` / `spawn_video_decoder_task` / `video_decoder_loop` が定義されている
- 各 endpoint の video 側 `handle_input_sample` 直呼出 / `drain_video_decoder_output` 直呼出が消えている
- audio 側 (`AudioDecoder::handle_input_sample`、 `drain_audio_decoder_output`) は無変更
- `input_tx.send` の Err は 3 endpoint 統一メッセージ `"video decoder task terminated unexpectedly"` で fatal error に map されている
- `impl Drop for VideoDecoderTask` が骨子に組み込まれ、 早期 return / panic unwind 経路での task leak を防いでいる (0071 の `Mp4FileReader::Drop` と同じ意図、 実装位置は task 保持側ではなく task 自身)
- `spawn_video_decoder_task` の task ライフサイクル (shutdown 正常経路、 pipeline closed 経路) の unit test が各 endpoint の `#[cfg(test)] mod tests` に追加されている

### RTMP
- `RtmpInboundEndpoint::run` 冒頭 (accept ループの外、 現状の `notify_ready` / `wait_subscribers_ready` (`:189-190`) より **前** に順序変更) で 1 回だけ `handle.publish_track(video_track_id).await?` + `spawn_video_decoder_task(options, handle.stats(), tx)` を呼び、 `video_decoder_task: Option<VideoDecoderTask>` を run 内ローカル変数として保持している
- `RtmpPublisherHandler::into_parts` の呼出側 (`:243-252`) の 4-tuple destructuring が `RtmpPublisherHandlerAudioParts` の struct pattern に書き換わっている
- `RtmpPublisherHandler` field: `video_track_tx` / `video_decoder` の 2 field が削除、 `video_decoder_input_tx: Option<UnboundedSender<DecoderInput>>` の 1 field が追加。 audio 側 field は無変更
- `RtmpPublisherHandler::new` のコンストラクタ引数が 9 → 8。 `#[expect(clippy::too_many_arguments)]` は維持
- `RtmpPublisherHandler::into_parts` は audio 側 2 field を持つ named struct (`RtmpPublisherHandlerAudioParts`) を返す (tuple 順序ミス防止)
- 各接続で `handler.new(..., video_decoder_input_tx=video_decoder_task.as_ref().map(|t| t.input_tx.clone()), ...)` を呼ぶ (task が None なら None)
- `handle_video_frame` の decoder 直呼出が `input_tx.send(...)` 経由に置換

### RTSP
- `RtspSubscriber::run` 冒頭 (session ループの外) で 1 回だけ `handle.publish_track(video_track_id).await?` + `spawn_video_decoder_task` を呼び、 `video_decoder_task: Option<VideoDecoderTask>` を run 内ローカル変数として保持
- `RtspOutputContext<'a>` の field: `video_decoder` / `video_track_tx` の 2 field が削除、 `video_decoder_input_tx: &'a mut Option<UnboundedSender<DecoderInput>>` の 1 field が追加。 audio 側 field は無変更
- `handle_rtp_packet` の video 側直呼出が `input_tx.send(...)` 経由に置換され、 Err が `SessionError::Fatal` に map されている
- `session_result` 3 経路のうち Fatal 経路のみ `task.shutdown().await` が呼ばれ、 Ok/Retryable 経路では task が継続保持されている
- unit test 3 箇所 (`:1676, :1715, :1760`) の `RtspOutputContext` 組み立てが新 field に置換され、 不要になった事前宣言変数 (`let mut video_track_tx = None;` 等) が削除されている

### SRT
- `SrtInboundEndpoint::run` 冒頭 (`handle.wait_subscribers_ready().await?` の前) で `handle.publish_track(video_track_id).await?` + `spawn_video_decoder_task` を呼び、 `video_decoder_task: Option<VideoDecoderTask>` を run 内ローカル変数として保持
- `publish_samples` (`:468-522`) の引数: video 側 2 引数 (`video_decoder`, `video_track_tx`) が削除、 `video_decoder_input_tx: &mut Option<UnboundedSender<DecoderInput>>` の 1 引数が追加
- `process_polled_events` クロージャ (`:229-274`) は同期のまま維持
- `publish_samples` 内で `input_tx.send(...)` が呼ばれ、 Err が `crate::Result<()>` で伝搬されている
- `reset_connection_state` (`:524-534`) では decoder task を触らない (継続保持)

### コマンド
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

### 準備段階

1. `src/rtmp/inbound_endpoint.rs` の module-private として §「decoder task 骨子」の `DecoderInput` / `VideoDecoderTask` / `spawn_video_decoder_task` / `video_decoder_loop` を追加 (`#[allow(dead_code)]` で警告抑制)
2. `src/rtsp/subscriber.rs` と `src/srt/inbound_endpoint.rs` にも同じ骨子を module-private でコピー (`#[allow(dead_code)]`)。 3 箇所コピーだが、 共通化は 0073 の未確定論点 4 に委ねる

### 移行段階 (endpoint ごと、 3 コミット推奨)

3. **RTMP** (`src/rtmp/inbound_endpoint.rs`):
    - `RtmpInboundEndpoint::run` 冒頭で `spawn_video_decoder_task` を追加
    - `RtmpPublisherHandler` の field 削除・追加、 コンストラクタ引数変更、 `into_parts` を named struct に変更
    - `handle_video_frame` を `input_tx.send(...)` 経由に書き換え
    - `#[allow(dead_code)]` を外す
    - task ライフサイクルの unit test を追加
4. **RTSP** (`src/rtsp/subscriber.rs`):
    - `RtspSubscriber::run` 冒頭で `spawn_video_decoder_task` を追加
    - `RtspOutputContext<'a>` の field 変更
    - `handle_rtp_packet` の video 側直呼出を書き換え
    - `session_result` の 3 経路分岐と `task.shutdown().await` を追加
    - unit test 3 箇所の `RtspOutputContext` 組み立てを更新
    - `#[allow(dead_code)]` を外す
    - task ライフサイクルの unit test を追加
5. **SRT** (`src/srt/inbound_endpoint.rs`):
    - `SrtInboundEndpoint::run` の現状 publish_track (`:185-194`) 直後に `spawn_video_decoder_task` を追加、 `video_decoder_task` と `video_decoder_input_tx` (`task.input_tx.clone()`) を run 内ローカル変数として宣言
    - `publish_samples` (`:468-522`) の引数変更 (`video_decoder` / `video_track_tx` 削除、 `video_decoder_input_tx` 追加)、 内部の video 側直呼出を `input_tx.send(...)` 経由に書き換え
    - `process_polled_events` クロージャ (`:229-274`) 内の `publish_samples` 呼出 2 箇所 (`:236-244` と `:248-256`) の引数リストも同時に書き換える
    - `#[allow(dead_code)]` を外す
    - task ライフサイクルの unit test を追加

各 step で `cargo check` を通せる中間状態を保つ。

### 仕上げ段階

6. §「コマンド」の全項目を通す

## 残懸念 (実装段階で prototype して確定させる項目)

1. **`unbounded_channel` の実運用時 OOM 耐性**: 3 endpoint はリアルタイム受信で decoder が上流速度に追いつかないケースは通常運用外だが、 極端な負荷 (低性能 CPU での 4K 入力等) で OOM が起きる可能性はある。 実測回帰があれば bounded + `try_send` + フレームドロップ (change カテゴリの別 issue) に切り替える判断材料になる。 RTSP の再接続時に前 session 由来の残 buffer が input_rx に残る場合の混入挙動、 SRT の `tsbpd_delay` と input_rx バッファの重複遅延も併せて実装後の cargo test 統合実行で観測する。 併せて **SRT 切断 → 再接続時の input_rx 残置** も観測する: 現状同期版では `demuxer.flush_pending()` 経由の残 PES は `reset_connection_state` の前に下流に到達済みだが、 spawn pattern 化後は `input_tx.send` 経由で decoder task の input queue に滞留するため、 `reset_connection_state` で demuxer / `connection_timestamp_offset` をリセットしても、 旧接続の残 frame は task 内で新接続の frame と FIFO で並ぶ (timestamp 非連続で下流に流れる可能性)。 現状同期版と挙動が異なる新規 semantic 変化。 実運用で問題化した場合は input_rx drain API 追加 or 切断時 flush 分の破棄 (挙動変更、 別 issue) で対応
2. **decoder task lifecycle unit test のスコープ**: `MediaPipeline::new` + `register_processor` + `publish_track` で実 `TrackPublisher` を組めることを実装段階で確認済み。 各 endpoint の本番停止経路に合わせて smoke test を追加:
    - **RTSP**: `shutdown_delivers_eos_to_subscriber` (Fatal 経路で本番使用する shutdown → Finished → `send_eos` → subscriber での `Message::Eos` 受信まで検証、 `send_eos` 消失の regression を catch)
    - **RTMP / SRT**: `drop_aborts_task` (本番唯一の停止経路である Drop → abort で task が終了して leak しないことを `AbortHandle::is_finished` + timeout で検証)。 graceful stop (`shutdown` / `DecoderInput::Eos`) は本番で使わないため YAGNI で削除済みで、 channel は `UnboundedSender<MediaFrame>` 直送の簡素版
    - **smoke test 対象外**: pipeline closed / panic 経路 (`send_media` false / task panic のシナリオ) は pipeline レベルの `cargo test` 統合実行で担保。 abort 時の subscriber への Eos 伝搬は pipeline 側 `drain_returned_subscribers` の「publisher 異常終了 → channel close → recv() が Eos を返す」契約に依拠。 Nvcodec 使用時のメトリクス乖離リスクは `AsyncVideoDecoder` docstring (`src/decoder.rs:380-383`) で明示された pre-existing 契約で、 endpoint 停止 = プロセス停止直前でしか発火せず観測性への影響は最小限
3. **RTSP Ok/Retryable 経路での task 死亡の early detection**: reconnect backoff (最大 5 秒) 中に task が dead になると、 次 session の初 RTP 到達まで検知が遅れる。 実運用で問題化するかは実測次第。 早期検知が必要な場合は select! に `join_handle` 監視を追加する (別 issue の候補)
4. **各 endpoint の module-private helper の共通化タイミング**: 実装の結果、 骨子は 2 系統に分岐した。 RTSP は graceful stop あり (`DecoderInput { Media, Eos }` + `shutdown`)、 RTMP / SRT は `UnboundedSender<MediaFrame>` 直送 + Drop abort のみの簡素版。 mp4 reader 版はさらに `discard_mode_tx` + `TrackSender` を持つ。 共通化する場合は両系統の差の吸収が論点になる (0073 の未確定論点 4 に追記済み)
5. **`handle_input` に相当する `AsyncVideoDecoder` API 追加の要否**: RTSP と mp4 reader の 2 箇所で match `DecoderInput { Media => handle_input_sample_sync(Some(_)), Eos => handle_input_sample_sync(None) }` が同型になる。 冗長と感じる場合は `AsyncVideoDecoder::handle_input(input: DecoderInput)` 相当の helper 追加を 0073 の共通化議論に合流させる

## CHANGES.md について

内部リファクタにつき記載不要。 inbound endpoint は library として外部公開していない (hisui は bin crate)。 外部プロトコル (RTMP / RTSP / SRT の受信挙動) は不変で、 decoder 経路の同期・非同期切替は endpoint 内部の実装詳細。

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): 親 issue、 `AsyncVideoDecoder` を導入
- closed/0068 (`feature/refactor-migrate-video-decoder-users-to-async`, 2026-07-03 close): 兄弟 issue。 subcommand_inspect + sora の 4 call site 単純置換。 `AsyncVideoDecoder::run` を追加した (本 issue の decoder task は再利用しない、 §「決定事項」1)
- closed/0071 (`feature/refactor-mp4-reader-async-video-decoder`, 2026-07-02 close): 兄弟 issue。 mp4 reader の spawn pattern 化。 本 issue の骨子は 0071 の `src/mp4/reader.rs:1528-1643` を参照実装として利用 (warm-up 機構と `TrackSender` を除外して写経)
- open/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`): 最終クリーンアップ。 同期 wrap `VideoDecoder` 削除 + `AsyncVideoDecoder` → `VideoDecoder` リネーム。 本 issue の完了を待つ。 本 issue の実装成果 (module-private helper の 3 箇所コピー) は 0073 の未確定論点 4 (spawn pattern 抽象の共通化) の判定材料になる
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 設計判断の親 issue。 採用案 C 「中途半端な 2 系統共存を残さない」原則との整合は 0073 で最終達成される
