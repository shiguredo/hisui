use orfail::OrFail;
use shiguredo_mp4::{
    Uint,
    boxes::{Avc1Box, AvccBox, SampleEntry},
};

use crate::{
    layout::Layout,
    types::EvenUsize,
    video::{self, VideoFormat, VideoFrame},
    video_h264::{
        H264_LEVEL_3_1, H264_NALU_TYPE_PPS, H264_NALU_TYPE_SEI, H264_NALU_TYPE_SPS,
        H264_PROFILE_BASELINE, H264AnnexBNalUnits, NALU_HEADER_LENGTH,
    },
};

#[derive(Debug)]
pub struct Openh264Encoder {
    inner: shiguredo_openh264::Encoder,
    encoded: Option<VideoFrame>,
    is_first: bool,
}

impl Openh264Encoder {
    pub fn new(lib: shiguredo_openh264::Openh264Library, layout: &Layout) -> orfail::Result<Self> {
        let width = layout.resolution.width().get();
        let height = layout.resolution.height().get();
        let config = shiguredo_openh264::EncoderConfig {
            fps_numerator: layout.frame_rate.numerator.get(),
            fps_denominator: layout.frame_rate.denumerator.get(),
            width,
            height,
            target_bitrate: layout.video_bitrate_bps(),
            ..layout.encode_params.openh264.clone().unwrap_or_default()
        };
        let inner = shiguredo_openh264::Encoder::new(lib, &config).or_fail()?;
        Ok(Self {
            inner,
            encoded: None,
            is_first: true,
        })
    }

    pub fn encode(&mut self, frame: VideoFrame) -> orfail::Result<()> {
        (frame.format == VideoFormat::I420).or_fail()?;

        let (y_plane, u_plane, v_plane) = frame.as_yuv_planes().or_fail()?;
        let encoded = self.inner.encode(y_plane, u_plane, v_plane).or_fail()?;
        let Some(encoded) = encoded else {
            return Ok(());
        };

        let sample_entry = if self.is_first {
            self.is_first = false;
            Some(sample_entry(frame.width, frame.height, &encoded.data).or_fail()?)
        } else {
            None
        };

        // AnnexB から MP4 向けの形式に変換する
        let mut data = Vec::new();
        for nal in H264AnnexBNalUnits::new(&encoded.data) {
            let nal = nal.or_fail()?;
            if nal.ty == H264_NALU_TYPE_SEI {
                // 一部のタイプは無視する
                continue;
            }

            data.extend_from_slice(&(nal.data.len() as u32).to_be_bytes());
            data.extend_from_slice(nal.data);
        }

        self.encoded = Some(VideoFrame {
            source_id: None,
            data,
            format: VideoFormat::H264,
            keyframe: encoded.keyframe,
            width: frame.width,
            height: frame.height,
            timestamp: frame.timestamp,
            duration: frame.duration,
            sample_entry,
        });

        Ok(())
    }

    // 他のエンコーダーに合わせてメソッドだけ用意しておく
    pub fn finish(&mut self) -> orfail::Result<()> {
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.encoded.take()
    }
}

fn sample_entry(width: EvenUsize, height: EvenUsize, data: &[u8]) -> orfail::Result<SampleEntry> {
    // H.264 ストリームから SPS と PPS と取り出す
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();
    for nalu in H264AnnexBNalUnits::new(data) {
        let nalu = nalu.or_fail()?;
        match nalu.ty {
            H264_NALU_TYPE_SPS => sps_list.push(nalu.data.to_vec()),
            H264_NALU_TYPE_PPS => pps_list.push(nalu.data.to_vec()),
            _ => {}
        }
    }
    (!sps_list.is_empty()).or_fail()?;
    (!pps_list.is_empty()).or_fail()?;

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
