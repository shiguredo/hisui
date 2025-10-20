use std::collections::VecDeque;
use std::sync::Arc;

use orfail::OrFail;
use shiguredo_mp4::{
    Uint,
    boxes::{Avc1Box, AvccBox, SampleEntry},
};

use crate::{
    encoder::VideoEncoderOptions,
    types::{CodecName, EvenUsize},
    video::{self, FrameRate, VideoFormat, VideoFrame},
    video_h264::{H264_LEVEL_3_1, H264_PROFILE_BASELINE, NALU_HEADER_LENGTH},
    video_h265,
};

#[derive(Debug)]
pub struct VideoToolboxEncoder {
    inner: shiguredo_video_toolbox::Encoder,
    input_queue: VecDeque<Arc<VideoFrame>>,
    output_queue: VecDeque<VideoFrame>,
    is_first: bool,
    width: EvenUsize,
    height: EvenUsize,
    format: VideoFormat,
    fps: FrameRate,
}

impl VideoToolboxEncoder {
    pub fn new_h264(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let width = options.width;
        let height = options.height;
        let config = shiguredo_video_toolbox::EncoderConfig {
            width: width.get(),
            height: height.get(),
            target_bitrate: options.bitrate,
            fps_numerator: options.frame_rate.numerator.get(),
            fps_denominator: options.frame_rate.denumerator.get(),
            ..options.encode_params.video_toolbox_h264.clone()
        };
        let inner = shiguredo_video_toolbox::Encoder::new_h264(&config).or_fail()?;
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

    pub fn new_h265(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let width = options.width;
        let height = options.height;
        let config = shiguredo_video_toolbox::EncoderConfig {
            width: width.get(),
            height: height.get(),
            target_bitrate: options.bitrate,
            fps_numerator: options.frame_rate.numerator.get(),
            fps_denominator: options.frame_rate.denumerator.get(),
            ..options.encode_params.video_toolbox_h265.clone()
        };
        let inner = shiguredo_video_toolbox::Encoder::new_h265(&config).or_fail()?;
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

    pub fn encode(&mut self, frame: Arc<VideoFrame>) -> orfail::Result<()> {
        (frame.format == VideoFormat::I420).or_fail()?;

        let (y_plane, u_plane, v_plane) = frame.as_yuv_planes().or_fail()?;
        self.inner.encode(y_plane, u_plane, v_plane).or_fail()?;

        // Video Toolbox のエンコーダーは非同期で動作し、
        // エンコードが終わるまでは入力バッファへの参照を保持する必要があるので、
        // バッファもキューに入れておく。
        // (将来的にはこの辺りはエンコーダー内で隠蔽した方が使いやすそう）
        self.input_queue.push_back(frame);

        self.handle_encoded().or_fail()?;

        Ok(())
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        self.inner.finish().or_fail()?;
        self.handle_encoded().or_fail()?;
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }

    fn handle_encoded(&mut self) -> orfail::Result<()> {
        while let Some(frame) = self.inner.next_frame() {
            let input_frame = self.input_queue.pop_front().or_fail()?;
            let sample_entry = if self.is_first {
                self.is_first = false;
                let sample_entry = if self.format == VideoFormat::H264 {
                    h264_sample_entry(
                        self.width,
                        self.height,
                        frame.sps_list.clone(),
                        frame.pps_list.clone(),
                    )
                    .or_fail()?
                } else {
                    video_h265::h265_sample_entry(
                        self.width,
                        self.height,
                        self.fps,
                        frame.vps_list.clone(),
                        frame.sps_list.clone(),
                        frame.pps_list.clone(),
                    )
                    .or_fail()?
                };
                Some(sample_entry)
            } else {
                None
            };

            self.output_queue.push_back(VideoFrame {
                source_id: None,
                data: frame.data,
                format: self.format,
                keyframe: frame.keyframe,
                width: self.width.get(),
                height: self.height.get(),
                timestamp: input_frame.timestamp,
                duration: input_frame.duration,
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
) -> orfail::Result<SampleEntry> {
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
