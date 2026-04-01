use shiguredo_webrtc::{AudioTrackSink, RtpTransceiver, VideoSink, VideoSinkWants};

/// SoraSubscriber processor から coordinator へ通知するイベント。
///
/// sora_sdk のコールバック（on_track, on_remove_track, on_notify 等）は
/// 別スレッドから呼ばれるため、mpsc channel 経由で coordinator に転送する。
pub enum SoraSourceEvent {
    /// on_track: リモートトラック到着
    TrackReceived {
        subscriber_name: String,
        transceiver: RtpTransceiver,
    },
    /// on_remove_track: リモートトラック削除
    TrackRemoved {
        subscriber_name: String,
        track_id: String,
    },
    /// on_notify: シグナリング通知（JSON 文字列）
    Notify {
        subscriber_name: String,
        json: String,
    },
    /// on_websocket_close: WebSocket 切断
    WebSocketClose {
        subscriber_name: String,
        code: Option<u16>,
        reason: String,
    },
    /// SoraClient タスク終了（正常・異常問わず）
    Disconnected { subscriber_name: String },
}

/// holder タスクへのコマンド
pub enum SoraTrackCommand {
    /// トラックを pipeline の TrackPublisher に接続してフレーム転送を開始する
    Attach { publisher: crate::TrackPublisher },
    /// フレーム転送を停止する（トラック自体は保持する）
    Detach,
}

/// SoraSubscriber の接続パラメータ。
#[derive(Clone)]
pub struct SoraSubscriber {
    pub subscriber_name: String,
    pub signaling_urls: Vec<String>,
    pub channel_id: String,
    pub client_id: Option<String>,
    pub bundle_id: Option<String>,
    pub metadata: Option<nojson::RawJsonOwned>,
    /// coordinator へのイベント送信チャネル
    pub event_tx: tokio::sync::mpsc::UnboundedSender<SoraSourceEvent>,
}

impl SoraSubscriber {
    pub async fn run(self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        let subscriber_name = self.subscriber_name.clone();
        let event_tx = self.event_tx.clone();

        // SoraClientContext を生成（RecvOnly なので外部 ADM は不要）
        let context = sora_sdk::SoraClientContext::new()
            .map_err(|e| crate::Error::new(format!("failed to create SoraClientContext: {e}")))?;

        // コールバック用のクローン
        let on_track_name = subscriber_name.clone();
        let on_track_tx = event_tx.clone();

        let on_remove_track_name = subscriber_name.clone();
        let on_remove_track_tx = event_tx.clone();

        let on_notify_name = subscriber_name.clone();
        let on_notify_tx = event_tx.clone();

        let on_ws_close_name = subscriber_name.clone();
        let on_ws_close_tx = event_tx.clone();

        // SoraClient を構築（RecvOnly）
        let mut builder = sora_sdk::SoraClient::builder(
            context,
            self.signaling_urls.clone(),
            self.channel_id.clone(),
            sora_sdk::Role::RecvOnly,
        )
        .on_track(move |transceiver| {
            let _ = on_track_tx.send(SoraSourceEvent::TrackReceived {
                subscriber_name: on_track_name.clone(),
                transceiver,
            });
        })
        .on_remove_track(move |receiver| {
            let track = receiver.track();
            let track_id = track.id().unwrap_or_default();
            let _ = on_remove_track_tx.send(SoraSourceEvent::TrackRemoved {
                subscriber_name: on_remove_track_name.clone(),
                track_id,
            });
        })
        .on_notify(move |json| {
            let _ = on_notify_tx.send(SoraSourceEvent::Notify {
                subscriber_name: on_notify_name.clone(),
                json: json.to_owned(),
            });
        })
        .on_websocket_close(move |code, reason| {
            let _ = on_ws_close_tx.send(SoraSourceEvent::WebSocketClose {
                subscriber_name: on_ws_close_name.clone(),
                code,
                reason: reason.to_owned(),
            });
        })
        .on_signaling_message(move |sig_type, direction, message| {
            tracing::debug!(
                "SoraSubscriber signaling: type={:?}, direction={:?}, message={}",
                sig_type,
                direction,
                &message[..message.len().min(200)]
            );
        });

        if let Some(client_id) = &self.client_id {
            builder = builder.client_id(client_id.clone());
        }
        if let Some(bundle_id) = &self.bundle_id {
            builder = builder.bundle_id(bundle_id.clone());
        }
        if let Some(metadata) = &self.metadata {
            let json_string: sora_sdk::JsonString = metadata
                .to_string()
                .parse()
                .map_err(|e| crate::Error::new(format!("failed to parse metadata: {e}")))?;
            builder = builder.metadata(json_string);
        }

        let (client, client_handle) = builder
            .build()
            .map_err(|e| crate::Error::new(format!("failed to build SoraClient: {e}")))?;

        tracing::info!(
            "SoraSubscriber '{}': SoraClient built, starting connection...",
            subscriber_name
        );

        // Sora 接続を開始（バックグラウンドタスク）
        let disconnected_name = subscriber_name.clone();
        let disconnected_tx = event_tx.clone();
        let mut client_task = tokio::spawn(async move {
            tracing::info!(
                "SoraSubscriber '{}': client.run() starting",
                disconnected_name
            );
            if let Err(e) = client.run().await {
                tracing::warn!(
                    "SoraSubscriber '{}' terminated with error: {e}",
                    disconnected_name
                );
            }
            tracing::info!(
                "SoraSubscriber '{}': client.run() finished",
                disconnected_name
            );
            let _ = disconnected_tx.send(SoraSourceEvent::Disconnected {
                subscriber_name: disconnected_name,
            });
        });

        handle.notify_ready();

        // client_task の完了を待つ。
        // processor が TerminateProcessor で abort された場合はここで中断される。
        let _ = (&mut client_task).await;

        // 切断（まだ接続中の場合）
        if let Err(e) = client_handle.disconnect().await {
            tracing::warn!(
                "failed to disconnect SoraSubscriber '{}': {e}",
                subscriber_name
            );
        }
        // タスク終了を待ち、タイムアウト時は abort する
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), &mut client_task).await;
        client_task.abort();

        Ok(())
    }
}

