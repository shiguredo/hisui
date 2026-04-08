//! obsws のリクエストと個別 output エンジンの接着層。
//! Stream/Record/Output の開始・停止・トグルリクエストを受け取り、各エンジンモジュールに委譲してイベントを組み立てる。
//! 共通の processor 停止ユーティリティと S3 クライアント生成もここに置く。

use std::time::Duration;

use super::{CommandResult, ObswsCoordinator, parse_required_non_empty_string_field};
use crate::obsws::event::TaggedEvent;
use crate::obsws::protocol::{
    OBSWS_EVENT_SUB_OUTPUTS, REQUEST_STATUS_MISSING_REQUEST_FIELD,
    REQUEST_STATUS_RESOURCE_NOT_FOUND,
};

/// output 操作の結果（成功/失敗 + レスポンス + 出力パス）
pub(crate) struct OutputOperationOutcome {
    pub(crate) response_text: nojson::RawJsonOwned,
    pub(crate) success: bool,
    pub(crate) output_path: Option<String>,
}

impl OutputOperationOutcome {
    pub(crate) fn success(
        response_text: nojson::RawJsonOwned,
        output_path: Option<String>,
    ) -> Self {
        Self {
            response_text,
            success: true,
            output_path,
        }
    }

    pub(crate) fn failure(response_text: nojson::RawJsonOwned) -> Self {
        Self {
            response_text,
            success: false,
            output_path: None,
        }
    }
}

