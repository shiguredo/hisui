# AsyncVideoEncoder を追加し VideoEncoder を内部 channel ベースに改修する

- Priority: Medium
- Created: 2026-06-26
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-add-async-video-encoder
- Polished: 2026-07-07
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0057 §3 で確定した採用案 C (全エンコーダー / デコーダーを Sender 経由の出力に統一) を、 closed issue 0066 で確立された派生方針 (δ) 「Async* 新規追加 + 既存を wrap 化 + 段階移行」と同型に、 VideoEncoder 系に適用する初弾。

具体的には:

1. **`AsyncVideoEncoder` を新規追加**: 内部 channel から `recv().await` でフレームを受け取る非同期インターフェース
2. **既存 `VideoEncoder` (同期) を `AsyncVideoEncoder` の wrapper として再構築**: 内部に `AsyncVideoEncoder` を保持し、 同期 API (`handle_input_message` / `handle_input_sample` / `poll_output` / `run`) は `AsyncVideoEncoder` への delegate として実装。 **外部 API 挙動は維持** し、 既存使用側の書き換えは不要
3. **inner 層 (`VideoEncoderInner` enum と各 variant) は `AsyncVideoEncoder` が保持**: 各 inner (`LibvpxEncoder` / `Openh264Encoder` / `SvtAv1Encoder` / `VideoToolboxEncoder` / `NvcodecEncoder`) はコンストラクタで `OutputSink` (`tx` + `total_output_metric` + `total_output_video_keyframe_count_metric` まとめ struct) を受け取って内包。 `encode()` / `finish()` は同期 fn のままで、 内部で出力フレームを `self.sink.emit_ok(frame)` で push する
4. **`NvcodecEncoder` の `error_slot` を廃止**: callback 内 `Err` を `sink.emit_err()` で即時通知 (decoder 側 0066 と同じ方式)
5. **メトリクス計上を `OutputSink` 内でペアリング**: `total_output_video_frame_count_metric` + keyframe 判定 + `total_output_video_keyframe_count_metric` を `emit_ok` 内で物理的に強制ペアリング (0066 と同じ設計)

本 issue の範囲は「`AsyncVideoEncoder` 追加 + 既存 `VideoEncoder` の wrapper 化 + inner の Sender 化 + `error_slot` 廃止 + メトリクス移植」までで、 使用側移行 / wrap 削除 + rename / 未使用 API 削除 / `NvcodecEncoder::flush()` 撤廃 + bp 機構 は別 issue で段階的に実施する (詳細は §後続実装 issue の分割 参照)。

以下は本 issue でも後続 issue でも扱わない:

- **RPC keyframe の keyframe 適用遅延**: `handle_rpc_message` (`src/encoder.rs:668-670`) の「低フレームレート入力で遅延し得る」既知問題は現状維持
- **Audio 系**: closed/0057 「スコープ」§で対象外と明示済み。 内部 API が同期 push 型で callback ABI 経路が存在せず、 再設計動機が成立しない

closed/0057 §3 採用案 C の「中途半端な 2 系統共存を残さない」原則との整合は、 本 issue で 2 系統共存を許容し、 後続の使用側移行 + wrap 削除 issue で最終解消する (decoder 系列と同じ運用)。 これは closed/0066 で採用された派生方針 (δ) を踏襲する。

## 優先度根拠

Medium。

- closed/0066 が (δ) 方針で完了し、 encoder 側だけ同型化が未実施の非対称状態。 これを解消しないと open/0069 / 0070 (AMF/VPL encoder decoder 追加) が旧構造を雛形にしてしまい将来コスト増
- 分離された perf issue (`NvcodecEncoder::flush()` 撤廃 + bp 機構) の実装コストは本 issue 完了で下地 (Sender 経由の Err 伝搬 + メトリクスペアリング + `error_slot` 廃止) が整うため大幅に下がる
- 本 issue 単体の作業は Sender 化の非対称解消のみで、 上記 2 点の下地整備効果 (open/0069 / 0070 の雛形品質、 後続 perf issue のコスト削減) を含めて Medium と判定

## 現状

closed issue 0066 (2026-07-01 完了、 commit hash は git log 参照) 完了後、 decoder 側は `src/decoder.rs` に `OutputSink` / `AsyncVideoDecoder` / wrap 化された `VideoDecoder` を持ち、 その後 0068 / 0071 / 0072 / 0073 / 0078 の段階移行で最終形 (`VideoDecoder` に一本化) に到達している。 encoder 側は 2026-07-07 時点で未着手で、 以下の同期 pull 型構造が残っている:

### 既存 VideoEncoder の内部構造

- `VideoEncoder.encoded: VecDeque<VideoFrame>` (`src/encoder.rs:435`) で同期 pull
- `VideoEncoder::poll_output()` (`src/encoder.rs:734-742`) で pull
- `drain_encoded_frames` (`src/encoder.rs:714-722`) が `inner.next_encoded_frame()` を pull ループで回して `push_encoded_frame_with_metrics` に流す
- `push_encoded_frame_with_metrics` (`src/encoder.rs:724-732`) で `total_output_video_frame_count_metric.inc()` + keyframe 判定 + `total_output_video_keyframe_count_metric.inc()` + sample_entry 不変条件 (closed/0027) を担保
- `drain_video_encoder_output` (`src/encoder.rs:745-764`) が「`VideoEncoder::poll_output` → `TrackPublisher::send_media`」を担う
- `run()` の `tokio::select!` (`src/encoder.rs:632-658`) は「入力 + RPC」の 2 腕構成

### RPC keyframe 経路

- `VideoEncoderRpcMessage::RequestKeyframe` (`src/encoder.rs:372-375`)
- `request_upstream_video_keyframe` (`src/encoder.rs:381-424`): pipeline_handle 経由で RPC を送信
- `run()` の 2 腕目で RPC 受信 → `handle_rpc_message` (`src/encoder.rs:663-674`) が `keyframe_request_pending = true` を設定
- `handle_input_sample` (`src/encoder.rs:684-712`) が `keyframe_request_pending` を検査 → `inner.request_keyframe()` を呼出

