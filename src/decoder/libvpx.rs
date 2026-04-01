use std::collections::VecDeque;

use crate::video::{VideoFormat, VideoFrame};

#[derive(Debug)]
pub struct LibvpxDecoder {
    inner: shiguredo_libvpx::Decoder,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
}

impl LibvpxDecoder {
    pub fn new_vp8() -> crate::Result<Self> {
        tracing::debug!("create libvpx(VP8) decoder");
        Ok(Self {
            inner: shiguredo_libvpx::Decoder::new(shiguredo_libvpx::DecoderConfig::new(
                shiguredo_libvpx::DecoderCodec::Vp8,
            ))?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
        })
    }

    pub fn new_vp9() -> crate::Result<Self> {
        tracing::debug!("create libvpx(VP9) decoder");
        Ok(Self {
            inner: shiguredo_libvpx::Decoder::new(shiguredo_libvpx::DecoderConfig::new(
                shiguredo_libvpx::DecoderCodec::Vp9,
            ))?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
        })
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
        if !matches!(frame.format, VideoFormat::Vp8 | VideoFormat::Vp9) {
            return Err(crate::Error::new(format!(
                "expected VP8 or VP9 format, got {:?}",
                frame.format
            )));
        }

        self.inner.decode(&frame.data)?;
        self.input_queue.push_back(frame.to_stripped());
        self.handle_decoded_frames()?;
        Ok(())
    }

    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.finish()?;
        self.handle_decoded_frames()?;
        Ok(())
    }

    fn handle_decoded_frames(&mut self) -> crate::Result<()> {
        while let Some(image) = self.inner.next_frame()? {
            if image.is_high_depth() {
                // 高ビット深度データの処理
                let input_frame = self.input_queue.pop_front().ok_or_else(|| {
                    crate::Error::new("decoded frame produced without input frame")
                })?;
                self.output_queue
                    .push_back(VideoFrame::new_i420_from_high_depth(
                        input_frame,
                        image.width(),
                        image.height(),
                        image.y_plane(),
                        image.u_plane(),
                        image.v_plane(),
                        image.y_stride(),
                        image.u_stride(),
                        image.v_stride(),
                    )?);
            } else {
                // 通常の 8 ビット I420 データの処理
                let input_frame = self.input_queue.pop_front().ok_or_else(|| {
                    crate::Error::new("decoded frame produced without input frame")
                })?;
                self.output_queue.push_back(VideoFrame::new_i420(
                    input_frame,
                    image.width(),
                    image.height(),
                    image.y_plane(),
                    image.u_plane(),
                    image.v_plane(),
                    image.y_stride(),
                    image.u_stride(),
                    image.v_stride(),
                ));
            }
        }
        Ok(())
    }

    pub fn next_decoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }
}
