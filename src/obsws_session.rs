use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use shiguredo_websocket::CloseCode;
use tokio::sync::RwLock;

use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::{
    ActivateRecordError, ActivateStreamError, ObswsInputRegistry, ObswsInputSettings,
    ObswsRecordRun, ObswsStreamRun, SetSceneItemEnabledError,
};
use crate::obsws_message::{ClientMessage, ObswsSessionStats, RequestBatchMessage};
use crate::obsws_protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, OBSWS_EVENT_SUB_INPUTS, OBSWS_EVENT_SUB_OUTPUTS,
    OBSWS_EVENT_SUB_SCENES, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_TYPE,
    REQUEST_STATUS_OUTPUT_NOT_RUNNING, REQUEST_STATUS_OUTPUT_RUNNING,
    REQUEST_STATUS_STREAM_NOT_RUNNING, REQUEST_STATUS_STREAM_RUNNING,
};

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
        // [NOTE]
        // 現在 hisui が実装している obsws requestStatus code では、
        // サーバー内部エラー専用のコードを定義していない。
        // そのため互換性を保ったまま 400 を内部エラーにも流用している。
        crate::obsws_response_builder::build_request_response_error(
            request_type,
            request_id,
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
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
                    crate::obsws_response_builder::build_record_state_changed_event(true, None),
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
                        outcome.output_path.as_deref(),
                    ),
                );
            }
            return Self::build_execution_from_outcome("StopRecord", outcome, events);
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
        if request_type == "SetSceneItemEnabled" {
            let action = self
                .handle_set_scene_item_enabled_request(&request_id, request.request_data.as_ref())
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

    fn parse_input_lookup_fields(
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> Option<(Option<String>, Option<String>)> {
        let request_data = request_data?;
        let input_uuid: Option<String> = request_data
            .value()
            .to_member("inputUuid")
            .ok()?
            .try_into()
            .ok()?;
        let input_name: Option<String> = request_data
            .value()
            .to_member("inputName")
            .ok()?
            .try_into()
            .ok()?;
        let input_uuid = input_uuid.filter(|v| !v.is_empty());
        let input_name = input_name.filter(|v| !v.is_empty());
        if input_uuid.is_none() && input_name.is_none() {
            return None;
        }
        Some((input_uuid, input_name))
    }

    async fn handle_set_current_program_scene_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let previous_scene_name = input_registry
            .current_program_scene()
            .map(|scene| scene.scene_name);
        let response_text = crate::obsws_response_builder::build_set_current_program_scene_response(
            request_id,
            request_data,
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENES) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(current_scene) = input_registry.current_program_scene() else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        if previous_scene_name.as_deref() == Some(current_scene.scene_name.as_str()) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }

        let event_text = crate::obsws_response_builder::build_current_program_scene_changed_event(
            &current_scene.scene_name,
            &current_scene.scene_uuid,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    async fn handle_create_scene_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let requested_scene_name =
            Self::parse_required_non_empty_string_request_field(request_data, "sceneName");
        let existed_before = requested_scene_name.as_deref().is_some_and(|scene_name| {
            input_registry
                .list_scenes()
                .into_iter()
                .any(|scene| scene.scene_name == scene_name)
        });
        let response_text = crate::obsws_response_builder::build_create_scene_response(
            request_id,
            request_data,
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENES) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        if existed_before {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(requested_scene_name) = requested_scene_name else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let Some(created_scene) = input_registry
            .list_scenes()
            .into_iter()
            .find(|scene| scene.scene_name == requested_scene_name)
        else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };

        let event_text = crate::obsws_response_builder::build_scene_created_event(
            &created_scene.scene_name,
            &created_scene.scene_uuid,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    async fn handle_remove_scene_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let target_scene_name =
            Self::parse_required_non_empty_string_request_field(request_data, "sceneName");
        let removed_scene = target_scene_name.as_deref().and_then(|scene_name| {
            input_registry
                .list_scenes()
                .into_iter()
                .find(|scene| scene.scene_name == scene_name)
        });
        let previous_current_scene_name = input_registry
            .current_program_scene()
            .map(|scene| scene.scene_name);
        let response_text = crate::obsws_response_builder::build_remove_scene_response(
            request_id,
            request_data,
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENES) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(removed_scene) = removed_scene else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let removed_succeeded = input_registry
            .list_scenes()
            .into_iter()
            .all(|scene| scene.scene_uuid != removed_scene.scene_uuid);
        if !removed_succeeded {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }

        let mut messages = vec![
            (response_text, "request response message"),
            (
                crate::obsws_response_builder::build_scene_removed_event(
                    &removed_scene.scene_name,
                    &removed_scene.scene_uuid,
                ),
                "event message",
            ),
        ];
        if previous_current_scene_name.as_deref() == Some(removed_scene.scene_name.as_str())
            && let Some(current_scene) = input_registry.current_program_scene()
        {
            messages.push((
                crate::obsws_response_builder::build_current_program_scene_changed_event(
                    &current_scene.scene_name,
                    &current_scene.scene_uuid,
                ),
                "event message",
            ));
        }

        SessionAction::SendTexts { messages }
    }

    async fn handle_create_input_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let requested_input_name =
            Self::parse_required_non_empty_string_request_field(request_data, "inputName");
        let existed_before = requested_input_name
            .as_deref()
            .is_some_and(|input_name| input_registry.find_input(None, Some(input_name)).is_some());
        let response_text = crate::obsws_response_builder::build_create_input_response(
            request_id,
            request_data,
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_INPUTS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        if existed_before {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(requested_input_name) = requested_input_name else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let Some(created_input) = input_registry
            .find_input(None, Some(requested_input_name.as_str()))
            .cloned()
        else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let event_text = crate::obsws_response_builder::build_input_created_event(
            &created_input.input_name,
            &created_input.input_uuid,
            created_input.input.kind_name(),
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    async fn handle_remove_input_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let mut input_registry = self.input_registry.write().await;
        let removed_input =
            Self::parse_input_lookup_fields(request_data).and_then(|(input_uuid, input_name)| {
                input_registry
                    .find_input(input_uuid.as_deref(), input_name.as_deref())
                    .cloned()
            });
        let response_text = crate::obsws_response_builder::build_remove_input_response(
            request_id,
            request_data,
            &mut input_registry,
        );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_INPUTS) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }
        let Some(removed_input) = removed_input else {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        };
        let removed_succeeded = input_registry
            .find_input(Some(&removed_input.input_uuid), None)
            .is_none();
        if !removed_succeeded {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }

        let event_text = crate::obsws_response_builder::build_input_removed_event(
            &removed_input.input_name,
            &removed_input.input_uuid,
            removed_input.input.kind_name(),
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    async fn handle_set_scene_item_enabled_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let Some(request_data) = request_data else {
            return SessionAction::SendText {
                text: crate::obsws_response_builder::build_request_response_error(
                    "SetSceneItemEnabled",
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing required requestData field",
                ),
                message_name: "request response message",
            };
        };
        let fields = (|| -> Result<(String, i64, bool), nojson::JsonParseError> {
            let request_data = request_data.value();
            let scene_name_raw = request_data.to_member("sceneName")?.required()?;
            let scene_name: String = scene_name_raw.try_into()?;
            if scene_name.is_empty() {
                return Err(scene_name_raw.invalid("string must not be empty"));
            }
            let scene_item_id: i64 = request_data
                .to_member("sceneItemId")?
                .required()?
                .try_into()?;
            let scene_item_enabled: bool = request_data
                .to_member("sceneItemEnabled")?
                .required()?
                .try_into()?;
            Ok((scene_name, scene_item_id, scene_item_enabled))
        })();
        let (scene_name, scene_item_id, scene_item_enabled) = match fields {
            Ok(fields) => fields,
            Err(e) => {
                return SessionAction::SendText {
                    text: crate::obsws_response_builder::build_request_response_error(
                        "SetSceneItemEnabled",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        &format!("Invalid requestData field: {e}"),
                    ),
                    message_name: "request response message",
                };
            }
        };

        let result = {
            let mut input_registry = self.input_registry.write().await;
            input_registry.set_scene_item_enabled(&scene_name, scene_item_id, scene_item_enabled)
        };
        let result = match result {
            Ok(result) => result,
            Err(SetSceneItemEnabledError::SceneNotFound) => {
                return SessionAction::SendText {
                    text: crate::obsws_response_builder::build_request_response_error(
                        "SetSceneItemEnabled",
                        request_id,
                        crate::obsws_protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                        "Scene not found",
                    ),
                    message_name: "request response message",
                };
            }
            Err(SetSceneItemEnabledError::SceneItemNotFound) => {
                return SessionAction::SendText {
                    text: crate::obsws_response_builder::build_request_response_error(
                        "SetSceneItemEnabled",
                        request_id,
                        crate::obsws_protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                        "Scene item not found",
                    ),
                    message_name: "request response message",
                };
            }
        };

        let response_text =
            crate::obsws_response_builder::build_set_scene_item_enabled_success_response(
                request_id,
            );
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_SCENES) || !result.changed {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }

        let event_text = crate::obsws_response_builder::build_scene_item_enable_state_changed_event(
            &scene_name,
            scene_item_id,
            scene_item_enabled,
        );
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    async fn handle_start_stream(&self, request_id: &str) -> RequestOutcome {
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
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                error_comment,
            );
        }

        RequestOutcome::success(
            crate::obsws_response_builder::build_start_stream_response(request_id, true),
            None,
        )
    }

    async fn handle_stop_stream(&self, request_id: &str) -> RequestOutcome {
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
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
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

    async fn handle_start_record(&self, request_id: &str) -> RequestOutcome {
        let (image_path, output_path, run) = {
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
            let ObswsInputSettings::ImageSource(settings) = &input.input.settings else {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartRecord",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Only image_source is supported for StartRecord",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Only image_source is supported for StartRecord",
                );
            };
            let Some(image_path) = settings.file.clone() else {
                return RequestOutcome::failure(
                    crate::obsws_response_builder::build_request_response_error(
                        "StartRecord",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "inputSettings.file is required for image_source",
                    ),
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "inputSettings.file is required for image_source",
                );
            };
            let run_id = input_registry.next_record_run_id();
            let source_processor_id = format!("obsws:record:{run_id}:png_source");
            let encoder_processor_id = format!("obsws:record:{run_id}:video_encoder");
            let writer_processor_id = format!("obsws:record:{run_id}:mp4_writer");
            let source_track_id = format!("obsws:record:{run_id}:raw_video");
            let encoded_track_id = format!("obsws:record:{run_id}:encoded_video");
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_millis();
            let output_path = input_registry
                .record_directory()
                .join(format!("obsws-record-{timestamp}.mp4"));
            let run = ObswsRecordRun {
                source_processor_id,
                encoder_processor_id,
                writer_processor_id,
                source_track_id,
                encoded_track_id,
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
            (image_path, output_path, run)
        };

        if let Some(parent) = output_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            let _ = self.input_registry.write().await.deactivate_record();
            let error_comment = format!("Failed to create record directory: {e}");
            return RequestOutcome::failure(
                Self::build_internal_error_response("StartRecord", request_id, &error_comment),
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                error_comment,
            );
        }

        let start_result = self
            .start_record_processors(&image_path, &output_path, &run)
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
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                error_comment,
            );
        }

        RequestOutcome::success(
            crate::obsws_response_builder::build_start_record_response(request_id, true),
            None,
        )
    }

    async fn handle_stop_record(&self, request_id: &str) -> RequestOutcome {
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
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
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

    async fn start_stream_processors(
        &self,
        image_path: &str,
        output_url: &str,
        stream_name: Option<&str>,
        run: &ObswsStreamRun,
    ) -> crate::Result<()> {
        // [NOTE]
        // ここで送る内部 JSON-RPC は常に 1 件ずつ送信して即時 await しているため、
        // 相関に id は使っておらず固定値を意図的に使用している。
        // 将来並列送信へ拡張する場合は id をユニーク化すること。
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

    async fn start_record_processors(
        &self,
        image_path: &str,
        output_path: &std::path::Path,
        run: &ObswsRecordRun,
    ) -> crate::Result<()> {
        // [NOTE]
        // ここで送る内部 JSON-RPC は常に 1 件ずつ送信して即時 await しているため、
        // 相関に id は使っておらず固定値を意図的に使用している。
        // 将来並列送信へ拡張する場合は id をユニーク化すること。
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

        let writer_request = nojson::object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("id", 1)?;
            f.member("method", "createMp4Writer")?;
            f.member(
                "params",
                nojson::object(|f| {
                    f.member("outputPath", output_path.display().to_string())?;
                    f.member("inputVideoTrackId", &run.encoded_track_id)?;
                    f.member("processorId", &run.writer_processor_id)
                }),
            )
        })
        .to_string();
        self.send_pipeline_rpc_request("createMp4Writer", &writer_request)
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

    async fn send_pipeline_rpc_request(
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

    async fn stop_stream_processors(&self, run: &ObswsStreamRun) -> crate::Result<()> {
        self.stop_processors(&[
            crate::ProcessorId::new(run.endpoint_processor_id.clone()),
            crate::ProcessorId::new(run.encoder_processor_id.clone()),
            crate::ProcessorId::new(run.source_processor_id.clone()),
        ])
        .await
    }

    async fn stop_record_processors(&self, run: &ObswsRecordRun) -> crate::Result<()> {
        self.stop_processors(&[
            crate::ProcessorId::new(run.writer_processor_id.clone()),
            crate::ProcessorId::new(run.encoder_processor_id.clone()),
            crate::ProcessorId::new(run.source_processor_id.clone()),
        ])
        .await
    }

    async fn stop_processors(&self, processor_ids: &[crate::ProcessorId]) -> crate::Result<()> {
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

    async fn wait_processors_stopped(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsws_auth::build_authentication_response;
    use crate::obsws_message::RequestMessage;
    use crate::obsws_protocol::{
        OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED,
        OBSWS_CLOSE_NOT_IDENTIFIED, OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, OBSWS_EVENT_SUB_INPUTS,
        OBSWS_EVENT_SUB_OUTPUTS, OBSWS_EVENT_SUB_SCENES, REQUEST_STATUS_INVALID_REQUEST_FIELD,
        REQUEST_STATUS_OUTPUT_NOT_RUNNING,
    };
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn input_registry() -> Arc<RwLock<ObswsInputRegistry>> {
        Arc::new(RwLock::new(ObswsInputRegistry::new_for_test()))
    }

    fn parse_request_status(text: &str) -> (bool, i64) {
        let json = nojson::RawJson::parse(text).expect("response must be valid json");
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])
            .expect("requestStatus access must succeed")
            .required()
            .expect("requestStatus must exist");
        let result: bool = status
            .to_member("result")
            .expect("result access must succeed")
            .required()
            .expect("result must exist")
            .try_into()
            .expect("result must be bool");
        let code: i64 = status
            .to_member("code")
            .expect("code access must succeed")
            .required()
            .expect("code must exist")
            .try_into()
            .expect("code must be i64");
        (result, code)
    }

    fn parse_request_type(text: &str) -> String {
        let json = nojson::RawJson::parse(text).expect("response must be valid json");
        json.value()
            .to_path_member(&["d", "requestType"])
            .expect("requestType access must succeed")
            .required()
            .expect("requestType must exist")
            .try_into()
            .expect("requestType must be string")
    }

    fn parse_response_scene_item_id(text: &str) -> i64 {
        let json = nojson::RawJson::parse(text).expect("response must be valid json");
        json.value()
            .to_path_member(&["d", "responseData", "sceneItemId"])
            .expect("sceneItemId access must succeed")
            .required()
            .expect("sceneItemId must exist")
            .try_into()
            .expect("sceneItemId must be i64")
    }

    fn parse_identified_message(text: &str) -> (i64, u32) {
        let json = nojson::RawJson::parse(text).expect("response must be valid json");
        let op: i64 = json
            .value()
            .to_member("op")
            .expect("op access must succeed")
            .required()
            .expect("op must exist")
            .try_into()
            .expect("op must be i64");
        let negotiated_rpc_version: u32 = json
            .value()
            .to_path_member(&["d", "negotiatedRpcVersion"])
            .expect("negotiatedRpcVersion access must succeed")
            .required()
            .expect("negotiatedRpcVersion must exist")
            .try_into()
            .expect("negotiatedRpcVersion must be u32");
        (op, negotiated_rpc_version)
    }

    fn parse_event_type_and_intent(text: &str) -> (i64, String, u32) {
        let json = nojson::RawJson::parse(text).expect("event must be valid json");
        let op: i64 = json
            .value()
            .to_member("op")
            .expect("op access must succeed")
            .required()
            .expect("op must exist")
            .try_into()
            .expect("op must be i64");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .expect("eventType access must succeed")
            .required()
            .expect("eventType must exist")
            .try_into()
            .expect("eventType must be string");
        let event_intent: u32 = json
            .value()
            .to_path_member(&["d", "eventIntent"])
            .expect("eventIntent access must succeed")
            .required()
            .expect("eventIntent must exist")
            .try_into()
            .expect("eventIntent must be u32");
        (op, event_type, event_intent)
    }

    fn parse_request_batch_results(text: &str) -> Vec<(String, bool, i64)> {
        let json = nojson::RawJson::parse(text).expect("response must be valid json");
        let mut results = json
            .value()
            .to_path_member(&["d", "results"])
            .expect("results access must succeed")
            .required()
            .expect("results must exist")
            .to_array()
            .expect("results must be array");
        results
            .by_ref()
            .map(|result| {
                let request_type: String = result
                    .to_member("requestType")
                    .expect("requestType access must succeed")
                    .required()
                    .expect("requestType must exist")
                    .try_into()
                    .expect("requestType must be string");
                let request_status = result
                    .to_member("requestStatus")
                    .expect("requestStatus access must succeed")
                    .required()
                    .expect("requestStatus must exist");
                let success: bool = request_status
                    .to_member("result")
                    .expect("result access must succeed")
                    .required()
                    .expect("result must exist")
                    .try_into()
                    .expect("result must be bool");
                let code: i64 = request_status
                    .to_member("code")
                    .expect("code access must succeed")
                    .required()
                    .expect("code must exist")
                    .try_into()
                    .expect("code must be i64");
                (request_type, success, code)
            })
            .collect()
    }

    #[test]
    fn on_connected_returns_hello_message_action() {
        let session = ObswsSession::new(None, input_registry(), None);
        let action = session.on_connected();
        let SessionAction::SendText { text, message_name } = action else {
            panic!("must be SendText");
        };
        assert_eq!(message_name, "hello message");
        assert!(text.contains("\"op\":0"));
    }

    #[tokio::test]
    async fn on_request_before_identify_returns_close_action() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let action = session
            .handle_request(RequestMessage {
                request_id: Some("req-1".to_owned()),
                request_type: Some("GetVersion".to_owned()),
                request_data: None,
            })
            .await;
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_NOT_IDENTIFIED);
        assert_eq!(reason, "identify is required");
    }

    #[tokio::test]
    async fn duplicate_identify_returns_already_identified_close() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let first = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await;
        assert!(first.is_ok());

        let second = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await;
        let action = second.expect("second identify must return action");
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_ALREADY_IDENTIFIED);
        assert_eq!(reason, "already identified");
    }

    #[tokio::test]
    async fn reidentify_before_identify_returns_not_identified_close() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let action = session
            .on_text_message(r#"{"op":3,"d":{}}"#)
            .await
            .expect("reidentify must be parsed");
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_NOT_IDENTIFIED);
        assert_eq!(reason, "identify is required");
    }

    #[tokio::test]
    async fn reidentify_after_identify_returns_identified_message() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let action = session
            .on_text_message(r#"{"op":3,"d":{"eventSubscriptions":1023}}"#)
            .await
            .expect("reidentify must be parsed");
        let SessionAction::SendText { text, message_name } = action else {
            panic!("must be SendText");
        };
        assert_eq!(message_name, "identified message");
        let (op, negotiated_rpc_version) = parse_identified_message(&text);
        assert_eq!(op, 2);
        assert_eq!(negotiated_rpc_version, 1);
    }

    #[tokio::test]
    async fn identify_with_event_subscriptions_updates_session_state() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":64}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(action, SessionAction::SendText { .. }));
        assert_eq!(session.event_subscriptions, OBSWS_EVENT_SUB_OUTPUTS);
    }

    #[tokio::test]
    async fn reidentify_updates_event_subscriptions_when_specified() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":1}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));
        assert_eq!(session.event_subscriptions, 1);

        let reidentify_action = session
            .on_text_message(r#"{"op":3,"d":{"eventSubscriptions":64}}"#)
            .await
            .expect("reidentify must succeed");
        assert!(matches!(reidentify_action, SessionAction::SendText { .. }));
        assert_eq!(session.event_subscriptions, OBSWS_EVENT_SUB_OUTPUTS);
    }

    #[tokio::test]
    async fn reidentify_without_event_subscriptions_keeps_previous_value() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":64}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let reidentify_action = session
            .on_text_message(r#"{"op":3,"d":{}}"#)
            .await
            .expect("reidentify must succeed");
        assert!(matches!(reidentify_action, SessionAction::SendText { .. }));
        assert_eq!(session.event_subscriptions, OBSWS_EVENT_SUB_OUTPUTS);
    }

    #[tokio::test]
    async fn create_scene_with_scene_subscription_returns_scene_created_event() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
            .expect("requestData must be valid json");
        let action = session
            .handle_request(RequestMessage {
                request_id: Some("req-create-scene".to_owned()),
                request_type: Some("CreateScene".to_owned()),
                request_data: Some(request_data),
            })
            .await;
        let SessionAction::SendTexts { messages } = action else {
            panic!("must be SendTexts");
        };
        assert_eq!(messages.len(), 2);
        let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
        assert_eq!(event_type, "SceneCreated");
        assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENES);
    }

    #[tokio::test]
    async fn set_current_program_scene_to_same_scene_returns_response_only() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene"}"#)
            .expect("requestData must be valid json");
        let action = session
            .handle_request(RequestMessage {
                request_id: Some("req-set-scene-same".to_owned()),
                request_type: Some("SetCurrentProgramScene".to_owned()),
                request_data: Some(request_data),
            })
            .await;
        assert!(matches!(action, SessionAction::SendText { .. }));
    }

    #[tokio::test]
    async fn remove_current_scene_with_scene_subscription_sends_scene_and_program_events() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let create_request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
            .expect("requestData must be valid json");
        let create_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-create-scene".to_owned()),
                request_type: Some("CreateScene".to_owned()),
                request_data: Some(create_request_data),
            })
            .await;
        assert!(matches!(create_action, SessionAction::SendTexts { .. }));

        let set_scene_request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
            .expect("requestData must be valid json");
        let set_scene_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-set-scene".to_owned()),
                request_type: Some("SetCurrentProgramScene".to_owned()),
                request_data: Some(set_scene_request_data),
            })
            .await;
        assert!(matches!(set_scene_action, SessionAction::SendTexts { .. }));

        let remove_request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
            .expect("requestData must be valid json");
        let remove_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-remove-scene".to_owned()),
                request_type: Some("RemoveScene".to_owned()),
                request_data: Some(remove_request_data),
            })
            .await;
        let SessionAction::SendTexts { messages } = remove_action else {
            panic!("must be SendTexts");
        };
        assert_eq!(messages.len(), 3);
        let (_, event_type_1, event_intent_1) = parse_event_type_and_intent(&messages[1].0);
        let (_, event_type_2, event_intent_2) = parse_event_type_and_intent(&messages[2].0);
        assert_eq!(event_type_1, "SceneRemoved");
        assert_eq!(event_intent_1, OBSWS_EVENT_SUB_SCENES);
        assert_eq!(event_type_2, "CurrentProgramSceneChanged");
        assert_eq!(event_intent_2, OBSWS_EVENT_SUB_SCENES);
    }

    #[tokio::test]
    async fn create_and_remove_input_with_input_subscription_send_input_events() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":8}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let create_request_data = nojson::RawJsonOwned::parse(
            r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
        )
        .expect("requestData must be valid json");
        let create_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-create-input".to_owned()),
                request_type: Some("CreateInput".to_owned()),
                request_data: Some(create_request_data),
            })
            .await;
        let SessionAction::SendTexts { messages } = create_action else {
            panic!("must be SendTexts");
        };
        let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
        assert_eq!(event_type, "InputCreated");
        assert_eq!(event_intent, OBSWS_EVENT_SUB_INPUTS);

        let remove_request_data = nojson::RawJsonOwned::parse(r#"{"inputName":"camera-1"}"#)
            .expect("requestData must be valid json");
        let remove_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-remove-input".to_owned()),
                request_type: Some("RemoveInput".to_owned()),
                request_data: Some(remove_request_data),
            })
            .await;
        let SessionAction::SendTexts { messages } = remove_action else {
            panic!("must be SendTexts");
        };
        let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
        assert_eq!(event_type, "InputRemoved");
        assert_eq!(event_intent, OBSWS_EVENT_SUB_INPUTS);
    }

    #[tokio::test]
    async fn set_scene_item_enabled_with_scene_subscription_sends_event_when_changed() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let create_request_data = nojson::RawJsonOwned::parse(
            r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
        )
        .expect("requestData must be valid json");
        let create_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-create-input".to_owned()),
                request_type: Some("CreateInput".to_owned()),
                request_data: Some(create_request_data),
            })
            .await;
        assert!(matches!(create_action, SessionAction::SendText { .. }));

        let get_scene_item_id_request_data =
            nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene","sourceName":"camera-1"}"#)
                .expect("requestData must be valid json");
        let get_scene_item_id_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-get-scene-item-id".to_owned()),
                request_type: Some("GetSceneItemId".to_owned()),
                request_data: Some(get_scene_item_id_request_data),
            })
            .await;
        let SessionAction::SendText { text, .. } = get_scene_item_id_action else {
            panic!("must be SendText");
        };
        let scene_item_id = parse_response_scene_item_id(&text);

        let set_request_data = nojson::RawJsonOwned::parse(&format!(
            r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemEnabled":false}}"#,
            scene_item_id
        ))
        .expect("requestData must be valid json");
        let set_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-set-scene-item-enabled".to_owned()),
                request_type: Some("SetSceneItemEnabled".to_owned()),
                request_data: Some(set_request_data),
            })
            .await;
        let SessionAction::SendTexts { messages } = set_action else {
            panic!("must be SendTexts");
        };
        assert_eq!(messages.len(), 2);
        let (_, event_type, event_intent) = parse_event_type_and_intent(&messages[1].0);
        assert_eq!(event_type, "SceneItemEnableStateChanged");
        assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENES);
    }

    #[tokio::test]
    async fn set_scene_item_enabled_with_same_value_returns_response_only() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":4}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let create_request_data = nojson::RawJsonOwned::parse(
            r#"{"sceneName":"Scene","inputName":"camera-1","inputKind":"image_source","inputSettings":{},"sceneItemEnabled":true}"#,
        )
        .expect("requestData must be valid json");
        let create_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-create-input".to_owned()),
                request_type: Some("CreateInput".to_owned()),
                request_data: Some(create_request_data),
            })
            .await;
        assert!(matches!(create_action, SessionAction::SendText { .. }));

        let get_scene_item_id_request_data =
            nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene","sourceName":"camera-1"}"#)
                .expect("requestData must be valid json");
        let get_scene_item_id_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-get-scene-item-id".to_owned()),
                request_type: Some("GetSceneItemId".to_owned()),
                request_data: Some(get_scene_item_id_request_data),
            })
            .await;
        let SessionAction::SendText { text, .. } = get_scene_item_id_action else {
            panic!("must be SendText");
        };
        let scene_item_id = parse_response_scene_item_id(&text);

        let set_request_data = nojson::RawJsonOwned::parse(&format!(
            r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemEnabled":true}}"#,
            scene_item_id
        ))
        .expect("requestData must be valid json");
        let set_action = session
            .handle_request(RequestMessage {
                request_id: Some("req-set-scene-item-enabled-same".to_owned()),
                request_type: Some("SetSceneItemEnabled".to_owned()),
                request_data: Some(set_request_data),
            })
            .await;
        assert!(matches!(set_action, SessionAction::SendText { .. }));
    }

    #[tokio::test]
    async fn unsupported_rpc_version_returns_close_action() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":2}}"#)
            .await
            .expect("identify must be parsed");
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION);
        assert_eq!(reason, "unsupported rpc version");
    }

    #[tokio::test]
    async fn invalid_authentication_returns_close_action() {
        let auth = ObswsAuthentication {
            salt: "test-salt".to_owned(),
            challenge: "test-challenge".to_owned(),
            expected_response: build_authentication_response(
                "test-password",
                "test-salt",
                "test-challenge",
            ),
        };
        let mut session = ObswsSession::new(Some(auth), input_registry(), None);
        let action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"authentication":"invalid"}}"#)
            .await
            .expect("identify must be parsed");
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_AUTHENTICATION_FAILED);
        assert_eq!(reason, "authentication failed");
    }

    #[tokio::test]
    async fn stop_record_when_inactive_returns_error_response() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let action = session
            .handle_request(RequestMessage {
                request_id: Some("req-stop-record".to_owned()),
                request_type: Some("StopRecord".to_owned()),
                request_data: None,
            })
            .await;
        let SessionAction::SendText { text, .. } = action else {
            panic!("must be SendText");
        };
        let (result, code) = parse_request_status(&text);
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_OUTPUT_NOT_RUNNING);
    }

    #[tokio::test]
    async fn start_record_without_image_input_returns_error_response() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let action = session
            .handle_request(RequestMessage {
                request_id: Some("req-start-record".to_owned()),
                request_type: Some("StartRecord".to_owned()),
                request_data: None,
            })
            .await;
        let SessionAction::SendText { text, .. } = action else {
            panic!("must be SendText");
        };
        let (result, code) = parse_request_status(&text);
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
    }

    #[tokio::test]
    async fn toggle_stream_without_image_input_returns_toggle_request_type_error() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let action = session
            .handle_request(RequestMessage {
                request_id: Some("req-toggle-stream".to_owned()),
                request_type: Some("ToggleStream".to_owned()),
                request_data: None,
            })
            .await;
        let SessionAction::SendText { text, .. } = action else {
            panic!("must be SendText");
        };
        let (result, code) = parse_request_status(&text);
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
        assert_eq!(parse_request_type(&text), "ToggleStream");
    }

    #[tokio::test]
    async fn toggle_record_without_image_input_returns_toggle_request_type_error() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let action = session
            .handle_request(RequestMessage {
                request_id: Some("req-toggle-record".to_owned()),
                request_type: Some("ToggleRecord".to_owned()),
                request_data: None,
            })
            .await;
        let SessionAction::SendText { text, .. } = action else {
            panic!("must be SendText");
        };
        let (result, code) = parse_request_status(&text);
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
        assert_eq!(parse_request_type(&text), "ToggleRecord");
    }

    #[tokio::test]
    async fn request_batch_with_halt_on_failure_stops_after_first_failure() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let action = session
            .on_text_message(
                r#"{"op":8,"d":{"requestId":"batch-1","haltOnFailure":true,"executionType":0,"requests":[{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"SetCurrentProgramScene","requestData":{"sceneName":"Scene B"}}]}}"#,
            )
            .await
            .expect("request batch must be parsed");
        let SessionAction::SendText { text, .. } = action else {
            panic!("must be SendText");
        };
        let results = parse_request_batch_results(&text);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "CreateScene");
        assert!(results[0].1);
        assert_eq!(results[1].0, "CreateScene");
        assert!(!results[1].1);
    }

    #[tokio::test]
    async fn request_batch_without_halt_on_failure_continues_after_failure() {
        let mut session = ObswsSession::new(None, input_registry(), None);
        let identify_action = session
            .on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#)
            .await
            .expect("identify must succeed");
        assert!(matches!(identify_action, SessionAction::SendText { .. }));

        let action = session
            .on_text_message(
                r#"{"op":8,"d":{"requestId":"batch-2","haltOnFailure":false,"executionType":0,"requests":[{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"SetCurrentProgramScene","requestData":{"sceneName":"Scene B"}}]}}"#,
            )
            .await
            .expect("request batch must be parsed");
        let SessionAction::SendText { text, .. } = action else {
            panic!("must be SendText");
        };
        let results = parse_request_batch_results(&text);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, "CreateScene");
        assert!(results[0].1);
        assert_eq!(results[1].0, "CreateScene");
        assert!(!results[1].1);
        assert_eq!(results[2].0, "SetCurrentProgramScene");
        assert!(results[2].1);
    }
}
