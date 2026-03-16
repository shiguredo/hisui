use super::*;

impl ObswsSession {
    pub(super) async fn handle_start_stream(&self, request_id: &str) -> RequestOutcome {
        let (output_url, stream_name, output_plan, run) = {
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

            let scene_inputs = input_registry.list_current_program_scene_input_entries();
            let run_id = input_registry.next_stream_run_id();
            let canvas_width = input_registry.canvas_width();
            let canvas_height = input_registry.canvas_height();
            let output_plan = match crate::obsws::output_plan::build_composed_output_plan(
                &scene_inputs,
                crate::obsws::source::ObswsOutputKind::Stream,
                run_id,
                canvas_width,
                canvas_height,
            ) {
                Ok(output_plan) => output_plan,
                Err(error) => {
                    let error_comment = error.message("StartStream");
                    return RequestOutcome::failure(
                        crate::obsws_response_builder::build_request_response_error(
                            "StartStream",
                            request_id,
                            REQUEST_STATUS_INVALID_REQUEST_FIELD,
                            &error_comment,
                        ),
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        error_comment,
                    );
                }
            };
            let video = output_plan
                .source_video_track_id
                .as_ref()
                .map(|source_track_id| ObswsRecordTrackRun {
                    encoder_processor_id: crate::ProcessorId::new(format!(
                        "obsws:stream:{run_id}:video_encoder"
                    )),
                    source_track_id: source_track_id.clone(),
                    encoded_track_id: crate::TrackId::new(format!(
                        "obsws:stream:{run_id}:encoded_video"
                    )),
                });
            let audio = output_plan
                .source_audio_track_id
                .as_ref()
                .map(|source_track_id| ObswsRecordTrackRun {
                    encoder_processor_id: crate::ProcessorId::new(format!(
                        "obsws:stream:{run_id}:audio_encoder"
                    )),
                    source_track_id: source_track_id.clone(),
                    encoded_track_id: crate::TrackId::new(format!(
                        "obsws:stream:{run_id}:encoded_audio"
                    )),
                });
            let run = ObswsStreamRun {
                source_processor_ids: output_plan.source_processor_ids.clone(),
                video,
                audio,
                audio_mixer_processor_id: output_plan.audio_mixer_processor_id.clone(),
                video_mixer_processor_id: output_plan.video_mixer_processor_id.clone(),
                publisher_processor_id: crate::ProcessorId::new(format!(
                    "obsws:stream:{run_id}:rtmp_publisher"
                )),
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

            (output_url, stream_service_settings.key, output_plan, run)
        };

        let start_result = self
            .start_stream_processors(&output_plan, &output_url, stream_name.as_deref(), &run)
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
        let (output_plan, output_path, run) = {
            let mut input_registry = self.input_registry.write().await;
            let scene_inputs = input_registry.list_current_program_scene_input_entries();
            let run_id = input_registry.next_record_run_id();
            let canvas_width = input_registry.canvas_width();
            let canvas_height = input_registry.canvas_height();
            let output_plan = match crate::obsws::output_plan::build_composed_output_plan(
                &scene_inputs,
                crate::obsws::source::ObswsOutputKind::Record,
                run_id,
                canvas_width,
                canvas_height,
            ) {
                Ok(output_plan) => output_plan,
                Err(error) => {
                    let error_comment = error.message("StartRecord");
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
            let writer_processor_id =
                crate::ProcessorId::new(format!("obsws:record:{run_id}:mp4_writer"));
            let video = output_plan
                .source_video_track_id
                .as_ref()
                .map(|source_track_id| ObswsRecordTrackRun {
                    encoder_processor_id: crate::ProcessorId::new(format!(
                        "obsws:record:{run_id}:video_encoder"
                    )),
                    source_track_id: source_track_id.clone(),
                    encoded_track_id: crate::TrackId::new(format!(
                        "obsws:record:{run_id}:encoded_video"
                    )),
                });
            let audio = output_plan
                .source_audio_track_id
                .as_ref()
                .map(|source_track_id| ObswsRecordTrackRun {
                    encoder_processor_id: crate::ProcessorId::new(format!(
                        "obsws:record:{run_id}:audio_encoder"
                    )),
                    source_track_id: source_track_id.clone(),
                    encoded_track_id: crate::TrackId::new(format!(
                        "obsws:record:{run_id}:encoded_audio"
                    )),
                });
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_millis();
            let output_path = input_registry
                .record_directory()
                .join(format!("obsws-record-{timestamp}.mp4"));
            let run = ObswsRecordRun {
                source_processor_ids: output_plan.source_processor_ids.clone(),
                video,
                audio,
                audio_mixer_processor_id: output_plan.audio_mixer_processor_id.clone(),
                video_mixer_processor_id: output_plan.video_mixer_processor_id.clone(),
                writer_processor_id,
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
            (output_plan, output_path, run)
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
            .start_record_processors(&output_plan, &output_path, &run)
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
        if run.video.is_some()
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

    /// createVideoMixer リクエストを生成して送信する
    async fn send_create_video_mixer_request(
        &self,
        output_plan: &crate::obsws::output_plan::ObswsComposedOutputPlan,
        video: &ObswsRecordTrackRun,
        video_mixer_processor_id: &crate::ProcessorId,
    ) -> crate::Result<()> {
        let video_mixer_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createVideoMixer")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("canvasWidth", output_plan.canvas_width)?;
                    f.member("canvasHeight", output_plan.canvas_height)?;
                    f.member("frameRate", 30)?;
                    f.member(
                        "inputTracks",
                        nojson::array(|f| {
                            for input_track in &output_plan.video_mixer_input_tracks {
                                f.element(nojson::object(|f| {
                                    f.member("trackId", &input_track.track_id)?;
                                    f.member("x", input_track.x)?;
                                    f.member("y", input_track.y)?;
                                    f.member("z", input_track.z)?;
                                    if let Some(width) = input_track.width {
                                        f.member("width", width)?;
                                    }
                                    if let Some(height) = input_track.height {
                                        f.member("height", height)?;
                                    }
                                    if let Some(scale_x) = input_track.scale_x {
                                        f.member("scaleX", scale_x)?;
                                    }
                                    if let Some(scale_y) = input_track.scale_y {
                                        f.member("scaleY", scale_y)?;
                                    }
                                    if input_track.crop_top != 0 {
                                        f.member("cropTop", input_track.crop_top)?;
                                    }
                                    if input_track.crop_bottom != 0 {
                                        f.member("cropBottom", input_track.crop_bottom)?;
                                    }
                                    if input_track.crop_left != 0 {
                                        f.member("cropLeft", input_track.crop_left)?;
                                    }
                                    if input_track.crop_right != 0 {
                                        f.member("cropRight", input_track.crop_right)?;
                                    }
                                    Ok(())
                                }))?;
                            }
                            Ok(())
                        }),
                    )?;
                    f.member("outputTrackId", &video.source_track_id)?;
                    f.member("processorId", video_mixer_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createVideoMixer", &video_mixer_request)
            .await
    }

    /// createAudioMixer リクエストを生成して送信する
    async fn send_create_audio_mixer_request(
        &self,
        source_plans: &[crate::obsws::source::ObswsRecordSourcePlan],
        audio: &ObswsRecordTrackRun,
        audio_mixer_processor_id: &crate::ProcessorId,
    ) -> crate::Result<()> {
        let audio_mixer_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createAudioMixer")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("sampleRate", 48_000)?;
                    f.member("channels", 2)?;
                    f.member("frameDurationMs", 20)?;
                    f.member("timestampRebaseThresholdMs", 100)?;
                    f.member("terminateOnInputEos", true)?;
                    f.member(
                        "inputTracks",
                        nojson::array(|f| {
                            for source_plan in source_plans {
                                if let Some(source_audio_track_id) =
                                    &source_plan.source_audio_track_id
                                {
                                    f.element(nojson::object(|f| {
                                        f.member("trackId", source_audio_track_id)
                                    }))?;
                                }
                            }
                            Ok(())
                        }),
                    )?;
                    f.member("outputTrackId", &audio.source_track_id)?;
                    f.member("processorId", audio_mixer_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createAudioMixer", &audio_mixer_request)
            .await
    }

    pub(super) async fn start_stream_processors(
        &self,
        output_plan: &crate::obsws::output_plan::ObswsComposedOutputPlan,
        output_url: &str,
        stream_name: Option<&str>,
        run: &ObswsStreamRun,
    ) -> crate::Result<()> {
        if let (Some(audio), Some(audio_mixer_processor_id)) =
            (&run.audio, &run.audio_mixer_processor_id)
        {
            self.send_create_audio_mixer_request(
                &output_plan.source_plans,
                audio,
                audio_mixer_processor_id,
            )
            .await?;
        }

        if let (Some(video), Some(video_mixer_processor_id)) =
            (&run.video, &run.video_mixer_processor_id)
        {
            self.send_create_video_mixer_request(output_plan, video, video_mixer_processor_id)
                .await?;
        }

        if let Some(video) = &run.video {
            let video_encoder_request = nojson::object(|f| {
                f.member("jsonrpc", "2.0")?;
                f.member("id", 1)?;
                f.member("method", "createVideoEncoder")?;
                f.member(
                    "params",
                    nojson::object(|f| {
                        f.member("inputTrackId", &video.source_track_id)?;
                        f.member("outputTrackId", &video.encoded_track_id)?;
                        f.member("codec", "H264")?;
                        f.member("bitrateBps", 2_000_000)?;
                        f.member("frameRate", 30)?;
                        f.member("processorId", &video.encoder_processor_id)
                    }),
                )
            })
            .to_string();
            self.send_pipeline_rpc_request("createVideoEncoder", &video_encoder_request)
                .await?;
        }

        if let Some(audio) = &run.audio {
            let audio_encoder_request = nojson::object(|f| {
                f.member("jsonrpc", "2.0")?;
                f.member("id", 1)?;
                f.member("method", "createAudioEncoder")?;
                f.member(
                    "params",
                    nojson::object(|f| {
                        f.member("inputTrackId", &audio.source_track_id)?;
                        f.member("outputTrackId", &audio.encoded_track_id)?;
                        f.member("codec", "AAC")?;
                        f.member("bitrateBps", 128_000)?;
                        f.member("processorId", &audio.encoder_processor_id)
                    }),
                )
            })
            .to_string();
            self.send_pipeline_rpc_request("createAudioEncoder", &audio_encoder_request)
                .await?;
        }

        let rtmp_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createRtmpPublisher")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("outputUrl", output_url)?;
                    if let Some(stream_name) = stream_name {
                        f.member("streamName", stream_name)?;
                    }
                    if let Some(audio) = &run.audio {
                        f.member("inputAudioTrackId", &audio.encoded_track_id)?;
                    }
                    if let Some(video) = &run.video {
                        f.member("inputVideoTrackId", &video.encoded_track_id)?;
                    }
                    f.member("processorId", &run.publisher_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createRtmpPublisher", &rtmp_request)
            .await?;

        for source_plan in &output_plan.source_plans {
            for request in &source_plan.requests {
                self.send_pipeline_rpc_request(request.method, &request.request_text)
                    .await?;
            }
        }

        Ok(())
    }

    pub(super) async fn start_record_processors(
        &self,
        output_plan: &crate::obsws::output_plan::ObswsComposedOutputPlan,
        output_path: &std::path::Path,
        run: &ObswsRecordRun,
    ) -> crate::Result<()> {
        if let (Some(audio), Some(audio_mixer_processor_id)) =
            (&run.audio, &run.audio_mixer_processor_id)
        {
            self.send_create_audio_mixer_request(
                &output_plan.source_plans,
                audio,
                audio_mixer_processor_id,
            )
            .await?;
        }

        if let (Some(video), Some(video_mixer_processor_id)) =
            (&run.video, &run.video_mixer_processor_id)
        {
            self.send_create_video_mixer_request(output_plan, video, video_mixer_processor_id)
                .await?;
        }

        if let Some(video) = &run.video {
            let video_encoder_request = nojson::object(|f| {
                f.member("jsonrpc", "2.0")?;
                f.member("id", 1)?;
                f.member("method", "createVideoEncoder")?;
                f.member(
                    "params",
                    nojson::object(|f| {
                        f.member("inputTrackId", &video.source_track_id)?;
                        f.member("outputTrackId", &video.encoded_track_id)?;
                        f.member("codec", "H264")?;
                        f.member("bitrateBps", 2_000_000)?;
                        f.member("frameRate", 30)?;
                        f.member("processorId", &video.encoder_processor_id)
                    }),
                )
            })
            .to_string();
            self.send_pipeline_rpc_request("createVideoEncoder", &video_encoder_request)
                .await?;
        }

        if let Some(audio) = &run.audio {
            let audio_encoder_request = nojson::object(|f| {
                f.member("jsonrpc", "2.0")?;
                f.member("id", 1)?;
                f.member("method", "createAudioEncoder")?;
                f.member(
                    "params",
                    nojson::object(|f| {
                        f.member("inputTrackId", &audio.source_track_id)?;
                        f.member("outputTrackId", &audio.encoded_track_id)?;
                        f.member("codec", "OPUS")?;
                        f.member("bitrateBps", 128_000)?;
                        f.member("processorId", &audio.encoder_processor_id)
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
                    if let Some(audio) = &run.audio {
                        f.member("inputAudioTrackId", &audio.encoded_track_id)?;
                    }
                    if let Some(video) = &run.video {
                        f.member("inputVideoTrackId", &video.encoded_track_id)?;
                    }
                    f.member("processorId", &run.writer_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createMp4Writer", &writer_request)
            .await?;

        for source_plan in &output_plan.source_plans {
            for request in &source_plan.requests {
                self.send_pipeline_rpc_request(request.method, &request.request_text)
                    .await?;
            }
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

        let writer_processor_id = run.writer_processor_id.clone();
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

        let Some(video) = run.video.as_ref() else {
            return Ok(());
        };
        let encoder_processor_id = video.encoder_processor_id.clone();
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
                    video.encoder_processor_id
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

    /// ソース → ミキサー → エンコーダー → パブリッシャーの順に段階的に停止する。
    pub(super) async fn stop_stream_processors(&self, run: &ObswsStreamRun) -> crate::Result<()> {
        // 1. ソースを停止
        self.stop_processors(&run.source_processor_ids).await?;

        // 2. 音声ミキサー + 映像ミキサーを停止
        {
            let mut mixer_ids = Vec::new();
            if let Some(mixer_id) = &run.audio_mixer_processor_id {
                mixer_ids.push(mixer_id.clone());
            }
            if let Some(mixer_id) = &run.video_mixer_processor_id {
                mixer_ids.push(mixer_id.clone());
            }
            if !mixer_ids.is_empty() {
                self.stop_processors(&mixer_ids).await?;
            }
        }

        // 3. エンコーダーを停止
        {
            let mut ids = Vec::new();
            if let Some(video) = &run.video {
                ids.push(video.encoder_processor_id.clone());
            }
            if let Some(audio) = &run.audio {
                ids.push(audio.encoder_processor_id.clone());
            }
            if !ids.is_empty() {
                self.stop_processors(&ids).await?;
            }
        }

        // 4. パブリッシャーを停止
        self.stop_processors(std::slice::from_ref(&run.publisher_processor_id))
            .await?;

        Ok(())
    }

    /// ソース → ミキサー → エンコーダー → ライターの順に段階的に停止する。
    /// EOS がパイプラインを伝播してから次の段階を停止することで、
    /// MP4 writer の finalize が確実に完了するようにする。
    pub(super) async fn stop_record_processors(&self, run: &ObswsRecordRun) -> crate::Result<()> {
        // 1. ソースを停止（データ生産を止める）
        self.stop_processors(&run.source_processor_ids).await?;

        // 2. 音声ミキサー + 映像ミキサーを停止（EOS をエンコーダーに伝播）
        {
            let mut mixer_ids = Vec::new();
            if let Some(mixer_id) = &run.audio_mixer_processor_id {
                mixer_ids.push(mixer_id.clone());
            }
            if let Some(mixer_id) = &run.video_mixer_processor_id {
                mixer_ids.push(mixer_id.clone());
            }
            if !mixer_ids.is_empty() {
                self.stop_processors(&mixer_ids).await?;
            }
        }

        // 3. エンコーダーを停止（EOS をライターに伝播）
        {
            let mut ids = Vec::new();
            if let Some(video) = &run.video {
                ids.push(video.encoder_processor_id.clone());
            }
            if let Some(audio) = &run.audio {
                ids.push(audio.encoder_processor_id.clone());
            }
            if !ids.is_empty() {
                self.stop_processors(&ids).await?;
            }
        }

        // 4. ライターを停止（finalize を完了させる）
        self.stop_processors(std::slice::from_ref(&run.writer_processor_id))
            .await?;

        Ok(())
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
