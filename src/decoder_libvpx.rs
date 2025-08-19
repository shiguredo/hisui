use std::collections::VecDeque;

use orfail::OrFail;

use crate::{
    types::EvenUsize,
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct LibvpxDecoder {
    inner: shiguredo_libvpx::Decoder,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
}

impl LibvpxDecoder {
    pub fn new_vp8() -> orfail::Result<Self> {
        log::debug!("create libvpx(VP8) decoder");
        Ok(Self {
            inner: shiguredo_libvpx::Decoder::new_vp8().or_fail()?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
        })
    }

    pub fn new_vp9() -> orfail::Result<Self> {
        log::debug!("create libvpx(VP9) decoder");
        Ok(Self {
            inner: shiguredo_libvpx::Decoder::new_vp9().or_fail()?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
        })
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> orfail::Result<()> {
        matches!(frame.format, VideoFormat::Vp8 | VideoFormat::Vp9).or_fail()?;

        self.inner.decode(&frame.data).or_fail()?;
        self.input_queue.push_back(frame.to_stripped());
        self.handle_decoded_frames().or_fail()?;
        Ok(())
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        self.inner.finish().or_fail()?;
        self.handle_decoded_frames().or_fail()?;
        Ok(())
    }

    fn handle_decoded_frames(&mut self) -> orfail::Result<()> {
        while let Some(image) = self.inner.next_frame() {
            self.output_queue.push_back(VideoFrame::new_i420(
                self.input_queue.pop_front().or_fail()?,
                EvenUsize::new(image.width()).or_fail()?,
                EvenUsize::new(image.height()).or_fail()?,
                image.y_plane(),
                image.u_plane(),
                image.v_plane(),
                image.y_stride(),
                image.u_stride(),
                image.v_stride(),
            ));
        }
        Ok(())
    }

    pub fn next_decoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }
}
