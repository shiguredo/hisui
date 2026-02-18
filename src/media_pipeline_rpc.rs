// NOTE: 長いので MediaPipelineHandle の RPC 関連の処理はこっちで実装している

use crate::media_pipeline::{
    MediaPipelineCommand, MediaPipelineHandle, ProcessorId, RegisterProcessorError, TrackId,
};

type RpcError = (i32, String);

fn parse_params<F, T>(
    maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    f: F,
) -> Result<T, RpcError>
where
    F: FnOnce(nojson::RawJsonValue<'_, '_>) -> Result<T, nojson::JsonParseError>,
{
    let params =
        maybe_params.ok_or_else(|| invalid_params("Invalid params: params is required"))?;
    f(params).map_err(|e| invalid_params(format!("Invalid params: {e}")))
}

fn invalid_params(message: impl Into<String>) -> RpcError {
    (crate::jsonrpc::INVALID_PARAMS, message.into())
}

fn method_not_found() -> RpcError {
    (
        crate::jsonrpc::METHOD_NOT_FOUND,
        "Method not found".to_owned(),
    )
}

fn internal_error(message: impl Into<String>) -> RpcError {
    (crate::jsonrpc::INTERNAL_ERROR, message.into())
}

impl MediaPipelineHandle {
    // JSON-RPC リクエストを処理する
    //
    // 通知の場合は None が、それ以外ならクライアントに返すレスポンス JSON が返される
    pub async fn rpc(&self, request_bytes: &[u8]) -> Option<nojson::RawJsonOwned> {
        let request_json = match crate::jsonrpc::parse_request_bytes(request_bytes) {
            Err(error_response) => return Some(error_response),
            Ok(json) => json,
        };
        let request = request_json.value();

        // parse_request_bytes() の中でバリデーションしているので、ここは常に成功する
        let method = request
            .to_member("method")
            .expect("bug")
            .required()
            .expect("bug")
            .as_string_str()
            .expect("bug");
        let maybe_id = request.to_member("id").ok().and_then(|v| v.get());
        let maybe_params = request.to_member("params").ok().and_then(|v| v.get());

        let result = match method {
            "createMp4FileSource" => self.handle_create_mp4_file_source_rpc(maybe_params).await,
            "createPngFileSource" => self.handle_create_png_file_source_rpc(maybe_params).await,
            "createVideoDeviceSource" => {
                self.handle_create_video_device_source_rpc(maybe_params)
                    .await
            }
            "createVideoMixer" => self.handle_create_video_mixer_rpc(maybe_params).await,
            "createWhipPublisher" => self.handle_create_whip_publisher_rpc(maybe_params).await,
            "createWhepSubscriber" => self.handle_create_whep_subscriber_rpc(maybe_params).await,
            "listTracks" => self.handle_list_tracks_rpc().await,
            "listProcessors" => self.handle_list_processors_rpc().await,
            _ => Err(method_not_found()),
        };

        if let Some(id) = maybe_id {
            Some(match result {
                Ok(v) => crate::jsonrpc::ok_response(id, v),
                Err((code, e)) => crate::jsonrpc::error_response(id, code, e),
            })
        } else {
            if let Err((code, message)) = result {
                tracing::warn!(
                    "rpc notification failed: method={method}, code={code}, message={message}"
                );
            }
            None
        }
    }

    async fn handle_create_mp4_file_source_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (source, processor_id): (crate::Mp4FileSource, Option<ProcessorId>) =
            parse_params(maybe_params, |params| {
                let source = params.try_into()?;
                let processor_id = params.to_member("processorId")?.try_into()?;
                Ok((source, processor_id))
            })?;
        let processor_id =
            processor_id.unwrap_or_else(|| ProcessorId::new(source.path.display().to_string()));

