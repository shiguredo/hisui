use std::sync::Arc;

use orfail::OrFail;

use crate::{
    encoder::VideoEncoderOptions,
    video::{VideoFormat, VideoFrame},
    video_h264::{self, H264_NALU_TYPE_SEI, H264AnnexBNalUnits},
};

#[derive(Debug)]
pub struct Openh264Encoder {
    inner: shiguredo_openh264::Encoder,
    encoded: Option<VideoFrame>,
    is_first: bool,
}

impl Openh264Encoder {
    pub fn new(
        lib: shiguredo_openh264::Openh264Library,
        options: &VideoEncoderOptions,
    ) -> orfail::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        let config = shiguredo_openh264::EncoderConfig {
            fps_numerator: options.frame_rate.numerator.get(),
            fps_denominator: options.frame_rate.denumerator.get(),
            width,
            height,
            target_bitrate: options.bitrate,
            ..options.encode_params.openh264.clone()
        };
        let inner = shiguredo_openh264::Encoder::new(lib, &config).or_fail()?;
        Ok(Self {
            inner,
            encoded: None,
            is_first: true,
        })
    }

    pub fn encode(&mut self, frame: Arc<VideoFrame>) -> orfail::Result<()> {
        (frame.format == VideoFormat::I420).or_fail()?;

        let (y_plane, u_plane, v_plane) = frame.as_yuv_planes().or_fail()?;
        let encoded = self.inner.encode(y_plane, u_plane, v_plane).or_fail()?;
        let Some(encoded) = encoded else {
            return Ok(());
        };

        let sample_entry = if self.is_first {
            self.is_first = false;
            Some(
                video_h264::h264_sample_entry_from_annexb(frame.width, frame.height, &encoded.data)
                    .or_fail()?,
            )
        } else {
            None
        };

        // AnnexB から MP4 向けの形式に変換する
        let mut data = Vec::new();
        for nal in H264AnnexBNalUnits::new(&encoded.data) {
            let nal = nal.or_fail()?;
            if nal.ty == H264_NALU_TYPE_SEI {
                // 一部のタイプは無視する
                continue;
            }

            data.extend_from_slice(&(nal.data.len() as u32).to_be_bytes());
            data.extend_from_slice(nal.data);
        }

        self.encoded = Some(VideoFrame {
            source_id: None,
            data,
            format: VideoFormat::H264,
            keyframe: encoded.keyframe,
            width: frame.width,
            height: frame.height,
            timestamp: frame.timestamp,
            duration: frame.duration,
            sample_entry,
        });

        Ok(())
    }

    // 他のエンコーダーに合わせてメソッドだけ用意しておく
    pub fn finish(&mut self) -> orfail::Result<()> {
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.encoded.take()
    }
}