### 使用側 (本 issue では書き換えない、 3 call site + 間接呼出)

- `src/sora/recording_subcommand_compose.rs` (`grep -n 'VideoEncoder::new' src/sora/` で確認)
- `src/sora/recording_subcommand_vmaf.rs` (同上)
- `src/subcommand_list_codecs.rs` (`get_engines` 呼出)
- `src/obsws/coordinator/output.rs` / `output_dash.rs` / `output_hls.rs` は `create_video_processor` / `create_video_processor_with_params` (`src/encoder.rs:997-1059`) 経由で間接利用
- 本 issue では `VideoEncoder::new` / `run` / `get_engines` の pub シグネチャを維持するため使用側は無変更

着手時に `grep -rn 'VideoEncoder::new\|create_video_processor\|VideoEncoder::run\|VideoEncoder::get_engines' src/` で使用側を再列挙して、 期待通り本 issue の変更対象外であることを確認する。

### 各 inner の現状出力モデル

| 実装 | 内部 API | 内部キュー | 備考 |
|------|----------|------------|------|
| `LibvpxEncoder` (`src/encoder/libvpx.rs`) | 同期 | `input_queue` + `output_queue` | `encode()` 内で `handle_encoded_frames()` を呼ぶ |
| `Openh264Encoder` (`src/encoder/openh264.rs`) | 同期 | `encoded: Option<VideoFrame>` 単発 | 出力 1 フレーム/encode。 sample_entry 未確定時に fail-fast `Err` を返す (`:76-82`) |
| `SvtAv1Encoder` (`src/encoder/svt_av1.rs`) | 同期 | `input_queue` + `output_queue` | 同上 |
| `VideoToolboxEncoder` (`src/encoder/video_toolbox.rs`) | 非同期 | `input_queue` + `output_queue` | `shiguredo_video_toolbox` 内で callback を `std::sync::mpsc::Sender` (tokio ではない同期版) でチャネル化済み、 上位は `inner.next_frame()` で pull。 SPS/PPS/VPS 未確定時に fail-fast `Err` を返す (`:171-193`) |
| `NvcodecEncoder` (`src/encoder/nvcodec.rs`) | 非同期 | `encoded_queue: Arc<Mutex<VecDeque>>` + `input_queue` + `error_slot: Arc<Mutex<Option<Error>>>` + `output_queue` | hisui コードが `FnEncodeHandler` (`:36-62`) を直接実装する唯一の経路。 callback は CUDA worker スレッドで実行される。 `encode()` (`:202-257`) 内で `self.inner.flush()` (`:254`) を強制呼び出しして worker 完了を待ち合わせ、 encoded_queue から pop して output_queue に流す |

### `NvcodecEncoder` の flush 強制同期化 (本 issue のスコープ外、 参考のみ)

`src/encoder/nvcodec.rs:250-253` のコメント:

> shiguredo_nvcodec のエンコーダーは内部の worker スレッドで非同期にエンコードし、 encode() は即時 return する。 上位パイプラインは同期 pull 型で、 上位側でペース制御しないと内部キューが溢れて encode() が "encoder buffer is full" で失敗するため、 投入直後に flush() で 1 フレーム分の完了を待って同期動作させる。

本 issue では `flush()` 強制同期化と `encoded_queue` は現状のまま維持する (Sender 化に追従するのは callback 内 Err 経路のみ)。 flush 撤廃と bp 機構設計は別 perf issue で扱う。

### メトリクスの現状

- `total_input_video_frame_count_metric.inc()` (`src/encoder.rs:701`): 入力フレーム数 (`handle_input_sample` 内)
- `total_output_video_frame_count_metric.inc()` (`src/encoder.rs:725`): 出力フレーム数 (`push_encoded_frame_with_metrics` 内)
- `total_output_video_keyframe_count_metric.inc()` (`src/encoder.rs:727`): keyframe 出力数 (`push_encoded_frame_with_metrics` 内の keyframe 判定)
- `total_video_keyframe_request_count_metric.inc()` (`src/encoder.rs:666`): RPC keyframe 要求受信数

本 issue で計上ポイントは **inner の `OutputSink::emit_ok` を呼ぶ瞬間に集約** する (二重計上を避けるため、 `OutputSink` 内で `send` と `inc` を物理的に強制ペアリング)。 計上 metric の所有は `OutputSink` 構造体内に持ち、 inner はそれを clone で受け取る。

## 設計方針

### Sender の流路と内部 channel = unbounded

inner の同期 `encode()` が `OutputSink::emit_ok` (内部で `tx.send(Ok(frame))` + `total_output_metric.inc()` + keyframe 判定を 1 関数で実行) 経由で `AsyncVideoEncoder` 内の `rx` に流し、 `rx` は非同期 API (`next_encoded_frame_async` = `rx.recv().await`) と wrap 側 (`VideoEncoder::poll_output` → `AsyncVideoEncoder::poll_output_sync` = `rx.try_recv()`) の 2 経路で吸い上げる。

- `tokio::sync::mpsc::unbounded_channel()` 採用。 採用根拠は closed/0066 §「unbounded channel 採用根拠」(bounded は inner の async fn 化を要求する / callback blocking_send は deadlock パスを持つ / バックプレッシャは下流 TrackPublisher の lag drop で発生) をそのまま踏襲する。 encoder 固有の補足として、 Nvcodec の GPU 側先行投入数は現状の `flush()` 維持で制限されるため本 issue の scope 内で unbounded による無制限投入の懸念は生じない (`flush()` 撤廃時の bp 機構は別 perf issue で扱う)

### OutputSink まとめ struct (encoder 版、 decoder 版と独立)

inner が「frame を Sender に流す」「output メトリクスを inc する」「keyframe を判定して keyframe メトリクスも inc する」を物理的に強制ペアリングするため、 まとめ struct を導入する。 decoder 側と対称に type alias も同時に導入する:

