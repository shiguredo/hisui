use std::collections::VecDeque;

use shiguredo_mp4::boxes::{Avc1Box, AvccBox, SampleEntry};
use shiguredo_openh264::Openh264Library;
use tokio::sync::mpsc;

use crate::video::{VideoFormat, VideoFrame};

/// Sender 化された OpenH264 デコーダー
///
/// keyframe 入力時には先に `finish()` を呼んで旧 SPS/PPS バッファをフラッシュしてから
/// 新 keyframe を decode するため、1 回の `decode()` 呼出で 0〜2 フレームを Sender に送信する
/// 可能性がある (旧 finish 由来 0〜1 + 新 decode 由来 0〜1)。
#[derive(Debug)]
pub struct Openh264Decoder {
    inner: shiguredo_openh264::Decoder,
    // デコード済みフレームと元入力フレームを対応付けるための保持列
    input_queue: VecDeque<VideoFrame>,
    // デコード結果を上位 run() の Receiver に流す Sender
    tx: mpsc::Sender<crate::Result<VideoFrame>>,
}

impl Openh264Decoder {
    pub fn new(
        lib: Openh264Library,
        tx: mpsc::Sender<crate::Result<VideoFrame>>,
    ) -> crate::Result<Self> {
        Ok(Self {
            inner: shiguredo_openh264::Decoder::new(lib)?,
            input_queue: VecDeque::new(),
            tx,
        })
    }

    pub async fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
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
            // finish() 経由の旧フレームも先に Sender に送信されるため、本 keyframe 処理を含めて
            // 1 回の decode() で 0〜2 フレーム送信される。
            self.finish().await?;
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
        self.tx
            .send(Ok(output_frame))
            .await
            .map_err(|_| crate::Error::new("decoded frame channel closed"))?;
        Ok(())
    }

    pub async fn finish(&mut self) -> crate::Result<()> {
        let Some(decoded) = self.inner.finish()? else {
            return Ok(());
        };
        let input_frame = self
            .input_queue
            .pop_front()
            .ok_or_else(|| crate::Error::new("decoded frame produced without input frame"))?;
        let output_frame = Self::to_rgb_frame(input_frame, decoded)?;
        self.tx
            .send(Ok(output_frame))
            .await
            .map_err(|_| crate::Error::new("decoded frame channel closed"))?;
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
                crate::video::h264::H264_NALU_TYPE_SPS => has_sps = true,
                crate::video::h264::H264_NALU_TYPE_PPS => has_pps = true,
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
    })) = frame.sample_entry.as_ref().map(|e| e.get())
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

    // PPS NAL を Annex-B 形式 (先頭 4 バイト start code + NAL バイト列) で表現したフィクスチャ
    const PPS_ANNEXB: &[u8] = &[0, 0, 0, 1, 0x68, 0xce, 0x06, 0xe2];

    #[test]
    fn build_annexb_input_prepends_missing_sps_pps_from_sample_entry() -> crate::Result<()> {
        // SPS は `crate::video::h264::tests::SPS_320X240` (24 バイト実 SPS、Baseline 320x240) を
        // Annex-B 形式に展開して使う。偽 SPS では parse_sps が完走しないため、実 SPS に差し替えている。
        let annexb: Vec<u8> = [
            &crate::video::h264::tests::SPS_320X240_ANNEXB[..],
            PPS_ANNEXB,
        ]
        .concat();
        let sample_entry = crate::video::h264::h264_sample_entry_from_annexb(&annexb)?;
        let frame = VideoFrame {
            data: vec![0, 0, 0, 2, 0x65, 0x88],
            format: VideoFormat::H264,
            keyframe: true,
            size: None,
            timestamp: std::time::Duration::ZERO,
            sample_entry: Some(crate::sample_entry::SharedSampleEntry::new(sample_entry)),
        };

        let annexb = build_annexb_input(&frame)?;
        let nalus = crate::video::h264::H264AnnexBNalUnits::new(&annexb)
            .collect::<crate::Result<Vec<_>>>()?;
        let nalu_types = nalus.iter().map(|nalu| nalu.ty).collect::<Vec<_>>();
        assert_eq!(
            nalu_types,
            vec![
                crate::video::h264::H264_NALU_TYPE_SPS,
                crate::video::h264::H264_NALU_TYPE_PPS,
                crate::video::h264::H264_NALU_TYPE_IDR,
            ]
        );
        Ok(())
    }

    #[test]
    fn build_annexb_input_keeps_existing_sps_pps() -> crate::Result<()> {
        // SPS は `crate::video::h264::tests::SPS_320X240` (24 バイト実 SPS、Baseline 320x240) を
        // Annex-B 形式に展開して sample_entry 構築に使う。
        let annexb: Vec<u8> = [
            &crate::video::h264::tests::SPS_320X240_ANNEXB[..],
            PPS_ANNEXB,
        ]
        .concat();
        let sample_entry = crate::video::h264::h264_sample_entry_from_annexb(&annexb)?;
        // AVCC 形式 frame.data を [SPS 長 prefix, SPS, PPS 長 prefix, PPS, IDR 長 prefix, IDR] で構築する。
        // SPS は SPS_320X240 の 24 バイト、PPS は 4 バイト、IDR は 2 バイト固定。
        let pps_nal = &PPS_ANNEXB[4..];
        let idr_nal: &[u8] = &[0x65, 0x88];
        let frame_data: Vec<u8> = [
            &(crate::video::h264::tests::SPS_320X240.len() as u32).to_be_bytes()[..],
            &crate::video::h264::tests::SPS_320X240[..],
            &(pps_nal.len() as u32).to_be_bytes()[..],
            pps_nal,
            &(idr_nal.len() as u32).to_be_bytes()[..],
            idr_nal,
        ]
        .concat();
        let frame = VideoFrame {
            data: frame_data,
            format: VideoFormat::H264,
            keyframe: true,
            size: None,
            timestamp: std::time::Duration::ZERO,
            sample_entry: Some(crate::sample_entry::SharedSampleEntry::new(sample_entry)),
        };

        let annexb = build_annexb_input(&frame)?;
        let nalus = crate::video::h264::H264AnnexBNalUnits::new(&annexb)
            .collect::<crate::Result<Vec<_>>>()?;
        let nalu_types = nalus.iter().map(|nalu| nalu.ty).collect::<Vec<_>>();
        assert_eq!(
            nalu_types,
            vec![
                crate::video::h264::H264_NALU_TYPE_SPS,
                crate::video::h264::H264_NALU_TYPE_PPS,
                crate::video::h264::H264_NALU_TYPE_IDR,
            ]
        );
        Ok(())
    }
}
