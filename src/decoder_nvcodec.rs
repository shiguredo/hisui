use std::collections::VecDeque;

use orfail::OrFail;

use crate::video::{VideoFormat, VideoFrame};
use crate::video_h265::{
    H265_NALU_TYPE_PPS, H265_NALU_TYPE_SPS, H265_NALU_TYPE_VPS, NALU_HEADER_LENGTH,
};

#[derive(Debug)]
pub struct NvcodecDecoder {
    inner: shiguredo_nvcodec::Decoder,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
    parameter_sets: Option<Vec<u8>>, // VPS/SPS/PPS をキャッシュ
}

impl NvcodecDecoder {
    pub fn new_h265() -> orfail::Result<Self> {
        log::debug!("create nvcodec(H265) decoder");
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new_h265().or_fail()?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
        })
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> orfail::Result<()> {
        (frame.format == VideoFormat::H265).or_fail()?;

        // サンプルエントリから VPS/SPS/PPS を抽出してキャッシュ
        if self.parameter_sets.is_none() {
            if let Some(sample_entry) = &frame.sample_entry {
                self.parameter_sets = Some(extract_parameter_sets_annexb(sample_entry).or_fail()?);
            }
        }

        // Annex.B 形式に変換する
        let mut data = &frame.data[..];
        let mut data_annexb = Vec::new();

        // キーフレームで、かつパラメータセットがデータに含まれていない場合は先頭に追加
        if frame.keyframe {
            if let Some(parameter_sets) = &self.parameter_sets {
                if !contains_parameter_sets(data) {
                    data_annexb.extend_from_slice(parameter_sets);
                }
            }
        }

        while !data.is_empty() {
            (data.len() > 3).or_fail()?;
            let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
            data = &data[4..];

            (data.len() >= n).or_fail()?;
            data_annexb.extend_from_slice(&[0, 0, 0, 1]);
            data_annexb.extend_from_slice(&data[..n]);

            data = &data[n..];
        }

        self.inner.decode(&data_annexb).or_fail()?;
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
        while let Some(nv12_frame) = self.inner.next_frame().or_fail()? {
            let input_frame = self.input_queue.pop_front().or_fail()?;

            // NV12 から I420 への変換
            let width = nv12_frame.width();
            let height = nv12_frame.height();

            // I420 用のバッファを確保
            let y_size = width * height;
            let uv_width = width.div_ceil(2);
            let uv_height = height.div_ceil(2);
            let uv_size = uv_width * uv_height;
            let total_size = y_size + uv_size * 2;

            let mut i420_data = vec![0u8; total_size];
            let (y_plane, rest) = i420_data.split_at_mut(y_size);
            let (u_plane, v_plane) = rest.split_at_mut(uv_size);

            // libyuv を使って NV12 から I420 に変換
            let src = shiguredo_libyuv::Nv12Planes {
                y: nv12_frame.y_plane(),
                y_stride: nv12_frame.y_stride(),
                uv: nv12_frame.uv_plane(),
                uv_stride: nv12_frame.uv_stride(),
            };

            let mut dst = shiguredo_libyuv::I420PlanesMut {
                y: y_plane,
                y_stride: width,
                u: u_plane,
                u_stride: uv_width,
                v: v_plane,
                v_stride: uv_width,
            };

            let size = shiguredo_libyuv::ImageSize::new(width, height);
            shiguredo_libyuv::nv12_to_i420(&src, &mut dst, size).or_fail()?;

            // I420 VideoFrame を作成
            self.output_queue.push_back(VideoFrame::new_i420(
                input_frame,
                width,
                height,
                y_plane,
                u_plane,
                v_plane,
                width,
                uv_width,
                uv_width,
            ));
        }
        Ok(())
    }

    pub fn next_decoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }
}

/// サンプルエントリから VPS/SPS/PPS を Annex.B 形式で抽出
fn extract_parameter_sets_annexb(
    sample_entry: &shiguredo_mp4::boxes::SampleEntry,
) -> orfail::Result<Vec<u8>> {
    use shiguredo_mp4::boxes::SampleEntry;

    let hevc_config = match sample_entry {
        SampleEntry::Hev1(entry) => &entry.hvcc_box,
        _ => return Err(orfail::Failure::new("Sample entry is not HEVC (hev1/hvc1)")),
    };

    let mut annexb_data = Vec::new();

    // 各 NAL unit array からパラメータセットを抽出
    for array in &hevc_config.nalu_arrays {
        for nalu in &array.nalus {
            // Annex.B start code を追加
            annexb_data.extend_from_slice(&[0, 0, 0, 1]);
            annexb_data.extend_from_slice(nalu);
        }
    }

    Ok(annexb_data)
}

/// データの先頭にパラメータセット（VPS/SPS/PPS）が含まれているかチェック
fn contains_parameter_sets(data: &[u8]) -> bool {
    if data.len() < NALU_HEADER_LENGTH + 1 {
        return false;
    }

    // 最初の NAL unit の type をチェック
    // H.265 の NAL unit type は 2バイト目の上位6ビット
    let nal_unit_type = (data[NALU_HEADER_LENGTH] >> 1) & 0x3F;
    matches!(
        nal_unit_type,
        H265_NALU_TYPE_PPS | H265_NALU_TYPE_SPS | H265_NALU_TYPE_VPS
    )
}