impl ObswsCoordinator {
    pub(crate) async fn handle_start_stream_request(&mut self, request_id: &str) -> CommandResult {
        let outcome = self
            .handle_start_stream("StartStream", request_id, "stream")
            .await;
        let mut events = Vec::new();
        if outcome.success {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_stream_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STARTING",
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
            events.push(TaggedEvent {
                text: crate::obsws::response::build_stream_state_changed_event(
                    true,
                    "OBS_WEBSOCKET_OUTPUT_STARTED",
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
        }
        self.build_result_from_response(outcome.response_text, events)
    }

    pub(crate) async fn handle_stop_stream_request(&mut self, request_id: &str) -> CommandResult {
        let outcome = self
            .handle_stop_stream("StopStream", request_id, "stream")
            .await;
        let mut events = Vec::new();
        if outcome.success {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_stream_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPING",
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
            events.push(TaggedEvent {
                text: crate::obsws::response::build_stream_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPED",
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
        }
        self.build_result_from_response(outcome.response_text, events)
    }

    pub(crate) async fn handle_toggle_stream_request(&mut self, request_id: &str) -> CommandResult {
        let was_active = self.outputs.get("stream").is_some_and(|o| o.runtime.active);
        let outcome = if was_active {
            self.handle_stop_stream("ToggleStream", request_id, "stream")
                .await
        } else {
            self.handle_start_stream("ToggleStream", request_id, "stream")
                .await
        };
        let mut events = Vec::new();
        if outcome.success {
            if was_active {
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_stream_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPING",
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_stream_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPED",
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
            } else {
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_stream_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STARTING",
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_stream_state_changed_event(
                        true,
                        "OBS_WEBSOCKET_OUTPUT_STARTED",
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
            }
        }
        let response_text = if outcome.success {
            crate::obsws::response::build_toggle_stream_response(request_id, !was_active)
        } else {
            outcome.response_text
        };
        self.build_result_from_response(response_text, events)
    }

    pub(crate) async fn handle_start_record_request(&mut self, request_id: &str) -> CommandResult {
        let outcome = self
            .handle_start_record("StartRecord", request_id, "record")
            .await;
        let mut events = Vec::new();
        if outcome.success {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_record_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STARTING",
                    None,
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
            events.push(TaggedEvent {
                text: crate::obsws::response::build_record_state_changed_event(
                    true,
                    "OBS_WEBSOCKET_OUTPUT_STARTED",
                    outcome.output_path.as_deref(),
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
        }
        self.build_result_from_response(outcome.response_text, events)
    }

    pub(crate) async fn handle_stop_record_request(&mut self, request_id: &str) -> CommandResult {
        let outcome = self
            .handle_stop_record("StopRecord", request_id, "record")
            .await;
        let mut events = Vec::new();
        if outcome.success {
            events.push(TaggedEvent {
                text: crate::obsws::response::build_record_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPING",
                    None,
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
            events.push(TaggedEvent {
                text: crate::obsws::response::build_record_state_changed_event(
                    false,
                    "OBS_WEBSOCKET_OUTPUT_STOPPED",
                    outcome.output_path.as_deref(),
                ),
                subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
            });
        }
        self.build_result_from_response(outcome.response_text, events)
    }

    pub(crate) async fn handle_toggle_record_request(&mut self, request_id: &str) -> CommandResult {
        let was_active = self.outputs.get("record").is_some_and(|o| o.runtime.active);
        let outcome = if was_active {
            self.handle_stop_record("ToggleRecord", request_id, "record")
                .await
        } else {
            self.handle_start_record("ToggleRecord", request_id, "record")
                .await
        };
        let mut events = Vec::new();
        if outcome.success {
            if was_active {
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPING",
                        None,
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STOPPED",
                        outcome.output_path.as_deref(),
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
            } else {
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_record_state_changed_event(
                        false,
                        "OBS_WEBSOCKET_OUTPUT_STARTING",
                        None,
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
                events.push(TaggedEvent {
                    text: crate::obsws::response::build_record_state_changed_event(
                        true,
                        "OBS_WEBSOCKET_OUTPUT_STARTED",
                        outcome.output_path.as_deref(),
                    ),
                    subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                });
            }
        }
        let response_text = if outcome.success {
            crate::obsws::response::build_toggle_record_response(request_id, !was_active)
        } else {
            outcome.response_text
        };
        self.build_result_from_response(response_text, events)
    }

    pub(crate) async fn handle_start_output_request(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(output_name) = parse_required_non_empty_string_field(request_data, "outputName")
        else {
            return self.build_error_result(
                "StartOutput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            );
        };
        let (outcome, events) = match output_name.as_str() {
            "stream" => {
                let outcome = self
                    .handle_start_stream("StartOutput", request_id, "stream")
                    .await;
                let mut events = Vec::new();
                if outcome.success {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_stream_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STARTING",
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_stream_state_changed_event(
                            true,
                            "OBS_WEBSOCKET_OUTPUT_STARTED",
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                }
                (outcome, events)
            }
            "record" => {
                let outcome = self
                    .handle_start_record("StartOutput", request_id, "record")
                    .await;
                let mut events = Vec::new();
                if outcome.success {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_record_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STARTING",
                            None,
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_record_state_changed_event(
                            true,
                            "OBS_WEBSOCKET_OUTPUT_STARTED",
                            outcome.output_path.as_deref(),
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                }
                (outcome, events)
            }
            "rtmp_outbound" => {
                let outcome = self
                    .handle_start_rtmp_outbound("StartOutput", request_id, "rtmp_outbound")
                    .await;
                (outcome, Vec::new())
            }
            "sora" => {
                let outcome = self
                    .handle_start_sora_publisher("StartOutput", request_id, "sora")
                    .await;
                (outcome, Vec::new())
            }
            "hls" => {
                let outcome = self
                    .handle_start_hls("StartOutput", request_id, "hls")
                    .await;
                (outcome, Vec::new())
            }
            "mpeg_dash" => {
                let outcome = self
                    .handle_start_mpeg_dash("StartOutput", request_id, "mpeg_dash")
                    .await;
                (outcome, Vec::new())
            }
            #[cfg(feature = "player")]
            "player" => {
                let outcome = self.handle_start_player("StartOutput", request_id).await;
                (outcome, Vec::new())
            }
            other => {
                // 動的に作成された output を kind に応じて起動する
                let outcome = self
                    .start_dynamic_output("StartOutput", request_id, other)
                    .await;
                (outcome, Vec::new())
            }
        };
        let response_text = if outcome.success {
            crate::obsws::response::build_start_output_response(request_id)
        } else {
            outcome.response_text
        };
        self.build_result_from_response(response_text, events)
    }

    pub(crate) async fn handle_stop_output_request(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(output_name) = parse_required_non_empty_string_field(request_data, "outputName")
        else {
            return self.build_error_result(
                "StopOutput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            );
        };
        let (outcome, events) = match output_name.as_str() {
            "stream" => {
                let outcome = self
                    .handle_stop_stream("StopOutput", request_id, "stream")
                    .await;
                let mut events = Vec::new();
                if outcome.success {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_stream_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPING",
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_stream_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPED",
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                }
                (outcome, events)
            }
            "record" => {
                let outcome = self
                    .handle_stop_record("StopOutput", request_id, "record")
                    .await;
                let mut events = Vec::new();
                if outcome.success {
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_record_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPING",
                            None,
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                    events.push(TaggedEvent {
                        text: crate::obsws::response::build_record_state_changed_event(
                            false,
                            "OBS_WEBSOCKET_OUTPUT_STOPPED",
                            outcome.output_path.as_deref(),
                        ),
                        subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                    });
                }
                (outcome, events)
            }
            "rtmp_outbound" => {
                let outcome = self
                    .handle_stop_rtmp_outbound("StopOutput", request_id, "rtmp_outbound")
                    .await;
                (outcome, Vec::new())
            }
            "sora" => {
                let outcome = self
                    .handle_stop_sora_publisher("StopOutput", request_id, "sora")
                    .await;
                (outcome, Vec::new())
            }
            "hls" => {
                let outcome = self.handle_stop_hls("StopOutput", request_id, "hls").await;
                (outcome, Vec::new())
            }
            "mpeg_dash" => {
                let outcome = self
                    .handle_stop_mpeg_dash("StopOutput", request_id, "mpeg_dash")
                    .await;
                (outcome, Vec::new())
            }
            #[cfg(feature = "player")]
            "player" => {
                let outcome = self.handle_stop_player("StopOutput", request_id).await;
                (outcome, Vec::new())
            }
            other => {
                let outcome = self
                    .stop_dynamic_output("StopOutput", request_id, other)
                    .await;
                (outcome, Vec::new())
            }
        };
        let response_text = if outcome.success {
            crate::obsws::response::build_stop_output_response(request_id)
        } else {
            outcome.response_text
        };
        self.build_result_from_response(response_text, events)
    }

    pub(crate) async fn handle_toggle_output_request(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(output_name) = parse_required_non_empty_string_field(request_data, "outputName")
        else {
            return self.build_error_result(
                "ToggleOutput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            );
        };
        let (outcome, output_active_on_success, events) = match output_name.as_str() {
            "stream" => {
                let was_active = self.outputs.get("stream").is_some_and(|o| o.runtime.active);
                let outcome = if was_active {
                    self.handle_stop_stream("ToggleOutput", request_id, "stream")
                        .await
                } else {
                    self.handle_start_stream("ToggleOutput", request_id, "stream")
                        .await
                };
                let mut events = Vec::new();
                if outcome.success {
                    if was_active {
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_stream_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPING",
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_stream_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPED",
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                    } else {
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_stream_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STARTING",
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_stream_state_changed_event(
                                true,
                                "OBS_WEBSOCKET_OUTPUT_STARTED",
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                    }
                }
                (outcome, !was_active, events)
            }
            "record" => {
                let was_active = self.outputs.get("record").is_some_and(|o| o.runtime.active);
                let outcome = if was_active {
                    self.handle_stop_record("ToggleOutput", request_id, "record")
                        .await
                } else {
                    self.handle_start_record("ToggleOutput", request_id, "record")
                        .await
                };
                let mut events = Vec::new();
                if outcome.success {
                    if was_active {
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_record_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPING",
                                None,
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_record_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STOPPED",
                                outcome.output_path.as_deref(),
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                    } else {
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_record_state_changed_event(
                                false,
                                "OBS_WEBSOCKET_OUTPUT_STARTING",
                                None,
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                        events.push(TaggedEvent {
                            text: crate::obsws::response::build_record_state_changed_event(
                                true,
                                "OBS_WEBSOCKET_OUTPUT_STARTED",
                                outcome.output_path.as_deref(),
                            ),
                            subscription_flag: OBSWS_EVENT_SUB_OUTPUTS,
                        });
                    }
                }
                (outcome, !was_active, events)
            }
            "rtmp_outbound" => {
                let was_active = self
                    .outputs
                    .get("rtmp_outbound")
                    .is_some_and(|o| o.runtime.active);
                let outcome = if was_active {
                    self.handle_stop_rtmp_outbound("ToggleOutput", request_id, "rtmp_outbound")
                        .await
                } else {
                    self.handle_start_rtmp_outbound("ToggleOutput", request_id, "rtmp_outbound")
                        .await
                };
                (outcome, !was_active, Vec::new())
            }
            "sora" => {
                let was_active = self.outputs.get("sora").is_some_and(|o| o.runtime.active);
                let outcome = if was_active {
                    self.handle_stop_sora_publisher("ToggleOutput", request_id, "sora")
                        .await
                } else {
                    self.handle_start_sora_publisher("ToggleOutput", request_id, "sora")
                        .await
                };
                (outcome, !was_active, Vec::new())
            }
            "hls" => {
                let was_active = self.outputs.get("hls").is_some_and(|o| o.runtime.active);
                let outcome = if was_active {
                    self.handle_stop_hls("ToggleOutput", request_id, "hls")
                        .await
                } else {
                    self.handle_start_hls("ToggleOutput", request_id, "hls")
                        .await
                };
                (outcome, !was_active, Vec::new())
            }
            "mpeg_dash" => {
                let was_active = self
                    .outputs
                    .get("mpeg_dash")
                    .is_some_and(|o| o.runtime.active);
                let outcome = if was_active {
                    self.handle_stop_mpeg_dash("ToggleOutput", request_id, "mpeg_dash")
                        .await
                } else {
                    self.handle_start_mpeg_dash("ToggleOutput", request_id, "mpeg_dash")
                        .await
                };
                (outcome, !was_active, Vec::new())
            }
            #[cfg(feature = "player")]
            "player" => {
                let was_active = self.input_registry.is_player_active();
                let outcome = if was_active {
                    self.handle_stop_player("ToggleOutput", request_id).await
                } else {
                    self.handle_start_player("ToggleOutput", request_id).await
                };
                (outcome, !was_active, Vec::new())
            }
            other => {
                let was_active = self.outputs.get(other).is_some_and(|o| o.runtime.active);
                let outcome = if was_active {
                    self.stop_dynamic_output("ToggleOutput", request_id, other)
                        .await
                } else {
                    self.start_dynamic_output("ToggleOutput", request_id, other)
                        .await
                };
                (outcome, !was_active, Vec::new())
            }
        };
        let response_text = if outcome.success {
            crate::obsws::response::build_toggle_output_response(
                request_id,
                output_active_on_success,
            )
        } else {
            outcome.response_text
        };
        self.build_result_from_response(response_text, events)
    }

    /// 動的に作成された output を kind に応じて起動する。
    /// outputs BTreeMap から output_name を検索し、OutputKind に応じて適切な start ハンドラを呼ぶ。
    async fn start_dynamic_output(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        use super::output_dynamic::OutputKind;
        let kind = self.outputs.get(output_name).map(|o| o.output_kind);
        let Some(kind) = kind else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                ),
            );
        };
        match kind {
            OutputKind::Stream => {
                self.handle_start_stream(request_type, request_id, output_name)
                    .await
            }
            OutputKind::Record => {
                self.handle_start_record(request_type, request_id, output_name)
                    .await
            }
            OutputKind::RtmpOutbound => {
                self.handle_start_rtmp_outbound(request_type, request_id, output_name)
                    .await
            }
            OutputKind::Sora => {
                self.handle_start_sora_publisher(request_type, request_id, output_name)
                    .await
            }
            OutputKind::Hls => {
                self.handle_start_hls(request_type, request_id, output_name)
                    .await
            }
            OutputKind::MpegDash => {
                self.handle_start_mpeg_dash(request_type, request_id, output_name)
                    .await
            }
        }
    }

    /// 動的に作成された output を kind に応じて停止する。
    async fn stop_dynamic_output(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        use super::output_dynamic::OutputKind;
        let kind = self.outputs.get(output_name).map(|o| o.output_kind);
        let Some(kind) = kind else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                ),
            );
        };
        match kind {
            OutputKind::Stream => {
                self.handle_stop_stream(request_type, request_id, output_name)
                    .await
            }
            OutputKind::Record => {
                self.handle_stop_record(request_type, request_id, output_name)
                    .await
            }
            OutputKind::RtmpOutbound => {
                self.handle_stop_rtmp_outbound(request_type, request_id, output_name)
                    .await
            }
            OutputKind::Sora => {
                self.handle_stop_sora_publisher(request_type, request_id, output_name)
                    .await
            }
            OutputKind::Hls => {
                self.handle_stop_hls(request_type, request_id, output_name)
                    .await
            }
            OutputKind::MpegDash => {
                self.handle_stop_mpeg_dash(request_type, request_id, output_name)
                    .await
            }
        }
    }
}

/// プロセッサを terminate してから停止を待つ
pub(crate) async fn terminate_and_wait(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_ids: &[crate::ProcessorId],
) -> crate::Result<()> {
    for id in processor_ids {
        let _ = pipeline_handle.terminate_processor(id.clone()).await;
    }
    wait_processors_stopped(pipeline_handle, processor_ids, Duration::from_secs(5)).await?;
    Ok(())
}

/// 指定したプロセッサが全て停止するまでポーリングする
pub(crate) async fn wait_processors_stopped(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_ids: &[crate::ProcessorId],
    timeout: Duration,
) -> crate::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let live = live_processor_ids(pipeline_handle, processor_ids).await;
        if live.is_empty() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(crate::Error::new("timeout waiting for processors to stop"));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// プロセッサの自然終了を待ち、タイムアウト後に強制停止する
pub(crate) async fn wait_or_terminate(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_ids: &[crate::ProcessorId],
    timeout: Duration,
) -> crate::Result<()> {
    if wait_processors_stopped(pipeline_handle, processor_ids, timeout)
        .await
        .is_ok()
    {
        return Ok(());
    }
    let live = live_processor_ids(pipeline_handle, processor_ids).await;
    if live.is_empty() {
        return Ok(());
    }
    terminate_and_wait(pipeline_handle, &live).await
}

/// 指定したプロセッサ ID のうち、まだ生存しているものを返す
pub(crate) async fn live_processor_ids(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_ids: &[crate::ProcessorId],
) -> Vec<crate::ProcessorId> {
    let Ok(live) = pipeline_handle.list_processors().await else {
        return Vec::new();
    };
    processor_ids
        .iter()
        .filter(|id| live.contains(id))
        .cloned()
        .collect()
}

/// ビデオエンコーダーとオーディオエンコーダーのプロセッサを起動する。
/// 各 output エンジン（record / stream / rtmp_outbound）で共通のエンコーダー起動処理。
pub(crate) async fn start_encoder_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    video: &crate::obsws::input_registry::ObswsRecordTrackRun,
    audio: &crate::obsws::input_registry::ObswsRecordTrackRun,
    audio_codec: crate::types::CodecName,
    frame_rate: crate::video::FrameRate,
) -> crate::Result<()> {
    crate::encoder::create_video_processor(
        pipeline_handle,
        video.source_track_id.clone(),
        video.encoded_track_id.clone(),
        crate::types::CodecName::H264,
        std::num::NonZeroUsize::new(2_000_000).expect("non-zero constant"),
        frame_rate,
        Some(video.encoder_processor_id.clone()),
    )
    .await?;
    crate::encoder::create_audio_processor(
        pipeline_handle,
        audio.source_track_id.clone(),
        audio.encoded_track_id.clone(),
        audio_codec,
        std::num::NonZeroUsize::new(128_000).expect("non-zero constant"),
        Some(audio.encoder_processor_id.clone()),
    )
    .await?;
    Ok(())
}

/// S3 クライアントを構築する
pub(crate) fn build_s3_client(
    region: &str,
    access_key_id: &str,
    secret_access_key: &str,
    session_token: Option<&str>,
    endpoint: Option<&str>,
    use_path_style: bool,
) -> crate::Result<crate::s3::S3HttpClient> {
    let credential = match session_token {
        Some(token) => {
            shiguredo_s3::Credential::with_session_token(access_key_id, secret_access_key, token)
        }
        None => shiguredo_s3::Credential::new(access_key_id, secret_access_key),
    };
    let mut config_builder = shiguredo_s3::S3Config::builder()
        .region(region)
        .credential(credential)
        .use_path_style(use_path_style);
    if let Some(ep) = endpoint {
        config_builder = config_builder.endpoint(ep);
    }
    let s3_config = config_builder
        .build()
        .map_err(|e| crate::Error::new(format!("failed to build S3 config: {e}")))?;
    Ok(crate::s3::S3HttpClient::new(s3_config))
}
