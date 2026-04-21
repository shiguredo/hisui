use std::sync::Arc;

use crate::{ProcessorHandle, TrackId};

/// Sora WebRTC Publisher processor。
/// Program 出力の raw フレーム（I420 + PCM）を sora-rust-sdk の SendOnly 接続で送信する。
#[derive(Debug, Clone)]
pub struct SoraPublisher {
    pub signaling_urls: Vec<String>,
    pub channel_id: String,
    pub client_id: Option<String>,
    pub bundle_id: Option<String>,
    pub metadata: Option<nojson::RawJsonOwned>,
    pub input_video_track_id: TrackId,
    pub input_audio_track_id: TrackId,
}

impl SoraPublisher {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        // video/audio track を購読
        let mut video_rx = handle.subscribe_track(self.input_video_track_id.clone());
        let mut audio_rx = handle.subscribe_track(self.input_audio_track_id.clone());

        // 音声データの供給に使う共有状態を準備
        let audio_state = Arc::new(crate::webrtc::audio::SharedAudioState::new());
        let adm_handler =
            crate::webrtc::audio::HisuiAudioDeviceModuleHandler::new(audio_state.clone());
        let external_adm =
            shiguredo_webrtc::AudioDeviceModule::new_with_handler(Box::new(adm_handler));

        // SoraConnectionContext を外部 ADM 付きで生成
        let context = sora_sdk::SoraConnectionContext::new_with_config(
            sora_sdk::SoraConnectionContextConfig {
                adm_config: sora_sdk::AdmConfig::UseExternal(external_adm),
                ..Default::default()
            },
        )
        .map_err(|e| crate::Error::new(format!("failed to create SoraConnectionContext: {e}")))?;

        // video track を作成
        let mut video_source = shiguredo_webrtc::AdaptedVideoTrackSource::new();
        let mut video_timestamp_aligner = shiguredo_webrtc::TimestampAligner::new();
        let video_track = context
            .create_video_track(&video_source.cast_to_video_track_source())
            .map_err(|e| crate::Error::new(format!("failed to create video track: {e}")))?;

        // audio track を作成
        let audio_source = context
            .create_audio_source()
            .map_err(|e| crate::Error::new(format!("failed to create audio source: {e}")))?;
        let audio_track = context
            .create_audio_track(&audio_source)
            .map_err(|e| crate::Error::new(format!("failed to create audio track: {e}")))?;

        // SoraConnection を構築
        let mut builder = sora_sdk::SoraConnection::builder(
            context,
            self.signaling_urls.clone(),
            self.channel_id.clone(),
            sora_sdk::Role::SendOnly,
        )
        .sender_video_track(video_track)
        .sender_audio_track(audio_track);

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

        let (connection, connection_handle) = builder
            .build()
            .map_err(|e| crate::Error::new(format!("failed to build SoraConnection: {e}")))?;

        // Sora 接続を開始（バックグラウンドタスク）
        let mut connection_task = tokio::spawn(async move {
            if let Err(e) = connection.run().await {
                tracing::warn!("Sora connection terminated with error: {e}");
            }
        });

        handle.notify_ready();

        // 起動直後に上流 video encoder へキーフレーム要求を送る
        if let Err(e) = crate::encoder::request_upstream_video_keyframe(
            &handle.pipeline_handle(),
            handle.processor_id(),
            "sora_publisher_start",
        )
        .await
        {
            tracing::warn!(
                "failed to request keyframe for Sora publisher start: {}",
                e.display()
            );
        }

        // メッセージ受信ループ
        // video/audio 両方の EOS を受け取ってから切断する。
        let mut video_eos = false;
        let mut audio_eos = false;
        loop {
            tokio::select! {
                message = video_rx.recv() => {
                    match message {
                        crate::Message::Media(crate::MediaFrame::Video(frame)) => {
                            if frame.format == crate::video::VideoFormat::I420 {
                                if let Err(e) = crate::webrtc::video::push_i420_frame(
                                    &mut video_source,
                                    &mut video_timestamp_aligner,
                                    &frame,
                                ) {
                                    tracing::warn!("failed to push video frame to Sora: {}", e.display());
                                }
                            } else {
                                tracing::debug!("unsupported video format: {}, expected I420", frame.format);
                            }
                        }
                        crate::Message::Media(crate::MediaFrame::Audio(_)) => {}
                        crate::Message::Eos => {
                            tracing::info!("video track EOS received");
                            video_eos = true;
                            if video_eos && audio_eos {
                                break;
                            }
                        }
                        crate::Message::Syn(_) => {}
                    }
                }
                message = audio_rx.recv() => {
                    match message {
                        crate::Message::Media(crate::MediaFrame::Audio(frame)) => {
                            if frame.format == crate::audio::AudioFormat::I16Be {
                                if let Err(e) = audio_state.push_audio_frame(&frame) {
                                    tracing::warn!("failed to push audio frame to Sora: {}", e.display());
                                }
                            } else {
                                tracing::debug!("unsupported audio format: {}, expected I16Be", frame.format);
                            }
                        }
                        crate::Message::Media(crate::MediaFrame::Video(_)) => {}
                        crate::Message::Eos => {
                            tracing::info!("audio track EOS received");
                            audio_eos = true;
                            if video_eos && audio_eos {
                                break;
                            }
                        }
                        crate::Message::Syn(_) => {}
                    }
                }
            }
        }

        // 切断
        if let Err(e) = connection_handle.disconnect().await {
            tracing::warn!("failed to disconnect Sora connection: {e}");
        }
        // タスク終了を待ち、タイムアウト時は abort する
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), &mut connection_task).await;
        connection_task.abort();

        Ok(())
    }
}

pub async fn create_processor(
    handle: &crate::MediaPipelineHandle,
    publisher: SoraPublisher,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    let processor_id = processor_id.unwrap_or_else(|| crate::ProcessorId::new("soraPublisher"));
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("sora_publisher"),
            move |h| publisher.run(h),
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}
