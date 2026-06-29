# VideoEncoder 系を Sender 出力に統一する

- Priority: Medium
- Created: 2026-06-26
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-video-encoder-sender-interface
- Polished:
- Reporter: @sile

## 目的

closed issue 0057 で確定した採用案 C (全エンコーダー / デコーダーを `tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>` 出力に統一) を **VideoEncoder 系に適用する** 後続 PR。issue 0066 (decoder 側) で確立した C 形式 interface パターンを encoder 側に展開しつつ、encoder 固有の要件 (RPC keyframe 経路 / `NvcodecEncoder` の `flush()` 強制同期化撤廃 / sample_entry 不変条件 / `error_slot` 廃止 / メトリクス計上の `run()` 受信側移植) をまとめて解決する。

## 優先度根拠

Medium。

- closed issue 0057 で採用案が確定済みで、Medium 維持の中核理由 (NVENC の非同期パイプライン並列性回復) を実現するには本 issue の `flush()` 強制同期化撤廃が必須
- 依存先: issue 0066 (decoder 側) 完了後に着手。0066 で C 形式 interface が成立しないと判明した場合は本 issue も再検討 (closed 0057 §3 採用案 C の案 A への後退トリガー)

## 現状

`src/encoder.rs` および `src/encoder/*.rs` の各 inner は同期 pull 型 (詳細は closed issue 0057 「現状」§の表を参照):

- 上位 `VideoEncoder.encoded: VecDeque<VideoFrame>` (`src/encoder.rs:435`) と `poll_output()` (`src/encoder.rs:734-742`) で同期 pull
- 同期 inner (Libvpx / Openh264 / SvtAv1) は `input_queue + output_queue` 構造 + `next_encoded_frame()` で pull (Openh264 は `encoded: Option<VideoFrame>` 単発)
- 非同期 inner (VideoToolbox) は `input_queue + output_queue` 構造 (`shiguredo_video_toolbox` 内で callback を `std::sync::mpsc::Sender` でチャネル化済み、上位は `next_frame()` で pull)
- 非同期 inner (Nvcodec) は hisui コードが `FnEncodeHandler` を直接実装し、`encoded_queue: Arc<Mutex<VecDeque>>` + `error_slot: Arc<Mutex<Option<Error>>>` で callback 結果を退避してから `handle_encoded_frames()` で pull (`src/encoder/nvcodec.rs:45-60, 271-278`)
- `NvcodecEncoder::encode()` は worker 完了を待つため `flush()` を毎フレーム強制呼び出ししており (`src/encoder/nvcodec.rs:248-256`)、NVENC の非同期パイプライン並列性が潰されている
- RPC keyframe 経路: `request_upstream_video_keyframe` (`src/encoder.rs:381-424`) と `VideoEncoderRpcMessage::RequestKeyframe` (`src/encoder.rs:372-375`) が `run()` の `tokio::select!` 入口に存在 (`src/encoder.rs:632-658`)。受信時 `keyframe_request_pending = true` 設定 → 次の input フレーム到着時に `inner.request_keyframe()` 呼出 (`src/encoder.rs:694-699`)
- メトリクス計上は `push_encoded_frame_with_metrics` (`src/encoder.rs:724-732`) で `total_output_video_frame_count_metric` + `total_output_video_keyframe_count_metric` + sample_entry 不変条件 (closed/0027、`src/encoder.rs:729-730` のコメント) を担保
- `drain_video_encoder_output` ヘルパ (`src/encoder.rs:745-764`) が「内部 pull バッファ → `TrackPublisher::send_media`」を担う

これらを C 形式 (全 inner が Sender push 型、上位 `run()` で Receiver を `tokio::select!` の腕で受ける) に書き換える。

## 設計方針

closed issue 0057 §3 採用案 C「実装前提」に従う。issue 0066 (decoder) で確定する Sender 型および enum dispatch 形式を踏襲する。encoder 固有の差分は以下:

