use std::sync::Arc;

use shiguredo_websocket::CloseCode;
use tokio::sync::RwLock;

use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::ObswsInputRegistry;
use crate::obsws_message_handler::{
    ClientMessage, ObswsSessionStats, build_hello_message, build_identified_message,
    handle_request_message, is_supported_rpc_version, parse_client_message,
};
use crate::obsws_protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION,
};

pub(crate) enum SessionAction {
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

pub(crate) struct ObswsSession {
    state: ObswsSessionState,
    auth: Option<ObswsAuthentication>,
    input_registry: Arc<RwLock<ObswsInputRegistry>>,
    stats: ObswsSessionStats,
}

impl ObswsSession {
    pub(crate) fn new(
        auth: Option<ObswsAuthentication>,
        input_registry: Arc<RwLock<ObswsInputRegistry>>,
    ) -> Self {
        Self {
            state: ObswsSessionState::AwaitingIdentify,
            auth,
            input_registry,
            stats: ObswsSessionStats::default(),
        }
    }

    pub(crate) fn stats_mut(&mut self) -> &mut ObswsSessionStats {
        &mut self.stats
    }

    pub(crate) fn on_connected(&self) -> SessionAction {
        SessionAction::SendText {
            text: build_hello_message(self.auth.as_ref()),
            message_name: "hello message",
        }
    }

    pub(crate) async fn on_text_message(&mut self, text: &str) -> crate::Result<SessionAction> {
        self.stats.incoming_messages = self.stats.incoming_messages.saturating_add(1);

        let message = parse_client_message(text)?;
        let action = match message {
            ClientMessage::Identify(identify) => self.handle_identify(identify),
            ClientMessage::Request(request) => self.handle_request(request).await,
        };
        Ok(action)
    }

    pub(crate) fn on_close_event(&self) -> SessionAction {
        SessionAction::Terminate
    }

    pub(crate) fn on_error_event(&self) -> SessionAction {
        SessionAction::Terminate
    }

    fn handle_identify(
        &mut self,
        identify: crate::obsws_message_handler::IdentifyMessage,
    ) -> SessionAction {
        if self.state == ObswsSessionState::Identified {
            return SessionAction::Close {
                code: OBSWS_CLOSE_ALREADY_IDENTIFIED,
                reason: "already identified",
                close_error_context: "failed to close websocket for duplicated identify",
            };
        }

        if !is_supported_rpc_version(identify.rpc_version) {
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
        SessionAction::SendText {
            text: build_identified_message(identify.rpc_version),
            message_name: "identified message",
        }
    }

    async fn handle_request(
        &mut self,
        request: crate::obsws_message_handler::RequestMessage,
    ) -> SessionAction {
        if self.state != ObswsSessionState::Identified {
            return SessionAction::Close {
                code: OBSWS_CLOSE_NOT_IDENTIFIED,
                reason: "identify is required",
                close_error_context: "failed to close websocket for unidentified request",
            };
        }

        let input_registry = self.input_registry.read().await;
        let response = handle_request_message(request, &self.stats, &input_registry);
        SessionAction::SendText {
            text: response.message,
            message_name: "request response message",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsws_auth::build_authentication_response;
    use crate::obsws_message_handler::RequestMessage;
    use crate::obsws_protocol::{
        OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED,
        OBSWS_CLOSE_NOT_IDENTIFIED, OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION,
    };
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn input_registry() -> Arc<RwLock<ObswsInputRegistry>> {
        Arc::new(RwLock::new(ObswsInputRegistry::new()))
    }

    #[test]
    fn on_connected_returns_hello_message_action() {
        let session = ObswsSession::new(None, input_registry());
        let action = session.on_connected();
        let SessionAction::SendText { text, message_name } = action else {
            panic!("must be SendText");
        };
        assert_eq!(message_name, "hello message");
        assert!(text.contains("\"op\":0"));
    }

    #[test]
    fn on_request_before_identify_returns_close_action() {
        let mut session = ObswsSession::new(None, input_registry());
        let action = session.handle_request(RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("GetVersion".to_owned()),
            request_data: None,
        });
        let action = tokio::runtime::Runtime::new()
            .expect("runtime init must succeed")
            .block_on(action);
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_NOT_IDENTIFIED);
        assert_eq!(reason, "identify is required");
    }

    #[tokio::test]
    async fn duplicate_identify_returns_already_identified_close() {
        let mut session = ObswsSession::new(None, input_registry());
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
    async fn unsupported_rpc_version_returns_close_action() {
        let mut session = ObswsSession::new(None, input_registry());
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
        let mut session = ObswsSession::new(Some(auth), input_registry());
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
}
