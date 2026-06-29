# AsyncVideoDecoder を追加し VideoDecoder を内部 channel ベースに改修する

- Priority: Medium
- Created: 2026-06-26
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-add-async-video-decoder
- Polished: 2026-06-29
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0057 §3 で確定した採用案 C (全エンコーダー / デコーダーを Sender 経由の出力に統一) を、 **段階的に移行可能な形で** VideoDecoder 系に適用する。

具体的には:

1. **`AsyncVideoDecoder` を新規追加**: 内部 channel から `recv().await` でフレームを受け取る非同期インターフェース
2. **既存 `VideoDecoder` (同期) を `AsyncVideoDecoder` の wrapper として再構築**: 内部に `AsyncVideoDecoder` を保持し、 同期 pull API (`handle_input_sample` / `next_decoded_frame` / `poll_output` / `run`) は `AsyncVideoDecoder` への delegate として実装。 **外部 API 挙動は維持** し、 既存使用側の書き換えは不要
3. **inner 層 (`VideoDecoderInner` enum と各 variant) は `AsyncVideoDecoder` が保持**: 各 inner (`LibvpxDecoder` / `Openh264Decoder` / `Dav1dDecoder` / `VideoToolboxDecoder` / `NvcodecDecoder`) はコンストラクタで `OutputSink` (`tx` + `total_output_metric` まとめ struct) を受け取って内包。 `decode()` / `finish()` は同期 fn のままで、 内部で出力フレームを `self.sink.emit(frame)` で push する

本 issue の範囲は「`AsyncVideoDecoder` 追加 + 既存 `VideoDecoder` の wrapper 化 + inner の Sender 化」までで、 各使用側の `AsyncVideoDecoder` への移行と旧 API 削除は **open issue 0068 で段階的に** 実施する。 closed/0057 §3 採用案 C の「中途半端な 2 系統共存を残さない」原則と整合させる責務は 0068 が引き受ける (本 issue 完了時点では 2 系統共存を意図的に許容)。

## 優先度根拠

Medium。

- closed issue 0057 で採用案が確定済み。 polish 完了後に実装着手予定
- 依存先: 0067 (encoder) は本 issue 完了後に同パターンで対称展開、 0068 (使用側移行) は本 issue の前提が完成してから着手
- 0066 + 0068 + 0067 + (0067 系移行 issue、 起票予定) の 4 つで closed/0057 採用案 C を分担達成する

## 現状

`src/decoder.rs` および `src/decoder/*.rs` の各 inner の構造は closed issue 0057 「現状」§の表を参照。 本 issue で書き換える範囲は以下:

### 既存 VideoDecoder の内部構造

- `VideoDecoder.decoded: VecDeque<VideoFrame>` (`src/decoder.rs:335`) で同期 pull
- `VideoDecoder::next_decoded_frame()` は **存在しない** (`src/decoder.rs` の同名 fn は `VideoDecoderInner` の private dispatch)。 公開同期 API は `handle_input_sample` (`:401-420`) と `poll_output` (`:422-430`) と `run` (`:360-391`)
- `drain_video_decoder_output` (`src/decoder.rs:514-533`) は `VideoDecoder::poll_output()` を回しながら `TrackPublisher::send_media()` に流す。 4 ファイル (`src/mp4/reader.rs:1236, 1286` / `src/rtsp/subscriber.rs:705` / `src/srt/inbound_endpoint.rs:512` / `src/rtmp/inbound_endpoint.rs:477`) で利用

これらの公開同期 API は **全て挙動維持する**。 内部のフレーム保持構造を `VecDeque` から `tokio::sync::mpsc::UnboundedReceiver` (経由で `AsyncVideoDecoder` 内) に置き換えるのみ。

### tests/e2e.rs での inner 直接利用

`tests/e2e.rs:1359, 1399, 1571, 1616, 1710, 1742, 1813, 1845` で `LibvpxDecoder::new_vp9()` を直接呼び、 `decoder.next_decoded_frame()` で出力を受け取っている (`use hisui::decoder::libvpx::LibvpxDecoder;`)。 各ブロックは「クロージャ定義 (前者) + new_vp9 呼出 (後者)」の 2 箇所構成で 4 ブロック × 2 行 = 8 箇所。 本 issue で inner のコンストラクタシグネチャと `next_decoded_frame()` を変更するため、 これら 4 ブロックを新 API (`LibvpxDecoder::new_vp9(sink)` + `rx.try_recv()`) に追従させる。

### 各 inner の現状出力モデル

| 実装 | 内部 API | 内部キュー | 備考 |
|------|----------|------------|------|
| `LibvpxDecoder` | 同期 | `input_queue` + `output_queue` | `decode()` 内で `handle_decoded_frames()` を呼ぶ |
| `Openh264Decoder` | 同期 | `input_queue` + `output_queue` | keyframe 入力時に `self.finish()` を呼んでバッファ flush。 `finish()` は 0 or 1 frame、 続く `decode()` も 0 or 1 frame、 合計 0〜2 frame 送信 |
| `Dav1dDecoder` | 同期 | `input_queue` + `output_queue` | |
| `VideoToolboxDecoder` | 同期 | `decoded: Option<VideoFrame>` 単発 | `inner.decode()` 戻り値で `Option<DecodedFrame>` を受け取る同期 API。 上書き喪失防止のため `reinitialize_if_need` で `decoded.is_some()` ガードあり |
| `NvcodecDecoder` | 非同期 | `decoded_queue: Arc<Mutex<VecDeque>>` + `input_queue` + `error_slot` | hisui コードが `FnDecodeHandler` を直接実装。 callback は別スレッドから呼ばれる前提 |

各 inner の `output_queue` / `decoded_queue` / `error_slot` / `decoded: Option<VideoFrame>` 等の中継キューを廃止し、 **コンストラクタで `OutputSink` を受け取って内包** する形に統一する。

### 外部利用箇所 (本 issue では一切書き換えない)

