use crate::{
    encoder::VideoEncoderOptions,
    video::{RawVideoFrame, VideoFormat, VideoFrame},
    video_h264::{self, H264_NALU_TYPE_SEI, H264AnnexBNalUnits},
};

#[derive(Debug)]
pub struct Openh264Encoder {
    inner: shiguredo_openh264::Encoder,
    encoded: Option<VideoFrame>,
    force_idr_pending: bool,
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
            force_idr_pending: false,
        })
    }

    pub fn encode(&mut self, frame: RawVideoFrame) -> crate::Result<()> {
        let video_frame = frame.as_video_frame();
        let (y_plane, u_plane, v_plane) = frame.as_i420_planes()?;
        let encode_options = shiguredo_openh264::EncodeOptions {
            force_idr: self.force_idr_pending,
        };
        let encoded = self
            .inner
            .encode(y_plane, u_plane, v_plane, &encode_options)?;
        let Some(encoded) = encoded else {
            return Ok(());
        };

        // OpenH264 は keyframe 要求時などに SPS/PPS が更新され得るため、
        // SPS/PPS を受け取ったフレームでは毎回 sample entry を更新する。
        // これにより、下流コンポーネントが参照する codec 設定を最新化し、
        // 古い parameter set 参照によるデコード失敗を避ける。
        let sample_entry = if !encoded.sps_list.is_empty() && !encoded.pps_list.is_empty() {
            let size = frame.size();
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

        let is_keyframe = matches!(
            encoded.frame_type,
            shiguredo_openh264::FrameType::Idr | shiguredo_openh264::FrameType::I
        );
        if self.force_idr_pending && is_keyframe {
            self.force_idr_pending = false;
        }

        self.encoded = Some(VideoFrame {
            data,
            format: VideoFormat::H264,
            keyframe: is_keyframe,
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

    pub fn request_keyframe(&mut self) {
        self.force_idr_pending = true;
    }
}
