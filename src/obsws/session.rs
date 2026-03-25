use shiguredo_websocket::CloseCode;

use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_coordinator::ObswsCoordinatorHandle;
use crate::obsws_message::{ClientMessage, ObswsSessionStats, RequestBatchMessage};
use crate::obsws_protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, OBSWS_EVENT_SUB_ALL,
};

pub mod output;
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

pub struct ObswsSession {
    state: ObswsSessionState,
    negotiated_rpc_version: Option<u32>,
    event_subscriptions: u32,
    auth: Option<ObswsAuthentication>,
    coordinator_handle: ObswsCoordinatorHandle,
    stats: ObswsSessionStats,
}

impl ObswsSession {
    pub fn new(
        auth: Option<ObswsAuthentication>,
        coordinator_handle: ObswsCoordinatorHandle,
    ) -> Self {
        Self {
            state: ObswsSessionState::AwaitingIdentify,
            negotiated_rpc_version: None,
            event_subscriptions: 0,
            auth,
            coordinator_handle,
            stats: ObswsSessionStats::default(),
        }
    }

    /// DataChannel 経由の接続用。認証なし・Identified 状態で初期化する。
    pub fn new_identified(coordinator_handle: ObswsCoordinatorHandle) -> Self {
        Self {
            state: ObswsSessionState::Identified,
            negotiated_rpc_version: Some(1),
            event_subscriptions: OBSWS_EVENT_SUB_ALL,
            auth: None,
            coordinator_handle,
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

    // -----------------------------------------------------------------------
    // リクエスト処理（coordinator に委譲）
    // -----------------------------------------------------------------------

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

        // Sleep は状態を変更しないため、coordinator を経由せず session 側で完結させる。
        // coordinator のキューを塞がないことで、他セッションの処理への影響を防ぐ。
        if request.request_type.as_deref() == Some("Sleep") {
            return self.handle_sleep(&request).await;
        }

        let result = match self
            .coordinator_handle
            .process_request(request, self.stats.clone())
            .await
        {
            Ok(result) => result,
            Err(_) => {
                return SessionAction::SendText {
                    text: crate::obsws_response_builder::build_request_response_error(
                        "",
                        "",
                        crate::obsws_protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Coordinator has terminated",
                    ),
                    message_name: "request response message",
                };
            }
        };

        // イベントを event_subscriptions でフィルタリングする
        let filtered_events: Vec<_> = result
            .events
            .into_iter()
            .filter(|e| (self.event_subscriptions & e.subscription_flag) != 0)
            .collect();

        if filtered_events.is_empty() {
            return SessionAction::SendText {
                text: result.response_text,
                message_name: "request response message",
            };
        }

        let mut messages = Vec::with_capacity(filtered_events.len() + 1);
        messages.push((result.response_text, "request response message"));
        messages.extend(
            filtered_events
                .into_iter()
                .map(|e| (e.text, "event message")),
        );
        SessionAction::SendTexts { messages }
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
        // WebSocket close で返す。
        let request_id = request_batch.request_id.unwrap_or_default();
        if request_id.is_empty() {
            return SessionAction::Close {
                code: CloseCode::INVALID_PAYLOAD,
                reason: "missing required requestId field in request batch",
                close_error_context: "failed to close websocket for invalid request batch",
            };
        }

        // OBS 互換: 未指定時は SerialRealtime (0) として扱う。
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

        // sub request の request_id を補完する
        let requests: Vec<_> = requests
            .into_iter()
            .enumerate()
            .map(|(index, r)| {
                let sub_request_id = r
                    .request_id
                    .clone()
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| format!("{request_id}:{index}"));
                crate::obsws_message::RequestMessage {
                    request_id: Some(sub_request_id),
                    request_type: r.request_type,
                    request_data: r.request_data,
                }
            })
            .collect();

        let batch_result = match self
            .coordinator_handle
            .process_request_batch(requests, self.stats.clone(), halt_on_failure)
            .await
        {
            Ok(result) => result,
            Err(_) => {
                return SessionAction::SendText {
                    text: crate::obsws_response_builder::build_request_response_error(
                        "RequestBatch",
                        &request_id,
                        crate::obsws_protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Coordinator has terminated",
                    ),
                    message_name: "request response message",
                };
            }
        };

        let response_text = crate::obsws_response_builder::build_request_batch_response(
            &request_id,
            &batch_result.results,
        );

        // イベントをフィルタリングする
        let filtered_events: Vec<_> = batch_result
            .events
            .into_iter()
            .filter(|e| (self.event_subscriptions & e.subscription_flag) != 0)
            .map(|e| (e.text, "event message"))
            .collect();

        if filtered_events.is_empty() {
            return SessionAction::SendText {
                text: response_text,
                message_name: "request batch response message",
            };
        }

        let mut messages = Vec::with_capacity(filtered_events.len() + 1);
        // RequestBatch では、RequestBatchResponse (op=9) を先に返し、その後に event を送信する。
        messages.push((response_text, "request batch response message"));
        messages.extend(filtered_events);
        SessionAction::SendTexts { messages }
    }

