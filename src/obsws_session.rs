use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use shiguredo_websocket::CloseCode;
use tokio::sync::RwLock;

use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::{
    ActivateRecordError, ActivateStreamError, ObswsInputRegistry, ObswsInputSettings,
    ObswsRecordRun, ObswsStreamRun, PauseRecordError, ResumeRecordError,
};
use crate::obsws_message::{ClientMessage, ObswsSessionStats, RequestBatchMessage};
use crate::obsws_protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, OBSWS_EVENT_SUB_INPUTS, OBSWS_EVENT_SUB_OUTPUTS,
    OBSWS_EVENT_SUB_SCENES, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_TYPE,
    REQUEST_STATUS_OUTPUT_NOT_RUNNING, REQUEST_STATUS_OUTPUT_RUNNING,
    REQUEST_STATUS_REQUEST_PROCESSING_FAILED, REQUEST_STATUS_STREAM_NOT_RUNNING,
    REQUEST_STATUS_STREAM_RUNNING,
};

#[path = "obsws_session_input.rs"]
mod obsws_session_input;
#[path = "obsws_session_output.rs"]
mod obsws_session_output;
#[path = "obsws_session_scene.rs"]
mod obsws_session_scene;
#[path = "obsws_session_scene_item.rs"]
mod obsws_session_scene_item;
#[cfg(test)]
#[path = "obsws_session_tests.rs"]
mod tests;

pub enum SessionAction {
    SendTexts {
        messages: Vec<(String, &'static str)>,
    },
    SendText {
        text: String,
        message_name: &'static str,
    },
    Close {
        code: CloseCode,
        reason: &'static str,
        close_error_context: &'static str,
    },
    Terminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObswsSessionState {
    AwaitingIdentify,
    Identified,
}

#[derive(Debug, Clone, Copy)]
enum RecordWriterRpcOperation {
    Pause,
    Resume,
}

impl RecordWriterRpcOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
        }
    }
}

struct RequestOutcome {
    response_text: String,
    success: bool,
    output_path: Option<String>,
    error_code: Option<i64>,
    error_comment: Option<String>,
}

impl RequestOutcome {
    fn success(response_text: String, output_path: Option<String>) -> Self {
        Self {
            response_text,
            success: true,
            output_path,
            error_code: None,
            error_comment: None,
        }
    }

    fn failure(response_text: String, error_code: i64, error_comment: impl Into<String>) -> Self {
        Self {
            response_text,
            success: false,
            output_path: None,
            error_code: Some(error_code),
            error_comment: Some(error_comment.into()),
        }
    }

    fn failure_with_output_path(
        response_text: String,
        error_code: i64,
        error_comment: impl Into<String>,
        output_path: String,
    ) -> Self {
        Self {
            response_text,
            success: false,
            output_path: Some(output_path),
            error_code: Some(error_code),
            error_comment: Some(error_comment.into()),
        }
    }
}

struct RequestExecutionResult {
    response_text: String,
    batch_result: crate::obsws_response_builder::RequestBatchResult,
    events: Vec<String>,
}

impl RequestExecutionResult {
    fn into_session_action(self) -> SessionAction {
        if self.events.is_empty() {
            return SessionAction::SendText {
                text: self.response_text,
                message_name: "request response message",
            };
        }
        let mut messages = Vec::with_capacity(self.events.len() + 1);
        messages.push((self.response_text, "request response message"));
        messages.extend(
            self.events
                .into_iter()
                .map(|event| (event, "event message")),
        );
        SessionAction::SendTexts { messages }
    }
}

pub struct ObswsSession {
    state: ObswsSessionState,
    negotiated_rpc_version: Option<u32>,
    event_subscriptions: u32,
    auth: Option<ObswsAuthentication>,
    input_registry: Arc<RwLock<ObswsInputRegistry>>,
    pipeline_handle: Option<crate::MediaPipelineHandle>,
    stats: ObswsSessionStats,
}

