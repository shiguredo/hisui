# VideoDecoder 系を Sender 出力に統一する

- Priority: Medium
- Created: 2026-06-26
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-video-decoder-sender-interface
- Polished:
- Reporter: @sile

## 目的

closed issue 0057 で確定した採用案 C (全エンコーダー / デコーダーを `tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>` 出力に統一) を **VideoDecoder 系に適用する** 先行 PR。decoder は encoder より単純 (RPC keyframe 経路なし / `flush()` 強制同期化なし / sample_entry 不変条件なし / メトリクス計上が `total_output_video_frame_count_metric` のみ) なので、本 issue で C 形式の interface パターンを実コードベースで確立し、後続 issue (encoder 側、本 issue の番号+1 想定) に展開する。

## 優先度根拠

Medium。

- closed issue 0057 で採用案が確定済みであり、設計検討フェーズは完了している。実装着手段階に入っているため Medium 維持
- decoder 側を先行する戦略的理由: encoder 側 (RPC keyframe + flush 撤廃 + sample_entry 不変条件 + メトリクス重) より単純な題材で C 形式 interface の実装可否を検証する。本 issue で実装困難が判明した場合は採用案 C を再検討 (closed 0057 §3 の案 A への後退トリガー) する弾力性ポイント

## 現状

`src/decoder.rs` および `src/decoder/*.rs` の各 inner は同期 pull 型 (詳細は closed issue 0057 「現状」§の表を参照):

- 上位 `VideoDecoder.decoded: VecDeque<VideoFrame>` (`src/decoder.rs:335`) と `poll_output()` (`src/decoder.rs:422-430`) で同期 pull
- 同期 inner (Libvpx / Openh264 / Dav1d) は `input_queue + output_queue` 構造 + `next_decoded_frame()` で pull
- 非同期 inner (VideoToolbox) は `decoded: Option<VideoFrame>` 単発 (`shiguredo_video_toolbox` 内で callback を `std::sync::mpsc::Sender` でチャネル化済み、上位は `next_frame()` で pull)
- 非同期 inner (Nvcodec) は hisui コードが `FnDecodeHandler` を直接実装し、`decoded_queue: Arc<Mutex<VecDeque>>` + `error_slot: Arc<Mutex<Option<Error>>>` で callback 結果を退避してから `handle_decoded_frames()` で pull (`src/decoder/nvcodec.rs:39-55, 221-228`)
- `VideoDecoderInner::Initial { options }` 遷移 (`src/decoder.rs:537-553`) で最初の入力フレーム到着時に実 decoder を生成
- `drain_video_decoder_output` ヘルパ (`src/decoder.rs:514-533`) が「内部 pull バッファ → `TrackPublisher::send_media`」を担う

これらを C 形式 (全 inner が Sender push 型に統一、上位 `run()` で Receiver を `tokio::select!` の腕で受ける) に書き換える。

## 設計方針

closed issue 0057 §3 の採用案 C「実装前提」に従う。decoder 固有の差分は以下:

- **RPC 経路なし**: decoder には `VideoEncoderRpcMessage` 相当の RPC が存在しないので、`tokio::select!` への腕追加は「入力 + Receiver」の 2 腕 (encoder の 3 腕より単純)
- **`flush()` 強制同期化の問題なし**: `NvcodecDecoder` は `decode()` 内で `flush()` を呼ばず `finish()` 時のみ呼ぶ (`src/decoder/nvcodec.rs:215`)。flush 撤廃という独立タスクは発生しない
- **メトリクス計上は最小限**: `total_output_video_frame_count_metric.inc()` のみ (`src/decoder.rs:415`)。keyframe 判定や sample_entry 不変条件 (closed/0027) は decoder にはない
- **Initial 遷移時の Sender 引き渡し**: `VideoDecoderInner::Initial { options }` を `Initial { options, sender }` に変更し、実 decoder 生成時に Sender を引き継ぐ
- **エラー伝搬**: `NvcodecDecoder` の `error_slot` を廃止し、callback 内 `Err` を `tx.blocking_send(Err(_))` (callback が tokio runtime 外で呼ばれるため) で即時通知

## 完了条件

- 下記「解決方法」の 1〜9 すべてが実装され、コードベースから旧構造 (`VideoDecoder.decoded`、`poll_output`、`next_decoded_frame` 系 dispatch、`drain_video_decoder_output`、`NvcodecDecoder::error_slot`) が消えている
- closed issue 0057 §3 の end-to-end テスト雛形相当のテスト (モック禁止規約 OK、実 decoder + tokio channel) が `src/decoder/nvcodec.rs` または `src/decoder.rs` に追加されている
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`

すべて通る。

## 解決方法

1. `VideoDecoderInner` の各 variant (`Libvpx` / `Openh264` / `Dav1d` / `VideoToolbox` / `Nvcodec`) を `tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>` 受け取り型コンストラクタに変更する。同期 inner は `decode()` を `async fn` 化して `tx.send(Ok(frame)).await?` を呼ぶ。非同期 inner は callback 内から `tx.blocking_send(...)` で push する
2. `VideoDecoder.decoded: VecDeque<VideoFrame>` を廃止し、`VideoDecoder` 内部に `tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>` を内蔵する (bounded、初期容量 N=8 推奨)
3. `VideoDecoder::poll_output()` (`src/decoder.rs:422-430`) と `VideoDecoderInner::next_decoded_frame()` 系 dispatch (`src/decoder.rs:706-717`) を廃止する
4. `VideoDecoder::run()` (`src/decoder.rs:360-391`) の `tokio::select!` に `decoded_rx.recv().await` 腕を追加し、受信した `Result<VideoFrame, Error>` を `output_tx.send_media()` に流す
5. `drain_video_decoder_output` ヘルパ (`src/decoder.rs:514-533`) を廃止し、上記 `run()` の Receiver 受信ループに統合する
6. `NvcodecDecoder` の `error_slot: Arc<Mutex<Option<Error>>>` (`src/decoder/nvcodec.rs:14-15, 36, 221-228`) を廃止し、callback 内 `Err` を `tx.blocking_send(Err(_))` で即時通知に変更する
7. `VideoDecoderInner::Initial { options }` (`src/decoder.rs:537-553`) を `Initial { options, sender }` に拡張し、実 decoder 生成 (`initialize_decoder` 内、`src/decoder.rs:555-668`) で sender を実 decoder に引き継ぐ
8. メトリクス計上 (`total_output_video_frame_count_metric.inc()`、現状 `src/decoder.rs:415`) を `run()` の Receiver 受信ループ内に移植する
9. 各 decoder 末尾テストおよび `src/decoder.rs:720-821` のエンジン選択テスト (`vp9_without_size_skips_video_toolbox` 等) を Sender 形式に書き換える。`#[tokio::test]` で `tokio::sync::mpsc::channel(N)` を作って Sender を渡し、Receiver でアサートする形に統一する

## CHANGES.md について

内部リファクタにつき記載不要。`VideoDecoder` 系は library として外部公開していないため、API 変更の後方互換影響は obsws coordinator / mixer / writer 等の crate 内利用箇所のみ。

## 関連

- closed/0057 (`feature/refactor-callback-friendly-codec-interface`): 設計検討の親 issue。本 issue は §3 採用案 C の decoder 部分実装
- closed/0027 (`feature/refactor-video-sample-entry-all-frames`): sample_entry 不変条件 (encoder 側のみ。decoder には適用外だが、後続 encoder issue で再確認)
- 後続 encoder 側 issue (本 issue の番号+1 想定): 本 issue で確立した C 形式 interface を encoder に展開する