`drain_video_decoder_output` 利用 4 ファイル / 5 call site (上記)、 `discard_video_decoder_output` 1 箇所 (`src/mp4/reader.rs:1388`)、 `VideoDecoder::new` 外部生成 9 call site (`src/subcommand_inspect.rs:215` / `src/rtsp/subscriber.rs:107` / `src/srt/inbound_endpoint.rs:204` / `src/rtmp/inbound_endpoint.rs:167` / `src/sora/recording_subcommand_vmaf.rs:362, 480` / `src/sora/recording_subcommand_compose.rs:463` / `src/obsws/source/file_mp4.rs:54` / `src/mp4/reader.rs:1369`)、 `set_video_decoder` (`src/obsws/source/file_mp4.rs:61` → `Mp4FileReader::set_video_decoder` `src/mp4/reader.rs:318`) は **すべて現状のまま動く**。

`VideoDecoder::new` のシグネチャは変えず、 `next_decoded_frame()` (内部 dispatch) と `poll_output()` (外部 API) の挙動も維持する。

### メトリクス計上の現状

- `total_input_video_frame_count_metric.inc()` (`src/decoder.rs:344-345, 405`): 入力フレーム数 (`handle_input_sample` 内)
- `total_output_video_frame_count_metric.inc()` (`src/decoder.rs:346-347, 415`): 出力フレーム数 (`handle_input_sample` 内の while ループで `inner.next_decoded_frame()` ごとに inc)

本 issue で計上ポイントは **inner の `OutputSink::emit` を呼ぶ瞬間に集約** する (二重計上を避けるため、 `OutputSink` 内で `send` と `inc` を物理的に強制ペアリング)。 計上 metric の所有は `OutputSink` 構造体内に持ち、 inner はそれを clone で受け取る。

## 設計方針

### Sender の流路と内部 channel = unbounded

```
inner.decode(frame)  [同期 fn]
   ↓
inner.sink.emit(frame)  [OutputSink::emit、 内部で `tx.send(Ok(frame))` + `total_output_metric.inc()` を 1 関数で実行]
   ↓
AsyncVideoDecoder.rx (tokio::sync::mpsc::UnboundedReceiver)
   ↓
   ├─ AsyncVideoDecoder::next_decoded_frame_async() = rx.recv().await  [非同期 API、 新規]
   └─ VideoDecoder (wrap) ::handle_input_sample / next_decoded_frame / poll_output  [同期 API、 既存維持]
```

- `tokio::sync::mpsc::unbounded_channel()` 採用
- `UnboundedSender::send(value) -> Result<(), SendError<T>>` は **Receiver が drop された場合にのみ Err を返し、 それ以外では失敗しない**
- `UnboundedSender::send` は同期 fn で tokio runtime context 不要 (内部はロックフリー queue)
- `UnboundedReceiver::try_recv()` も runtime 不要、 `UnboundedReceiver::recv().await` は runtime 必要

### unbounded channel 採用根拠 (closed/0057 §3 確定 bounded N=8 からの変更)

closed/0057 §3 は「bounded N=8」を実装前提として確定していたが、 本 issue では unbounded に変更する。 根拠:

- bounded + `tx.send().await` は **inner の `decode()` を `async fn` 化する**必要があり、 (δ) 方針「inner は同期 fn のまま」と矛盾する
- bounded + `tx.try_send()` は容量超過時の処理 (drop / err / wait) を選ぶ必要があり、 inner レイヤで本質的でない複雑性が増える
- バックプレッシャは下流 `output_tx: TrackPublisher` (`tokio::sync::broadcast` ベース) の lag drop で発生し、 本質的なメモリ上界は下流 broadcast capacity で決まる
- **メモリ上界の上限**: Nvcodec の `max_num_decode_surfaces: 20` (`src/decoder.rs:316`) が GPU 側の先行投入数を 20 に制限。 raw I420 1080p × 20 ≒ 60MB がワーストケースで、 実用上問題なし
- 0067 (encoder) も同じ unbounded 方針で揃える。 ただし encoder の `flush()` 強制同期化撤廃 (closed/0057 中核動機) は 0067 で別途バックプレッシャ機構 (内部キュー上限ベースのセルフペーシング等) を確定する

### OutputSink まとめ struct

inner が「frame を Sender に流す」と「metric を inc する」を物理的に強制ペアリングするため、 まとめ struct を導入する:

```rust
/// inner が出力フレーム / エラーを `AsyncVideoDecoder` 内の rx に流すための sink。
///
/// `tx.send` 失敗 (= Receiver drop) は、 構造体不変条件上発生しない (sink と rx は
/// `AsyncVideoDecoder` 内で同居)。 万一発生した場合は bug として `debug_assert` で潰す。
/// `tests/e2e.rs` の integration test から inner 直叩きする経路があるため `pub` で公開する。
/// フィールドは private のままにし、 構築は `OutputSink::new` 経由に統一する
/// (struct literal は別 crate (integration test) から書けないため、 pub コンストラクタが必須)。
#[derive(Debug, Clone)]
pub struct OutputSink {
    tx: tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>,
    total_output_metric: crate::stats::StatsCounter,
}

impl OutputSink {
    /// `tests/e2e.rs` (integration test) を含む別 crate からも構築できるよう、 pub コンストラクタを提供する。
    pub fn new(
        tx: tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>,
        total_output_metric: crate::stats::StatsCounter,
    ) -> Self {
        Self { tx, total_output_metric }
    }

    pub fn emit_ok(&self, frame: VideoFrame) {
        self.total_output_metric.inc();
        // sink と rx は AsyncVideoDecoder 内で同居するため、 通常時の `send` は必ず成功する。
        // 失敗するのは Receiver が drop された場合のみで、 これは構造体不変条件違反 = bug。
        let send_result = self.tx.send(Ok(frame));
        debug_assert!(
            send_result.is_ok(),
            "decoder output sink receiver dropped before sink (bug)"
        );
    }

    pub fn emit_err(&self, err: crate::Error) {
        let send_result = self.tx.send(Err(err));
        debug_assert!(
            send_result.is_ok(),
            "decoder output sink receiver dropped before sink (bug)"
        );
    }
}
```

`StatsCounter` は内部 `Arc` で clone が cheap、 `UnboundedSender` も同様。 `OutputSink::clone()` は両方の Arc bump のみで安価。 inner は `OutputSink` 1 個だけ持てば良く、 `tx` と `metric` を別々に管理する手間が消える。

### 構造体設計の確定: VideoDecoder は AsyncVideoDecoder を wrap する

```rust
pub struct AsyncVideoDecoder {
    inner: VideoDecoderInner,
    rx: tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>,
    sink: OutputSink,  // Initial 遷移時に inner に clone を渡すため保持
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    eos: bool,
}

pub struct VideoDecoder {
    inner_decoder: AsyncVideoDecoder,
}
```

