// NOTE: 長いので MediaPipelineHandle のパイプライン操作メソッドはこっちで実装している

use crate::media_pipeline::{
    MediaPipelineCommand, MediaPipelineHandle, PROCESSOR_TYPE_VIDEO_ENCODER,
    PipelineOperationError, ProcessorId, ProcessorMetadata, RegisterProcessorError, TrackId,
};

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
}
