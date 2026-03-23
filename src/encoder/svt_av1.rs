use std::collections::VecDeque;

use shiguredo_mp4::boxes::SampleEntry;

use crate::{
    encoder::VideoEncoderOptions,
    types::EvenUsize,
    video::av1,
    video::{RawVideoFrame, VideoFormat, VideoFrame, VideoFrameSize},
};

#[derive(Debug)]
pub struct SvtAv1Encoder {
    inner: shiguredo_svt_av1::Encoder,
    input_queue: VecDeque<RawVideoFrame>,
    output_queue: VecDeque<VideoFrame>,
    sample_entry: Option<SampleEntry>,
    width: EvenUsize,
    height: EvenUsize,
}

impl SvtAv1Encoder {
    pub fn new(options: &VideoEncoderOptions) -> crate::Result<Self> {
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
            output_queue: VecDeque::new(),
            sample_entry: Some(sample_entry),
            width,
            height,
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
            force_keyframe: false,
        };
        self.inner.encode(&frame_data, &options)?;
        self.input_queue.push_back(frame);
        self.handle_encoded_frames()?;

        Ok(())
    }

    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.finish()?;
        self.handle_encoded_frames()?;
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }

    fn handle_encoded_frames(&mut self) -> crate::Result<()> {
        while let Some(frame) = self.inner.next_frame() {
            // B フレームはない前提なので、タイムスタンプのいれかわりもない
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("encoded frame produced without input frame"))?;

            self.output_queue.push_back(VideoFrame {
                data: frame.data().to_vec(),
                format: VideoFormat::Av1,
                keyframe: frame.is_keyframe(),
                size: Some(VideoFrameSize {
                    width: self.width.get(),
                    height: self.height.get(),
                }),
                timestamp: input_frame.as_video_frame().timestamp,
                sample_entry: self.sample_entry.take(),
            });
        }
        Ok(())
    }
}