VideoDecoder と AsyncVideoDecoder を同時に保持する必要はない (使用側はどちらか一方を選ぶ) ため、 wrap で同一 inner 共有問題は発生しない。 wrap 採用理由:

- 既存 `VideoDecoder` の同期 API 実装を `AsyncVideoDecoder` 経由の delegate に置き換えるだけで済む
- 0068 で使用側を `AsyncVideoDecoder` に移行した後、 同期 wrap (`VideoDecoder`) を丸ごと削除して `AsyncVideoDecoder` を `VideoDecoder` にリネームする最終クリーンアップが綺麗

棄却した選択肢と再考トリガー:

- **(α) `AsyncVideoDecoder` が `VideoDecoder` を wrap**: 既存 `VideoDecoder` 内部に inner と rx を保持し、 `AsyncVideoDecoder` がそれを薄く包む。 0068 完了時に「`VideoDecoder` を削除」できなくなる (内側にあるため) ので棄却。 再考トリガー: 0068 が放棄され、 同期 API 維持が必要になった場合
- **(β) 2 つの独立構造体 + 共通 inner trait**: コード重複大、 メトリクス所有関係が分散して複雑化。 棄却。 再考トリガー: wrap delegate のオーバーヘッドが計測で問題視された場合

### 各 inner の Sender 化形態

全 inner は **同期 fn のまま**、 コンストラクタで `sink: OutputSink` を受け取って内包する。 各 inner の最終構造:

| inner | 新コンストラクタ | 残存フィールド | 廃止フィールド |
|-------|------------------|----------------|----------------|
| `LibvpxDecoder` | `new_vp8(sink)` / `new_vp9(sink)` | `inner`, `input_queue`, `sink` | `output_queue` |
| `Openh264Decoder` | `new(lib, sink)` | `inner`, `input_queue`, `sink` | `output_queue` |
| `Dav1dDecoder` | `new(sink)` | `inner`, `input_queue`, `sink` | `output_queue` |
| `VideoToolboxDecoder` | `new_h264(frame, sink)` 等 | `inner`, `vps`, `sps`, `pps`, `resolution`, `sink` | `decoded: Option<VideoFrame>` |
| `NvcodecDecoder` | `new_h264(params, sink)` 等 | `inner`, `input_queue` (※下記、 方針 (a) なら `Arc<Mutex<VecDeque>>` 化), `parameter_sets`, `sink` | `decoded_queue`, `error_slot`, `output_queue` |

`decode` / `finish` 統一シグネチャ: `fn decode(&mut self, frame: &VideoFrame) -> Result<()>` (同期 fn、 `Result<()>` の `Err` は decode 中の同期エラー = inner レベルの不正入力等。 callback Err は OutputSink 経由で流す)。

### inner ごとの個別対応

- **同期 inner (Libvpx / Openh264 / Dav1d)**: `decode()` 内で `self.sink.emit_ok(frame)` を呼ぶ。 `decode()` 自身が `Err` を返すのは inner が同期的に検出した不正入力など (現状の `inner.decode()` Err と同等)。 既存の挙動 (decode Err → 同期呼出側に Err 直返し) を保つ
- **Openh264Decoder の keyframe シーケンス**: `decode()` 内で `if frame.keyframe { self.finish()?; }` の後に decode を実行する既存挙動を維持。 1 回の `decode()` 呼出で 0〜2 frame 送信 (旧 finish 0〜1 + 新 decode 0〜1)。 `decode()` と `finish()` の両方で同じ `self.sink.emit_ok(frame)` 呼出を使い、 emit パスを一本化する
- **VideoToolboxDecoder**: `decoded: Option<VideoFrame>` 廃止により、 「未消費 frame が reinitialize で喪失するリスク」自体が解消される (Sender へ既に emit 済みのため)。 `reinitialize_if_need` の `decoded.is_some()` ガード (`src/decoder/video_toolbox.rs:133-137, 148-152, 180-184`) を廃止。 reinitialize 時の self 再代入 (`src/decoder/video_toolbox.rs:138, 153, 186` の `*self = Self::new_h264(frame)?` 等) は **`*self = Self::new_h264(frame, self.sink.clone())?` のように既存 sink を引き継ぐ**こと (新コンストラクタ内で勝手に新 channel を生成すると元の `AsyncVideoDecoder` の rx に届かない bug を生む)
- **NvcodecDecoder**: `error_slot` 廃止により callback Err を `sink.emit_err()` で即時通知。 `decoded_queue` 廃止により callback 内で NV12→I420 変換 + `sink.emit_ok(frame)` を直接実行。 ただし `input_queue` のスレッド扱いは以下のいずれかを選択 (実装段階で確定、 暫定推奨は (a)):
  - (a) **callback 側で `input_queue` を pop して NV12→I420 変換 + emit**: `input_queue` を `Arc<Mutex<VecDeque<VideoFrame>>>` 化 (上表 `※` 印参照)。 callback スレッドで libyuv 変換するため CUDA callback 戻り遅延の懸念あり (1080p NV12→I420 で実測 1ms 程度の想定、 実装段階で計測して 5ms 超なら (b) 検討)。 (a) 採用時の制約:
    - `input_queue` の `Mutex` ホールドスコープは **`pop_front()` のみ** に限定する。 NV12→I420 変換は lock 解放後に実行 (libyuv 呼出中の lock ホールドは本スレッド側次回 `decode()` (= `input_queue.push_back`) を不必要にブロックする)
    - `parameter_sets` (`src/decoder/nvcodec.rs:26`) は本スレッド側 (`decode()` 呼出側) のみが更新する。 callback 側からは read のみで触らない (現状の SPS/PPS 取り扱いを踏襲)
    - shiguredo_nvcodec の callback dispatch が CUDA worker thread 単一かを実装段階で確認 (一般に NVDEC は worker thread 1 つで dispatch されるため、 ロングタイム lock ホールドは GPU 全体スループットを下げ得る)
  - (b) **callback では NV12 frame を中継 channel に転送、 別 task で I420 変換 + emit**: 中継 channel が残るが callback 側軽量。 unbounded × 2 段で全体構造が複雑化

### AsyncVideoDecoder の擬似実装

