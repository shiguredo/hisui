use orfail::OrFail;
use shiguredo_mp4::boxes::SampleEntry;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

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

/// メディアフレーム用チャネルサイズ
const FRAME_CHANNEL_SIZE: usize = 100;

#[derive(Debug, Default, Clone)]
pub struct RtmpOutboundEndpointOptions {}

/// RTMP Play Server - クライアントからの play リクエストを受け付けてメディアストリームを配信する
#[derive(Debug)]
pub struct RtmpOutboundEndpoint {
    input_audio_stream_id: Option<MediaStreamId>,
    input_video_stream_id: Option<MediaStreamId>,
    tx: Option<tokio::sync::mpsc::Sender<MediaSample>>,
    stats: RtmpOutboundEndpointStats,
}

impl RtmpOutboundEndpoint {
    pub fn start(
        runtime: &tokio::runtime::Runtime,
        input_audio_stream_id: Option<MediaStreamId>,
        input_video_stream_id: Option<MediaStreamId>,
        url: shiguredo_rtmp::RtmpUrl,
        _options: RtmpOutboundEndpointOptions,
    ) -> Self {
        let stats = RtmpOutboundEndpointStats::default();
        let (tx, rx) = tokio::sync::mpsc::channel(FRAME_CHANNEL_SIZE);

        let stats_clone = stats.clone();

        runtime.spawn(async move {
            let mut server = RtmpPlayServer {
                url: url.clone(),
                listener: None,
                rx,
                media_streams: Arc::new(Mutex::new(HashMap::new())),
                stats: stats_clone.clone(),
            };

            if let Err(e) = server.run().await.or_fail() {
                log::error!("RTMP play server error: {e}");
                server.stats.error.set(true);
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

impl MediaProcessor for RtmpOutboundEndpoint {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self
                .input_audio_stream_id
                .into_iter()
                .chain(self.input_video_stream_id)
                .collect(),
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::RtmpOutboundEndpoint(self.stats.clone()),
            workload_hint: MediaProcessorWorkloadHint::ASYNC_IO,
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
                tx.try_send(MediaSample::Audio(sample))
                    .or_fail_with(|e| format!("failed to send audio frame: {e}"))?;
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
                tx.try_send(MediaSample::Video(sample))
                    .or_fail_with(|e| format!("failed to send video frame: {e}"))?;
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

/// RTMP Play Server の実装
#[derive(Debug)]
struct RtmpPlayServer {
    url: shiguredo_rtmp::RtmpUrl,
    listener: Option<TcpListener>,
    rx: tokio::sync::mpsc::Receiver<MediaSample>,
    /// ストリーム ID -> ストリーム状態のマップ
    media_streams: Arc<Mutex<HashMap<String, MediaStreamState>>>,
    stats: RtmpOutboundEndpointStats,
}

impl RtmpPlayServer {
    async fn run(&mut self) -> orfail::Result<()> {
        log::debug!(
            "Starting RTMP play server on {}:{}",
            self.url.host,
            self.url.port
        );

        // TCP リスナーをバインドする
        let addr = format!("{}:{}", self.url.host, self.url.port);
        let listener = TcpListener::bind(&addr).await.or_fail()?;
        self.listener = Some(listener);

        let listener = self.listener.as_ref().or_fail()?;

        // メインイベントループ
        loop {
            tokio::select! {
                // 新しいクライアント接続を受け付ける
                accept_result = listener.accept() => {
                    let (stream, peer_addr) = accept_result.or_fail()?;
                    log::debug!("New RTMP client connection from: {}", peer_addr);

                    let media_streams = self.media_streams.clone();
                    let stats = self.stats.clone();

                    // クライアント接続を別タスクで処理する
                    tokio::spawn(async move {
                        let mut handler = RtmpClientHandler {
                            stream,
                            connection: shiguredo_rtmp::RtmpServerConnection::new(),
                            media_streams,
                            recv_buf: vec![0u8; 4096],
                            stream_id: String::new(),
                            received_keyframe: false,
                            stats,
                        };

                        if let Err(e) = handler.run().await.or_fail() {
                            log::error!("RTMP client handler error: {e}");
                        }
                        log::debug!("RTMP client disconnected: {}", peer_addr);
                    });
                }

                // 上流からメディアサンプルを受信する
                Some(sample) = self.rx.recv() => {
                    self.handle_media_sample(sample).await.or_fail()?;
                }
                else => {
                    // すべての入力が終了した
                    break;
                }
            }
        }

        log::debug!("RTMP play server finished");
        Ok(())
    }

    /// メディアサンプルを受け取り、すべてのプレイヤーに配信する
    async fn handle_media_sample(&self, sample: MediaSample) -> orfail::Result<()> {
        let mut streams = self.media_streams.lock().await;

        match sample {
            MediaSample::Audio(audio) => {
                // すべてのストリームの状態を更新し、プレイヤーに配信する
                for stream_state in streams.values_mut() {
                    // シーケンスヘッダーを保存（初回のみ）
                    if stream_state.audio_sequence_header.is_none() {
                        stream_state.audio_sequence_header = Some(audio.clone());
                    }

                    for player in stream_state.players.values_mut() {
                        // 初回接続時はシーケンスヘッダーを先に送信
                        if !player.audio_sequence_header_sent {
                            if let Some(seq_header) = &stream_state.audio_sequence_header {
                                let _ = player
                                    .tx
                                    .send(ClientMediaFrame::Audio(seq_header.clone()))
                                    .await;
                                player.audio_sequence_header_sent = true;
                            }
                        }
                        let _ = player.tx.send(ClientMediaFrame::Audio(audio.clone())).await;
                    }
                }
            }
            MediaSample::Video(video) => {
                // すべてのストリームの状態を更新し、プレイヤーに配信する
                for stream_state in streams.values_mut() {
                    // キーフレームのシーケンスヘッダーを保存
                    if video.keyframe {
                        stream_state.video_sequence_header = Some(video.clone());
                    }

                    for player in stream_state.players.values_mut() {
                        // シーケンスヘッダーを送信していない場合は先に送信
                        if !player.video_sequence_header_sent {
                            if let Some(seq_header) = &stream_state.video_sequence_header {
                                let _ = player
                                    .tx
                                    .send(ClientMediaFrame::Video(seq_header.clone()))
                                    .await;
                                player.video_sequence_header_sent = true;
                            }
                        }

                        // キーフレームを待っている場合
                        if !player.video_keyframe_sent {
                            if !video.keyframe {
                                continue; // キーフレームが来るまでスキップ
                            }
                            player.video_keyframe_sent = true;
                        }

                        let _ = player.tx.send(ClientMediaFrame::Video(video.clone())).await;
                    }
                }
            }
        }

        Ok(())
    }
}

/// 配信されているメディアストリームの状態
#[derive(Debug, Default)]
struct MediaStreamState {
    /// このストリームの配信者のクライアント ID
    publisher_id: Option<usize>,
    /// 音声シーケンスヘッダーデータ
    audio_sequence_header: Option<Arc<AudioData>>,
    /// 映像シーケンスヘッダーデータ（キーフレーム）
    video_sequence_header: Option<Arc<VideoFrame>>,
    /// このストリームをプレイしているクライアント
    players: HashMap<usize, PlayerState>,
}

/// ストリームをプレイしている個別クライアントの状態
#[derive(Debug)]
struct PlayerState {
    /// 音声シーケンスヘッダーが送信されたかどうか
    audio_sequence_header_sent: bool,
    /// 映像シーケンスヘッダーが送信されたかどうか
    video_sequence_header_sent: bool,
    /// キーフレームが送信されたかどうか
    video_keyframe_sent: bool,
    /// このクライアントにメディアフレームを送信するチャネル
    tx: tokio::sync::mpsc::Sender<ClientMediaFrame>,
}

/// クライアント配信用の内部メディアフレーム表現
#[derive(Debug, Clone)]
enum ClientMediaFrame {
    Audio(Arc<AudioData>),
    Video(Arc<VideoFrame>),
}

/// 個別のクライアント接続を処理する
#[derive(Debug)]
struct RtmpClientHandler {
    stream: TcpStream,
    connection: shiguredo_rtmp::RtmpServerConnection,
    media_streams: Arc<Mutex<HashMap<String, MediaStreamState>>>,
    recv_buf: Vec<u8>,
    stream_id: String,
    received_keyframe: bool,
    stats: RtmpOutboundEndpointStats,
}

impl RtmpClientHandler {
    async fn run(&mut self) -> orfail::Result<()> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(FRAME_CHANNEL_SIZE);

        loop {
            tokio::select! {
                // このクライアント用のメディアフレームを受信する
                Some(frame) = rx.recv() => {
                    self.handle_client_media_frame(frame).or_fail()?;
                    self.flush_send_buf().await.or_fail()?;
                }

                // クライアントソケットからデータを受信する
                read_result = self.stream.read(&mut self.recv_buf) => {
                    let n = read_result.or_fail()?;
                    if n == 0 {
                        // クライアントが切断した
                        break;
                    }

                    self.stats.total_received_bytes.add(n as u64);
                    self.connection.feed_recv_buf(&self.recv_buf[..n]).or_fail()?;

                    // イベントを処理する
                    while let Some(event) = self.connection.next_event() {
                        log::debug!("RTMP event: {:?}", event);
                        self.stats.total_event_count.increment();
                        self.handle_event(event, tx.clone()).await.or_fail()?;
                    }

                    self.flush_send_buf().await.or_fail()?;
                }
            }
        }

        self.cleanup().await;
        Ok(())
    }

    /// RTMP イベントを処理する
    async fn handle_event(
        &mut self,
        event: shiguredo_rtmp::RtmpConnectionEvent,
        tx: tokio::sync::mpsc::Sender<ClientMediaFrame>,
    ) -> orfail::Result<()> {
        match event {
            shiguredo_rtmp::RtmpConnectionEvent::PlayRequested {
                app, stream_name, ..
            } => {
                self.stream_id = format!("{}/{}", app, stream_name);
                self.handle_play_requested(tx).await.or_fail()?;
            }
            shiguredo_rtmp::RtmpConnectionEvent::PublishRequested { .. } => {
                self.connection
                    .reject("Publishing is not supported by this server")
                    .or_fail()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Play リクエストを処理する
    async fn handle_play_requested(
        &mut self,
        tx: tokio::sync::mpsc::Sender<ClientMediaFrame>,
    ) -> orfail::Result<()> {
        // Play リクエストを許可する
        self.connection.accept().or_fail()?;

        // このクライアントをプレイヤーとして登録する
        let mut streams = self.media_streams.lock().await;
        let stream_state = streams.entry(self.stream_id.clone()).or_default();

        let client_id = self.stream_id.len() as usize; // シンプルなクライアント ID

        let mut player_state = PlayerState {
            audio_sequence_header_sent: false,
            video_sequence_header_sent: false,
            video_keyframe_sent: false,
            tx,
        };

        // 既存のシーケンスヘッダーがあれば、新しいプレイヤーにすぐに送信できるようにマーク
        if stream_state.audio_sequence_header.is_some() {
            player_state.audio_sequence_header_sent = true;
        }
        if stream_state.video_sequence_header.is_some() {
            player_state.video_sequence_header_sent = true;
            player_state.video_keyframe_sent = true;
        }

        stream_state.players.insert(client_id, player_state);

        log::debug!("Client started playing stream: {}", self.stream_id);
        Ok(())
    }

    /// クライアント用メディアフレームを処理する
    fn handle_client_media_frame(&mut self, frame: ClientMediaFrame) -> orfail::Result<()> {
        match frame {
            ClientMediaFrame::Audio(audio) => {
                self.handle_audio_frame(audio).or_fail()?;
            }
            ClientMediaFrame::Video(video) => {
                self.handle_video_frame(video).or_fail()?;
            }
        }
        Ok(())
    }

    /// 音声フレームをクライアントに送信する
    fn handle_audio_frame(&mut self, audio: Arc<AudioData>) -> orfail::Result<()> {
        // 必要に応じて音声シーケンスヘッダーを作成して送信
        if let Some(entry) = &audio.sample_entry {
            if let Ok(seq_header) = create_audio_sequence_header(entry) {
                let seq_frame = shiguredo_rtmp::AudioFrame {
                    timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(0),
                    format: shiguredo_rtmp::AudioFormat::Aac,
                    sample_rate: shiguredo_rtmp::AudioFrame::AAC_SAMPLE_RATE,
                    is_stereo: shiguredo_rtmp::AudioFrame::AAC_STEREO,
                    is_8bit_sample: false,
                    is_aac_sequence_header: true,
                    data: seq_header,
                };
                self.connection.send_audio(seq_frame).or_fail()?;
                self.stats.total_audio_sequence_header_count.increment();
                log::debug!("Sent AAC sequence header");
            }
        }

        // 音声フレームを送信する
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
        self.connection.send_audio(frame).or_fail()?;
        self.stats.total_audio_frame_count.increment();
        Ok(())
    }

    /// 映像フレームをクライアントに送信する
    fn handle_video_frame(&mut self, video: Arc<VideoFrame>) -> orfail::Result<()> {
        let timestamp_ms = video.timestamp.as_millis() as u32;

        // キーフレームの処理
        if video.keyframe {
            self.stats.total_video_keyframe_count.increment();

            // 利用可能な場合はシーケンスヘッダーを送信する
            if let Some(entry) = &video.sample_entry {
                if let Ok(seq_header_data) = create_video_sequence_header(entry) {
                    let seq_frame = shiguredo_rtmp::VideoFrame {
                        timestamp: shiguredo_rtmp::RtmpTimestamp::from_millis(timestamp_ms),
                        composition_timestamp_offset: shiguredo_rtmp::RtmpTimestampDelta::ZERO,
                        frame_type: shiguredo_rtmp::VideoFrameType::KeyFrame,
                        codec: shiguredo_rtmp::VideoCodec::Avc,
                        avc_packet_type: Some(shiguredo_rtmp::AvcPacketType::SequenceHeader),
                        data: seq_header_data,
                    };
                    self.connection.send_video(seq_frame).or_fail()?;
                    self.stats.total_video_sequence_header_count.increment();
                    log::debug!("Sent H.264 sequence header");
                }
            }
            self.received_keyframe = true;
        }

        // キーフレームを受け取るまではフレームをスキップする
        if !self.received_keyframe && !video.keyframe {
            return Ok(());
        }

        // 必要に応じて映像データ形式を変換する
        let frame_data = match video.format {
            VideoFormat::H264 => {
                // MP4 形式を Annex B に変換する
                crate::video_h264::convert_nalu_to_annexb(&video.data, 4).or_fail()?
            }
            VideoFormat::H264AnnexB => {
                // 既に Annex B 形式
                video.data.clone()
            }
            _ => {
                return Err(orfail::Failure::new(
                    "BUG: unsupported video format in handle_video_frame",
                ));
            }
        };

        // 映像フレームを送信する
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

    /// 送信バッファをストリームにフラッシュする
    async fn flush_send_buf(&mut self) -> orfail::Result<()> {
        let send_data = self.connection.send_buf();
        if !send_data.is_empty() {
            self.stream.write_all(send_data).await.or_fail()?;
            self.stats.total_sent_bytes.add(send_data.len() as u64);
            let len = send_data.len();
            self.connection.advance_send_buf(len);
        }
        Ok(())
    }

    /// クリーンアップ処理
    async fn cleanup(&mut self) {
        if let Ok(mut streams) = self.media_streams.try_lock() {
            if let Some(state) = streams.get_mut(&self.stream_id) {
                // プレイヤー一覧からクライアントを削除する
                state.players.remove(&(self.stream_id.len() as usize));

                // 空になったストリームをクリーンアップする
                if state.players.is_empty() && state.publisher_id.is_none() {
                    streams.remove(&self.stream_id);
                }
            }
        }
    }
}

/// サンプルエントリーから音声シーケンスヘッダーを作成する
fn create_audio_sequence_header(entry: &SampleEntry) -> orfail::Result<Vec<u8>> {
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

/// サンプルエントリーから映像シーケンスヘッダーを作成する
fn create_video_sequence_header(entry: &SampleEntry) -> orfail::Result<Vec<u8>> {
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

/// [`RtmpOutboundEndpoint`] 用の統計情報
#[derive(Debug, Default, Clone)]
pub struct RtmpOutboundEndpointStats {
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

    /// 送信した音声シーケンスヘッダーの数
    pub total_audio_sequence_header_count: SharedAtomicCounter,

    /// 送信した映像シーケンスヘッダーの数
    pub total_video_sequence_header_count: SharedAtomicCounter,

    /// 処理に掛かった時間
    pub total_processing_duration: SharedAtomicDuration,

    /// エラーで中断したかどうか
    pub error: SharedAtomicFlag,
}

impl nojson::DisplayJson for RtmpOutboundEndpointStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", "rtmp_outbound_endpoint")?;
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
