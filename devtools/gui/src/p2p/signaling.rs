//! HTTP Bootstrap と DataChannel シグナリングのメッセージプロトコル。
//!
//! `devtools/src/p2p/signaling.ts` の Rust 移植。

use nojson::{JsonParseError, RawJsonValue};

use super::types::{ClientMessage, CloseCode, ServerMessage};

/// シグナリングプロトコルのパースエラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalingError(pub String);

impl std::fmt::Display for SignalingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SignalingError {}

fn required_member<'text, 'raw>(
    value: RawJsonValue<'text, 'raw>,
    name: &str,
    missing_message: &str,
) -> Result<String, SignalingError> {
    let member = value
        .to_member(name)
        .and_then(|member| member.required())
        .map_err(|_: JsonParseError| SignalingError(missing_message.to_owned()))?;
    member
        .try_into()
        .map_err(|_: JsonParseError| SignalingError(missing_message.to_owned()))
}

/// サーバーメッセージをパースする。
pub fn parse_server_message(raw: &str) -> Result<ServerMessage, SignalingError> {
    let parsed = nojson::RawJson::parse(raw)
        .map_err(|_| SignalingError("failed to parse server message: invalid JSON".to_owned()))?;
    let value = parsed.value();

    // type フィールドの存在確認 (文字列でない場合もエラーにする)
    let type_value = match value.to_member("type") {
        Ok(member) => member.optional(),
        Err(_) => None,
    };
    let Some(type_value) = type_value else {
        return Err(SignalingError(
            "failed to parse server message: missing type field".to_owned(),
        ));
    };
    let msg_type: String = type_value.try_into().map_err(|_| {
        SignalingError("failed to parse server message: missing type field".to_owned())
    })?;

    match msg_type.as_str() {
        "offer" => {
            let sdp: String = required_member(value, "sdp", "missing sdp field in offer message")?;
            Ok(ServerMessage::Offer { sdp })
        }
        "close" => {
            let code_value: String = value
                .to_member("code")
                .and_then(|member| member.required())
                .and_then(|v| v.try_into())
                .map_err(|_| SignalingError("missing code field in close message".to_owned()))?;
            let code = CloseCode::parse(&code_value)
                .ok_or_else(|| SignalingError(format!("unknown close code: {}", code_value)))?;
            let reason: String =
                required_member(value, "reason", "missing reason field in close message")?;
            Ok(ServerMessage::Close(super::types::CloseMessage {
                code,
                reason,
            }))
        }
        other => Err(SignalingError(format!(
            "unknown server message type: {}",
            other
        ))),
    }
}

/// クライアントメッセージをシリアライズする。
pub fn serialize_client_message(message: &ClientMessage) -> String {
    match message {
        ClientMessage::Answer { sdp } => nojson::object(|f| {
            f.member("type", "answer")?;
            f.member("sdp", sdp.as_str())
        })
        .to_string(),
        ClientMessage::Disconnect => nojson::object(|f| f.member("type", "disconnect")).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ブラウザ版 signaling.test.ts のテストを移植したもの

    #[test]
    fn parse_server_message_parses_valid_offer() {
        let result = parse_server_message(r#"{"type":"offer","sdp":"v=0\r\n"}"#)
            .expect("パースに失敗しないこと");
        assert_eq!(
            result,
            ServerMessage::Offer {
                sdp: "v=0\r\n".to_owned()
            }
        );
    }

    #[test]
    fn parse_server_message_parses_valid_close() {
        let result =
            parse_server_message(r#"{"type":"close","code":"timeout","reason":"timed out"}"#)
                .expect("パースに失敗しないこと");
        assert_eq!(
            result,
            ServerMessage::Close(super::super::types::CloseMessage {
                code: CloseCode::Timeout,
                reason: "timed out".to_owned(),
            })
        );
    }

    #[test]
    fn parse_server_message_rejects_invalid_json() {
        let error = parse_server_message("not json").expect_err("エラーになること");
        assert_eq!(error.0, "failed to parse server message: invalid JSON");
    }

    #[test]
    fn parse_server_message_rejects_missing_type() {
        let error = parse_server_message(r#"{"sdp":"v=0\r\n"}"#).expect_err("エラーになること");
        assert_eq!(
            error.0,
            "failed to parse server message: missing type field"
        );
    }

    #[test]
    fn parse_server_message_rejects_non_string_type() {
        let error = parse_server_message(r#"{"type":42}"#).expect_err("エラーになること");
        assert_eq!(
            error.0,
            "failed to parse server message: missing type field"
        );
    }

    #[test]
    fn parse_server_message_rejects_unknown_type() {
        let error = parse_server_message(r#"{"type":"unknown"}"#).expect_err("エラーになること");
        assert_eq!(error.0, "unknown server message type: unknown");
    }

    #[test]
    fn parse_server_message_rejects_offer_without_sdp() {
        let error = parse_server_message(r#"{"type":"offer"}"#).expect_err("エラーになること");
        assert_eq!(error.0, "missing sdp field in offer message");
    }

    #[test]
    fn parse_server_message_rejects_offer_with_non_string_sdp() {
        let error =
            parse_server_message(r#"{"type":"offer","sdp":42}"#).expect_err("エラーになること");
        assert_eq!(error.0, "missing sdp field in offer message");
    }

    #[test]
    fn parse_server_message_rejects_close_without_code() {
        let error = parse_server_message(r#"{"type":"close","reason":"test"}"#)
            .expect_err("エラーになること");
        assert_eq!(error.0, "missing code field in close message");
    }

    #[test]
    fn parse_server_message_rejects_close_without_reason() {
        let error = parse_server_message(r#"{"type":"close","code":"timeout"}"#)
            .expect_err("エラーになること");
        assert_eq!(error.0, "missing reason field in close message");
    }

    #[test]
    fn parse_server_message_rejects_close_with_non_string_reason() {
        let error = parse_server_message(r#"{"type":"close","code":"timeout","reason":42}"#)
            .expect_err("エラーになること");
        assert_eq!(error.0, "missing reason field in close message");
    }

    #[test]
    fn parse_server_message_rejects_invalid_close_code() {
        let error = parse_server_message(r#"{"type":"close","code":"invalid","reason":"test"}"#)
            .expect_err("エラーになること");
        assert_eq!(error.0, "unknown close code: invalid");
    }

    #[test]
    fn serialize_client_message_serializes_answer() {
        let result = serialize_client_message(&ClientMessage::Answer {
            sdp: "v=0\r\n".to_owned(),
        });
        assert_eq!(result, r#"{"type":"answer","sdp":"v=0\r\n"}"#);
    }

    #[test]
    fn serialize_client_message_serializes_disconnect() {
        let result = serialize_client_message(&ClientMessage::Disconnect);
        assert_eq!(result, r#"{"type":"disconnect"}"#);
    }
}
