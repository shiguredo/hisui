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
    stats::{ProcessorStats, SharedAtomicCounter, SharedAtomicDuration, SharedAtomicFlag},
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct RtmpPublisher {
    input_audio_stream_id: Option<MediaStreamId>,
    input_video_stream_id: Option<MediaStreamId>,
    tx: Option<tokio::sync::mpsc::Sender<MediaSample>>,
    stats: RtmpPublisherStats,
}

impl RtmpPublisher {
    pub fn start(
        runtime: &tokio::runtime::Runtime,
        input_audio_stream_id: Option<MediaStreamId>,
        input_video_stream_id: Option<MediaStreamId>,
        url: shiguredo_rtmp::RtmpUrl,
    ) -> Self {
        let stats = RtmpPublisherStats::default();
        let (tx, rx) = tokio::sync::mpsc::channel(100); // TODO: サイズは変更できるようにする

        let connection = shiguredo_rtmp::RtmpPublishClientConnection::new(url.clone());
        let mut runner = RtmpPublishRunner {
            url,
            rx,
            recv_buf: vec![0u8; 8192],
            connection,
            ready: false,
            video_sequence_header_data: None,
            audio_sequence_header_data: None,
            last_video_keyframe_timestamp: None,
            video_nalu_length_size: 4,
            stats: stats.clone(),
        };
        runtime.spawn(async move {
            if let Err(e) = runner.run().await.or_fail() {
                log::error!("RTMP publish error: {e}");
                runner.stats.error.set(true);
            }
        });
        Self {
            input_audio_stream_id,
            input_video_stream_id,
            tx: Some(tx),
            stats,
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
            stats: ProcessorStats::RtmpPublisher(self.stats.clone()),
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
    url: shiguredo_rtmp::RtmpUrl,
    rx: tokio::sync::mpsc::Receiver<MediaSample>,
    recv_buf: Vec<u8>,
    connection: shiguredo_rtmp::RtmpPublishClientConnection,
    ready: bool,
    video_sequence_header_data: Option<Vec<u8>>,
    audio_sequence_header_data: Option<Vec<u8>>,
    last_video_keyframe_timestamp: Option<u32>,
    video_nalu_length_size: u8,
    stats: RtmpPublisherStats,
}

impl RtmpPublishRunner {
    async fn run(&mut self) -> orfail::Result<()> {
        log::debug!("Starting RTMP publisher: {}", self.url);

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
                self.stats.total_event_count.increment();
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
                self.stats.total_sent_bytes.add(send_data.len() as u64);
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

        self.stats.total_received_bytes.add(n as u64);
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
        // 最初のサンプルまたは新しいサンプルエントリーが来た場合、シーケンスヘッダを送信
        if self.audio_sequence_header_data.is_none()
            && let Some(entry) = &audio.sample_entry
            && let Ok(seq_header) = create_audio_sequence_header(entry)
        {
            let seq_frame = shiguredo_rtmp::AudioFrame {
                timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(0),
                format: shiguredo_rtmp::AudioFormat::Aac,
                sample_rate: shiguredo_rtmp::AudioFrame::AAC_SAMPLE_RATE,
                is_stereo: shiguredo_rtmp::AudioFrame::AAC_STEREO,
                is_8bit_sample: false, // Hisui は 16 bit サンプル前提
                is_aac_sequence_header: true,
                data: seq_header.clone(),
            };
            self.connection.send_audio(seq_frame).or_fail()?;
            self.audio_sequence_header_data = Some(seq_header);
            self.stats.total_audio_sequence_header_count.increment();
            log::debug!("Sent AAC sequence header");
        }

        // 音声フレームデータを送信
        let frame = shiguredo_rtmp::AudioFrame {
            timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(
                audio.timestamp.as_millis() as u32
            ),
            format: shiguredo_rtmp::AudioFormat::Aac,
            sample_rate: shiguredo_rtmp::AudioFrame::AAC_SAMPLE_RATE,
            is_stereo: shiguredo_rtmp::AudioFrame::AAC_STEREO,
            is_8bit_sample: false, // Hisui は 16 bit サンプル前提
            is_aac_sequence_header: false,
            data: audio.data.clone(),
        };
        self.connection.send_audio(frame).or_fail()?;
        self.stats.total_audio_frame_count.increment();
        Ok(())
    }

    /// 映像サンプルを処理してサーバーに送信する
    fn handle_video_sample(&mut self, video: Arc<VideoFrame>) -> orfail::Result<()> {
        let timestamp_ms = video.timestamp.as_millis() as u32;

        // キーフレームの場合、シーケンスヘッダを送信
        if video.keyframe {
            self.stats.total_video_keyframe_count.increment();

            // 新しいサンプルエントリーが来た場合
            if let Some(entry) = &video.sample_entry
                && let Ok(seq_header_data) = create_video_sequence_header(entry)
            {
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
                self.stats.total_video_sequence_header_count.increment();
                log::debug!("Sent H.264 sequence header");
            }
            self.last_video_keyframe_timestamp = Some(timestamp_ms);
        }

        // 映像フレームデータを送信
        // Annex B 形式に変換する（必要に応じて）
        let frame_data = match video.format {
            VideoFormat::H264 => {
                // MP4形式の場合はAnnex Bに変換
                convert_nalu_to_annexb(&video.data, self.video_nalu_length_size)
            }
            VideoFormat::H264AnnexB => {
                // 既にAnnex B形式の場合は変換不要
                video.data.clone()
            }
            _ => {
                return Err(orfail::Failure::new(
                    "BUG: unsupported video format in handle_video_sample",
                ));
            }
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
        self.connection.send_video(frame).or_fail()?;
        self.stats.total_video_frame_count.increment();
        Ok(())
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

/// MP4 ファイルの H.264 映像フレームの形式を RTMP がサポートしている Annex B 形式に変換する
fn convert_nalu_to_annexb(data: &[u8], length_size: u8) -> Vec<u8> {
    let mut result = Vec::new();
    let mut offset = 0;
    let length_size = length_size as usize;

    while offset < data.len() {
        if offset + length_size > data.len() {
            break;
        }

        // MP4 ファイル形式で H.264 の NALU 長を読み取る
        let length = match length_size {
            1 => data[offset] as usize,
            2 => u16::from_be_bytes([data[offset], data[offset + 1]]) as usize,
            3 => u32::from_be_bytes([0, data[offset], data[offset + 1], data[offset + 2]]) as usize,
            4 => u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize,
            _ => {
                unreachable!() // MP4 ライブラリがチェックしているのでここには来ないはず
            }
        };

        offset += length_size;

        if offset + length > data.len() {
            break;
        }

        // Annex B の形式（先頭に固定の区切りバイト列が付与される）に変換する
        result.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        result.extend_from_slice(&data[offset..offset + length]);

        offset += length;
    }

    result
}

/// [`RtmpPublisher`] 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct RtmpPublisherStats {
    /// 配信した音声フレームの数
    pub total_audio_frame_count: SharedAtomicCounter,

    /// 配信した映像フレームの数
    pub total_video_frame_count: SharedAtomicCounter,

    /// RTMP イベント処理の回数
    pub total_event_count: SharedAtomicCounter,

    /// RTMP で送信したバイト数
    pub total_sent_bytes: SharedAtomicCounter,

    /// RTMP で受信したバイト数
    pub total_received_bytes: SharedAtomicCounter,

    /// 配信したキーフレーム（映像）の数
    pub total_video_keyframe_count: SharedAtomicCounter,

    /// 送信した音声シーケンスヘッダの数
    pub total_audio_sequence_header_count: SharedAtomicCounter,

    /// 送信した映像シーケンスヘッダの数
    pub total_video_sequence_header_count: SharedAtomicCounter,

    /// 処理に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for RtmpPublisherStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "rtmp_publisher")?;
            f.member("total_audio_frame_count", &self.total_audio_frame_count)?;
            f.member("total_video_frame_count", &self.total_video_frame_count)?;
            f.member("total_event_count", &self.total_event_count)?;
            f.member("total_sent_bytes", &self.total_sent_bytes)?;
            f.member("total_received_bytes", &self.total_received_bytes)?;
            f.member(
                "total_video_keyframe_count",
                &self.total_video_keyframe_count,
            )?;
            f.member(
                "total_audio_sequence_header_count",
                &self.total_audio_sequence_header_count,
            )?;
            f.member(
                "total_video_sequence_header_count",
                &self.total_video_sequence_header_count,
            )?;
            f.member("total_processing_seconds", &self.total_processing_duration)?;
            f.member("error", self.error.get())?;
            Ok(())
        })
    }
}
