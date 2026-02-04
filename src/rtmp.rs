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
    video_nalu_length_size: u8,
    received_keyframe: bool,
    stats: RtmpOutgoingFrameHandlerStats,
}

impl RtmpOutgoingFrameHandler {
    pub fn new(stats: RtmpOutgoingFrameHandlerStats) -> Self {
        Self {
            video_sequence_header_data: None,
            audio_sequence_header_data: None,
            video_nalu_length_size: 4, // 後でちゃんとした値で更新されるが、最初は典型的な値を設定しておく
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
                self.video_nalu_length_size = extract_nalu_length_size(entry)?;

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

        // 映像フレームデータをRTMP形式（AVC 形式）で処理
        let frame_data = match video.format {
            crate::video::VideoFormat::H264 => {
                // もともと AVC 形式の場合は変換は不要
                video.data.clone()
            }
            crate::video::VideoFormat::H264AnnexB => {
                // Annex B 形式（開始コード付き）から AVC 形式に変換が必要
                crate::video_h264::convert_annexb_to_nalu(&video.data, self.video_nalu_length_size)?
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

/// RTMP フレーム処理の共通ロジック（受信側）
#[derive(Debug)]
pub struct RtmpIncomingFrameHandler {
    audio_codec_info: Option<AudioCodecInfo>,
    audio_sample_entry: Option<SampleEntry>,
    video_sample_entry: Option<SampleEntry>,
    received_video_keyframe: bool,
    stats: RtmpIncomingFrameHandlerStats,
    last_video_timestamp: Option<std::time::Duration>,
    rtmp_base_timestamp: Option<u32>,
    timestamp_offset: std::time::Duration,
}

#[derive(Debug, Clone)]
struct AudioCodecInfo {
    format: crate::audio::AudioFormat,
    sample_rate: u32,
    channels: u8,
}

impl RtmpIncomingFrameHandler {
    pub fn new(
        timestamp_offset: std::time::Duration,
        stats: RtmpIncomingFrameHandlerStats,
    ) -> Self {
        Self {
            audio_codec_info: None,
            audio_sample_entry: None,
            video_sample_entry: None,
            received_video_keyframe: false,
            stats,
            last_video_timestamp: None,
            rtmp_base_timestamp: None,
            timestamp_offset,
        }
    }

    /// タイムスタンプを調整する
    /// 計算式: `RTMP timestamp - RTMP base timestamp + offset timestamp`
    fn adjust_timestamp(&mut self, rtmp_timestamp_ms: u64) -> u64 {
        let rtmp_ts = rtmp_timestamp_ms as u32;

        // 最初のタイムスタンプを基準値として記録
        if self.rtmp_base_timestamp.is_none() {
            self.rtmp_base_timestamp = Some(rtmp_ts);
        }

        let base = self.rtmp_base_timestamp.unwrap();
        // RTMP timestamp - RTMP base timestamp + offset timestamp
        (rtmp_ts as i64 - base as i64) as u64 + self.timestamp_offset.as_millis() as u64
    }

    /// 受信した音声フレームを処理
    pub fn process_audio_frame(
        &mut self,
        frame: shiguredo_rtmp::AudioFrame,
    ) -> orfail::Result<AudioData> {
        // シーケンスヘッダーの処理
        if frame.is_aac_sequence_header {
            self.stats.total_audio_sequence_header_count.increment();

            // AAC シーケンスヘッダー（Audio Specific Config）をパース
            let (sample_rate, channels) = parse_aac_audio_specific_config(&frame.data)?;

            // SampleEntry を生成
            let sample_entry = create_audio_sample_entry(&frame.data, sample_rate, channels)?;

            self.audio_codec_info = Some(AudioCodecInfo {
                format: crate::audio::AudioFormat::Aac,
                sample_rate,
                channels,
            });

            self.audio_sample_entry = Some(sample_entry);

            log::debug!(
                "Received AAC sequence header: {}Hz, {} channels",
                sample_rate,
                channels
            );
        }

        self.stats.total_audio_frame_count.increment();

        // タイムスタンプを調整
        let adjusted_timestamp_ms = self.adjust_timestamp(frame.timestamp.as_millis() as u64);

        let codec_info = self.audio_codec_info.as_ref().or_fail()?;

        Ok(AudioData {
            timestamp: std::time::Duration::from_millis(adjusted_timestamp_ms),
            duration: std::time::Duration::ZERO,
            format: codec_info.format,
            sample_rate: codec_info.sample_rate as u16,
            stereo: codec_info.channels == 2,
            sample_entry: self.audio_sample_entry.clone(),
            data: frame.data,
            source_id: None,
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

            // Use shiguredo_rtmp::AvcSequenceHeader directly
            let seq_header =
                shiguredo_rtmp::AvcSequenceHeader::from_bytes(&frame.data).or_fail()?;

            // いったん解像度は 0 扱いにしておく
            // TODO: SPS から実際の width, height を抽出
            let width = 0;
            let height = 0;

            // SampleEntry を生成
            let sample_entry = avc_sequence_header_to_sample_entry(&seq_header, width, height)?;
            self.video_sample_entry = Some(sample_entry);

            log::debug!("Received H.264 sequence header: {}x{}", width, height);
            return Ok(None);
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

        // タイムスタンプを調整
        let adjusted_timestamp_ms = self.adjust_timestamp(frame.timestamp.as_millis() as u64);
        let current_timestamp = std::time::Duration::from_millis(adjusted_timestamp_ms);

        // サンプルエントリーを処理
        let sample_entry = self.video_sample_entry.as_ref().or_fail()?;

        // サンプルエントリーから解像度を取得
        let (width, height) =
            crate::video_h264::extract_video_dimensions(sample_entry).or_fail()?;

        // durationを計算
        //
        // TODO: 将来的には VideoFrame 側で「duration が不明」を表現できるようにする
        let duration = if let Some(last_ts) = self.last_video_timestamp {
            if current_timestamp > last_ts {
                current_timestamp - last_ts
            } else {
                // タイムスタンプがリセットされた場合は20msのデフォルト値
                std::time::Duration::from_millis(20)
            }
        } else {
            // 最初のフレームは20msで決め打ち
            std::time::Duration::from_millis(20)
        };

        // 次のフレーム処理用に現在のタイムスタンプを記録
        self.last_video_timestamp = Some(current_timestamp);

        Ok(Some(VideoFrame {
            timestamp: current_timestamp,
            duration,
            keyframe: frame.frame_type == shiguredo_rtmp::VideoFrameType::KeyFrame,
            sample_entry: Some(sample_entry.clone()),
            format: crate::video::VideoFormat::H264,
            width: width as usize,
            height: height as usize,
            source_id: None,
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

/// AAC Audio Specific Config をパースしてサンプルレートとチャンネル数を取得
fn parse_aac_audio_specific_config(data: &[u8]) -> orfail::Result<(u32, u8)> {
    (data.len() >= 2).or_fail()?;

    let byte0 = data[0];
    let byte1 = data[1];

    // Audio Object Type (5 bits): byte0 >> 3
    // Sampling Frequency Index (4 bits): ((byte0 & 0x07) << 1) | (byte1 >> 7)
    let sample_rate_index = ((byte0 & 0x07) << 1) | (byte1 >> 7);

    // Channel Configuration (4 bits): (byte1 >> 3) & 0x0F
    let channels = (byte1 >> 3) & 0x0F;

    let sample_rate = match sample_rate_index {
        0 => 96000,
        1 => 88200,
        2 => 64000,
        3 => 48000,
        4 => 44100,
        5 => 32000,
        6 => 24000,
        7 => 22050,
        8 => 16000,
        9 => 12000,
        10 => 11025,
        11 => 8000,
        12 => 7350,
        _ => return Err(orfail::Failure::new("Invalid AAC sample rate index")),
    };

    let num_channels = match channels {
        0 => {
            return Err(orfail::Failure::new(
                "AAC channel configuration 0 is invalid",
            ));
        }
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 5,
        6 => 6,
        7 => 8,
        _ => return Err(orfail::Failure::new("Invalid AAC channel configuration")),
    };

    Ok((sample_rate, num_channels as u8))
}

/// AAC SampleEntry を生成
fn create_audio_sample_entry(
    audio_specific_config: &[u8],
    sample_rate: u32,
    channels: u8,
) -> orfail::Result<SampleEntry> {
    use shiguredo_mp4::{
        FixedPointNumber, Uint,
        boxes::{AudioSampleEntryFields, EsdsBox, Mp4aBox, SampleEntry},
        descriptors::{DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor},
    };

    Ok(SampleEntry::Mp4a(Mp4aBox {
        audio: AudioSampleEntryFields {
            data_reference_index: AudioSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
            channelcount: channels as u16,
            samplesize: 16,
            samplerate: FixedPointNumber::new(sample_rate as u16, 0),
        },
        esds_box: EsdsBox {
            es: EsDescriptor {
                es_id: EsDescriptor::MIN_ES_ID,
                stream_priority: EsDescriptor::LOWEST_STREAM_PRIORITY,
                depends_on_es_id: None,
                url_string: None,
                ocr_es_id: None,
                dec_config_descr: DecoderConfigDescriptor {
                    object_type_indication:
                        DecoderConfigDescriptor::OBJECT_TYPE_INDICATION_AUDIO_ISO_IEC_14496_3,
                    stream_type: DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
                    up_stream: DecoderConfigDescriptor::UP_STREAM_FALSE,
                    dec_specific_info: Some(DecoderSpecificInfo {
                        payload: audio_specific_config.to_vec(),
                    }),
                    buffer_size_db: Uint::new(65536),
                    max_bitrate: 256000,
                    avg_bitrate: 128000,
                },
                sl_config_descr: shiguredo_mp4::descriptors::SlConfigDescriptor,
            },
        },
        unknown_boxes: Vec::new(),
    }))
}

/// AvcSequenceHeader から SampleEntry を生成（RTMP 受信用）
fn avc_sequence_header_to_sample_entry(
    seq_header: &shiguredo_rtmp::AvcSequenceHeader,
    width: usize,
    height: usize,
) -> orfail::Result<SampleEntry> {
    use shiguredo_mp4::{Uint, boxes::Avc1Box, boxes::AvccBox};

    Ok(SampleEntry::Avc1(Avc1Box {
        visual: crate::video::sample_entry_visual_fields(width, height),
        avcc_box: AvccBox {
            sps_list: seq_header.sps_list.clone(),
            pps_list: seq_header.pps_list.clone(),
            avc_profile_indication: seq_header.avc_profile_indication,
            avc_level_indication: seq_header.avc_level_indication,
            profile_compatibility: seq_header.profile_compatibility,
            length_size_minus_one: Uint::new(seq_header.length_size_minus_one),
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext_list: Vec::new(),
        },
        unknown_boxes: Vec::new(),
    }))
}