```rust
impl AsyncVideoDecoder {
    pub fn new(options: VideoDecoderOptions, mut stats: crate::stats::Stats) -> Self {
        let engine_metric = stats.string("engine");
        let codec_metric = stats.string("codec");
        let total_input_video_frame_count_metric = stats.counter("total_input_video_frame_count");
        let total_output_metric = stats.counter("total_output_video_frame_count");
        stats.flag("error").set(false);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = OutputSink::new(tx, total_output_metric);
        Self {
            inner: VideoDecoderInner::Initial { options, sink: sink.clone() },
            rx,
            sink,
            engine_metric,
            codec_metric,
            total_input_video_frame_count_metric,
            eos: false,
        }
    }

    /// 同期 wrap (`VideoDecoder`) から呼ぶ同期 API
    ///
    /// 既存挙動を維持: `inner.decode()` 内で発生した同期 Err は `?` 直返しで同期返却する。
    /// Nvcodec callback Err は `sink.emit_err()` 経由で channel に流れ、 後続の
    /// `poll_output_sync` の `try_recv` で受信して同期返却される (既存 `error_slot` と意味論互換)。
    pub fn handle_input_sample_sync(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        if let Some(s) = sample {
            let frame = s.expect_video()?;
            self.total_input_video_frame_count_metric.inc();
            // `VideoDecoderInner` は各 variant 内に sink を内包する設計のため、 外部から sink を
            // 引数で渡す必要はない (Initial → 実 variant 遷移時に Initial variant 内の sink を
            // `initialize_decoder` 経由で実 inner コンストラクタへ clone 渡し)。
            self.inner.decode(&frame, &self.codec_metric, &self.engine_metric)?;
        } else {
            self.eos = true;
            self.inner.finish()?;
        }
        Ok(())
    }

    /// 同期 wrap (`VideoDecoder`) から呼ぶ同期 poll
    ///
    /// 既存 `poll_output()` の戻り値型と意味論を完全維持。 try_recv の Empty / Disconnected を
    /// eos と組み合わせて判定する。
    pub fn poll_output_sync(&mut self) -> Result<DecoderRunOutput> {
        match self.rx.try_recv() {
            Ok(Ok(frame)) => Ok(DecoderRunOutput::Processed(MediaFrame::video(frame))),
            Ok(Err(e)) => Err(e),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if self.eos {
                    // 既存実装は eos で即 Finished を返していた。 wrap 構造では同期 inner の
                    // emit はすべて handle_input_sample_sync 内で完了するため、 eos に至った
                    // 時点で sink 内に残物はない (Nvcodec は finish() が flush 待ち合わせ済)。
                    // よって既存挙動を維持して Finished を返す。
                    Ok(DecoderRunOutput::Finished)
                } else {
                    Ok(DecoderRunOutput::Pending)
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // wrap 構造では sink (= tx) は self.sink で生存。 Disconnected が起きるなら
                // 構造体内の self.sink が drop された後で、 本来到達しない。 bug を検出。
                Err(crate::Error::new(
                    "decoder output channel disconnected unexpectedly (sink dropped before rx)"
                ))
            }
        }
    }

    /// 非同期 API (新規)
    ///
    /// `None` 返却は `tx` が drop された場合のみ。 sink は `AsyncVideoDecoder` 内で同居するため、
    /// 通常時に `None` が返ることはない。 もし起きたら `OutputSink` 側の `debug_assert` で
    /// 既に検出されているはずだが、 念のため呼出側は `None` を「内部 sink drop = bug」相当の
    /// 終端として扱う想定 (0068 で各使用側を移行する際の helper 設計と合わせて確定)。
    pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>> {
        self.rx.recv().await
    }

    /// `VideoDecoder::get_engines` と同等の engine 選択ロジック。
    /// 既存 `src/decoder.rs:432-491` のロジックをそのまま移植 (本 issue 内では `AsyncVideoDecoder`
    /// に移植し、 `VideoDecoder::get_engines` は `AsyncVideoDecoder::get_engines` への薄い委譲とする)。
    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> {
        // 実装は既存 VideoDecoder::get_engines の本体をそのまま移植。
        unimplemented!("既存 VideoDecoder::get_engines (src/decoder.rs:432-491) を移植")
    }
}
```

`AsyncVideoDecoder::run` は本 issue (0066) では **提供しない**。 同期 `VideoDecoder::run` 側で既存ロジックを維持し (下記 §)、 非同期版 `run` は使用側が `AsyncVideoDecoder` に切り替わる 0068 で必要性が顕在化した時点で別途設計する (現時点で sink → rx の流路設計だけ確定すれば、 `run` 形態の async 化は 0068 範囲)。

### VideoDecoder の wrap 化と既存 helper の扱い

```rust
pub struct VideoDecoder {
    inner_decoder: AsyncVideoDecoder,
}

impl VideoDecoder {
    pub fn new(options: VideoDecoderOptions, stats: crate::stats::Stats) -> Self {
        Self { inner_decoder: AsyncVideoDecoder::new(options, stats) }
    }

    pub fn handle_input_sample(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        self.inner_decoder.handle_input_sample_sync(sample)
    }

    pub fn poll_output(&mut self) -> Result<DecoderRunOutput> {
        self.inner_decoder.poll_output_sync()
    }

    pub fn handle_input_message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Media(sample) => self.handle_input_sample(Some(sample)),
            Message::Eos => self.handle_input_sample(None),
            Message::Syn(_) => Ok(()),
        }
    }

    /// run は既存実装 (src/decoder.rs:360-391) のロジックをそのまま維持。
    /// 内部で `drain_video_decoder_output(&mut self, ...)` を呼ぶため、 wrap 構造でも
    /// 引き続き `&mut VideoDecoder` を取れる。 delegate ではなく自前で書く。
    pub async fn run(
        mut self,
        handle: ProcessorHandle,
        input_track_id: TrackId,
        output_track_id: TrackId,
    ) -> Result<()> {
        // 既存実装 (src/decoder.rs:360-391) と完全同一の構造を踏襲する:
        //   1. handle が提供する `input_rx.recv().await` で Message を受信
        //   2. self.handle_input_message(msg) で input を inner にディスパッチ
        //      (= self.inner_decoder.handle_input_sample_sync 経由で inner.decode/finish 実行)
        //   3. drain_video_decoder_output(&mut self, &mut output_tx) で
        //      poll_output_sync を Pending まで回しながら output_tx.send_media() に流す
        //   4. is_eos (input_rx 終端) 時点で return Err(終端) もしくは Finished を返す
        //
        // 重要不変条件: 同期 inner は self.handle_input_sample_sync の中で同期完了し、
        // Nvcodec の場合も finish() が flush 待ち合わせ済 (src/decoder/nvcodec.rs の実装より)。
        // よって is_eos に至った時点で sink 内の残物は drain ループで try_recv 完全回収可能。
        // 既存 VecDeque ベースと同じ「`pop` が空になるまで吐き出して終了」の意味論が保たれる。
        //
        // wrap 後でも自己シグネチャは &mut VideoDecoder で変わらないため、
        // drain_video_decoder_output(decoder: &mut VideoDecoder, ...) のシグネチャ不変条件を満たす。
        // 詳細擬似コードは省略 (既存 src/decoder.rs:360-391 を 1 行も変えずに維持できる前提)。
        todo!("既存 src/decoder.rs:360-391 のロジックを wrap 構造のまま維持")
    }

    /// `AsyncVideoDecoder::get_engines` への薄い委譲 (engine 選択ロジック本体は
    /// `AsyncVideoDecoder::get_engines` 内に移植済み)。
    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> {
        AsyncVideoDecoder::get_engines(codec, is_openh264_available)
    }
}
```

