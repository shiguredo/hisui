use std::collections::VecDeque;

use shiguredo_mp4::{
    Uint,
    boxes::{Avc1Box, AvccBox, SampleEntry},
};

use crate::{
    encoder::VideoEncoderOptions,
    types::{CodecName, EvenUsize},
    video::{self, FrameRate, RawVideoFrame, VideoFormat, VideoFrame, VideoFrameSize},
    video_h264::{H264_LEVEL_3_1, H264_PROFILE_BASELINE, NALU_HEADER_LENGTH},
    video_h265,
};

#[derive(Debug)]
pub struct VideoToolboxEncoder {
    inner: shiguredo_video_toolbox::Encoder,
    input_queue: VecDeque<RawVideoFrame>,
    output_queue: VecDeque<VideoFrame>,
    is_first: bool,
    width: EvenUsize,
    height: EvenUsize,
    format: VideoFormat,
    fps: FrameRate,
}

impl VideoToolboxEncoder {
    pub fn new_h264(options: &VideoEncoderOptions) -> crate::Result<Self> {
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
        let inner = shiguredo_video_toolbox::Encoder::new(config)?;
        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            is_first: true,
            width,
            height,
            format: VideoFormat::H264,
            fps: options.frame_rate,
        })
    }

    pub fn new_h265(options: &VideoEncoderOptions) -> crate::Result<Self> {
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
        let inner = shiguredo_video_toolbox::Encoder::new(config)?;
        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            is_first: true,
            width,
            height,
            format: VideoFormat::H265,
            fps: options.frame_rate,
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
            &shiguredo_video_toolbox::EncodeOptions::default(),
        )?;

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

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }

    fn handle_encoded(&mut self) -> crate::Result<()> {
        while let Some(frame) = self.inner.next_frame() {
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("encoded frame produced without input frame"))?;
            let sample_entry = if self.is_first {
                self.is_first = false;
                let sample_entry = if self.format == VideoFormat::H264 {
                    h264_sample_entry(
                        self.width,
                        self.height,
                        frame.sps_list.clone(),
                        frame.pps_list.clone(),
                    )?
                } else {
                    video_h265::h265_sample_entry(
                        self.width,
                        self.height,
                        self.fps,
                        frame.vps_list.clone(),
                        frame.sps_list.clone(),
                        frame.pps_list.clone(),
                    )?
                };
                Some(sample_entry)
            } else {
                None
            };

            self.output_queue.push_back(VideoFrame {
                data: frame.data,
                format: self.format,
                keyframe: frame.keyframe,
                size: Some(VideoFrameSize {
                    width: self.width.get(),
                    height: self.height.get(),
                }),
                timestamp: input_frame.as_video_frame().timestamp,
                sample_entry,
            });
        }
        Ok(())
    }
}

fn h264_sample_entry(
    width: EvenUsize,
    height: EvenUsize,
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
) -> crate::Result<SampleEntry> {
    Ok(SampleEntry::Avc1(Avc1Box {
        visual: video::sample_entry_visual_fields(width.get(), height.get()),
        avcc_box: AvccBox {
            // 実際のエンコードストリームに合わせた値
            sps_list,
            pps_list,

            // 以下は Hisui では固定値
            avc_profile_indication: H264_PROFILE_BASELINE,
            avc_level_indication: H264_LEVEL_3_1,
            profile_compatibility: 0, // いったん 0 を指定しているが、もし支障があれば調整する
            length_size_minus_one: Uint::new(NALU_HEADER_LENGTH as u8 - 1),
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext_list: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    }))
}
