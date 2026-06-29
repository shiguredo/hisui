use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::{DecodeConfig, OutputSink};
use crate::video::h264::{H264_NALU_TYPE_PPS, H264_NALU_TYPE_SPS};
use crate::video::h265::{
    H265_NALU_TYPE_PPS, H265_NALU_TYPE_SPS, H265_NALU_TYPE_VPS, NALU_HEADER_LENGTH,
};
use crate::video::{VideoFormat, VideoFrame};

/// 本スレッド側 (`decode()` 呼出側) が push し、 CUDA worker thread から呼ばれる
/// callback 側が pop する FIFO キュー。 lock 保持区間は **push_back / pop_front のみ** に限定し、
/// 重い処理 (NV12→I420 変換等) は lock 解放後に実行する。
type InputQueue = Arc<Mutex<VecDeque<VideoFrame>>>;

#[derive(Debug)]
pub struct NvcodecDecoder {
    inner: shiguredo_nvcodec::Decoder<
        shiguredo_nvcodec::FnDecodeHandler<(), shiguredo_nvcodec::Error>,
    >,
    // decode() で push、 callback で pop する FIFO キュー
    // (callback 側で I420 変換 + emit を行うため Arc<Mutex<VecDeque>> 化している)
    input_queue: InputQueue,
    parameter_sets: Option<Vec<u8>>, // VPS/SPS/PPS をキャッシュ (本スレッド側のみが更新する)
}

/// CUDA worker thread から呼ばれる callback の本体を共有 closure 化したもの。
///
/// `input_queue` の `Mutex` ホールドスコープは **`pop_front()` のみ** に限定し、
/// NV12→I420 変換は lock 解放後に実行する (本スレッド側次回 `decode()` の `push_back` を
/// 不必要にブロックしないため)。
fn build_handler(
    input_queue: InputQueue,
    sink: OutputSink,
) -> shiguredo_nvcodec::FnDecodeHandler<(), shiguredo_nvcodec::Error> {
    shiguredo_nvcodec::FnDecodeHandler::new(move |result| {
        handle_decode_callback(&input_queue, &sink, result);
    })
}

fn handle_decode_callback(
    input_queue: &InputQueue,
    sink: &OutputSink,
    result: std::result::Result<shiguredo_nvcodec::DecodedFrame<()>, shiguredo_nvcodec::Error>,
) {
    match result {
        Ok(nv12_frame) => {
            // lock 保持は pop_front のみ。 libyuv 変換は lock 解放後。
            let input_frame = {
                let mut q = input_queue
                    .lock()
                    .expect("nvcodec input queue lock poisoned");
                q.pop_front()
            };
            let Some(input_frame) = input_frame else {
                sink.emit_err(crate::Error::new(
                    "decoded frame produced without input frame",
                ));
                return;
            };
            match convert_nv12_to_i420(input_frame, nv12_frame) {
                Ok(frame) => sink.emit_ok(frame),
                Err(err) => sink.emit_err(err),
            }
        }
        Err(err) => {
            clear_input_queue_and_emit_err(
                input_queue,
                sink,
                crate::Error::new(format!("nvcodec decode error: {err}")),
            );
        }
    }
}

/// callback の Err 分岐共通処理: `input_queue` をクリアして `sink` に `Err` を流す。
///
/// shiguredo_nvcodec lib 側は `drain_frames` の Err 時に `pending_user_data.clear()` を行うため、
/// hisui 側の `input_queue` も同じタイミングでクリアして残骸を残さない
/// (残しておくと後続 decode で input_frame / nv12_frame の timestamp 入れ違いという silent bug を生む)。
fn clear_input_queue_and_emit_err(input_queue: &InputQueue, sink: &OutputSink, err: crate::Error) {
    {
        let mut q = input_queue
            .lock()
            .expect("nvcodec input queue lock poisoned");
        q.clear();
    }
    sink.emit_err(err);
}

