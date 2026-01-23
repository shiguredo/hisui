use orfail::OrFail;
use shiguredo_mp4::boxes::SampleEntry;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    audio::{AudioData, AudioFormat},
    media::{MediaSample, MediaStreamId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    stats::ProcessorStats,
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug, Clone)]
pub struct RtmpStreamUrl {
    pub host: String,
    pub port: u16,
    pub app: String,
    pub stream_name: String,
    pub tls: bool,
}

impl std::fmt::Display for RtmpStreamUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let scheme = if self.tls { "rtmps" } else { "rtmp" };
        write!(
            f,
            "{}://{}:{}/{}/{}",
            scheme, self.host, self.port, self.app, self.stream_name
        )
    }
}

// TODO: impl Parse

#[derive(Debug)]
pub struct RtmpPublisher {
    input_audio_stream_id: Option<MediaStreamId>,
    input_video_stream_id: Option<MediaStreamId>,
    tx: Option<tokio::sync::mpsc::Sender<MediaSample>>,
}

impl RtmpPublisher {
    pub fn start(
        runtime: &tokio::runtime::Runtime,
        input_audio_stream_id: Option<MediaStreamId>,
        input_video_stream_id: Option<MediaStreamId>,
        url: RtmpStreamUrl,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(100); // TODO: サイズは変更できるようにする
        runtime.spawn(async move {
            let connection = shiguredo_rtmp::RtmpPublishClientConnection::new(
                &url.to_string(),
                &url.stream_name,
            );
            let runner = RtmpPublishRunner {
                url,
                rx,
                recv_buf: vec![0u8; 8192],
                connection,
                ready: false,
                video_sequence_header_data: None,
                audio_sequence_header_data: None,
                last_video_keyframe_timestamp: None,
            };
            if let Err(e) = runner.run().await.or_fail() {
                log::error!("RTMP publish error: {e}");
                // TODO: stats 更新
            }
        });
        Self {
            input_audio_stream_id,
            input_video_stream_id,
            tx: Some(tx),
        }
    }
}

impl MediaProcessor for RtmpPublisher {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self
                .input_audio_stream_id
                .into_iter()
                .chain(self.input_video_stream_id)
                .collect(),
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::other("rtmp-publisher"), // TODO: 専用の構造体を用意する
            workload_hint: MediaProcessorWorkloadHint::WRITER, // TODO: 非同期 I/O 用の値を追加する
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        match input.sample {
            Some(MediaSample::Audio(sample))
                if Some(input.stream_id) == self.input_audio_stream_id =>
            {
                // 音声は AAC のみ許可する
                (sample.format == AudioFormat::Aac)
                    .or_fail_with(|()| format!("unsupported audio codec: {}", sample.format))?;

                let tx = self.tx.as_ref().or_fail()?;

                // TODO: ちゃんとエラーハンドリングする（一時的に詰まっているだけならエラーにしない）
                tx.try_send(MediaSample::Audio(sample)).or_fail()?;
            }
            None if Some(input.stream_id) == self.input_audio_stream_id => {
                self.input_audio_stream_id = None;
            }
            Some(MediaSample::Video(sample))
                if Some(input.stream_id) == self.input_video_stream_id =>
            {
                // 映像は H.264 （AVC or Annex.B 形式） のみ許可する
                matches!(sample.format, VideoFormat::H264 | VideoFormat::H264AnnexB)
                    .or_fail_with(|()| format!("unsupported video codec: {}", sample.format))?;

                let tx = self.tx.as_ref().or_fail()?;

                // TODO: ちゃんとエラーハンドリングする（一時的に詰まっているだけならエラーにしない）
                tx.try_send(MediaSample::Video(sample)).or_fail()?;
            }
            None if Some(input.stream_id) == self.input_video_stream_id => {
                self.input_video_stream_id = None;
            }
            _ => return Err(orfail::Failure::new("BUG: unexpected input stream")),
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        // TODO: ネットワークが詰まっている場合には、それを前段にフィードバックする

        if self.input_audio_stream_id.is_some() || self.input_video_stream_id.is_some() {
            Ok(MediaProcessorOutput::awaiting_any())
        } else {
            self.tx = None;
            Ok(MediaProcessorOutput::Finished)
        }
    }
}

#[derive(Debug)]
struct RtmpPublishRunner {
    url: RtmpStreamUrl,
    rx: tokio::sync::mpsc::Receiver<MediaSample>,
    recv_buf: Vec<u8>,
    connection: shiguredo_rtmp::RtmpPublishClientConnection,
    ready: bool,
    video_sequence_header_data: Option<Vec<u8>>,
    audio_sequence_header_data: Option<Vec<u8>>,
    last_video_keyframe_timestamp: Option<u32>,
}

