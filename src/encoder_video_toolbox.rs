use std::collections::VecDeque;

use orfail::OrFail;
use shiguredo_mp4::{
    boxes::{Avc1Box, AvccBox, Hev1Box, HvccBox, HvccNalUintArray, SampleEntry},
    Uint,
};

use crate::{
    layout::Layout,
    types::{CodecName, EvenUsize},
    video::{self, FrameRate, VideoFormat, VideoFrame},
    video_h264::{
        H264_LEVEL_3_1, H264_PROFILE_BASELINE, H265_NALU_TYPE_PPS, H265_NALU_TYPE_SPS,
        H265_NALU_TYPE_VPS, NALU_HEADER_LENGTH,
    },
};

#[derive(Debug)]
pub struct VideoToolboxEncoder {
    inner: shiguredo_video_toolbox::Encoder,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
    is_first: bool,
    width: EvenUsize,
    height: EvenUsize,
    format: VideoFormat,
    fps: FrameRate,
}

impl VideoToolboxEncoder {
    pub fn new_h264(layout: &Layout) -> orfail::Result<Self> {
        let width = layout.resolution.width();
        let height = layout.resolution.height();
        let config = shiguredo_video_toolbox::EncoderConfig {
            width: width.get(),
            height: height.get(),
            target_bitrate: layout.video_bitrate_bps(),
            fps_numerator: layout.fps.numerator.get(),
            fps_denominator: layout.fps.denumerator.get(),
            ..Default::default()
        };
        let inner = shiguredo_video_toolbox::Encoder::new_h264(&config).or_fail()?;
        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            is_first: true,
            width,
            height,
            format: VideoFormat::H264,
            fps: layout.fps,
        })
    }

    pub fn new_h265(layout: &Layout) -> orfail::Result<Self> {
        let width = layout.resolution.width();
        let height = layout.resolution.height();
        let config = shiguredo_video_toolbox::EncoderConfig {
            width: width.get(),
            height: height.get(),
            target_bitrate: layout.video_bitrate_bps(),
            fps_numerator: layout.fps.numerator.get(),
            fps_denominator: layout.fps.denumerator.get(),
            ..Default::default()
        };
        let inner = shiguredo_video_toolbox::Encoder::new_h265(&config).or_fail()?;
        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            is_first: true,
            width,
            height,
            format: VideoFormat::H265,
            fps: layout.fps,
        })
    }

    pub fn codec(&self) -> CodecName {
        if self.format == VideoFormat::H264 {
            CodecName::H264
        } else {
            CodecName::H265
        }
    }

    pub fn encode(&mut self, frame: VideoFrame) -> orfail::Result<()> {
        (frame.format == VideoFormat::I420).or_fail()?;

        let (y_plane, u_plane, v_plane) = frame.as_yuv_planes().or_fail()?;
        self.inner.encode(y_plane, u_plane, v_plane).or_fail()?;

        // Video Toolbox のエンコーダーは非同期で動作し、
        // エンコードが終わるまでは入力バッファへの参照を保持する必要があるので、
        // バッファもキューに入れておく。
        // (将来的にはこの辺りはエンコーダー内で隠蔽した方が使いやすそう）
        self.input_queue.push_back(frame);

        self.handle_encoded().or_fail()?;

        Ok(())
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        self.inner.finish().or_fail()?;
        self.handle_encoded().or_fail()?;
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }

    fn handle_encoded(&mut self) -> orfail::Result<()> {
        while let Some(frame) = self.inner.next_frame() {
            let input_frame = self.input_queue.pop_front().or_fail()?;
            let sample_entry = if self.is_first {
                self.is_first = false;
                let sample_entry = if self.format == VideoFormat::H264 {
                    h264_sample_entry(
                        self.width,
                        self.height,
                        frame.sps_list.clone(),
                        frame.pps_list.clone(),
                    )
                    .or_fail()?
                } else {
                    h265_sample_entry(
                        self.width,
                        self.height,
                        self.fps,
                        frame.vps_list.clone(),
                        frame.sps_list.clone(),
                        frame.pps_list.clone(),
                    )
                    .or_fail()?
                };
                Some(sample_entry)
            } else {
                None
            };

            self.output_queue.push_back(VideoFrame {
                source_id: None,
                data: frame.data,
                format: self.format,
                keyframe: frame.keyframe,
                width: self.width,
                height: self.height,
                timestamp: input_frame.timestamp,
                duration: input_frame.duration,
                sample_entry,
            });
        }
        Ok(())
    }
}