/// NV12 フォーマットの decoded frame を I420 に変換して `VideoFrame` を構築する
fn convert_nv12_to_i420(
    input_frame: VideoFrame,
    nv12_frame: shiguredo_nvcodec::DecodedFrame<()>,
) -> crate::Result<VideoFrame> {
    let width = nv12_frame.width();
    let height = nv12_frame.height();

    let y_size = width * height;
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let uv_size = uv_width * uv_height;
    let total_size = y_size + uv_size * 2;

    let mut i420_data = vec![0u8; total_size];
    let (y_plane, rest) = i420_data.split_at_mut(y_size);
    let (u_plane, v_plane) = rest.split_at_mut(uv_size);

    let src = shiguredo_libyuv::Nv12Image {
        y: nv12_frame.y_plane(),
        y_stride: nv12_frame.y_stride(),
        uv: nv12_frame.uv_plane(),
        uv_stride: nv12_frame.uv_stride(),
    };
    let mut dst = shiguredo_libyuv::I420ImageMut {
        y: y_plane,
        y_stride: width,
        u: u_plane,
        u_stride: uv_width,
        v: v_plane,
        v_stride: uv_width,
    };

    let size = shiguredo_libyuv::ImageSize::new(width, height);
    shiguredo_libyuv::nv12_to_i420(&src, &mut dst, size)?;

    Ok(VideoFrame::new_i420(
        input_frame,
        width,
        height,
        y_plane,
        u_plane,
        v_plane,
        width,
        uv_width,
        uv_width,
    ))
}