```rust
/// 内部エンコーダーが出力フレーム / エラーを `AsyncVideoEncoder` 内の受信側 (`rx`) に流すための送信側の型エイリアス
pub type EncoderOutputSender = tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>;

/// `AsyncVideoEncoder` 内部で内部エンコーダーからの出力フレーム / エラーを受け取る受信側の型エイリアス
pub(crate) type EncoderOutputReceiver = tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>;

/// inner が出力フレーム / エラーを `AsyncVideoEncoder` 内の rx に流すための sink。
///
/// `tx.send` 失敗 (= Receiver drop) は、 構造体不変条件上発生しない (sink と rx は
/// `AsyncVideoEncoder` 内で同居)。 万一発生した場合は bug として `unreachable!()` で潰す。
#[derive(Debug, Clone)]
pub struct OutputSink {
    tx: EncoderOutputSender,
    total_output_metric: crate::stats::StatsCounter,
    total_output_keyframe_metric: crate::stats::StatsCounter,
}

impl OutputSink {
    pub fn new(
        tx: EncoderOutputSender,
        total_output_metric: crate::stats::StatsCounter,
        total_output_keyframe_metric: crate::stats::StatsCounter,
    ) -> Self {
        Self { tx, total_output_metric, total_output_keyframe_metric }
    }

    pub fn emit_ok(&self, frame: VideoFrame) {
        // keyframe フラグは send 前に取り出す。 VideoFrame は data: Vec<u8> を持ち Clone は
        // 圧縮ペイロード全体の deep copy になるため送信は move。
        let is_keyframe = frame.keyframe;
        if self.tx.send(Ok(frame)).is_err() {
            unreachable!("encoder output sink receiver dropped before sink (bug)");
        }
        // 送信成功後に増分することで「送信できなかったフレームをカウントする」嘘を物理的に防ぐ。
        self.total_output_metric.inc();
        if is_keyframe {
            self.total_output_keyframe_metric.inc();
        }
    }

    pub fn emit_err(&self, err: crate::Error) {
        if self.tx.send(Err(err)).is_err() {
            unreachable!("encoder output sink receiver dropped before sink (bug)");
        }
    }
}
```

decoder 側 (`crate::decoder::OutputSink`) の再利用ではなく encoder 版を新設する理由:

- decoder 版は `total_output_metric` の 1 カウンタのみだが、 encoder 版は keyframe カウンタも必要 (集約対象が異なる)
- decoder / encoder のメトリクス命名は独立で、 型を共有する意義が薄い
- 別 module に閉じることで、 将来どちらかを変更する際の波及を局所化できる

### 構造体設計の確定: VideoEncoder は AsyncVideoEncoder を wrap する

```rust
pub struct AsyncVideoEncoder {
    inner: Option<VideoEncoderInner>,
    rx: EncoderOutputReceiver,
    sink: OutputSink,
    options: VideoEncoderOptions,
    openh264_lib: Option<Openh264Library>,
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    total_video_keyframe_request_count_metric: crate::stats::StatsCounter,
    keyframe_request_pending: bool,
    eos: bool,
}

pub struct VideoEncoder {
    inner_encoder: AsyncVideoEncoder,
}
```

decoder 側と同様、 `inner: Option<VideoEncoderInner>` は遅延初期化 (最初のフレームの解像度で確定) を維持する。 `AsyncVideoEncoder::new` で `sink` と `rx` を確定し、 inner の初期化時に `sink.clone()` を各 variant のコンストラクタに渡す。

### 各 inner の Sender 化形態

全 inner は **同期 fn のまま**、 コンストラクタで `sink: OutputSink` を受け取って内包する。

| inner | 新コンストラクタ | 残存フィールド | 廃止フィールド |
|-------|------------------|----------------|----------------|
| `LibvpxEncoder` | `new_vp8(options, sink)` / `new_vp9(options, sink)` | `inner`, `format`, `sample_entry`, `keyframe_request_pending`, `input_queue`, `sink` | `output_queue` |
| `Openh264Encoder` | `new(lib, options, sink)` | `inner`, `sink`, `force_idr_pending`, `last_sample_entry` | `encoded: Option<VideoFrame>` |
| `SvtAv1Encoder` | `new(options, sink)` | `inner`, `input_queue`, `sample_entry`, `width`, `height`, `keyframe_request_pending`, `sink` | `output_queue` |
| `VideoToolboxEncoder` | `new_h264(options, sink)` / `new_h265(options, sink)` | `inner`, `input_queue`, `sample_entry`, `width`, `height`, `format`, `fps`, `keyframe_request_pending`, `sink` | `output_queue` |
| `NvcodecEncoder` | `new_h264(options, sink)` / `new_h265(options, sink)` / `new_av1(options, sink)` | `inner`, `input_queue`, `sample_entry`, `encoded_format`, `av1_sequence_header`, `force_keyframe_next`, `sink` | `output_queue`, `encoded_queue`, `error_slot` |

各 inner の `encode` / `finish` シグネチャ: `fn encode(&mut self, frame: RawVideoFrame) -> crate::Result<()>` (同期 fn 維持)。 `Result<()>` の `Err` は inner が同期的に検出した不正入力 (現状の `inner.encode()` `Err` と同等)。 callback `Err` は `OutputSink::emit_err` 経由で流す。

### inner ごとの個別対応

