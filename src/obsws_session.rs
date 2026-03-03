use shiguredo_websocket::CloseCode;

use crate::obsws_auth::ObswsAuthentication;
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

pub(crate) struct ObswsSession {
    identified: bool,
    auth: Option<ObswsAuthentication>,
    stats: ObswsSessionStats,
}

impl ObswsSession {
    pub(crate) fn new(auth: Option<ObswsAuthentication>) -> Self {
        Self {
            identified: false,
            auth,
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

    pub(crate) fn on_text_message(&mut self, text: &str) -> crate::Result<SessionAction> {
        self.stats.incoming_messages = self.stats.incoming_messages.saturating_add(1);

        let message = parse_client_message(text)?;
        let action = match message {
            ClientMessage::Identify(identify) => self.handle_identify(identify),
            ClientMessage::Request(request) => self.handle_request(request),
        };
        Ok(action)
    }

    pub(crate) fn on_close_event(&mut self) -> SessionAction {
        SessionAction::Terminate
    }

    pub(crate) fn on_error_event(&mut self) -> SessionAction {
        SessionAction::Terminate
    }

    fn handle_identify(
        &mut self,
        identify: crate::obsws_message_handler::IdentifyMessage,
    ) -> SessionAction {
        if self.identified {
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

        self.identified = true;
        SessionAction::SendText {
            text: build_identified_message(identify.rpc_version),
            message_name: "identified message",
        }
    }

    fn handle_request(
        &mut self,
        request: crate::obsws_message_handler::RequestMessage,
    ) -> SessionAction {
        if !self.identified {
            return SessionAction::Close {
                code: OBSWS_CLOSE_NOT_IDENTIFIED,
                reason: "identify is required",
                close_error_context: "failed to close websocket for unidentified request",
            };
        }

        let response = handle_request_message(request, &self.stats);
        SessionAction::SendText {
            text: response.message,
            message_name: "request response message",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsws_message_handler::RequestMessage;
    use crate::obsws_protocol::{OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_NOT_IDENTIFIED};

    #[test]
    fn on_connected_returns_hello_message_action() {
        let session = ObswsSession::new(None);
        let action = session.on_connected();
        let SessionAction::SendText { text, message_name } = action else {
            panic!("must be SendText");
        };
        assert_eq!(message_name, "hello message");
        assert!(text.contains("\"op\":0"));
    }

    #[test]
    fn on_request_before_identify_returns_close_action() {
        let mut session = ObswsSession::new(None);
        let action = session.handle_request(RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("GetVersion".to_owned()),
        });
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_NOT_IDENTIFIED);
        assert_eq!(reason, "identify is required");
    }

    #[test]
    fn duplicate_identify_returns_already_identified_close() {
        let mut session = ObswsSession::new(None);
        let first = session.on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#);
        assert!(first.is_ok());

        let second = session.on_text_message(r#"{"op":1,"d":{"rpcVersion":1}}"#);
        let action = second.expect("second identify must return action");
        let SessionAction::Close { code, reason, .. } = action else {
            panic!("must be Close");
        };
        assert_eq!(code, OBSWS_CLOSE_ALREADY_IDENTIFIED);
        assert_eq!(reason, "already identified");
    }
}