impl ObswsSession {
    pub fn new(
        auth: Option<ObswsAuthentication>,
        input_registry: Arc<RwLock<ObswsInputRegistry>>,
        pipeline_handle: Option<crate::MediaPipelineHandle>,
    ) -> Self {
        Self {
            state: ObswsSessionState::AwaitingIdentify,
            negotiated_rpc_version: None,
            event_subscriptions: 0,
            auth,
            input_registry,
            pipeline_handle,
            stats: ObswsSessionStats::default(),
        }
    }

    pub fn stats_mut(&mut self) -> &mut ObswsSessionStats {
        &mut self.stats
    }

    pub fn on_connected(&self) -> SessionAction {
        SessionAction::SendText {
            text: crate::obsws_message::build_hello_message(self.auth.as_ref()),
            message_name: "hello message",
        }
    }

    pub async fn on_text_message(&mut self, text: &str) -> crate::Result<SessionAction> {
        self.stats.incoming_messages = self.stats.incoming_messages.saturating_add(1);

        let message = crate::obsws_message::parse_client_message(text)?;
        let action = match message {
            ClientMessage::Identify(identify) => self.handle_identify(identify),
            ClientMessage::Reidentify(reidentify) => self.handle_reidentify(reidentify),
            ClientMessage::Request(request) => self.handle_request(request).await,
            ClientMessage::RequestBatch(request_batch) => {
                self.handle_request_batch(request_batch).await
            }
        };
        Ok(action)
    }

    pub fn on_close_event(&self) -> SessionAction {
        SessionAction::Terminate
    }

    pub fn on_error_event(&self) -> SessionAction {
        SessionAction::Terminate
    }

    fn build_internal_error_response(
        request_type: &str,
        request_id: &str,
        message: &str,
    ) -> String {
        crate::obsws_response_builder::build_request_response_error(
            request_type,
            request_id,
            REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
            message,
        )
    }

    fn handle_identify(
        &mut self,
        identify: crate::obsws_message::IdentifyMessage,
    ) -> SessionAction {
        if self.state == ObswsSessionState::Identified {
            return SessionAction::Close {
                code: OBSWS_CLOSE_ALREADY_IDENTIFIED,
                reason: "already identified",
                close_error_context: "failed to close websocket for duplicated identify",
            };
        }

        if !crate::obsws_message::is_supported_rpc_version(identify.rpc_version) {
            return SessionAction::Close {
                code: OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION,
                reason: "unsupported rpc version",
                close_error_context: "failed to close websocket for unsupported rpc version",
            };
        }

        if let Some(auth) = self.auth.as_ref()
            && !auth.is_valid_response(identify.authentication.as_deref())
        {
            return SessionAction::Close {
                code: OBSWS_CLOSE_AUTHENTICATION_FAILED,
                reason: "authentication failed",
                close_error_context: "failed to close websocket for authentication failure",
            };
        }

        self.state = ObswsSessionState::Identified;
        self.negotiated_rpc_version = Some(identify.rpc_version);
        self.event_subscriptions = identify.event_subscriptions.unwrap_or(0);
        SessionAction::SendText {
            text: crate::obsws_message::build_identified_message(identify.rpc_version),
            message_name: "identified message",
        }
    }

    fn handle_reidentify(
        &mut self,
        reidentify: crate::obsws_message::ReidentifyMessage,
    ) -> SessionAction {
        if self.state != ObswsSessionState::Identified {
            return SessionAction::Close {
                code: OBSWS_CLOSE_NOT_IDENTIFIED,
                reason: "identify is required",
                close_error_context: "failed to close websocket for unidentified reidentify",
            };
        }
        if let Some(event_subscriptions) = reidentify.event_subscriptions {
            self.event_subscriptions = event_subscriptions;
        }

        let negotiated_rpc_version = self
            .negotiated_rpc_version
            .expect("negotiated rpc version must be set after identify");
        SessionAction::SendText {
            text: crate::obsws_message::build_identified_message(negotiated_rpc_version),
            message_name: "identified message",
        }
    }

