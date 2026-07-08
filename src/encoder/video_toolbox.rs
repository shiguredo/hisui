use std::collections::VecDeque;

use crate::{
    encoder::{OutputSink, VideoEncoderOptions},
    sample_entry::SharedSampleEntry,
    types::{CodecName, EvenUsize},
    video::h264,
    video::h265,
    video::{FrameRate, RawVideoFrame, VideoFormat, VideoFrame, VideoFrameSize},
};

#[derive(Debug)]
pub struct VideoToolboxEncoder {
    inner: shiguredo_video_toolbox::Encoder,
    input_queue: VecDeque<RawVideoFrame>,
    sink: OutputSink,
    // 最初の出力フレームの SPS/PPS から確定するサンプルエントリー。確定後は全フレームに載せる。
    sample_entry: Option<SharedSampleEntry>,
    width: EvenUsize,
    height: EvenUsize,
    format: VideoFormat,
    fps: FrameRate,
    keyframe_request_pending: bool,
}

impl VideoToolboxEncoder {
    pub fn new_h264(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let width = options.width;
        let height = options.height;
        let mut config = options.encode_params.video_toolbox_h264.clone();
        config.width = u32::try_from(width.get())
            .map_err(|_| crate::Error::new("video width is too large for VideoToolbox"))?;
        config.height = u32::try_from(height.get())
            .map_err(|_| crate::Error::new("video height is too large for VideoToolbox"))?;
        config.average_bitrate = Some(options.bitrate as u64);
        config.fps_numerator = options.frame_rate.numerator.get() as u32;
        config.fps_denominator = options.frame_rate.denumerator.get() as u32;
        if !matches!(config.codec, shiguredo_video_toolbox::CodecConfig::H264(_)) {
            return Err(crate::Error::new(
                "BUG: VideoToolbox H.264 config must use H264 codec settings",
            ));
        }
        // B フレーム並べ替えを許すと VTCompressionSession の出力順が入力順と異なり、
        // handle_encoded の input_queue ペアリングで timestamp 同期が崩れる。
        // shiguredo_video_toolbox 側も allow_frame_reordering: false を前提として
        // 設計されているため、true で生成しようとした時点でコンストラクタ Err にする。
        if config.allow_frame_reordering {
            return Err(crate::Error::new(
                "VideoToolbox H.264 encoder does not support allow_frame_reordering=true \
                 (timestamp synchronization assumes reorder-free output)",
            ));
        }
        let inner = shiguredo_video_toolbox::Encoder::new(config)?;
        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            sink,
            sample_entry: None,
            width,
            height,
            format: VideoFormat::H264,
            fps: options.frame_rate,
            keyframe_request_pending: false,
        })
    }

    pub fn new_h265(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let width = options.width;
        let height = options.height;
        let mut config = options.encode_params.video_toolbox_h265.clone();
        config.width = u32::try_from(width.get())
            .map_err(|_| crate::Error::new("video width is too large for VideoToolbox"))?;
        config.height = u32::try_from(height.get())
            .map_err(|_| crate::Error::new("video height is too large for VideoToolbox"))?;
        config.average_bitrate = Some(options.bitrate as u64);
        config.fps_numerator = options.frame_rate.numerator.get() as u32;
        config.fps_denominator = options.frame_rate.denumerator.get() as u32;
        if !matches!(config.codec, shiguredo_video_toolbox::CodecConfig::Hevc(_)) {
            return Err(crate::Error::new(
                "BUG: VideoToolbox H.265 config must use HEVC codec settings",
            ));
        }
        // B フレーム並べ替えを許すと VTCompressionSession の出力順が入力順と異なり、
        // handle_encoded の input_queue ペアリングで timestamp 同期が崩れる。
        // shiguredo_video_toolbox 側も allow_frame_reordering: false を前提として
        // 設計されているため、true で生成しようとした時点でコンストラクタ Err にする。
        if config.allow_frame_reordering {
            return Err(crate::Error::new(
                "VideoToolbox H.265 encoder does not support allow_frame_reordering=true \
                 (timestamp synchronization assumes reorder-free output)",
            ));
        }
        let inner = shiguredo_video_toolbox::Encoder::new(config)?;
        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            sink,
            sample_entry: None,
            width,
            height,
            format: VideoFormat::H265,
            fps: options.frame_rate,
            keyframe_request_pending: false,
        })
    }

    pub fn codec(&self) -> CodecName {
        if self.format == VideoFormat::H264 {
            CodecName::H264
        } else {
            CodecName::H265
        }
    }

    pub fn encode(&mut self, frame: RawVideoFrame) -> crate::Result<()> {
        let (y_plane, u_plane, v_plane) = frame.as_i420_planes()?;
        self.inner.encode(
            &shiguredo_video_toolbox::FrameData::I420 {
                y: y_plane,
                u: u_plane,
                v: v_plane,
            },
            &shiguredo_video_toolbox::EncodeOptions {
                force_key_frame: self.keyframe_request_pending,
            },
        )?;
        self.keyframe_request_pending = false;

        // Video Toolbox のエンコーダーは非同期で動作し、
        // エンコードが終わるまでは入力バッファへの参照を保持する必要があるので、
        // バッファもキューに入れておく。
        // (将来的にはこの辺りはエンコーダー内で隠蔽した方が使いやすそう）
        self.input_queue.push_back(frame);

        self.handle_encoded()?;

        Ok(())
    }

    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.finish()?;
        self.handle_encoded()?;
        Ok(())
    }

    pub fn request_keyframe(&mut self) {
        self.keyframe_request_pending = true;
    }

    fn handle_encoded(&mut self) -> crate::Result<()> {
        while let Some(frame) = self.inner.next_frame()? {
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("encoded frame produced without input frame"))?;
            // 最初の出力フレームの SPS / PPS (H.265 は VPS も) からサンプルエントリーを
            // 確定して保持し、以後の全出力フレームには保持済みのサンプルエントリーを載せる。
            // shiguredo_video_toolbox は keyframe 出力時のみ SPS / PPS / VPS を返すため、
            // 非 keyframe フレームでは frame.sps_list 等が空になる。
            // sample_entry 未確定のまま素材が空の出力が来たら writer 入口の不変条件
            // (圧縮フレームの sample_entry は有効でなければならない) を満たせないため
            // fail-fast 停止する。h264 / h265 のヘルパー関数も空入力では Err を返すが、
            // ここで先に空チェックして PTS / keyframe を含む診断情報付き Err を返す。
            if self.sample_entry.is_none() {
                let sample_entry = if self.format == VideoFormat::H264 {
                    if frame.sps_list.is_empty() || frame.pps_list.is_empty() {
                        return Err(crate::Error::new(format!(
                            "video_toolbox encoder produced H.264 output before SPS/PPS established the sample_entry \
                             (pts={:?}, keyframe={})",
                            input_frame.as_video_frame().timestamp,
                            frame.keyframe,
                        )));
                    }
                    let (entry, _frame_size) = h264::h264_sample_entry_from_sps_pps_lists(
                        frame.sps_list.clone(),
                        frame.pps_list.clone(),
                    )?;
                    entry
                } else {
                    if frame.vps_list.is_empty()
                        || frame.sps_list.is_empty()
                        || frame.pps_list.is_empty()
                    {
                        return Err(crate::Error::new(format!(
                            "video_toolbox encoder produced H.265 output before VPS/SPS/PPS established the sample_entry \
                             (pts={:?}, keyframe={})",
                            input_frame.as_video_frame().timestamp,
                            frame.keyframe,
                        )));
                    }
                    let (entry, _frame_size) = h265::h265_sample_entry_from_vps_sps_pps_lists(
                        frame.vps_list.clone(),
                        frame.sps_list.clone(),
                        frame.pps_list.clone(),
                        self.fps,
                    )?;
                    entry
                };
                self.sample_entry = Some(SharedSampleEntry::new(sample_entry));
            }

            self.sink.emit_ok(VideoFrame {
                data: frame.data,
                format: self.format,
                keyframe: frame.keyframe,
                size: Some(VideoFrameSize {
                    width: self.width.get(),
                    height: self.height.get(),
                }),
                timestamp: input_frame.as_video_frame().timestamp,
                sample_entry: self.sample_entry.clone(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;
    use crate::encoder::test_helpers::{make_encoder_sink, raw_i420_frame};

    fn options() -> VideoEncoderOptions {
        VideoEncoderOptions {
            codec: CodecName::H264,
            engines: None,
            bitrate: 500_000,
            width: EvenUsize::truncating_new(64),
            height: EvenUsize::truncating_new(64),
            frame_rate: FrameRate {
                numerator: NonZeroUsize::MIN.saturating_add(29),
                denumerator: NonZeroUsize::MIN,
            },
            encode_params: crate::encoder::default_video_encode_config_for_rpc(),
        }
    }

    // 全出力フレームに sample_entry が載る不変条件を検証する。
    // VideoToolbox は最初の keyframe で SPS/PPS が確定し、以降は保持値を伝播する。
    fn assert_every_output_frame_has_sample_entry(
        mut encoder: VideoToolboxEncoder,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>,
    ) -> crate::Result<()> {
        let mut output_count = 0;
        for i in 0..10u64 {
            encoder.encode(raw_i420_frame(i * 33))?;
            while let Ok(result) = rx.try_recv() {
                let frame = result?;
                assert!(
                    frame.sample_entry.is_some(),
                    "出力フレームに sample_entry が載っていない（フレーム番号: {output_count}）"
                );
                output_count += 1;
            }
        }
        encoder.finish()?;
        while let Ok(result) = rx.try_recv() {
            let frame = result?;
            assert!(
                frame.sample_entry.is_some(),
                "finish 後の出力フレームに sample_entry が載っていない"
            );
            output_count += 1;
        }
        // 全フレーム付与を確認するには 2 フレーム以上の出力が必要。
        assert!(
            output_count >= 2,
            "出力フレーム数が少なすぎる（確認には 2 フレーム以上必要）: {output_count}"
        );
        Ok(())
    }

    #[test]
    fn video_toolbox_h264_sets_sample_entry_on_every_output_frame() -> crate::Result<()> {
        let (sink, rx) = make_encoder_sink();
        let encoder = VideoToolboxEncoder::new_h264(&options(), sink)?;
        assert_every_output_frame_has_sample_entry(encoder, rx)
    }

    #[test]
    fn video_toolbox_h265_sets_sample_entry_on_every_output_frame() -> crate::Result<()> {
        let (sink, rx) = make_encoder_sink();
        let encoder = VideoToolboxEncoder::new_h265(&options(), sink)?;
        assert_every_output_frame_has_sample_entry(encoder, rx)
    }
}