- **同期 inner (Libvpx / SvtAv1)**: `encode()` 内で `handle_encoded_frames()` を呼び、 `self.sink.emit_ok(frame)` に切り替える。 既存の Err パス (input_queue とペアリングエラー等) は `encode()` の Err 直返しを維持
- **Openh264Encoder**: `encoded: Option<VideoFrame>` 廃止。 `encode()` 内で SPS/PPS 確定後に `self.sink.emit_ok(frame)` に流す。 sample_entry 未確定時の fail-fast `Err` (`:76-82`) は現状の `encode()` の Err 直返しを維持 (Sender 経由でなく、 同期返却)
- **VideoToolboxEncoder**: 現状 `shiguredo_video_toolbox` 内部の callback → std::sync::mpsc → 上位 `inner.next_frame()` の pull を、 `encode()` / `finish()` 呼出直後に `inner.next_frame()` を pull してその場で `self.sink.emit_ok(frame)` に流す形に変更する (0066 と同じ「本スレッド上で pull → emit」パターン)。 SPS/PPS/VPS 未確定時の fail-fast `Err` (`:171-193`) は現状の `encode()` の Err 直返しを維持
- **NvcodecEncoder**: 以下を実施する:
  - `error_slot` (type alias `ErrorSlot` (`:16`) と field / var / closure 全 hit) を廃止し、 callback 内 `Err` を `sink.emit_err()` で即時通知する (`build_handler` `:36-62` を書き換え)
  - `encoded_queue` (type alias `EncodedQueue` (`:15`) と field / var / closure 全 hit) を廃止し、 callback スレッドで直接 `sink.emit_ok(frame)` に流す (Annex B → MP4 変換とキーフレーム判定は callback スレッドで実施)
  - `input_queue` は `Arc<Mutex<VecDeque<VideoFrame>>>` 化して callback からアクセスできるようにする (Mutex ホールドスコープは `pop_front()` / `push_back()` のみに限定、 変換処理は lock 解放後)
  - `flush()` 呼出 (`:254, 265`) は現状のまま維持 (本 issue のスコープ外。 別 perf issue)
  - `handle_encoded_frames` (`:270-336`) の責務は callback 側に移動し、 メソッド自体を廃止
  - shiguredo_nvcodec の callback dispatch が CUDA worker thread 単一かを実装段階で確認

**Nvcodec callback で Annex B → MP4 変換 + Sequence Header OBU 付与を実施する妥当性**:
- (a) 案 (本 issue で採用): callback スレッドで pop + 変換 + emit を実施。 flush() 維持のため CUDA worker は encode() 内でブロック完了し、 変換コストはそのまま `encode()` 待ち時間に加算される (1080p キーフレーム 100KB 級で Annex B → MP4 の byte scan と 1 回 memcpy は 1ms 未満想定)。 実装後 `--features nvcodec` の smoke で `encode()` 平均レイテンシを計測し、 現状比 +5ms 以内であることを確認する
- (b) 案 (却下): callback は生ペイロードを Sender に流し、 別 task で変換 + emit。 中継 channel が増える + Send 制約 + tracing span 継承の追加考慮が必要。 flush() 維持で (a) の変換コストが許容範囲なら (b) の複雑性は不要
- 将来 flush() を撤廃する perf issue では (a) の変換コストが CUDA worker を block させて GPU 全体スループット低下を招く可能性があり、 その時点で (b) 案を再検討する

**Nvcodec の push/pop 順序保証**:
- `encode()` は本スレッドで `input_queue.lock().push_back(video_frame.to_stripped())` (`:249` の現状位置) → `inner.encode(...)` (`:248` を上に移動) → `inner.flush()` (`:254`) の順で実行する
- callback は `inner.flush()` 内で発火するため、 callback 側 pop 時には push 済みが flush 同期障壁で担保される
- これにより「encoded frame produced without input frame」エラー (現状 `:294`) の発生条件が維持される (現状の本スレッド側 pop と同じ順序契約)

**`build_handler` の書換擬似コード** (現状 `:36-62`):

```rust
fn build_handler(
    sink: OutputSink,
    input_queue: Arc<Mutex<VecDeque<VideoFrame>>>,
    sample_entry: SharedSampleEntry,
    encoded_format: VideoFormat,
    av1_sequence_header: Arc<Vec<u8>>,
) -> shiguredo_nvcodec::FnEncodeHandler<(), shiguredo_nvcodec::Error> {
    shiguredo_nvcodec::FnEncodeHandler::new(move |result| match result {
        Ok(encoded_frame) => {
            let input_frame = input_queue
                .lock()
                .expect("nvcodec input queue lock poisoned")
                .pop_front();
            let Some(input_frame) = input_frame else {
                sink.emit_err(crate::Error::new("encoded frame produced without input frame"));
                return;
            };
            let keyframe = matches!(
                encoded_frame.picture_type(),
                shiguredo_nvcodec::PictureType::I | shiguredo_nvcodec::PictureType::Idr
            );
            // Annex B → MP4 変換 (H.264/H.265) または Sequence Header OBU 付与 (AV1) は
            // 現状の handle_encoded_frames 相当。 実装は現状ロジックをそのまま callback に移す。
            let frame_data = if encoded_format == VideoFormat::Av1 {
                let (mut data, _) = encoded_frame.into_parts();
                if keyframe && !has_sequence_header(&data) {
                    let mut new_data = Vec::with_capacity(av1_sequence_header.len() + data.len());
                    new_data.extend_from_slice(&av1_sequence_header);
                    new_data.extend_from_slice(&data);
                    data = new_data;
                }
                data
            } else {
                match convert_annexb_to_mp4(encoded_frame.data()) {
                    Ok(data) => data,
                    Err(e) => { sink.emit_err(e); return; }
                }
            };
            sink.emit_ok(VideoFrame {
                data: frame_data,
                format: encoded_format,
                keyframe,
                size: input_frame.size,
                timestamp: input_frame.timestamp,
                sample_entry: Some(sample_entry.clone()),
            });
        }
        Err(err) => {
            sink.emit_err(crate::Error::new(format!("nvcodec encode error: {err}")));
        }
    })
}
```

`av1_sequence_header` は callback から参照するため、 現状の `av1_sequence_header: Vec<u8>` (`src/encoder/nvcodec.rs:31`) を `Arc<Vec<u8>>` に置換する。 コンストラクタで `Arc::new(seq_params)` を作って inner field と handler capture の両方に clone で渡す。 `has_sequence_header` と `convert_annexb_to_mp4` は現状の pure fn として残す (現状 `:339-364`, `:379-423`)。

