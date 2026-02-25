use std::sync::Arc;

use shiguredo_mp4::boxes::SampleEntry;

use crate::{Error, audio::AudioFrame, video::VideoFrame};

/// RTMP フレーム処理の共通ロジック（送信側）
#[derive(Debug)]
pub struct RtmpOutgoingFrameHandler {
    video_sequence_header_data: Option<Vec<u8>>,
    audio_sequence_header_data: Option<Vec<u8>>,
    video_nalu_length_size: u8,
    received_keyframe: bool,
}

impl RtmpOutgoingFrameHandler {
    pub fn new() -> Self {
        Self {
            video_sequence_header_data: None,
            audio_sequence_header_data: None,
            video_nalu_length_size: 4, // 後でちゃんとした値で更新されるが、最初は典型的な値を設定しておく
            received_keyframe: false,
        }
    }

    /// 音声フレームを処理
    pub fn prepare_audio_frame(
        &mut self,
        frame: Arc<AudioFrame>,
    ) -> crate::Result<(
        Option<shiguredo_rtmp::AudioFrame>,
        shiguredo_rtmp::AudioFrame,
    )> {
        // シーケンスヘッダーが必要な場合は生成
        let seq_frame = if self.audio_sequence_header_data.is_none() {
            if let Some(entry) = &frame.sample_entry {
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
                tracing::debug!("Sent AAC sequence header");
                Some(frame)
            } else {
                None
            }
        } else {
            None
        };

        // 実フレームデータ
        let rtmp_frame = shiguredo_rtmp::AudioFrame {
            timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(
                frame.timestamp.as_millis() as u32
            ),
            format: shiguredo_rtmp::AudioFormat::Aac,
            sample_rate: shiguredo_rtmp::AudioFrame::AAC_SAMPLE_RATE,
            is_stereo: shiguredo_rtmp::AudioFrame::AAC_STEREO,
            is_8bit_sample: false,
            is_aac_sequence_header: false,
            data: frame.data.clone(),
        };

        Ok((seq_frame, rtmp_frame))
    }

    /// 映像フレームを処理
    pub fn prepare_video_frame(
        &mut self,
        video: Arc<VideoFrame>,
    ) -> crate::Result<
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
                tracing::debug!("Sent H.264 sequence header");
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
                crate::video_h264::convert_annexb_to_nalu(&video.data, self.video_nalu_length_size)
                    .map_err(|e| e.with_context("failed to convert Annex B to NALU"))?
            }
            _ => return Err(Error::new("unsupported video format")),
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

        Ok(Some((seq_frame, frame)))
    }
}

impl Default for RtmpOutgoingFrameHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// RTMP フレーム処理の共通ロジック（受信側）
#[derive(Debug)]
pub struct RtmpIncomingFrameHandler {
    audio_codec_info: Option<AudioCodecInfo>,
    audio_sample_entry: Option<SampleEntry>,
    video_sample_entry: Option<SampleEntry>,
    received_video_keyframe: bool,
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
    pub fn new(timestamp_offset: std::time::Duration) -> Self {
        Self {
            audio_codec_info: None,
            audio_sample_entry: None,
            video_sample_entry: None,
            received_video_keyframe: false,
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

        let base = self.rtmp_base_timestamp.unwrap_or(0);
        // RTMP timestamp - RTMP base timestamp + offset timestamp
        (rtmp_ts as i64 - base as i64) as u64 + self.timestamp_offset.as_millis() as u64
    }

    /// 受信した音声フレームを処理
    pub fn process_audio_frame(
        &mut self,
        frame: shiguredo_rtmp::AudioFrame,
    ) -> crate::Result<Option<AudioFrame>> {
        // シーケンスヘッダーの処理
        if frame.is_aac_sequence_header {
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

            tracing::debug!(
                "Received AAC sequence header: {}Hz, {} channels",
                sample_rate,
                channels
            );
            return Ok(None);
        }

        // タイムスタンプを調整
        let adjusted_timestamp_ms = self.adjust_timestamp(frame.timestamp.as_millis() as u64);

        let codec_info = self
            .audio_codec_info
            .as_ref()
            .ok_or_else(|| Error::new("audio codec info is not initialized"))?;

        Ok(Some(AudioFrame {
            timestamp: std::time::Duration::from_millis(adjusted_timestamp_ms),
            duration: std::time::Duration::ZERO,
            format: codec_info.format,
            sample_rate: codec_info.sample_rate as u16,
            stereo: codec_info.channels == 2,
            sample_entry: self.audio_sample_entry.clone(),
            data: frame.data,
        }))
    }

    /// 受信した映像フレームを処理
    pub fn process_video_frame(
        &mut self,
        frame: shiguredo_rtmp::VideoFrame,
    ) -> crate::Result<Option<VideoFrame>> {
        // シーケンスヘッダーの処理
        if frame.avc_packet_type == Some(shiguredo_rtmp::AvcPacketType::SequenceHeader) {
            let seq_header = shiguredo_rtmp::AvcSequenceHeader::from_bytes(&frame.data)
                .map_err(|e| Error::new(format!("failed to parse AVC sequence header: {e}")))?;

            // いったん解像度は 0 扱いにしておく
            // TODO: SPS から実際の width, height を抽出
            let width = 0;
            let height = 0;

            // SampleEntry を生成
            let sample_entry = avc_sequence_header_to_sample_entry(&seq_header, width, height)?;
            self.video_sample_entry = Some(sample_entry);

            tracing::debug!("Received H.264 sequence header: {}x{}", width, height);
            return Ok(None);
        }
        if frame.avc_packet_type == Some(shiguredo_rtmp::AvcPacketType::EndOfSequence) {
            tracing::debug!("Received H.264 end of sequence");
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
        }

        // タイムスタンプを調整
        let adjusted_timestamp_ms = self.adjust_timestamp(frame.timestamp.as_millis() as u64);
        let current_timestamp = std::time::Duration::from_millis(adjusted_timestamp_ms);

        // サンプルエントリーを処理
        let sample_entry = self
            .video_sample_entry
            .as_ref()
            .ok_or_else(|| Error::new("video sample entry is not initialized"))?;

        // サンプルエントリーから解像度を取得
        let (width, height) = crate::video_h264::extract_video_dimensions(sample_entry)
            .map_err(|e| e.with_context("failed to extract video dimensions"))?;

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
            data: frame.data,
        }))
    }
}

/// AVC1エントリーから nalu_length_size を抽出
fn extract_nalu_length_size(entry: &SampleEntry) -> crate::Result<u8> {
    match entry {
        SampleEntry::Avc1(avc1) => Ok(avc1.avcc_box.length_size_minus_one.get() + 1),
        _ => Err(Error::new("Not an H.264 video sample entry")),
    }
}

pub fn create_audio_sequence_header(entry: &SampleEntry) -> crate::Result<Vec<u8>> {
    match entry {
        SampleEntry::Mp4a(mp4a) => mp4a
            .esds_box
            .es
            .dec_config_descr
            .dec_specific_info
            .as_ref()
            .map(|info| info.payload.clone())
            .ok_or_else(|| Error::new("No decoder specific info available")),
        _ => Err(Error::new("Not an audio sample entry")),
    }
}

pub fn create_video_sequence_header(entry: &SampleEntry) -> crate::Result<Vec<u8>> {
    match entry {
        SampleEntry::Avc1(avc1) => {
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
                .map_err(|e| Error::new(format!("Failed to create AVC sequence header: {e}")))
        }
        _ => Err(Error::new("Not an H.264 video sample entry")),
    }
}

/// AAC Audio Specific Config をパースしてサンプルレートとチャンネル数を取得
fn parse_aac_audio_specific_config(data: &[u8]) -> crate::Result<(u32, u8)> {
    if data.len() < 2 {
        return Err(Error::new("AAC audio specific config is too short"));
    }

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
        _ => return Err(Error::new("Invalid AAC sample rate index")),
    };

    let num_channels = match channels {
        0 => return Err(Error::new("AAC channel configuration 0 is invalid")),
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 5,
        6 => 6,
        7 => 8,
        _ => return Err(Error::new("Invalid AAC channel configuration")),
    };

    Ok((sample_rate, num_channels as u8))
}

/// AAC SampleEntry を生成
fn create_audio_sample_entry(
    audio_specific_config: &[u8],
    sample_rate: u32,
    channels: u8,
) -> crate::Result<SampleEntry> {
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
) -> crate::Result<SampleEntry> {
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
