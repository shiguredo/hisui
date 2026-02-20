use std::collections::VecDeque;

use shiguredo_openh264::Openh264Library;

use crate::video::{VideoFormat, VideoFrame};

#[derive(Debug)]
pub struct Openh264Decoder {
    inner: shiguredo_openh264::Decoder,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
}

impl Openh264Decoder {
    pub fn new(lib: Openh264Library) -> crate::Result<Self> {
        Ok(Self {
            inner: shiguredo_openh264::Decoder::new(lib)?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
        })
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
        if !matches!(frame.format, VideoFormat::H264 | VideoFormat::H264AnnexB) {
            return Err(crate::Error::new("condition is false"));
        }

        if frame.keyframe {
            // SPS / PPS などが変わると、デコーダーのバッファ内のフレームが失われることがあるようなので、
            // 変更の可能性があるキーフレームを処理する前に、常に finish() を呼ぶようにしている。
            // （よりちゃんとやるなら、frame.data をパースして SPS / PPS の変更をチェックするようにするといい）
            self.finish()?;
        }

        let decoded = if matches!(frame.format, VideoFormat::H264) {
            // Annex.B 形式に変換する
            let mut data = &frame.data[..];
            let mut data_annexb = Vec::new();
            while !data.is_empty() {
                if data.len() <= 3 {
                    return Err(crate::Error::new("condition is false"));
                }
                let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
                data = &data[4..];

                if data.len() < n {
                    return Err(crate::Error::new("condition is false"));
                }
                data_annexb.extend_from_slice(&[0, 0, 0, 1]);
                data_annexb.extend_from_slice(&data[..n]);

                data = &data[n..];
            }

            self.inner.decode(&data_annexb)?
        } else {
            self.inner.decode(&frame.data)?
        };
        self.input_queue.push_back(frame.to_stripped());

        let Some(decoded) = decoded else {
            // まだデコーダーのバッファ内にある
            return Ok(());
        };

        let input_frame = self
            .input_queue
            .pop_front()
            .ok_or_else(|| crate::Error::new("value is missing"))?;
        let output_frame = Self::to_rgb_frame(input_frame, decoded)?;
        self.output_queue.push_back(output_frame);
        Ok(())
    }

    pub fn finish(&mut self) -> crate::Result<()> {
        let Some(decoded) = self.inner.finish()? else {
            return Ok(());
        };
        let input_frame = self
            .input_queue
            .pop_front()
            .ok_or_else(|| crate::Error::new("value is missing"))?;
        let output_frame = Self::to_rgb_frame(input_frame, decoded)?;
        self.output_queue.push_back(output_frame);
        Ok(())
    }

    fn to_rgb_frame(
        input_frame: VideoFrame,
        frame: shiguredo_openh264::DecodedFrame,
    ) -> crate::Result<VideoFrame> {
        Ok(VideoFrame::new_i420(
            input_frame,
            frame.width(),
            frame.height(),
            frame.y_plane(),
            frame.u_plane(),
            frame.v_plane(),
            frame.y_stride(),
            frame.u_stride(),
            frame.v_stride(),
        ))
    }

    pub fn next_decoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }
}
