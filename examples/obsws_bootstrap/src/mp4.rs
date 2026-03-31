use std::io::{Seek, SeekFrom, Write};
use std::num::NonZeroU32;

use shiguredo_mp4::FixedPointNumber;
use shiguredo_mp4::Uint;
use shiguredo_mp4::boxes::{
    AudioSampleEntryFields, DopsBox, OpusBox, SampleEntry, VisualSampleEntryFields, Vp09Box,
    VpccBox,
};
use shiguredo_mp4::mux::{Mp4FileMuxer, Mp4FileMuxerOptions, Sample};

// MP4 のタイムスケールはマイクロ秒固定にする
pub const TIMESCALE: NonZeroU32 = NonZeroU32::MIN.saturating_add(1_000_000 - 1);

// VP9 SampleEntry 用の定数
const CHROMA_SUBSAMPLING_I420: Uint<u8, 3, 1> = Uint::new(1);
const BIT_DEPTH: Uint<u8, 4, 4> = Uint::new(8);
const LEGAL_RANGE: Uint<u8, 1> = Uint::new(0);
const BT_709: u8 = 1;

pub fn vp9_sample_entry(width: usize, height: usize) -> SampleEntry {
    SampleEntry::Vp09(Vp09Box {
        visual: VisualSampleEntryFields {
            data_reference_index: VisualSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            width: width as u16,
            height: height as u16,
            horizresolution: VisualSampleEntryFields::DEFAULT_HORIZRESOLUTION,
            vertresolution: VisualSampleEntryFields::DEFAULT_VERTRESOLUTION,
            frame_count: VisualSampleEntryFields::DEFAULT_FRAME_COUNT,
            compressorname: VisualSampleEntryFields::NULL_COMPRESSORNAME,
            depth: VisualSampleEntryFields::DEFAULT_DEPTH,
        },
        vpcc_box: VpccBox {
            profile: 0,
            level: 0,
            bit_depth: BIT_DEPTH,
            chroma_subsampling: CHROMA_SUBSAMPLING_I420,
            video_full_range_flag: LEGAL_RANGE,
            colour_primaries: BT_709,
            transfer_characteristics: BT_709,
            matrix_coefficients: BT_709,
            codec_initialization_data: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    })
}

pub fn opus_sample_entry_value(channels: u8, pre_skip: u16) -> SampleEntry {
    SampleEntry::Opus(OpusBox {
        audio: AudioSampleEntryFields {
            data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            channelcount: channels as u16,
            samplesize: AudioSampleEntryFields::DEFAULT_SAMPLESIZE,
            samplerate: FixedPointNumber::new(48000u16, 0u16),
        },
        dops_box: DopsBox {
            output_channel_count: channels,
            pre_skip,
            input_sample_rate: 48000,
            output_gain: 0,
        },
        unknown_boxes: Vec::new(),
    })
}

pub struct SimpleMp4Writer {
    file: std::io::BufWriter<std::fs::File>,
    muxer: Mp4FileMuxer,
    next_position: u64,
    pub video_sample_entry: Option<SampleEntry>,
    pub video_sample_count: usize,
    last_video_timestamp_us: Option<i64>,
    pub audio_sample_entry: Option<SampleEntry>,
    pub audio_sample_count: usize,
}

impl SimpleMp4Writer {
    pub fn new(path: &str) -> Result<Self, String> {
        let muxer_options = Mp4FileMuxerOptions {
            creation_timestamp: std::time::UNIX_EPOCH
                .elapsed()
                .map_err(|e| format!("failed to get epoch: {e}"))?,
            reserved_moov_box_size: 0,
        };
        let muxer =
            Mp4FileMuxer::with_options(muxer_options).map_err(|e| format!("muxer error: {e}"))?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("failed to create MP4 file: {e}"))?;

        let initial_bytes = muxer.initial_boxes_bytes();
        file.write_all(initial_bytes)
            .map_err(|e| format!("failed to write initial boxes: {e}"))?;
        let next_position = initial_bytes.len() as u64;

        Ok(Self {
            file: std::io::BufWriter::new(file),
            muxer,
            next_position,
            video_sample_entry: None,
            video_sample_count: 0,
            last_video_timestamp_us: None,
            audio_sample_entry: None,
            audio_sample_count: 0,
        })
    }

    pub fn append_video(
        &mut self,
        data: &[u8],
        keyframe: bool,
        sample_entry: Option<SampleEntry>,
        timestamp_us: i64,
    ) -> Result<(), String> {
        // duration は前のフレームとのタイムスタンプ差から計算する
        let duration_us = if let Some(last_ts) = self.last_video_timestamp_us {
            let d = timestamp_us - last_ts;
            if d > 0 { d as u32 } else { 33333 } // デフォルト 30fps 相当
        } else {
            33333
        };
        self.last_video_timestamp_us = Some(timestamp_us);

        self.file
            .write_all(data)
            .map_err(|e| format!("failed to write video data: {e}"))?;

        let sample = Sample {
            track_kind: shiguredo_mp4::TrackKind::Video,
            sample_entry: sample_entry.or_else(|| self.video_sample_entry.clone()),
            keyframe,
            timescale: TIMESCALE,
            duration: duration_us,
            composition_time_offset: None,
            data_offset: self.next_position,
            data_size: data.len(),
        };

        // 最初のサンプルで sample_entry を記録する
        if self.video_sample_entry.is_none() {
            self.video_sample_entry = sample.sample_entry.clone();
        }

        self.muxer
            .append_sample(&sample)
            .map_err(|e| format!("failed to append video sample: {e}"))?;
        self.next_position += data.len() as u64;
        self.video_sample_count += 1;
        Ok(())
    }

    pub fn append_audio(
        &mut self,
        data: &[u8],
        sample_entry: Option<SampleEntry>,
        duration: u32,
    ) -> Result<(), String> {
        self.file
            .write_all(data)
            .map_err(|e| format!("failed to write audio data: {e}"))?;

        let sample = Sample {
            track_kind: shiguredo_mp4::TrackKind::Audio,
            sample_entry: sample_entry.or_else(|| self.audio_sample_entry.clone()),
            keyframe: true,
            timescale: TIMESCALE,
            duration,
            composition_time_offset: None,
            data_offset: self.next_position,
            data_size: data.len(),
        };

        if self.audio_sample_entry.is_none() {
            self.audio_sample_entry = sample.sample_entry.clone();
        }

        self.muxer
            .append_sample(&sample)
            .map_err(|e| format!("failed to append audio sample: {e}"))?;
        self.next_position += data.len() as u64;
        self.audio_sample_count += 1;
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<(), String> {
        let finalized = self
            .muxer
            .finalize()
            .map_err(|e| format!("failed to finalize muxer: {e}"))?;
        for (offset, bytes) in finalized.offset_and_bytes_pairs() {
            self.file
                .seek(SeekFrom::Start(offset))
                .map_err(|e| format!("failed to seek: {e}"))?;
            self.file
                .write_all(bytes)
                .map_err(|e| format!("failed to write finalized data: {e}"))?;
        }
        self.file
            .flush()
            .map_err(|e| format!("failed to flush: {e}"))?;
        Ok(())
    }
}
