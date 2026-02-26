use std::collections::VecDeque;
use std::sync::Arc;

use shiguredo_mp4::{
    Uint,
    boxes::{SampleEntry, Vp08Box, Vp09Box, VpccBox},
};

use crate::{
    encoder::VideoEncoderOptions,
    types::CodecName,
    video::{self, RawVideoFrame, VideoFormat, VideoFrame, VideoFrameSize},
};

// エンコードパラメーターのデフォルト値
pub const DEFAULT_CQ_LEVEL: &str = "30";
pub const DEFAULT_MIN_Q: &str = "10";
pub const DEFAULT_MAX_Q: &str = "50";

// サンプルパラメーター用の設定値
const CHROMA_SUBSAMPLING_I420: Uint<u8, 3, 1> = Uint::new(1); // 4:2:0 colocated with luma (0,0)
const BIT_DEPTH: Uint<u8, 4, 4> = Uint::new(8);
const LEGAL_RANGE: Uint<u8, 1> = Uint::new(0); // 典型的な値。必要に応じて調整する
const BT_709: u8 = 1; // 典型的な値。必要に応じて調整する

#[derive(Debug)]
pub struct LibvpxEncoder {
    inner: shiguredo_libvpx::Encoder,
    format: VideoFormat,
    sample_entry: Option<SampleEntry>,
    input_queue: VecDeque<Arc<RawVideoFrame>>,
    output_queue: VecDeque<VideoFrame>,
}

impl LibvpxEncoder {
    pub fn new_vp8(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        let config = shiguredo_libvpx::EncoderConfig {
            width,
            height,
            fps_numerator: options.frame_rate.numerator.get(),
            fps_denominator: options.frame_rate.denumerator.get(),
            target_bitrate: options.bitrate,
            ..options.encode_params.libvpx_vp8.clone()
        };
        tracing::debug!("libvpx vp8 encoder config: {config:?}");
        let inner = shiguredo_libvpx::Encoder::new_vp8(&config)?;
        let sample_entry = vp8_sample_entry(width, height);

        Ok(Self {
            inner,
            format: VideoFormat::Vp8,
            sample_entry: Some(sample_entry),
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
        })
    }

    pub fn new_vp9(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        let config = shiguredo_libvpx::EncoderConfig {
            width,
            height,
            fps_numerator: options.frame_rate.numerator.get(),
            fps_denominator: options.frame_rate.denumerator.get(),
            target_bitrate: options.bitrate,
            ..options.encode_params.libvpx_vp9.clone()
        };
        tracing::debug!("libvpx vp9 encoder config: {config:?}");
        let inner = shiguredo_libvpx::Encoder::new_vp9(&config)?;
        let sample_entry = vp9_sample_entry(width, height);

        Ok(Self {
            inner,
            format: VideoFormat::Vp9,
            sample_entry: Some(sample_entry),
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
        })
    }

    pub fn codec(&self) -> CodecName {
        if self.format == VideoFormat::Vp8 {
            CodecName::Vp8
        } else {
            CodecName::Vp9
        }
    }

    pub fn encode(&mut self, frame: Arc<RawVideoFrame>) -> crate::Result<()> {
        let video_frame = frame.as_video_frame();
        if video_frame.format != VideoFormat::I420 {
            return Err(crate::Error::new(format!(
                "expected I420 format, got {:?}",
                video_frame.format
            )));
        }

        let (y_plane, u_plane, v_plane) = video_frame
            .as_yuv_planes()
            .ok_or_else(|| crate::Error::new("invalid I420 frame data"))?;
        self.inner.encode(y_plane, u_plane, v_plane)?;
        self.input_queue.push_back(frame);
        self.handle_encoded_frames()?;

        Ok(())
    }

    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.finish()?;
        self.handle_encoded_frames()?;
        Ok(())
    }

    fn handle_encoded_frames(&mut self) -> crate::Result<()> {
        while let Some(frame) = self.inner.next_frame() {
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("encoded frame produced without input frame"))?;
            self.output_queue.push_back(VideoFrame {
                sample_entry: self.sample_entry.take(),
                data: frame.data().to_vec(),
                format: self.format,
                keyframe: frame.is_keyframe(),
                size: Some(VideoFrameSize {
                    width: frame.width() as usize,
                    height: frame.height() as usize,
                }),
                timestamp: input_frame.as_video_frame().timestamp,
            });
        }

        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }
}

fn vp8_sample_entry(width: usize, height: usize) -> SampleEntry {
    SampleEntry::Vp08(Vp08Box {
        visual: video::sample_entry_visual_fields(width, height),
        vpcc_box: VpccBox {
            // Hisui 固有の固定値 (VP8 / VP9 共通)
            bit_depth: BIT_DEPTH,
            chroma_subsampling: CHROMA_SUBSAMPLING_I420,
            video_full_range_flag: LEGAL_RANGE,
            colour_primaries: BT_709,
            transfer_characteristics: BT_709,
            matrix_coefficients: BT_709,

            // VP8 では以下の値は常に固定値
            profile: 0,
            level: 0,
            codec_initialization_data: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    })
}

fn vp9_sample_entry(width: usize, height: usize) -> SampleEntry {
    SampleEntry::Vp09(Vp09Box {
        visual: video::sample_entry_visual_fields(width, height),
        vpcc_box: VpccBox {
            profile: 0, // 0 は "8bit color depth, chroma-subsampling-4:2:0" を意味する
            level: 0,   // 適切な値を指定するのは大変なので undefined 扱いにしておく

            // Hisui 固有の固定値 (VP8 / VP9 共通)
            bit_depth: BIT_DEPTH,
            chroma_subsampling: CHROMA_SUBSAMPLING_I420,
            video_full_range_flag: LEGAL_RANGE,
            colour_primaries: BT_709,
            transfer_characteristics: BT_709,
            matrix_coefficients: BT_709,

            // VP9 では以下の値は常に固定値
            codec_initialization_data: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    })
}
