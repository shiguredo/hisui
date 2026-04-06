/// メインスレッドへの制御コマンド
pub enum PlayerCommand {
    Start {
        canvas_width: i32,
        canvas_height: i32,
        /// 世代 ID。Stopped イベントに含めて返すことで、古い停止イベントを無視できる
        generation: u64,
        reply_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    Stop,
    Terminate,
}

/// player のライフサイクルイベント
#[derive(Debug)]
pub enum PlayerLifecycleEvent {
    /// ウィンドウが閉じられた、または SDL エラーで停止した
    Stopped {
        /// Start 時に渡された世代 ID
        generation: u64,
    },
}

/// メインスレッド（SDL）に渡すメディアデータ
pub enum PlayerMediaMessage {
    Video {
        y: Vec<u8>,
        u: Vec<u8>,
        v: Vec<u8>,
        width: i32,
        height: i32,
        pts_us: i64,
    },
    Audio {
        /// リトルエンディアン S16 PCM データ
        data: Vec<u8>,
        pts_us: i64,
        sample_rate: i32,
        channels: i32,
    },
}

/// Program 出力の映像・音声フレームを受信して raw_player に転送するタスク
pub async fn run_player_subscriber(
    pipeline_handle: crate::MediaPipelineHandle,
    video_track_id: crate::TrackId,
    audio_track_id: crate::TrackId,
    media_tx: std::sync::mpsc::SyncSender<PlayerMediaMessage>,
) {
    let processor_handle = match pipeline_handle
        .register_processor(
            crate::ProcessorId::new("player"),
            crate::ProcessorMetadata::new("player"),
        )
        .await
    {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("failed to register player processor: {e}");
            return;
        }
    };

    let mut video_rx = processor_handle.subscribe_track(video_track_id);
    let mut audio_rx = processor_handle.subscribe_track(audio_track_id);

    loop {
        // 映像と音声を同時に待ち受ける
        tokio::select! {
            msg = video_rx.recv() => {
                match msg {
                    crate::media_pipeline::Message::Media(media_frame) => {
                        let video_frame = match media_frame {
                            crate::MediaFrame::Video(f) => f,
                            _ => continue,
                        };

                        let raw_frame =
                            match crate::video::RawVideoFrame::from_i420_video_frame(video_frame.clone()) {
                                Ok(f) => f,
                                Err(_) => continue,
                            };

                        let size = raw_frame.size();
                        let (y, u, v) = match raw_frame.as_i420_planes() {
                            Ok(planes) => planes,
                            Err(_) => continue,
                        };

                        let msg = PlayerMediaMessage::Video {
                            y: y.to_vec(),
                            u: u.to_vec(),
                            v: v.to_vec(),
                            width: size.width as i32,
                            height: size.height as i32,
                            pts_us: video_frame.timestamp.as_micros() as i64,
                        };

                        if send_media(&media_tx, msg).is_err() {
                            break;
                        }
                    }
                    crate::media_pipeline::Message::Eos => break,
                    crate::media_pipeline::Message::Syn(_) => {}
                }
            }
            msg = audio_rx.recv() => {
                match msg {
                    crate::media_pipeline::Message::Media(media_frame) => {
                        let audio_frame = match media_frame {
                            crate::MediaFrame::Audio(f) => f,
                            _ => continue,
                        };

                        if audio_frame.format != crate::audio::AudioFormat::I16Be {
                            continue;
                        }

                        let msg = PlayerMediaMessage::Audio {
                            data: i16be_to_i16le(&audio_frame.data),
                            pts_us: audio_frame.timestamp.as_micros() as i64,
                            sample_rate: audio_frame.sample_rate.get() as i32,
                            channels: audio_frame.channels.get() as i32,
                        };

                        if send_media(&media_tx, msg).is_err() {
                            break;
                        }
                    }
                    crate::media_pipeline::Message::Eos => break,
                    crate::media_pipeline::Message::Syn(_) => {}
                }
            }
        }
    }
}

/// ウィンドウが閉じられた場合は Err を返す
fn send_media(
    tx: &std::sync::mpsc::SyncSender<PlayerMediaMessage>,
    msg: PlayerMediaMessage,
) -> Result<(), ()> {
    match tx.try_send(msg) {
        Ok(()) => Ok(()),
        // バッファ満杯ならドロップ（リアルタイム表示優先）
        Err(std::sync::mpsc::TrySendError::Full(_)) => Ok(()),
        // ウィンドウが閉じられた
        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => Err(()),
    }
}

/// ビッグエンディアン I16 PCM をリトルエンディアン I16 PCM に変換する
fn i16be_to_i16le(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(2) {
        out.push(chunk[1]);
        out.push(chunk[0]);
    }
    out
}
