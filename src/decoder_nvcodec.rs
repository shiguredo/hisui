use std::borrow::Cow;
use std::collections::VecDeque;

use crate::decoder::DecodeConfig;
use crate::video::{VideoFormat, VideoFrame};
use crate::video_h264::{H264_NALU_TYPE_PPS, H264_NALU_TYPE_SPS};
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
    pub fn new_h264(params: &DecodeConfig) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(H264) decoder");
        let mut config = params.nvcodec_h264.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::H264;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config)?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
        })
    }

    pub fn new_h265(params: &DecodeConfig) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(H265) decoder");
        let mut config = params.nvcodec_h265.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Hevc;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config)?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
        })
    }

    pub fn new_av1(params: &DecodeConfig) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(AV1) decoder");
        let mut config = params.nvcodec_av1.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Av1;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config)?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
        })
    }

    pub fn new_vp8(params: &DecodeConfig) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(VP8) decoder");
        let mut config = params.nvcodec_vp8.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Vp8;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config)?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
        })
    }

    pub fn new_vp9(params: &DecodeConfig) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(VP9) decoder");
        let mut config = params.nvcodec_vp9.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Vp9;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config)?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
        })
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
        if !matches!(
            frame.format,
            VideoFormat::H264
                | VideoFormat::H264AnnexB
                | VideoFormat::H265
                | VideoFormat::Vp8
                | VideoFormat::Vp9
                | VideoFormat::Av1
        ) {
            return Err(crate::Error::new(format!(
                "unsupported input format for NVDEC: {:?}",
                frame.format
            )));
        }

        // サンプルエントリからパラメータセットを抽出してキャッシュ
        if self.parameter_sets.is_none()
            && let Some(sample_entry) = &frame.sample_entry
        {
            self.parameter_sets = Some(extract_parameter_sets_annexb(sample_entry, frame.format)?);
        }

        let data = if matches!(
            frame.format,
            VideoFormat::Vp8 | VideoFormat::Vp9 | VideoFormat::Av1
        ) {
            // VP8 / VP9 / AV1 の場合は Annex B 形式は存在しないので、データの変換は不要
            Cow::Borrowed(&frame.data)
        } else if frame.format == VideoFormat::H264AnnexB {
            // すでに Annex B 形式の場合はそのまま使用
            Cow::Borrowed(&frame.data)
        } else {
            // Annex.B 形式に変換する (H264/H265)
            let mut data = &frame.data[..];
            let mut data_annexb = Vec::new();

            // キーフレームで、かつパラメータセットがデータに含まれていない場合は先頭に追加
            if frame.keyframe
                && let Some(parameter_sets) = &self.parameter_sets
                && !contains_parameter_sets(data, frame.format)
            {
                data_annexb.extend_from_slice(parameter_sets);
            }

            while !data.is_empty() {
                if data.len() < NALU_HEADER_LENGTH {
                    return Err(crate::Error::new(format!(
                        "invalid AVC/HEVC payload: NALU length header is truncated (remaining={})",
                        data.len()
                    )));
                }
                let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
                data = &data[NALU_HEADER_LENGTH..];

                if data.len() < n {
                    return Err(crate::Error::new(format!(
                        "invalid AVC/HEVC payload: NALU data is truncated (required={n}, remaining={})",
                        data.len()
                    )));
                }
                data_annexb.extend_from_slice(&[0, 0, 0, 1]);
                data_annexb.extend_from_slice(&data[..n]);

                data = &data[n..];
            }

            Cow::Owned(data_annexb)
        };

        self.inner.decode(&data)?;
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
        while let Some(nv12_frame) = self.inner.next_frame()? {
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("decoded frame produced without input frame"))?;

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
            shiguredo_libyuv::nv12_to_i420(&src, &mut dst, size)?;

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

/// サンプルエントリからパラメータセットを Annex.B 形式で抽出
fn extract_parameter_sets_annexb(
    sample_entry: &shiguredo_mp4::boxes::SampleEntry,
    format: VideoFormat,
) -> crate::Result<Vec<u8>> {
    use shiguredo_mp4::boxes::SampleEntry;

    match (sample_entry, format) {
        (SampleEntry::Hev1(entry), VideoFormat::H265) => {
            let mut annexb_data = Vec::new();
            for array in &entry.hvcc_box.nalu_arrays {
                for nalu in &array.nalus {
                    annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                    annexb_data.extend_from_slice(nalu);
                }
            }
            Ok(annexb_data)
        }
        (SampleEntry::Hvc1(entry), VideoFormat::H265) => {
            let mut annexb_data = Vec::new();
            for array in &entry.hvcc_box.nalu_arrays {
                for nalu in &array.nalus {
                    annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                    annexb_data.extend_from_slice(nalu);
                }
            }
            Ok(annexb_data)
        }
        (SampleEntry::Avc1(entry), VideoFormat::H264) => {
            let mut annexb_data = Vec::new();
            // SPS
            for sps in &entry.avcc_box.sps_list {
                annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                annexb_data.extend_from_slice(sps);
            }
            // PPS
            for pps in &entry.avcc_box.pps_list {
                annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                annexb_data.extend_from_slice(pps);
            }
            Ok(annexb_data)
        }
        _ => {
            // VP8 / VP9 / AV1はパラメータセットを個別に送る必要がないため空のVecを返す
            Ok(Vec::new())
        }
    }
}

/// データの先頭にパラメータセットが含まれているかチェック
fn contains_parameter_sets(data: &[u8], format: VideoFormat) -> bool {
    if data.len() < NALU_HEADER_LENGTH + 1 {
        return false;
    }

    match format {
        VideoFormat::H265 => {
            // H.265 の NAL unit type は 2バイト目の上位6ビット
            let nal_unit_type = (data[NALU_HEADER_LENGTH] >> 1) & 0x3F;
            matches!(
                nal_unit_type,
                H265_NALU_TYPE_PPS | H265_NALU_TYPE_SPS | H265_NALU_TYPE_VPS
            )
        }
        VideoFormat::H264 => {
            // H.264 の NAL unit type は下位5ビット
            let nal_unit_type = data[NALU_HEADER_LENGTH] & 0x1F;
            matches!(nal_unit_type, H264_NALU_TYPE_SPS | H264_NALU_TYPE_PPS)
        }
        VideoFormat::Av1 => {
            // AV1はパラメータセットの概念が異なるため常にfalse
            false
        }
        _ => false,
    }
}
