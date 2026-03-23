// NOTE: 長いので MediaPipelineHandle のパイプライン操作メソッドはこっちで実装している

use crate::media_pipeline::{
    MediaPipelineCommand, MediaPipelineHandle, PROCESSOR_TYPE_VIDEO_ENCODER,
    PipelineOperationError, ProcessorId, ProcessorMetadata, RegisterProcessorError, TrackId,
};

/// 入力 MP4 ファイルパスの事前検証
fn validate_mp4_input_path(path: &std::path::Path) -> Result<(), PipelineOperationError> {
    if !path.exists() {
        return Err(PipelineOperationError::InvalidParams(format!(
            "input path does not exist: {}",
            path.display()
        )));
    }
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| ext.eq_ignore_ascii_case("mp4"))
        .is_none()
    {
        return Err(PipelineOperationError::InvalidParams(format!(
            "input path must be an mp4 file: {}",
            path.display()
        )));
    }
    Ok(())
}

/// updateVideoMixer の結果
pub struct UpdateVideoMixerResult {
    pub previous_canvas_width: usize,
    pub previous_canvas_height: usize,
    pub previous_frame_rate: crate::video::FrameRate,
    pub previous_input_tracks: Vec<crate::mixer_realtime_video::InputTrack>,
}

fn default_video_encode_config_for_rpc() -> crate::encoder::EncodeConfig {
    // server RPC の既定 encode params は、compose 既定値と同じ値を利用する。
    crate::sora::recording_layout_encode_params::LayoutEncodeParams::default().config
}

impl MediaPipelineHandle {
    // --- 型付き public メソッド ---

    fn map_register_error(
        e: RegisterProcessorError,
        processor_id: &ProcessorId,
    ) -> PipelineOperationError {
        match e {
            RegisterProcessorError::DuplicateProcessorId => {
                PipelineOperationError::DuplicateProcessorId(processor_id.clone())
            }
            RegisterProcessorError::PipelineTerminated => {
                PipelineOperationError::PipelineTerminated
            }
        }
    }

    fn map_get_rpc_sender_error(
        e: crate::media_pipeline::GetProcessorRpcSenderError,
        processor_id: &ProcessorId,
        component: &str,
    ) -> PipelineOperationError {
        match e {
            crate::media_pipeline::GetProcessorRpcSenderError::PipelineTerminated => {
                PipelineOperationError::PipelineTerminated
            }
            crate::media_pipeline::GetProcessorRpcSenderError::ProcessorNotFound => {
                PipelineOperationError::InvalidParams(format!(
                    "processorId not found: {processor_id}"
                ))
            }
            crate::media_pipeline::GetProcessorRpcSenderError::SenderNotRegistered
            | crate::media_pipeline::GetProcessorRpcSenderError::TypeMismatch => {
                PipelineOperationError::InvalidParams(format!(
                    "processor does not support {component} updates: {processor_id}"
                ))
            }
        }
    }

