// NOTE: 長いので MediaPipelineHandle の RPC 関連の処理はこっちで実装している

use crate::media_pipeline::{
    MediaPipelineCommand, MediaPipelineHandle, ProcessorId, ProcessorMetadata,
    RegisterProcessorError, TrackId,
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

fn invalid_request(message: impl Into<String>) -> RpcError {
    (crate::jsonrpc::INVALID_REQUEST, message.into())
}

fn internal_error(message: impl Into<String>) -> RpcError {
    (crate::jsonrpc::INTERNAL_ERROR, message.into())
}

fn parse_required_mp4_path(
    params: &nojson::RawJsonValue<'_, '_>,
) -> Result<std::path::PathBuf, nojson::JsonParseError> {
    let path: std::path::PathBuf = params.to_member("path")?.required()?.try_into()?;
    if !path.exists() {
        let error_value = params.to_member("path")?.required()?;
        return Err(error_value.invalid(format!("input path does not exist: {}", path.display())));
    }
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| ext.eq_ignore_ascii_case("mp4"))
        .is_none()
    {
        let error_value = params.to_member("path")?.required()?;
        return Err(error_value.invalid(format!(
            "input path must be an mp4 file: {}",
            path.display()
        )));
    }
    Ok(path)
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
            "createMp4VideoReader" => self.handle_create_mp4_video_reader_rpc(maybe_params).await,
            "createMp4AudioReader" => self.handle_create_mp4_audio_reader_rpc(maybe_params).await,
            "createAudioDecoder" => self.handle_create_audio_decoder_rpc(maybe_params).await,
            "createVideoDecoder" => self.handle_create_video_decoder_rpc(maybe_params).await,
            "createPngFileSource" => self.handle_create_png_file_source_rpc(maybe_params).await,
            "createVideoDeviceSource" => {
                self.handle_create_video_device_source_rpc(maybe_params)
                    .await
            }
            "createVideoMixer" => self.handle_create_video_mixer_rpc(maybe_params).await,
            "createRtmpPublisher" => self.handle_create_rtmp_publisher_rpc(maybe_params).await,
            "createRtmpInboundEndpoint" => {
                self.handle_create_rtmp_inbound_endpoint_rpc(maybe_params)
                    .await
            }
            "createSrtInboundEndpoint" => {
                self.handle_create_srt_inbound_endpoint_rpc(maybe_params)
                    .await
            }
            "createRtmpOutboundEndpoint" => {
                self.handle_create_rtmp_outbound_endpoint_rpc(maybe_params)
                    .await
            }
            "createWhipPublisher" => self.handle_create_whip_publisher_rpc(maybe_params).await,
            "createWhepSubscriber" => self.handle_create_whep_subscriber_rpc(maybe_params).await,
            "listTracks" => self.handle_list_tracks_rpc().await,
            "listProcessors" => self.handle_list_processors_rpc().await,
            "triggerStart" => self.handle_trigger_start_rpc().await,
            "waitProcessorTerminated" => {
                self.handle_wait_processor_terminated_rpc(maybe_params)
                    .await
            }
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

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("mp4_file_source"),
            move |handle| source.run(handle),
        )
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

    async fn handle_create_mp4_video_reader_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (path, processor_id): (std::path::PathBuf, Option<ProcessorId>) =
            parse_params(maybe_params, |params| {
                let path = parse_required_mp4_path(&params)?;
                let processor_id = params.to_member("processorId")?.try_into()?;
                Ok((path, processor_id))
            })?;
        let processor_id =
            processor_id.unwrap_or_else(|| ProcessorId::new(path.display().to_string()));

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("mp4_video_reader"),
            move |handle| async move {
                let reader = crate::sora_recording_reader::VideoReader::new(
                    crate::types::ContainerFormat::Mp4,
                    std::time::Duration::ZERO,
                    vec![path],
                    handle.stats(),
                )?;
                reader.run(handle).await
            },
        )
        .await
        .map_err(|e| match e {
            RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                "Invalid params: processorId already exists: {processor_id}"
            )),
            RegisterProcessorError::PipelineTerminated => {
                internal_error("Internal error: pipeline has terminated".to_owned())
            }
        })?;

        Ok(RpcSuccessResult::CreateMp4VideoReader { processor_id })
    }

    async fn handle_create_mp4_audio_reader_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (path, processor_id): (std::path::PathBuf, Option<ProcessorId>) =
            parse_params(maybe_params, |params| {
                let path = parse_required_mp4_path(&params)?;
                let processor_id = params.to_member("processorId")?.try_into()?;
                Ok((path, processor_id))
            })?;
        let processor_id =
            processor_id.unwrap_or_else(|| ProcessorId::new(path.display().to_string()));

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("mp4_audio_reader"),
            move |handle| async move {
                let reader = crate::sora_recording_reader::AudioReader::new(
                    crate::types::ContainerFormat::Mp4,
                    std::time::Duration::ZERO,
                    vec![path],
                    handle.stats(),
                )?;
                reader.run(handle).await
            },
        )
        .await
        .map_err(|e| match e {
            RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                "Invalid params: processorId already exists: {processor_id}"
            )),
            RegisterProcessorError::PipelineTerminated => {
                internal_error("Internal error: pipeline has terminated".to_owned())
            }
        })?;

        Ok(RpcSuccessResult::CreateMp4AudioReader { processor_id })
    }

    async fn handle_create_audio_decoder_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (input_track_id, output_track_id, processor_id): (
            TrackId,
            TrackId,
            Option<ProcessorId>,
        ) = parse_params(maybe_params, |params| {
            let input_track_id = params.to_member("inputTrackId")?.required()?.try_into()?;
            let output_track_id = params.to_member("outputTrackId")?.required()?.try_into()?;
            let processor_id = params.to_member("processorId")?.try_into()?;
            Ok((input_track_id, output_track_id, processor_id))
        })?;
        let processor_id = processor_id
            .unwrap_or_else(|| ProcessorId::new(format!("audioDecoder:{input_track_id}")));

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("audio_decoder"),
            move |handle| async move {
                let decoder = crate::decoder::AudioDecoder::new(handle.stats())?;
                decoder.run(handle, input_track_id, output_track_id).await
            },
        )
        .await
        .map_err(|e| match e {
            RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                "Invalid params: processorId already exists: {processor_id}"
            )),
            RegisterProcessorError::PipelineTerminated => {
                internal_error("Internal error: pipeline has terminated".to_owned())
            }
        })?;

        Ok(RpcSuccessResult::CreateAudioDecoder { processor_id })
    }

    async fn handle_create_video_decoder_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (input_track_id, output_track_id, processor_id): (
            TrackId,
            TrackId,
            Option<ProcessorId>,
        ) = parse_params(maybe_params, |params| {
            let input_track_id = params.to_member("inputTrackId")?.required()?.try_into()?;
            let output_track_id = params.to_member("outputTrackId")?.required()?.try_into()?;
            let processor_id = params.to_member("processorId")?.try_into()?;
            Ok((input_track_id, output_track_id, processor_id))
        })?;
        let processor_id = processor_id
            .unwrap_or_else(|| ProcessorId::new(format!("videoDecoder:{input_track_id}")));

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("video_decoder"),
            move |handle| async move {
                // 現時点の RPC は最小構成として default のみを受け付ける。
                // engines / decode_params は必要になった時点で RPC パラメーターを拡張する。
                // openh264_lib は RPC ではなく、コマンドライン引数 / 環境変数で指定する想定。
                let decoder = crate::decoder::VideoDecoder::new(
                    crate::decoder::VideoDecoderOptions::default(),
                    handle.stats(),
                );
                decoder.run(handle, input_track_id, output_track_id).await
            },
        )
        .await
        .map_err(|e| match e {
            RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                "Invalid params: processorId already exists: {processor_id}"
            )),
            RegisterProcessorError::PipelineTerminated => {
                internal_error("Internal error: pipeline has terminated".to_owned())
            }
        })?;

        Ok(RpcSuccessResult::CreateVideoDecoder { processor_id })
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

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("png_file_source"),
            move |handle| source.run(handle),
        )
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

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("video_device_source"),
            move |handle| source.run(handle),
        )
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

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("video_mixer"),
            move |handle| mixer.run(handle),
        )
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

        self.spawn_local_processor(
            processor_id.clone(),
            ProcessorMetadata::new("whip_publisher"),
            move |handle| publisher.run(handle),
        )
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

    async fn handle_create_rtmp_publisher_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (publisher, processor_id): (crate::publisher_rtmp::RtmpPublisher, Option<ProcessorId>) =
            parse_params(maybe_params, |params| {
                let publisher = params.try_into()?;
                let processor_id = params.to_member("processorId")?.try_into()?;
                Ok((publisher, processor_id))
            })?;
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("rtmpPublisher"));

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("rtmp_publisher"),
            move |handle| publisher.run(handle),
        )
        .await
        .map_err(|e| match e {
            RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                "Invalid params: processorId already exists: {processor_id}"
            )),
            RegisterProcessorError::PipelineTerminated => {
                internal_error("Internal error: pipeline has terminated".to_owned())
            }
        })?;

        Ok(RpcSuccessResult::CreateRtmpPublisher { processor_id })
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

        self.spawn_local_processor(
            processor_id.clone(),
            ProcessorMetadata::new("whep_subscriber"),
            move |handle| subscriber.run(handle),
        )
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

    async fn handle_create_rtmp_inbound_endpoint_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (endpoint, processor_id): (
            crate::inbound_endpoint_rtmp::RtmpInboundEndpoint,
            Option<ProcessorId>,
        ) = parse_params(maybe_params, |params| {
            let endpoint = params.try_into()?;
            let processor_id = params.to_member("processorId")?.try_into()?;
            Ok((endpoint, processor_id))
        })?;
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("rtmpInboundEndpoint"));

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("rtmp_inbound_endpoint"),
            move |handle| endpoint.run(handle),
        )
        .await
        .map_err(|e| match e {
            RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                "Invalid params: processorId already exists: {processor_id}"
            )),
            RegisterProcessorError::PipelineTerminated => {
                internal_error("Internal error: pipeline has terminated".to_owned())
            }
        })?;

        Ok(RpcSuccessResult::CreateRtmpInboundEndpoint { processor_id })
    }

    async fn handle_create_srt_inbound_endpoint_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (endpoint, processor_id): (
            crate::inbound_endpoint_srt::SrtInboundEndpoint,
            Option<ProcessorId>,
        ) = parse_params(maybe_params, |params| {
            let endpoint = params.try_into()?;
            let processor_id = params.to_member("processorId")?.try_into()?;
            Ok((endpoint, processor_id))
        })?;
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("srtInboundEndpoint"));

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("srt_inbound_endpoint"),
            move |handle| endpoint.run(handle),
        )
        .await
        .map_err(|e| match e {
            RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                "Invalid params: processorId already exists: {processor_id}"
            )),
            RegisterProcessorError::PipelineTerminated => {
                internal_error("Internal error: pipeline has terminated".to_owned())
            }
        })?;

        Ok(RpcSuccessResult::CreateSrtInboundEndpoint { processor_id })
    }

    async fn handle_create_rtmp_outbound_endpoint_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let (endpoint, processor_id): (
            crate::outbound_endpoint_rtmp::RtmpOutboundEndpoint,
            Option<ProcessorId>,
        ) = parse_params(maybe_params, |params| {
            let endpoint = params.try_into()?;
            let processor_id = params.to_member("processorId")?.try_into()?;
            Ok((endpoint, processor_id))
        })?;
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("rtmpOutboundEndpoint"));

        self.spawn_processor(
            processor_id.clone(),
            ProcessorMetadata::new("rtmp_outbound_endpoint"),
            move |handle| endpoint.run(handle),
        )
        .await
        .map_err(|e| match e {
            RegisterProcessorError::DuplicateProcessorId => invalid_params(format!(
                "Invalid params: processorId already exists: {processor_id}"
            )),
            RegisterProcessorError::PipelineTerminated => {
                internal_error("Internal error: pipeline has terminated".to_owned())
            }
        })?;

        Ok(RpcSuccessResult::CreateRtmpOutboundEndpoint { processor_id })
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
        let processor_ids = self.list_processor_ids_for_rpc().await?;

        Ok(RpcSuccessResult::ListProcessors { processor_ids })
    }

    async fn handle_trigger_start_rpc(&self) -> Result<RpcSuccessResult, RpcError> {
        let started = self
            .trigger_start()
            .await
            .map_err(|_| internal_error("Internal error: pipeline has terminated"))?;
        if !started {
            return Err(invalid_request(
                "Invalid request: pipeline has already started",
            ));
        }
        Ok(RpcSuccessResult::TriggerStart { started })
    }

    async fn handle_wait_processor_terminated_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcSuccessResult, RpcError> {
        let processor_id: ProcessorId = parse_params(maybe_params, |params| {
            params.to_member("processorId")?.required()?.try_into()
        })?;

        loop {
            let processor_ids = self.list_processor_ids_for_rpc().await?;
            if !processor_ids.iter().any(|id| id == &processor_id) {
                return Ok(RpcSuccessResult::WaitProcessorTerminated { processor_id });
            }
            // 現状は e2e テスト用途を主眼にした簡易実装として、短い間隔でポーリングしている。
            // これを汎用用途でも使う場合は media pipeline 側に終了待機コマンドを追加して待機する方が望ましい。
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    async fn list_processor_ids_for_rpc(&self) -> Result<Vec<ProcessorId>, RpcError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(MediaPipelineCommand::ListProcessors { reply_tx });

        let processor_ids = reply_rx
            .await
            .map_err(|_| internal_error("Internal error: pipeline has terminated"))?;

        Ok(processor_ids)
    }
}