        self.spawn_processor(processor_id.clone(), move |handle| source.run(handle))
            .await
            .map_err(|e| match e {
                RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                    "Invalid params: processorId already exists: {processor_id}"
                )),
                RegisterProcessorError::PipelineTerminated => {
                    internal_error("Internal error: pipeline has terminated".to_owned())
                }
            })?;

        Ok(RpcSuccessResult::CreateMp4FileSource { processor_id })
    }

    async fn handle_create_png_file_source_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (source, processor_id): (crate::PngFileSource, Option<ProcessorId>) =
            parse_params(maybe_params, |params| {
                let source = params.try_into()?;
                let processor_id = params.to_member("processorId")?.try_into()?;
                Ok((source, processor_id))
            })?;
        let processor_id =
            processor_id.unwrap_or_else(|| ProcessorId::new(source.path.display().to_string()));

        self.spawn_processor(processor_id.clone(), move |handle| source.run(handle))
            .await
            .map_err(|e| match e {
                RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                    "Invalid params: processorId already exists: {processor_id}"
                )),
                RegisterProcessorError::PipelineTerminated => {
                    internal_error("Internal error: pipeline has terminated".to_owned())
                }
            })?;

        Ok(RpcSuccessResult::CreatePngFileSource { processor_id })
    }

    async fn handle_create_video_device_source_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (source, processor_id): (crate::VideoDeviceSource, Option<ProcessorId>) =
            parse_params(maybe_params, |params| {
                let source = params.try_into()?;
                let processor_id = params.to_member("processorId")?.try_into()?;
                Ok((source, processor_id))
            })?;
        let processor_id = processor_id.unwrap_or_else(|| {
            if let Some(device_id) = source.device_id.as_deref() {
                ProcessorId::new(format!("videoDeviceSource:{device_id}"))
            } else {
                ProcessorId::new("videoDeviceSource:default")
            }
        });

        self.spawn_processor(processor_id.clone(), move |handle| source.run(handle))
            .await
            .map_err(|e| match e {
                RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                    "Invalid params: processorId already exists: {processor_id}"
                )),
                RegisterProcessorError::PipelineTerminated => {
                    internal_error("Internal error: pipeline has terminated".to_owned())
                }
            })?;

        Ok(RpcSuccessResult::CreateVideoDeviceSource { processor_id })
    }

    async fn handle_create_video_mixer_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (mixer, processor_id): (
            crate::mixer_realtime_video::VideoRealtimeMixer,
            Option<ProcessorId>,
        ) = parse_params(maybe_params, |params| {
            let mixer = params.try_into()?;
            let processor_id = params.to_member("processorId")?.try_into()?;
            Ok((mixer, processor_id))
        })?;
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("videoMixer"));

        self.spawn_processor(processor_id.clone(), move |handle| mixer.run(handle))
            .await
            .map_err(|e| match e {
                RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                    "Invalid params: processorId already exists: {processor_id}"
                )),
                RegisterProcessorError::PipelineTerminated => {
                    internal_error("Internal error: pipeline has terminated".to_owned())
                }
            })?;

        Ok(RpcSuccessResult::CreateVideoMixer { processor_id })
    }

    async fn handle_create_whip_publisher_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (publisher, processor_id): (crate::publisher_whip::WhipPublisher, Option<ProcessorId>) =
            parse_params(maybe_params, |params| {
                let publisher = params.try_into()?;
                let processor_id = params.to_member("processorId")?.try_into()?;
                Ok((publisher, processor_id))
            })?;
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("whipPublisher"));

        self.spawn_local_processor(processor_id.clone(), move |handle| publisher.run(handle))
            .await
            .map_err(|e| match e {
                RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                    "Invalid params: processorId already exists: {processor_id}"
                )),
                RegisterProcessorError::PipelineTerminated => {
                    internal_error("Internal error: pipeline has terminated".to_owned())
                }
            })?;

        Ok(RpcSuccessResult::CreateWhipPublisher { processor_id })
    }

    async fn handle_create_whep_subscriber_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (subscriber, processor_id): (
            crate::subscriber_whep::WhepSubscriber,
            Option<ProcessorId>,
        ) = parse_params(maybe_params, |params| {
            let subscriber = params.try_into()?;
            let processor_id = params.to_member("processorId")?.try_into()?;
            Ok((subscriber, processor_id))
        })?;
        let processor_id =
            processor_id.unwrap_or_else(|| ProcessorId::new(subscriber.input_url.clone()));

        self.spawn_local_processor(processor_id.clone(), move |handle| subscriber.run(handle))
            .await
            .map_err(|e| match e {
                RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                    "Invalid params: processorId already exists: {processor_id}"
                )),
                RegisterProcessorError::PipelineTerminated => {
                    internal_error("Internal error: pipeline has terminated".to_owned())
                }
            })?;

        Ok(RpcSuccessResult::CreateWhepSubscriber { processor_id })
    }

    async fn handle_list_tracks_rpc(&self) -> Result<RpcSuccessResult, RpcError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(MediaPipelineCommand::ListTracks { reply_tx });

        let track_ids = reply_rx
            .await
            .map_err(|_| internal_error("Internal error: pipeline has terminated"))?;

        Ok(RpcSuccessResult::ListTracks { track_ids })
    }

    async fn handle_list_processors_rpc(&self) -> Result<RpcSuccessResult, RpcError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(MediaPipelineCommand::ListProcessors { reply_tx });

        let processor_ids = reply_rx
            .await
            .map_err(|_| internal_error("Internal error: pipeline has terminated"))?;

        Ok(RpcSuccessResult::ListProcessors { processor_ids })
    }
}

