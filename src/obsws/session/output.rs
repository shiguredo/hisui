use super::*;

impl ObswsSession {
    pub(super) async fn handle_start_stream(&self, request_id: &str) -> RequestOutcome {
        let (output_url, stream_name, image_path, run) = {
            let mut input_registry = self.input_registry.write().await;
            let stream_service_settings = input_registry.stream_service_settings().clone();
            if stream_service_settings.stream_service_type != "rtmp_custom" {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartStream",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Unsupported streamServiceType field",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Unsupported streamServiceType field",
                );
            }
            let Some(output_url) = stream_service_settings.server else {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartStream",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Missing streamServiceSettings.server field",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing streamServiceSettings.server field",
                );
            };

            let scene_inputs = input_registry.list_current_program_scene_inputs();
            if scene_inputs.len() != 1 {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartStream",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Exactly one enabled input is required in the current program scene",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Exactly one enabled input is required in the current program scene",
                );
            }
            let input = &scene_inputs[0];
            let ObswsInputSettings::ImageSource(settings) = &input.input.settings else {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartStream",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Only image_source is supported for StartStream",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Only image_source is supported for StartStream",
                );
            };
            let Some(image_path) = settings.file.clone() else {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartStream",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "inputSettings.file is required for image_source",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "inputSettings.file is required for image_source",
                );
            };

            let run_id = input_registry.next_stream_run_id();
            let source_processor_id = format!("obsws:stream:{run_id}:png_source");
            let encoder_processor_id = format!("obsws:stream:{run_id}:video_encoder");
            let endpoint_processor_id = format!("obsws:stream:{run_id}:rtmp_outbound");
            let source_track_id = format!("obsws:stream:{run_id}:raw_video");
            let encoded_track_id = format!("obsws:stream:{run_id}:encoded_video");
            let run = ObswsStreamRun {
                source_processor_id: source_processor_id.clone(),
                encoder_processor_id: encoder_processor_id.clone(),
                endpoint_processor_id: endpoint_processor_id.clone(),
                source_track_id: source_track_id.clone(),
                encoded_track_id: encoded_track_id.clone(),
            };
            if let Err(ActivateStreamError::AlreadyActive) =
                input_registry.activate_stream(run.clone())
            {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartStream",
                        request_id,
                        REQUEST_STATUS_STREAM_RUNNING,
                        "Stream is already active",
                    ),
                    REQUEST_STATUS_STREAM_RUNNING,
                    "Stream is already active",
                );
            }

            (output_url, stream_service_settings.key, image_path, run)
        };

        let start_result = self
            .start_stream_processors(&image_path, &output_url, stream_name.as_deref(), &run)
            .await;

        if let Err(e) = start_result {
            let _ = self.input_registry.write().await.deactivate_stream();
            if let Err(cleanup_error) = self.stop_stream_processors(&run).await {
                tracing::warn!(
                    "failed to cleanup stream processors after start failure: {}",
                    cleanup_error.display()
                );
            }
            let error_comment = format!("Failed to start stream: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response("StartStream", request_id, &error_comment),
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                error_comment,
            );
        }

        RequestOutcome::success(
            crate::obsws_response_builder::build_start_stream_response(request_id, true),
            None,
        )
    }

    pub(super) async fn handle_stop_stream(&self, request_id: &str) -> RequestOutcome {
        let run = {
            let input_registry = self.input_registry.read().await;
            if !input_registry.is_stream_active() {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StopStream",
                        request_id,
                        REQUEST_STATUS_STREAM_NOT_RUNNING,
                        "Stream is not active",
                    ),
                    REQUEST_STATUS_STREAM_NOT_RUNNING,
                    "Stream is not active",
                );
            }
            input_registry
                .stream_run()
                .expect("infallible: active stream must have run state")
        };
        if let Err(e) = self.stop_stream_processors(&run).await {
            let error_comment = format!("Failed to stop stream: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response("StopStream", request_id, &error_comment),
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                error_comment,
            );
        }
        let mut input_registry = self.input_registry.write().await;
        if input_registry.deactivate_stream().is_none() {
            tracing::warn!("stream runtime was already deactivated while stopping stream");
        }
        RequestOutcome::success(
            crate::obsws_response_builder::build_stop_stream_response(request_id),
            None,
        )
    }

    pub(super) async fn handle_start_record(&self, request_id: &str) -> RequestOutcome {
        let (source_plan, output_path, run) = {
            let mut input_registry = self.input_registry.write().await;
            let scene_inputs = input_registry.list_current_program_scene_inputs();
            if scene_inputs.len() != 1 {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartRecord",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Exactly one enabled input is required in the current program scene",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Exactly one enabled input is required in the current program scene",
                );
            }
            let input = &scene_inputs[0];
            let run_id = input_registry.next_record_run_id();
            let source_plan = match crate::obsws::source::build_record_source_plan(input, run_id) {
                Ok(source_plan) => source_plan,
                Err(error) => {
                    let error_comment = error.message();
                    return RequestOutcome::failure(
                        crate::obsws_response_builder::build_request_response_error(
                            "StartRecord",
                            request_id,
                            REQUEST_STATUS_INVALID_REQUEST_FIELD,
                            &error_comment,
                        ),
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        error_comment,
                    );
                }
            };
            if source_plan.source_video_track_id.is_none()
                && source_plan.source_audio_track_id.is_none()
            {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartRecord",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "At least one audio or video track is required for StartRecord",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "At least one audio or video track is required for StartRecord",
                );
            }
            let video_encoder_processor_id = source_plan
                .source_video_track_id
                .as_ref()
                .map(|_| format!("obsws:record:{run_id}:video_encoder"));
            let audio_encoder_processor_id = source_plan
                .source_audio_track_id
                .as_ref()
                .map(|_| format!("obsws:record:{run_id}:audio_encoder"));
            let writer_processor_id = format!("obsws:record:{run_id}:mp4_writer");
            let encoded_video_track_id = source_plan
                .source_video_track_id
                .as_ref()
                .map(|_| format!("obsws:record:{run_id}:encoded_video"));
            let encoded_audio_track_id = source_plan
                .source_audio_track_id
                .as_ref()
                .map(|_| format!("obsws:record:{run_id}:encoded_audio"));
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_millis();
            let output_path = input_registry
                .record_directory()
                .join(format!("obsws-record-{timestamp}.mp4"));
            let run = ObswsRecordRun {
                source_processor_id: source_plan.source_processor_id.clone(),
                video_encoder_processor_id,
                audio_encoder_processor_id,
                writer_processor_id,
                source_video_track_id: source_plan.source_video_track_id.clone(),
                source_audio_track_id: source_plan.source_audio_track_id.clone(),
                encoded_video_track_id,
                encoded_audio_track_id,
                output_path: output_path.clone(),
            };
            if let Err(ActivateRecordError::AlreadyActive) =
                input_registry.activate_record(run.clone())
            {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartRecord",
                        request_id,
                        REQUEST_STATUS_OUTPUT_RUNNING,
                        "Record is already active",
                    ),
                    REQUEST_STATUS_OUTPUT_RUNNING,
                    "Record is already active",
                );
            }
            (source_plan, output_path, run)
        };

        if let Some(parent) = output_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            let _ = self.input_registry.write().await.deactivate_record();
            let error_comment = format!("Failed to create record directory: {e}");
            return RequestOutcome::failure(
                Self::build_internal_error_response("StartRecord", request_id, &error_comment),
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                error_comment,
            );
        }

        let start_result = self
            .start_record_processors(&source_plan, &output_path, &run)
            .await;
        if let Err(e) = start_result {
            let _ = self.input_registry.write().await.deactivate_record();
            if let Err(cleanup_error) = self.stop_record_processors(&run).await {
                tracing::warn!(
                    "failed to cleanup record processors after start failure: {}",
                    cleanup_error.display()
                );
            }
            let error_comment = format!("Failed to start record: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response("StartRecord", request_id, &error_comment),
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                error_comment,
            );
        }

        RequestOutcome::success(
            crate::obsws_response_builder::build_start_record_response(request_id, true),
            None,
        )
    }

    pub(super) async fn handle_stop_record(&self, request_id: &str) -> RequestOutcome {
        let run = {
            let input_registry = self.input_registry.read().await;
            if !input_registry.is_record_active() {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StopRecord",
                        request_id,
                        REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "Record is not active",
                    ),
                    REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "Record is not active",
                );
            }
            input_registry
                .record_run()
                .expect("infallible: active record must have run state")
        };
        if let Err(e) = self.stop_record_processors(&run).await {
            let error_comment = format!("Failed to stop record: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response("StopRecord", request_id, &error_comment),
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                error_comment,
            );
        }
        let mut input_registry = self.input_registry.write().await;
        if input_registry.deactivate_record().is_none() {
            tracing::warn!("record runtime was already deactivated while stopping record");
        }
        let output_path = run.output_path.display().to_string();
        RequestOutcome::success(
            crate::obsws_response_builder::build_stop_record_response(request_id, &output_path),
            Some(output_path),
        )
    }

    pub(super) async fn handle_pause_record(&self, request_id: &str) -> RequestOutcome {
        let run = {
            let input_registry = self.input_registry.read().await;
            if !input_registry.is_record_active() {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "PauseRecord",
                        request_id,
                        REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "Record is not active",
                    ),
                    REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "Record is not active",
                );
            }
            if input_registry.is_record_paused() {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "PauseRecord",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Record is already paused",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Record is already paused",
                );
            }
            input_registry
                .record_run()
                .expect("infallible: active record must have run state")
        };

        if let Err(e) = self.pause_record_processors(&run).await {
            let error_comment = format!("Failed to pause record: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response("PauseRecord", request_id, &error_comment),
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                error_comment,
            );
        }

        let mut input_registry = self.input_registry.write().await;
        match input_registry.pause_record() {
            Ok(()) => RequestOutcome::success(
                crate::obsws_response_builder::build_pause_record_response(request_id),
                None,
            ),
            Err(PauseRecordError::RecordNotActive) => RequestOutcome::failure(
                crate::obsws_response_builder::build_request_response_error(
                    "PauseRecord",
                    request_id,
                    REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "Record is not active",
                ),
                REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                "Record is not active",
            ),
            Err(PauseRecordError::AlreadyPaused) => RequestOutcome::failure(
                crate::obsws_response_builder::build_request_response_error(
                    "PauseRecord",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Record is already paused",
                ),
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Record is already paused",
            ),
        }
    }

    pub(super) async fn handle_resume_record(&self, request_id: &str) -> RequestOutcome {
        let run = {
            let input_registry = self.input_registry.read().await;
            if !input_registry.is_record_active() {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "ResumeRecord",
                        request_id,
                        REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "Record is not active",
                    ),
                    REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "Record is not active",
                );
            }
            if !input_registry.is_record_paused() {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "ResumeRecord",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Record is not paused",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Record is not paused",
                );
            }
            input_registry
                .record_run()
                .expect("infallible: active record must have run state")
        };

        if let Err(e) = self.resume_record_processors(&run).await {
            let error_comment = format!("Failed to resume record: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response("ResumeRecord", request_id, &error_comment),
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                error_comment,
            );
        }
        if run.encoded_video_track_id.is_some()
            && let Err(e) = self.request_record_resume_keyframe(&run).await
        {
            if let Err(rollback_error) = self.pause_record_processors(&run).await {
                tracing::warn!(
                    "failed to rollback record resume after keyframe request failure: {}",
                    rollback_error.display()
                );
                match self.stop_record_processors(&run).await {
                    Ok(()) => {
                        let mut input_registry = self.input_registry.write().await;
                        if input_registry.deactivate_record().is_none() {
                            tracing::warn!(
                                "record runtime was already deactivated during resume fallback stop"
                            );
                        }
                        let output_path = run.output_path.display().to_string();
                        let error_comment = format!(
                            "Failed to request record resume keyframe: {}; rollback pause failed: {}; record was forcibly stopped",
                            e.display(),
                            rollback_error.display(),
                        );
                        return RequestOutcome::failure_with_output_path(
                            Self::build_internal_error_response(
                                "ResumeRecord",
                                request_id,
                                &error_comment,
                            ),
                            REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                            error_comment,
                            output_path,
                        );
                    }
                    Err(stop_error) => {
                        let error_comment = format!(
                            "Failed to request record resume keyframe: {}; rollback pause failed: {}; forced stop failed: {}",
                            e.display(),
                            rollback_error.display(),
                            stop_error.display(),
                        );
                        return RequestOutcome::failure(
                            Self::build_internal_error_response(
                                "ResumeRecord",
                                request_id,
                                &error_comment,
                            ),
                            REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                            error_comment,
                        );
                    }
                }
            }
            let error_comment =
                format!("Failed to request record resume keyframe: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response("ResumeRecord", request_id, &error_comment),
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                error_comment,
            );
        }

        let mut input_registry = self.input_registry.write().await;
        match input_registry.resume_record() {
            Ok(()) => RequestOutcome::success(
                crate::obsws_response_builder::build_resume_record_response(request_id),
                None,
            ),
            Err(ResumeRecordError::RecordNotActive) => RequestOutcome::failure(
                crate::obsws_response_builder::build_request_response_error(
                    "ResumeRecord",
                    request_id,
                    REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "Record is not active",
                ),
                REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                "Record is not active",
            ),
            Err(ResumeRecordError::NotPaused) => RequestOutcome::failure(
                crate::obsws_response_builder::build_request_response_error(
                    "ResumeRecord",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Record is not paused",
                ),
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Record is not paused",
            ),
        }
    }

    pub(super) async fn start_stream_processors(
        &self,
        image_path: &str,
        output_url: &str,
        stream_name: Option<&str>,
        run: &ObswsStreamRun,
    ) -> crate::Result<()> {
        let video_encoder_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createVideoEncoder")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("inputTrackId", &run.source_track_id)?;
                    f.member("outputTrackId", &run.encoded_track_id)?;
                    f.member("codec", "H264")?;
                    f.member("bitrateBps", 2_000_000)?;
                    f.member("frameRate", 30)?;
                    f.member("processorId", &run.encoder_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createVideoEncoder", &video_encoder_request)
            .await?;

        let rtmp_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createRtmpOutboundEndpoint")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("outputUrl", output_url)?;
                    if let Some(stream_name) = stream_name {
                        f.member("streamName", stream_name)?;
                    }
                    f.member("inputVideoTrackId", &run.encoded_track_id)?;
                    f.member("processorId", &run.endpoint_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createRtmpOutboundEndpoint", &rtmp_request)
            .await?;

        let png_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createPngFileSource")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("path", image_path)?;
                    f.member("frameRate", 30)?;
                    f.member("outputVideoTrackId", &run.source_track_id)?;
                    f.member("processorId", &run.source_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createPngFileSource", &png_request)
            .await
    }

    pub(super) async fn start_record_processors(
        &self,
        source_plan: &crate::obsws::source::ObswsRecordSourcePlan,
        output_path: &std::path::Path,
        run: &ObswsRecordRun,
    ) -> crate::Result<()> {
        if let (
            Some(source_video_track_id),
            Some(encoded_video_track_id),
            Some(video_encoder_processor_id),
        ) = (
            run.source_video_track_id.as_ref(),
            run.encoded_video_track_id.as_ref(),
            run.video_encoder_processor_id.as_ref(),
        ) {
            let video_encoder_request = nojson::object(|f| {
                f.member("jsonrpc", "2.0")?;
                f.member("id", 1)?;
                f.member("method", "createVideoEncoder")?;
                f.member(
                    "params",
                    nojson::object(|f| {
                        f.member("inputTrackId", source_video_track_id)?;
                        f.member("outputTrackId", encoded_video_track_id)?;
                        f.member("codec", "H264")?;
                        f.member("bitrateBps", 2_000_000)?;
                        f.member("frameRate", 30)?;
                        f.member("processorId", video_encoder_processor_id)
                    }),
                )
            })
            .to_string();
            self.send_pipeline_rpc_request("createVideoEncoder", &video_encoder_request)
                .await?;
        }

        if let (
            Some(source_audio_track_id),
            Some(encoded_audio_track_id),
            Some(audio_encoder_processor_id),
        ) = (
            run.source_audio_track_id.as_ref(),
            run.encoded_audio_track_id.as_ref(),
            run.audio_encoder_processor_id.as_ref(),
        ) {
            let audio_encoder_request = nojson::object(|f| {
                f.member("jsonrpc", "2.0")?;
                f.member("id", 1)?;
                f.member("method", "createAudioEncoder")?;
                f.member(
                    "params",
                    nojson::object(|f| {
                        f.member("inputTrackId", source_audio_track_id)?;
                        f.member("outputTrackId", encoded_audio_track_id)?;
                        f.member("codec", "OPUS")?;
                        f.member("bitrateBps", 128_000)?;
                        f.member("processorId", audio_encoder_processor_id)
                    }),
                )
            })
            .to_string();
            self.send_pipeline_rpc_request("createAudioEncoder", &audio_encoder_request)
                .await?;
        }

        let writer_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createMp4Writer")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("outputPath", output_path.display().to_string())?;
                    if let Some(encoded_audio_track_id) = &run.encoded_audio_track_id {
                        f.member("inputAudioTrackId", encoded_audio_track_id)?;
                    }
                    if let Some(encoded_video_track_id) = &run.encoded_video_track_id {
                        f.member("inputVideoTrackId", encoded_video_track_id)?;
                    }
                    f.member("processorId", &run.writer_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createMp4Writer", &writer_request)
            .await?;

        for request in &source_plan.requests {
            self.send_pipeline_rpc_request(request.method, &request.request_text)
                .await?;
        }

        Ok(())
    }

    pub(super) async fn pause_record_processors(&self, run: &ObswsRecordRun) -> crate::Result<()> {
        self.send_record_writer_rpc(run, RecordWriterRpcOperation::Pause)
            .await
    }

    pub(super) async fn resume_record_processors(&self, run: &ObswsRecordRun) -> crate::Result<()> {
        self.send_record_writer_rpc(run, RecordWriterRpcOperation::Resume)
            .await
    }

    pub(super) async fn send_record_writer_rpc(
        &self,
        run: &ObswsRecordRun,
        operation: RecordWriterRpcOperation,
    ) -> crate::Result<()> {
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            return Err(crate::Error::new(
                "BUG: obsws pipeline handle is not initialized",
            ));
        };

        let writer_processor_id = crate::ProcessorId::new(run.writer_processor_id.clone());
        let writer_rpc_sender = pipeline_handle
            .get_rpc_sender::<
                tokio::sync::mpsc::UnboundedSender<crate::writer_mp4::Mp4WriterRpcMessage>,
            >(&writer_processor_id)
            .await
            .map_err(|e| {
                crate::Error::new(format!(
                    "failed to get record writer RPC sender ({writer_processor_id}): {e}"
                ))
            })?;

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let rpc_message = match operation {
            RecordWriterRpcOperation::Pause => {
                crate::writer_mp4::Mp4WriterRpcMessage::Pause { reply_tx }
            }
            RecordWriterRpcOperation::Resume => {
                crate::writer_mp4::Mp4WriterRpcMessage::Resume { reply_tx }
            }
        };
        writer_rpc_sender.send(rpc_message).map_err(|_| {
            crate::Error::new(format!(
                "failed to send {} RPC to record writer: {}",
                operation.as_str(),
                run.writer_processor_id
            ))
        })?;
        reply_rx.await.map_err(|_| {
            crate::Error::new(format!(
                "failed to receive {} RPC response from record writer",
                operation.as_str(),
            ))
        })?
    }

    pub(super) async fn request_record_resume_keyframe(
        &self,
        run: &ObswsRecordRun,
    ) -> crate::Result<()> {
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            return Err(crate::Error::new(
                "BUG: obsws pipeline handle is not initialized",
            ));
        };

        let Some(video_encoder_processor_id) = run.video_encoder_processor_id.as_ref() else {
            return Ok(());
        };
        let encoder_processor_id = crate::ProcessorId::new(video_encoder_processor_id.clone());
        let encoder_rpc_sender = pipeline_handle
            .get_rpc_sender::<
                tokio::sync::mpsc::UnboundedSender<crate::encoder::VideoEncoderRpcMessage>,
            >(&encoder_processor_id)
            .await
            .map_err(|e| {
                crate::Error::new(format!(
                    "failed to get record encoder RPC sender ({encoder_processor_id}): {e}"
                ))
            })?;
        encoder_rpc_sender
            .send(crate::encoder::VideoEncoderRpcMessage::RequestKeyframe)
            .map_err(|_| {
                crate::Error::new(format!(
                    "failed to send keyframe request to record encoder: {}",
                    video_encoder_processor_id
                ))
            })
    }

    pub(super) async fn send_pipeline_rpc_request(
        &self,
        method: &str,
        request_text: &str,
    ) -> crate::Result<()> {
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            return Err(crate::Error::new(
                "BUG: obsws pipeline handle is not initialized",
            ));
        };
        let Some(response_json) = pipeline_handle.rpc(request_text.as_bytes()).await else {
            return Err(crate::Error::new(format!(
                "failed to run {method}: response is missing",
            )));
        };

        if let Some(error_value) = response_json.value().to_member("error")?.optional() {
            let message = error_value
                .to_member("message")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok())
                .unwrap_or_else(|| "unknown rpc error".to_owned());
            return Err(crate::Error::new(format!(
                "failed to run {method}: {message}"
            )));
        }

        Ok(())
    }

    pub(super) async fn stop_stream_processors(&self, run: &ObswsStreamRun) -> crate::Result<()> {
        self.stop_processors(&[
            crate::ProcessorId::new(run.endpoint_processor_id.clone()),
            crate::ProcessorId::new(run.encoder_processor_id.clone()),
            crate::ProcessorId::new(run.source_processor_id.clone()),
        ])
        .await
    }

    pub(super) async fn stop_record_processors(&self, run: &ObswsRecordRun) -> crate::Result<()> {
        let mut processor_ids = vec![
            crate::ProcessorId::new(run.writer_processor_id.clone()),
            crate::ProcessorId::new(run.source_processor_id.clone()),
        ];
        if let Some(video_encoder_processor_id) = &run.video_encoder_processor_id {
            processor_ids.push(crate::ProcessorId::new(video_encoder_processor_id.clone()));
        }
        if let Some(audio_encoder_processor_id) = &run.audio_encoder_processor_id {
            processor_ids.push(crate::ProcessorId::new(audio_encoder_processor_id.clone()));
        }
        self.stop_processors(&processor_ids).await
    }

    pub(super) async fn stop_processors(
        &self,
        processor_ids: &[crate::ProcessorId],
    ) -> crate::Result<()> {
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            return Err(crate::Error::new(
                "BUG: obsws pipeline handle is not initialized",
            ));
        };

        let mut terminate_error = None;
        for processor_id in processor_ids {
            if pipeline_handle
                .terminate_processor(processor_id.clone())
                .await
                .is_err()
                && terminate_error.is_none()
            {
                terminate_error = Some(crate::Error::new(
                    "failed to terminate processor: pipeline has terminated",
                ));
            }
        }

        self.wait_processors_stopped(pipeline_handle, processor_ids, Duration::from_secs(2))
            .await?;

        if let Some(e) = terminate_error {
            return Err(e);
        }

        Ok(())
    }

    pub(super) async fn wait_processors_stopped(
        &self,
        pipeline_handle: &crate::MediaPipelineHandle,
        processor_ids: &[crate::ProcessorId],
        timeout: Duration,
    ) -> crate::Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let live_processors = pipeline_handle.list_processors().await.map_err(|_| {
                crate::Error::new("failed to list processors: pipeline has terminated")
            })?;
            if processor_ids
                .iter()
                .all(|processor_id| !live_processors.iter().any(|id| id == processor_id))
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let pending = processor_ids
                    .iter()
                    .filter(|processor_id| live_processors.iter().any(|id| id == *processor_id))
                    .map(|processor_id| processor_id.get().to_owned())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(crate::Error::new(format!(
                    "processors did not terminate in time: {pending}"
                )));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
