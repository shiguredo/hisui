use std::collections::VecDeque;

use orfail::OrFail;

use crate::video::{VideoFormat, VideoFrame};

#[derive(Debug)]
pub struct NvcodecEncoder {
    inner: shiguredo_nvcodec::Encoder,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
}

impl NvcodecEncoder {
    pub fn new_h265(width: usize, height: usize) -> orfail::Result<Self> {
        log::debug!("create nvcodec(H265) encoder: {}x{}", width, height);
        Ok(Self {
            inner: shiguredo_nvcodec::Encoder::new_h265(width as u32, height as u32).or_fail()?,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
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
        let uv_size = uv_width * uv_height * 2; // インターリーブなので2倍
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
            uv_stride: width, // NV12 の UV プレーンは横幅と同じストライド
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

            // H.265 VideoFrame を作成
            self.output_queue.push_back(VideoFrame {
                source_id: input_frame.source_id.clone(),
                data: encoded_frame.data().to_vec(),
                format: VideoFormat::H265,
                keyframe,
                width: input_frame.width,
                height: input_frame.height,
                timestamp: input_frame.timestamp,
                duration: input_frame.duration,
                sample_entry: None, // TODO: H.265 sample entry の生成
            });
        }
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }
}