**`drain_video_decoder_output` (`src/decoder.rs:514-533`) のシグネチャは不変**: `fn drain_video_decoder_output(decoder: &mut VideoDecoder, output_tx: &mut crate::TrackPublisher) -> Result<DrainResult>`。 内部で `decoder.poll_output()` を呼ぶが、 wrap 構造により `decoder.poll_output() -> self.inner_decoder.poll_output_sync()` の delegate チェーンで動く。 4 ファイルの呼出側は一切書き換え不要。

### `drain_video_decoder_output` 挙動の厳密性

`drain_video_decoder_output` は `poll_output` を Pending が返るまでループする (`src/decoder.rs:514-533`)。 channel 化後の `poll_output_sync` は `rx.try_recv()` を 1 件取って返す。 Nvcodec の callback timing 依存で 1 回の `drain` 呼出中に取れる frame 数が変動する可能性があるが、 **既存 `handle_decoded_frames` も同じ性質を持つ** (本スレッド側 `handle_decoded_frames` 呼出時に積まれた分だけ pop)。 「悪化なし」と言える。

### `VideoDecoderInner::Initial` への OutputSink 引き渡し

```rust
enum VideoDecoderInner {
    Initial { options: VideoDecoderOptions, sink: OutputSink },
    Libvpx(LibvpxDecoder),
    Openh264(Openh264Decoder),
    Dav1d(Dav1dDecoder),
    #[cfg(target_os = "macos")]
    VideoToolbox(Box<VideoToolboxDecoder>),
    #[cfg(feature = "nvcodec")]
    Nvcodec(NvcodecDecoder),
}
```

`Initial` variant に `sink` を含めることで、 `initialize_decoder` (現 `src/decoder.rs:555-668`) 内で `*self = LibvpxDecoder::new_vp8(sink)?` などとして渡せる。 `sink` は `OutputSink::clone()` で安価に複製可能。

`VideoDecoderInner::decode` 自体は **3 引数のまま** (`frame`, `codec_metric`, `engine_metric`) で、 sink を引数として受け取らない (各 variant の inner が自身に sink を内包しているため、 第 4 引数で受ける必要なし)。 `Initial` variant の場合のみ、 `decode` 内で `match self { Self::Initial { options, sink } => self.initialize_decoder(..., options.clone(), sink.clone()) }` のように Initial 内 sink を取り出して `initialize_decoder` に渡し、 そこから実 inner コンストラクタへ `sink.clone()` で引き継ぐ。

### `initialize_decoder` の引数 (現状維持 + sink 追加)

```rust
fn initialize_decoder(
    &mut self,
    frame: &VideoFrame,
    codec_metric: &crate::stats::StatsString,
    engine_metric: &crate::stats::StatsString,
    options: VideoDecoderOptions,
    sink: OutputSink,  // 追加
) -> crate::Result<()>;
```

`OutputSink` は `Clone` (内部 `Arc` bump のみで安価)、 `VideoDecoderOptions` も `Clone` のため、 現状の `initialize_decoder` (`src/decoder.rs:677-680` で `let options = options.clone();` パターン) を踏襲し、 `match self { Self::Initial { options, sink } => (options.clone(), sink.clone()), _ => unreachable!() }` 相当の形で取り出して使う。 `std::mem::replace` を使ったムーブは **不要** (clone コストが上限を超える要素は無い)。

### エラー伝搬の確定方式

エラー伝搬は **二系統のみ** (`last_error` フィールドは使わない):

- **同期 inner (Libvpx / Openh264 / Dav1d / VideoToolbox) の `decode()` / `finish()` Err および `initialize_decoder` Err**: `?` 直返し経路。 `handle_input_sample_sync` 内で `self.inner.decode(...)?` / `self.inner.finish()?` の `?` 演算子で同期返却される (`Initial` 遷移時の `initialize_decoder` Err も同じ経路。 既存 `src/decoder.rs:401-420` の挙動と完全一致)
- **非同期 inner (Nvcodec) callback の `Err`**: channel 経路。 `sink.emit_err(e)` で channel に流し、 同期 `poll_output_sync` 経路では `try_recv` で `Ok(Err(e))` を受信して同期返却 (Nvcodec callback タイミングにより 1 frame 程度の遅延あり、 既存 `error_slot` も同じ性質のため挙動互換)。 非同期 `next_decoded_frame_async` 経路では `rx.recv().await` で `Some(Err(e))` を返却 (呼出側は `match` で Err 捕捉する想定)

`AsyncVideoDecoder` に `last_error: Option<crate::Error>` フィールドを **持たない** (書き込み経路がなく dead field になるため)。

### メトリクス計上の集約と inner 直叩きパターンの不変条件

- `total_input_video_frame_count_metric.inc()`: `AsyncVideoDecoder::handle_input_sample_sync` 入口で 1 回呼出 (= wrap 経由のみ)
- `total_output_video_frame_count_metric.inc()`: `OutputSink::emit_ok` 内で 1 回呼出 (= inner からの emit 経路)

