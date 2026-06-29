use std::collections::VecDeque;

use super::OutputSink;
use crate::video::{VideoFormat, VideoFrame};

#[derive(Debug)]
pub struct Dav1dDecoder {
    inner: shiguredo_dav1d::Decoder,
    input_queue: VecDeque<VideoFrame>,
    sink: OutputSink,
}

impl Dav1dDecoder {
    pub fn new(sink: OutputSink) -> crate::Result<Self> {
        Ok(Self {
            inner: shiguredo_dav1d::Decoder::new(shiguredo_dav1d::DecoderConfig::default())?,
            input_queue: VecDeque::new(),
            sink,
        })
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
        if frame.format != VideoFormat::Av1 {
            return Err(crate::Error::new(format!(
                "expected AV1 format, got {:?}",
                frame.format
            )));
        }

        self.inner.decode(&frame.data)?;
        self.input_queue.push_back(frame.to_stripped());
        self.emit_decoded_frames()?;
        Ok(())
    }

    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.finish()?;
        self.emit_decoded_frames()?;
        Ok(())
    }

    fn emit_decoded_frames(&mut self) -> crate::Result<()> {
        while let Some(decoded) = self.inner.next_frame()? {
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("decoded frame produced without input frame"))?;
            self.sink.emit_ok(VideoFrame::new_i420(
                input_frame,
                decoded.width(),
                decoded.height(),
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
}