impl RtmpPublishRunner {
    async fn run(mut self) -> orfail::Result<()> {
        let tc_url = self.url.to_string();
        log::debug!("Starting RTMP publisher: {tc_url}");

        // TCP または TLS 接続を確立
        let mut stream =
            crate::tcp::TcpOrTlsStream::connect(&self.url.host, self.url.port, self.url.tls)
                .await
                .or_fail()?;

        // イベント処理ループ
        loop {
            // イベント処理
            while let Some(event) = self.connection.next_event() {
                log::debug!("RTMP event: {:?}", event);
                if matches!(
                    event,
                    shiguredo_rtmp::RtmpConnectionEvent::StateChanged(
                        shiguredo_rtmp::RtmpConnectionState::Publishing
                    )
                ) {
                    self.ready = true;
                }
            }

            // 送信バッファをストリームに書き込む
            while !self.connection.send_buf().is_empty() {
                let send_data = self.connection.send_buf();
                stream.write_all(send_data).await.or_fail()?;
                self.connection.advance_send_buf(send_data.len());
            }

            // select! マクロでストリーム受信とメディアサンプル受信を並行処理
            tokio::select! {
                result = stream.read(&mut self.recv_buf) => {
                    // TCP / TLS ストリームからデータを受信
                    self.handle_read_result(result).or_fail()?;
                }
                Some(sample) = self.rx.recv(), if self.ready => {
                    // RTMP サーバーとの接続処理が完了したら、メディアサンプルを受信処理
                    self.handle_media_sample(sample).or_fail()?;
                }
                else => {
                    // 配信すべき入力サンプルがなくなった（正常終了）
                    break;
                }
            }
        }

        log::debug!("RTMP publisher finished");
        Ok(())
    }

    fn handle_read_result(&mut self, result: std::io::Result<usize>) -> orfail::Result<()> {
        let n = result.or_fail()?;

        // サーバーから切断されるのは想定外なのでエラー扱いにする
        (n > 0).or_fail_with(|()| "connection reset by server".to_owned())?;

        self.connection
            .feed_recv_buf(&self.recv_buf[..n])
            .or_fail()?;
        Ok(())
    }

    fn handle_media_sample(&mut self, sample: MediaSample) -> orfail::Result<()> {
        match sample {
            MediaSample::Audio(audio) => self.handle_audio_sample(audio),
            MediaSample::Video(video) => self.handle_video_sample(video),
        }
    }

    /// 音声サンプルを処理してサーバーに送信する
    fn handle_audio_sample(&mut self, audio: Arc<AudioData>) -> orfail::Result<()> {
        // サンプルエントリーから実際のメタデータを抽出
        let (sample_rate, is_stereo, is_8bit) = if let Some(entry) = &audio.sample_entry {
            extract_audio_params(entry)?
        } else {
            // フォールバック: audio フィールドから取得
            let sample_rate = hz_to_rtmp_sample_rate(audio.sample_rate);
            (sample_rate, audio.stereo, false)
        };

        // 最初のサンプルまたは新しいサンプルエントリーが来た場合、シーケンスヘッダを送信
        if self.audio_sequence_header_data.is_none() {
            if let Some(entry) = &audio.sample_entry {
                if let Ok(seq_header) = create_audio_sequence_header(entry) {
                    let seq_frame = shiguredo_rtmp::AudioFrame {
                        timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(0),
                        format: shiguredo_rtmp::AudioFormat::Aac,
                        sample_rate,
                        is_8bit_sample: is_8bit,
                        is_stereo,
                        is_aac_sequence_header: true,
                        data: seq_header.clone(),
                    };
                    self.connection.send_audio(seq_frame).or_fail()?;
                    self.audio_sequence_header_data = Some(seq_header);
                    log::debug!("Sent AAC sequence header");
                }
            }
        }

        // 音声フレームデータを送信
        let frame = shiguredo_rtmp::AudioFrame {
            timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(
                audio.timestamp.as_millis() as u32
            ),
            format: shiguredo_rtmp::AudioFormat::Aac,
            sample_rate,
            is_8bit_sample: is_8bit,
            is_stereo,
            is_aac_sequence_header: false,
            data: audio.data.clone(),
        };
        self.connection.send_audio(frame).or_fail()?;
        Ok(())
    }

