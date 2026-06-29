use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::DecodeConfig;
use crate::video::h264::{H264_NALU_TYPE_PPS, H264_NALU_TYPE_SPS};
use crate::video::h265::{
    H265_NALU_TYPE_PPS, H265_NALU_TYPE_SPS, H265_NALU_TYPE_VPS, NALU_HEADER_LENGTH,
};
use crate::video::{VideoFormat, VideoFrame};

/// 入力フレーム保持列の型エイリアス
///
/// `shiguredo_nvcodec` の callback と `decode()` push 側で共有するため `Arc<Mutex<>>` 化している。
type SharedInputQueue = Arc<Mutex<VecDeque<VideoFrame>>>;

/// Sender 化された NVDEC (NVIDIA hardware) デコーダー
///
/// 旧 `decoded_queue` / `error_slot` の中継キューを廃止し、`FnDecodeHandler` の callback 内で
/// 直接 NV12→I420 変換と `tx.blocking_send()` を実行する。callback は
/// `shiguredo_nvcodec::Decoder::decode()` を呼んだ tokio worker thread 上で同期 dispatch される
/// (NVIDIA Video Decoder SDK の `cuvidParseVideoData` 同期仕様) ため、`tokio::task::block_in_place`
/// で worker block を safe にした上で `blocking_send` を呼ぶ。
#[derive(Debug)]
pub struct NvcodecDecoder {
    inner: shiguredo_nvcodec::Decoder<
        shiguredo_nvcodec::FnDecodeHandler<(), shiguredo_nvcodec::Error>,
    >,
    // callback と decode() push 側で共有する入力フレーム保持列
    input_queue: SharedInputQueue,
    parameter_sets: Option<Vec<u8>>, // VPS/SPS/PPS をキャッシュ
}

/// `shiguredo_nvcodec::Decoder` の生成に必要なハンドラを構築する
///
/// callback 内で:
/// 1. 共有 `input_queue` から対応する入力フレームを取り出す
/// 2. NV12→I420 変換を実行
/// 3. `tokio::task::block_in_place` + `tx.blocking_send()` で上位 Receiver に流す
fn build_handler(
    tx: mpsc::Sender<crate::Result<VideoFrame>>,
    input_queue: SharedInputQueue,
) -> shiguredo_nvcodec::FnDecodeHandler<(), shiguredo_nvcodec::Error> {
    shiguredo_nvcodec::FnDecodeHandler::new(move |result| match result {
        Ok(nv12_frame) => {
            let input_frame = {
                let mut queue = input_queue
                    .lock()
                    .expect("nvcodec input queue lock poisoned");
                queue.pop_front()
            };
            let Some(input_frame) = input_frame else {
                // shiguredo_nvcodec の cuvidParseVideoData は投入順に callback を呼ぶ
                // 仕様のため、ここで None になることは想定外。設計通りなら起きないが
                // 安全側で fail-fast でエラーを上位に伝搬する。
                tokio::task::block_in_place(|| {
                    let _ = tx.blocking_send(Err(crate::Error::new(
                        "decoded frame produced without input frame",
                    )));
                });
                return;
            };

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

            let src = shiguredo_libyuv::Nv12Image {
                y: nv12_frame.y_plane(),
                y_stride: nv12_frame.y_stride(),
                uv: nv12_frame.uv_plane(),
                uv_stride: nv12_frame.uv_stride(),
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
            if let Err(e) = shiguredo_libyuv::nv12_to_i420(&src, &mut dst, size) {
                tokio::task::block_in_place(|| {
                    let _ = tx.blocking_send(Err(e.into()));
                });
                return;
            }

            let out = VideoFrame::new_i420(
                input_frame,
                width,
                height,
                y_plane,
                u_plane,
                v_plane,
                width,
                uv_width,
                uv_width,
            );

            tokio::task::block_in_place(|| {
                let _ = tx.blocking_send(Ok(out));
            });
        }
        Err(err) => {
            // callback 内エラーは即時に Sender 経由で上位 Receiver に通知する
            tokio::task::block_in_place(|| {
                let _ = tx.blocking_send(Err(crate::Error::new(format!(
                    "nvcodec decode error: {err}"
                ))));
            });
        }
    })
}

impl NvcodecDecoder {
    pub fn new_h264(
        params: &DecodeConfig,
        tx: mpsc::Sender<crate::Result<VideoFrame>>,
    ) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(H264) decoder");
        let mut config = params.nvcodec_h264.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::H264;
        Self::new_inner(config, tx)
    }

    pub fn new_h265(
        params: &DecodeConfig,
        tx: mpsc::Sender<crate::Result<VideoFrame>>,
    ) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(H265) decoder");
        let mut config = params.nvcodec_h265.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Hevc;
        Self::new_inner(config, tx)
    }

    pub fn new_av1(
        params: &DecodeConfig,
        tx: mpsc::Sender<crate::Result<VideoFrame>>,
    ) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(AV1) decoder");
        let mut config = params.nvcodec_av1.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Av1;
        Self::new_inner(config, tx)
    }

    pub fn new_vp8(
        params: &DecodeConfig,
        tx: mpsc::Sender<crate::Result<VideoFrame>>,
    ) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(VP8) decoder");
        let mut config = params.nvcodec_vp8.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Vp8;
        Self::new_inner(config, tx)
    }

    pub fn new_vp9(
        params: &DecodeConfig,
        tx: mpsc::Sender<crate::Result<VideoFrame>>,
    ) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(VP9) decoder");
        let mut config = params.nvcodec_vp9.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Vp9;
        Self::new_inner(config, tx)
    }

    /// 全コーデック共通の生成ロジック
    ///
    /// `input_queue` を `Arc<Mutex<>>` で生成し、handler と decoder 構造体の両方で共有する。
    fn new_inner(
        config: shiguredo_nvcodec::DecoderConfig,
        tx: mpsc::Sender<crate::Result<VideoFrame>>,
    ) -> crate::Result<Self> {
        let input_queue: SharedInputQueue = Arc::new(Mutex::new(VecDeque::new()));
        let handler = build_handler(tx, input_queue.clone());
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, handler)?,
            input_queue,
            parameter_sets: None,
        })
    }

    pub async fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
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

        // input_queue に投入してから decode を呼ぶ (callback 内で pop_front するため)
        {
            let mut queue = self
                .input_queue
                .lock()
                .expect("nvcodec input queue lock poisoned");
            queue.push_back(frame.to_stripped());
        }
        self.inner.decode(&data, ())?;
        Ok(())
    }

    pub async fn finish(&mut self) -> crate::Result<()> {
        // flush で in-flight 完了を待ち合わせる。callback は flush の内部で同期 dispatch されて
        // 残フレームをすべて Sender に流す。
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
