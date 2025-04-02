use std::collections::VecDeque;

use orfail::OrFail;

use crate::{
    types::EvenUsize,
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct Dav1dDecoder {
    inner: shiguredo_dav1d::Decoder,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
}

impl Dav1dDecoder {
    pub fn new() -> orfail::Result<Self> {
        Ok(Self {
            inner: shiguredo_dav1d::Decoder::new().or_fail()?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
        })
    }

    pub fn decode(&mut self, frame: VideoFrame) -> orfail::Result<()> {
        (frame.format == VideoFormat::Av1).or_fail()?;

        self.inner.decode(&frame.data).or_fail()?;
        self.input_queue.push_back(frame);
        self.handle_decoded_frames().or_fail()?;
        Ok(())
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        self.inner.finish().or_fail()?;
        self.handle_decoded_frames().or_fail()?;
        Ok(())
    }

    fn handle_decoded_frames(&mut self) -> orfail::Result<()> {
        while let Some(decoded) = self.inner.next_frame().or_fail()? {
            self.output_queue.push_back(VideoFrame::new_i420(
                self.input_queue.pop_front().or_fail()?,
                EvenUsize::new(decoded.width()).or_fail()?,
                EvenUsize::new(decoded.height()).or_fail()?,
                decoded.y_plane(),
                decoded.u_plane(),
                decoded.v_plane(),
                decoded.y_stride(),
                decoded.u_stride(),
                decoded.v_stride(),
            ));
        }
        Ok(())
    }

    pub fn next_decoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }
}