`tests/e2e.rs` のような **inner 直叩きパターン (例: `LibvpxDecoder::new_vp9(sink)` を直接呼ぶ) では `total_input_metric.inc()` は走らない**。 これは設計上の仕様 (テスト内では metric を検証しないため許容)。 完了条件で「テスト内では metric の不変条件は検証しない」を明示する。

### closed/0057 §3 採用案 C の長所 5 項目との対応

closed/0057 §3 で採用案 C の長所として挙げられた 5 項目を本 issue + 0068 で達成する分担:

| 採用案 C の長所 | 本 issue (0066) | 後続 (0068) |
|-----------------|------------------|--------------|
| (i) `VideoEncoderInner` enum の dispatch が `encode()` だけに集約 | inner の `next_decoded_frame()` 系 dispatch を 0066 で廃止 (内部 channel 経由に統一) | - |
| (ii) inner 構造が 1 系統に揃う (全 inner が Sender push 型) | 達成 | - |
| (iii) 上位 aggregation コードが消える | `drain_video_decoder_output` 等は 0068 で削除 | 達成 |
| (iv) テストパターン統一 | 既存テストは挙動互換で維持、 新規 e2e テストは Sender + try_recv 形式 | 同期 API 削除に合わせて全テスト統一 |
| (v) callback friendly 定義 (ホップ数上限 1) を真に満たす | `AsyncVideoDecoder` 経由なら達成 (inner → channel → AsyncVideoDecoder)、 `VideoDecoder` 経由は wrap 1 段追加で 2 段 | 0068 で `AsyncVideoDecoder` 一本に絞れば達成 |

「中途半端な 2 系統共存禁止」は本 issue では意図的に許容し、 0068 完了時に 1 系統に収束させる。 closed/0057 §3 分割表 (line 350-353) には方針 (δ) 採用と本 issue + 0068 の責任分担を本 PR で同時 commit して追記する (完了条件参照)。

### tokio runtime context 要件

- inner の `decode()` / `finish()`、 `UnboundedSender::send` (= `OutputSink::emit_ok/err`) は tokio runtime context **不要**
- 同期 `VideoDecoder::poll_output()` 内の `rx.try_recv()` は runtime **不要**
- 非同期 `AsyncVideoDecoder::next_decoded_frame_async()` の `rx.recv().await` は runtime **必要**

### shiguredo-rust 規約整合

- トレイト追加なし (`VideoDecoderInner` enum を維持)
- `#[non_exhaustive]` 不使用
- モック / スタブ不使用 (テストは実 decoder + tokio channel)
- 規約上の許可取得は不要

## 完了条件

- `AsyncVideoDecoder` が `src/decoder.rs` に新規追加され、 `pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>>` を提供する。 同期 API delegate のための `handle_input_sample_sync` / `poll_output_sync` / `get_engines` も提供する
- `AsyncVideoDecoder::run` は **本 issue では提供しない** (使用側が `AsyncVideoDecoder` に移行する 0068 で必要性が顕在化した時点で別途設計)。 同期 `VideoDecoder::run` は既存 `src/decoder.rs:360-391` のロジックを wrap 構造のまま維持する
- `OutputSink` 構造体が `src/decoder.rs` に追加され、 `emit_ok(frame)` / `emit_err(err)` で `tx.send` + `metric.inc` をペアリングしている
- `VideoDecoder` が `AsyncVideoDecoder` を内包する wrap 構造に変更され、 同期 API (`new`, `handle_input_sample`, `poll_output`, `run`, `handle_input_message`, `get_engines`) の **外部挙動は不変** (戻り値型・タイミング・エラー伝搬経路すべて維持)。 ただし `#[derive(Debug)]` の出力文字列は変わる (`Debug` 表現は外部 API 契約に含まないと判断)
- 各 inner のコンストラクタが `sink: OutputSink` を 1 個受け取って内包する形に変更されている
- 各 inner の `output_queue` / `decoded_queue` / `error_slot` / `decoded: Option<VideoFrame>` 等の中継キューが廃止されている
- 各 inner の `next_decoded_frame()` 系 API が廃止されている
- 各 inner の `decode()` / `finish()` シグネチャは同期 fn のまま維持 (async fn 化なし)
- `VideoDecoderInner::Initial { options, sink }` 拡張済み、 `initialize_decoder` シグネチャに `sink: OutputSink` 追加済み
- `tests/e2e.rs` の `LibvpxDecoder::new_vp9` 呼出 4 ブロック (8 行: `:1359, :1399, :1571, :1616, :1710, :1742, :1813, :1845`) が新 API (`LibvpxDecoder::new_vp9(sink)` + `rx.try_recv()` ベース) に追従している
- `src/decoder.rs:720-821` のエンジン選択テストは `decoder.inner` を参照する全箇所 (`matches!(decoder.inner, ...)` と `std::mem::discriminant(&decoder.inner)`) を `decoder.inner_decoder.inner` に書き換え (wrap 化で `decoder.inner` の経路が 1 段深くなるため)。 実装段階で `grep -n 'decoder\.inner' src/decoder.rs` で全箇所を確認すること
- `MediaPipeline` / `ProcessorHandle` 経由の既存使用側 9 ファイル (compose / vmaf / inspect / mp4 reader / rtmp / rtsp / srt / obsws/file_mp4) は **一切書き換えない**。 検証コマンド: `git diff --stat develop -- src/mp4/ src/rtsp/ src/srt/ src/rtmp/ src/obsws/ src/subcommand_inspect.rs src/sora/recording_subcommand_compose.rs src/sora/recording_subcommand_vmaf.rs` がゼロ行
- **ファイル名 rename**: `0066-feature-refactor-video-decoder-sender-interface.md` → `0066-feature-refactor-add-async-video-decoder.md` (Branch メタと整合)。 本 PR 内で `git mv` する
- **0068 line 30 修正**: `0068-feature-refactor-migrate-video-decoder-users-to-async.md` line 30 の「`VideoDecoder` 内部実装が `UnboundedReceiver` ベースに切り替わっている」を「`VideoDecoder` が内部に `AsyncVideoDecoder` を保持する wrap 構造に切り替わっており、 出力は内部 channel 経由で受け取る」に書き換え (本 PR 内で同時 commit、 0068 polish 時に再確認)
- **0068 line 158 / 0067 line 85 の Branch 表記修正**: ファイル名 rename に合わせて `feature/refactor-add-async-video-decoder` 表記に統一 (本 PR 内で同時 commit)
- **closed/0057 §3 分割表更新**: `issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md` line 350-353 に 0068 行追加 + 末尾備考に方針 (δ) 注記を本 PR 内で同時 commit。 追加文字列の具体例は §解決方法 6 参照
- 新規 end-to-end テスト 3 ケースが追加されている:
  - (a) `src/decoder/nvcodec.rs` 末尾: `NvcodecDecoder` で callback 内 `Err` が次回 `try_recv()` で取得できる。 `#[cfg(feature = "nvcodec")]` ガード + GPU 環境のないテストでも実行可能な形 (構造体生成は `is_cuda_library_available()` 等でガード、 channel 経由検証は `OutputSink` を直接生成して `emit_err` を呼ぶ形で書く)
  - (b) `src/decoder/openh264.rs` 末尾: `Openh264Decoder` で keyframe 入力時の `finish()` 経路フレームと新 frame の順序が保たれる (組合せは 4 通りあり得るため、 順序保証のみを検証)
  - (c) `src/decoder.rs` 末尾: `AsyncVideoDecoder::next_decoded_frame_async()` の `#[tokio::test(flavor = "multi_thread")]` で正常動作確認
