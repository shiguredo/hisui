use std::sync::Arc;

use crate::types::EvenUsize;

/// ビデオスケーラープロセッサの設定
pub struct VideoScalerConfig {
    pub input_track_id: crate::TrackId,
    pub output_track_id: crate::TrackId,
    pub width: EvenUsize,
    pub height: EvenUsize,
}

/// ビデオスケーラープロセッサを作成する。
/// 入力トラックのビデオフレームを指定解像度にリサイズして出力トラックに送る。
pub async fn create_processor(
    handle: &crate::MediaPipelineHandle,
    config: VideoScalerConfig,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    let processor_id = processor_id.unwrap_or_else(|| crate::ProcessorId::new("videoScaler"));
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("video_scaler"),
            move |h| async move { run(h, config).await },
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}

async fn run(handle: crate::ProcessorHandle, config: VideoScalerConfig) -> crate::Result<()> {
    let mut input_rx = handle.subscribe_track(config.input_track_id);
    let mut output_tx = handle.publish_track(config.output_track_id).await?;
    handle.notify_ready();

    loop {
        match input_rx.recv().await {
            crate::Message::Media(crate::MediaFrame::Video(frame)) => {
                // VideoFrame::resize() でリサイズし、解像度が同じ場合は None が返る
                let output_frame = match frame.resize(
                    config.width,
                    config.height,
                    shiguredo_libyuv::FilterMode::Bilinear,
                )? {
                    Some(resized) => resized,
                    None => Arc::unwrap_or_clone(frame),
                };
                if !output_tx.send_video(output_frame) {
                    break;
                }
            }
            crate::Message::Eos => {
                output_tx.send_eos();
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
