use std::collections::VecDeque;

use shiguredo_mp4::boxes::{Avc1Box, AvccBox, SampleEntry};
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
            return Err(crate::Error::new(format!(
                "expected H264 or H264AnnexB format, got {:?}",
                frame.format
            )));
        }

        if frame.keyframe {
            // SPS / PPS などが変わると、デコーダーのバッファ内のフレームが失われることがあるようなので、
            // 変更の可能性があるキーフレームを処理する前に、常に finish() を呼ぶようにしている。
            // （よりちゃんとやるなら、frame.data をパースして SPS / PPS の変更をチェックするようにするといい）
            self.finish()?;
        }

        let decoded = if matches!(frame.format, VideoFormat::H264) {
            self.inner.decode(&build_annexb_input(frame)?)?
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
            .ok_or_else(|| crate::Error::new("decoded frame produced without input frame"))?;
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
            .ok_or_else(|| crate::Error::new("decoded frame produced without input frame"))?;
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

fn build_annexb_input(frame: &VideoFrame) -> crate::Result<Vec<u8>> {
    let mut data = &frame.data[..];
    let mut payload_annexb = Vec::new();
    let mut has_sps = false;
    let mut has_pps = false;
    while !data.is_empty() {
        if data.len() <= 3 {
            return Err(crate::Error::new(format!(
                "invalid H264 AVCC payload: NALU length header is truncated (remaining={})",
                data.len()
            )));
        }
        let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        data = &data[4..];

        if data.len() < n {
            return Err(crate::Error::new(format!(
                "invalid H264 AVCC payload: NALU data is truncated (required={n}, remaining={})",
                data.len()
            )));
        }
        let nalu = &data[..n];
        if let Some(header) = nalu.first() {
            match header & 0b0001_1111 {
                crate::video_h264::H264_NALU_TYPE_SPS => has_sps = true,
                crate::video_h264::H264_NALU_TYPE_PPS => has_pps = true,
                _ => {}
            }
        }
        payload_annexb.extend_from_slice(&[0, 0, 0, 1]);
        payload_annexb.extend_from_slice(nalu);

        data = &data[n..];
    }

    if has_sps && has_pps {
        return Ok(payload_annexb);
    }

    let Some(SampleEntry::Avc1(Avc1Box {
        avcc_box: AvccBox {
            sps_list, pps_list, ..
        },
        ..
    })) = frame.sample_entry.as_ref()
    else {
        return Ok(payload_annexb);
    };

    let mut annexb = Vec::new();
    if !has_sps {
        for sps in sps_list {
            annexb.extend_from_slice(&[0, 0, 0, 1]);
            annexb.extend_from_slice(sps);
        }
    }
    if !has_pps {
        for pps in pps_list {
            annexb.extend_from_slice(&[0, 0, 0, 1]);
            annexb.extend_from_slice(pps);
        }
    }
    annexb.extend_from_slice(&payload_annexb);
    Ok(annexb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_annexb_input_prepends_missing_sps_pps_from_sample_entry() -> crate::Result<()> {
        let sample_entry = crate::video_h264::h264_sample_entry_from_annexb(
            320,
            320,
            &[
                0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f, 0xe5, 0x88, 0x68, 0x54, 0, 0, 0, 1, 0x68, 0xce,
                0x06, 0xe2,
            ],
        )?;
        let frame = VideoFrame {
            data: vec![0, 0, 0, 2, 0x65, 0x88],
            format: VideoFormat::H264,
            keyframe: true,
            size: None,
            timestamp: std::time::Duration::ZERO,
            sample_entry: Some(sample_entry),
        };

        let annexb = build_annexb_input(&frame)?;
        let nalus = crate::video_h264::H264AnnexBNalUnits::new(&annexb)
            .collect::<crate::Result<Vec<_>>>()?;
        let nalu_types = nalus.iter().map(|nalu| nalu.ty).collect::<Vec<_>>();
        assert_eq!(
            nalu_types,
            vec![
                crate::video_h264::H264_NALU_TYPE_SPS,
                crate::video_h264::H264_NALU_TYPE_PPS,
                crate::video_h264::H264_NALU_TYPE_IDR,
            ]
        );
        Ok(())
    }

    #[test]
    fn build_annexb_input_keeps_existing_sps_pps() -> crate::Result<()> {
        let sample_entry = crate::video_h264::h264_sample_entry_from_annexb(
            320,
            320,
            &[
                0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f, 0xe5, 0x88, 0x68, 0x54, 0, 0, 0, 1, 0x68, 0xce,
                0x06, 0xe2,
            ],
        )?;
        let frame = VideoFrame {
            data: vec![
                0, 0, 0, 8, 0x67, 0x42, 0x00, 0x1f, 0xe5, 0x88, 0x68, 0x54, 0, 0, 0, 4, 0x68, 0xce,
                0x06, 0xe2, 0, 0, 0, 2, 0x65, 0x88,
            ],
            format: VideoFormat::H264,
            keyframe: true,
            size: None,
            timestamp: std::time::Duration::ZERO,
            sample_entry: Some(sample_entry),
        };

        let annexb = build_annexb_input(&frame)?;
        let nalus = crate::video_h264::H264AnnexBNalUnits::new(&annexb)
            .collect::<crate::Result<Vec<_>>>()?;
        let nalu_types = nalus.iter().map(|nalu| nalu.ty).collect::<Vec<_>>();
        assert_eq!(
            nalu_types,
            vec![
                crate::video_h264::H264_NALU_TYPE_SPS,
                crate::video_h264::H264_NALU_TYPE_PPS,
                crate::video_h264::H264_NALU_TYPE_IDR,
            ]
        );
        Ok(())
    }
}
