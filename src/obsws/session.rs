use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use shiguredo_websocket::CloseCode;
use tokio::sync::RwLock;

use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::{
    ActivateRecordError, ActivateRtmpOutboundError, ActivateStreamError, ObswsInputRegistry,
    ObswsRecordRun, ObswsRecordTrackRun, ObswsRtmpOutboundRun, ObswsStreamRun,
};
use crate::obsws_message::{ClientMessage, ObswsSessionStats, RequestBatchMessage};
use crate::obsws_protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, OBSWS_EVENT_SUB_ALL, OBSWS_EVENT_SUB_GENERAL,
    OBSWS_EVENT_SUB_INPUTS, OBSWS_EVENT_SUB_OUTPUTS, OBSWS_EVENT_SUB_SCENE_ITEMS,
    OBSWS_EVENT_SUB_SCENES, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_MISSING_REQUEST_DATA, REQUEST_STATUS_MISSING_REQUEST_FIELD,
    REQUEST_STATUS_MISSING_REQUEST_TYPE, REQUEST_STATUS_OUTPUT_NOT_RUNNING,
    REQUEST_STATUS_OUTPUT_RUNNING, REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
    REQUEST_STATUS_RESOURCE_NOT_FOUND, REQUEST_STATUS_STREAM_NOT_RUNNING,
    REQUEST_STATUS_STREAM_RUNNING,
};

mod input;
mod output;
mod scene;
mod scene_item;
#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;