enum RpcSuccessResult {
    CreateMp4FileSource { processor_id: ProcessorId },
    CreateMp4VideoReader { processor_id: ProcessorId },
    CreateMp4AudioReader { processor_id: ProcessorId },
    CreateAudioDecoder { processor_id: ProcessorId },
    CreateVideoDecoder { processor_id: ProcessorId },
    CreatePngFileSource { processor_id: ProcessorId },
    CreateVideoDeviceSource { processor_id: ProcessorId },
    CreateVideoMixer { processor_id: ProcessorId },
    CreateRtmpPublisher { processor_id: ProcessorId },
    CreateRtmpInboundEndpoint { processor_id: ProcessorId },
    CreateSrtInboundEndpoint { processor_id: ProcessorId },
    CreateRtmpOutboundEndpoint { processor_id: ProcessorId },
    CreateWhipPublisher { processor_id: ProcessorId },
    CreateWhepSubscriber { processor_id: ProcessorId },
    ListTracks { track_ids: Vec<TrackId> },
    ListProcessors { processor_ids: Vec<ProcessorId> },
    TriggerStart { started: bool },
    WaitProcessorTerminated { processor_id: ProcessorId },
}

impl nojson::DisplayJson for RpcSuccessResult {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::CreateMp4FileSource { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateMp4VideoReader { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateMp4AudioReader { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateAudioDecoder { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateVideoDecoder { processor_id } => {
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
            Self::CreateRtmpPublisher { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateRtmpInboundEndpoint { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateSrtInboundEndpoint { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
            Self::CreateRtmpOutboundEndpoint { processor_id } => {
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
            Self::TriggerStart { started } => f.object(|f| f.member("started", *started)),
            Self::WaitProcessorTerminated { processor_id } => {
                f.object(|f| f.member("processorId", processor_id))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::BufWriter, time::Duration};

    use crate::media_pipeline::{
        MediaPipeline, MediaPipelineHandle, ProcessorId, ProcessorMetadata, TrackId,
    };

    const TEST_MP4_PATH: &str = "testdata/archive-red-320x320-av1.mp4";
    const TEST_MP4_AUDIO_PATH: &str = "testdata/red-320x320-h264-aac.mp4";

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
    async fn create_mp4_video_reader_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createMp4VideoReader"}"#;

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
    async fn create_mp4_video_reader_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4VideoReader","params":{{"path":"{TEST_MP4_PATH}","processorId":"custom-mp4-video-reader"}}}}"#
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-mp4-video-reader"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_mp4_video_reader_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4VideoReader","params":{{"path":"{TEST_MP4_PATH}","processorId":"duplicate-mp4-video-reader"}}}}"#
        );

        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            result_processor_id(&first_response).expect("parse result.processorId"),
            "duplicate-mp4-video-reader"
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
    async fn create_mp4_audio_reader_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createMp4AudioReader"}"#;

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
    async fn create_mp4_audio_reader_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4AudioReader","params":{{"path":"{TEST_MP4_AUDIO_PATH}","processorId":"custom-mp4-audio-reader"}}}}"#
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-mp4-audio-reader"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_mp4_audio_reader_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4AudioReader","params":{{"path":"{TEST_MP4_AUDIO_PATH}","processorId":"duplicate-mp4-audio-reader"}}}}"#
        );

        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            result_processor_id(&first_response).expect("parse result.processorId"),
            "duplicate-mp4-audio-reader"
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
    async fn create_audio_decoder_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createAudioDecoder"}"#;

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
    async fn create_audio_decoder_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createAudioDecoder","params":{"inputTrackId":"audio-input"}}"#;

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
    async fn create_audio_decoder_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_audio_decoder_request(None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "audioDecoder:audio-input"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_audio_decoder_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_audio_decoder_request(Some("custom-audio-decoder"));

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-audio-decoder"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_audio_decoder_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_audio_decoder_request(Some("duplicate-audio-decoder"));

        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            result_processor_id(&first_response).expect("parse result.processorId"),
            "duplicate-audio-decoder"
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
    async fn create_video_decoder_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createVideoDecoder"}"#;

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
    async fn create_video_decoder_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createVideoDecoder","params":{"inputTrackId":"video-input"}}"#;

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
    async fn create_video_decoder_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_video_decoder_request(None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "videoDecoder:video-input"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_decoder_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_video_decoder_request(Some("custom-video-decoder"));

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-video-decoder"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_video_decoder_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_video_decoder_request(Some("duplicate-video-decoder"));

        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            result_processor_id(&first_response).expect("parse result.processorId"),
            "duplicate-video-decoder"
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
            .register_processor(
                ProcessorId::new("duplicate-whip-publisher"),
                ProcessorMetadata::new("test_processor"),
            )
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
    async fn create_rtmp_publisher_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createRtmpPublisher"}"#;

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
    async fn create_rtmp_publisher_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createRtmpPublisher","params":{"outputUrl":"ws://example.com/live","inputVideoTrackId":"video-main"}}"#;

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
    async fn create_rtmp_publisher_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_rtmp_publisher_request(None, Some("video-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "rtmpPublisher"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_publisher_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request =
            create_rtmp_publisher_request(Some("custom-rtmp-publisher"), Some("video-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-rtmp-publisher"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_publisher_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_rtmp_publisher_request(
            Some("duplicate-rtmp-publisher"),
            Some("video-main"),
            None,
        );

        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            result_processor_id(&first_response).expect("parse result.processorId"),
            "duplicate-rtmp-publisher"
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
    async fn create_rtmp_inbound_endpoint_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createRtmpInboundEndpoint"}"#;

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
    async fn create_rtmp_inbound_endpoint_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let invalid_url_request = r#"{"jsonrpc":"2.0","id":1,"method":"createRtmpInboundEndpoint","params":{"inputUrl":"ws://example.com/live","outputVideoTrackId":"video-main"}}"#;
        let missing_output_track_request = r#"{"jsonrpc":"2.0","id":1,"method":"createRtmpInboundEndpoint","params":{"inputUrl":"rtmp://127.0.0.1:1935/live","streamName":"stream-main"}}"#;

        let invalid_url_response = handle
            .rpc(invalid_url_request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&invalid_url_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        let missing_output_track_response = handle
            .rpc(missing_output_track_request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&missing_output_track_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_rtmp_inbound_endpoint_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_rtmp_inbound_endpoint_request(None, Some("audio-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "rtmpInboundEndpoint"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_inbound_endpoint_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_rtmp_inbound_endpoint_request(
            Some("custom-rtmp-inbound-endpoint"),
            Some("audio-main"),
            Some("video-main"),
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-rtmp-inbound-endpoint"
        );

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
        let request = create_rtmp_inbound_endpoint_request(
            Some("duplicate-rtmp-inbound-endpoint"),
            Some("audio-main"),
            Some("video-main"),
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
    async fn create_rtmp_inbound_endpoint_accepts_audio_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_rtmp_inbound_endpoint_request(None, Some("audio-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "rtmpInboundEndpoint"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_inbound_endpoint_accepts_video_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_rtmp_inbound_endpoint_request(None, None, Some("video-main"));

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "rtmpInboundEndpoint"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_srt_inbound_endpoint_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createSrtInboundEndpoint"}"#;

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
    async fn create_srt_inbound_endpoint_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let invalid_scheme_request = r#"{"jsonrpc":"2.0","id":1,"method":"createSrtInboundEndpoint","params":{"inputUrl":"ws://example.com/live","outputVideoTrackId":"video-main"}}"#;
        let missing_output_track_request = r#"{"jsonrpc":"2.0","id":1,"method":"createSrtInboundEndpoint","params":{"inputUrl":"srt://127.0.0.1:10080"}}"#;
        let key_length_without_passphrase_request = r#"{"jsonrpc":"2.0","id":1,"method":"createSrtInboundEndpoint","params":{"inputUrl":"srt://127.0.0.1:10080","outputVideoTrackId":"video-main","keyLength":16}}"#;

        let invalid_scheme_response = handle
            .rpc(invalid_scheme_request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&invalid_scheme_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        let missing_output_track_response = handle
            .rpc(missing_output_track_request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&missing_output_track_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        let key_length_without_passphrase_response = handle
            .rpc(key_length_without_passphrase_request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&key_length_without_passphrase_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_srt_inbound_endpoint_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_srt_inbound_endpoint_request(None, Some("audio-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "srtInboundEndpoint"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_srt_inbound_endpoint_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_srt_inbound_endpoint_request(
            Some("custom-srt-inbound-endpoint"),
            Some("audio-main"),
            Some("video-main"),
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-srt-inbound-endpoint"
        );

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
        let request = create_srt_inbound_endpoint_request(
            Some("duplicate-srt-inbound-endpoint"),
            Some("audio-main"),
            Some("video-main"),
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
    async fn create_srt_inbound_endpoint_accepts_audio_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_srt_inbound_endpoint_request(None, Some("audio-main"), None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "srtInboundEndpoint"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_srt_inbound_endpoint_accepts_video_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_srt_inbound_endpoint_request(None, None, Some("video-main"));

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "srtInboundEndpoint"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_outbound_endpoint_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createRtmpOutboundEndpoint"}"#;

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
    async fn create_rtmp_outbound_endpoint_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let invalid_url_request = r#"{"jsonrpc":"2.0","id":1,"method":"createRtmpOutboundEndpoint","params":{"outputUrl":"ws://example.com/live","inputVideoTrackId":"video-main"}}"#;
        let missing_input_track_request = r#"{"jsonrpc":"2.0","id":1,"method":"createRtmpOutboundEndpoint","params":{"outputUrl":"rtmp://127.0.0.1:29350/live","streamName":"stream-main"}}"#;
        let missing_tls_cert_request = r#"{"jsonrpc":"2.0","id":1,"method":"createRtmpOutboundEndpoint","params":{"outputUrl":"rtmps://127.0.0.1:29350/live","streamName":"stream-main","inputVideoTrackId":"video-main"}}"#;

        let invalid_url_response = handle
            .rpc(invalid_url_request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&invalid_url_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        let missing_input_track_response = handle
            .rpc(missing_input_track_request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&missing_input_track_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        let missing_tls_cert_response = handle
            .rpc(missing_tls_cert_request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            error_code(&missing_tls_cert_response).expect("parse error.code"),
            crate::jsonrpc::INVALID_PARAMS
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_rtmp_outbound_endpoint_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request =
            create_rtmp_outbound_endpoint_request(None, Some("audio-main"), None, None, None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "rtmpOutboundEndpoint"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_outbound_endpoint_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = create_rtmp_outbound_endpoint_request(
            Some("custom-rtmp-outbound-endpoint"),
            Some("audio-main"),
            Some("video-main"),
            None,
            None,
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "custom-rtmp-outbound-endpoint"
        );

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
        let request = create_rtmp_outbound_endpoint_request(
            Some("duplicate-rtmp-outbound-endpoint"),
            Some("audio-main"),
            Some("video-main"),
            None,
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
    async fn create_rtmp_outbound_endpoint_accepts_audio_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request =
            create_rtmp_outbound_endpoint_request(None, Some("audio-main"), None, None, None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "rtmpOutboundEndpoint"
        );

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn create_rtmp_outbound_endpoint_accepts_video_only() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request =
            create_rtmp_outbound_endpoint_request(None, None, Some("video-main"), None, None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            result_processor_id(&response).expect("parse result.processorId"),
            "rtmpOutboundEndpoint"
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
            .register_processor(
                ProcessorId::new("duplicate-whep-subscriber"),
                ProcessorMetadata::new("test_processor"),
            )
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

    #[tokio::test]
    async fn trigger_start_succeeds_first_call() {
        let (handle, pipeline_task) = spawn_test_pipeline_without_start().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"triggerStart"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert!(
            result_trigger_start_started(&response).expect("parse result.started"),
            "triggerStart must start pipeline on first call"
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn trigger_start_rejects_when_pipeline_already_started() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"triggerStart"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(
            error_code(&response).expect("parse error.code"),
            crate::jsonrpc::INVALID_REQUEST
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn trigger_start_ignores_params() {
        let (handle, pipeline_task) = spawn_test_pipeline_without_start().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"triggerStart","params":{"dummy":1}}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert!(
            result_trigger_start_started(&response).expect("parse result.started"),
            "triggerStart must ignore params"
        );

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn wait_processor_terminated_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"waitProcessorTerminated"}"#;

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
    async fn wait_processor_terminated_returns_processor_id_when_processor_absent() {
        let (handle, pipeline_task) = spawn_test_pipeline().await;
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"waitProcessorTerminated","params":{"processorId":"missing-processor"}}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        let processor_id = result_wait_processor_terminated(&response).expect("parse result");
        assert_eq!(processor_id, "missing-processor");

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
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"waitProcessorTerminated","params":{"processorId":"alive-processor"}}"#;

        let wait_result =
            tokio::time::timeout(Duration::from_millis(50), handle.rpc(request.as_bytes())).await;
        assert!(
            wait_result.is_err(),
            "must keep waiting while processor is alive"
        );

        drop(blocker);

        let response = tokio::time::timeout(Duration::from_secs(5), handle.rpc(request.as_bytes()))
            .await
            .expect("rpc wait timed out")
            .expect("response must exist");
        let processor_id = result_wait_processor_terminated(&response).expect("parse result");
        assert_eq!(processor_id, "alive-processor");

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

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

    fn result_wait_processor_terminated(
        response: &nojson::RawJsonOwned,
    ) -> Result<String, nojson::JsonParseError> {
        let result = response.value().to_member("result")?.required()?;
        result.to_member("processorId")?.required()?.try_into()
    }

    fn result_trigger_start_started(
        response: &nojson::RawJsonOwned,
    ) -> Result<bool, nojson::JsonParseError> {
        let result = response.value().to_member("result")?.required()?;
        result.to_member("started")?.required()?.try_into()
    }

    fn create_audio_decoder_request(processor_id: Option<&str>) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createAudioDecoder","params":{{"inputTrackId":"audio-input","outputTrackId":"audio-output"{processor_id_part}}}}}"#
        )
    }

    fn create_video_decoder_request(processor_id: Option<&str>) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createVideoDecoder","params":{{"inputTrackId":"video-input","outputTrackId":"video-output"{processor_id_part}}}}}"#
        )
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

    fn create_rtmp_publisher_request(
        processor_id: Option<&str>,
        input_video_track_id: Option<&str>,
        input_audio_track_id: Option<&str>,
    ) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();
        let input_video_track_id_part = input_video_track_id
            .map(|id| format!(r#","inputVideoTrackId":"{id}""#))
            .unwrap_or_default();
        let input_audio_track_id_part = input_audio_track_id
            .map(|id| format!(r#","inputAudioTrackId":"{id}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createRtmpPublisher","params":{{"outputUrl":"rtmp://127.0.0.1:1935/live","streamName":"stream-main"{input_video_track_id_part}{input_audio_track_id_part}{processor_id_part}}}}}"#
        )
    }

    fn create_rtmp_inbound_endpoint_request(
        processor_id: Option<&str>,
        output_audio_track_id: Option<&str>,
        output_video_track_id: Option<&str>,
    ) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();
        let output_audio_track_id_part = output_audio_track_id
            .map(|id| format!(r#","outputAudioTrackId":"{id}""#))
            .unwrap_or_default();
        let output_video_track_id_part = output_video_track_id
            .map(|id| format!(r#","outputVideoTrackId":"{id}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createRtmpInboundEndpoint","params":{{"inputUrl":"rtmp://127.0.0.1:1935/live","streamName":"stream-main"{output_audio_track_id_part}{output_video_track_id_part}{processor_id_part}}}}}"#
        )
    }

    fn create_srt_inbound_endpoint_request(
        processor_id: Option<&str>,
        output_audio_track_id: Option<&str>,
        output_video_track_id: Option<&str>,
    ) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();
        let output_audio_track_id_part = output_audio_track_id
            .map(|id| format!(r#","outputAudioTrackId":"{id}""#))
            .unwrap_or_default();
        let output_video_track_id_part = output_video_track_id
            .map(|id| format!(r#","outputVideoTrackId":"{id}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createSrtInboundEndpoint","params":{{"inputUrl":"srt://127.0.0.1:10080"{output_audio_track_id_part}{output_video_track_id_part}{processor_id_part}}}}}"#
        )
    }

    fn create_rtmp_outbound_endpoint_request(
        processor_id: Option<&str>,
        input_audio_track_id: Option<&str>,
        input_video_track_id: Option<&str>,
        cert_path: Option<&str>,
        key_path: Option<&str>,
    ) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();
        let input_audio_track_id_part = input_audio_track_id
            .map(|id| format!(r#","inputAudioTrackId":"{id}""#))
            .unwrap_or_default();
        let input_video_track_id_part = input_video_track_id
            .map(|id| format!(r#","inputVideoTrackId":"{id}""#))
            .unwrap_or_default();
        let cert_path_part = cert_path
            .map(|path| format!(r#","certPath":"{path}""#))
            .unwrap_or_default();
        let key_path_part = key_path
            .map(|path| format!(r#","keyPath":"{path}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createRtmpOutboundEndpoint","params":{{"outputUrl":"rtmp://127.0.0.1:29350/live","streamName":"stream-main"{input_audio_track_id_part}{input_video_track_id_part}{cert_path_part}{key_path_part}{processor_id_part}}}}}"#
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