- メトリクス検証はテスト内では行わない (inner 直叩きでは `total_input` が増えない仕様)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

### 1. `OutputSink` 型と `DecoderOutputSender` 型エイリアスの確定

`src/decoder.rs` に以下を追加:

```rust
pub type DecoderOutputSender = tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>;

/// inner が出力フレーム / エラーを `AsyncVideoDecoder` 内の rx に流すための sink。
/// `tests/e2e.rs` の integration test から inner を直接構築する経路があるため `pub` で公開する。
/// フィールドは private のままにし、 構築は `OutputSink::new` 経由に統一する
/// (struct literal は別 crate から書けないため、 pub コンストラクタが必須)。
/// `Debug` 派生は `VideoDecoderInner::Initial { sink: OutputSink }` を含む `VideoDecoderInner` の
/// `#[derive(Debug)]` 維持 (既存 `src/decoder.rs:535`) のため必須。
#[derive(Debug, Clone)]
pub struct OutputSink {
    tx: DecoderOutputSender,
    total_output_metric: crate::stats::StatsCounter,
}

impl OutputSink {
    /// 別 crate (`tests/e2e.rs` 等) からも構築できるよう、 pub コンストラクタを提供する。
    pub fn new(tx: DecoderOutputSender, total_output_metric: crate::stats::StatsCounter) -> Self {
        Self { tx, total_output_metric }
    }

    pub fn emit_ok(&self, frame: VideoFrame) {
        self.total_output_metric.inc();
        let send_result = self.tx.send(Ok(frame));
        debug_assert!(
            send_result.is_ok(),
            "decoder output sink receiver dropped before sink (bug)"
        );
    }
    pub fn emit_err(&self, err: crate::Error) {
        let send_result = self.tx.send(Err(err));
        debug_assert!(
            send_result.is_ok(),
            "decoder output sink receiver dropped before sink (bug)"
        );
    }
}
```

### 2. inner の Sender 化 (同期 fn のまま)

設計方針 §「各 inner の Sender 化形態」の表に従って 5 inner を改修:

- 全 inner: `output_queue` / `decoded_queue` / `error_slot` / `decoded: Option<VideoFrame>` を廃止
- 全 inner: コンストラクタに `sink: OutputSink` 追加
- 全 inner の `next_decoded_frame` 系 API 廃止
- VideoToolboxDecoder の `reinitialize_if_need` 内 `decoded.is_some()` ガード (`src/decoder/video_toolbox.rs:133-137, 148-152, 180-184`) を廃止
- Openh264Decoder の `decode()` 内 `self.finish()?` 呼出経路では、 `decode()` / `finish()` の両方で同じ `self.sink.emit_ok(frame)` を呼ぶ (emit パス一本化)
- NvcodecDecoder の `input_queue` 扱いは設計方針 §「inner ごとの個別対応」(a) / (b) のいずれかを実装段階で確定 (暫定推奨 (a)、 (a) 採用時は `Arc<Mutex<VecDeque<VideoFrame>>>` 化)

### 3. AsyncVideoDecoder の新規追加

設計方針 §「AsyncVideoDecoder の擬似実装」に従って実装。 `Initial { options, sink }` 遷移時に `sink.clone()` を実 inner に渡す。

### 4. VideoDecoder の wrap 化

設計方針 §「VideoDecoder の wrap 化と既存 helper の扱い」に従って実装。 `drain_video_decoder_output` のシグネチャは不変。

### 5. tests/e2e.rs の追従

`tests/e2e.rs:1359, 1399, 1571, 1616, 1710, 1742, 1813, 1845` の 4 ブロックで `LibvpxDecoder::new_vp9()` を呼び `decoder.next_decoded_frame()` で出力を取っている箇所を以下に書き換え:

```rust
// `tests/e2e.rs` は integration test (別 crate) のため、 全ての参照は `hisui::*` 経由。
let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
let mut stats = hisui::stats::Stats::new();
let test_metric = stats.counter("test_total_output");
let sink = hisui::decoder::OutputSink::new(tx, test_metric);
let mut decoder = LibvpxDecoder::new_vp9(sink)?;
decoder.decode(&frame)?;
while let Ok(result) = rx.try_recv() {
    let frame = result?;
    // ... 既存検証ロジック
}
```

`OutputSink` および `DecoderOutputSender` は `pub` で公開、 `OutputSink::new` も `pub` のため `tests/e2e.rs` (integration test = 別 crate) から `hisui::decoder::{OutputSink, DecoderOutputSender}` で参照可能。 `hisui::stats::Stats::new` / `Stats::counter` の可視性は `src/stats.rs` で要確認、 必要なら最小限の可視性引き上げ (`pub` 化) を本 PR 内で行う。 `Stats::counter` は `&mut self` を要求するため、 一時変数ではチェーン不能 (上記サンプルのように `let mut stats = ...` で受けてから `stats.counter(...)` で呼ぶ)。

### 6. closed/0057 §3 分割表の追記 (本 PR 同時 commit)

`issues/closed/0057-feature-refactor-callback-friendly-codec-interface.md` line 350-353 への追加内容:

```diff
 | ID | 範囲 | 推定 LOC | 依存先 | 後方互換影響 |
 |----|------|----------|---------|---------------|
