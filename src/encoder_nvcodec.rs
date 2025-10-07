use std::collections::VecDeque;

use orfail::OrFail;
use shiguredo_mp4::boxes::SampleEntry;

use crate::{
    types::EvenUsize,
    video::{FrameRate, VideoFormat, VideoFrame},
    video_av1, video_h264, video_h265,
};

#[derive(Debug)]
pub struct NvcodecEncoder {
    inner: shiguredo_nvcodec::Encoder,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
    sample_entry: Option<SampleEntry>,
    encoded_format: VideoFormat,
}

impl NvcodecEncoder {
    pub fn new_h264(width: usize, height: usize) -> orfail::Result<Self> {
        log::debug!("create nvcodec(H264) encoder: {}x{}", width, height);
        let width = EvenUsize::new(width).or_fail()?;
        let height = EvenUsize::new(height).or_fail()?;

        let mut inner =
            shiguredo_nvcodec::Encoder::new_h264(width.get() as u32, height.get() as u32)
                .or_fail()?;
        let seq_params = inner.get_sequence_params().or_fail()?;
        let sample_entry =
            video_h264::h264_sample_entry_from_annexb(width.get(), height.get(), &seq_params)
                .or_fail()?;

        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            sample_entry: Some(sample_entry),
            encoded_format: VideoFormat::H264,
        })
    }

    pub fn new_h265(width: usize, height: usize) -> orfail::Result<Self> {
        log::debug!("create nvcodec(H265) encoder: {}x{}", width, height);
        let width = EvenUsize::new(width).or_fail()?;
        let height = EvenUsize::new(height).or_fail()?;
        // TODO: フレームレートを適切に設定する
        let fps = FrameRate::FPS_25;

        let mut inner =
            shiguredo_nvcodec::Encoder::new_h265(width.get() as u32, height.get() as u32)
                .or_fail()?;
        let seq_params = inner.get_sequence_params().or_fail()?;
        let sample_entry =
            video_h265::h265_sample_entry_from_annexb(width.get(), height.get(), fps, &seq_params)
                .or_fail()?;

        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            sample_entry: Some(sample_entry),
            encoded_format: VideoFormat::H265,
        })
    }

    pub fn new_av1(width: usize, height: usize) -> orfail::Result<Self> {
        log::debug!("create nvcodec(AV1) encoder: {}x{}", width, height);
        let width = EvenUsize::new(width).or_fail()?;
        let height = EvenUsize::new(height).or_fail()?;

        let mut inner =
            shiguredo_nvcodec::Encoder::new_av1(width.get() as u32, height.get() as u32)
                .or_fail()?;
        let seq_params = inner.get_sequence_params().or_fail()?;
        let sample_entry = video_av1::av1_sample_entry(width, height, &seq_params);

        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            sample_entry: Some(sample_entry),
            encoded_format: VideoFormat::Av1,
        })
    }

    pub fn encode(&mut self, frame: &VideoFrame) -> orfail::Result<()> {
        (frame.format == VideoFormat::I420).or_fail()?;

        // I420 から NV12 への変換
        let width = frame.width;
        let height = frame.height;
        let (y_plane, u_plane, v_plane) = frame.as_yuv_planes().or_fail()?;

        // NV12 用のバッファを確保
        let y_size = width * height;
        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);
        let uv_size = uv_width * uv_height * 2; // U と V が交互に配置されているため
        let total_size = y_size + uv_size;

        let mut nv12_data = vec![0u8; total_size];
        let (nv12_y, nv12_uv) = nv12_data.split_at_mut(y_size);

        // libyuv を使って I420 から NV12 に変換
        let src = shiguredo_libyuv::I420Planes {
            y: y_plane,
            y_stride: width,
            u: u_plane,
            u_stride: uv_width,
            v: v_plane,
            v_stride: uv_width,
        };

        let mut dst = shiguredo_libyuv::Nv12PlanesMut {
            y: nv12_y,
            y_stride: width,
            uv: nv12_uv,
            uv_stride: width,
        };

        let size = shiguredo_libyuv::ImageSize::new(width, height);
        shiguredo_libyuv::i420_to_nv12(&src, &mut dst, size).or_fail()?;

        // エンコード実行
        self.inner.encode(&nv12_data).or_fail()?;
        self.input_queue.push_back(frame.to_stripped());
        self.handle_encoded_frames().or_fail()?;
        Ok(())
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        self.inner.finish().or_fail()?;
        self.handle_encoded_frames().or_fail()?;
        Ok(())
    }

    fn handle_encoded_frames(&mut self) -> orfail::Result<()> {
        while let Some(encoded_frame) = self.inner.next_frame() {
            let input_frame = self.input_queue.pop_front().or_fail()?;

            // キーフレーム判定
            let keyframe = matches!(
                encoded_frame.picture_type(),
                shiguredo_nvcodec::PictureType::I | shiguredo_nvcodec::PictureType::Idr
            );

            // AV1 の場合は変換不要、H.264/H.265 の場合は Annex B から MP4 形式に変換
            let frame_data = if self.encoded_format == VideoFormat::Av1 {
                encoded_frame.into_data()
            } else {
                convert_annexb_to_mp4(encoded_frame.data()).or_fail()?
            };

            // VideoFrame を作成
            self.output_queue.push_back(VideoFrame {
                source_id: input_frame.source_id.clone(),
                data: frame_data,
                format: self.encoded_format,
                keyframe,
                width: input_frame.width,
                height: input_frame.height,
                timestamp: input_frame.timestamp,
                duration: input_frame.duration,
                sample_entry: self.sample_entry.take(),
            });
        }
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }
}

/// Annex B 形式から MP4 形式への変換
///
/// Annex B 形式: スタートコード (0x00000001 or 0x000001) + NALU データ
/// MP4 形式: サイズ (4バイト) + NALU データ
fn convert_annexb_to_mp4(annexb_data: &[u8]) -> orfail::Result<Vec<u8>> {
    let mut mp4_data = Vec::new();
    let mut pos = 0;

    while pos < annexb_data.len() {
        // スタートコードを探す (0x00000001 or 0x000001)
        let start_code_len =
            if pos + 4 <= annexb_data.len() && annexb_data[pos..pos + 4] == [0, 0, 0, 1] {
                4
            } else if pos + 3 <= annexb_data.len() && annexb_data[pos..pos + 3] == [0, 0, 1] {
                3
            } else if pos == 0 {
                return Err(orfail::Failure::new("No start code found at beginning"));
            } else {
                break;
            };

        pos += start_code_len;

        // 次のスタートコードまたはデータ終端を探す
        let nalu_start = pos;
        let mut nalu_end = annexb_data.len();

        for i in (pos + 3)..annexb_data.len() {
            if i + 4 <= annexb_data.len() && annexb_data[i..i + 4] == [0, 0, 0, 1] {
                nalu_end = i;
                break;
            }
            if i + 3 <= annexb_data.len() && annexb_data[i..i + 3] == [0, 0, 1] {
                nalu_end = i;
                break;
            }
        }

        let nalu_size = nalu_end - nalu_start;

        // MP4 形式: 4 バイトのサイズ + NALU データ
        mp4_data.extend_from_slice(&(nalu_size as u32).to_be_bytes());
        mp4_data.extend_from_slice(&annexb_data[nalu_start..nalu_end]);

        pos = nalu_end;
    }

    Ok(mp4_data)
}
