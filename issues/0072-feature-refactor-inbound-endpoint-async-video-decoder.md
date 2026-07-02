# RTMP / RTSP / SRT inbound endpoint の video decoder 経路を spawn pattern に再設計する

- Priority: Medium
- Created: 2026-07-02
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-inbound-endpoint-async-video-decoder
- Polished:

## 目的

closed issue 0066 で `AsyncVideoDecoder` が導入され、 同期 `VideoDecoder` は wrap 構造で挙動維持されている。 本 issue は使用側移行のうち **RTMP / RTSP / SRT の 3 つの inbound endpoint** を `AsyncVideoDecoder` ベースの spawn pattern に再設計する。

現状 3 endpoint は「同期 `VideoDecoder` を保持 + 受信ループで `handle_input_sample` + `drain_video_decoder_output` を直叩き」の構造だが、 decoder の所有パターンは 3 者三様で「移行 (migrate)」ではなく「受信ループの再設計 (refactor)」が主眼になる:

- **rtmp** (`src/rtmp/inbound_endpoint.rs`): `RtmpInboundEndpoint::run` 内のローカル変数を、 接続ごとに `RtmpPublisherHandler` に `take()` で move し、 切断時に `handler.into_parts()` で `restored_video_decoder` を回収して次接続に使い回す (接続跨ぎで decoder は長寿命)
- **rtsp** (`src/rtsp/subscriber.rs`): `RtspSubscriber::run` 内のローカル変数を `RtspOutputContext<'a>` に `&mut Option<VideoDecoder>` として借用で渡す
- **srt** (`src/srt/inbound_endpoint.rs`): `SrtInboundEndpoint::run` 内のローカル変数を receive path に `&mut Option<VideoDecoder>` として借用で渡す。 `process_polled_events` は同期クロージャで `publish_samples` (同期 fn) を呼ぶ

`AsyncVideoDecoder` 化では 3 者を「spawn task で decoder を持ち、 main は `mpsc::Sender<Message>` で input を送る」統一 pattern に置換する。 単純な call site 置換とは性質が異なる (受信ループ全体の設計変更、 SRT 同期クロージャの async 化、 rtmp 接続跨ぎ再利用の spawn pattern 化) ため、 open issue 0068 の polish で分離された。

## 優先度根拠

Medium。

- closed issue 0066 の wrap 段階的移行方針 (δ) を、 closed issue 0057 §3 「中途半端な 2 系統共存を残さない」原則と整合させるには全使用側移行が必要。 本 issue はその 1 パート
- 本 issue 単独では外部挙動 (受信 → publish のセマンティクス) は不変。 内部 refactor 相当で緊急性なし
- 後続の open issue 0073 (最終クリーンアップ) は open issue 0068 / 0071 / 本 issue の全完了を待つ。 本 issue が完了しないと 0073 で `VideoDecoder` の名前を消せない
- 実装難所 (接続跨ぎパターン、 再接続時ライフサイクル、 SRT 同期クロージャの async 化) が集中しており、 open issue 0068 と同居させると polish しきれない事情から分割された

## 現状

### RTMP (`src/rtmp/inbound_endpoint.rs`)

- `RtmpInboundEndpoint::run` 内で decoder 生成: `let mut video_decoder = ... crate::decoder::VideoDecoder::new(...)` (`:164-176`、 `VideoDecoder::new` 呼出 `:167`)
- 接続受け付け時: `video_decoder.take()` を `RtmpPublisherHandler::new` に渡す (`:227`)
- 切断時: `handler.into_parts()` で `restored_video_decoder` を回収 (`:246-247`) → `video_decoder = restored_video_decoder;` (`:251`) で次接続に引き継ぎ
- `RtmpPublisherHandler` 構造体:
  - field 宣言 `video_decoder: Option<crate::decoder::VideoDecoder>` (`:304`)
  - コンストラクタ引数 (`:321`)、 `into_parts` 戻り値型 (`:372-373`) / return 値 (`:378-379`)
- 受信ループ内 (`:473`): `decoder.handle_input_sample(Some(crate::MediaFrame::Video(...)))?`
- 受信ループ内 (`:477`): `crate::decoder::drain_video_decoder_output(decoder, tx)?`

decoder が接続を跨いで生きる設計は「keyframe 待ちや decoder 内部バッファを保持したまま再接続後もフレーム連続で流す」意図と推測される。

### RTSP (`src/rtsp/subscriber.rs`)

- `RtspSubscriber::run` 内で decoder 生成: `Some(crate::decoder::VideoDecoder::new(...))` (`:107`)
- `RtspOutputContext<'a>` に借用で渡す (field `video_decoder: &'a mut Option<crate::decoder::VideoDecoder>` `:281`)
- 受信ループ内: `handle_input_sample` 呼出 (`:700`)、 `drain_video_decoder_output` 呼出 (`:705`)
- 再接続経路 (`SessionError::Retryable`): decoder はローカル変数のまま生存 (継続)

### SRT (`src/srt/inbound_endpoint.rs`)