pub async fn create_processor(
    handle: &crate::MediaPipelineHandle,
    subscriber: SoraSubscriber,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    let processor_id = processor_id.unwrap_or_else(|| {
        crate::ProcessorId::new(format!("soraSubscriber:{}", subscriber.subscriber_name))
    });
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("sora_subscriber"),
            move |h| subscriber.run(h),
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}

// --- フレーム転送関連 ---

/// I420 フレームデータ（libwebrtc スレッドから非同期チャネル経由で転送する）
struct RawI420Frame {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    width: u32,
    height: u32,
    timestamp_us: i64,
}

/// libwebrtc の VideoSinkHandler 実装。
/// on_frame() で I420 フレームを mpsc channel に送信する。
struct VideoFrameSinkHandler {
    frame_tx: tokio::sync::mpsc::Sender<RawI420Frame>,
}

impl shiguredo_webrtc::VideoSinkHandler for VideoFrameSinkHandler {
    fn on_frame(&mut self, frame: shiguredo_webrtc::VideoFrameRef<'_>) {
        let width = frame.width() as u32;
        let height = frame.height() as u32;
        let timestamp_us = frame.timestamp_us();
        let buffer = frame.buffer();

        let y = buffer.y_data().to_vec();
        let u = buffer.u_data().to_vec();
        let v = buffer.v_data().to_vec();

        // 満杯時はドロップ（backpressure）
        let _ = self.frame_tx.try_send(RawI420Frame {
            y,
            u,
            v,
            width,
            height,
            timestamp_us,
        });
    }
}

/// 音声フレームデータ（libwebrtc スレッドから非同期チャネル経由で転送する）
struct RawAudioFrame {
    data: Vec<u8>,
    sample_rate: i32,
    channels: usize,
}

/// libwebrtc の AudioTrackSinkHandler 実装。
struct AudioFrameSinkHandler {
    frame_tx: tokio::sync::mpsc::Sender<RawAudioFrame>,
}

impl shiguredo_webrtc::AudioTrackSinkHandler for AudioFrameSinkHandler {
    fn on_data(
        &mut self,
        audio_data: &[u8],
        _bits_per_sample: i32,
        sample_rate: i32,
        number_of_channels: usize,
        _number_of_frames: usize,
    ) {
        let _ = self.frame_tx.try_send(RawAudioFrame {
            data: audio_data.to_vec(),
            sample_rate,
            channels: number_of_channels,
        });
    }
}

