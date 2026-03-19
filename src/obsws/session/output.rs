use super::*;

impl ObswsSession {
    const RECORD_STOP_SOURCE_GRACE_TIMEOUT: Duration = Duration::from_secs(1);
    const RECORD_STOP_FINISH_RPC_RETRY_TIMEOUT: Duration = Duration::from_millis(500);
    const RECORD_STOP_FINISH_RPC_RETRY_INTERVAL: Duration = Duration::from_millis(10);

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
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STARTING",
                    None,
                ),
            );
            events.push(
                crate::obsws_response_builder::build_record_state_changed_event(
                    true,
                    "OBS_WEBSOCKET_OUTPUT_STARTED",
                    outcome.output_path.as_deref(),
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
                    "OBS_WEBSOCKET_OUTPUT_STOPPING",
                    None,
                ),
            );
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
            if was_active {
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPING",
                        None,
                    ),
                );
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPED",
                        outcome.output_path.as_deref(),
                    ),
                );
            } else {
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STARTING",
                        None,
                    ),
                );
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        true,
                        "OBS_WEBSOCKET_OUTPUT_STARTED",
                        outcome.output_path.as_deref(),
                    ),
                );
            }
        }
        let response_text = Self::build_toggle_response_from_outcome(
            "ToggleRecord",
            request_id,
            !was_active,
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
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STARTING",
                            None,
                        ),
                    );
                    events.push(
                        crate::obsws_response_builder::build_record_state_changed_event(
                            true,
                            "OBS_WEBSOCKET_OUTPUT_STARTED",
                            outcome.output_path.as_deref(),
                        ),
                    );
                }
                (outcome, events)
            }
            "rtmp_outbound" => {
                let outcome = self
                    .handle_start_rtmp_outbound("StartOutput", request_id)
                    .await;
                // rtmp_outbound はイベント通知を行わない（hisui 独自拡張のため）
                (outcome, Vec::new())
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
                            "OBS_WEBSOCKET_OUTPUT_STOPPING",
                            None,
                        ),
                    );
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
            "rtmp_outbound" => {
                let outcome = self
                    .handle_stop_rtmp_outbound("StopOutput", request_id)
                    .await;
                (outcome, Vec::new())
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
                    if was_active {
                        events.push(
                            crate::obsws_response_builder::build_record_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPING",
                                None,
                            ),
                        );
                        events.push(
                            crate::obsws_response_builder::build_record_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPED",
                                outcome.output_path.as_deref(),
                            ),
                        );
                    } else {
                        events.push(
                            crate::obsws_response_builder::build_record_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STARTING",
                                None,
                            ),
                        );
                        events.push(
                            crate::obsws_response_builder::build_record_state_changed_event(
                                true,
                                "OBS_WEBSOCKET_OUTPUT_STARTED",
                                outcome.output_path.as_deref(),
                            ),
                        );
                    }
                }
                (outcome, !was_active, events)
            }
            "rtmp_outbound" => {
                let was_active = self.input_registry.read().await.is_rtmp_outbound_active();
                let outcome = if was_active {
                    self.handle_stop_rtmp_outbound("ToggleOutput", request_id)
                        .await
                } else {
                    self.handle_start_rtmp_outbound("ToggleOutput", request_id)
                        .await
                };
                (outcome, !was_active, Vec::new())
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
        let (output_url, stream_name, mut output_plan, run) = {
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
            .start_stream_processors(&mut output_plan, &output_url, stream_name.as_deref(), &run)
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

    pub(super) async fn handle_start_rtmp_outbound(
        &self,
        request_type: &str,
        request_id: &str,
    ) -> RequestOutcome {
        let (output_url, stream_name, mut output_plan, run) = {
            let mut input_registry = self.input_registry.write().await;
            let rtmp_outbound_settings = input_registry.rtmp_outbound_settings().clone();
            let Some(output_url) = rtmp_outbound_settings.output_url else {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        request_type,
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Missing outputSettings.outputUrl field",
                    ),
                    None,
                );
            };

            let run_id = match input_registry.next_rtmp_outbound_run_id() {
                Ok(run_id) => run_id,
                Err(_) => {
                    return RequestOutcome::failure(
                        crate::obsws_response_builder::build_request_response_error(
                            request_type,
                            request_id,
                            REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                            "RTMP outbound run ID overflow",
                        ),
                        None,
                    );
                }
            };
            let output_plan = match Self::build_output_plan_or_error(
                request_type,
                request_id,
                &input_registry,
                crate::obsws::source::ObswsOutputKind::RtmpOutbound,
                run_id,
            ) {
                Ok(output_plan) => output_plan,
                Err(outcome) => return outcome,
            };
            let video = ObswsRecordTrackRun::new(
                "rtmp_outbound",
                run_id,
                "video",
                &output_plan.video_track_id,
            );
            let audio = ObswsRecordTrackRun::new(
                "rtmp_outbound",
                run_id,
                "audio",
                &output_plan.audio_track_id,
            );
            let run = ObswsRtmpOutboundRun {
                source_processor_ids: output_plan.source_processor_ids.clone(),
                video,
                audio,
                audio_mixer_processor_id: output_plan.audio_mixer_processor_id.clone(),
                video_mixer_processor_id: output_plan.video_mixer_processor_id.clone(),
                endpoint_processor_id: crate::ProcessorId::new(format!(
                    "obsws:rtmp_outbound:{run_id}:rtmp_outbound_endpoint"
                )),
            };
            if let Err(ActivateRtmpOutboundError::AlreadyActive) =
                input_registry.activate_rtmp_outbound(run.clone())
            {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        request_type,
                        request_id,
                        REQUEST_STATUS_OUTPUT_RUNNING,
                        "RTMP outbound is already active",
                    ),
                    None,
                );
            }

            (
                output_url,
                rtmp_outbound_settings.stream_name,
                output_plan,
                run,
            )
        };

        let start_result = self
            .start_rtmp_outbound_processors(
                &mut output_plan,
                &output_url,
                stream_name.as_deref(),
                &run,
            )
            .await;

        if let Err(e) = start_result {
            let _ = self.input_registry.write().await.deactivate_rtmp_outbound();
            if let Err(cleanup_error) = self.stop_rtmp_outbound_processors(&run).await {
                tracing::warn!(
                    "failed to cleanup rtmp_outbound processors after start failure: {}",
                    cleanup_error.display()
                );
            }
            let error_comment = format!("Failed to start rtmp_outbound: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response(request_type, request_id, &error_comment),
                None,
            );
        }

        RequestOutcome::success(
            crate::obsws_response_builder::build_start_output_response(request_id),
            None,
        )
    }

    pub(super) async fn handle_stop_rtmp_outbound(
        &self,
        request_type: &str,
        request_id: &str,
    ) -> RequestOutcome {
        let run = {
            let input_registry = self.input_registry.read().await;
            if !input_registry.is_rtmp_outbound_active() {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        request_type,
                        request_id,
                        REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "RTMP outbound is not active",
                    ),
                    None,
                );
            }
            input_registry
                .rtmp_outbound_run()
                .expect("infallible: active rtmp_outbound must have run state")
        };
        if let Err(e) = self.stop_rtmp_outbound_processors(&run).await {
            let error_comment = format!("Failed to stop rtmp_outbound: {}", e.display());
            return RequestOutcome::failure(
                Self::build_internal_error_response(request_type, request_id, &error_comment),
                None,
            );
        }
        let mut input_registry = self.input_registry.write().await;
        if input_registry.deactivate_rtmp_outbound().is_none() {
            tracing::warn!(
                "rtmp_outbound runtime was already deactivated while stopping rtmp_outbound"
            );
        }
        RequestOutcome::success(
            crate::obsws_response_builder::build_stop_output_response(request_id),
            None,
        )
    }

    pub(super) async fn handle_start_record(
        &self,
        request_type: &str,
        request_id: &str,
    ) -> RequestOutcome {
        let (mut output_plan, output_path, run) = {
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
            .start_record_processors(&mut output_plan, &output_path, &run)
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

        let output_path_str = output_path.display().to_string();
        RequestOutcome::success(
            crate::obsws_response_builder::build_start_record_response(request_id),
            Some(output_path_str),
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

    // --- パイプライン操作メソッド ---

    fn get_pipeline_handle(&self) -> crate::Result<&crate::MediaPipelineHandle> {
        self.pipeline_handle
            .as_ref()
            .ok_or_else(|| crate::Error::new("BUG: obsws pipeline handle is not initialized"))
    }

    /// createVideoMixer を型付きメソッドで呼び出す
    async fn send_create_video_mixer_request(
        &self,
        output_plan: &crate::obsws::output_plan::ObswsComposedOutputPlan,
        video: &ObswsRecordTrackRun,
        video_mixer_processor_id: &crate::ProcessorId,
    ) -> crate::Result<()> {
        let pipeline_handle = self.get_pipeline_handle()?;
        let input_tracks = output_plan
            .video_mixer_input_tracks
            .iter()
            .map(|t| crate::mixer_realtime_video::InputTrack {
                track_id: t.track_id.clone(),
                x: t.x as isize,
                y: t.y as isize,
                z: t.z as isize,
                width: t
                    .width
                    .and_then(|w| crate::types::EvenUsize::new(w as usize)),
                height: t
                    .height
                    .and_then(|h| crate::types::EvenUsize::new(h as usize)),
                scale_x: t.scale_x,
                scale_y: t.scale_y,
                crop_top: t.crop_top as usize,
                crop_bottom: t.crop_bottom as usize,
                crop_left: t.crop_left as usize,
                crop_right: t.crop_right as usize,
            })
            .collect();
        let mixer = crate::mixer_realtime_video::VideoRealtimeMixer {
            canvas_width: output_plan.canvas_width,
            canvas_height: output_plan.canvas_height,
            frame_rate: output_plan.frame_rate,
            input_tracks,
            output_track_id: video.source_track_id.clone(),
        };
        pipeline_handle
            .create_video_mixer(mixer, Some(video_mixer_processor_id.clone()))
            .await?;
        Ok(())
    }

    /// createAudioMixer を型付きメソッドで呼び出す
    async fn send_create_audio_mixer_request(
        &self,
        source_plans: &[crate::obsws::source::ObswsRecordSourcePlan],
        audio: &ObswsRecordTrackRun,
        audio_mixer_processor_id: &crate::ProcessorId,
    ) -> crate::Result<()> {
        let pipeline_handle = self.get_pipeline_handle()?;
        let input_tracks = source_plans
            .iter()
            .filter_map(|sp| {
                sp.source_audio_track_id.as_ref().map(|track_id| {
                    crate::mixer_realtime_audio::AudioRealtimeInputTrack {
                        track_id: track_id.clone(),
                    }
                })
            })
            .collect();
        let mixer = crate::mixer_realtime_audio::AudioRealtimeMixer {
            sample_rate: crate::audio::SampleRate::HZ_48000,
            channels: crate::audio::Channels::STEREO,
            frame_duration: std::time::Duration::from_millis(20),
            timestamp_rebase_threshold: std::time::Duration::from_millis(100),
            terminate_on_input_eos: true,
            input_tracks,
            output_track_id: audio.source_track_id.clone(),
        };
        pipeline_handle
            .create_audio_mixer(mixer, Some(audio_mixer_processor_id.clone()))
            .await?;
        Ok(())
    }

    pub(super) async fn start_stream_processors(
        &self,
        output_plan: &mut crate::obsws::output_plan::ObswsComposedOutputPlan,
        output_url: &str,
        stream_name: Option<&str>,
        run: &ObswsStreamRun,
    ) -> crate::Result<()> {
        let pipeline_handle = self.get_pipeline_handle()?;

        self.send_create_audio_mixer_request(
            &output_plan.source_plans,
            &run.audio,
            &run.audio_mixer_processor_id,
        )
        .await?;

        self.send_create_video_mixer_request(
            output_plan,
            &run.video,
            &run.video_mixer_processor_id,
        )
        .await?;

        pipeline_handle
            .create_video_encoder(
                run.video.source_track_id.clone(),
                run.video.encoded_track_id.clone(),
                crate::types::CodecName::H264,
                std::num::NonZeroUsize::new(2_000_000).unwrap(),
                output_plan.frame_rate,
                Some(run.video.encoder_processor_id.clone()),
            )
            .await?;

        pipeline_handle
            .create_audio_encoder(
                run.audio.source_track_id.clone(),
                run.audio.encoded_track_id.clone(),
                crate::types::CodecName::Aac,
                std::num::NonZeroUsize::new(128_000).unwrap(),
                Some(run.audio.encoder_processor_id.clone()),
            )
            .await?;

        let publisher = crate::publisher_rtmp::RtmpPublisher {
            output_url: output_url.to_owned(),
            stream_name: stream_name.map(|s| s.to_owned()),
            input_audio_track_id: Some(run.audio.encoded_track_id.clone()),
            input_video_track_id: Some(run.video.encoded_track_id.clone()),
            options: Default::default(),
        };
        pipeline_handle
            .create_rtmp_publisher(publisher, Some(run.publisher_processor_id.clone()))
            .await?;

        for source_plan in &mut output_plan.source_plans {
            for request in source_plan.requests.drain(..) {
                request.execute(pipeline_handle).await?;
            }
        }

        Ok(())
    }

    pub(super) async fn start_record_processors(
        &self,
        output_plan: &mut crate::obsws::output_plan::ObswsComposedOutputPlan,
        output_path: &std::path::Path,
        run: &ObswsRecordRun,
    ) -> crate::Result<()> {
        let pipeline_handle = self.get_pipeline_handle()?;

        self.send_create_audio_mixer_request(
            &output_plan.source_plans,
            &run.audio,
            &run.audio_mixer_processor_id,
        )
        .await?;

        self.send_create_video_mixer_request(
            output_plan,
            &run.video,
            &run.video_mixer_processor_id,
        )
        .await?;

        pipeline_handle
            .create_video_encoder(
                run.video.source_track_id.clone(),
                run.video.encoded_track_id.clone(),
                crate::types::CodecName::H264,
                std::num::NonZeroUsize::new(2_000_000).unwrap(),
                output_plan.frame_rate,
                Some(run.video.encoder_processor_id.clone()),
            )
            .await?;

        pipeline_handle
            .create_audio_encoder(
                run.audio.source_track_id.clone(),
                run.audio.encoded_track_id.clone(),
                crate::types::CodecName::Opus,
                std::num::NonZeroUsize::new(128_000).unwrap(),
                Some(run.audio.encoder_processor_id.clone()),
            )
            .await?;

        pipeline_handle
            .create_mp4_writer(
                output_path.to_path_buf(),
                Some(run.audio.encoded_track_id.clone()),
                Some(run.video.encoded_track_id.clone()),
                Some(run.writer_processor_id.clone()),
            )
            .await?;

        for source_plan in &mut output_plan.source_plans {
            for request in source_plan.requests.drain(..) {
                request.execute(pipeline_handle).await?;
            }
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

    pub(super) async fn start_rtmp_outbound_processors(
        &self,
        output_plan: &mut crate::obsws::output_plan::ObswsComposedOutputPlan,
        output_url: &str,
        stream_name: Option<&str>,
        run: &ObswsRtmpOutboundRun,
    ) -> crate::Result<()> {
        let pipeline_handle = self.get_pipeline_handle()?;

        self.send_create_audio_mixer_request(
            &output_plan.source_plans,
            &run.audio,
            &run.audio_mixer_processor_id,
        )
        .await?;

        self.send_create_video_mixer_request(
            output_plan,
            &run.video,
            &run.video_mixer_processor_id,
        )
        .await?;

        pipeline_handle
            .create_video_encoder(
                run.video.source_track_id.clone(),
                run.video.encoded_track_id.clone(),
                crate::types::CodecName::H264,
                std::num::NonZeroUsize::new(2_000_000).unwrap(),
                output_plan.frame_rate,
                Some(run.video.encoder_processor_id.clone()),
            )
            .await?;

        // RTMP outbound は AAC エンコーディングを使用する（RTMP の制約）
        pipeline_handle
            .create_audio_encoder(
                run.audio.source_track_id.clone(),
                run.audio.encoded_track_id.clone(),
                crate::types::CodecName::Aac,
                std::num::NonZeroUsize::new(128_000).unwrap(),
                Some(run.audio.encoder_processor_id.clone()),
            )
            .await?;

        let endpoint = crate::outbound_endpoint_rtmp::RtmpOutboundEndpoint {
            output_url: output_url.to_owned(),
            stream_name: stream_name.map(|s| s.to_owned()),
            input_audio_track_id: Some(run.audio.encoded_track_id.clone()),
            input_video_track_id: Some(run.video.encoded_track_id.clone()),
            options: Default::default(),
        };
        pipeline_handle
            .create_rtmp_outbound_endpoint(endpoint, Some(run.endpoint_processor_id.clone()))
            .await?;

        for source_plan in &mut output_plan.source_plans {
            for request in source_plan.requests.drain(..) {
                request.execute(pipeline_handle).await?;
            }
        }

        Ok(())
    }

    /// ソース → ミキサー → エンコーダー → エンドポイントの順に段階的に停止する。
    pub(super) async fn stop_rtmp_outbound_processors(
        &self,
        run: &ObswsRtmpOutboundRun,
    ) -> crate::Result<()> {
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

        // 4. エンドポイントを停止
        self.stop_processors(std::slice::from_ref(&run.endpoint_processor_id))
            .await?;

        Ok(())
    }

    /// ソース → ミキサー → エンコーダー → ライターの順に段階的に停止する。
    /// mixer に Finish RPC を送信して EOS を発行させ、下流は EOS 伝播で自然終了させる。
    pub(super) async fn stop_record_processors(&self, run: &ObswsRecordRun) -> crate::Result<()> {
        let pipeline_handle = self.get_pipeline_handle()?;

        // 1. ソースを停止して新規入力を止める（入力 0 件なら何もしない）
        self.stop_processors(&run.source_processor_ids).await?;

        // 2. source 停止だけで自然終了するケースを短時間だけ待つ。
        let mixer_ids = vec![
            run.audio_mixer_processor_id.clone(),
            run.video_mixer_processor_id.clone(),
        ];
        if self
            .wait_processors_stopped(
                pipeline_handle,
                &mixer_ids,
                Self::RECORD_STOP_SOURCE_GRACE_TIMEOUT,
            )
            .await
            .is_err()
        {
            self.finish_or_stop_record_mixers(pipeline_handle, run)
                .await?;
        }

        // 3. エンコーダーは EOS 伝播での自然終了を優先し、残れば停止する。
        {
            let ids = vec![
                run.video.encoder_processor_id.clone(),
                run.audio.encoder_processor_id.clone(),
            ];
            self.wait_or_stop_processors(&ids, Duration::from_secs(5))
                .await?;
        }

        // 4. ライターは finalize を優先し、残れば最後に停止する。
        self.wait_or_stop_processors(
            std::slice::from_ref(&run.writer_processor_id),
            Duration::from_secs(5),
        )
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

        self.wait_processors_stopped_with_default_timeout(processor_ids)
            .await?;

        if let Some(e) = terminate_error {
            return Err(e);
        }

        Ok(())
    }

    pub(super) async fn wait_processors_stopped_with_default_timeout(
        &self,
        processor_ids: &[crate::ProcessorId],
    ) -> crate::Result<()> {
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            return Err(crate::Error::new(
                "BUG: obsws pipeline handle is not initialized",
            ));
        };

        self.wait_processors_stopped(pipeline_handle, processor_ids, Duration::from_secs(5))
            .await
    }

    pub(super) async fn wait_or_stop_processors(
        &self,
        processor_ids: &[crate::ProcessorId],
        timeout: Duration,
    ) -> crate::Result<()> {
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            return Err(crate::Error::new(
                "BUG: obsws pipeline handle is not initialized",
            ));
        };

        if self
            .wait_processors_stopped(pipeline_handle, processor_ids, timeout)
            .await
            .is_ok()
        {
            return Ok(());
        }

        let pending = self
            .live_processor_ids(pipeline_handle, processor_ids)
            .await?;
        if pending.is_empty() {
            return Ok(());
        }
        self.stop_processors(&pending).await
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

    async fn finish_or_stop_record_mixers(
        &self,
        pipeline_handle: &crate::MediaPipelineHandle,
        run: &ObswsRecordRun,
    ) -> crate::Result<()> {
        let mut terminate_ids = Vec::new();

        if let Err(e) = self
            .finish_audio_mixer_with_retry(pipeline_handle, run.audio_mixer_processor_id.clone())
            .await
        {
            tracing::debug!("audio mixer finish fallback to terminate: {}", e.display());
            terminate_ids.push(run.audio_mixer_processor_id.clone());
        }
        if let Err(e) = self
            .finish_video_mixer_with_retry(pipeline_handle, run.video_mixer_processor_id.clone())
            .await
        {
            tracing::debug!("video mixer finish fallback to terminate: {}", e.display());
            terminate_ids.push(run.video_mixer_processor_id.clone());
        }

        if !terminate_ids.is_empty() {
            self.stop_processors(&terminate_ids).await?;
        }

        let mixer_ids = vec![
            run.audio_mixer_processor_id.clone(),
            run.video_mixer_processor_id.clone(),
        ];
        if let Err(e) = self
            .wait_processors_stopped_with_default_timeout(&mixer_ids)
            .await
        {
            let pending = self
                .live_processor_ids(pipeline_handle, &mixer_ids)
                .await
                .map_err(|list_error| {
                    crate::Error::new(format!(
                        "failed to finish record mixers: {}; additionally failed to inspect live mixers: {}",
                        e.display(),
                        list_error.display()
                    ))
                })?;
            if pending.is_empty() {
                return Ok(());
            }
            self.stop_processors(&pending).await?;
        }

        Ok(())
    }

    async fn finish_audio_mixer_with_retry(
        &self,
        pipeline_handle: &crate::MediaPipelineHandle,
        processor_id: crate::ProcessorId,
    ) -> crate::Result<()> {
        let deadline = tokio::time::Instant::now() + Self::RECORD_STOP_FINISH_RPC_RETRY_TIMEOUT;
        loop {
            let sender = match pipeline_handle
                .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                    crate::mixer_realtime_audio::AudioRealtimeMixerRpcMessage,
                >>(&processor_id)
                .await
            {
                Ok(sender) => sender,
                Err(crate::media_pipeline::GetProcessorRpcSenderError::ProcessorNotFound) => {
                    return Ok(());
                }
                Err(crate::media_pipeline::GetProcessorRpcSenderError::SenderNotRegistered) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(crate::Error::new(format!(
                            "audio mixer RPC sender is not registered: {processor_id}"
                        )));
                    }
                    tokio::time::sleep(Self::RECORD_STOP_FINISH_RPC_RETRY_INTERVAL).await;
                    continue;
                }
                Err(e) => {
                    return Err(crate::Error::new(format!(
                        "failed to get audio mixer RPC sender ({processor_id}): {e}"
                    )));
                }
            };

            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            sender
                .send(
                    crate::mixer_realtime_audio::AudioRealtimeMixerRpcMessage::Finish { reply_tx },
                )
                .map_err(|_| {
                    crate::Error::new(format!(
                        "failed to send finish RPC to audio mixer: {processor_id}"
                    ))
                })?;
            reply_rx.await.map_err(|_| {
                crate::Error::new(format!(
                    "failed to receive finish RPC response from audio mixer: {processor_id}"
                ))
            })?;
            return Ok(());
        }
    }

    async fn finish_video_mixer_with_retry(
        &self,
        pipeline_handle: &crate::MediaPipelineHandle,
        processor_id: crate::ProcessorId,
    ) -> crate::Result<()> {
        let deadline = tokio::time::Instant::now() + Self::RECORD_STOP_FINISH_RPC_RETRY_TIMEOUT;
        loop {
            let sender = match pipeline_handle
                .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                    crate::mixer_realtime_video::VideoRealtimeMixerRpcMessage,
                >>(&processor_id)
                .await
            {
                Ok(sender) => sender,
                Err(crate::media_pipeline::GetProcessorRpcSenderError::ProcessorNotFound) => {
                    return Ok(());
                }
                Err(crate::media_pipeline::GetProcessorRpcSenderError::SenderNotRegistered) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(crate::Error::new(format!(
                            "video mixer RPC sender is not registered: {processor_id}"
                        )));
                    }
                    tokio::time::sleep(Self::RECORD_STOP_FINISH_RPC_RETRY_INTERVAL).await;
                    continue;
                }
                Err(e) => {
                    return Err(crate::Error::new(format!(
                        "failed to get video mixer RPC sender ({processor_id}): {e}"
                    )));
                }
            };

            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            sender
                .send(
                    crate::mixer_realtime_video::VideoRealtimeMixerRpcMessage::Finish { reply_tx },
                )
                .map_err(|_| {
                    crate::Error::new(format!(
                        "failed to send finish RPC to video mixer: {processor_id}"
                    ))
                })?;
            reply_rx.await.map_err(|_| {
                crate::Error::new(format!(
                    "failed to receive finish RPC response from video mixer: {processor_id}"
                ))
            })?;
            return Ok(());
        }
    }

    async fn live_processor_ids(
        &self,
        pipeline_handle: &crate::MediaPipelineHandle,
        processor_ids: &[crate::ProcessorId],
    ) -> crate::Result<Vec<crate::ProcessorId>> {
        let live_processors = pipeline_handle
            .list_processors()
            .await
            .map_err(|_| crate::Error::new("failed to list processors: pipeline has terminated"))?;
        Ok(processor_ids
            .iter()
            .filter(|processor_id| live_processors.iter().any(|id| id == *processor_id))
            .cloned()
            .collect())
    }
}
