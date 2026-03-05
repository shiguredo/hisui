use crate::{
    encoder::VideoEncoderOptions,
    video::{RawVideoFrame, VideoFormat, VideoFrame},
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
    ) -> crate::Result<Self> {
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
        let inner = shiguredo_openh264::Encoder::new(lib, config)?;
        Ok(Self {
            inner,
            encoded: None,
            is_first: true,
        })
    }

    pub fn encode(&mut self, frame: RawVideoFrame) -> crate::Result<()> {
        let video_frame = frame.as_video_frame();
        let (y_plane, u_plane, v_plane) = frame.as_i420_planes()?;
        let encoded = self.inner.encode(
            y_plane,
            u_plane,
            v_plane,
            &shiguredo_openh264::EncodeOptions::default(),
        )?;
        let Some(encoded) = encoded else {
            return Ok(());
        };

        let sample_entry = if self.is_first
            && !encoded.sps_list.is_empty()
            && !encoded.pps_list.is_empty()
        {
            let size = frame.size();
            self.is_first = false;
            Some(video_h264::h264_sample_entry_from_annexb(
                size.width,
                size.height,
                &video_h264::create_sequence_header_annexb(&encoded.sps_list, &encoded.pps_list),
            )?)
        } else {
            None
        };

        // AnnexB から MP4 向けの形式に変換する
        let mut data = Vec::new();
        for nal in H264AnnexBNalUnits::new(&encoded.data) {
            let nal = nal?;
            if nal.ty == H264_NALU_TYPE_SEI {
                // 一部のタイプは無視する
                continue;
            }

            data.extend_from_slice(&(nal.data.len() as u32).to_be_bytes());
            data.extend_from_slice(nal.data);
        }

        self.encoded = Some(VideoFrame {
            data,
            format: VideoFormat::H264,
            keyframe: matches!(
                encoded.frame_type,
                shiguredo_openh264::FrameType::Idr | shiguredo_openh264::FrameType::I
            ),
            size: Some(frame.size()),
            timestamp: video_frame.timestamp,
            sample_entry,
        });

        Ok(())
    }

    // 他のエンコーダーに合わせてメソッドだけ用意しておく
    pub fn finish(&mut self) -> crate::Result<()> {
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.encoded.take()
    }
}