### AsyncVideoEncoder の擬似実装

```rust
impl AsyncVideoEncoder {
    pub fn new(
        options: &VideoEncoderOptions,
        openh264_lib: Option<Openh264Library>,
        mut compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        let engine_metric = compose_stats.string("engine");
        let codec_metric = compose_stats.string("codec");
        let total_input_video_frame_count_metric =
            compose_stats.counter("total_input_video_frame_count");
        let total_output_metric = compose_stats.counter("total_output_video_frame_count");
        let total_output_keyframe_metric = compose_stats.counter("total_output_video_keyframe_count");
        let total_video_keyframe_request_count_metric =
            compose_stats.counter("total_video_keyframe_request_count");
        let error_flag = compose_stats.flag("error");
        error_flag.set(false);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = OutputSink::new(tx, total_output_metric, total_output_keyframe_metric);
        // 上記で取得した metric handle と sink / rx を §構造体設計 の struct 定義に従って初期化して返す。
        // `inner: None`、 `keyframe_request_pending: false`、 `eos: false`。
        Ok(Self { inner: None, rx, sink, /* 残りは struct 定義の順に初期化 */ })
    }

    /// 同期 wrap (`VideoEncoder`) から呼ぶ同期入力 API。 wrap 経路と将来 `AsyncVideoEncoder::run` (後続 issue で追加) 経路のみ想定のため `pub(crate)`。
    pub(crate) fn handle_input_sample_sync(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        if let Some(sample) = sample {
            let frame = sample.expect_video()?;
            let frame = RawVideoFrame::from_video_frame(frame)?;
            let size = frame.size();
            if self.inner.is_none() {
                // initialize_inner 内で `self.sink.clone()` を各 variant コンストラクタ
                // (new_vp8/new_vp9/new_openh264/new_svt_av1/new_video_toolbox_*/new_nvcodec_*) に渡す。
                // 現状の VideoEncoder::initialize_inner / create_inner を移植し sink 引数を追加した形。
                self.initialize_inner(size.width, size.height)?;
            }
            if self.keyframe_request_pending {
                if let Some(inner) = self.inner.as_mut() {
                    inner.request_keyframe();
                }
                self.keyframe_request_pending = false;
            }
            self.total_input_video_frame_count_metric.inc();
            self.inner.as_mut().expect("infallible").encode(frame)?;
        } else {
            self.eos = true;
            if let Some(inner) = &mut self.inner {
                inner.finish()?;
            }
        }
        Ok(())
    }

    /// 同期 wrap から呼ぶ同期 poll。 wrap 経路と将来 `AsyncVideoEncoder::run` 経路のみ想定のため `pub(crate)`。
    pub(crate) fn poll_output_sync(&mut self) -> Result<EncoderRunOutput> {
        match self.rx.try_recv() {
            Ok(Ok(frame)) => Ok(EncoderRunOutput::Processed(MediaFrame::video(frame))),
            Ok(Err(e)) => Err(e),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if self.eos {
                    Ok(EncoderRunOutput::Finished)
                } else {
                    Ok(EncoderRunOutput::Pending)
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                unreachable!(
                    "encoder output channel disconnected unexpectedly (sink dropped before rx)"
                )
            }
        }
    }

    /// 非同期 API (新規)
    pub async fn next_encoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>> {
        self.rx.recv().await
    }

    /// wrap (`VideoEncoder`) の `run` 内 RPC 腕から delegate される同期 RPC ハンドラ。
    /// `keyframe_request_pending` と `total_video_keyframe_request_count_metric` は本構造体側にあるため、
    /// wrap 側で直接触らず本 API 経由で更新する。 crate 外呼出は想定しないため `pub(crate)`。
    pub(crate) fn handle_rpc_message_sync(&mut self, message: VideoEncoderRpcMessage) {
        match message {
            VideoEncoderRpcMessage::RequestKeyframe => {
                self.total_video_keyframe_request_count_metric.inc();
                // 複数の keyframe 要求は 1 件に集約 (現状挙動維持)。 実際の keyframe 要求適用は次の入力フレーム到着時。
                self.keyframe_request_pending = true;
            }
        }
    }

    /// codec とライブラリの利用可否に応じて候補となる engine のリストを返す。
    /// 現状の `VideoEncoder::get_engines` (`src/encoder.rs:574-613`) の match ブロックをそのまま貼付する。
    /// wrap 側からは薄い委譲で呼び出す (closed/0066 と同じ運用、 使用側移行 issue で AsyncVideoEncoder 直接利用に切り替えた際も同型で使える)。
    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> {
        // 既存 src/encoder.rs:574-613 の match ブロックをそのまま貼付 (self 依存なしの associated fn)
    }

    /// AsyncVideoEncoder の non-wrap 版 run (0066 の AsyncVideoDecoder::run 相当は本 issue では追加しない)。
    /// 各使用側の AsyncVideoEncoder 直接利用への移行は後続 refactor issue で扱う。
}
```

`AsyncVideoEncoder::run` は本 issue では **提供しない**。 同期 `VideoEncoder::run` 側で既存ロジックを維持し、 非同期版 `run` は後続の使用側移行 issue で必要性が顕在化した時点で別途設計する (0066 と同じ運用)。

### VideoEncoder の wrap 化と既存 helper の扱い