-| open/0066 (`feature/refactor-video-decoder-sender-interface`) | VideoDecoder + 全 inner (Libvpx/Openh264/Dav1d/VideoToolbox/Nvcodec) を Sender 出力に統一、`error_slot` 廃止 | 千行前後 | なし | 内部 API のみ |
+| open/0066 (`feature/refactor-add-async-video-decoder`) | AsyncVideoDecoder 新規追加 + VideoDecoder の wrap 化 + 全 inner (Libvpx/Openh264/Dav1d/VideoToolbox/Nvcodec) の Sender 化 (OutputSink 経由)、 既存外部 API 維持 | 千行前後 | なし | 内部 API のみ |
+| open/0068 (`feature/refactor-migrate-video-decoder-users-to-async`) | 0066 完了後に各使用側を AsyncVideoDecoder に移行 + 最終クリーンアップ (同期 VideoDecoder 削除 + AsyncVideoDecoder を VideoDecoder にリネーム) | 千行台 | 0066 | 内部 API のみ |
 | open/0067 (`feature/refactor-video-encoder-sender-interface`) | VideoEncoder + 全 inner (Libvpx/Openh264/SvtAv1/VideoToolbox/Nvcodec) を Sender 出力に統一、`NvcodecEncoder` の `flush()` 強制撤廃、`error_slot` 廃止、メトリクス計上の `run()` 受信側移植、RPC keyframe 経路維持 | 千行台 | 内部 API のみ |
```

末尾備考に方針 (δ) 注記を追加:

```
- 方針 (δ): 0066 polish 後の Decision Owner 判断で「2 系統共存を意図的に許容し 0068 で最終解消する派生」を採用。 0066 + 0068 で採用案 C の長所 5 項目を分担達成 (詳細は open/0066 §設計方針)
```

### 7. ファイル名 rename と関連 issue の Branch 表記更新 (本 PR 同時 commit)

- `git mv issues/0066-feature-refactor-video-decoder-sender-interface.md issues/0066-feature-refactor-add-async-video-decoder.md`
- `issues/0068-feature-refactor-migrate-video-decoder-users-to-async.md` line 158 の `feature/refactor-add-async-video-decoder` 表記を確認 (既に新名なら不変)
- `issues/0067-feature-refactor-video-encoder-sender-interface.md` line 85 の旧 Branch 名参照を新名 `feature/refactor-add-async-video-decoder` に置換

### 8. end-to-end テスト追加

完了条件 (a)(b)(c) の 3 ケースを以下に配置:

- (a) `src/decoder/nvcodec.rs` 末尾: `OutputSink` を直接生成して `emit_err` を呼び、 `rx.try_recv()` で `Err` を取得できることを検証 (GPU 不要、 channel 部分のみ)。 加えて `#[cfg(feature = "nvcodec")]` で構造体生成テストも別途追加 (`is_cuda_library_available()` でガード)
- (b) `src/decoder/openh264.rs` 末尾: 実 H.264 fixture (`tests/e2e.rs` 流用) で keyframe 入力時の順序保証検証
- (c) `src/decoder.rs` 末尾: `AsyncVideoDecoder::next_decoded_frame_async()` の `#[tokio::test(flavor = "multi_thread")]` で正常動作確認

### 9. 既存テストの追従

`src/decoder.rs:720-821` のエンジン選択テストは `decoder.inner` を参照する全箇所 (`matches!(decoder.inner, ...)` と `std::mem::discriminant(&decoder.inner)`) を `decoder.inner_decoder.inner` に書き換える最小改変で追従する (wrap 化で 1 段深くなるため)。 実装段階で `grep -n 'decoder\.inner' src/decoder.rs` で全箇所を確認する。

### 10. cargo 系チェック全通過

`cargo fmt` / `cargo check` (default + `--no-default-features`) / `cargo clippy` / `cargo test` を完了条件全項目で通す。

## 段階的移行 (本 issue では実施しない)

各使用側の `VideoDecoder` → `AsyncVideoDecoder` への移行は **open issue 0068 で 1 件にまとめて段階的に** 実施 (0068 着手段階で必要に応じて細分化)。 最終的に同期 `VideoDecoder` の使用箇所がゼロになった時点で `VideoDecoder` 自体を削除し、 `AsyncVideoDecoder` を `VideoDecoder` にリネームする最終ステップも 0068 に含む。

移行候補の優先順位は open/0068 「現状」§参照。

## CHANGES.md について

内部リファクタにつき記載不要。 `VideoDecoder` 系は library として外部公開していないため、 API 変更の後方互換影響は本 issue ではゼロ (既存 API 挙動を維持)。 後続 0068 でも各々判断する。

## 関連

- closed/0057 (`feature/refactor-callback-friendly-codec-interface`): 設計検討の親 issue。 本 issue は §3 採用案 C の decoder 部分実装 (派生方針 (δ))。 本 issue 完了時に 0057 §3 分割表に方針 (δ) 採用を追記 (完了条件参照)
- closed/0054 (`feature/refactor-encoder-defer-output-until-sample-entry-ready`): encoder で sample_entry 未確定時の出力を `Err` 化する fail-fast 整備。 decoder 側は sample_entry 不変条件の対象外だが、 本 issue でも `Result<VideoFrame>` 流路を採用
- open/0067 (`feature/refactor-video-encoder-sender-interface`): encoder 側。 本 issue で確立した「`Async*` 追加 + 同期既存維持 + 段階移行」パターンを encoder に展開する想定。 ただし encoder 固有要件 (NVENC `flush()` 強制撤廃のための bp 機構、 RPC keyframe、 sample_entry 不変条件、 メトリクス重) があるため、 0067 は (δ) 方針への書き換えと polish 再実施が必要。 0067 polish 時に Decision Owner メタの追加と、 本 issue の Branch 表記更新 (line 85) を同時に行う
- open/0068 (`feature/refactor-migrate-video-decoder-users-to-async`): フォローアップ。 本 issue で確立した `AsyncVideoDecoder` への全使用側移行と最終クリーンアップ (`VideoDecoder` 削除 + `AsyncVideoDecoder` リネーム)。 0068 polish 時に「現状」§ line 30 の wrap 構造記述更新と Decision Owner メタ追加を行う
