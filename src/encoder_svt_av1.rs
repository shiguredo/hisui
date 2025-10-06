use std::collections::VecDeque;
use std::sync::Arc;

use orfail::OrFail;
use shiguredo_mp4::{
    Uint,
    boxes::{Av01Box, Av1cBox, SampleEntry},
};

use crate::{
    encoder::VideoEncoderOptions,
    types::EvenUsize,
    video::{self, VideoFormat, VideoFrame},
    video_av1,
};

#[derive(Debug)]
pub struct SvtAv1Encoder {
    inner: shiguredo_svt_av1::Encoder,
    input_queue: VecDeque<Arc<VideoFrame>>,
    output_queue: VecDeque<VideoFrame>,
    sample_entry: Option<SampleEntry>,
    width: EvenUsize,
    height: EvenUsize,
}

impl SvtAv1Encoder {
    pub fn new(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let width = options.width;
        let height = options.height;
        let config = shiguredo_svt_av1::EncoderConfig {
            target_bitrate: options.bitrate,
            width: width.get(),
            height: height.get(),
            fps_numerator: options.frame_rate.numerator.get(),
            fps_denominator: options.frame_rate.denumerator.get(),
            ..options.encode_params.svt_av1.clone()
        };
        let inner = shiguredo_svt_av1::Encoder::new(&config).or_fail()?;
        let sample_entry = video_av1::sample_entry(width, height, inner.extra_data());

        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            sample_entry: Some(sample_entry),
            width,
            height,
        })
    }

    pub fn encode(&mut self, frame: Arc<VideoFrame>) -> orfail::Result<()> {
        (frame.format == VideoFormat::I420).or_fail()?;

        let (y_plane, u_plane, v_plane) = frame.as_yuv_planes().or_fail()?;
        self.inner.encode(y_plane, u_plane, v_plane).or_fail()?;
        self.input_queue.push_back(frame);
        self.handle_encoded_frames().or_fail()?;

        Ok(())
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        self.inner.finish().or_fail()?;
        self.handle_encoded_frames().or_fail()?;
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }

    fn handle_encoded_frames(&mut self) -> orfail::Result<()> {
        while let Some(frame) = self.inner.next_frame().or_fail()? {
            // B フレームはない前提なので、タイムスタンプのいれかわりもない
            let input_frame = self.input_queue.pop_front().or_fail()?;

            self.output_queue.push_back(VideoFrame {
                source_id: None,
                data: frame.data().to_vec(),
                format: VideoFormat::Av1,
                keyframe: frame.is_keyframe(),
                width: self.width.get(),
                height: self.height.get(),
                timestamp: input_frame.timestamp,
                duration: input_frame.duration,
                sample_entry: self.sample_entry.take(),
            });
        }
        Ok(())
    }
}