- `SrtInboundEndpoint::run` 内で decoder 生成 (`:204` 付近の `VideoDecoder::new`)
- receive path に `&mut Option<VideoDecoder>` で借用 (`:473`)
- `process_polled_events` は **同期クロージャ** (`:229-244` 付近、 `|conn, peer_addr| -> Result<()>`) として定義され、 その内側から同期 fn `publish_samples` (`:468` 付近) を呼ぶ
- 受信ループ内: `handle_input_sample` 呼出 (`:508`)、 `drain_video_decoder_output` 呼出 (`:512`)

SRT は同期クロージャ内から decoder への送信を行うため、 `.send().await` を導入すると `process_polled_events` / `publish_samples` の両方の async 化波及が発生する。 借用ライフタイム (`conn: &mut SrtConnection`, `peer_addr: &mut Option<SocketAddr>`) と async クロージャの絡みでコンパイル通過に非自明な工夫が要る可能性あり。

### 現状の背圧

3 endpoint とも受信ループ内で `handle_input_sample` (同期) を呼び、 その直後の `drain_video_decoder_output` (同期) で出力を publish する pull ループ構造。 decoder 内部でキューイングが完結しており、 receive path 側からは背圧が効いていない。 spawn pattern 化後は decoder task の入力 channel に背圧の粒度が移る。

### AsyncVideoDecoder の現状 API

`src/decoder.rs` (0066 完了時点) が提供する API:

- `pub fn AsyncVideoDecoder::new(options, stats) -> Self` (`:400`)
- `pub fn handle_input_sample_sync(&mut self, Option<MediaFrame>) -> Result<()>` (`:424`)
- `pub fn poll_output_sync(&mut self) -> Result<DecoderRunOutput>` (`:441`)
- `pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>>` (`:472`)

`handle_input_message` (Message enum dispatch) と `run` (`ProcessorHandle` ベース) は同期 wrap `VideoDecoder` のみに存在し `AsyncVideoDecoder` 側には未実装。 本 issue の spawn クロージャ内では `Message::Media / Eos / Syn(_)` を自前で `handle_input_sample_sync` に dispatch する形で回避可能 (もしくは open issue 0068 で `AsyncVideoDecoder::handle_input_message` の追加が確定されれば利用する)。

## 設計方針

### 統一 pattern (骨子)

```
main task (受信ループ):
    let (decoder_input_tx, decoder_input_rx) = mpsc::channel::<Message>(N);
    let decoder_join_handle = tokio::spawn(async move {
        let mut async_decoder = AsyncVideoDecoder::new(options, stats);
        while let Some(message) = decoder_input_rx.recv().await {
            match message {
                Message::Media(sample) => async_decoder.handle_input_sample_sync(Some(sample))?,
                Message::Eos => async_decoder.handle_input_sample_sync(None)?,
                Message::Syn(_) => {} // 現状 decoder は Syn を無視する
            }
            while let Some(result) = async_decoder.next_decoded_frame_async().await {
                output_tx.send(result?);
            }
        }
        Ok::<_, crate::Error>(())
    });
    // 受信側: decoder_input_tx.send(Message::Media(frame)).await?
    // 終了時: drop(decoder_input_tx); join_handle.await??;
```

3 endpoint はこの骨子を共有する。 endpoint 固有の差分は下記の未確定論点で確定させる。

### 未確定論点 (polish で確定させる)

以下は本 issue 起票時点で意図的に選択肢を残している。 `/polish-issue 72` 段階で 1 案に絞り込む:

1. **RTMP: 接続跨ぎ decoder 再利用の実現方針**
    - option A: per-connection spawn / 切断時 EOS 送信 + `join_handle.await` / 次接続で新 spawn
    - option B: endpoint 全体で decoder task を長生きさせ、 接続開始 / 終了は Message で通知 (`Message::Connect` / `Message::Disconnect` に相当する制御 message の新設)
    - 現状の「decoder 内部バッファを接続跨ぎで保持する」意図をどこまで保つか
2. **RTSP: `SessionError::Retryable` 経路での decoder ライフサイクル**
    - 継続 (現状挙動、 decoder task を殺さない)
    - リセット (task を落として再生成)
    - EOS+join+再 spawn (mp4 reader 側 open issue 0071 と同じ)
    - keyframe 待ち状態が再接続前後で引き継がれるかの意味論確定
3. **SRT: 同期クロージャ `process_polled_events` の async 対応**
    - option A: `process_polled_events` / `publish_samples` を async 化 (クロージャ→async クロージャ、 fn→async fn)、 借用ライフタイムを維持
    - option B: `publish_samples` は同期のまま、 `mpsc::Sender::try_send` で背圧代替 (溢れた場合はフレームドロップ)
    - option C: `blocking_send` (tokio runtime handle が必要)