impl NvcodecDecoder {
    pub fn new_h264(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(H264) decoder");
        let mut config = params.nvcodec_h264.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::H264;
        let input_queue: InputQueue = Arc::new(Mutex::new(VecDeque::new()));
        let handler = build_handler(input_queue.clone(), sink);
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, handler)?,
            input_queue,
            parameter_sets: None,
        })
    }

    pub fn new_h265(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(H265) decoder");
        let mut config = params.nvcodec_h265.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Hevc;
        let input_queue: InputQueue = Arc::new(Mutex::new(VecDeque::new()));
        let handler = build_handler(input_queue.clone(), sink);
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, handler)?,
            input_queue,
            parameter_sets: None,
        })
    }

    pub fn new_av1(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(AV1) decoder");
        let mut config = params.nvcodec_av1.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Av1;
        let input_queue: InputQueue = Arc::new(Mutex::new(VecDeque::new()));
        let handler = build_handler(input_queue.clone(), sink);
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, handler)?,
            input_queue,
            parameter_sets: None,
        })
    }

    pub fn new_vp8(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(VP8) decoder");
        let mut config = params.nvcodec_vp8.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Vp8;
        let input_queue: InputQueue = Arc::new(Mutex::new(VecDeque::new()));
        let handler = build_handler(input_queue.clone(), sink);
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, handler)?,
            input_queue,
            parameter_sets: None,
        })
    }

    pub fn new_vp9(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(VP9) decoder");
        let mut config = params.nvcodec_vp9.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Vp9;
        let input_queue: InputQueue = Arc::new(Mutex::new(VecDeque::new()));
        let handler = build_handler(input_queue.clone(), sink);
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, handler)?,
            input_queue,
            parameter_sets: None,
        })
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
        if !matches!(
            frame.format,
            VideoFormat::H264
                | VideoFormat::H264AnnexB
                | VideoFormat::H265
                | VideoFormat::Vp8
                | VideoFormat::Vp9
                | VideoFormat::Av1
        ) {
            return Err(crate::Error::new(format!(
                "unsupported input format for NVDEC: {:?}",
                frame.format
            )));
        }

        // サンプルエントリーからパラメータセットを抽出してキャッシュ (本スレッド側のみ更新)
        if self.parameter_sets.is_none()
            && let Some(sample_entry) = &frame.sample_entry
        {
            self.parameter_sets = Some(extract_parameter_sets_annexb(
                sample_entry.get(),
                frame.format,
            )?);
        }

        let data = if matches!(
            frame.format,
            VideoFormat::Vp8 | VideoFormat::Vp9 | VideoFormat::Av1
        ) {
            // VP8 / VP9 / AV1 の場合は Annex B 形式は存在しないので、データの変換は不要
            Cow::Borrowed(&frame.data)
        } else if frame.format == VideoFormat::H264AnnexB {
            // すでに Annex B 形式の場合はそのまま使用
            Cow::Borrowed(&frame.data)
        } else {
            // Annex.B 形式に変換する (H264/H265)
            let mut data = &frame.data[..];
            let mut data_annexb = Vec::new();

            // キーフレームで、かつパラメータセットがデータに含まれていない場合は先頭に追加
            if frame.keyframe
                && let Some(parameter_sets) = &self.parameter_sets
                && !contains_parameter_sets(data, frame.format)
            {
                data_annexb.extend_from_slice(parameter_sets);
            }

            while !data.is_empty() {
                if data.len() < NALU_HEADER_LENGTH {
                    return Err(crate::Error::new(format!(
                        "invalid AVC/HEVC payload: NALU length header is truncated (remaining={})",
                        data.len()
                    )));
                }
                let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
                data = &data[NALU_HEADER_LENGTH..];

                if data.len() < n {
                    return Err(crate::Error::new(format!(
                        "invalid AVC/HEVC payload: NALU data is truncated (required={n}, remaining={})",
                        data.len()
                    )));
                }
                data_annexb.extend_from_slice(&[0, 0, 0, 1]);
                data_annexb.extend_from_slice(&data[..n]);

                data = &data[n..];
            }

            Cow::Owned(data_annexb)
        };

        self.inner.decode(&data, ())?;
        // input_queue の lock 保持は push_back のみ。
        {
            let mut q = self
                .input_queue
                .lock()
                .expect("nvcodec input queue lock poisoned");
            q.push_back(frame.to_stripped());
        }
        Ok(())
    }

    /// in-flight フレームの decode 完了を待ち合わせる。
    ///
    /// `shiguredo_nvcodec::Decoder::flush()` の戻り時点で callback はすべて同期的に呼び切られている
    /// 前提であり、 残フレーム / Err の emit はその callback 内で完了するため、 ここでは追加処理不要。
    ///
    /// **重要**: callback 内で発生した Err は `sink.emit_err()` 経由で内部 channel に積まれており、
    /// `finish()` の戻り値からは検出できない。 利用側は `finish()` の直後に `poll_output_sync` の
    /// `try_recv` ループ (= `drain_video_decoder_output` 経由) で残物を全て吸い出すこと。
    /// 旧実装 (`error_slot` 同期 take) との挙動互換は `VideoDecoder::run` の drain ループで担保される。
    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.flush()?;
        Ok(())
    }
}

/// サンプルエントリーからパラメータセットを Annex.B 形式で抽出
fn extract_parameter_sets_annexb(
    sample_entry: &shiguredo_mp4::boxes::SampleEntry,
    format: VideoFormat,
) -> crate::Result<Vec<u8>> {
    use shiguredo_mp4::boxes::SampleEntry;

    match (sample_entry, format) {
        (SampleEntry::Hev1(entry), VideoFormat::H265) => {
            let mut annexb_data = Vec::new();
            for array in &entry.hvcc_box.nalu_arrays {
                for nalu in &array.nalus {
                    annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                    annexb_data.extend_from_slice(nalu);
                }
            }
            Ok(annexb_data)
        }
        (SampleEntry::Hvc1(entry), VideoFormat::H265) => {
            let mut annexb_data = Vec::new();
            for array in &entry.hvcc_box.nalu_arrays {
                for nalu in &array.nalus {
                    annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                    annexb_data.extend_from_slice(nalu);
                }
            }
            Ok(annexb_data)
        }
        (SampleEntry::Avc1(entry), VideoFormat::H264) => {
            let mut annexb_data = Vec::new();
            // SPS
            for sps in &entry.avcc_box.sps_list {
                annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                annexb_data.extend_from_slice(sps);
            }
            // PPS
            for pps in &entry.avcc_box.pps_list {
                annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                annexb_data.extend_from_slice(pps);
            }
            Ok(annexb_data)
        }
        _ => {
            // VP8 / VP9 / AV1はパラメータセットを個別に送る必要がないため空のVecを返す
            Ok(Vec::new())
        }
    }
}