```rust
pub struct VideoEncoder {
    inner_encoder: AsyncVideoEncoder,
}

impl VideoEncoder {
    pub fn new(
        options: &VideoEncoderOptions,
        openh264_lib: Option<Openh264Library>,
        compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        Ok(Self {
            inner_encoder: AsyncVideoEncoder::new(options, openh264_lib, compose_stats)?,
        })
    }

    pub fn handle_input_sample(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        self.inner_encoder.handle_input_sample_sync(sample)
    }

    pub fn poll_output(&mut self) -> Result<EncoderRunOutput> {
        self.inner_encoder.poll_output_sync()
    }

    pub fn handle_input_message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Media(sample) => self.handle_input_sample(Some(sample)),
            Message::Eos => self.handle_input_sample(None),
            Message::Syn(_) => Ok(()),
        }
    }

    /// 既存実装 (src/encoder.rs:615-661) の 2 腕 tokio::select! (入力 + RPC) 構造を維持。
    /// RPC 腕は self.handle_rpc_message(msg) で self.inner_encoder.handle_rpc_message_sync(msg) に delegate。
    /// 既存 recv_video_encoder_rpc_message_or_pending (src/encoder.rs:766-774) はそのまま流用。
    /// 入力腕から呼ぶ drain_video_encoder_output (src/encoder.rs:745-764) のシグネチャは不変で、
    /// 内部で self.poll_output() → self.inner_encoder.poll_output_sync() の delegate チェーンで動く。
    pub async fn run(
        mut self,
        handle: ProcessorHandle,
        input_track_id: TrackId,
        output_track_id: TrackId,
    ) -> Result<()> {
        todo!("既存 src/encoder.rs:615-661 のロジックを wrap 構造のまま維持 (詳細は上記コメント)")
    }

    fn handle_rpc_message(&mut self, message: VideoEncoderRpcMessage) {
        // AsyncVideoEncoder 側に keyframe_request_pending と total_video_keyframe_request_count_metric があるため delegate。
        self.inner_encoder.handle_rpc_message_sync(message);
    }

    /// AsyncVideoEncoder::get_engines への薄い委譲 (engine 選択ロジック本体は移植済み)。
    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> {
        AsyncVideoEncoder::get_engines(codec, is_openh264_available)
    }
}
```

**`drain_video_encoder_output` (`src/encoder.rs:745-764`) のシグネチャは不変**: `fn drain_video_encoder_output(encoder: &mut VideoEncoder, output_tx: &mut crate::TrackPublisher) -> Result<bool>`。 内部で `encoder.poll_output()` を呼ぶが、 wrap 構造により `encoder.poll_output() -> self.inner_encoder.poll_output_sync()` の delegate チェーンで動く。 使用側 (`compose` / `recording` / `vmaf` / `obsws`) の call site は一切書き換え不要。

### RPC keyframe 経路の維持

現状の `run()` の 2 腕 `tokio::select!` (`:632-658`) は wrap 側 `run` でそのまま維持する。 3 腕構成 (Receiver 追加) には拡張しない (Receiver の pull は wrap 側の同期 `drain_video_encoder_output` → `poll_output` → `poll_output_sync` → `try_recv` で行われる)。

順序保証 (RPC 受信 → `keyframe_request_pending = true` → 次入力時に `inner.request_keyframe()` 呼出) は現状のまま。 `handle_rpc_message` (`:663-674`) の低フレームレート入力時の適用遅延は既知の別問題として本 issue のスコープ外。

### shiguredo-rust 規約整合

- モック / スタブ不使用 (テストは実 encoder + tokio channel)
- トレイト追加なし (`VideoEncoderInner` enum を維持)
- `#[non_exhaustive]` 不使用
- 規約上の許可取得は不要

## 完了条件

- `AsyncVideoEncoder` が `src/encoder.rs` に新規追加され、 `pub async fn next_encoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>>` を提供する。 同期 API delegate のための `handle_input_sample_sync` / `poll_output_sync` も提供する
- `OutputSink` 構造体が `src/encoder.rs` に追加され、 `emit_ok(frame)` で `tx.send` + `total_output_metric.inc` + keyframe 判定 + `total_output_keyframe_metric.inc` を物理的にペアリングしている
- `VideoEncoder` が `AsyncVideoEncoder` を内包する wrap 構造に変更され、 同期 API (`new`, `handle_input_sample`, `poll_output`, `run`, `handle_input_message`, `get_engines`) の **外部挙動は不変** (戻り値型・タイミング・エラー伝搬経路すべて維持)
- 各 inner のコンストラクタが `sink: OutputSink` を追加で受け取って内包する形に変更されている
- 各 inner の `output_queue` / `encoded: Option<VideoFrame>` / `encoded_queue` / `error_slot` 中継キューが廃止されている
- 各 inner の `next_encoded_frame()` API が廃止されている
- 各 inner の `encode()` / `finish()` シグネチャは同期 fn のまま維持 (async fn 化なし)
- **本 issue のスコープ外として維持する項目**:
  - `NvcodecEncoder::encode()` 内の `self.inner.flush()` 呼出 (`src/encoder/nvcodec.rs:254, 265`) は現状のまま維持
  - `AsyncVideoEncoder::run` は本 issue では提供しない (0066 と同じ運用)
  - 使用側 (`compose` / `recording` / `vmaf` / `obsws` / `list_codecs` / `create_video_processor(_with_params)`) は一切書き換えない
- grep 検証 (機械検証可能な形に限定。 wrap 側 `poll_output` / `drain_video_encoder_output` は delegate として維持するため grep 対象から除外):
  - `grep -rn 'next_encoded_frame\|push_encoded_frame_with_metrics' src/encoder.rs src/encoder/libvpx.rs src/encoder/openh264.rs src/encoder/svt_av1.rs src/encoder/video_toolbox.rs src/encoder/nvcodec.rs` の hit が 0 件 (VideoEncoder inner の pull API と push_encoded_frame_with_metrics の廃止確認)
  - `grep -rn 'drain_encoded_frames' src/encoder.rs` の hit が 0 件 (VideoEncoder 側廃止確認。 `src/encoder/fdk_aac.rs` の `drain_encoded_frames` は audio 側で無関係のため path 除外)
  - `grep -rn 'error_slot\|encoded_queue\|EncodedQueue\|ErrorSlot' src/encoder/nvcodec.rs` の hit が 0 件 (Nvcodec の中継バッファと type alias の廃止確認)
  - `grep -n 'OutputSink' src/encoder.rs src/encoder/libvpx.rs src/encoder/openh264.rs src/encoder/svt_av1.rs src/encoder/video_toolbox.rs src/encoder/nvcodec.rs` の hit が 6 箇所以上 (5 inner の import + `AsyncVideoEncoder` 経路の新型適用確認)
  - `grep -n 'AsyncVideoEncoder\|inner_encoder' src/encoder.rs` の hit が想定通り (新構造の存在確認)
  - `grep -n 'pub type EncoderOutputSender\|pub(crate) type EncoderOutputReceiver' src/encoder.rs` の hit が各 1 件 (decoder 側と対称な type alias 導入の確認)
