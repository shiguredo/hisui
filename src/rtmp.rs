use std::sync::Arc;

use orfail::OrFail;
use shiguredo_mp4::boxes::SampleEntry;

use crate::{audio::AudioData, stats::SharedAtomicCounter, video::VideoFrame};

#[derive(Debug)]
pub struct RtmpOutgoingFrameHandlerStats {
    pub total_audio_frame_count: SharedAtomicCounter,
    pub total_video_frame_count: SharedAtomicCounter,
    pub total_video_keyframe_count: SharedAtomicCounter,
    pub total_audio_sequence_header_count: SharedAtomicCounter,
    pub total_video_sequence_header_count: SharedAtomicCounter,
}

#[derive(Debug)]
pub struct RtmpIncomingFrameHandlerStats {
    pub total_audio_frame_count: SharedAtomicCounter,
    pub total_video_frame_count: SharedAtomicCounter,
    pub total_video_keyframe_count: SharedAtomicCounter,
    pub total_audio_sequence_header_count: SharedAtomicCounter,
    pub total_video_sequence_header_count: SharedAtomicCounter,
}

/// RTMP フレーム処理の共通ロジック（送信側）
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

        // 映像フレームデータを Annex B 形式に変換
        let frame_data = match video.format {
            crate::video::VideoFormat::H264 => {
                let nalu_length_size = self
                    .video_nalu_length_size
                    .ok_or_else(|| orfail::Failure::new("nalu_length_size not initialized"))?;
                crate::video_h264::convert_nalu_to_annexb(&video.data, nalu_length_size)?
            }
            crate::video::VideoFormat::H264AnnexB => video.data.clone(),
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

/// RTMP フレーム処理の共通ロジック（受信側）
#[derive(Debug)]
pub struct RtmpIncomingFrameHandler {
    audio_codec_info: Option<AudioCodecInfo>,
    video_sample_entry: Option<SampleEntry>,
    received_video_keyframe: bool,
    stats: RtmpIncomingFrameHandlerStats,
}

#[derive(Debug, Clone)]
struct AudioCodecInfo {
    format: crate::audio::AudioFormat,
    sample_rate: u32,
    channels: u8,
    aac_config: Vec<u8>,
}

impl RtmpIncomingFrameHandler {
    pub fn new(stats: RtmpIncomingFrameHandlerStats) -> Self {
        Self {
            audio_codec_info: None,
            video_sample_entry: None,
            received_video_keyframe: false,
            stats,
        }
    }

    /// 受信した音声フレームを処理
    pub fn process_audio_frame(
        &mut self,
        frame: shiguredo_rtmp::AudioFrame,
    ) -> orfail::Result<AudioData> {
        // シーケンスヘッダーの処理
        if frame.is_aac_sequence_header {
            self.stats.total_audio_sequence_header_count.increment();

            // TODO: ダミー値、AAC の場合は audio specific config バイナリから取得する必要がある
            let sample_rate = 44000;
            let channels = 2;

            self.audio_codec_info = Some(AudioCodecInfo {
                format: crate::audio::AudioFormat::Aac,
                sample_rate,
                channels,
                aac_config: frame.data.clone(),
            });

            log::debug!("Received AAC sequence header");
        }

        self.stats.total_audio_frame_count.increment();

        let codec_info = self.audio_codec_info.as_ref().or_fail()?;

        Ok(AudioData {
            timestamp: std::time::Duration::from_millis(frame.timestamp.as_millis() as u64),
            duration: std::time::Duration::ZERO,
            format: codec_info.format,
            sample_rate: codec_info.sample_rate as u16, // TODO: 理屈上は精度が落ちる可能性があるキャスト
            stereo: codec_info.channels == 2,
            sample_entry: None,
            data: frame.data,
            source_id: None, // TODO: ちゃんとする
        })
    }

    /// 受信した映像フレームを処理
    pub fn process_video_frame(
        &mut self,
        frame: shiguredo_rtmp::VideoFrame,
    ) -> orfail::Result<Option<VideoFrame>> {
        // シーケンスヘッダーの処理
        if frame.avc_packet_type == Some(shiguredo_rtmp::AvcPacketType::SequenceHeader) {
            self.stats.total_video_sequence_header_count.increment();

            // Annex B 形式の SPS/PPS からサンプルエントリーを生成
            // TODO: SPS から実際の width, height を抽出する
            let sample_entry = crate::video_h264::h264_sample_entry_from_annexb(
                1920, // placeholder: 実装時に SPS から抽出
                1080, // placeholder: 実装時に SPS から抽出
                &frame.data,
            )?;

            self.video_sample_entry = Some(sample_entry);

            log::debug!("Received H.264 sequence header");
            return Ok(None); // シーケンスヘッダー自体はスキップ
        }

        // キーフレームを待っている場合はスキップ
        if !self.received_video_keyframe
            && frame.frame_type != shiguredo_rtmp::VideoFrameType::KeyFrame
        {
            return Ok(None);
        }

        if frame.frame_type == shiguredo_rtmp::VideoFrameType::KeyFrame {
            self.received_video_keyframe = true;
            self.stats.total_video_keyframe_count.increment();
        }

        self.stats.total_video_frame_count.increment();

        let sample_entry = self.video_sample_entry.as_ref().or_fail()?;

        // サンプルエントリーから寸法を取得
        let (width, height) = crate::video_h264::extract_video_dimensions(sample_entry)?;

        Ok(Some(VideoFrame {
            // TODO: タイムスタンプのラップアラウンドを考慮する
            timestamp: std::time::Duration::from_millis(frame.timestamp.as_millis() as u64),
            duration: std::time::Duration::ZERO, // TODO: ちゃんとする（遅延は少し増えるけど、次のフレームとの差分を取るとか）
            keyframe: frame.frame_type == shiguredo_rtmp::VideoFrameType::KeyFrame,
            sample_entry: Some(sample_entry.clone()),
            format: crate::video::VideoFormat::H264AnnexB,
            width: width as usize,
            height: height as usize,
            source_id: None, // TODO: ここは外側から指定すべき or URL の値を採用する
            data: frame.data,
        }))
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
            let sps_list = &avc1.avcc_box.sps_list;
            let pps_list = &avc1.avcc_box.pps_list;
            Ok(crate::video_h264::create_sequence_header_annexb(
                sps_list, pps_list,
            ))
        }
        _ => Err(orfail::Failure::new("Not an H.264 video sample entry")),
    }
}

