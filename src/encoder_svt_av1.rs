use std::collections::VecDeque;

use orfail::OrFail;
use shiguredo_mp4::{
    Uint,
    boxes::{Av01Box, Av1cBox, SampleEntry},
};

use crate::{
    layout::Layout,
    types::EvenUsize,
    video::{self, VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct SvtAv1Encoder {
    inner: shiguredo_svt_av1::Encoder,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
    sample_entry: Option<SampleEntry>,
    width: EvenUsize,
    height: EvenUsize,
}

impl SvtAv1Encoder {
    pub fn new(layout: &Layout) -> orfail::Result<Self> {
        let width = layout.resolution.width();
        let height = layout.resolution.height();
        let config = shiguredo_svt_av1::EncoderConfig {
            target_bitrate: layout.video_bitrate_bps(),
            width: width.get(),
            height: height.get(),
            fps_numerator: layout.frame_rate.numerator.get(),
            fps_denominator: layout.frame_rate.denumerator.get(),
            ..layout.encode_params.svt_av1.clone().unwrap_or_default()
        };
        let inner = shiguredo_svt_av1::Encoder::new(&config).or_fail()?;
        let sample_entry = sample_entry(width, height, inner.extra_data());

        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            sample_entry: Some(sample_entry),
            width,
            height,
        })
    }

    pub fn encode(&mut self, frame: VideoFrame) -> orfail::Result<()> {
        (frame.format == VideoFormat::I420).or_fail()?;

        let (y_plane, u_plane, v_plane) = frame.as_yuv_planes().or_fail()?;
        self.inner.encode(y_plane, u_plane, v_plane).or_fail()?;
        self.input_queue.push_back(frame);
        self.handle_encoded_frames().or_fail()?;

        Ok(())
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        self.inner.finish().or_fail()?;
        self.handle_encoded_frames().or_fail()?;
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }

    fn handle_encoded_frames(&mut self) -> orfail::Result<()> {
        while let Some(frame) = self.inner.next_frame().or_fail()? {
            // B フレームはない前提なので、タイムスタンプのいれかわりもない
            let input_frame = self.input_queue.pop_front().or_fail()?;

            self.output_queue.push_back(VideoFrame {
                source_id: None,
                data: frame.data().to_vec(),
                format: VideoFormat::Av1,
                keyframe: frame.is_keyframe(),
                width: self.width,
                height: self.height,
                timestamp: input_frame.timestamp,
                duration: input_frame.duration,
                sample_entry: self.sample_entry.take(),
            });
        }
        Ok(())
    }
}

fn sample_entry(width: EvenUsize, height: EvenUsize, config_obus: &[u8]) -> SampleEntry {
    SampleEntry::Av01(Av01Box {
        visual: video::sample_entry_visual_fields(width, height),
        av1c_box: Av1cBox {
            seq_profile: Uint::new(0),            // Main profile
            seq_level_idx_0: Uint::new(0),        // Default level (unrestricted)
            seq_tier_0: Uint::new(0),             // Main tier
            high_bitdepth: Uint::new(0),          // false
            twelve_bit: Uint::new(0),             // false
            monochrome: Uint::new(0),             // false
            chroma_subsampling_x: Uint::new(1),   // 4:2:0 subsampling
            chroma_subsampling_y: Uint::new(1),   // 4:2:0 subsampling
            chroma_sample_position: Uint::new(0), // Colocated with luma (0, 0)
            initial_presentation_delay_minus_one: None,
            config_obus: config_obus.to_vec(),
        },
        unknown_boxes: Vec::new(),
    })
}
