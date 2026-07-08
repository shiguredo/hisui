//! 各エンコーダの単体テストから共有する補助関数。

use std::sync::Arc;
use std::time::Duration;

use crate::encoder::OutputSink;
use crate::video::{RawVideoFrame, VideoFormat, VideoFrame, VideoFrameSize};

// 64x64 の I420 グレーフレームを作る。
// Y は 16、UV は 128 (BT.601 / BT.709 の中性グレー相当) で、各エンコーダの
// sample_entry 不変条件テストの入力に使う。
pub(crate) fn raw_i420_frame(ts_ms: u64) -> RawVideoFrame {
    let (width, height) = (64usize, 64usize);
    let y_size = width * height;
    let uv_size = (width / 2) * (height / 2);
    let data: Vec<u8> = std::iter::repeat_n(16u8, y_size)
        .chain(std::iter::repeat_n(128u8, uv_size * 2))
        .collect();
    let frame = VideoFrame {
        data,
        format: VideoFormat::I420,
        keyframe: true,
        size: Some(VideoFrameSize { width, height }),
        timestamp: Duration::from_millis(ts_ms),
        sample_entry: None,
    };
    RawVideoFrame::from_i420_video_frame(Arc::new(frame)).expect("有効な I420 フレームのはず")
}

// テスト用の OutputSink と Receiver を生成する。
pub(crate) fn make_encoder_sink() -> (
    OutputSink,
    tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>,
) {
    let (sink, rx, _total, _keyframe) = make_encoder_sink_with_counters();
    (sink, rx)
}

// テスト用の OutputSink / Receiver / 内部で使う 2 つのカウンターを返す。
// OutputSink 契約テスト (emit_ok の keyframe 分岐、 emit_err の非 inc、 clone の共有等)
// では counter の外部観測が必須で、 make_encoder_sink だけでは検証できない。
pub(crate) fn make_encoder_sink_with_counters() -> (
    OutputSink,
    tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>,
    crate::stats::StatsCounter,
    crate::stats::StatsCounter,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut stats = crate::stats::Stats::new();
    let total_output = stats.counter("test_total_output");
    let total_keyframe = stats.counter("test_total_keyframe");
    // counter は Arc 内部の Clone なので、 sink 側と外部観測側で同一インスタンスを共有する。
    // struct literal で field 名指定にして、 同型 StatsCounter 2 個の取り違えバグを防ぐ。
    let sink = OutputSink {
        tx,
        total_output_metric: total_output.clone(),
        total_output_keyframe_metric: total_keyframe.clone(),
    };
    (sink, rx, total_output, total_keyframe)
}