/// Annex B 形式から NALU長プレフィックス形式に変換
pub fn convert_annexb_to_nalu(data: &[u8], nalu_length_size: u8) -> orfail::Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut offset = 0;
    let nalu_length_size = nalu_length_size as usize;

    while offset < data.len() {
        // Start code を探す
        let start = if offset + 4 <= data.len() && data[offset..offset + 4] == [0, 0, 0, 1] {
            offset += 4;
            offset
        } else if offset + 3 <= data.len() && data[offset..offset + 3] == [0, 0, 1] {
            offset += 3;
            offset
        } else {
            offset += 1;
            continue;
        };

        // 次の start code を探す
        let mut end = start;
        while end + 4 <= data.len() {
            if data[end..end + 4] == [0, 0, 0, 1] || data[end..end + 3] == [0, 0, 1] {
                break;
            }
            end += 1;
        }
        if end == start {
            end = data.len();
        }

        let nalu_data = &data[start..end];

        // NALU 長をプレフィックスとして追加
        let length = nalu_data.len() as u32;
        match nalu_length_size {
            1 => result.push(length as u8),
            2 => result.extend_from_slice(&(length as u16).to_be_bytes()),
            3 => result.extend_from_slice(&length.to_be_bytes()[1..]),
            4 => result.extend_from_slice(&length.to_be_bytes()),
            _ => return Err(orfail::Failure::new("Invalid NALU length size")),
        }

        result.extend_from_slice(nalu_data);
        offset = end;
    }

    Ok(result)
}
