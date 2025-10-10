use std::collections::VecDeque;

use orfail::OrFail;
use shiguredo_mp4::boxes::SampleEntry;

use crate::{
    encoder::VideoEncoderOptions,
    types::CodecName,
    video::{VideoFormat, VideoFrame},
    video_av1, video_h264, video_h265,
};

#[derive(Debug)]
pub struct NvcodecEncoder {
    inner: shiguredo_nvcodec::Encoder,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
    sample_entry: Option<SampleEntry>,
    encoded_format: VideoFormat,
    av1_sequence_header: Vec<u8>,
}

impl NvcodecEncoder {
    pub fn new_h264(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        log::debug!("create nvcodec(H264) encoder: {}x{}", width, height);

        let config = shiguredo_nvcodec::EncoderConfig {
            width: width as u32,
            height: height as u32,
            fps_numerator: options.frame_rate.numerator.get() as u32,
            fps_denominator: options.frame_rate.denumerator.get() as u32,
            target_bitrate: Some(options.bitrate as u32),
            ..options.encode_params.nvcodec_h264.clone()
        };
        log::debug!("nvcodec h264 encoder config: {config:?}");

        let mut inner = shiguredo_nvcodec::Encoder::new_h264(config).or_fail()?;
        let seq_params = inner.get_sequence_params().or_fail()?;
        let sample_entry =
            video_h264::h264_sample_entry_from_annexb(width, height, &seq_params).or_fail()?;

        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            sample_entry: Some(sample_entry),
            encoded_format: VideoFormat::H264,
            av1_sequence_header: Vec::new(),
        })
    }

    pub fn new_h265(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        log::debug!("create nvcodec(H265) encoder: {}x{}", width, height);

        let config = shiguredo_nvcodec::EncoderConfig {
            width: width as u32,
            height: height as u32,
            fps_numerator: options.frame_rate.numerator.get() as u32,
            fps_denominator: options.frame_rate.denumerator.get() as u32,
            target_bitrate: Some(options.bitrate as u32),
            ..options.encode_params.nvcodec_h265.clone()
        };
        log::debug!("nvcodec h265 encoder config: {config:?}");

        let mut inner = shiguredo_nvcodec::Encoder::new_h265(config).or_fail()?;
        let seq_params = inner.get_sequence_params().or_fail()?;
        let sample_entry = video_h265::h265_sample_entry_from_annexb(
            width,
            height,
            options.frame_rate,
            &seq_params,
        )
        .or_fail()?;

        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            sample_entry: Some(sample_entry),
            encoded_format: VideoFormat::H265,
            av1_sequence_header: Vec::new(),
        })
    }

    pub fn new_av1(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let width = options.width;
        let height = options.height;
        log::debug!(
            "create nvcodec(AV1) encoder: {}x{}",
            width.get(),
            height.get()
        );

        let config = shiguredo_nvcodec::EncoderConfig {
            width: width.get() as u32,
            height: height.get() as u32,
            fps_numerator: options.frame_rate.numerator.get() as u32,
            fps_denominator: options.frame_rate.denumerator.get() as u32,
            target_bitrate: Some(options.bitrate as u32),
            ..options.encode_params.nvcodec_av1.clone()
        };
        log::debug!("nvcodec av1 encoder config: {config:?}");

        let mut inner = shiguredo_nvcodec::Encoder::new_av1(config).or_fail()?;

        // NVENC SDK 13.0 のドキュメント (https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html#retrieving-sequence-parameters)
        // には以下の記載がある:
        //   "By default, SPS/PPS and Sequence Header OBU data will be attached to every IDR frame and Key frame for H.264/HEVC and AV1 respectively."
        //
        // しかし実際には、AV1の場合、最初のキーフレームにのみ Sequence Header OBU が付与され、
        // 二番目以降のキーフレームには含まれない。これにより、二番目以降のキーフレームからシークすると、
        // デコーダが解像度やプロファイルなどの情報を取得できず、映像が再生できない問題が発生する。
        //
        // そのため、ここで Sequence Header OBU を get_sequence_params() で取得して保持しておき、
        // キーフレームのエンコード時に Sequence Header が含まれていない場合は、
        // hisui 側で明示的に付与するワークアラウンドを実装している。
        let seq_params = inner.get_sequence_params().or_fail()?;

        let sample_entry = video_av1::av1_sample_entry(width, height, &seq_params);

        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            sample_entry: Some(sample_entry),
            encoded_format: VideoFormat::Av1,
            av1_sequence_header: seq_params,
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

            // AV1 の場合は変換不要だが、キーフレームに Sequence Header が含まれていない場合は付与
            // H.264/H.265 の場合は Annex B から MP4 形式に変換
            let frame_data = if self.encoded_format == VideoFormat::Av1 {
                let mut data = encoded_frame.into_data();

                // AV1 のキーフレームで Sequence Header OBU が含まれていない場合は先頭に付与
                if keyframe && !self.has_sequence_header(&data) {
                    log::debug!(
                        "prepending Sequence Header OBU to AV1 keyframe (seq_header: {} bytes, frame: {} bytes)",
                        seq_header.len().len(),
                        data.len()
                    );
                    let mut new_data = Vec::with_capacity(seq_header.len() + data.len());
                    new_data.extend_from_slice(&self.av1_sequence_header);
                    new_data.extend_from_slice(&data);
                    data = new_data;
                }
                data
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

    /// AV1 ペイロードの先頭に Sequence Header OBU が含まれているかチェック
    fn has_sequence_header(&self, data: &[u8]) -> bool {
        if data.len() < 2 {
            return false;
        }

        // 先頭の OBU Header を解析
        // obu_header のビット構成:
        //   - bit 0: obu_forbidden_bit (常に0)
        //   - bit 1-4: obu_type
        //   - bit 5: obu_extension_flag
        //   - bit 6: obu_has_size_field
        //   - bit 7: obu_reserved_1bit
        let obu_header = data[0];
        let obu_type = (obu_header >> 3) & 0x0F;

        // 先頭が Sequence Header (type=1) なら true
        obu_type == 1
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }

    pub fn codec(&self) -> CodecName {
        self.encoded_format.codec_name().expect("infallible")
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