pub enum SessionAction {
    SendTexts {
        messages: Vec<(nojson::RawJsonOwned, &'static str)>,
    },
    SendText {
        text: nojson::RawJsonOwned,
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
    response_text: nojson::RawJsonOwned,
    success: bool,
    output_path: Option<String>,
}

impl RequestOutcome {
    fn success(response_text: nojson::RawJsonOwned, output_path: Option<String>) -> Self {
        Self {
            response_text,
            success: true,
            output_path,
        }
    }

    fn failure(response_text: nojson::RawJsonOwned, output_path: Option<String>) -> Self {
        Self {
            response_text,
            success: false,
            output_path,
        }
    }
}

struct RequestExecutionResult {
    response_text: nojson::RawJsonOwned,
    batch_result: crate::obsws_response_builder::RequestBatchResult,
    events: Vec<nojson::RawJsonOwned>,
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
    ) -> nojson::RawJsonOwned {
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
        self.event_subscriptions = identify.event_subscriptions.unwrap_or(OBSWS_EVENT_SUB_ALL);
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

        // OBS 互換: RequestBatch のバリデーションエラーは op:7 レスポンスではなく
        // WebSocket close で返す。OBS 本体と同じ挙動にすることで、
        // OBS 向けクライアントの復旧ロジックとの互換性を確保する。
        let request_id = request_batch.request_id.unwrap_or_default();
        if request_id.is_empty() {
            return SessionAction::Close {
                code: CloseCode::INVALID_PAYLOAD,
                reason: "missing required requestId field in request batch",
                close_error_context: "failed to close websocket for invalid request batch",
            };
        }

        // OBS 互換: 未指定時は SerialRealtime (0) として扱う。
        // hisui は SerialRealtime のみ対応し、それ以外は拒否する。
        let execution_type = request_batch.execution_type.unwrap_or(0);
        if execution_type != 0 {
            return SessionAction::Close {
                code: CloseCode::INVALID_PAYLOAD,
                reason: "unsupported executionType field in request batch",
                close_error_context: "failed to close websocket for invalid request batch",
            };
        }

        let Some(requests) = request_batch.requests else {
            return SessionAction::Close {
                code: CloseCode::INVALID_PAYLOAD,
                reason: "missing required requests field in request batch",
                close_error_context: "failed to close websocket for invalid request batch",
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
            return self.handle_start_stream_request(&request_id).await;
        }
        if request_type == "StopStream" {
            return self.handle_stop_stream_request(&request_id).await;
        }
        if request_type == "ToggleStream" {
            return self.handle_toggle_stream_request(&request_id).await;
        }
        if request_type == "StartRecord" {
            return self.handle_start_record_request(&request_id).await;
        }
        if request_type == "StopRecord" {
            return self.handle_stop_record_request(&request_id).await;
        }
        if request_type == "ToggleRecord" {
            return self.handle_toggle_record_request(&request_id).await;
        }
        if request_type == "StartOutput" {
            return self
                .handle_start_output_request(&request_id, request.request_data.as_ref())
                .await;
        }
        if request_type == "StopOutput" {
            return self
                .handle_stop_output_request(&request_id, request.request_data.as_ref())
                .await;
        }
        if request_type == "ToggleOutput" {
            return self
                .handle_toggle_output_request(&request_id, request.request_data.as_ref())
                .await;
        }
        if request_type == "BroadcastCustomEvent" {
            let action = self
                .handle_broadcast_custom_event_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
        }
        if request_type == "Sleep" {
            let action = self
                .handle_sleep_request(&request_id, request.request_data.as_ref())
                .await;
            return Self::build_execution_from_action(action);
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
        let response = crate::obsws_message::handle_request_message_with_pipeline_handle(
            request,
            &self.stats,
            &mut input_registry,
            self.pipeline_handle.as_ref(),
        );
        Self::build_execution_from_response_text(response.message, Vec::new())
    }

    fn build_execution_from_outcome(
        outcome: RequestOutcome,
        events: Vec<nojson::RawJsonOwned>,
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
                request_id: request_id.to_owned(),
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
        response_text: nojson::RawJsonOwned,
        events: Vec<nojson::RawJsonOwned>,
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
    ) -> crate::Result<nojson::RawJsonOwned> {
        // 失敗時は outcome.response_text に正しい request_type でエラーが構築済み
        if !outcome.success {
            return Ok(outcome.response_text.clone());
        }

        match toggle_request_type {
            "ToggleStream" => Ok(crate::obsws_response_builder::build_toggle_stream_response(
                request_id,
                output_active_on_success,
            )),
            "ToggleRecord" => Ok(crate::obsws_response_builder::build_toggle_record_response(
                request_id,
                output_active_on_success,
            )),
            _ => Err(crate::Error::new("unknown toggle request type")),
        }
    }

    fn build_output_response_from_outcome(
        request_type: &str,
        request_id: &str,
        output_active_on_success: bool,
        outcome: &RequestOutcome,
    ) -> nojson::RawJsonOwned {
        // 失敗時は outcome.response_text に正しい request_type でエラーが構築済み
        if !outcome.success {
            return outcome.response_text.clone();
        }

        match request_type {
            "StartOutput" => crate::obsws_response_builder::build_start_output_response(request_id),
            "ToggleOutput" => crate::obsws_response_builder::build_toggle_output_response(
                request_id,
                output_active_on_success,
            ),
            "StopOutput" => crate::obsws_response_builder::build_stop_output_response(request_id),
            _ => unreachable!("BUG: unsupported output request type: {request_type}"),
        }
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
                REQUEST_STATUS_MISSING_REQUEST_DATA,
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

    async fn handle_broadcast_custom_event_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let Some(request_data) = request_data else {
            return Self::build_missing_request_data_error_action(
                "BroadcastCustomEvent",
                request_id,
            );
        };
        let event_data = match Self::parse_custom_event_request_data(request_data) {
            Ok(event_data) => event_data,
            Err(error) => {
                return Self::build_parse_error_action("BroadcastCustomEvent", request_id, &error);
            }
        };

        let response_text =
            crate::obsws_response_builder::build_broadcast_custom_event_response(request_id);
        if !self.is_event_subscription_enabled(OBSWS_EVENT_SUB_GENERAL) {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request response message",
            };
        }

        let event_text = crate::obsws_response_builder::build_custom_event(&event_data);
        SessionAction::SendTexts {
            messages: vec![
                (response_text, "request response message"),
                (event_text, "event message"),
            ],
        }
    }

    async fn handle_sleep_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> SessionAction {
        let Some(request_data) = request_data else {
            return Self::build_missing_request_data_error_action("Sleep", request_id);
        };
        let sleep_millis = match Self::parse_sleep_millis_request_field(request_data) {
            Ok(sleep_millis) => sleep_millis,
            Err(error) => return Self::build_parse_error_action("Sleep", request_id, &error),
        };
        tokio::time::sleep(Duration::from_millis(sleep_millis)).await;
        SessionAction::SendText {
            text: crate::obsws_response_builder::build_sleep_response(request_id),
            message_name: "request response message",
        }
    }

    fn parse_custom_event_request_data(
        request_data: &nojson::RawJsonOwned,
    ) -> Result<nojson::RawJsonOwned, nojson::JsonParseError> {
        let event_data = request_data.value().to_member("eventData")?.required()?;
        if event_data.kind() != nojson::JsonValueKind::Object {
            return Err(event_data.invalid("object is required"));
        }
        nojson::RawJsonOwned::try_from(event_data)
    }

    fn parse_sleep_millis_request_field(
        request_data: &nojson::RawJsonOwned,
    ) -> Result<u64, nojson::JsonParseError> {
        let raw_sleep_millis = request_data.value().to_member("sleepMillis")?.required()?;
        let sleep_millis: i64 = raw_sleep_millis.try_into()?;
        if sleep_millis < 0 {
            return Err(raw_sleep_millis.invalid("sleepMillis must be greater than or equal to 0"));
        }
        if sleep_millis > 50_000 {
            return Err(raw_sleep_millis.invalid("sleepMillis must be less than or equal to 50000"));
        }
        Ok(sleep_millis as u64)
    }
}
