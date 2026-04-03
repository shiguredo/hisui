/// raw_player 表示用のフレームデータ
pub struct RawPlayerFrame {
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub pts_us: i64,
}

/// Program 出力の映像フレームを受信して raw_player に転送するタスク
pub async fn run_monitor_subscriber(
    pipeline_handle: crate::MediaPipelineHandle,
    video_track_id: crate::TrackId,
    frame_tx: std::sync::mpsc::SyncSender<RawPlayerFrame>,
) {
    let processor_handle = match pipeline_handle
        .register_processor(
            crate::ProcessorId::new("monitor"),
            crate::ProcessorMetadata::new("monitor"),
        )
        .await
    {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("failed to register monitor processor: {e}");
            return;
        }
    };

    let mut rx = processor_handle.subscribe_track(video_track_id);

    loop {
        match rx.recv().await {
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

                let frame = RawPlayerFrame {
                    y: y.to_vec(),
                    u: u.to_vec(),
                    v: v.to_vec(),
                    width: size.width as i32,
                    height: size.height as i32,
                    pts_us: video_frame.timestamp.as_micros() as i64,
                };

                // バッファ満杯ならフレームドロップ（リアルタイム表示優先）
                match frame_tx.try_send(frame) {
                    Ok(()) => {}
                    Err(std::sync::mpsc::TrySendError::Full(_)) => {}
                    Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                        // ウィンドウが閉じられた
                        break;
                    }
                }
            }
            crate::media_pipeline::Message::Eos => break,
            crate::media_pipeline::Message::Syn(_) => {}
        }
    }
}
