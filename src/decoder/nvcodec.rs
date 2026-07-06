use std::borrow::Cow;

use super::{DecodeConfig, OutputSink};
use crate::video::h264::{H264_NALU_TYPE_PPS, H264_NALU_TYPE_SPS};
use crate::video::h265::{
    H265_NALU_TYPE_PPS, H265_NALU_TYPE_SPS, H265_NALU_TYPE_VPS, NALU_HEADER_LENGTH,
};
use crate::video::{VideoFormat, VideoFrame};

#[derive(Debug)]
pub struct NvcodecDecoder {
    inner: shiguredo_nvcodec::Decoder<
        shiguredo_nvcodec::FnDecodeHandler<VideoFrame, shiguredo_nvcodec::Error>,
    >,
    parameter_sets: Option<Vec<u8>>, // VPS/SPS/PPS をキャッシュ
}

/// CUDA ワーカースレッドから呼ばれるコールバックの本体を共有クロージャ化したもの。
fn build_handler(
    sink: OutputSink,
) -> shiguredo_nvcodec::FnDecodeHandler<VideoFrame, shiguredo_nvcodec::Error> {
    shiguredo_nvcodec::FnDecodeHandler::new(move |result| {
        handle_decode_callback(&sink, result);
    })
}

fn handle_decode_callback(
    sink: &OutputSink,
    result: std::result::Result<
        shiguredo_nvcodec::DecodedFrame<VideoFrame>,
        shiguredo_nvcodec::Error,
    >,
) {
    match result {
        Ok(decoded) => match convert_nv12_to_i420(decoded) {
            Ok(frame) => sink.emit_ok(frame),
            Err(err) => sink.emit_err(err),
        },
        Err(err) => {
            sink.emit_err(crate::Error::new(format!("nvcodec decode error: {err}")));
        }
    }
}

/// NV12 フォーマットのデコード済みフレームを I420 に変換して `VideoFrame` を構築する
///
/// `decoded.user_data()` に入力フレームが含まれており、 変換後のフレームにペアリングする。
fn convert_nv12_to_i420(
    decoded: shiguredo_nvcodec::DecodedFrame<VideoFrame>,
) -> crate::Result<VideoFrame> {
    let width = decoded.width();
    let height = decoded.height();

    let y_size = width * height;
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let uv_size = uv_width * uv_height;
    let total_size = y_size + uv_size * 2;

    let mut i420_data = vec![0u8; total_size];
    let (y_plane, rest) = i420_data.split_at_mut(y_size);
    let (u_plane, v_plane) = rest.split_at_mut(uv_size);

    let src = shiguredo_libyuv::Nv12Image {
        y: decoded.y_plane(),
        y_stride: decoded.y_stride(),
        uv: decoded.uv_plane(),
        uv_stride: decoded.uv_stride(),
    };
    let mut dst = shiguredo_libyuv::I420ImageMut {
        y: y_plane,
        y_stride: width,
        u: u_plane,
        u_stride: uv_width,
        v: v_plane,
        v_stride: uv_width,
    };

    let size = shiguredo_libyuv::ImageSize::new(width, height);
    shiguredo_libyuv::nv12_to_i420(&src, &mut dst, size)?;

    // `decoded` の借用終了後、 `into_parts` で入力フレームを取り出す
    let (_nv12_data, input_frame) = decoded.into_parts();

    Ok(VideoFrame::new_i420(
        input_frame,
        width,
        height,
        y_plane,
        u_plane,
        v_plane,
        width,
        uv_width,
        uv_width,
    ))
}

impl NvcodecDecoder {
    pub fn new_h264(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(H264) decoder");
        let mut config = params.nvcodec_h264.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::H264;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, build_handler(sink))?,
            parameter_sets: None,
        })
    }

    pub fn new_h265(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(H265) decoder");
        let mut config = params.nvcodec_h265.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Hevc;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, build_handler(sink))?,
            parameter_sets: None,
        })
    }

    pub fn new_av1(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(AV1) decoder");
        let mut config = params.nvcodec_av1.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Av1;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, build_handler(sink))?,
            parameter_sets: None,
        })
    }

    pub fn new_vp8(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(VP8) decoder");
        let mut config = params.nvcodec_vp8.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Vp8;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, build_handler(sink))?,
            parameter_sets: None,
        })
    }

    pub fn new_vp9(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(VP9) decoder");
        let mut config = params.nvcodec_vp9.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Vp9;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, build_handler(sink))?,
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

        // サンプルエントリーからパラメータセットを抽出してキャッシュ
        if self.parameter_sets.is_none()
            && let Some(sample_entry) = &frame.sample_entry
        {
            self.parameter_sets = Some(extract_parameter_sets_annexb(
                sample_entry.get(),
                frame.format,
            )?);
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

        // 入力フレームを軽量化した VideoFrame を `UserData` として渡す。
        // shiguredo_nvcodec が `pending_user_data` を FIFO 管理し、 Err 時の clear まで自動で行う。
        self.inner.decode(&data, frame.to_stripped())?;
        Ok(())
    }

    /// 進行中のフレームのデコード完了を待ち合わせる。
    ///
    /// `shiguredo_nvcodec::Decoder::flush()` の戻り時点でコールバックはすべて同期的に呼び切られている
    /// 前提であり、 残フレームおよびエラーのシンクへの emit はそのコールバック内で完了するため、
    /// ここでは追加処理不要。 コールバック内で発生した `Err` は `sink.emit_err()` 経由で内部
    /// チャンネルに積まれ、 `finish()` の戻り値からは検出できないため、 利用側は `finish()` の直後に
    /// `poll_output` の `try_recv` ループで残物を全て吸い出すこと。
    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.flush()?;
        Ok(())
    }
}

/// サンプルエントリーからパラメータセットを Annex.B 形式で抽出
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
