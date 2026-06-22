use std::sync::Arc;

use shiguredo_mp4::boxes::SampleEntry;

use crate::{
    Error,
    audio::{AudioFrame, Channels, SampleRate},
    sample_entry::SharedSampleEntry,
    video::{VideoFrame, VideoFrameSize},
};

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
                let seq_header = create_audio_sequence_header(entry.get())?;
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
                self.video_nalu_length_size = extract_nalu_length_size(entry.get())?;

                let seq_header_data = create_video_sequence_header(entry.get())?;
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
                crate::video::h264::convert_annexb_to_nalu(&video.data, self.video_nalu_length_size)
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

    pub fn is_waiting_for_keyframe(&self) -> bool {
        !self.received_keyframe
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
    // RTMP の A/V は同一の u32 ミリ秒 timestamp 空間を共有するため、
    // wrap 補正も 1 つの mapper で統一して扱う。
    timestamp_mapper: crate::timestamp::mapper::TimestampMapper,
}

#[derive(Debug, Clone)]
struct AudioCodecInfo {
    format: crate::audio::AudioFormat,
    sample_rate: SampleRate,
    channels: Channels,
}

impl RtmpIncomingFrameHandler {
    pub fn new(timestamp_offset: std::time::Duration) -> crate::Result<Self> {
        Ok(Self {
            audio_codec_info: None,
            audio_sample_entry: None,
            video_sample_entry: None,
            received_video_keyframe: false,
            timestamp_mapper: crate::timestamp::mapper::TimestampMapper::new(
                32,
                1_000,
                timestamp_offset,
            )?,
        })
    }