/// データの先頭にパラメータセットが含まれているかチェック
fn contains_parameter_sets(data: &[u8], format: VideoFormat) -> bool {
    if data.len() < NALU_HEADER_LENGTH + 1 {
        return false;
    }

    match format {
        VideoFormat::H265 => {
            // H.265 の NAL unit type は 2バイト目の上位6ビット
            let nal_unit_type = (data[NALU_HEADER_LENGTH] >> 1) & 0x3F;
            matches!(
                nal_unit_type,
                H265_NALU_TYPE_PPS | H265_NALU_TYPE_SPS | H265_NALU_TYPE_VPS
            )
        }
        VideoFormat::H264 => {
            // H.264 の NAL unit type は下位5ビット
            let nal_unit_type = data[NALU_HEADER_LENGTH] & 0x1F;
            matches!(nal_unit_type, H264_NALU_TYPE_SPS | H264_NALU_TYPE_PPS)
        }
        VideoFormat::Av1 => {
            // AV1はパラメータセットの概念が異なるため常にfalse
            false
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::Stats;
    use crate::video::VideoFormat;

    /// テスト用 `VideoFrame` を 1 件生成する (data・フォーマットは適当な値)
    fn make_dummy_video_frame() -> VideoFrame {
        VideoFrame {
            data: vec![0x00],
            format: VideoFormat::H264,
            keyframe: true,
            size: None,
            timestamp: std::time::Duration::ZERO,
            sample_entry: None,
        }
    }

    /// callback の Err 分岐に相当する `clear_input_queue_and_emit_err` が
    /// (1) `input_queue` の残骸を完全クリアし、 (2) `sink` 経由で `Err` を rx へ流す
    /// ことを検証する。
    ///
    /// これにより shiguredo_nvcodec lib 側 `pending_user_data.clear()` と整合させた
    /// silent timestamp 入れ違い bug の回帰検出を担保する。
    #[test]
    fn clear_input_queue_and_emit_err_clears_queue_and_sends_error() {
        // input_queue に「callback Err 発生時点で残ってしまうはずだった」残骸を 2 件 push する
        let input_queue: InputQueue = Arc::new(Mutex::new(VecDeque::from(vec![
            make_dummy_video_frame(),
            make_dummy_video_frame(),
        ])));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut stats = Stats::new();
        let counter = stats.counter("test_total_output");
        let sink = OutputSink::new(tx, counter.clone());

        clear_input_queue_and_emit_err(
            &input_queue,
            &sink,
            crate::Error::new("test nvcodec callback error"),
        );

        // (1) input_queue の残骸が完全クリアされている
        assert!(
            input_queue.lock().expect("lock").is_empty(),
            "callback Err 後に input_queue がクリアされているはず (timestamp 入れ違い silent bug 防止)"
        );
        // (2) sink 経由で Err が rx に届いている
        match rx.try_recv() {
            Ok(Err(e)) => {
                let msg = e.display().to_string();
                assert!(
                    msg.contains("test nvcodec callback error"),
                    "予期したエラーメッセージが含まれていない: {msg}"
                );
            }
            other => panic!("Ok(Err(_)) を期待したが {other:?} を受信した"),
        }
        // emit_err は counter を inc しない (R-4 の二重計上禁止契約)
        assert_eq!(
            counter.get(),
            0,
            "Err 経路は total_output_metric を inc しないはず"
        );
    }
}
