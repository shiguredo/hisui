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
            "createVideoMixer" => self.handle_create_video_mixer_rpc(maybe_params).await,
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
    CreateVideoMixer { processor_id: ProcessorId },
    ListTracks { track_ids: Vec<TrackId> },
    ListProcessors { processor_ids: Vec<ProcessorId> },
}

impl nojson::DisplayJson for RpcSuccessResult {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::CreateMp4FileSource { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateVideoMixer { processor_id } => {
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
    use std::time::Duration;

    use crate::media_pipeline::{MediaPipeline, MediaPipelineHandle, ProcessorId, TrackId};

    const TEST_MP4_PATH: &str = "testdata/archive-red-320x320-av1.mp4";

    #[tokio::test]
    async fn notification_error_returns_no_response() {
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
    async fn create_video_mixer_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
    async fn list_processors_returns_empty_array_when_no_processors() {
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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
        let (handle, pipeline_task) = spawn_test_pipeline();
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

    fn spawn_test_pipeline() -> (MediaPipelineHandle, tokio::task::JoinHandle<()>) {
        let pipeline = MediaPipeline::new();
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
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
}