enum RpcSuccessResult {
    CreateMp4FileSource { processor_id: ProcessorId },
    CreatePngFileSource { processor_id: ProcessorId },
    CreateVideoDeviceSource { processor_id: ProcessorId },
    CreateVideoMixer { processor_id: ProcessorId },
    CreateWhipPublisher { processor_id: ProcessorId },
    CreateWhepSubscriber { processor_id: ProcessorId },
    ListTracks { track_ids: Vec<TrackId> },
    ListProcessors { processor_ids: Vec<ProcessorId> },
}

impl nojson::DisplayJson for RpcSuccessResult {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::CreateMp4FileSource { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreatePngFileSource { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateVideoDeviceSource { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateVideoMixer { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateWhipPublisher { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateWhepSubscriber { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::ListTracks { track_ids } => f.array(|f| {
                f.elements(track_ids.iter().map(|track_id| {
                    nojson::json(move |f| f.object(|f| f.member("trackId", track_id)))
                }))
            }),
            Self::ListProcessors { processor_ids } => f.array(|f| {
                f.elements(processor_ids.iter().map(|processor_id| {
                    nojson::json(move |f| f.object(|f| f.member("processorId", processor_id)))
                }))
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::BufWriter, time::Duration};

    use crate::media_pipeline::{MediaPipeline, MediaPipelineHandle, ProcessorId, TrackId};

    const TEST_MP4_PATH: &str = "testdata/archive-red-320x320-av1.mp4";

    #[tokio::test]
    async fn notification_error_returns_no_response() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","method":"createMp4FileSource"}"#;

        let response = handle.rpc(request.as_bytes()).await;
        assert!(response.is_none());

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createMp4FileSource"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_validates_mp4_source_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4FileSource","params":{{"path":"{TEST_MP4_PATH}"}}}}"#
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_uses_path_as_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4FileSource","params":{{"path":"{TEST_MP4_PATH}","realtime":false,"loopPlayback":false,"videoTrackId":"video-default-id"}}}}"#
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            TEST_MP4_PATH
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4FileSource","params":{{"path":"{TEST_MP4_PATH}","processorId":"custom-source","realtime":false,"loopPlayback":false,"videoTrackId":"video-custom-id"}}}}"#
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-source"
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4FileSource","params":{{"path":"{TEST_MP4_PATH}","processorId":"duplicate-source","realtime":true,"loopPlayback":false,"videoTrackId":"video-duplicate-id"}}}}"#
        );

        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            result_processor_id(&first_response).expect("parse result.processorId"),
            "duplicate-source"
        );

        let second_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&second_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_png_file_source_requires_params() -> crate::Result<()> {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createPngFileSource"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
        Ok(())
    }

    #[tokio::test]
    async fn create_png_file_source_validates_source_params() -> crate::Result<()> {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let png_file = create_test_png_file(2, 2, png::ColorType::Rgba, &[255; 16])?;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createPngFileSource","params":{{"path":"{}"}}}}"#,
            png_file.path().display()
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
        Ok(())
    }

