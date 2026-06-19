use std::collections::VecDeque;

use crate::{
    encoder::VideoEncoderOptions,
    sample_entry::SharedSampleEntry,
    types::CodecName,
    video::{
        RawVideoFrame, VideoFormat, VideoFrame, VideoFrameSize,
        vpx::{vp8_sample_entry, vp9_sample_entry},
    },
};

// エンコードパラメーターのデフォルト値
pub const DEFAULT_CQ_LEVEL: &str = "30";
pub const DEFAULT_MIN_Q: &str = "10";
pub const DEFAULT_MAX_Q: &str = "50";

#[derive(Debug)]
pub struct LibvpxEncoder {
    inner: shiguredo_libvpx::Encoder,
    format: VideoFormat,
    // 全出力フレームに載せるサンプルエントリー。Arc 共有なので毎フレームの clone は安価。
    sample_entry: SharedSampleEntry,
    keyframe_request_pending: bool,
    input_queue: VecDeque<RawVideoFrame>,
    output_queue: VecDeque<VideoFrame>,
}

impl LibvpxEncoder {
    pub fn new_vp8(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        let config = shiguredo_libvpx::EncoderConfig {
            width,
            height,
            fps_numerator: options.frame_rate.numerator.get(),
            fps_denominator: options.frame_rate.denumerator.get(),
            target_bitrate: options.bitrate,
            ..options.encode_params.libvpx_vp8.clone()
        };
        tracing::debug!("libvpx vp8 encoder config: {config:?}");
        let inner = shiguredo_libvpx::Encoder::new(config)?;
        let sample_entry = vp8_sample_entry(width, height);

        Ok(Self {
            inner,
            format: VideoFormat::Vp8,
            sample_entry: SharedSampleEntry::new(sample_entry),
            keyframe_request_pending: false,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
        })
    }

    pub fn new_vp9(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        let config = shiguredo_libvpx::EncoderConfig {
            width,
            height,
            fps_numerator: options.frame_rate.numerator.get(),
            fps_denominator: options.frame_rate.denumerator.get(),
            target_bitrate: options.bitrate,
            ..options.encode_params.libvpx_vp9.clone()
        };
        tracing::debug!("libvpx vp9 encoder config: {config:?}");
        let inner = shiguredo_libvpx::Encoder::new(config)?;
        let sample_entry = vp9_sample_entry(width, height);

        Ok(Self {
            inner,
            format: VideoFormat::Vp9,
            sample_entry: SharedSampleEntry::new(sample_entry),
            keyframe_request_pending: false,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
        })
    }

    pub fn codec(&self) -> CodecName {
        if self.format == VideoFormat::Vp8 {
            CodecName::Vp8
        } else {
            CodecName::Vp9
        }
    }

    pub fn encode(&mut self, frame: RawVideoFrame) -> crate::Result<()> {
        let (y_plane, u_plane, v_plane) = frame.as_i420_planes()?;
        let encode_options = shiguredo_libvpx::EncodeOptions {
            force_keyframe: self.keyframe_request_pending,
        };
        self.inner.encode(
            &shiguredo_libvpx::ImageData::I420 {
                y: y_plane,
                u: u_plane,
                v: v_plane,
            },
            &encode_options,
        )?;
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

    fn handle_encoded_frames(&mut self) -> crate::Result<()> {
        while let Some(frame) = self.inner.next_frame() {
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("encoded frame produced without input frame"))?;
            self.output_queue.push_back(VideoFrame {
                sample_entry: Some(self.sample_entry.clone()),
                data: frame.data().to_vec(),
                format: self.format,
                keyframe: frame.is_keyframe(),
                size: Some(VideoFrameSize {
                    width: frame.width() as usize,
                    height: frame.height() as usize,
                }),
                timestamp: input_frame.as_video_frame().timestamp,
            });
        }

        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }

    pub fn request_keyframe(&mut self) {
        self.keyframe_request_pending = true;
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::sync::Arc;

    use super::*;
    use crate::types::EvenUsize;
    use crate::video::FrameRate;

    // new_vp8 / new_vp9 は options.codec を参照しないため、固定値を使う。
    fn options() -> VideoEncoderOptions {
        VideoEncoderOptions {
            codec: CodecName::Vp8,
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

    // 64x64 の I420 グレーフレームを作る。
    fn raw_i420_frame(ts_ms: u64) -> RawVideoFrame {
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
            timestamp: std::time::Duration::from_millis(ts_ms),
            sample_entry: None,
        };
        RawVideoFrame::from_i420_video_frame(Arc::new(frame)).expect("有効な I420 フレーム")
    }

    // 全出力フレームに sample_entry が載る不変条件を検証する（issue 0027 の核心）。
    // 旧実装（self.sample_entry.take()）では 2 フレーム目以降が None になっていた。
    fn assert_every_output_frame_has_sample_entry(mut encoder: LibvpxEncoder) -> crate::Result<()> {
        let mut output_count = 0;
        for i in 0..10 {
            encoder.encode(raw_i420_frame(i * 33))?;
            while let Some(frame) = encoder.next_encoded_frame() {
                assert!(
                    frame.sample_entry.is_some(),
                    "出力フレームに sample_entry が載っていない"
                );
                output_count += 1;
            }
        }
        encoder.finish()?;
        while let Some(frame) = encoder.next_encoded_frame() {
            assert!(
                frame.sample_entry.is_some(),
                "finish 後の出力フレームに sample_entry が載っていない"
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

    #[test]
    fn libvpx_vp8_sets_sample_entry_on_every_output_frame() -> crate::Result<()> {
        assert_every_output_frame_has_sample_entry(LibvpxEncoder::new_vp8(&options())?)
    }

    #[test]
    fn libvpx_vp9_sets_sample_entry_on_every_output_frame() -> crate::Result<()> {
        assert_every_output_frame_has_sample_entry(LibvpxEncoder::new_vp9(&options())?)
    }
}