    async fn handle_request(
        &mut self,
        request: crate::obsws_message::RequestMessage,
    ) -> SessionAction {
        if self.state != ObswsSessionState::Identified {
            return SessionAction::Close {
                code: OBSWS_CLOSE_NOT_IDENTIFIED,
                reason: "identify is required",
                close_error_context: "failed to close websocket for unidentified request",
            };
        }
        let request_id = request.request_id.clone().unwrap_or_default();
        let request_type = request.request_type.clone().unwrap_or_default();
        match self.handle_request_internal(request).await {
            Ok(execution) => execution.into_session_action(),
            Err(_) => SessionAction::SendText {
                text: Self::build_internal_error_response(
                    &request_type,
                    &request_id,
                    "Failed to build internal request response",
                ),
                message_name: "request response message",
            },
        }
    }

    async fn handle_request_batch(&mut self, request_batch: RequestBatchMessage) -> SessionAction {
        if self.state != ObswsSessionState::Identified {
            return SessionAction::Close {
                code: OBSWS_CLOSE_NOT_IDENTIFIED,
                reason: "identify is required",
                close_error_context: "failed to close websocket for unidentified request batch",
            };
        }

        let request_id = request_batch.request_id.unwrap_or_default();
        if request_id.is_empty() {
            return SessionAction::SendText {
                text: crate::obsws_response_builder::build_request_response_error(
                    "RequestBatch",
                    &request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing required requestId field",
                ),
                message_name: "request response message",
            };
        }

        let execution_type = request_batch.execution_type.unwrap_or(0);
        if execution_type != 0 {
            return SessionAction::SendText {
                text: crate::obsws_response_builder::build_request_response_error(
                    "RequestBatch",
                    &request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Unsupported executionType field",
                ),
                message_name: "request response message",
            };
        }

        let Some(requests) = request_batch.requests else {
            return SessionAction::SendText {
                text: crate::obsws_response_builder::build_request_response_error(
                    "RequestBatch",
                    &request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing required requests field",
                ),
                message_name: "request response message",
            };
        };

        let halt_on_failure = request_batch.halt_on_failure.unwrap_or(false);
        let mut results = Vec::new();
        let mut events = Vec::new();
        for (index, request) in requests.into_iter().enumerate() {
            let sub_request_id = request
                .request_id
                .clone()
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| format!("{request_id}:{index}"));
            let execution = match self
                .handle_request_internal(crate::obsws_message::RequestMessage {
                    request_id: Some(sub_request_id),
                    request_type: request.request_type,
                    request_data: request.request_data,
                })
                .await
            {
                Ok(execution) => execution,
                Err(_) => {
                    return SessionAction::SendText {
                        text: Self::build_internal_error_response(
                            "RequestBatch",
                            &request_id,
                            "Failed to build request batch response",
                        ),
                        message_name: "request response message",
                    };
                }
            };
            let request_succeeded = execution.batch_result.request_status_result;
            results.push(execution.batch_result);
            events.extend(
                execution
                    .events
                    .into_iter()
                    .map(|text| (text, "event message")),
            );
            if halt_on_failure && !request_succeeded {
                break;
            }
        }