    /// 映像サンプルを処理してサーバーに送信する
    fn handle_video_sample(&mut self, video: Arc<VideoFrame>) -> orfail::Result<()> {
        let timestamp_ms = video.timestamp.as_millis() as u32;

        // キーフレームの場合、シーケンスヘッダを送信
        if video.keyframe {
            // 最初のキーフレームまたは新しいサンプルエントリーが来た場合
            if let Some(entry) = &video.sample_entry {
                if self.video_sequence_header_data.is_none() {
                    if let Ok(seq_header_data) = create_video_sequence_header(entry) {
                        let seq_frame = shiguredo_rtmp::VideoFrame {
                            timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(timestamp_ms),
                            composition_timestamp_offset: shiguredo_rtmp::RtmpTimestampDelta::ZERO,
                            frame_type: shiguredo_rtmp::VideoFrameType::KeyFrame,
                            codec: shiguredo_rtmp::VideoCodec::Avc,
                            avc_packet_type: Some(shiguredo_rtmp::AvcPacketType::SequenceHeader),
                            data: seq_header_data.clone(),
                        };
                        self.connection.send_video(seq_frame).or_fail()?;
                        self.video_sequence_header_data = Some(seq_header_data);
                        log::debug!("Sent H.264 sequence header");
                    }
                } else if self
                    .last_video_keyframe_timestamp
                    .map(|ts| timestamp_ms.saturating_sub(ts) > 5000)
                    .unwrap_or(false)
                {
                    // 5秒ごとにシーケンスヘッダを再送（オプション）
                    if let Some(ref seq_header_data) = self.video_sequence_header_data {
                        let seq_frame = shiguredo_rtmp::VideoFrame {
                            timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(timestamp_ms),
                            composition_timestamp_offset: shiguredo_rtmp::RtmpTimestampDelta::ZERO,
                            frame_type: shiguredo_rtmp::VideoFrameType::KeyFrame,
                            codec: shiguredo_rtmp::VideoCodec::Avc,
                            avc_packet_type: Some(shiguredo_rtmp::AvcPacketType::SequenceHeader),
                            data: seq_header_data.clone(),
                        };
                        self.connection.send_video(seq_frame).or_fail()?;
                    }
                }
            }
            self.last_video_keyframe_timestamp = Some(timestamp_ms);
        }

        // 映像フレームデータを送信
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
            data: video.data.clone(),
        };
        self.connection.send_video(frame).or_fail()?;
        Ok(())
    }
}

/// Hz のサンプリングレートを RTMP の AudioSampleRate に変換
fn hz_to_rtmp_sample_rate(hz: u16) -> shiguredo_rtmp::AudioSampleRate {
    match hz {
        5500 => shiguredo_rtmp::AudioSampleRate::Khz5,
        11000 => shiguredo_rtmp::AudioSampleRate::Khz11,
        22000 => shiguredo_rtmp::AudioSampleRate::Khz22,
        44100 => shiguredo_rtmp::AudioSampleRate::Khz44,
        48000 => shiguredo_rtmp::AudioSampleRate::Khz44, // TODO: Check if Khz48 exists in shiguredo_rtmp
        _ => {
            log::warn!("Unsupported sample rate: {hz}Hz, defaulting to 44.1kHz");
            shiguredo_rtmp::AudioSampleRate::Khz44
        }
    }
}

/// サンプルエントリーから音声パラメーターを抽出
fn extract_audio_params(
    entry: &SampleEntry,
) -> orfail::Result<(shiguredo_rtmp::AudioSampleRate, bool, bool)> {
    match entry {
        SampleEntry::Mp4a(mp4a) => {
            let channel_count = mp4a.audio.channelcount as u8;
            let is_stereo = channel_count >= 2;
            let is_8bit = mp4a.audio.samplesize == 8;
            let sample_rate_hz = mp4a.audio.samplerate.integer;
            let sample_rate = hz_to_rtmp_sample_rate(sample_rate_hz);

            Ok((sample_rate, is_stereo, is_8bit))
        }
        _ => Err(orfail::Failure::new("Not an audio sample entry")),
    }
}

/// 音声シーケンスヘッダを作成する
fn create_audio_sequence_header(entry: &SampleEntry) -> orfail::Result<Vec<u8>> {
    match entry {
        SampleEntry::Mp4a(mp4a) => {
            // EsdsBox から DecoderSpecificInfo を取得
            mp4a.esds_box
                .es
                .dec_config_descr
                .dec_specific_info
                .as_ref()
                .map(|info| info.payload.clone())
                .ok_or_else(|| orfail::Failure::new("No decoder specific info available"))
        }
        _ => Err(orfail::Failure::new("Not an audio sample entry")),
    }
}

/// 映像シーケンスヘッダを作成する
fn create_video_sequence_header(entry: &SampleEntry) -> orfail::Result<Vec<u8>> {
    match entry {
        SampleEntry::Avc1(avc1) => {
            let sps_list = &avc1.avcc_box.sps_list;
            let pps_list = &avc1.avcc_box.pps_list;
            Ok(create_avc_sequence_header_annexb(sps_list, pps_list))
        }
        _ => Err(orfail::Failure::new("Not an H.264 video sample entry")),
    }
}

/// H.264 のシーケンスヘッダを Annex B 形式で作成する
fn create_avc_sequence_header_annexb(sps_list: &[Vec<u8>], pps_list: &[Vec<u8>]) -> Vec<u8> {
    let mut result = Vec::new();

    // 全ての SPS を追加
    for sps in sps_list {
        result.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        result.extend_from_slice(sps);
    }

    // 全ての PPS を追加
    for pps in pps_list {
        result.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        result.extend_from_slice(pps);
    }

    result
}
