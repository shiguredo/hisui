use std::collections::VecDeque;

use crate::{
    encoder::{OutputSink, VideoEncoderOptions},
    sample_entry::SharedSampleEntry,
    types::EvenUsize,
    video::av1,
    video::{RawVideoFrame, VideoFormat, VideoFrame, VideoFrameSize},
};

#[derive(Debug)]
pub struct SvtAv1Encoder {
    inner: shiguredo_svt_av1::Encoder,
    input_queue: VecDeque<RawVideoFrame>,
    sink: OutputSink,
    // 全出力フレームに載せるサンプルエントリー。Arc 共有なので毎フレームの clone は安価。
    sample_entry: SharedSampleEntry,
    width: EvenUsize,
    height: EvenUsize,
    keyframe_request_pending: bool,
}

impl SvtAv1Encoder {
    pub fn new(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let width = options.width;
        let height = options.height;
        let config = shiguredo_svt_av1::EncoderConfig {
            target_bit_rate: options.bitrate,
            width: width.get(),
            height: height.get(),
            fps_numerator: options.frame_rate.numerator.get(),
            fps_denominator: options.frame_rate.denumerator.get(),
            ..options.encode_params.svt_av1.clone()
        };
        let inner = shiguredo_svt_av1::Encoder::new(config)?;
        let sample_entry = av1::av1_sample_entry(width, height, inner.extra_data());

        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            sink,
            sample_entry: SharedSampleEntry::new(sample_entry),
            width,
            height,
            keyframe_request_pending: false,
        })
    }

    pub fn encode(&mut self, frame: RawVideoFrame) -> crate::Result<()> {
        let (y_plane, u_plane, v_plane) = frame.as_i420_planes()?;
        let frame_data = shiguredo_svt_av1::FrameData::I420 {
            y: y_plane,
            u: u_plane,
            v: v_plane,
        };
        let options = shiguredo_svt_av1::EncodeOptions {
            force_keyframe: self.keyframe_request_pending,
        };
        self.inner.encode(&frame_data, &options)?;
        self.keyframe_request_pending = false;
        self.input_queue.push_back(frame);
        self.handle_encoded_frames()?;

        Ok(())
    }

    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.finish()?;
        self.handle_encoded_frames()?;
        Ok(())
    }

    pub fn request_keyframe(&mut self) {
        self.keyframe_request_pending = true;
    }

    fn handle_encoded_frames(&mut self) -> crate::Result<()> {
        while let Some(frame) = self.inner.next_frame() {
            // B フレームはない前提なので、タイムスタンプのいれかわりもない
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("encoded frame produced without input frame"))?;

            self.sink.emit_ok(VideoFrame {
                data: frame.data().to_vec(),
                format: VideoFormat::Av1,
                keyframe: frame.is_keyframe(),
                size: Some(VideoFrameSize {
                    width: self.width.get(),
                    height: self.height.get(),
                }),
                timestamp: input_frame.as_video_frame().timestamp,
                sample_entry: Some(self.sample_entry.clone()),
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
    use crate::types::CodecName;
    use crate::video::FrameRate;

    // SvtAv1Encoder::new は options.codec を参照しないため、固定値を使う。
    fn options() -> VideoEncoderOptions {
        VideoEncoderOptions {
            codec: CodecName::Av1,
            engines: None,
            bitrate: 100_000,
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
    // svt_av1 は sample_entry をコンストラクタで確定し全フレームに載せる設計で、
    // `self.sample_entry.take()` のような 1 回消費実装だと 2 フレーム目以降が None になる回帰を検出する。
    // 全フレームに載るのはコンストラクタで確定した同一の sample_entry なので、
    // 実体まで一致することを確認する (is_some だけでは中身の退行を検出できない)。
    fn assert_every_output_frame_has_sample_entry(
        mut encoder: SvtAv1Encoder,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>,
    ) -> crate::Result<()> {
        let expected = encoder.sample_entry.get().clone();

        let mut output_count = 0;
        for i in 0..10 {
            encoder.encode(raw_i420_frame(i * 33))?;
            while let Ok(result) = rx.try_recv() {
                let frame = result?;
                assert_eq!(
                    frame.sample_entry.as_ref().map(|e| e.get()),
                    Some(&expected),
                    "出力フレームに確定済みの sample_entry が載っていない"
                );
                output_count += 1;
            }
        }
        encoder.finish()?;
        while let Ok(result) = rx.try_recv() {
            let frame = result?;
            assert_eq!(
                frame.sample_entry.as_ref().map(|e| e.get()),
                Some(&expected),
                "finish 後の出力フレームに確定済みの sample_entry が載っていない"
            );
            output_count += 1;
        }
        // 全フレーム付与を確認するには 2 フレーム以上の出力が必要。
        assert!(
            output_count >= 2,
            "出力フレーム数が少なすぎる: {output_count}"
        );
        Ok(())
    }

    // svt_av1 は libvpx と同じく feature gate されず常時利用可能なので単体テストで検証する。
    #[test]
    fn svt_av1_sets_sample_entry_on_every_output_frame() -> crate::Result<()> {
        let (sink, rx) = make_encoder_sink();
        let encoder = SvtAv1Encoder::new(&options(), sink)?;
        assert_every_output_frame_has_sample_entry(encoder, rx)
    }
}