        let response_text =
            crate::obsws_response_builder::build_request_batch_response(&request_id, &results);
        if events.is_empty() {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request batch response message",
            };
        }

        let mut messages = Vec::with_capacity(events.len() + 1);
        // [NOTE]
        // RequestBatch では、client が batch 完了を判定しやすいよう
        // RequestBatchResponse (op=9) を先に返し、その後に event を送信する。
        // OBS 実装と厳密に同一順序ではない可能性があるため、
        // 互換性要件が変わる場合は送信順序を再検討すること。
        messages.push((response_text, "request batch response message"));
        messages.extend(events);
        SessionAction::SendTexts { messages }
    }

    async fn handle_request_internal(
        &mut self,
        request: crate::obsws_message::RequestMessage,
    ) -> crate::Result<RequestExecutionResult> {
        let request_id = request.request_id.clone().unwrap_or_default();
        let request_type = request.request_type.clone().unwrap_or_default();
        if request_id.is_empty() {
            return Ok(Self::build_error_execution(
                &request_type,
                &request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required requestId field",
            ));
        }
        if request_type.is_empty() {
            return Ok(Self::build_error_execution(
                &request_type,
                &request_id,
                REQUEST_STATUS_MISSING_REQUEST_TYPE,
                "Missing required requestType field",
            ));
        }

        if request_type == "StartStream" {
            let outcome = self.handle_start_stream(&request_id).await;
            let mut events = Vec::new();
            if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                events.push(crate::obsws_response_builder::build_stream_state_changed_event(true));
            }
            return Self::build_execution_from_outcome("StartStream", outcome, events);
        }
        if request_type == "ToggleStream" {
            let was_active = self.input_registry.read().await.is_stream_active();
            let outcome = if was_active {
                self.handle_stop_stream(&request_id).await
            } else {
                self.handle_start_stream(&request_id).await
            };
            let mut events = Vec::new();
            if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                events.push(
                    crate::obsws_response_builder::build_stream_state_changed_event(!was_active),
                );
            }
            let response_text = Self::build_toggle_response_from_outcome(
                "ToggleStream",
                &request_id,
                !was_active,
                &outcome,
            )?;
            return Self::build_execution_from_response_text(response_text, events);
        }
        if request_type == "StopStream" {
            let outcome = self.handle_stop_stream(&request_id).await;
            let mut events = Vec::new();
            if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                events.push(crate::obsws_response_builder::build_stream_state_changed_event(false));
            }
            return Self::build_execution_from_outcome("StopStream", outcome, events);
        }
        if request_type == "StartRecord" {
            let outcome = self.handle_start_record(&request_id).await;
            let mut events = Vec::new();
            if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        true, false, None,
                    ),
                );
            }
            return Self::build_execution_from_outcome("StartRecord", outcome, events);
        }
        if request_type == "ToggleRecord" {
            let was_active = self.input_registry.read().await.is_record_active();
            let outcome = if was_active {
                self.handle_stop_record(&request_id).await
            } else {
                self.handle_start_record(&request_id).await
            };
            let mut events = Vec::new();
            if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                let output_path = if was_active {
                    outcome.output_path.as_deref()
                } else {
                    None
                };
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        !was_active,
                        false,
                        output_path,
                    ),
                );
            }
            let response_text = Self::build_toggle_response_from_outcome(
                "ToggleRecord",
                &request_id,
                !was_active,
                &outcome,
            )?;
            return Self::build_execution_from_response_text(response_text, events);
        }
        if request_type == "StopRecord" {
            let outcome = self.handle_stop_record(&request_id).await;
            let mut events = Vec::new();
            if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        false,
                        false,
                        outcome.output_path.as_deref(),
                    ),
                );
            }
            return Self::build_execution_from_outcome("StopRecord", outcome, events);
        }
        if request_type == "PauseRecord" {
            let outcome = self.handle_pause_record(&request_id).await;
            let mut events = Vec::new();
            if outcome.success && self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                events.push(
                    crate::obsws_response_builder::build_record_state_changed_event(
                        true, true, None,
                    ),
                );
            }
            return Self::build_execution_from_outcome("PauseRecord", outcome, events);
        }
        if request_type == "ResumeRecord" {
            let outcome = self.handle_resume_record(&request_id).await;
            let mut events = Vec::new();
            if self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                if outcome.success {
                    events.push(
                        crate::obsws_response_builder::build_record_state_changed_event(
                            true, false, None,
                        ),
                    );
                } else if outcome.output_path.is_some() {
                    // [NOTE]
                    // ResumeRecord の内部復旧で録画停止へフォールバックした場合は、
                    // request 自体は失敗でも出力状態の遷移（ inactive ）を通知する。
                    events.push(
                        crate::obsws_response_builder::build_record_state_changed_event(
                            false,
                            false,
                            outcome.output_path.as_deref(),
                        ),
                    );
                }
            }
            return Self::build_execution_from_outcome("ResumeRecord", outcome, events);
        }
        if request_type == "ToggleRecordPause" {
            let was_paused = self.input_registry.read().await.is_record_paused();
            let outcome = if was_paused {
                self.handle_resume_record(&request_id).await
            } else {
                self.handle_pause_record(&request_id).await
            };
            let mut events = Vec::new();
            if self.is_event_subscription_enabled(OBSWS_EVENT_SUB_OUTPUTS) {
                if outcome.success {
                    events.push(
                        crate::obsws_response_builder::build_record_state_changed_event(
                            true,
                            !was_paused,
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
                            false,
                            outcome.output_path.as_deref(),
                        ),
                    );
                }
            }
            let response_text = Self::build_toggle_response_from_outcome(
                "ToggleRecordPause",
                &request_id,
                !was_paused,
                &outcome,
            )?;
            return Self::build_execution_from_response_text(response_text, events);
        }
        if request_type == "SetCurrentProgramScene" {
            let action = self
                .handle_set_current_program_scene_request(
                    &request_id,
                    request.request_data.as_ref(),
                )
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "SetCurrentPreviewScene" {
            let action = self
                .handle_set_current_preview_scene_request(
                    &request_id,
                    request.request_data.as_ref(),
                )
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "CreateScene" {
            let action = self
                .handle_create_scene_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "RemoveScene" {
            let action = self
                .handle_remove_scene_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "CreateInput" {
            let action = self
                .handle_create_input_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "RemoveInput" {
            let action = self
                .handle_remove_input_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "SetInputSettings" {
            let action = self
                .handle_set_input_settings_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "SetInputName" {
            let action = self
                .handle_set_input_name_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "CreateSceneItem" {
            let action = self
                .handle_create_scene_item_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "RemoveSceneItem" {
            let action = self
                .handle_remove_scene_item_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "DuplicateSceneItem" {
            let action = self
                .handle_duplicate_scene_item_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "SetSceneItemEnabled" {
            let action = self
                .handle_set_scene_item_enabled_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "SetSceneItemLocked" {
            let action = self
                .handle_set_scene_item_locked_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "SetSceneItemIndex" {
            let action = self
                .handle_set_scene_item_index_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "SetSceneItemBlendMode" {
            let action = self
                .handle_set_scene_item_blend_mode_request(
                    &request_id,
                    request.request_data.as_ref(),
                )
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "SetSceneItemTransform" {
            let action = self
                .handle_set_scene_item_transform_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }

        let mut input_registry = self.input_registry.write().await;
        let response =
            crate::obsws_message::handle_request_message(request, &self.stats, &mut input_registry);
        Self::build_execution_from_response_text(response.message, Vec::new())
    }

    fn build_execution_from_outcome(
        _request_type: &str,
        outcome: RequestOutcome,
        events: Vec<String>,
    ) -> crate::Result<RequestExecutionResult> {
        Self::build_execution_from_response_text(outcome.response_text, events)
    }

    fn build_error_execution(
        request_type: &str,
        request_id: &str,
        status_code: i64,
        status_comment: &str,
    ) -> RequestExecutionResult {
        RequestExecutionResult {
            response_text: crate::obsws_response_builder::build_request_response_error(
                request_type,
                request_id,
                status_code,
                status_comment,
            ),
            batch_result: crate::obsws_response_builder::RequestBatchResult {
                request_type: request_type.to_owned(),
                request_status_result: false,
                request_status_code: status_code,
                request_status_comment: Some(status_comment.to_owned()),
                response_data: None,
            },
            events: Vec::new(),
        }
    }

    fn build_execution_from_response_text(
        response_text: String,
        events: Vec<String>,
    ) -> crate::Result<RequestExecutionResult> {
        let batch_result =
            crate::obsws_response_builder::parse_request_response_for_batch_result(&response_text)?;
        Ok(RequestExecutionResult {
            response_text,
            batch_result,
            events,
        })
    }

    fn build_execution_from_action(action: SessionAction) -> crate::Result<RequestExecutionResult> {
        match action {
            SessionAction::SendText { text, .. } => {
                Self::build_execution_from_response_text(text, Vec::new())
            }
            SessionAction::SendTexts { messages } => {
                let mut iter = messages.into_iter();
                // [NOTE]
                // SendTexts は「先頭が request response、2 件目以降が event」という
                // 形式を前提に組み立てる。これは obsws の各 request ハンドラ
                // （例: Scene / Input / Output 系）が共通で守る契約とする。
                // もし順序規約を変更する場合は、この抽出ロジックも同時に更新すること。
                let Some((response_text, _)) = iter.next() else {
                    return Err(crate::Error::new("response message is missing"));
                };
                let events = iter.map(|(text, _)| text).collect();
                Self::build_execution_from_response_text(response_text, events)
            }
            SessionAction::Close { .. } | SessionAction::Terminate => {
                Err(crate::Error::new("request handler returned invalid action"))
            }
        }
    }

    fn build_toggle_response_from_outcome(
        toggle_request_type: &str,
        request_id: &str,
        output_active_on_success: bool,
        outcome: &RequestOutcome,
    ) -> crate::Result<String> {
        if outcome.success {
            return match toggle_request_type {
                "ToggleStream" => Ok(crate::obsws_response_builder::build_toggle_stream_response(
                    request_id,
                    output_active_on_success,
                )),
                "ToggleRecord" => Ok(crate::obsws_response_builder::build_toggle_record_response(
                    request_id,
                    output_active_on_success,
                )),
                "ToggleRecordPause" => Ok(
                    crate::obsws_response_builder::build_toggle_record_pause_response(
                        request_id,
                        output_active_on_success,
                    ),
                ),
                _ => Err(crate::Error::new("unknown toggle request type")),
            };
        }

        let code = outcome
            .error_code
            .unwrap_or(REQUEST_STATUS_INVALID_REQUEST_FIELD);
        let comment = outcome
            .error_comment
            .as_deref()
            .unwrap_or("Unknown request error");
        Ok(crate::obsws_response_builder::build_request_response_error(
            toggle_request_type,
            request_id,
            code,
            comment,
        ))
    }

    fn is_event_subscription_enabled(&self, event_flag: u32) -> bool {
        (self.event_subscriptions & event_flag) != 0
    }

    fn parse_required_non_empty_string_request_field(
        request_data: Option<&nojson::RawJsonOwned>,
        field_name: &str,
    ) -> Option<String> {
        let request_data = request_data?;
        let value: Option<String> = request_data
            .value()
            .to_member(field_name)
            .ok()?
            .try_into()
            .ok()?;
        let value = value?;
        if value.is_empty() {
            return None;
        }
        Some(value)
    }

    fn build_missing_request_data_error_action(
        request_type: &str,
        request_id: &str,
    ) -> SessionAction {
        SessionAction::SendText {
            text: crate::obsws_response_builder::build_request_response_error(
                request_type,
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required requestData field",
            ),
            message_name: "request response message",
        }
    }

    fn build_parse_error_action(
        request_type: &str,
        request_id: &str,
        error: &nojson::JsonParseError,
    ) -> SessionAction {
        let code = crate::obsws_response_builder::request_status_code_for_parse_error(error);
        SessionAction::SendText {
            text: crate::obsws_response_builder::build_request_response_error(
                request_type,
                request_id,
                code,
                &error.to_string(),
            ),
            message_name: "request response message",
        }
    }
}