- **RPC keyframe 経路の維持**: 現状の「RPC 受信 → `keyframe_request_pending = true` → 次の input フレーム到着時に `inner.request_keyframe()` 呼出」経路は変更しない。`run()` の `tokio::select!` は「入力 + RPC + Receiver」の 3 腕に拡張
- **`NvcodecEncoder` の `flush()` 強制同期化撤廃**: bounded `tokio::sync::mpsc::channel(N)` のバックプレッシャによって "encoder buffer is full" を防ぐ。`encode()` 直後の `self.inner.flush()` 呼出 (`src/encoder/nvcodec.rs:254`) を撤廃し、callback ハンドラから `tx.blocking_send(...)` で push する
- **sample_entry 不変条件 (closed/0027) の維持**: `VideoFrame` 構造体に `sample_entry: Option<Arc<...>>` が既に乗っているので、Sender 経由で流れるフレームに自動的に含まれる。維持責任は inner 側 (フレーム生成時に sample_entry を埋める)
- **メトリクス計上を `run()` 受信側に移植**: `push_encoded_frame_with_metrics` 相当 (`total_output_video_frame_count_metric.inc()` + keyframe フラグ判定 + `total_output_video_keyframe_count_metric.inc()`) を `run()` の Receiver 受信ループに移植する
- **エラー伝搬**: `NvcodecEncoder` の `error_slot` を廃止し、callback 内 `Err` を `tx.blocking_send(Err(_))` で即時通知 (decoder 側 = issue 0066 と同じ方式)
- **遅延初期化**: `VideoEncoder.inner: Option<VideoEncoderInner>` の遅延初期化 (`initialize_inner` `src/encoder.rs:480-503`) で最初のフレームから初期化する構造は維持。Sender は `VideoEncoder::new` 時点で確定し、`initialize_inner` 時に inner に渡す

## 完了条件

- 下記「解決方法」の 1〜10 すべてが実装され、コードベースから旧構造 (`VideoEncoder.encoded`、`poll_output`、`next_encoded_frame` 系 dispatch、`drain_video_encoder_output`、`drain_encoded_frames`、`push_encoded_frame_with_metrics`、`NvcodecEncoder::error_slot`、`NvcodecEncoder::encode()` 内の `self.inner.flush()` 呼出) が消えている
- `NvcodecEncoder` で `flush()` 撤廃後も "encoder buffer is full" にならないことを実機で確認 (固定素材 1080p30 / 60 秒で実際に compose を走らせて完走確認)
- 上記実機計測の前後 (flush あり / 撤廃後) で wall-clock 時間と p99 frame latency を比較して closed issue 0057 §3 採用基準の暫定値 (wall-clock 短縮 ≥ 15%、p99 改善 ≥ 5ms) を満たすことを確認。下回った場合は数値とともに残懸念として本 issue に追記
- closed issue 0057 §3 の end-to-end テスト雛形相当のテスト (モック禁止規約 OK、実 encoder + tokio channel) が `src/encoder/nvcodec.rs` または `src/encoder.rs` に追加されている
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`

すべて通る。

## 解決方法

1. `VideoEncoderInner` の各 variant (`Libvpx` / `Openh264` / `SvtAv1` / `VideoToolbox` / `Nvcodec`) を `tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>` 受け取り型コンストラクタに変更する。同期 inner は `encode()` を `async fn` 化して `tx.send(Ok(frame)).await?` を呼ぶ。非同期 inner は callback 内から `tx.blocking_send(...)` で push する
2. `VideoEncoder.encoded: VecDeque<VideoFrame>` を廃止し、`VideoEncoder` 内部に `tokio::sync::mpsc::Sender<crate::Result<VideoFrame>>` を内蔵する (bounded、初期容量 N=8 推奨、計測で調整可)
3. `VideoEncoder::poll_output()` (`src/encoder.rs:734-742`) と `VideoEncoderInner::next_encoded_frame()` 系 dispatch (`src/encoder.rs:862-872`) を廃止する
4. `VideoEncoder::run()` (`src/encoder.rs:632-658`) の `tokio::select!` に `encoded_rx.recv().await` 腕を追加し、受信した `Result<VideoFrame, Error>` を `output_tx.send_media()` に流す (既存の入力腕 + RPC 腕に追加した 3 腕構成)
5. `drain_video_encoder_output` ヘルパ (`src/encoder.rs:745-764`) と `drain_encoded_frames` (`src/encoder.rs:714-722`) を廃止し、上記 `run()` の Receiver 受信ループに統合する
6. `push_encoded_frame_with_metrics` (`src/encoder.rs:724-732`) で行っている `total_output_video_frame_count_metric.inc()` / `total_output_video_keyframe_count_metric.inc()` / sample_entry 不変条件チェック (closed/0027) を、`run()` の Receiver 受信ループに移植する
7. `NvcodecEncoder::encode()` (`src/encoder/nvcodec.rs:202-257`) 内の `self.inner.flush()` 呼出 (`src/encoder/nvcodec.rs:254`) を撤廃する。bounded channel のバックプレッシャによって "encoder buffer is full" を防ぐ
8. `NvcodecEncoder` の `error_slot: Arc<Mutex<Option<Error>>>` (`src/encoder/nvcodec.rs:16, 27, 271-278`) を廃止し、callback 内 `Err` を `tx.blocking_send(Err(_))` で即時通知に変更する
9. RPC keyframe 経路 (`VideoEncoderRpcMessage::RequestKeyframe` + `request_upstream_video_keyframe`) は現状の挙動 (受信時 `keyframe_request_pending = true` → 次の input フレーム到着時 `inner.request_keyframe()` 呼出) を維持する
10. 各 encoder 末尾テスト (`src/encoder/libvpx.rs` / `openh264.rs` / `svt_av1.rs` / `video_toolbox.rs` / `nvcodec.rs`) および `src/encoder/test_helpers.rs` を Sender 形式に書き換える。`#[tokio::test]` で `tokio::sync::mpsc::channel(N)` を作って Sender を渡し、Receiver でアサートする形に統一する