fn h264_sample_entry(
    width: EvenUsize,
    height: EvenUsize,
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
) -> orfail::Result<SampleEntry> {
    Ok(SampleEntry::Avc1(Avc1Box {
        visual: video::sample_entry_visual_fields(width, height),
        avcc_box: AvccBox {
            // 実際のエンコードストリームに合わせた値
            sps_list,
            pps_list,

            // 以下は Hisui では固定値
            avc_profile_indication: H264_PROFILE_BASELINE,
            avc_level_indication: H264_LEVEL_3_1,
            profile_compatibility: 0, // いったん 0 を指定しているが、もし支障があれば調整する
            length_size_minus_one: Uint::new(NALU_HEADER_LENGTH as u8 - 1),
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext_list: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    }))
}

fn h265_sample_entry(
    width: EvenUsize,
    height: EvenUsize,
    fps: FrameRate,
    vps_list: Vec<Vec<u8>>,
    sps_list: Vec<Vec<u8>>,
    pps_list: Vec<Vec<u8>>,
) -> orfail::Result<SampleEntry> {
    Ok(SampleEntry::Hev1(Hev1Box {
        visual: video::sample_entry_visual_fields(width, height),
        hvcc_box: HvccBox {
            // 以下はSora の録画ファイルに合わせた値（必要に応じて調整すること）
            general_profile_compatibility_flags: 0x60000000,
            general_constraint_indicator_flags: Uint::new(0xb00000000000),
            general_level_idc: 123,
            general_profile_space: Uint::new(0),
            general_tier_flag: Uint::new(0),
            num_temporal_layers: Uint::new(0),
            temporal_id_nested: Uint::new(0),
            min_spatial_segmentation_idc: Uint::new(0),
            parallelism_type: Uint::new(0),

            // Hisui ではフレームレートは固定（整数にならない場合は切り上げ）
            avg_frame_rate: (fps.numerator.get().div_ceil(fps.denumerator.get())) as u16,
            constant_frame_rate: Uint::new(1), // CFR (固定フレームレート)

            // Hisui ではヘッダサイズが固定であることが前提
            length_size_minus_one: Uint::new(NALU_HEADER_LENGTH as u8 - 1),

            // 以下は実際のストリームから取得した値
            nalu_arrays: vec![
                hvcc_nalu_array(H265_NALU_TYPE_VPS, vps_list),
                hvcc_nalu_array(H265_NALU_TYPE_SPS, sps_list),
                hvcc_nalu_array(H265_NALU_TYPE_PPS, pps_list),
            ],

            // これ以降はエンコーダーへの指定に対応する値を設定している

            // 色空間 (4:2:0)
            chroma_format_idc: Uint::new(1),

            // kVTProfileLevel_HEVC_Main_AutoLevel に対応する値
            general_profile_idc: Uint::new(1),     // Main
            bit_depth_luma_minus8: Uint::new(0),   // 8 ビット深度
            bit_depth_chroma_minus8: Uint::new(0), // 8 ビット深度
        },
        unknown_boxes: Vec::new(),
    }))
}

fn hvcc_nalu_array(nalu_type: u8, nalus: Vec<Vec<u8>>) -> HvccNalUintArray {
    HvccNalUintArray {
        array_completeness: Uint::new(1), // true
        nal_unit_type: Uint::new(nalu_type),
        nalus,
    }
}