    /// 受信した音声フレームを処理
    pub fn process_audio_frame(
        &mut self,
        frame: shiguredo_rtmp::AudioFrame,
    ) -> crate::Result<Option<AudioFrame>> {
        // シーケンスヘッダーの処理
        if frame.is_aac_sequence_header {
            // AAC シーケンスヘッダー（Audio Specific Config）をパース
            let (sample_rate, channels) =
                crate::audio::aac::parse_audio_specific_config(&frame.data)?;

            // SampleEntry を生成
            let sample_entry =
                crate::audio::aac::create_mp4a_sample_entry(&frame.data, sample_rate, channels)?;

            self.audio_codec_info = Some(AudioCodecInfo {
                format: crate::audio::AudioFormat::Aac,
                sample_rate,
                channels,
            });

            self.audio_sample_entry = Some(sample_entry);

            tracing::debug!(
                "Received AAC sequence header: {}Hz, {} channels",
                sample_rate.get(),
                channels.get()
            );
            return Ok(None);
        }

        // タイムスタンプを調整
        let timestamp = self
            .timestamp_mapper
            .map(u64::from(frame.timestamp.as_millis()));

        let codec_info = self
            .audio_codec_info
            .as_ref()
            .ok_or_else(|| Error::new("audio codec info is not initialized"))?;
        Ok(Some(AudioFrame {
            timestamp,
            format: codec_info.format,
            sample_rate: codec_info.sample_rate,
            channels: codec_info.channels,
            sample_entry: self.audio_sample_entry.clone().map(SharedSampleEntry::new),
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

            // SampleEntry と SPS 由来の解像度を取得
            let (sample_entry, frame_size) = avc_sequence_header_to_sample_entry(&seq_header)?;
            self.video_sample_entry = Some(sample_entry);

            tracing::debug!(
                "Received H.264 sequence header: {}x{}",
                frame_size.width,
                frame_size.height,
            );
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
        let current_timestamp = self
            .timestamp_mapper
            .map(u64::from(frame.timestamp.as_millis()));

        // サンプルエントリーを処理
        let sample_entry = self
            .video_sample_entry
            .as_ref()
            .ok_or_else(|| Error::new("video sample entry is not initialized"))?;

        Ok(Some(VideoFrame {
            timestamp: current_timestamp,
            keyframe: frame.frame_type == shiguredo_rtmp::VideoFrameType::KeyFrame,
            sample_entry: Some(SharedSampleEntry::new(sample_entry.clone())),
            format: crate::video::VideoFormat::H264,
            // RTMP inbound では payload を解析せずに H.264 を受け渡すため、
            // フレームサイズは常に未知扱いにする。
            size: None,
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

/// AvcSequenceHeader.sps_list / .pps_list を h264_sample_entry_from_sps_pps_lists に委譲する薄いラッパー。
///
/// 入力契約は h264_sample_entry_from_sps_pps_lists と同じ EBSP 形式 (NAL ヘッダ 1 バイト含む、start
/// code なし)。shiguredo_rtmp::AvcSequenceHeader::from_bytes が AVCDecoderConfigurationRecord から
/// 取り出す sps_list / pps_list の格納形式と一致するため、そのまま clone して渡す。
///
/// seq_header.avc_profile_indication / profile_compatibility / avc_level_indication /
/// length_size_minus_one は捨てて、すべて SPS 由来実値および固定値で avcC を埋める
/// (length_size_minus_one は Hisui 全体方針の NALU_HEADER_LENGTH = 4 バイト固定に揃える)。
fn avc_sequence_header_to_sample_entry(
    seq_header: &shiguredo_rtmp::AvcSequenceHeader,
) -> crate::Result<(SampleEntry, VideoFrameSize)> {
    crate::video::h264::h264_sample_entry_from_sps_pps_lists(
        seq_header.sps_list.clone(),
        seq_header.pps_list.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::h264::tests::{PPS_NAL, SPS_320X240};

    // SPS_320X240 の RBSP byte[0..3] は profile_idc=66 / constraint_set_flags=0xc0 /
    // level_idc=13。seq_header 側にはこれと異なる値を入れて、改修後の avcC が SPS 由来
    // 実値で埋まる (= seq_header 由来は捨てられる) ことを検証する。
    fn baseline_seq_header_with_dummy_avcc_fields() -> shiguredo_rtmp::AvcSequenceHeader {
        shiguredo_rtmp::AvcSequenceHeader {
            sps_list: vec![SPS_320X240.to_vec()],
            pps_list: vec![PPS_NAL.to_vec()],
            // SPS 由来 (66) と異なるダミー値を入れて、seq_header 由来が捨てられることを示す
            avc_profile_indication: 0xff,
            // SPS 由来 (0xc0) と異なるダミー値
            profile_compatibility: 0x00,
            // SPS 由来 (13) と異なるダミー値
            avc_level_indication: 0xff,
            // 改修後は seq_header.length_size_minus_one を読まずに 3 固定にするため、
            // ダミー値 0 を入れても avcC は 3 になる
            length_size_minus_one: 0,
        }
    }

    #[test]
    fn avc_sequence_header_to_sample_entry_maps_baseline_sps_to_avcc() {
        // Baseline SPS で薄いラッパー化後の avcC / visual / frame_size を検証する
        let seq_header = baseline_seq_header_with_dummy_avcc_fields();

        let (entry, frame_size) =
            avc_sequence_header_to_sample_entry(&seq_header).expect("Baseline 反映が成功すること");

        let SampleEntry::Avc1(b) = entry else {
            panic!("Avc1 SampleEntry を期待したが他の variant が返った: {entry:?}");
        };

        // avcC が SPS 由来実値で埋まり、seq_header 由来は捨てられている
        assert_eq!(b.avcc_box.avc_profile_indication, 66);
        assert_eq!(b.avcc_box.profile_compatibility, 0xc0);
        assert_eq!(b.avcc_box.avc_level_indication, 13);
        // length_size_minus_one は NALU_HEADER_LENGTH - 1 = 3 で固定
        assert_eq!(b.avcc_box.length_size_minus_one.get(), 3);
        // Baseline プロファイルでは chroma_format / bit_depth_* は None
        assert!(b.avcc_box.chroma_format.is_none());
        assert!(b.avcc_box.bit_depth_luma_minus8.is_none());
        assert!(b.avcc_box.bit_depth_chroma_minus8.is_none());
        // sps_list / pps_list が seq_header からクローンされて AvccBox に詰められる
        assert_eq!(b.avcc_box.sps_list, vec![SPS_320X240.to_vec()]);
        assert_eq!(b.avcc_box.pps_list, vec![PPS_NAL.to_vec()]);
        // visual.width / .height が SPS 由来実値で埋まる (0 ではない)
        assert_eq!(b.visual.width, 320);
        assert_eq!(b.visual.height, 240);
        // 戻り値タプルの VideoFrameSize も SPS 由来実値
        assert_eq!(frame_size.width, 320);
        assert_eq!(frame_size.height, 240);
    }

    #[test]
    fn avc_sequence_header_to_sample_entry_returns_err_on_empty_sps_list() {
        // sps_list 空のときは下層 h264_sample_entry_from_sps_pps_lists 由来の Err が伝播する
        let mut seq_header = baseline_seq_header_with_dummy_avcc_fields();
        seq_header.sps_list = vec![];

        let result = avc_sequence_header_to_sample_entry(&seq_header);

        let err = result.expect_err("sps_list 空は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("missing H.264 SPS"),
            "想定外のエラーメッセージ: {display}",
        );
    }

    #[test]
    fn avc_sequence_header_to_sample_entry_returns_err_on_empty_pps_list() {
        // pps_list 空のときは下層 h264_sample_entry_from_sps_pps_lists 由来の Err が伝播する
        let mut seq_header = baseline_seq_header_with_dummy_avcc_fields();
        seq_header.pps_list = vec![];

        let result = avc_sequence_header_to_sample_entry(&seq_header);

        let err = result.expect_err("pps_list 空は Err を返すこと");
        let display = format!("{err:?}");
        assert!(
            display.contains("missing H.264 PPS"),
            "想定外のエラーメッセージ: {display}",
        );
    }
}
