use super::*;

impl ObswsSession {
    // --- リクエストハンドラ（handle_request_internal から委譲される） ---

    pub(super) async fn handle_start_stream_request(
        &self,
        request_id: &str,
    ) -> crate::Result<RequestExecutionResult> {
        let outcome = self.handle_start_stream("StartStream", request_id).await;
        let mut events = Vec::new();
        if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
            // hisui はストリーム開始が同期的に完了するため即座に STARTED に遷移する。
            // OBS は STARTING を response 前に送信するが、hisui は response 後にまとめて送信する。
            events.push(
                crate::obsws_response_builder::build_stream_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STARTING",
                ),
            );
            events.push(
                crate::obsws_response_builder::build_stream_state_changed_event(
                    true,
                    "OBS_WEBSOCKET_OUTPUT_STARTED",
                ),
            );
        }
        Self::build_execution_from_outcome(outcome, events)
    }

    pub(super) async fn handle_stop_stream_request(
        &self,
        request_id: &str,
    ) -> crate::Result<RequestExecutionResult> {
        let outcome = self.handle_stop_stream("StopStream", request_id).await;
        let mut events = Vec::new();
        if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
            events.push(
                crate::obsws_response_builder::build_stream_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPING",
                ),
            );
            events.push(
                crate::obsws_response_builder::build_stream_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPED",
                ),
            );
        }
        Self::build_execution_from_outcome(outcome, events)
    }

    pub(super) async fn handle_toggle_stream_request(
        &self,
        request_id: &str,
    ) -> crate::Result<RequestExecutionResult> {
        let was_active = self.input_registry.read().await.is_stream_active();
        let outcome = if was_active {
            self.handle_stop_stream("ToggleStream", request_id).await
        } else {
            self.handle_start_stream("ToggleStream", request_id).await
        };
        let mut events = Vec::new();
        if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
            if was_active {
                events.push(
                    crate::obsws_response_builder::build_stream_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPING",
                    ),
                );
                events.push(
                    crate::obsws_response_builder::build_stream_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPED",
                    ),
                );
            } else {
                events.push(
                    crate::obsws_response_builder::build_stream_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STARTING",
                    ),
                );
                events.push(
                    crate::obsws_response_builder::build_stream_state_changed_event(
                        true,
                        "OBS_WEBSOCKET_OUTPUT_STARTED",
                    ),
                );
            }
        }
        let response_text = Self::build_toggle_response_from_outcome(
            "ToggleStream",
            request_id,
            !was_active,
            &outcome,
        )?;
        Self::build_execution_from_response_text(response_text, events)
    }

    pub(super) async fn handle_start_record_request(
        &self,
        request_id: &str,
    ) -> crate::Result<RequestExecutionResult> {
        let outcome = self.handle_start_record("StartRecord", request_id).await;
        let mut events = Vec::new();
        if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
            events.push(
                crate::obsws_response_builder::build_record_state_changed_event(
                    true,
                    "OBS_WEBSOCKET_OUTPUT_STARTED",
                    None,
                ),
            );
        }
        Self::build_execution_from_outcome(outcome, events)
    }

    pub(super) async fn handle_stop_record_request(
        &self,
        request_id: &str,
    ) -> crate::Result<RequestExecutionResult> {
        let outcome = self.handle_stop_record("StopRecord", request_id).await;
        let mut events = Vec::new();
        if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
            events.push(
                crate::obsws_response_builder::build_record_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPED",
                    outcome.output_path.as_deref(),
                ),
            );
        }
        Self::build_execution_from_outcome(outcome, events)
    }

    pub(super) async fn handle_toggle_record_request(
        &self,
        request_id: &str,
    ) -> crate::Result<RequestExecutionResult> {
        let was_active = self.input_registry.read().await.is_record_active();
        let outcome = if was_active {
            self.handle_stop_record("ToggleRecord", request_id).await
        } else {
            self.handle_start_record("ToggleRecord", request_id).await
        };
        let mut events = Vec::new();
        if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
            let (output_state, output_path) = if was_active {
                (
                    "OBS_WEBSOCKET_OUTPUT_STOPPED",
                    outcome.output_path.as_deref(),
                )
            } else {
                ("OBS_WEBSOCKET_OUTPUT_STARTED", None)
            };
            events.push(
                crate::obsws_response_builder::build_record_state_changed_event(
                    !was_active,
                    output_state,
                    output_path,
                ),
            );
        }
        let response_text = Self::build_toggle_response_from_outcome(
            "ToggleRecord",
            request_id,
            !was_active,
            &outcome,
        )?;
        Self::build_execution_from_response_text(response_text, events)
    }

    pub(super) async fn handle_pause_record_request(
        &self,
        request_id: &str,
    ) -> crate::Result<RequestExecutionResult> {
        let outcome = self.handle_pause_record(request_id).await;
        let mut events = Vec::new();
        if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
            events.push(
                crate::obsws_response_builder::build_record_state_changed_event(
                    true,
                    "OBS_WEBSOCKET_OUTPUT_PAUSED",
                    None,
                ),
            );
        }
        Self::build_execution_from_outcome(outcome, events)
    }

    pub(super) async fn handle_resume_record_request(
        &self,
        request_id: &str,
    ) -> crate::Result<RequestExecutionResult> {
        let outcome = self.handle_resume_record(request_id).await;
        let mut events = Vec::new();
        if self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
            if outcome.success {
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        true,
                        "OBS_WEBSOCKET_OUTPUT_RESUMED",
                        None,
                    ),
                );
            } else if outcome.output_path.is_some() {
                // [NOTE]
                // ResumeRecord の内部復旧で録画停止へフォールバックした場合は、
                // request 自体は失敗でも出力状態の遷移（ inactive ）を通知する。
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPED",
                        outcome.output_path.as_deref(),
                    ),
                );
            }
        }
        Self::build_execution_from_outcome(outcome, events)
    }

    pub(super) async fn handle_toggle_record_pause_request(
        &self,
        request_id: &str,
    ) -> crate::Result<RequestExecutionResult> {
        let was_paused = self.input_registry.read().await.is_record_paused();
        let outcome = if was_paused {
            self.handle_resume_record(request_id).await
        } else {
            self.handle_pause_record(request_id).await
        };
        let mut events = Vec::new();
        if self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
            if outcome.success {
                let output_state = if !was_paused {
                    "OBS_WEBSOCKET_OUTPUT_PAUSED"
                } else {
                    "OBS_WEBSOCKET_OUTPUT_RESUMED"
                };
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        true,
                        output_state,
                        None,
                    ),
                );
            } else if outcome.output_path.is_some() {
                // [NOTE]
                // ToggleRecordPause が resume 経路で内部復旧に失敗して
                // 録画停止へフォールバックした場合は、request 自体は失敗でも
                // 出力状態の遷移（ inactive ）を通知する。
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPED",
                        outcome.output_path.as_deref(),
                    ),
                );
            }
        }
        let response_text = Self::build_toggle_response_from_outcome(
            "ToggleRecordPause",
            request_id,
            !was_paused,
            &outcome,
        )?;
        Self::build_execution_from_response_text(response_text, events)
    }

    pub(super) async fn handle_start_output_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> crate::Result<RequestExecutionResult> {
        let Some(output_name) =
            Self::parse_required_non_empty_string_request_field(request_data, "outputName")
        else {
            return Ok(Self::build_error_execution(
                "StartOutput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            ));
        };
        let (outcome, events) = match output_name.as_str() {
            "stream" => {
                let outcome = self.handle_start_stream("StartOutput", request_id).await;
                let mut events = Vec::new();
                if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                    events.push(
                        crate::obsws_response_builder::build_stream_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STARTING",
                        ),
                    );
                    events.push(
                        crate::obsws_response_builder::build_stream_state_changed_event(
                            true,
                            "OBS_WEBSOCKET_OUTPUT_STARTED",
                        ),
                    );
                }
                (outcome, events)
            }
            "record" => {
                let outcome = self.handle_start_record("StartOutput", request_id).await;
                let mut events = Vec::new();
                if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                    events.push(
                        crate::obsws_response_builder::build_record_state_changed_event(
                            true,
                            "OBS_WEBSOCKET_OUTPUT_STARTED",
                            None,
                        ),
                    );
                }
                (outcome, events)
            }
            _ => {
                return Ok(Self::build_error_execution(
                    "StartOutput",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                ));
            }
        };
        let response_text =
            Self::build_output_response_from_outcome("StartOutput", request_id, true, &outcome);
        Self::build_execution_from_response_text(response_text, events)
    }

    pub(super) async fn handle_stop_output_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> crate::Result<RequestExecutionResult> {
        let Some(output_name) =
            Self::parse_required_non_empty_string_request_field(request_data, "outputName")
        else {
            return Ok(Self::build_error_execution(
                "StopOutput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            ));
        };
        let (outcome, events) = match output_name.as_str() {
            "stream" => {
                let outcome = self.handle_stop_stream("StopOutput", request_id).await;
                let mut events = Vec::new();
                if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                    events.push(
                        crate::obsws_response_builder::build_stream_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPING",
                        ),
                    );
                    events.push(
                        crate::obsws_response_builder::build_stream_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPED",
                        ),
                    );
                }
                (outcome, events)
            }
            "record" => {
                let outcome = self.handle_stop_record("StopOutput", request_id).await;
                let mut events = Vec::new();
                if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                    events.push(
                        crate::obsws_response_builder::build_record_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPED",
                            outcome.output_path.as_deref(),
                        ),
                    );
                }
                (outcome, events)
            }
            _ => {
                return Ok(Self::build_error_execution(
                    "StopOutput",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                ));
            }
        };
        let response_text =
            Self::build_output_response_from_outcome("StopOutput", request_id, false, &outcome);
        Self::build_execution_from_response_text(response_text, events)
    }

    pub(super) async fn handle_toggle_output_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> crate::Result<RequestExecutionResult> {
        let Some(output_name) =
            Self::parse_required_non_empty_string_request_field(request_data, "outputName")
        else {
            return Ok(Self::build_error_execution(
                "ToggleOutput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            ));
        };
        let (outcome, output_active_on_success, events) = match output_name.as_str() {
            "stream" => {
                let was_active = self.input_registry.read().await.is_stream_active();
                let outcome = if was_active {
                    self.handle_stop_stream("ToggleOutput", request_id).await
                } else {
                    self.handle_start_stream("ToggleOutput", request_id).await
                };
                let mut events = Vec::new();
                if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                    if was_active {
                        events.push(
                            crate::obsws_response_builder::build_stream_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPING",
                            ),
                        );
                        events.push(
                            crate::obsws_response_builder::build_stream_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPED",
                            ),
                        );
                    } else {
                        events.push(
                            crate::obsws_response_builder::build_stream_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STARTING",
                            ),
                        );
                        events.push(
                            crate::obsws_response_builder::build_stream_state_changed_event(
                                true,
                                "OBS_WEBSOCKET_OUTPUT_STARTED",
                            ),
                        );
                    }
                }
                (outcome, !was_active, events)
            }
            "record" => {
                let was_active = self.input_registry.read().await.is_record_active();
                let outcome = if was_active {
                    self.handle_stop_record("ToggleOutput", request_id).await
                } else {
                    self.handle_start_record("ToggleOutput", request_id).await
                };
                let mut events = Vec::new();
                if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                    let (output_state, output_path) = if was_active {
                        (
                            "OBS_WEBSOCKET_OUTPUT_STOPPED",
                            outcome.output_path.as_deref(),
                        )
                    } else {
                        ("OBS_WEBSOCKET_OUTPUT_STARTED", None)
                    };
                    events.push(
                        crate::obsws_response_builder::build_record_state_changed_event(
                            !was_active,
                            output_state,
                            output_path,
                        ),
                    );
                }
                (outcome, !was_active, events)
            }
            _ => {
                return Ok(Self::build_error_execution(
                    "ToggleOutput",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                ));
            }
        };
        let response_text = Self::build_output_response_from_outcome(
            "ToggleOutput",
            request_id,
            output_active_on_success,
            &outcome,
        );
        Self::build_execution_from_response_text(response_text, events)
    }

    // --- 内部操作メソッド ---

    /// output_plan の構築を行い、失敗時は RequestOutcome を返すヘルパー
    fn build_output_plan_or_error(
        request_type: &str,
        request_id: &str,
        input_registry: &ObswsInputRegistry,
        output_kind: crate::obsws::source::ObswsOutputKind,
        run_id: u64,
    ) -> Result<crate::obsws::output_plan::ObswsComposedOutputPlan, RequestOutcome> {
        let scene_inputs = input_registry.list_current_program_scene_input_entries();
        let canvas_width = input_registry.canvas_width();
        let canvas_height = input_registry.canvas_height();
        let frame_rate = input_registry.frame_rate();
        crate::obsws::output_plan::build_composed_output_plan(
            &scene_inputs,
            output_kind,
            run_id,
            canvas_width,
            canvas_height,
            frame_rate,
        )
        .map_err(|error| {
            let error_comment = error.message();
            RequestOutcome::failure(
                crate::obsws_response_builder::build_request_response_error(
                    request_type,
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &error_comment,
                ),
                None,
            )
        })
    }

    pub(super) async fn handle_start_stream(
        &self,
        request_type: &str,
        request_id: &str,
    ) -> RequestOutcome {
        let (output_url, stream_name, output_plan, run) = {
            let mut input_registry = self.input_registry.write().await;
            let stream_service_settings = input_registry.stream_service_settings().clone();
            if stream_service_settings.stream_service_type != "rtmp_custom" {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        request_type,
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Unsupported streamServiceType field",
                    ),
                    None,
                );
            }
            // hisui は配信先 URL (server) の事前設定を必須とする設計であり、
            // OBS のように GUI で事前設定済みの状態を前提としない。
            let Some(output_url) = stream_service_settings.server else {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        request_type,
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Missing streamServiceSettings.server field",
                    ),
                    None,
                );
            };

            let run_id = match input_registry.next_stream_run_id() {
                Ok(run_id) => run_id,
                Err(_) => {
                    return RequestOutcome::failure(
                        crate::obsws_response_builder::build_request_response_error(
                            request_type,
                            request_id,
                            REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                            "Stream run ID overflow",
                        ),
                        None,
                    );
                }
            };
            let output_plan = match Self::build_output_plan_or_error(
                request_type,
                request_id,
                &input_registry,
                crate::obsws::source::ObswsOutputKind::Stream,
                run_id,
            ) {
                Ok(output_plan) => output_plan,
                Err(outcome) => return outcome,
            };
            let video =
                ObswsRecordTrackRun::new("stream", run_id, "video", &output_plan.video_track_id);
            let audio =
                ObswsRecordTrackRun::new("stream", run_id, "audio", &output_plan.audio_track_id);
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
                        request_type,
                        request_id,
                        REQUEST_STATUS_STREAM_RUNNING,
                        "Stream is already active",
                    ),
                    None,
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
                Self::build_internal_error_response(request_type, request_id, &error_comment),
                None,
            );
        }

        RequestOutcome::success(
            crate::obsws_response_builder::build_start_stream_response(request_id),
            None,
        )
    }

    pub(super) async fn handle_stop_stream(
        &self,
        request_type: &str,
        request_id: &str,
    ) -> RequestOutcome {
        let run = {
            let input_registry = self.input_registry.read().await;
            if !input_registry.is_stream_active() {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        request_type,
                        request_id,
                        REQUEST_STATUS_STREAM_NOT_RUNNING,
                        "Stream is not active",
                    ),
                    None,
                );
            }
            input_registry
                .stream_run()
                .expect("infallible: active stream must have run state")
        };
        if let Err(e) = self.stop_stream_processors(&run).await {
            let error_comment = format!("Failed to stop stream: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response(request_type, request_id, &error_comment),
                None,
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

    pub(super) async fn handle_start_record(
        &self,
        request_type: &str,
        request_id: &str,
    ) -> RequestOutcome {
        let (output_plan, output_path, run) = {
            let mut input_registry = self.input_registry.write().await;
            let run_id = match input_registry.next_record_run_id() {
                Ok(run_id) => run_id,
                Err(_) => {
                    return RequestOutcome::failure(
                        crate::obsws_response_builder::build_request_response_error(
                            request_type,
                            request_id,
                            REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                            "Record run ID overflow",
                        ),
                        None,
                    );
                }
            };
            let output_plan = match Self::build_output_plan_or_error(
                request_type,
                request_id,
                &input_registry,
                crate::obsws::source::ObswsOutputKind::Record,
                run_id,
            ) {
                Ok(output_plan) => output_plan,
                Err(outcome) => return outcome,
            };
            let writer_processor_id =
                crate::ProcessorId::new(format!("obsws:record:{run_id}:mp4_writer"));
            let video =
                ObswsRecordTrackRun::new("record", run_id, "video", &output_plan.video_track_id);
            let audio =
                ObswsRecordTrackRun::new("record", run_id, "audio", &output_plan.audio_track_id);
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
                        request_type,
                        request_id,
                        REQUEST_STATUS_OUTPUT_RUNNING,
                        "Record is already active",
                    ),
                    None,
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
                Self::build_internal_error_response(request_type, request_id, &error_comment),
                None,
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
                Self::build_internal_error_response(request_type, request_id, &error_comment),
                None,
            );
        }

        RequestOutcome::success(
            crate::obsws_response_builder::build_start_record_response(request_id),
            None,
        )
    }

    pub(super) async fn handle_stop_record(
        &self,
        request_type: &str,
        request_id: &str,
    ) -> RequestOutcome {
        let run = {
            let input_registry = self.input_registry.read().await;
            if !input_registry.is_record_active() {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        request_type,
                        request_id,
                        REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "Record is not active",
                    ),
                    None,
                );
            }
            input_registry
                .record_run()
                .expect("infallible: active record must have run state")
        };
        if let Err(e) = self.stop_record_processors(&run).await {
            let error_comment = format!("Failed to stop record: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response(request_type, request_id, &error_comment),
                None,
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
                    None,
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
                    None,
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
                None,
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
                None,
            ),
            Err(PauseRecordError::AlreadyPaused) => RequestOutcome::failure(
                crate::obsws_response_builder::build_request_response_error(
                    "PauseRecord",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Record is already paused",
                ),
                None,
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
                    None,
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
                    None,
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
                None,
            );
        }
        if let Err(e) = self.request_record_resume_keyframe(&run).await {
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
                        return RequestOutcome::failure(
                            Self::build_internal_error_response(
                                "ResumeRecord",
                                request_id,
                                &error_comment,
                            ),
                            Some(output_path),
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
                            None,
                        );
                    }
                }
            }
            let error_comment =
                format!("Failed to request record resume keyframe: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response("ResumeRecord", request_id, &error_comment),
                None,
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
                None,
            ),
            Err(ResumeRecordError::NotPaused) => RequestOutcome::failure(
                crate::obsws_response_builder::build_request_response_error(
                    "ResumeRecord",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Record is not paused",
                ),
                None,
            ),
        }
    }

    // --- パイプライン操作メソッド ---

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
                    f.member("frameRate", output_plan.frame_rate)?;
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
        {
            let audio = &run.audio;
            let audio_mixer_processor_id = &run.audio_mixer_processor_id;
            self.send_create_audio_mixer_request(
                &output_plan.source_plans,
                audio,
                audio_mixer_processor_id,
            )
            .await?;
        }

        {
            let video = &run.video;
            let video_mixer_processor_id = &run.video_mixer_processor_id;
            self.send_create_video_mixer_request(output_plan, video, video_mixer_processor_id)
                .await?;
        }

        {
            let video = &run.video;
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
                        f.member("frameRate", output_plan.frame_rate)?;
                        f.member("processorId", &video.encoder_processor_id)
                    }),
                )
            })
            .to_string();
            self.send_pipeline_rpc_request("createVideoEncoder", &video_encoder_request)
                .await?;
        }

        {
            let audio = &run.audio;
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
                    f.member("inputAudioTrackId", &run.audio.encoded_track_id)?;
                    f.member("inputVideoTrackId", &run.video.encoded_track_id)?;
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
        {
            let audio = &run.audio;
            let audio_mixer_processor_id = &run.audio_mixer_processor_id;
            self.send_create_audio_mixer_request(
                &output_plan.source_plans,
                audio,
                audio_mixer_processor_id,
            )
            .await?;
        }

        {
            let video = &run.video;
            let video_mixer_processor_id = &run.video_mixer_processor_id;
            self.send_create_video_mixer_request(output_plan, video, video_mixer_processor_id)
                .await?;
        }

        {
            let video = &run.video;
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
                        f.member("frameRate", output_plan.frame_rate)?;
                        f.member("processorId", &video.encoder_processor_id)
                    }),
                )
            })
            .to_string();
            self.send_pipeline_rpc_request("createVideoEncoder", &video_encoder_request)
                .await?;
        }

        {
            let audio = &run.audio;
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
                    f.member("inputAudioTrackId", &run.audio.encoded_track_id)?;
                    f.member("inputVideoTrackId", &run.video.encoded_track_id)?;
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

        let video = &run.video;
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
            let mixer_ids = vec![
                run.audio_mixer_processor_id.clone(),
                run.video_mixer_processor_id.clone(),
            ];
            self.stop_processors(&mixer_ids).await?;
        }

        // 3. エンコーダーを停止
        {
            let ids = vec![
                run.video.encoder_processor_id.clone(),
                run.audio.encoder_processor_id.clone(),
            ];
            self.stop_processors(&ids).await?;
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
            let mixer_ids = vec![
                run.audio_mixer_processor_id.clone(),
                run.video_mixer_processor_id.clone(),
            ];
            self.stop_processors(&mixer_ids).await?;
        }

        // 3. エンコーダーを停止（EOS をライターに伝播）
        {
            let ids = vec![
                run.video.encoder_processor_id.clone(),
                run.audio.encoder_processor_id.clone(),
            ];
            self.stop_processors(&ids).await?;
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
