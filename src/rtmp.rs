use shiguredo_mp4::boxes::SampleEntry;
use std::sync::Arc;

use orfail::OrFail;

use crate::{audio::AudioData, stats::SharedAtomicCounter, video::VideoFrame};

#[derive(Debug)]
pub struct RtmpOutgoingFrameHandlerStats {
    pub total_audio_frame_count: SharedAtomicCounter,
    pub total_video_frame_count: SharedAtomicCounter,
    pub total_video_keyframe_count: SharedAtomicCounter,
    pub total_audio_sequence_header_count: SharedAtomicCounter,
    pub total_video_sequence_header_count: SharedAtomicCounter,
}

/// RTMP フレーム処理の共通ロジック
#[derive(Debug)]
pub struct RtmpOutgoingFrameHandler {
    video_sequence_header_data: Option<Vec<u8>>,
    audio_sequence_header_data: Option<Vec<u8>>,
    video_nalu_length_size: Option<u8>,
    received_keyframe: bool,
    stats: RtmpOutgoingFrameHandlerStats,
}

impl RtmpOutgoingFrameHandler {
    pub fn new(stats: RtmpOutgoingFrameHandlerStats) -> Self {
        Self {
            video_sequence_header_data: None,
            audio_sequence_header_data: None,
            video_nalu_length_size: None,
            received_keyframe: false,
            stats,
        }
    }

    /// 音声フレームを処理
    pub fn prepare_audio_frame(
        &mut self,
        audio: Arc<AudioData>,
    ) -> orfail::Result<(
        Option<shiguredo_rtmp::AudioFrame>,
        shiguredo_rtmp::AudioFrame,
    )> {
        // シーケンスヘッダーが必要な場合は生成
        let seq_frame = if self.audio_sequence_header_data.is_none() {
            if let Some(entry) = &audio.sample_entry {
                let seq_header = create_audio_sequence_header(entry)?;
                let frame = shiguredo_rtmp::AudioFrame {
                    timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(0),
                    format: shiguredo_rtmp::AudioFormat::Aac,
                    sample_rate: shiguredo_rtmp::AudioFrame::AAC_SAMPLE_RATE,
                    is_stereo: shiguredo_rtmp::AudioFrame::AAC_STEREO,
                    is_8bit_sample: false,
                    is_aac_sequence_header: true,
                    data: seq_header.clone(),
                };
                self.audio_sequence_header_data = Some(seq_header);
                self.stats.total_audio_sequence_header_count.increment();
                log::debug!("Sent AAC sequence header");
                Some(frame)
            } else {
                None
            }
        } else {
            None
        };

        // 実フレームデータ
        let frame = shiguredo_rtmp::AudioFrame {
            timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(
                audio.timestamp.as_millis() as u32
            ),
            format: shiguredo_rtmp::AudioFormat::Aac,
            sample_rate: shiguredo_rtmp::AudioFrame::AAC_SAMPLE_RATE,
            is_stereo: shiguredo_rtmp::AudioFrame::AAC_STEREO,
            is_8bit_sample: false,
            is_aac_sequence_header: false,
            data: audio.data.clone(),
        };
        self.stats.total_audio_frame_count.increment();

        Ok((seq_frame, frame))
    }