- 変更ファイル制限:
  - `git diff --name-only develop...HEAD -- src/sora/ src/obsws/ src/subcommand_list_codecs.rs` の hit が 0 件 (使用側無変更)
- 各 inner の末尾テスト (`src/encoder/libvpx.rs` / `openh264.rs` / `svt_av1.rs` / `video_toolbox.rs` の `#[cfg(test)] mod tests`) が Sender 形式に書き換えられている (`tokio::sync::mpsc::unbounded_channel()` + `OutputSink::new` を作って inner に渡し、 encode ループ + `encoder.finish()?` + `rx.try_recv()` の drain ループで検証する形)
- `src/encoder/test_helpers.rs` に `OutputSink` 構築 helper (`fn make_encoder_sink() -> (OutputSink, tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>)`) を追加して各テストのボイラープレートを削減する
- **ファイル名 rename**: `0067-feature-refactor-video-encoder-sender-interface.md` → `0067-feature-refactor-add-async-video-encoder.md` (Branch メタと整合)。 本 polish 完了時点で develop 上で `git mv` する (実装 PR 開始前)。 closed/0071 の precedent (commit `2881325b` 「polish で Branch を変更した際、 ファイル名の変更は git mv が必要でユーザー判断領域として保留していた。 実装ブランチ切りに合わせて Branch とファイル名を統一する」) と同型
- **closed/0057 §3 分割表の 0067 行更新**: スコープ縮小 ((δ) 方針採用 + flush 撤廃を別 perf issue に分離) + Branch 名更新 (`feature/refactor-video-encoder-sender-interface` → `feature/refactor-add-async-video-encoder`) + 分割後続 issue 行追加 (使用側移行 / wrap 削除 rename / 未使用 API 削除 / flush 撤廃 perf) を実装 PR 内で同時 commit する (closed/0073 §3 分割表更新 precedent と同型)
- **open/0069 / open/0070 の関連節への 0067 依存追記**: 「依存: closed/0067 (Sender 化 API 確定後の雛形踏襲)」を 0069 / 0070 側の関連節に追記する。 本 polish 完了時の後片付けとして develop で直接コミット (作業を伴わない issue 更新のため)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

実装作業の大部分は §設計方針 の各サブ節に従って進める (§OutputSink まとめ struct / §各 inner の Sender 化形態 / §AsyncVideoEncoder の擬似実装 / §VideoEncoder の wrap 化 / §NvcodecEncoder / §VideoToolboxEncoder)。 §設計方針 との重複を避け、 本節ではそれらに含まれない実装細目のみを扱う。

### 実装順序

コンパイル通過順で実装する:

1. `OutputSink` 型 + `EncoderOutputSender` / `EncoderOutputReceiver` type alias を追加
2. `AsyncVideoEncoder` の skeleton (`inner: None` の状態で `new` / `handle_input_sample_sync` / `poll_output_sync` / `handle_rpc_message_sync` / `get_engines` / `next_encoded_frame_async`) を追加
3. 各 inner を `OutputSink` を受け取るコンストラクタに順次書換 (§各 inner の Sender 化形態 表の全 5 inner)。 `initialize_inner` / `create_inner` を `AsyncVideoEncoder` 側に移植し、 各 variant の `new_*(options, sink)` を呼ぶ。 openh264 は `new(lib, options, sink)`、 nvcodec は `new_*(options, sink)`、 その他は `new_*(options, sink)` の引数順に注意
4. 既存 `VideoEncoder` を `AsyncVideoEncoder` の wrap 構造に置換 (§VideoEncoder の wrap 化)
5. `push_encoded_frame_with_metrics` (`src/encoder.rs:724-732`) の keyframe 判定 + メトリクス計上責務を `OutputSink::emit_ok` に移植し、 `push_encoded_frame_with_metrics` / `drain_encoded_frames` / `VideoEncoder.encoded` を廃止

### tests/e2e.rs の使用側無変更確認

`grep -rn 'VideoEncoder\|create_video_processor' tests/` で使用箇所を確認し、 `VideoEncoder::new` / `run` / `get_engines` の pub シグネチャ不変を活かして無変更で通せることを確認する。 変更が発生した場合は本 issue の scope に含めるか別 issue に切り出すかを判断。

### 各 inner の末尾テスト書き換え

`libvpx.rs` / `openh264.rs` / `svt_av1.rs` / `video_toolbox.rs` の末尾 `#[cfg(test)] mod tests` を Sender 形式に書き換える。 具体形は encode ループ + finish + drain の完全形:

```rust
let (sink, mut rx) = crate::encoder::test_helpers::make_encoder_sink();
let mut encoder = LibvpxEncoder::new_vp9(&options, sink)?;
let mut output_count = 0;
for i in 0..10 {
    encoder.encode(raw_i420_frame(i * 33))?;
    while let Ok(result) = rx.try_recv() {
        let frame = result?;
        assert!(frame.sample_entry.is_some());
        output_count += 1;
    }
}
encoder.finish()?;
while let Ok(result) = rx.try_recv() {
    let frame = result?;
    assert!(frame.sample_entry.is_some());
    output_count += 1;
}
assert!(output_count >= 2, "出力フレーム数が少なすぎる: {output_count}");
```

`test_helpers.rs` に helper を追加してボイラープレートを削減する:

```rust
pub(crate) fn make_encoder_sink() -> (crate::encoder::OutputSink, tokio::sync::mpsc::UnboundedReceiver<crate::Result<crate::video::VideoFrame>>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut stats = crate::stats::Stats::new();
    let total_output = stats.counter("test_total_output");
    let total_keyframe = stats.counter("test_total_keyframe");
    let sink = crate::encoder::OutputSink::new(tx, total_output, total_keyframe);
    (sink, rx)
}
```

### `clippy::large_enum_variant` の予防的注意

各 inner に `sink: OutputSink` (3 フィールド、 うち `Arc` 系 2 個で約 24 bytes 増) が追加され、 かつ Nvcodec の `input_queue` が `Arc<Mutex<VecDeque<VideoFrame>>>` 化されるとサイズ順位が入れ替わり得る。 完了条件の `cargo clippy --deny warnings` で機械的に検出されるため、 発火時は該当 variant を `Box<...>` 化して対応する (現状 `Libvpx(Box<LibvpxEncoder>)` `Nvcodec(Box<NvcodecEncoder>)` と同型)。

## 後続実装 issue の分割 (本 issue では起票しない)

本 issue 完了後に以下を段階的に別 issue で扱う (decoder 系列 0068 / 0071 / 0072 / 0073 / 0078 と対応)。 実装依存順:

1. **使用側移行 refactor issue** (本 issue の PR merge 後に起票): `compose` / `recording` / `vmaf` / `obsws` / `list_codecs` を `AsyncVideoEncoder` 直接利用へ移行 + `AsyncVideoEncoder::run` を追加する (decoder 系列 0068 相当)。 encoder の使用側は全て outbound で decoder ほど物理的距離が離れていない (compose / recording / vmaf / obsws / list_codecs はすべて lib target 内) ため、 decoder 系列の 0068 / 0071 / 0072 の 3 分割相当を 1 issue にまとめる余地がある (実装時に scope 確定)
2. **wrap 削除 + rename refactor issue** ((1) の PR merge 後に起票): 同期 wrap `VideoEncoder` 削除 + `AsyncVideoEncoder` → `VideoEncoder` リネーム + 内部メソッド `_sync` / `_async` サフィックス整理 (decoder 系列 0073 相当)
3. **未使用 API 削除 refactor issue** ((2) の PR merge 後に起票): 使用側移行完了後に実測して dead code になった public API を削除 + `EncoderOutputReceiver` 等の可視性整理 (decoder 系列 0078 相当。 具体的にどの API が dead になるかは (1) / (2) の実装完了時点で判明)
4. **NvcodecEncoder の flush() 撤廃 + bp 機構 perf issue** (本 issue の PR merge 後に独立に起票可能): NVENC 非同期パイプライン並列性回復 (0057 §3 の中核動機)。 Sender 化とは本質的に別問題で refactor カテゴリではなく perf カテゴリで扱う。 wall-clock 短縮 15% / p99 改善 5ms 等の実機計測を完了条件に据える。 bp 機構は「内部キュー上限ベースのセルフペーシング」等を 0057 §3 §2 の議論を踏まえて設計確定する。 (1)〜(3) と独立に着手可能だが、 (2) 完了後の型面 (`VideoEncoder` = 旧 `AsyncVideoEncoder`) を前提にすると diff が小さくなる

これらは本 issue の PR merge 時 (使用側移行 issue) または実装段階で判明する dead code の実測後 (未使用 API 削除 issue) など、 タイミングを分けて Decision Owner (@sile) が起票する。 本 issue 単独では起票しない。

## CHANGES.md について

内部リファクタにつき記載不要。 `VideoEncoder` 系は library として外部公開していない (hisui の lib target は crates.io 未 publish で workspace 内 bin / tests 専用のため、 API 変更の後方互換影響は本 issue ではゼロ = 既存 pub API 挙動を維持)。

## 関連

- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 設計検討の親 issue。 本 issue は §3 採用案 C の encoder 部分実装 (派生方針 (δ))。 本 issue 完了時に closed/0057 §3 分割表への 0067 実績反映 (別 issue で扱う可能性)
- closed/0066 (`feature/refactor-add-async-video-decoder`): decoder 側の完成形。 本 issue の設計方針は 0066 の判断 (unbounded + OutputSink + wrap + inner 同期 fn 維持) を encoder 側に移し替えたもの。 closed/0066 line 619-620 で「0067 は (δ) 方針への書き換えと polish 再実施が必要」と宣言された宿題を本 polish で消化
- closed/0068 / closed/0071 / closed/0072: decoder 側の使用側移行実例。 encoder 側は使用側が全て outbound で 1 issue にまとめる想定 (§後続実装 issue の分割 (1) 参照)
- closed/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`): decoder 側 wrap 削除 + rename。 encoder 側でも同パターンで最終ステップを別 issue で扱う
- closed/0078 (`feature/refactor-remove-unused-next-decoded-frame`): decoder 側の未使用 API 削除。 encoder 側でも同パターンで扱う可能性 (使用側移行完了後の dead code に応じて別 issue)
- closed/0027 (`feature/refactor-video-sample-entry-all-frames`) / closed/0030 (`feature/refactor-encoded-frame-sample-entry-invariant`) / closed/0051 (`feature/refactor-remove-writer-sample-entry-fallback`): sample_entry 不変条件の系列 (全出力フレームに sample_entry を載せる + 圧縮フレームの sample_entry は必ず Some)。 本 issue の OutputSink 経由経路でも inner 側で維持責任を保つ
- closed/0054 (`feature/refactor-encoder-defer-output-until-sample-entry-ready`): エンコーダーで sample_entry 未確定時の出力を Err 化する fail-fast 整備。 Openh264 / VideoToolbox の fail-fast Err は本 issue でも `encode()` の Err 直返しで維持
- open/0069 (`feature/add-amf-encoder-decoder`) / open/0070 (`feature/add-vpl-encoder-decoder`): 本 issue 完了後に着手 (Sender 化 API 確定後の雛形踏襲のため)