    #[tokio::test]
    async fn create_png_file_source_uses_path_as_default_processor_id() -> crate::Result<()> {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let png_file = create_test_png_file(2, 2, png::ColorType::Rgb, &[0; 12])?;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createPngFileSource","params":{{"path":"{}","frameRate":1,"outputVideoTrackId":"png-video-default"}}}}"#,
            png_file.path().display()
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            png_file.path().display().to_string()
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn create_png_file_source_uses_explicit_processor_id() -> crate::Result<()> {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let png_file = create_test_png_file(2, 2, png::ColorType::Rgb, &[0; 12])?;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createPngFileSource","params":{{"path":"{}","processorId":"custom-png-source","frameRate":1,"outputVideoTrackId":"png-video-custom"}}}}"#,
            png_file.path().display()
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-png-source"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn create_png_file_source_rejects_duplicate_processor_id() -> crate::Result<()> {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let png_file = create_test_png_file(2, 2, png::ColorType::Rgba, &[255; 16])?;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createPngFileSource","params":{{"path":"{}","processorId":"duplicate-png-source","frameRate":1,"outputVideoTrackId":"png-video-duplicate"}}}}"#,
            png_file.path().display()
        );
        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            result_processor_id(&first_response).expect("parse result.processorId"),
            "duplicate-png-source"
        );

        let second_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&second_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
        Ok(())
    }