    /// 映像フレームを処理
    pub fn prepare_video_frame(
        &mut self,
        video: Arc<VideoFrame>,
    ) -> orfail::Result<
        Option<(
            Option<shiguredo_rtmp::VideoFrame>,
            shiguredo_rtmp::VideoFrame,
        )>,
    > {
        // キーフレームを待っている場合はスキップ
        if !self.received_keyframe && !video.keyframe {
            return Ok(None);
        }
        if !self.received_keyframe {
            self.received_keyframe = true;
        }

        let timestamp_ms = video.timestamp.as_millis() as u32;

        let seq_frame = if video.keyframe {
            self.stats.total_video_keyframe_count.increment();

            if let Some(entry) = &video.sample_entry {
                // サンプルエントリーから nalu_length_size を取得
                if self.video_nalu_length_size.is_none() {
                    self.video_nalu_length_size = Some(extract_nalu_length_size(entry)?);
                }

                let seq_header_data = create_video_sequence_header(entry)?;
                let frame = shiguredo_rtmp::VideoFrame {
                    timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(timestamp_ms),
                    composition_timestamp_offset: shiguredo_rtmp::RtmpTimestampDelta::ZERO,
                    frame_type: shiguredo_rtmp::VideoFrameType::KeyFrame,
                    codec: shiguredo_rtmp::VideoCodec::Avc,
                    avc_packet_type: Some(shiguredo_rtmp::AvcPacketType::SequenceHeader),
                    data: seq_header_data.clone(),
                };
                self.video_sequence_header_data = Some(seq_header_data);
                self.stats.total_video_sequence_header_count.increment();
                log::debug!("Sent H.264 sequence header");
                Some(frame)
            } else {
                None
            }
        } else {
            None
        };

        // 映像フレームデータをRTMP形式（AVC パケット形式）で処理
        let frame_data = match video.format {
            crate::video::VideoFormat::H264 => {
                // MP4形式（NALUヘッダ付き）のデータはそのまま使用
                // RTMPではAVC形式（サイズ付きNALU）で送信
                video.data.clone()
            }
            crate::video::VideoFormat::H264AnnexB => {
                // Annex B形式（開始コード付き）からAVC形式に変換が必要
                let nalu_length_size = self
                    .video_nalu_length_size
                    .ok_or_else(|| orfail::Failure::new("nalu_length_size not initialized"))?;
                crate::video_h264::convert_annexb_to_nalu(&video.data, nalu_length_size)?
            }
            _ => return Err(orfail::Failure::new("unsupported video format")),
        };

        let frame = shiguredo_rtmp::VideoFrame {
            timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(timestamp_ms),
            composition_timestamp_offset: shiguredo_rtmp::RtmpTimestampDelta::ZERO,
            frame_type: if video.keyframe {
                shiguredo_rtmp::VideoFrameType::KeyFrame
            } else {
                shiguredo_rtmp::VideoFrameType::InterFrame
            },
            codec: shiguredo_rtmp::VideoCodec::Avc,
            avc_packet_type: Some(shiguredo_rtmp::AvcPacketType::NalUnit),
            data: frame_data,
        };
        self.stats.total_video_frame_count.increment();

        Ok(Some((seq_frame, frame)))
    }
}

/// AVC1エントリーから nalu_length_size を抽出
fn extract_nalu_length_size(entry: &SampleEntry) -> orfail::Result<u8> {
    match entry {
        SampleEntry::Avc1(avc1) => Ok(avc1.avcc_box.length_size_minus_one.get() + 1),
        _ => Err(orfail::Failure::new("Not an H.264 video sample entry")),
    }
}

pub fn create_audio_sequence_header(entry: &SampleEntry) -> orfail::Result<Vec<u8>> {
    match entry {
        SampleEntry::Mp4a(mp4a) => mp4a
            .esds_box
            .es
            .dec_config_descr
            .dec_specific_info
            .as_ref()
            .map(|info| info.payload.clone())
            .ok_or_else(|| orfail::Failure::new("No decoder specific info available")),
        _ => Err(orfail::Failure::new("Not an audio sample entry")),
    }
}

pub fn create_video_sequence_header(entry: &SampleEntry) -> orfail::Result<Vec<u8>> {
    match entry {
        SampleEntry::Avc1(avc1) => {
            // shiguredo_rtmp::AvcSequenceHeader を使用して生成
            let avc_header = shiguredo_rtmp::AvcSequenceHeader {
                avc_profile_indication: avc1.avcc_box.avc_profile_indication,
                profile_compatibility: avc1.avcc_box.profile_compatibility,
                avc_level_indication: avc1.avcc_box.avc_level_indication,
                length_size_minus_one: avc1.avcc_box.length_size_minus_one.get(),
                sps_list: avc1.avcc_box.sps_list.clone(),
                pps_list: avc1.avcc_box.pps_list.clone(),
            };
            avc_header
                .to_bytes()
                .or_fail_with(|e| format!("Failed to create AVC sequence header: {e}"))
        }
        _ => Err(orfail::Failure::new("Not an H.264 video sample entry")),
    }
}