    pub async fn create_mp4_file_source(
        &self,
        source: crate::obsws::source::file_mp4::Mp4FileSource,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id =
            processor_id.unwrap_or_else(|| ProcessorId::new(source.path.display().to_string()));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("mp4_file_source"),
            move |handle| source.run(handle),
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_mp4_video_reader(
        &self,
        path: std::path::PathBuf,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        validate_mp4_input_path(&path)?;
        let processor_id =
            processor_id.unwrap_or_else(|| ProcessorId::new(path.display().to_string()));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("mp4_video_reader"),
            move |handle| async move {
                let reader = crate::sora::recording_reader::VideoReader::new(
                    crate::types::ContainerFormat::Mp4,
                    std::time::Duration::ZERO,
                    vec![path],
                    handle.stats(),
                )?;
                reader.run(handle).await
            },
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_mp4_audio_reader(
        &self,
        path: std::path::PathBuf,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        validate_mp4_input_path(&path)?;
        let processor_id =
            processor_id.unwrap_or_else(|| ProcessorId::new(path.display().to_string()));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("mp4_audio_reader"),
            move |handle| async move {
                let reader = crate::sora::recording_reader::AudioReader::new(
                    crate::types::ContainerFormat::Mp4,
                    std::time::Duration::ZERO,
                    vec![path],
                    handle.stats(),
                )?;
                reader.run(handle).await
            },
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_mp4_writer(
        &self,
        output_path: std::path::PathBuf,
        input_audio_track_id: Option<TrackId>,
        input_video_track_id: Option<TrackId>,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        if input_audio_track_id.is_none() && input_video_track_id.is_none() {
            return Err(PipelineOperationError::InvalidParams(
                "inputAudioTrackId or inputVideoTrackId is required".to_owned(),
            ));
        }

        let is_mp4 = output_path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"));
        if !is_mp4 {
            return Err(PipelineOperationError::InvalidParams(format!(
                "outputPath must be an mp4 file: {}",
                output_path.display()
            )));
        }

        if let Some(parent) = output_path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            return Err(PipelineOperationError::InvalidParams(format!(
                "outputPath parent directory does not exist: {}",
                parent.display()
            )));
        }

        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("mp4Writer"));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("mp4_writer"),
            move |handle| async move {
                let writer = crate::writer_mp4::Mp4Writer::new(
                    &output_path,
                    None,
                    input_audio_track_id.clone(),
                    input_video_track_id.clone(),
                    handle.stats(),
                )?;
                writer
                    .run(handle, input_audio_track_id, input_video_track_id)
                    .await
            },
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_video_decoder(
        &self,
        input_track_id: TrackId,
        output_track_id: TrackId,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id
            .unwrap_or_else(|| ProcessorId::new(format!("videoDecoder:{input_track_id}")));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("video_decoder"),
            move |handle| async move {
                let decoder = crate::decoder::VideoDecoder::new(
                    crate::decoder::VideoDecoderOptions {
                        openh264_lib: handle.config().openh264_lib.clone(),
                        ..Default::default()
                    },
                    handle.stats(),
                );
                decoder.run(handle, input_track_id, output_track_id).await
            },
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_audio_decoder(
        &self,
        input_track_id: TrackId,
        output_track_id: TrackId,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id
            .unwrap_or_else(|| ProcessorId::new(format!("audioDecoder:{input_track_id}")));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("audio_decoder"),
            move |handle| async move {
                #[cfg(feature = "fdk-aac")]
                let fdk_aac_lib = handle.config().fdk_aac_lib.clone();
                let decoder = crate::decoder::AudioDecoder::new(
                    #[cfg(feature = "fdk-aac")]
                    fdk_aac_lib,
                    handle.stats(),
                )?;
                decoder.run(handle, input_track_id, output_track_id).await
            },
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_audio_encoder(
        &self,
        input_track_id: TrackId,
        output_track_id: TrackId,
        codec: crate::types::CodecName,
        bitrate_bps: std::num::NonZeroUsize,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id
            .unwrap_or_else(|| ProcessorId::new(format!("audioEncoder:{input_track_id}")));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("audio_encoder"),
            move |handle| async move {
                #[cfg(feature = "fdk-aac")]
                let fdk_aac_lib = handle.config().fdk_aac_lib.clone();
                let encoder = crate::encoder::AudioEncoder::new(
                    codec,
                    bitrate_bps,
                    #[cfg(feature = "fdk-aac")]
                    fdk_aac_lib,
                    handle.stats(),
                )?;
                encoder.run(handle, input_track_id, output_track_id).await
            },
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_video_encoder(
        &self,
        input_track_id: TrackId,
        output_track_id: TrackId,
        codec: crate::types::CodecName,
        bitrate_bps: std::num::NonZeroUsize,
        frame_rate: crate::video::FrameRate,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id
            .unwrap_or_else(|| ProcessorId::new(format!("videoEncoder:{input_track_id}")));
        let options = crate::encoder::VideoEncoderOptions {
            codec,
            engines: None,
            bitrate: bitrate_bps.get(),
            width: crate::types::EvenUsize::ZERO,
            height: crate::types::EvenUsize::ZERO,
            frame_rate,
            encode_params: default_video_encode_config_for_rpc(),
        };
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new(PROCESSOR_TYPE_VIDEO_ENCODER),
            move |handle| async move {
                let encoder = crate::encoder::VideoEncoder::new(
                    &options,
                    handle.config().openh264_lib.clone(),
                    handle.stats(),
                )?;
                encoder.run(handle, input_track_id, output_track_id).await
            },
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_png_file_source(
        &self,
        source: crate::obsws::source::png_file::PngFileSource,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id =
            processor_id.unwrap_or_else(|| ProcessorId::new(source.path.display().to_string()));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("png_file_source"),
            move |handle| source.run(handle),
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_video_device_source(
        &self,
        source: crate::obsws::source::video_device::VideoDeviceSource,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id.unwrap_or_else(|| {
            if let Some(device_id) = source.device_id.as_deref() {
                ProcessorId::new(format!("videoDeviceSource:{device_id}"))
            } else {
                ProcessorId::new("videoDeviceSource:default")
            }
        });
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("video_device_source"),
            move |handle| source.run(handle),
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_audio_mixer(
        &self,
        mixer: crate::mixer_realtime_audio::AudioRealtimeMixer,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("audioMixer"));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("audio_mixer"),
            move |handle| mixer.run(handle),
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn update_audio_mixer_inputs(
        &self,
        processor_id: ProcessorId,
        input_tracks: Vec<crate::mixer_realtime_audio::AudioRealtimeInputTrack>,
    ) -> Result<Vec<crate::mixer_realtime_audio::AudioRealtimeInputTrack>, PipelineOperationError>
    {
        let sender = self
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::mixer_realtime_audio::AudioRealtimeMixerRpcMessage,
            >>(&processor_id)
            .await
            .map_err(|e| Self::map_get_rpc_sender_error(e, &processor_id, "audio mixer input"))?;

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        sender
            .send(
                crate::mixer_realtime_audio::AudioRealtimeMixerRpcMessage::UpdateInputs {
                    input_tracks,
                    reply_tx,
                },
            )
            .map_err(|_| {
                PipelineOperationError::InternalError(
                    "audio mixer RPC sender channel is closed".to_owned(),
                )
            })?;
        let result = reply_rx.await.map_err(|_| {
            PipelineOperationError::InternalError(
                "audio mixer RPC response channel is closed".to_owned(),
            )
        })?;
        let result = result.map_err(|e| PipelineOperationError::InvalidParams(e.display()))?;
        Ok(result.previous_input_tracks)
    }

    pub async fn finish_audio_mixer(
        &self,
        processor_id: ProcessorId,
    ) -> Result<(), PipelineOperationError> {
        let sender = self
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::mixer_realtime_audio::AudioRealtimeMixerRpcMessage,
            >>(&processor_id)
            .await
            .map_err(|e| Self::map_get_rpc_sender_error(e, &processor_id, "audio mixer"))?;

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        sender
            .send(crate::mixer_realtime_audio::AudioRealtimeMixerRpcMessage::Finish { reply_tx })
            .map_err(|_| {
                PipelineOperationError::InternalError(
                    "audio mixer RPC sender channel is closed".to_owned(),
                )
            })?;
        reply_rx.await.map_err(|_| {
            PipelineOperationError::InternalError(
                "audio mixer RPC response channel is closed".to_owned(),
            )
        })?;
        Ok(())
    }

    pub async fn create_video_mixer(
        &self,
        mixer: crate::mixer_realtime_video::VideoRealtimeMixer,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("videoMixer"));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("video_mixer"),
            move |handle| mixer.run(handle),
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn update_video_mixer(
        &self,
        processor_id: ProcessorId,
        request: crate::mixer_realtime_video::VideoRealtimeMixerUpdateConfigRequest,
    ) -> Result<UpdateVideoMixerResult, PipelineOperationError> {
        let sender = self
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::mixer_realtime_video::VideoRealtimeMixerRpcMessage,
            >>(&processor_id)
            .await
            .map_err(|e| Self::map_get_rpc_sender_error(e, &processor_id, "video mixer"))?;

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        sender
            .send(
                crate::mixer_realtime_video::VideoRealtimeMixerRpcMessage::UpdateConfig {
                    request,
                    reply_tx,
                },
            )
            .map_err(|_| {
                PipelineOperationError::InternalError(
                    "video mixer RPC sender channel is closed".to_owned(),
                )
            })?;
        let result = reply_rx.await.map_err(|_| {
            PipelineOperationError::InternalError(
                "video mixer RPC response channel is closed".to_owned(),
            )
        })?;
        let result = result.map_err(|e| PipelineOperationError::InvalidParams(e.display()))?;
        Ok(UpdateVideoMixerResult {
            previous_canvas_width: result.previous_canvas_width,
            previous_canvas_height: result.previous_canvas_height,
            previous_frame_rate: result.previous_frame_rate,
            previous_input_tracks: result.previous_input_tracks,
        })
    }

    pub async fn finish_video_mixer(
        &self,
        processor_id: ProcessorId,
    ) -> Result<(), PipelineOperationError> {
        let sender = self
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::mixer_realtime_video::VideoRealtimeMixerRpcMessage,
            >>(&processor_id)
            .await
            .map_err(|e| Self::map_get_rpc_sender_error(e, &processor_id, "video mixer"))?;

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        sender
            .send(crate::mixer_realtime_video::VideoRealtimeMixerRpcMessage::Finish { reply_tx })
            .map_err(|_| {
                PipelineOperationError::InternalError(
                    "video mixer RPC sender channel is closed".to_owned(),
                )
            })?;
        reply_rx.await.map_err(|_| {
            PipelineOperationError::InternalError(
                "video mixer RPC response channel is closed".to_owned(),
            )
        })?;
        Ok(())
    }

    pub async fn create_rtmp_publisher(
        &self,
        publisher: crate::publisher_rtmp::RtmpPublisher,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("rtmpPublisher"));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("rtmp_publisher"),
            move |handle| publisher.run(handle),
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_rtmp_inbound_endpoint(
        &self,
        endpoint: crate::inbound_endpoint_rtmp::RtmpInboundEndpoint,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("rtmpInboundEndpoint"));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("rtmp_inbound_endpoint"),
            move |handle| endpoint.run(handle),
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_srt_inbound_endpoint(
        &self,
        endpoint: crate::inbound_endpoint_srt::SrtInboundEndpoint,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("srtInboundEndpoint"));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("srt_inbound_endpoint"),
            move |handle| endpoint.run(handle),
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_rtmp_outbound_endpoint(
        &self,
        endpoint: crate::outbound_endpoint_rtmp::RtmpOutboundEndpoint,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("rtmpOutboundEndpoint"));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("rtmp_outbound_endpoint"),
            move |handle| endpoint.run(handle),
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn create_rtsp_subscriber(
        &self,
        subscriber: crate::subscriber_rtsp::RtspSubscriber,
        processor_id: Option<ProcessorId>,
    ) -> Result<ProcessorId, PipelineOperationError> {
        let processor_id =
            processor_id.unwrap_or_else(|| ProcessorId::new(subscriber.input_url.clone()));
        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("rtsp_subscriber"),
            move |handle| subscriber.run(handle),
        )
        .await
        .map_err(|e| Self::map_register_error(e, &processor_id))?;
        Ok(processor_id)
    }

    pub async fn list_tracks(&self) -> Result<Vec<TrackId>, PipelineOperationError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(MediaPipelineCommand::ListTracks { reply_tx });
        reply_rx
            .await
            .map_err(|_| PipelineOperationError::PipelineTerminated)
    }

    pub async fn list_processor_ids(&self) -> Result<Vec<ProcessorId>, PipelineOperationError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(MediaPipelineCommand::ListProcessors { reply_tx });
        reply_rx
            .await
            .map_err(|_| PipelineOperationError::PipelineTerminated)
    }

    pub async fn wait_processor_terminated(
        &self,
        processor_id: ProcessorId,
    ) -> Result<(), PipelineOperationError> {
        loop {
            let processor_ids = self.list_processor_ids().await?;
            if !processor_ids.iter().any(|id| id == &processor_id) {
                return Ok(());
            }
            // 現状は e2e テスト用途を主眼にした簡易実装として、短い間隔でポーリングしている。
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use crate::media_pipeline::{
        MediaPipeline, MediaPipelineHandle, ProcessorId, ProcessorMetadata, TrackId,
    };

    const TEST_MP4_PATH: &str = "testdata/archive-red-320x320-av1.mp4";
    const TEST_MP4_AUDIO_PATH: &str = "testdata/red-320x320-h264-aac.mp4";

    // --- MP4 ファイルソース ---

    #[tokio::test]
    async fn create_mp4_file_source_uses_path_as_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let source = crate::obsws::source::file_mp4::Mp4FileSource {
            path: PathBuf::from(TEST_MP4_PATH),
            realtime: false,
            loop_playback: false,
            audio_track_id: None,
            video_track_id: Some(TrackId::new("video-default-id")),
        };

        let processor_id = handle
            .create_mp4_file_source(source, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), TEST_MP4_PATH);

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let source = crate::obsws::source::file_mp4::Mp4FileSource {
            path: PathBuf::from(TEST_MP4_PATH),
            realtime: false,
            loop_playback: false,
            audio_track_id: None,
            video_track_id: Some(TrackId::new("video-custom-id")),
        };

        let processor_id = handle
            .create_mp4_file_source(source, Some(ProcessorId::new("custom-source")))
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-source");

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let source1 = crate::obsws::source::file_mp4::Mp4FileSource {
            path: PathBuf::from(TEST_MP4_PATH),
            realtime: true,
            loop_playback: false,
            audio_track_id: None,
            video_track_id: Some(TrackId::new("video-duplicate-id")),
        };
        let source2 = crate::obsws::source::file_mp4::Mp4FileSource {
            path: PathBuf::from(TEST_MP4_PATH),
            realtime: true,
            loop_playback: false,
            audio_track_id: None,
            video_track_id: Some(TrackId::new("video-duplicate-id")),
        };

        let processor_id = handle
            .create_mp4_file_source(source1, Some(ProcessorId::new("duplicate-source")))
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-source");

        let result = handle
            .create_mp4_file_source(source2, Some(ProcessorId::new("duplicate-source")))
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    // --- MP4 ビデオリーダー ---

    #[tokio::test]
    async fn create_mp4_video_reader_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_mp4_video_reader(
                PathBuf::from(TEST_MP4_PATH),
                Some(ProcessorId::new("custom-mp4-video-reader")),
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-mp4-video-reader");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_mp4_video_reader_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_mp4_video_reader(
                PathBuf::from(TEST_MP4_PATH),
                Some(ProcessorId::new("duplicate-mp4-video-reader")),
            )
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-mp4-video-reader");

        let result = handle
            .create_mp4_video_reader(
                PathBuf::from(TEST_MP4_PATH),
                Some(ProcessorId::new("duplicate-mp4-video-reader")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- MP4 オーディオリーダー ---

    #[tokio::test]
    async fn create_mp4_audio_reader_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_mp4_audio_reader(
                PathBuf::from(TEST_MP4_AUDIO_PATH),
                Some(ProcessorId::new("custom-mp4-audio-reader")),
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-mp4-audio-reader");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_mp4_audio_reader_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_mp4_audio_reader(
                PathBuf::from(TEST_MP4_AUDIO_PATH),
                Some(ProcessorId::new("duplicate-mp4-audio-reader")),
            )
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-mp4-audio-reader");

        let result = handle
            .create_mp4_audio_reader(
                PathBuf::from(TEST_MP4_AUDIO_PATH),
                Some(ProcessorId::new("duplicate-mp4-audio-reader")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- MP4 ライター ---

    #[tokio::test]
    async fn create_mp4_writer_validates_params() {
        // inputAudioTrackId と inputVideoTrackId が両方 None の場合はエラー
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let result = handle
            .create_mp4_writer(PathBuf::from("out.mp4"), None, None, None)
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::InvalidParams(_))
        ));

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_writer_rejects_non_mp4_output_path() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let result = handle
            .create_mp4_writer(
                PathBuf::from("out.webm"),
                Some(TrackId::new("audio-input-track")),
                None,
                None,
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::InvalidParams(_))
        ));

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_writer_rejects_missing_output_parent_directory() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let missing_output_path = temp_dir.path().join("missing").join("out.mp4");

        let result = handle
            .create_mp4_writer(
                missing_output_path,
                Some(TrackId::new("audio-input-track")),
                None,
                None,
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::InvalidParams(_))
        ));

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_writer_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let output_path = tempfile::Builder::new()
            .suffix(".mp4")
            .tempfile()
            .expect("create temp file")
            .into_temp_path()
            .to_path_buf();

        let processor_id = handle
            .create_mp4_writer(
                output_path,
                Some(TrackId::new("audio-input-track")),
                None,
                None,
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "mp4Writer");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_mp4_writer_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let output_path = tempfile::Builder::new()
            .suffix(".mp4")
            .tempfile()
            .expect("create temp file")
            .into_temp_path()
            .to_path_buf();

        let processor_id = handle
            .create_mp4_writer(
                output_path,
                Some(TrackId::new("audio-input-track")),
                None,
                Some(ProcessorId::new("custom-mp4-writer")),
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-mp4-writer");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_mp4_writer_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let output_path = tempfile::Builder::new()
            .suffix(".mp4")
            .tempfile()
            .expect("create temp file")
            .into_temp_path()
            .to_path_buf();

        let processor_id = handle
            .create_mp4_writer(
                output_path.clone(),
                Some(TrackId::new("audio-input-track")),
                None,
                Some(ProcessorId::new("duplicate-mp4-writer")),
            )
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-mp4-writer");

        let result = handle
            .create_mp4_writer(
                output_path,
                Some(TrackId::new("audio-input-track")),
                None,
                Some(ProcessorId::new("duplicate-mp4-writer")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- オーディオデコーダー ---

    #[tokio::test]
    async fn create_audio_decoder_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_audio_decoder(
                TrackId::new("audio-input"),
                TrackId::new("audio-output"),
                None,
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "audioDecoder:audio-input");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_audio_decoder_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_audio_decoder(
                TrackId::new("audio-input"),
                TrackId::new("audio-output"),
                Some(ProcessorId::new("custom-audio-decoder")),
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-audio-decoder");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_audio_decoder_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_audio_decoder(
                TrackId::new("audio-input"),
                TrackId::new("audio-output"),
                Some(ProcessorId::new("duplicate-audio-decoder")),
            )
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-audio-decoder");

        let result = handle
            .create_audio_decoder(
                TrackId::new("audio-input"),
                TrackId::new("audio-output"),
                Some(ProcessorId::new("duplicate-audio-decoder")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- オーディオエンコーダー ---

    #[tokio::test]
    async fn create_audio_encoder_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_audio_encoder(
                TrackId::new("audio-input"),
                TrackId::new("audio-output"),
                crate::types::CodecName::Opus,
                std::num::NonZeroUsize::new(64000).unwrap(),
                None,
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "audioEncoder:audio-input");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_audio_encoder_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_audio_encoder(
                TrackId::new("audio-input"),
                TrackId::new("audio-output"),
                crate::types::CodecName::Opus,
                std::num::NonZeroUsize::new(64000).unwrap(),
                Some(ProcessorId::new("custom-audio-encoder")),
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-audio-encoder");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_audio_encoder_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_audio_encoder(
                TrackId::new("audio-input"),
                TrackId::new("audio-output"),
                crate::types::CodecName::Opus,
                std::num::NonZeroUsize::new(64000).unwrap(),
                Some(ProcessorId::new("duplicate-audio-encoder")),
            )
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-audio-encoder");

        let result = handle
            .create_audio_encoder(
                TrackId::new("audio-input"),
                TrackId::new("audio-output"),
                crate::types::CodecName::Opus,
                std::num::NonZeroUsize::new(64000).unwrap(),
                Some(ProcessorId::new("duplicate-audio-encoder")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- ビデオエンコーダー ---

    #[tokio::test]
    async fn create_video_encoder_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_video_encoder(
                TrackId::new("video-input"),
                TrackId::new("video-output"),
                crate::types::CodecName::H264,
                std::num::NonZeroUsize::new(2_000_000).unwrap(),
                crate::video::FrameRate::FPS_30,
                None,
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "videoEncoder:video-input");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_encoder_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_video_encoder(
                TrackId::new("video-input"),
                TrackId::new("video-output"),
                crate::types::CodecName::H264,
                std::num::NonZeroUsize::new(2_000_000).unwrap(),
                crate::video::FrameRate::FPS_30,
                Some(ProcessorId::new("custom-video-encoder")),
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-video-encoder");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_encoder_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_video_encoder(
                TrackId::new("video-input"),
                TrackId::new("video-output"),
                crate::types::CodecName::H264,
                std::num::NonZeroUsize::new(2_000_000).unwrap(),
                crate::video::FrameRate::FPS_30,
                Some(ProcessorId::new("duplicate-video-encoder")),
            )
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-video-encoder");

        let result = handle
            .create_video_encoder(
                TrackId::new("video-input"),
                TrackId::new("video-output"),
                crate::types::CodecName::H264,
                std::num::NonZeroUsize::new(2_000_000).unwrap(),
                crate::video::FrameRate::FPS_30,
                Some(ProcessorId::new("duplicate-video-encoder")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- ビデオデコーダー ---

    #[tokio::test]
    async fn create_video_decoder_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_video_decoder(
                TrackId::new("video-input"),
                TrackId::new("video-output"),
                None,
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "videoDecoder:video-input");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_decoder_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_video_decoder(
                TrackId::new("video-input"),
                TrackId::new("video-output"),
                Some(ProcessorId::new("custom-video-decoder")),
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-video-decoder");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_decoder_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_id = handle
            .create_video_decoder(
                TrackId::new("video-input"),
                TrackId::new("video-output"),
                Some(ProcessorId::new("duplicate-video-decoder")),
            )
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-video-decoder");

        let result = handle
            .create_video_decoder(
                TrackId::new("video-input"),
                TrackId::new("video-output"),
                Some(ProcessorId::new("duplicate-video-decoder")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- PNG ファイルソース ---

    #[tokio::test]
    async fn create_png_file_source_uses_path_as_default_processor_id() -> crate::Result<()> {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let png_file = create_test_png_file(2, 2, nopng::PixelFormat::Rgb8, &[0; 12])?;
        let file_path = png_file.path().to_path_buf();
        let source = crate::obsws::source::png_file::PngFileSource {
            path: file_path.clone(),
            frame_rate: crate::video::FrameRate::FPS_1,
            output_video_track_id: TrackId::new("png-video-default"),
        };

        let processor_id = handle
            .create_png_file_source(source, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), file_path.display().to_string());

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn create_png_file_source_uses_explicit_processor_id() -> crate::Result<()> {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let png_file = create_test_png_file(2, 2, nopng::PixelFormat::Rgb8, &[0; 12])?;
        let source = crate::obsws::source::png_file::PngFileSource {
            path: png_file.path().to_path_buf(),
            frame_rate: crate::video::FrameRate::FPS_1,
            output_video_track_id: TrackId::new("png-video-custom"),
        };

        let processor_id = handle
            .create_png_file_source(source, Some(ProcessorId::new("custom-png-source")))
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-png-source");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn create_png_file_source_rejects_duplicate_processor_id() -> crate::Result<()> {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let png_file = create_test_png_file(2, 2, nopng::PixelFormat::Rgba8, &[255; 16])?;
        let source1 = crate::obsws::source::png_file::PngFileSource {
            path: png_file.path().to_path_buf(),
            frame_rate: crate::video::FrameRate::FPS_1,
            output_video_track_id: TrackId::new("png-video-duplicate"),
        };
        let source2 = crate::obsws::source::png_file::PngFileSource {
            path: png_file.path().to_path_buf(),
            frame_rate: crate::video::FrameRate::FPS_1,
            output_video_track_id: TrackId::new("png-video-duplicate"),
        };

        let processor_id = handle
            .create_png_file_source(source1, Some(ProcessorId::new("duplicate-png-source")))
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-png-source");

        let result = handle
            .create_png_file_source(source2, Some(ProcessorId::new("duplicate-png-source")))
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
        Ok(())
    }

    // --- ビデオデバイスソース ---

    #[tokio::test]
    async fn create_video_device_source_uses_default_processor_id_for_default_device() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let source = crate::obsws::source::video_device::VideoDeviceSource {
            output_video_track_id: TrackId::new("video-device-output"),
            device_id: None,
            width: None,
            height: None,
            fps: None,
        };

        let processor_id = handle
            .create_video_device_source(source, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "videoDeviceSource:default");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_device_source_uses_default_processor_id_for_device_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let source = crate::obsws::source::video_device::VideoDeviceSource {
            output_video_track_id: TrackId::new("video-device-output"),
            device_id: Some("camera0".to_owned()),
            width: None,
            height: None,
            fps: None,
        };

        let processor_id = handle
            .create_video_device_source(source, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "videoDeviceSource:camera0");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_device_source_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let source = crate::obsws::source::video_device::VideoDeviceSource {
            output_video_track_id: TrackId::new("video-device-output"),
            device_id: None,
            width: None,
            height: None,
            fps: None,
        };

        let processor_id = handle
            .create_video_device_source(source, Some(ProcessorId::new("custom-video-device")))
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-video-device");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_device_source_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let source1 = crate::obsws::source::video_device::VideoDeviceSource {
            output_video_track_id: TrackId::new("video-device-output"),
            device_id: None,
            width: None,
            height: None,
            fps: None,
        };
        let source2 = crate::obsws::source::video_device::VideoDeviceSource {
            output_video_track_id: TrackId::new("video-device-output"),
            device_id: None,
            width: None,
            height: None,
            fps: None,
        };

        let processor_id = handle
            .create_video_device_source(source1, Some(ProcessorId::new("duplicate-video-device")))
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-video-device");

        let result = handle
            .create_video_device_source(source2, Some(ProcessorId::new("duplicate-video-device")))
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- オーディオミキサー ---

    #[tokio::test]
    async fn create_audio_mixer_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(
                ProcessorId::new("audio-mixer-blocker"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register audio-mixer-blocker");
        let occupied_sender = blocker
            .publish_track(TrackId::new("audio-mixer-output"))
            .await
            .expect("publish audio-mixer-output");
        let mixer = crate::mixer_realtime_audio::AudioRealtimeMixer {
            sample_rate: crate::audio::SampleRate::HZ_48000,
            channels: crate::audio::Channels::STEREO,
            frame_duration: Duration::from_millis(20),
            timestamp_rebase_threshold: Duration::from_millis(100),
            terminate_on_input_eos: true,
            input_tracks: vec![crate::mixer_realtime_audio::AudioRealtimeInputTrack {
                track_id: TrackId::new("audio-input-track"),
            }],
            output_track_id: TrackId::new("audio-mixer-output"),
        };

        let processor_id = handle
            .create_audio_mixer(mixer, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "audioMixer");

        drop(occupied_sender);
        drop(blocker);
        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_audio_mixer_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(
                ProcessorId::new("audio-mixer-blocker"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register audio-mixer-blocker");
        let occupied_sender = blocker
            .publish_track(TrackId::new("audio-mixer-output"))
            .await
            .expect("publish audio-mixer-output");
        let mixer = crate::mixer_realtime_audio::AudioRealtimeMixer {
            sample_rate: crate::audio::SampleRate::HZ_48000,
            channels: crate::audio::Channels::STEREO,
            frame_duration: Duration::from_millis(20),
            timestamp_rebase_threshold: Duration::from_millis(100),
            terminate_on_input_eos: true,
            input_tracks: vec![crate::mixer_realtime_audio::AudioRealtimeInputTrack {
                track_id: TrackId::new("audio-input-track"),
            }],
            output_track_id: TrackId::new("audio-mixer-output"),
        };

        let processor_id = handle
            .create_audio_mixer(mixer, Some(ProcessorId::new("custom-audio-mixer")))
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-audio-mixer");

        drop(occupied_sender);
        drop(blocker);
        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_audio_mixer_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(
                ProcessorId::new("audio-mixer-blocker"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register audio-mixer-blocker");
        let occupied_sender = blocker
            .publish_track(TrackId::new("audio-mixer-output-dup"))
            .await
            .expect("publish audio-mixer-output-dup");
        let mixer1 = crate::mixer_realtime_audio::AudioRealtimeMixer {
            sample_rate: crate::audio::SampleRate::HZ_48000,
            channels: crate::audio::Channels::STEREO,
            frame_duration: Duration::from_millis(20),
            timestamp_rebase_threshold: Duration::from_millis(100),
            terminate_on_input_eos: true,
            input_tracks: vec![crate::mixer_realtime_audio::AudioRealtimeInputTrack {
                track_id: TrackId::new("audio-input-track"),
            }],
            output_track_id: TrackId::new("audio-mixer-output-dup"),
        };
        let mixer2 = crate::mixer_realtime_audio::AudioRealtimeMixer {
            sample_rate: crate::audio::SampleRate::HZ_48000,
            channels: crate::audio::Channels::STEREO,
            frame_duration: Duration::from_millis(20),
            timestamp_rebase_threshold: Duration::from_millis(100),
            terminate_on_input_eos: true,
            input_tracks: vec![crate::mixer_realtime_audio::AudioRealtimeInputTrack {
                track_id: TrackId::new("audio-input-track"),
            }],
            output_track_id: TrackId::new("audio-mixer-output-dup"),
        };

        let processor_id = handle
            .create_audio_mixer(mixer1, Some(ProcessorId::new("duplicate-audio-mixer")))
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-audio-mixer");

        let result = handle
            .create_audio_mixer(mixer2, Some(ProcessorId::new("duplicate-audio-mixer")))
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(occupied_sender);
        drop(blocker);
        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- オーディオミキサー入力更新 ---

    #[tokio::test]
    async fn update_audio_mixer_inputs_rejects_unknown_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let result = handle
            .update_audio_mixer_inputs(
                ProcessorId::new("unknown-audio-mixer"),
                vec![crate::mixer_realtime_audio::AudioRealtimeInputTrack {
                    track_id: TrackId::new("audio-a"),
                }],
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::InvalidParams(_))
        ));

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn update_audio_mixer_inputs_returns_previous_input_tracks() {
        // spawn_test_pipeline() は内部で trigger_start() を実行済み。
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let mixer = crate::mixer_realtime_audio::AudioRealtimeMixer {
            sample_rate: crate::audio::SampleRate::HZ_48000,
            channels: crate::audio::Channels::STEREO,
            frame_duration: Duration::from_millis(20),
            timestamp_rebase_threshold: Duration::from_millis(100),
            terminate_on_input_eos: true,
            input_tracks: vec![crate::mixer_realtime_audio::AudioRealtimeInputTrack {
                track_id: TrackId::new("audio-input-track"),
            }],
            output_track_id: TrackId::new("audio-mixer-update-output"),
        };

        let processor_id = handle
            .create_audio_mixer(mixer, Some(ProcessorId::new("updatable-audio-mixer")))
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "updatable-audio-mixer");

        let previous_input_tracks = handle
            .update_audio_mixer_inputs(
                ProcessorId::new("updatable-audio-mixer"),
                vec![
                    crate::mixer_realtime_audio::AudioRealtimeInputTrack {
                        track_id: TrackId::new("audio-input-a"),
                    },
                    crate::mixer_realtime_audio::AudioRealtimeInputTrack {
                        track_id: TrackId::new("audio-input-b"),
                    },
                ],
            )
            .await
            .expect("must succeed");

        let previous_track_ids: Vec<String> = previous_input_tracks
            .iter()
            .map(|t| t.track_id.get().to_owned())
            .collect();
        assert_eq!(previous_track_ids, vec!["audio-input-track".to_owned()]);

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- ビデオミキサー ---

    #[tokio::test]
    async fn create_video_mixer_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(
                ProcessorId::new("video-mixer-blocker"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register video-mixer-blocker");
        let occupied_sender = blocker
            .publish_track(TrackId::new("video-mixer-output"))
            .await
            .expect("publish video-mixer-output");
        let mixer = crate::mixer_realtime_video::VideoRealtimeMixer {
            canvas_width: crate::types::EvenUsize::new(640).unwrap(),
            canvas_height: crate::types::EvenUsize::new(480).unwrap(),
            frame_rate: crate::video::FrameRate::FPS_30,
            input_tracks: vec![crate::mixer_realtime_video::InputTrack {
                track_id: TrackId::new("video-input-track"),
                x: 0,
                y: 0,
                z: 0,
                width: None,
                height: None,
                scale_x: None,
                scale_y: None,
                crop_top: 0,
                crop_bottom: 0,
                crop_left: 0,
                crop_right: 0,
            }],
            output_track_id: TrackId::new("video-mixer-output"),
        };

        let processor_id = handle
            .create_video_mixer(mixer, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "videoMixer");

        drop(occupied_sender);
        drop(blocker);
        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_mixer_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(
                ProcessorId::new("video-mixer-blocker"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register video-mixer-blocker");
        let occupied_sender = blocker
            .publish_track(TrackId::new("video-mixer-output"))
            .await
            .expect("publish video-mixer-output");
        let mixer = crate::mixer_realtime_video::VideoRealtimeMixer {
            canvas_width: crate::types::EvenUsize::new(640).unwrap(),
            canvas_height: crate::types::EvenUsize::new(480).unwrap(),
            frame_rate: crate::video::FrameRate::FPS_30,
            input_tracks: vec![crate::mixer_realtime_video::InputTrack {
                track_id: TrackId::new("video-input-track"),
                x: 0,
                y: 0,
                z: 0,
                width: None,
                height: None,
                scale_x: None,
                scale_y: None,
                crop_top: 0,
                crop_bottom: 0,
                crop_left: 0,
                crop_right: 0,
            }],
            output_track_id: TrackId::new("video-mixer-output"),
        };

        let processor_id = handle
            .create_video_mixer(mixer, Some(ProcessorId::new("custom-video-mixer")))
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-video-mixer");

        drop(occupied_sender);
        drop(blocker);
        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_mixer_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let mixer1 = crate::mixer_realtime_video::VideoRealtimeMixer {
            canvas_width: crate::types::EvenUsize::new(640).unwrap(),
            canvas_height: crate::types::EvenUsize::new(480).unwrap(),
            frame_rate: crate::video::FrameRate::FPS_30,
            input_tracks: vec![crate::mixer_realtime_video::InputTrack {
                track_id: TrackId::new("video-input-track"),
                x: 0,
                y: 0,
                z: 0,
                width: None,
                height: None,
                scale_x: None,
                scale_y: None,
                crop_top: 0,
                crop_bottom: 0,
                crop_left: 0,
                crop_right: 0,
            }],
            output_track_id: TrackId::new("video-mixer-output-dup"),
        };
        let mixer2 = crate::mixer_realtime_video::VideoRealtimeMixer {
            canvas_width: crate::types::EvenUsize::new(640).unwrap(),
            canvas_height: crate::types::EvenUsize::new(480).unwrap(),
            frame_rate: crate::video::FrameRate::FPS_30,
            input_tracks: vec![crate::mixer_realtime_video::InputTrack {
                track_id: TrackId::new("video-input-track"),
                x: 0,
                y: 0,
                z: 0,
                width: None,
                height: None,
                scale_x: None,
                scale_y: None,
                crop_top: 0,
                crop_bottom: 0,
                crop_left: 0,
                crop_right: 0,
            }],
            output_track_id: TrackId::new("video-mixer-output-dup"),
        };

        let processor_id = handle
            .create_video_mixer(mixer1, Some(ProcessorId::new("duplicate-video-mixer")))
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-video-mixer");

        let result = handle
            .create_video_mixer(mixer2, Some(ProcessorId::new("duplicate-video-mixer")))
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- ビデオミキサー更新 ---

    #[tokio::test]
    async fn update_video_mixer_rejects_unknown_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = crate::mixer_realtime_video::VideoRealtimeMixerUpdateConfigRequest {
            canvas_width: crate::types::EvenUsize::new(1280).unwrap(),
            canvas_height: crate::types::EvenUsize::new(720).unwrap(),
            frame_rate: crate::video::FrameRate::FPS_25,
            input_tracks: vec![crate::mixer_realtime_video::InputTrack {
                track_id: TrackId::new("video-a"),
                x: 0,
                y: 0,
                z: 0,
                width: None,
                height: None,
                scale_x: None,
                scale_y: None,
                crop_top: 0,
                crop_bottom: 0,
                crop_left: 0,
                crop_right: 0,
            }],
        };

        let result = handle
            .update_video_mixer(ProcessorId::new("unknown-video-mixer"), request)
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::InvalidParams(_))
        ));

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn update_video_mixer_returns_previous_config() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let mixer = crate::mixer_realtime_video::VideoRealtimeMixer {
            canvas_width: crate::types::EvenUsize::new(640).unwrap(),
            canvas_height: crate::types::EvenUsize::new(480).unwrap(),
            frame_rate: crate::video::FrameRate::FPS_30,
            input_tracks: vec![crate::mixer_realtime_video::InputTrack {
                track_id: TrackId::new("video-input-track"),
                x: 0,
                y: 0,
                z: 0,
                width: None,
                height: None,
                scale_x: None,
                scale_y: None,
                crop_top: 0,
                crop_bottom: 0,
                crop_left: 0,
                crop_right: 0,
            }],
            output_track_id: TrackId::new("video-mixer-update-config-output"),
        };

        let processor_id = handle
            .create_video_mixer(
                mixer,
                Some(ProcessorId::new("updatable-video-mixer-config")),
            )
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "updatable-video-mixer-config");

        let update_request = crate::mixer_realtime_video::VideoRealtimeMixerUpdateConfigRequest {
            canvas_width: crate::types::EvenUsize::new(800).unwrap(),
            canvas_height: crate::types::EvenUsize::new(600).unwrap(),
            frame_rate: "30000/1001".parse::<crate::video::FrameRate>().unwrap(),
            input_tracks: vec![
                crate::mixer_realtime_video::InputTrack {
                    track_id: TrackId::new("video-input-a"),
                    x: 10,
                    y: 20,
                    z: 0,
                    width: None,
                    height: None,
                    scale_x: None,
                    scale_y: None,
                    crop_top: 0,
                    crop_bottom: 0,
                    crop_left: 0,
                    crop_right: 0,
                },
                crate::mixer_realtime_video::InputTrack {
                    track_id: TrackId::new("video-input-b"),
                    x: 100,
                    y: 50,
                    z: 1,
                    width: Some(crate::types::EvenUsize::new(320).unwrap()),
                    height: Some(crate::types::EvenUsize::new(180).unwrap()),
                    scale_x: None,
                    scale_y: None,
                    crop_top: 0,
                    crop_bottom: 0,
                    crop_left: 0,
                    crop_right: 0,
                },
            ],
        };

        let result = handle
            .update_video_mixer(
                ProcessorId::new("updatable-video-mixer-config"),
                update_request,
            )
            .await
            .expect("must succeed");

        assert_eq!(result.previous_canvas_width, 640);
        assert_eq!(result.previous_canvas_height, 480);
        assert_eq!(result.previous_frame_rate, crate::video::FrameRate::FPS_30);
        let previous_track_ids: Vec<String> = result
            .previous_input_tracks
            .iter()
            .map(|t| t.track_id.get().to_owned())
            .collect();
        assert_eq!(previous_track_ids, vec!["video-input-track".to_owned()]);

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- RTMP パブリッシャー ---

    #[tokio::test]
    async fn create_rtmp_publisher_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let publisher = crate::publisher_rtmp::RtmpPublisher {
            output_url: "rtmp://127.0.0.1:1935/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            input_audio_track_id: None,
            input_video_track_id: Some(TrackId::new("video-main")),
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_publisher(publisher, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "rtmpPublisher");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_publisher_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let publisher = crate::publisher_rtmp::RtmpPublisher {
            output_url: "rtmp://127.0.0.1:1935/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            input_audio_track_id: None,
            input_video_track_id: Some(TrackId::new("video-main")),
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_publisher(publisher, Some(ProcessorId::new("custom-rtmp-publisher")))
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-rtmp-publisher");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_publisher_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let publisher1 = crate::publisher_rtmp::RtmpPublisher {
            output_url: "rtmp://127.0.0.1:1935/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            input_audio_track_id: None,
            input_video_track_id: Some(TrackId::new("video-main")),
            options: Default::default(),
        };
        let publisher2 = crate::publisher_rtmp::RtmpPublisher {
            output_url: "rtmp://127.0.0.1:1935/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            input_audio_track_id: None,
            input_video_track_id: Some(TrackId::new("video-main")),
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_publisher(
                publisher1,
                Some(ProcessorId::new("duplicate-rtmp-publisher")),
            )
            .await
            .expect("must succeed");
        assert_eq!(processor_id.get(), "duplicate-rtmp-publisher");

        let result = handle
            .create_rtmp_publisher(
                publisher2,
                Some(ProcessorId::new("duplicate-rtmp-publisher")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- RTMP インバウンドエンドポイント ---

    #[tokio::test]
    async fn create_rtmp_inbound_endpoint_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::inbound_endpoint_rtmp::RtmpInboundEndpoint {
            input_url: "rtmp://127.0.0.1:1935/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            output_audio_track_id: Some(TrackId::new("audio-main")),
            output_video_track_id: None,
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_inbound_endpoint(endpoint, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "rtmpInboundEndpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_inbound_endpoint_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::inbound_endpoint_rtmp::RtmpInboundEndpoint {
            input_url: "rtmp://127.0.0.1:1935/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            output_audio_track_id: Some(TrackId::new("audio-main")),
            output_video_track_id: Some(TrackId::new("video-main")),
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_inbound_endpoint(
                endpoint,
                Some(ProcessorId::new("custom-rtmp-inbound-endpoint")),
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-rtmp-inbound-endpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_inbound_endpoint_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(
                ProcessorId::new("duplicate-rtmp-inbound-endpoint"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register duplicate-rtmp-inbound-endpoint");
        let endpoint = crate::inbound_endpoint_rtmp::RtmpInboundEndpoint {
            input_url: "rtmp://127.0.0.1:1935/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            output_audio_track_id: Some(TrackId::new("audio-main")),
            output_video_track_id: Some(TrackId::new("video-main")),
            options: Default::default(),
        };

        let result = handle
            .create_rtmp_inbound_endpoint(
                endpoint,
                Some(ProcessorId::new("duplicate-rtmp-inbound-endpoint")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(blocker);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_rtmp_inbound_endpoint_accepts_audio_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::inbound_endpoint_rtmp::RtmpInboundEndpoint {
            input_url: "rtmp://127.0.0.1:1935/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            output_audio_track_id: Some(TrackId::new("audio-main")),
            output_video_track_id: None,
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_inbound_endpoint(endpoint, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "rtmpInboundEndpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_inbound_endpoint_accepts_video_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::inbound_endpoint_rtmp::RtmpInboundEndpoint {
            input_url: "rtmp://127.0.0.1:1935/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            output_audio_track_id: None,
            output_video_track_id: Some(TrackId::new("video-main")),
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_inbound_endpoint(endpoint, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "rtmpInboundEndpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- SRT インバウンドエンドポイント ---

    #[tokio::test]
    async fn create_srt_inbound_endpoint_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::inbound_endpoint_srt::SrtInboundEndpoint {
            input_url: "srt://127.0.0.1:10080".to_owned(),
            output_audio_track_id: Some(TrackId::new("audio-main")),
            output_video_track_id: None,
            stream_id: None,
            passphrase: None,
            key_length: None,
            tsbpd_delay_ms: None,
        };

        let processor_id = handle
            .create_srt_inbound_endpoint(endpoint, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "srtInboundEndpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_srt_inbound_endpoint_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::inbound_endpoint_srt::SrtInboundEndpoint {
            input_url: "srt://127.0.0.1:10080".to_owned(),
            output_audio_track_id: Some(TrackId::new("audio-main")),
            output_video_track_id: Some(TrackId::new("video-main")),
            stream_id: None,
            passphrase: None,
            key_length: None,
            tsbpd_delay_ms: None,
        };

        let processor_id = handle
            .create_srt_inbound_endpoint(
                endpoint,
                Some(ProcessorId::new("custom-srt-inbound-endpoint")),
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-srt-inbound-endpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_srt_inbound_endpoint_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(
                ProcessorId::new("duplicate-srt-inbound-endpoint"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register duplicate-srt-inbound-endpoint");
        let endpoint = crate::inbound_endpoint_srt::SrtInboundEndpoint {
            input_url: "srt://127.0.0.1:10080".to_owned(),
            output_audio_track_id: Some(TrackId::new("audio-main")),
            output_video_track_id: Some(TrackId::new("video-main")),
            stream_id: None,
            passphrase: None,
            key_length: None,
            tsbpd_delay_ms: None,
        };

        let result = handle
            .create_srt_inbound_endpoint(
                endpoint,
                Some(ProcessorId::new("duplicate-srt-inbound-endpoint")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(blocker);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_srt_inbound_endpoint_accepts_audio_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::inbound_endpoint_srt::SrtInboundEndpoint {
            input_url: "srt://127.0.0.1:10080".to_owned(),
            output_audio_track_id: Some(TrackId::new("audio-main")),
            output_video_track_id: None,
            stream_id: None,
            passphrase: None,
            key_length: None,
            tsbpd_delay_ms: None,
        };

        let processor_id = handle
            .create_srt_inbound_endpoint(endpoint, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "srtInboundEndpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_srt_inbound_endpoint_accepts_video_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::inbound_endpoint_srt::SrtInboundEndpoint {
            input_url: "srt://127.0.0.1:10080".to_owned(),
            output_audio_track_id: None,
            output_video_track_id: Some(TrackId::new("video-main")),
            stream_id: None,
            passphrase: None,
            key_length: None,
            tsbpd_delay_ms: None,
        };

        let processor_id = handle
            .create_srt_inbound_endpoint(endpoint, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "srtInboundEndpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- RTMP アウトバウンドエンドポイント ---

    #[tokio::test]
    async fn create_rtmp_outbound_endpoint_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::outbound_endpoint_rtmp::RtmpOutboundEndpoint {
            output_url: "rtmp://127.0.0.1:29350/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            input_audio_track_id: Some(TrackId::new("audio-main")),
            input_video_track_id: None,
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_outbound_endpoint(endpoint, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "rtmpOutboundEndpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_outbound_endpoint_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::outbound_endpoint_rtmp::RtmpOutboundEndpoint {
            output_url: "rtmp://127.0.0.1:29350/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            input_audio_track_id: Some(TrackId::new("audio-main")),
            input_video_track_id: Some(TrackId::new("video-main")),
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_outbound_endpoint(
                endpoint,
                Some(ProcessorId::new("custom-rtmp-outbound-endpoint")),
            )
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-rtmp-outbound-endpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_outbound_endpoint_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(
                ProcessorId::new("duplicate-rtmp-outbound-endpoint"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register duplicate-rtmp-outbound-endpoint");
        let endpoint = crate::outbound_endpoint_rtmp::RtmpOutboundEndpoint {
            output_url: "rtmp://127.0.0.1:29350/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            input_audio_track_id: Some(TrackId::new("audio-main")),
            input_video_track_id: Some(TrackId::new("video-main")),
            options: Default::default(),
        };

        let result = handle
            .create_rtmp_outbound_endpoint(
                endpoint,
                Some(ProcessorId::new("duplicate-rtmp-outbound-endpoint")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(blocker);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_rtmp_outbound_endpoint_accepts_audio_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::outbound_endpoint_rtmp::RtmpOutboundEndpoint {
            output_url: "rtmp://127.0.0.1:29350/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            input_audio_track_id: Some(TrackId::new("audio-main")),
            input_video_track_id: None,
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_outbound_endpoint(endpoint, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "rtmpOutboundEndpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_outbound_endpoint_accepts_video_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let endpoint = crate::outbound_endpoint_rtmp::RtmpOutboundEndpoint {
            output_url: "rtmp://127.0.0.1:29350/live".to_owned(),
            stream_name: Some("stream-main".to_owned()),
            input_audio_track_id: None,
            input_video_track_id: Some(TrackId::new("video-main")),
            options: Default::default(),
        };

        let processor_id = handle
            .create_rtmp_outbound_endpoint(endpoint, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "rtmpOutboundEndpoint");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    // --- RTSP サブスクライバー ---

    #[tokio::test]
    async fn create_rtsp_subscriber_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let subscriber = crate::subscriber_rtsp::RtspSubscriber {
            input_url: "rtsp://example.com/live".to_owned(),
            output_video_track_id: Some(TrackId::new("video-main")),
            output_audio_track_id: None,
        };

        let processor_id = handle
            .create_rtsp_subscriber(subscriber, None)
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "rtsp://example.com/live");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtsp_subscriber_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let subscriber = crate::subscriber_rtsp::RtspSubscriber {
            input_url: "rtsp://example.com/live".to_owned(),
            output_video_track_id: Some(TrackId::new("video-main")),
            output_audio_track_id: Some(TrackId::new("audio-main")),
        };

        let processor_id = handle
            .create_rtsp_subscriber(subscriber, Some(ProcessorId::new("custom-rtsp-subscriber")))
            .await
            .expect("must succeed");

        assert_eq!(processor_id.get(), "custom-rtsp-subscriber");

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtsp_subscriber_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(
                ProcessorId::new("duplicate-rtsp-subscriber"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register duplicate-rtsp-subscriber");
        let subscriber = crate::subscriber_rtsp::RtspSubscriber {
            input_url: "rtsp://example.com/live".to_owned(),
            output_video_track_id: Some(TrackId::new("video-main")),
            output_audio_track_id: None,
        };

        let result = handle
            .create_rtsp_subscriber(
                subscriber,
                Some(ProcessorId::new("duplicate-rtsp-subscriber")),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::PipelineOperationError::DuplicateProcessorId(_))
        ));

        drop(blocker);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    // --- listProcessors ---

    #[tokio::test]
    async fn list_processors_returns_empty_array_when_no_processors() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let processor_ids = handle.list_processor_ids().await.expect("must succeed");
        assert!(processor_ids.is_empty());

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn list_processors_returns_registered_processors() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let processor_a = handle
            .register_processor(
                ProcessorId::new("list-processor-a"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register list-processor-a");
        let processor_b = handle
            .register_processor(
                ProcessorId::new("list-processor-b"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register list-processor-b");

        let processor_ids = handle.list_processor_ids().await.expect("must succeed");
        let id_strings: Vec<&str> = processor_ids.iter().map(|id| id.get()).collect();

        assert!(id_strings.contains(&"list-processor-a"));
        assert!(id_strings.contains(&"list-processor-b"));

        drop(processor_a);
        drop(processor_b);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    // --- listTracks ---

    #[tokio::test]
    async fn list_tracks_returns_empty_array_when_no_tracks() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        let track_ids = handle.list_tracks().await.expect("must succeed");
        assert!(track_ids.is_empty());

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn list_tracks_returns_created_tracks() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let publisher = handle
            .register_processor(
                ProcessorId::new("list-tracks-publisher"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register list-tracks-publisher");
        publisher
            .publish_track(TrackId::new("list-track-a"))
            .await
            .expect("publish list-track-a");
        publisher
            .publish_track(TrackId::new("list-track-b"))
            .await
            .expect("publish list-track-b");

        let track_ids = handle.list_tracks().await.expect("must succeed");
        let id_strings: Vec<&str> = track_ids.iter().map(|id| id.get()).collect();

        assert!(id_strings.contains(&"list-track-a"));
        assert!(id_strings.contains(&"list-track-b"));

        drop(publisher);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    // --- triggerStart ---

    #[tokio::test]
    async fn trigger_start_succeeds_first_call() {
        let (handle, pipeline_task) = spawn_test_pipeline_without_start().await;

        let started = handle
            .trigger_start()
            .await
            .expect("trigger_start must succeed");
        assert!(started, "triggerStart must start pipeline on first call");

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn trigger_start_returns_false_when_pipeline_already_started() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        // spawn_test_pipeline() は内部で trigger_start() を実行済みなので、2 回目は false を返す
        let started = handle
            .trigger_start()
            .await
            .expect("trigger_start must succeed");
        assert!(!started);

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    // --- waitProcessorTerminated ---

    #[tokio::test]
    async fn wait_processor_terminated_returns_ok_when_processor_absent() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;

        // 存在しないプロセッサーに対しては即座に Ok(()) を返す
        handle
            .wait_processor_terminated(ProcessorId::new("missing-processor"))
            .await
            .expect("must succeed");

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn wait_processor_terminated_waits_until_terminated() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(
                ProcessorId::new("alive-processor"),
                ProcessorMetadata::new("test_processor"),
            )
            .await
            .expect("register alive-processor");

        // プロセッサーが生きている間はタイムアウトするはず
        let wait_result = tokio::time::timeout(
            Duration::from_millis(50),
            handle.wait_processor_terminated(ProcessorId::new("alive-processor")),
        )
        .await;
        assert!(
            wait_result.is_err(),
            "must keep waiting while processor is alive"
        );

        // プロセッサーを終了させる
        drop(blocker);

        // 終了後は成功するはず
        tokio::time::timeout(
            Duration::from_secs(5),
            handle.wait_processor_terminated(ProcessorId::new("alive-processor")),
        )
        .await
        .expect("wait timed out")
        .expect("must succeed");

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    // --- ヘルパー関数 ---

    async fn spawn_test_pipeline() -> (MediaPipelineHandle, tokio::task::JoinHandle<()>) {
        let (handle, pipeline_task) = spawn_test_pipeline_without_start().await;
        assert!(
            handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );
        (handle, pipeline_task)
    }

    async fn spawn_test_pipeline_without_start()
    -> (MediaPipelineHandle, tokio::task::JoinHandle<()>) {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        (handle, pipeline_task)
    }

    fn create_test_png_file(
        width: u32,
        height: u32,
        pixel_format: nopng::PixelFormat,
        data: &[u8],
    ) -> crate::Result<tempfile::NamedTempFile> {
        let spec = nopng::ImageSpec::new(width, height, pixel_format);
        let png_bytes =
            nopng::encode_image(&spec, data).map_err(|e| crate::Error::new(e.to_string()))?;
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), &png_bytes)?;
        Ok(file)
    }
}