## CHANGES.md について

内部リファクタにつき記載不要。`VideoEncoder` 系は library として外部公開していないため、API 変更の後方互換影響は obsws coordinator / mixer / writer 等の crate 内利用箇所のみ。

ただし `NvcodecEncoder` の `flush()` 強制同期化撤廃によって NVENC の非同期パイプライン並列性が回復するため、実機計測で有意な性能改善 (例: wall-clock 短縮 ≥ 15%) が確認された場合は `[UPDATE]` で「nvcodec エンコーダーの非同期パイプライン並列性を回復させて wall-clock 時間を X% 短縮した」旨を記載することを検討する (実装段階で Decision Owner = @sile が判断)。

## 関連

- closed/0057 (`feature/refactor-callback-friendly-codec-interface`): 設計検討の親 issue。本 issue は §3 採用案 C の encoder 部分実装
- open/0066 (`feature/refactor-add-async-video-decoder`): decoder 側の先行 PR。本 issue は 0066 完了後に着手する
- closed/0027 (`feature/refactor-video-sample-entry-all-frames`): 「映像エンコーダは全出力フレームに sample_entry を載せる」不変条件。本 issue の Receiver 受信ループで維持責任を保つ
- closed/0030 (`feature/refactor-encoded-frame-sample-entry-invariant`): エンコード済みフレームの sample_entry 必須化。同上
- closed/0051 (`feature/refactor-remove-writer-sample-entry-fallback`): writer 入口の不変条件 (圧縮フレームの sample_entry は必ず `Some`)。本 issue の Receiver 受信ループで違反しない設計を維持
- closed/0054 (`feature/refactor-encoder-defer-output-until-sample-entry-ready`): エンコーダーで sample_entry 未確定時の出力を `Err` 化する fail-fast 整備。本 issue で callback 経路に切り替えても fail-fast を維持できるよう、Sender に `Result` 型を流す形に統一する