    // -----------------------------------------------------------------------
    // session-local ハンドラ（coordinator を経由しない）
    // -----------------------------------------------------------------------

    /// Sleep は状態を変更しないため session 側で完結させる。
    async fn handle_sleep(&self, request: &crate::obsws_message::RequestMessage) -> SessionAction {
        let request_id = request.request_id.as_deref().unwrap_or_default();
        let Some(request_data) = request.request_data.as_ref() else {
            return SessionAction::SendText {
                text: crate::obsws_response_builder::build_request_response_error(
                    "Sleep",
                    request_id,
                    crate::obsws_protocol::REQUEST_STATUS_MISSING_REQUEST_DATA,
                    "Missing required requestData field",
                ),
                message_name: "request response message",
            };
        };
        let sleep_millis = match Self::parse_sleep_millis(request_data) {
            Ok(millis) => millis,
            Err(error) => {
                let code =
                    crate::obsws_response_builder::request_status_code_for_parse_error(&error);
                return SessionAction::SendText {
                    text: crate::obsws_response_builder::build_request_response_error(
                        "Sleep",
                        request_id,
                        code,
                        &error.to_string(),
                    ),
                    message_name: "request response message",
                };
            }
        };
        tokio::time::sleep(std::time::Duration::from_millis(sleep_millis)).await;
        SessionAction::SendText {
            text: crate::obsws_response_builder::build_sleep_response(request_id),
            message_name: "request response message",
        }
    }

    fn parse_sleep_millis(
        request_data: &nojson::RawJsonOwned,
    ) -> Result<u64, nojson::JsonParseError> {
        let raw = request_data.value().to_member("sleepMillis")?.required()?;
        let millis: i64 = raw.try_into()?;
        if millis < 0 {
            return Err(raw.invalid("sleepMillis must be greater than or equal to 0"));
        }
        if millis > 50_000 {
            return Err(raw.invalid("sleepMillis must be less than or equal to 50000"));
        }
        Ok(millis as u64)
    }

    // -----------------------------------------------------------------------
    // Identify / Reidentify
    // -----------------------------------------------------------------------

    fn handle_identify(
        &mut self,
        identify: crate::obsws_message::IdentifyMessage,
    ) -> SessionAction {
        if self.state != ObswsSessionState::AwaitingIdentify {
            return SessionAction::Close {
                code: OBSWS_CLOSE_ALREADY_IDENTIFIED,
                reason: "already identified",
                close_error_context: "failed to close websocket for already identified",
            };
        }

        let rpc_version = identify.rpc_version;
        if rpc_version != 1 {
            return SessionAction::Close {
                code: OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION,
                reason: "unsupported rpc version",
                close_error_context: "failed to close websocket for unsupported rpc version",
            };
        }

        if let Some(auth) = &self.auth
            && !auth.is_valid_response(identify.authentication.as_deref())
        {
            return SessionAction::Close {
                code: OBSWS_CLOSE_AUTHENTICATION_FAILED,
                reason: "authentication failed",
                close_error_context: "failed to close websocket for authentication failure",
            };
        }

        self.state = ObswsSessionState::Identified;
        self.negotiated_rpc_version = Some(rpc_version);
        self.event_subscriptions = identify.event_subscriptions.unwrap_or(OBSWS_EVENT_SUB_ALL);

        SessionAction::SendText {
            text: crate::obsws_message::build_identified_message(rpc_version),
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

        self.event_subscriptions = reidentify
            .event_subscriptions
            .unwrap_or(OBSWS_EVENT_SUB_ALL);

        SessionAction::SendText {
            text: crate::obsws_message::build_identified_message(
                self.negotiated_rpc_version.unwrap_or(1),
            ),
            message_name: "identified message",
        }
    }
}