4. **decoder task 入力 channel の bounded/unbounded と型**
    - `tokio::sync::mpsc::channel::<Message>(N)` の `N` (bounded 採用時) を何にするか
    - `unbounded_channel` を使うか (0066 で decoder 内部は unbounded に統一済み)
    - `Message` (`crate::Message` の Media / Eos / Syn) をそのまま流すか、 decoder 専用 enum (`DecoderInput` = Media / Eos のみ) を新設するか
5. **decoder task の出力経路の統一**
    - option A: decoder task が直接 `output_tx: TrackPublisher` に流す (`TrackPublisher` を task に move)
    - option B: decoder task は output channel (`mpsc::Receiver<Result<VideoFrame>>`) を返し、 main が別途取り出して publish
    - option C: decoder task の中で `next_decoded_frame_async().await` を回して `output_tx.send()` を直接呼ぶ (骨子で示した形)
6. **decoder task のエラー / panic 伝搬**
    - `JoinHandle::await` の `Err(JoinError)` (panic) → `crate::Error::new(format!("video decoder task panicked: {e}"))` として fatal 化
    - `output_tx.send()` 失敗 (下流 pipeline closed) → task を `Ok(())` で正常終了
    - 内部 async エラー (`Some(Err(e))`) → task から `Err(e)` を返却 → 親側で pipeline を止める
    - decoder task 死亡を receive path が検出する経路の確定

### 決定事項 (polish で覆さない前提)

- `AsyncVideoDecoder` は 0066 で導入済みのものを利用 (再設計しない)
- 各 inner の Sender 化は 0066 で完了済み (`OutputSink` 経由)
- decoder 内部 channel は unbounded (0066 確定)
- 3 endpoint すべてで統一 pattern (spawn + `mpsc::Sender<Message>` + `JoinHandle`) を採用

### shiguredo-rust 規約整合

- モック / スタブ不使用 (テストは実 endpoint + 実 decoder + tokio runtime)
- `#[non_exhaustive]` 不使用
- 新規 trait 追加なし

## 完了条件

- RTMP: `RtmpPublisherHandler` の `video_decoder: Option<VideoDecoder>` field / コンストラクタ引数 / `into_parts` 戻り値型が spawn pattern (`decoder_input_tx: Option<mpsc::Sender<Message>>` + `decoder_join_handle: Option<JoinHandle<crate::Result<()>>>`) に置換されている
- RTMP: 接続跨ぎ decoder 使い回しの意味論が確定した方針 (§未確定論点 1) で実装され、 動作確認できている
- RTSP: `RtspOutputContext` の `&mut Option<VideoDecoder>` が spawn pattern の借用に置換され、 再接続時のライフサイクル (§未確定論点 2) が実装されている
- SRT: 同期クロージャ `process_polled_events` / 同期 fn `publish_samples` が §未確定論点 3 の確定案で書き換わっている
- 3 endpoint 共通で `decoder.handle_input_sample(...)` 直呼出 / `drain_video_decoder_output(...)` 直呼出が消えている
- decoder task の panic / エラー時の伝搬経路が §未確定論点 6 の確定案で実装され、 テスト可能な範囲でカバーされている
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

実装着手時の推奨手順 (詳細は polish で確定):

1. 設計方針 §「未確定論点」の 6 論点を実装着手前に polish で確定
2. 3 endpoint 共通のヘルパ (spawn helper、 EOS + join helper、 エラー伝搬 helper) を先に用意
3. **RTSP から着手** (再接続ループが比較的シンプルで pattern を確立しやすい)
4. **SRT** (同期クロージャの async 化を含む波及作業)
5. **RTMP** (接続跨ぎ再利用の spawn pattern 化が最複雑)
6. 各 endpoint で回帰テスト実行、 特に RTMP の切断 → 再接続シナリオを重点確認
7. `cargo fmt` / `cargo check` (default + `--no-default-features`) / `cargo clippy` / `cargo test` 全通過

各 step で `cargo check` を通せる中間状態を保つ。

## CHANGES.md について

内部リファクタにつき記載不要。 inbound endpoint は library として外部公開していない (hisui は bin crate)。 API 変更の影響は crate 内利用箇所のみ。 外部プロトコル (RTMP / RTSP / SRT の受信挙動) は不変。

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): 親 issue、 `AsyncVideoDecoder` を導入した
- open/0068 (`feature/refactor-migrate-video-decoder-users-to-async`): 兄弟 issue。 `src/subcommand_inspect.rs` / `src/sora/recording_subcommand_compose.rs` / `src/sora/recording_subcommand_vmaf.rs` の単純 call site 置換 3 ファイルを扱う。 0068 の polish 過程で本 issue が分離された
- open/0071 (`feature/change-mp4-reader-async-video-decoder`): mp4 reader 側の async 化。 本 issue と互いに独立で並行実施可能
- open/0073 予定: 最終クリーンアップ (同期 `VideoDecoder` 削除 + `AsyncVideoDecoder` を `VideoDecoder` にリネーム)。 本 issue の完了を待つ
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 設計判断の親 issue。 採用案 C 「中途半端な 2 系統共存を残さない」原則との整合は 0073 で最終達成される