    #[tokio::test]
    async fn create_video_device_source_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createVideoDeviceSource"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_device_source_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createVideoDeviceSource","params":{"deviceId":"camera0"}}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_device_source_uses_default_processor_id_for_default_device() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_video_device_source_request(None, None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "videoDeviceSource:default"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_device_source_uses_default_processor_id_for_device_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_video_device_source_request(None, Some("camera0"));

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "videoDeviceSource:camera0"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_device_source_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_video_device_source_request(Some("custom-video-device"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-video-device"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_device_source_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_video_device_source_request(Some("duplicate-video-device"), None);

        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            result_processor_id(&first_response).expect("parse result.processorId"),
            "duplicate-video-device"
        );

        let second_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&second_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_mixer_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createVideoMixer"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_mixer_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request =
            r#"{"jsonrpc":"2.0","id":1,"method":"createVideoMixer","params":{"canvasWidth":640}}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_mixer_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(ProcessorId::new("video-mixer-blocker"))
            .await
            .expect("register video-mixer-blocker");
        let occupied_sender = blocker
            .publish_track(TrackId::new("video-mixer-output"))
            .await
            .expect("publish video-mixer-output");
        let request = create_video_mixer_request("video-mixer-output", None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "videoMixer"
        );

        drop(occupied_sender);
        drop(blocker);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_mixer_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(ProcessorId::new("video-mixer-blocker"))
            .await
            .expect("register video-mixer-blocker");
        let occupied_sender = blocker
            .publish_track(TrackId::new("video-mixer-output"))
            .await
            .expect("publish video-mixer-output");
        let request = create_video_mixer_request("video-mixer-output", Some("custom-video-mixer"));

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-video-mixer"
        );

        drop(occupied_sender);
        drop(blocker);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_mixer_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request =
            create_video_mixer_request("video-mixer-output-dup", Some("duplicate-video-mixer"));

        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            result_processor_id(&first_response).expect("parse result.processorId"),
            "duplicate-video-mixer"
        );

        let second_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&second_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_whip_publisher_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createWhipPublisher"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_whip_publisher_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createWhipPublisher","params":{"outputUrl":"ws://example.com/whip/live","inputVideoTrackId":"video-main"}}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_whip_publisher_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_whip_publisher_request(None, None, Some("video-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "whipPublisher"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_whip_publisher_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_whip_publisher_request(
            Some("custom-whip-publisher"),
            None,
            Some("video-main"),
            None,
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-whip-publisher"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_whip_publisher_accepts_bearer_token() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request =
            create_whip_publisher_request(None, Some("test-token"), Some("video-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "whipPublisher"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_whip_publisher_rejects_empty_bearer_token() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_whip_publisher_request(None, Some("   "), Some("video-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_whip_publisher_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(ProcessorId::new("duplicate-whip-publisher"))
            .await
            .expect("register duplicate-whip-publisher");
        let request = create_whip_publisher_request(
            Some("duplicate-whip-publisher"),
            None,
            Some("video-main"),
            None,
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(blocker);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_whip_publisher_accepts_input_audio_track_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_whip_publisher_request(None, None, None, Some("audio-main"));

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "whipPublisher"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_whip_publisher_accepts_without_input_track_ids() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_whip_publisher_request(None, None, None, None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "whipPublisher"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_whep_subscriber_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createWhepSubscriber"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_whep_subscriber_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createWhepSubscriber","params":{"inputUrl":"ws://example.com/whep/live","outputVideoTrackId":"video-main"}}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_whep_subscriber_requires_output_video_track_id_for_now() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_whep_subscriber_request(None, None, None, None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_whep_subscriber_rejects_output_audio_track_id_for_now() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request =
            create_whep_subscriber_request(None, None, Some("video-main"), Some("audio-main"));

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_whep_subscriber_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_whep_subscriber_request(None, None, Some("video-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "https://example.com/whep/live"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_whep_subscriber_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_whep_subscriber_request(
            Some("custom-whep-subscriber"),
            None,
            Some("video-main"),
            None,
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-whep-subscriber"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_whep_subscriber_accepts_bearer_token() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request =
            create_whep_subscriber_request(None, Some("test-token"), Some("video-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "https://example.com/whep/live"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_whep_subscriber_rejects_empty_bearer_token() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_whep_subscriber_request(None, Some("   "), Some("video-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_whep_subscriber_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let blocker = handle
            .register_processor(ProcessorId::new("duplicate-whep-subscriber"))
            .await
            .expect("register duplicate-whep-subscriber");
        let request = create_whep_subscriber_request(
            Some("duplicate-whep-subscriber"),
            None,
            Some("video-main"),
            None,
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(blocker);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn list_processors_returns_empty_array_when_no_processors() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"listProcessors"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert!(
            result_processor_ids(&response)
                .expect("parse result processor ids")
                .is_empty()
        );

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
            .register_processor(ProcessorId::new("list-processor-a"))
            .await
            .expect("register list-processor-a");
        let processor_b = handle
            .register_processor(ProcessorId::new("list-processor-b"))
            .await
            .expect("register list-processor-b");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"listProcessors"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        let processor_ids = result_processor_ids(&response).expect("parse result processor ids");

        assert!(processor_ids.contains(&"list-processor-a".to_owned()));
        assert!(processor_ids.contains(&"list-processor-b".to_owned()));

        drop(processor_a);
        drop(processor_b);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn list_tracks_returns_empty_array_when_no_tracks() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"listTracks"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert!(
            result_track_ids(&response)
                .expect("parse result track ids")
                .is_empty()
        );

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
            .register_processor(ProcessorId::new("list-tracks-publisher"))
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
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"listTracks"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        let track_ids = result_track_ids(&response).expect("parse result track ids");

        assert!(track_ids.contains(&"list-track-a".to_owned()));
        assert!(track_ids.contains(&"list-track-b".to_owned()));

        drop(publisher);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn list_rpcs_ignore_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let list_tracks_request =
            r#"{"jsonrpc":"2.0","id":1,"method":"listTracks","params":{"dummy":1}}"#;
        let list_processors_request =
            r#"{"jsonrpc":"2.0","id":2,"method":"listProcessors","params":["dummy"]}"#;

        let tracks_response = handle
            .rpc(list_tracks_request.as_bytes())
            .await
            .expect("response must exist");
        let processors_response = handle
            .rpc(list_processors_request.as_bytes())
            .await
            .expect("response must exist");

        assert!(tracks_response.value().to_member("result").is_ok());
        assert!(processors_response.value().to_member("result").is_ok());

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    async fn spawn_test_pipeline() -> (MediaPipelineHandle, tokio::task::JoinHandle<()>) {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        handle.complete_initial_processor_registration();
        (handle, pipeline_task)
    }

    fn error_code(response: &nojson::RawJsonOwned) -> Result<i32, nojson::JsonParseError> {
        response
            .value()
            .to_member("error")?
            .required()?
            .to_member("code")?
            .required()?
            .try_into()
    }

    fn result_processor_id(
        response: &nojson::RawJsonOwned,
    ) -> Result<String, nojson::JsonParseError> {
        response
            .value()
            .to_member("result")?
            .required()?
            .to_member("processorId")?
            .required()?
            .try_into()
    }

    fn result_track_ids(
        response: &nojson::RawJsonOwned,
    ) -> Result<Vec<String>, nojson::JsonParseError> {
        response
            .value()
            .to_member("result")?
            .required()?
            .to_array()?
            .map(|v| v.to_member("trackId")?.required()?.try_into())
            .collect()
    }

    fn result_processor_ids(
        response: &nojson::RawJsonOwned,
    ) -> Result<Vec<String>, nojson::JsonParseError> {
        response
            .value()
            .to_member("result")?
            .required()?
            .to_array()?
            .map(|v| v.to_member("processorId")?.required()?.try_into())
            .collect()
    }

    fn create_video_mixer_request(output_track_id: &str, processor_id: Option<&str>) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createVideoMixer","params":{{"canvasWidth":640,"canvasHeight":480,"frameRate":30,"inputTracks":[{{"trackId":"video-input-track","x":0,"y":0,"z":0}}],"outputTrackId":"{output_track_id}"{processor_id_part}}}}}"#
        )
    }

    fn create_video_device_source_request(
        processor_id: Option<&str>,
        device_id: Option<&str>,
    ) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();
        let device_id_part = device_id
            .map(|id| format!(r#","deviceId":"{id}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createVideoDeviceSource","params":{{"outputVideoTrackId":"video-device-output"{device_id_part}{processor_id_part}}}}}"#
        )
    }

    fn create_whip_publisher_request(
        processor_id: Option<&str>,
        bearer_token: Option<&str>,
        input_video_track_id: Option<&str>,
        input_audio_track_id: Option<&str>,
    ) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();
        let bearer_token_part = bearer_token
            .map(|token| format!(r#","bearerToken":"{token}""#))
            .unwrap_or_default();
        let input_video_track_id_part = input_video_track_id
            .map(|id| format!(r#","inputVideoTrackId":"{id}""#))
            .unwrap_or_default();
        let input_audio_track_id_part = input_audio_track_id
            .map(|id| format!(r#","inputAudioTrackId":"{id}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createWhipPublisher","params":{{"outputUrl":"https://example.com/whip/live"{input_video_track_id_part}{input_audio_track_id_part}{bearer_token_part}{processor_id_part}}}}}"#
        )
    }

    fn create_whep_subscriber_request(
        processor_id: Option<&str>,
        bearer_token: Option<&str>,
        output_video_track_id: Option<&str>,
        output_audio_track_id: Option<&str>,
    ) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();
        let bearer_token_part = bearer_token
            .map(|token| format!(r#","bearerToken":"{token}""#))
            .unwrap_or_default();
        let output_video_track_id_part = output_video_track_id
            .map(|id| format!(r#","outputVideoTrackId":"{id}""#))
            .unwrap_or_default();
        let output_audio_track_id_part = output_audio_track_id
            .map(|id| format!(r#","outputAudioTrackId":"{id}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createWhepSubscriber","params":{{"inputUrl":"https://example.com/whep/live"{output_video_track_id_part}{output_audio_track_id_part}{bearer_token_part}{processor_id_part}}}}}"#
        )
    }

    fn create_test_png_file(
        width: u32,
        height: u32,
        color_type: png::ColorType,
        data: &[u8],
    ) -> crate::Result<tempfile::NamedTempFile> {
        let file = tempfile::NamedTempFile::new()?;
        let writer = BufWriter::new(File::create(file.path())?);
        let mut encoder = png::Encoder::new(writer, width, height);
        encoder.set_color(color_type);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| crate::Error::new(e.to_string()))?;
        writer
            .write_image_data(data)
            .map_err(|e| crate::Error::new(e.to_string()))?;
        Ok(file)
    }
}