/// リモートトラックの WebRTC 型を保持し、コマンドに応じてフレーム転送を制御するタスク。
///
/// coordinator は !Sync な WebRTC 型を直接保持できないため、
/// このタスクに所有権を移して管理する。
/// リモートトラックの WebRTC 型を保持し、コマンドに応じてフレーム転送を制御するタスク。
///
/// coordinator は !Sync な WebRTC 型を直接保持できないため、
/// このタスクに所有権を移して管理する。
///
/// 注意: RtpTransceiver から取得した track / receiver は !Send の可能性があるため、
/// .await をまたいで保持しないように、初期化時に cast してから command loop に入る。
pub async fn sora_track_holder_task(
    transceiver: RtpTransceiver,
    track_kind: String,
    mut command_rx: tokio::sync::mpsc::UnboundedReceiver<SoraTrackCommand>,
) {
    // track_kind に応じて事前に cast する。
    // receiver / track は !Send のため、.await をまたいで保持してはいけない。
    // ブロックスコープで囲んで、.await 前に確実に drop する。
    enum TrackVariant {
        Video(shiguredo_webrtc::VideoTrack),
        Audio(shiguredo_webrtc::AudioTrack),
        Unsupported,
    }
    let mut variant = {
        let receiver = transceiver.receiver();
        let track = receiver.track();
        match track_kind.as_str() {
            "video" => TrackVariant::Video(track.cast_to_video_track()),
            "audio" => TrackVariant::Audio(track.cast_to_audio_track()),
            _ => TrackVariant::Unsupported,
        }
    };

    let mut _video_sink: Option<VideoSink> = None;
    let mut _audio_sink: Option<AudioTrackSink> = None;
    let mut forward_abort: Option<tokio::task::AbortHandle> = None;

    while let Some(cmd) = command_rx.recv().await {
        match cmd {
            SoraTrackCommand::Attach { publisher } => {
                tracing::debug!("sora_track_holder: Attach, kind={}", track_kind);
                if let Some(abort) = forward_abort.take() {
                    abort.abort();
                }
                _video_sink = None;
                _audio_sink = None;

                match &mut variant {
                    TrackVariant::Video(video_track) => {
                        let (frame_tx, frame_rx) = tokio::sync::mpsc::channel::<RawI420Frame>(2);
                        let sink_handler = VideoFrameSinkHandler { frame_tx };
                        let sink = VideoSink::new_with_handler(Box::new(sink_handler));
                        let wants = VideoSinkWants::new();
                        video_track.add_or_update_sink(&sink, &wants);
                        _video_sink = Some(sink);

                        let task = tokio::spawn(video_forward_task(frame_rx, publisher));
                        forward_abort = Some(task.abort_handle());
                    }
                    TrackVariant::Audio(audio_track) => {
                        let (frame_tx, frame_rx) = tokio::sync::mpsc::channel::<RawAudioFrame>(4);
                        let sink_handler = AudioFrameSinkHandler { frame_tx };
                        let sink = AudioTrackSink::new_with_handler(Box::new(sink_handler));
                        audio_track.add_sink(&sink);
                        _audio_sink = Some(sink);

                        let task = tokio::spawn(audio_forward_task(frame_rx, publisher));
                        forward_abort = Some(task.abort_handle());
                    }
                    TrackVariant::Unsupported => {
                        tracing::warn!("unsupported track kind for sora_source: {}", track_kind);
                    }
                }
            }
            SoraTrackCommand::Detach => {
                if let Some(abort) = forward_abort.take() {
                    abort.abort();
                }
                _video_sink = None;
                _audio_sink = None;
            }
        }
    }

    if let Some(abort) = forward_abort.take() {
        abort.abort();
    }
    // transceiver はここで drop される
}

/// 映像フレーム転送タスク: mpsc channel → pipeline publish
async fn video_forward_task(
    mut frame_rx: tokio::sync::mpsc::Receiver<RawI420Frame>,
    mut publisher: crate::TrackPublisher,
) {
    tracing::debug!("video_forward_task: started");
    while let Some(frame) = frame_rx.recv().await {
        let width = frame.width as usize;
        let height = frame.height as usize;

        // I420 コンパクトデータを構築（stride を詰める）
        let y_size = width * height;
        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);
        let uv_size = uv_width * uv_height;

        let stride_y = frame.y.len() / height;
        let stride_u = frame.u.len() / uv_height;
        let stride_v = frame.v.len() / uv_height;

        let mut i420_data = Vec::with_capacity(y_size + uv_size * 2);

        for row in 0..height {
            let start = row * stride_y;
            i420_data.extend_from_slice(&frame.y[start..start + width]);
        }
        for row in 0..uv_height {
            let start = row * stride_u;
            i420_data.extend_from_slice(&frame.u[start..start + uv_width]);
        }
        for row in 0..uv_height {
            let start = row * stride_v;
            i420_data.extend_from_slice(&frame.v[start..start + uv_width]);
        }

        let video_frame = crate::VideoFrame {
            format: crate::video::VideoFormat::I420,
            keyframe: true,
            size: Some(crate::video::VideoFrameSize { width, height }),
            timestamp: std::time::Duration::from_micros(frame.timestamp_us.max(0) as u64),
            sample_entry: None,
            data: i420_data,
        };

        if !publisher.send_video(video_frame) {
            tracing::warn!("video_forward_task: pipeline closed, stopping");
            break;
        }
    }
}

/// 音声フレーム転送タスク: mpsc channel → pipeline publish
async fn audio_forward_task(
    mut frame_rx: tokio::sync::mpsc::Receiver<RawAudioFrame>,
    mut publisher: crate::TrackPublisher,
) {
    while let Some(frame) = frame_rx.recv().await {
        // PCM i16 LE → I16Be 変換
        // libwebrtc の AudioTrackSinkHandler は i16 LE で提供する
        let mut i16be_data = Vec::with_capacity(frame.data.len());
        for chunk in frame.data.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            i16be_data.extend_from_slice(&sample.to_be_bytes());
        }

        let channels = match crate::audio::Channels::from_u8(frame.channels as u8) {
            Ok(ch) => ch,
            Err(_) => continue,
        };
        let sample_rate = match crate::audio::SampleRate::from_u32(frame.sample_rate as u32) {
            Ok(sr) => sr,
            Err(_) => continue,
        };

        let audio_frame = crate::AudioFrame {
            data: i16be_data,
            format: crate::audio::AudioFormat::I16Be,
            channels,
            sample_rate,
            timestamp: std::time::Duration::ZERO,
            sample_entry: None,
        };

        if !publisher.send_audio(audio_frame) {
            break;
        }
    }
}
