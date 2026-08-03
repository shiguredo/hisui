//! OBS WebSocket 5.x プロトコル定義。
//!
//! `devtools/src/obsdc/protocol.ts` の Rust 移植。
//! ブラウザ版と同じく、オペコード・メッセージ型・パース・シリアライズを提供する。

use nojson::{JsonParseError, RawJsonOwned, RawJsonValue};

/// OBS WebSocket 5.x のオペコード。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    /// サーバーが接続確立時に送信する
    Hello,
    /// クライアントが認証情報を送信する
    Identify,
    /// サーバーが認証完了を通知する
    Identified,
    /// クライアントがイベント購読設定を変更する
    Reidentify,
    /// サーバーがイベントを送信する
    Event,
    /// クライアントがリクエストを送信する
    Request,
    /// サーバーがリクエストへの応答を送信する
    RequestResponse,
    /// クライアントが複数リクエストを一括送信する
    RequestBatch,
    /// サーバーが複数リクエストへの応答を一括送信する
    RequestBatchResponse,
}

impl OpCode {
    pub fn from_int(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Hello),
            1 => Some(Self::Identify),
            2 => Some(Self::Identified),
            3 => Some(Self::Reidentify),
            5 => Some(Self::Event),
            6 => Some(Self::Request),
            7 => Some(Self::RequestResponse),
            8 => Some(Self::RequestBatch),
            9 => Some(Self::RequestBatchResponse),
            _ => None,
        }
    }

    pub fn to_int(self) -> i64 {
        match self {
            Self::Hello => 0,
            Self::Identify => 1,
            Self::Identified => 2,
            Self::Reidentify => 3,
            Self::Event => 5,
            Self::Request => 6,
            Self::RequestResponse => 7,
            Self::RequestBatch => 8,
            Self::RequestBatchResponse => 9,
        }
    }
}

/// イベント購読のビットフラグ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSubscription(pub u64);

impl EventSubscription {
    /// 全イベントを購読する (下位 12 ビット)
    pub const ALL: Self = Self((1 << 12) - 1);
    /// 高ボリュームイベント (InputVolumeMeters)
    pub const INPUT_VOLUME_METERS: Self = Self(1 << 16);
    /// 高ボリュームイベント (InputActiveStateChanged)
    pub const INPUT_ACTIVE_STATE_CHANGED: Self = Self(1 << 17);
    /// 高ボリュームイベント (InputShowStateChanged)
    pub const INPUT_SHOW_STATE_CHANGED: Self = Self(1 << 18);
    /// 高ボリュームイベント (SceneItemTransformChanged)
    pub const SCENE_ITEM_TRANSFORM_CHANGED: Self = Self(1 << 19);
}

/// 認証チャレンジ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticationChallenge {
    pub challenge: String,
    pub salt: String,
}

/// Hello メッセージのデータ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloData {
    pub obs_studio_version: String,
    pub obs_web_socket_version: String,
    pub rpc_version: u64,
    pub authentication: Option<AuthenticationChallenge>,
}

/// Identify メッセージのデータ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifyData {
    pub rpc_version: u64,
    pub authentication: Option<String>,
    pub event_subscriptions: Option<u64>,
}

/// Identified メッセージのデータ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifiedData {
    pub negotiated_rpc_version: u64,
}

/// Event メッセージのデータ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventData {
    pub event_type: String,
    pub event_intent: u64,
    pub event_data: Option<RawJsonOwned>,
}

/// Request メッセージのデータ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestData {
    pub request_type: String,
    pub request_id: String,
    pub request_data: Option<RawJsonOwned>,
}

/// リクエストの処理結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestStatus {
    pub result: bool,
    pub code: u64,
    pub comment: Option<String>,
}

/// RequestResponse メッセージのデータ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestResponseData {
    pub request_type: String,
    pub request_id: String,
    pub request_status: RequestStatus,
    pub response_data: Option<RawJsonOwned>,
}

/// サーバーから送信されるメッセージ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMessage {
    Hello(HelloData),
    Identified(IdentifiedData),
    Event(EventData),
    RequestResponse(RequestResponseData),
}

/// クライアントから送信されるメッセージ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    Identify(IdentifyData),
    Reidentify { event_subscriptions: Option<u64> },
    Request(RequestData),
}

/// プロトコルのパース・シリアライズエラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError(pub String);

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProtocolError {}

fn parse_object_member<'text, 'raw>(
    value: RawJsonValue<'text, 'raw>,
    name: &str,
) -> Result<RawJsonValue<'text, 'raw>, ProtocolError> {
    value
        .to_member(name)
        .and_then(|member| member.required())
        .map_err(|e| ProtocolError(format!("missing {} field: {}", name, e)))
}

fn parse_optional_string<'text, 'raw>(
    value: RawJsonValue<'text, 'raw>,
    name: &str,
) -> Result<Option<String>, ProtocolError> {
    match value.to_member(name) {
        Ok(member) => match member.optional() {
            Some(v) => v.try_into().map(Some).map_err(|e: JsonParseError| {
                ProtocolError(format!("invalid {} field: {}", name, e))
            }),
            None => Ok(None),
        },
        Err(_) => Ok(None),
    }
}

fn parse_optional_boolean<'text, 'raw>(
    value: RawJsonValue<'text, 'raw>,
    name: &str,
) -> Result<Option<bool>, ProtocolError> {
    match value.to_member(name) {
        Ok(member) => match member.optional() {
            Some(v) => v.try_into().map(Some).map_err(|e: JsonParseError| {
                ProtocolError(format!("invalid {} field: {}", name, e))
            }),
            None => Ok(None),
        },
        Err(_) => Ok(None),
    }
}

/// サーバーメッセージをパースする。
pub fn parse_server_message(raw: &str) -> Result<ServerMessage, ProtocolError> {
    let parsed = nojson::RawJson::parse(raw)
        .map_err(|e| ProtocolError(format!("failed to parse server message: {}", e)))?;
    let value = parsed.value();
    let op_value = parse_object_member(value, "op")?;
    let op: i64 = op_value
        .try_into()
        .map_err(|e: JsonParseError| ProtocolError(format!("invalid op field: {}", e)))?;
    let d_value = parse_object_member(value, "d")?;

    let op_code =
        OpCode::from_int(op).ok_or_else(|| ProtocolError(format!("unknown op: {}", op)))?;
    match op_code {
        OpCode::Hello => {
            let obs_studio_version: String = parse_object_member(d_value, "obsStudioVersion")?
                .try_into()
                .map_err(|e: JsonParseError| {
                    ProtocolError(format!("invalid obsStudioVersion: {}", e))
                })?;
            let obs_web_socket_version: String =
                parse_object_member(d_value, "obsWebSocketVersion")?
                    .try_into()
                    .map_err(|e: JsonParseError| {
                        ProtocolError(format!("invalid obsWebSocketVersion: {}", e))
                    })?;
            let rpc_version: i64 = parse_object_member(d_value, "rpcVersion")?
                .try_into()
                .map_err(|e: JsonParseError| ProtocolError(format!("invalid rpcVersion: {}", e)))?;
            let authentication = match d_value.to_member("authentication") {
                Ok(member) => match member.optional() {
                    Some(authentication) => {
                        let challenge: String = parse_object_member(authentication, "challenge")?
                            .try_into()
                            .map_err(|e: JsonParseError| {
                                ProtocolError(format!("invalid challenge: {}", e))
                            })?;
                        let salt: String = parse_object_member(authentication, "salt")?
                            .try_into()
                            .map_err(|e: JsonParseError| {
                                ProtocolError(format!("invalid salt: {}", e))
                            })?;
                        Some(AuthenticationChallenge { challenge, salt })
                    }
                    None => None,
                },
                Err(_) => None,
            };
            Ok(ServerMessage::Hello(HelloData {
                obs_studio_version,
                obs_web_socket_version,
                rpc_version: rpc_version as u64,
                authentication,
            }))
        }
        OpCode::Identified => {
            let negotiated_rpc_version: i64 = parse_object_member(d_value, "negotiatedRpcVersion")?
                .try_into()
                .map_err(|e: JsonParseError| {
                    ProtocolError(format!("invalid negotiatedRpcVersion: {}", e))
                })?;
            Ok(ServerMessage::Identified(IdentifiedData {
                negotiated_rpc_version: negotiated_rpc_version as u64,
            }))
        }
        OpCode::Event => {
            let event_type: String = parse_object_member(d_value, "eventType")?
                .try_into()
                .map_err(|e: JsonParseError| ProtocolError(format!("invalid eventType: {}", e)))?;
            let event_intent: i64 = parse_object_member(d_value, "eventIntent")?
                .try_into()
                .map_err(|e: JsonParseError| {
                    ProtocolError(format!("invalid eventIntent: {}", e))
                })?;
            let event_data: Option<RawJsonOwned> = match d_value.to_member("eventData") {
                Ok(member) => member
                    .optional()
                    .map(|v| v.try_into())
                    .transpose()
                    .map_err(|e: JsonParseError| {
                        ProtocolError(format!("invalid eventData: {}", e))
                    })?,
                Err(_) => None,
            };
            Ok(ServerMessage::Event(EventData {
                event_type,
                event_intent: event_intent as u64,
                event_data,
            }))
        }
        OpCode::RequestResponse => {
            let request_type: String = parse_object_member(d_value, "requestType")?
                .try_into()
                .map_err(|e: JsonParseError| {
                    ProtocolError(format!("invalid requestType: {}", e))
                })?;
            let request_id: String = parse_object_member(d_value, "requestId")?
                .try_into()
                .map_err(|e: JsonParseError| ProtocolError(format!("invalid requestId: {}", e)))?;
            let status_value = parse_object_member(d_value, "requestStatus")?;
            let result = parse_optional_boolean(status_value, "result")?
                .ok_or_else(|| ProtocolError("missing result field in requestStatus".to_owned()))?;
            let code: i64 = parse_object_member(status_value, "code")?
                .try_into()
                .map_err(|e: JsonParseError| ProtocolError(format!("invalid code: {}", e)))?;
            let comment = parse_optional_string(status_value, "comment")?;
            let response_data: Option<RawJsonOwned> = match d_value.to_member("responseData") {
                Ok(member) => member
                    .optional()
                    .map(|v| v.try_into())
                    .transpose()
                    .map_err(|e: JsonParseError| {
                        ProtocolError(format!("invalid responseData: {}", e))
                    })?,
                Err(_) => None,
            };
            Ok(ServerMessage::RequestResponse(RequestResponseData {
                request_type,
                request_id,
                request_status: RequestStatus {
                    result,
                    code: code as u64,
                    comment,
                },
                response_data,
            }))
        }
        _ => Err(ProtocolError("unexpected server message op".to_owned())),
    }
}

/// クライアントメッセージをシリアライズする。
pub fn serialize_client_message(message: &ClientMessage) -> String {
    match message {
        ClientMessage::Identify(data) => nojson::object(|f| {
            f.member("op", OpCode::Identify.to_int())?;
            f.member(
                "d",
                nojson::object(|f| {
                    f.member("rpcVersion", data.rpc_version)?;
                    if let Some(authentication) = &data.authentication {
                        f.member("authentication", authentication.as_str())?;
                    }
                    if let Some(event_subscriptions) = data.event_subscriptions {
                        f.member("eventSubscriptions", event_subscriptions)?;
                    }
                    Ok(())
                }),
            )
        })
        .to_string(),
        ClientMessage::Reidentify {
            event_subscriptions,
        } => nojson::object(|f| {
            f.member("op", OpCode::Reidentify.to_int())?;
            f.member(
                "d",
                nojson::object(|f| {
                    if let Some(event_subscriptions) = event_subscriptions {
                        f.member("eventSubscriptions", *event_subscriptions)?;
                    }
                    Ok(())
                }),
            )
        })
        .to_string(),
        ClientMessage::Request(data) => nojson::object(|f| {
            f.member("op", OpCode::Request.to_int())?;
            f.member(
                "d",
                nojson::object(|f| {
                    f.member("requestType", data.request_type.as_str())?;
                    f.member("requestId", data.request_id.as_str())?;
                    if let Some(request_data) = &data.request_data {
                        f.member("requestData", request_data.clone())?;
                    }
                    Ok(())
                }),
            )
        })
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ブラウザ版 protocol.test.ts のテストを移植したもの

    #[test]
    fn op_code_values_match_spec() {
        assert_eq!(OpCode::Hello.to_int(), 0);
        assert_eq!(OpCode::Identify.to_int(), 1);
        assert_eq!(OpCode::Identified.to_int(), 2);
        assert_eq!(OpCode::Reidentify.to_int(), 3);
        assert_eq!(OpCode::Event.to_int(), 5);
        assert_eq!(OpCode::Request.to_int(), 6);
        assert_eq!(OpCode::RequestResponse.to_int(), 7);
        assert_eq!(OpCode::RequestBatch.to_int(), 8);
        assert_eq!(OpCode::RequestBatchResponse.to_int(), 9);
    }

    #[test]
    fn event_subscription_all_has_lower_12_bits() {
        assert_eq!(EventSubscription::ALL.0, 4095);
    }

    #[test]
    fn event_subscription_high_volume_events_use_bit_16_and_above() {
        assert_eq!(EventSubscription::INPUT_VOLUME_METERS.0, 1 << 16);
        assert_eq!(EventSubscription::INPUT_ACTIVE_STATE_CHANGED.0, 1 << 17);
        assert_eq!(EventSubscription::INPUT_SHOW_STATE_CHANGED.0, 1 << 18);
        assert_eq!(EventSubscription::SCENE_ITEM_TRANSFORM_CHANGED.0, 1 << 19);
    }

    #[test]
    fn parse_server_message_parses_hello() {
        let raw = r#"{"op":0,"d":{"obsStudioVersion":"30.2.2","obsWebSocketVersion":"5.5.2","rpcVersion":1,"authentication":{"challenge":"abc123","salt":"def456"}}}"#;
        let message = parse_server_message(raw).expect("パースに失敗しないこと");
        match message {
            ServerMessage::Hello(hello) => {
                assert_eq!(hello.obs_studio_version, "30.2.2");
                assert_eq!(hello.obs_web_socket_version, "5.5.2");
                assert_eq!(hello.rpc_version, 1);
                let authentication = hello.authentication.expect("authentication があること");
                assert_eq!(authentication.challenge, "abc123");
                assert_eq!(authentication.salt, "def456");
            }
            other => panic!("Hello メッセージであること: {:?}", other),
        }
    }

    #[test]
    fn parse_server_message_parses_hello_without_authentication() {
        let raw = r#"{"op":0,"d":{"obsStudioVersion":"30.2.2","obsWebSocketVersion":"5.5.2","rpcVersion":1}}"#;
        let message = parse_server_message(raw).expect("パースに失敗しないこと");
        match message {
            ServerMessage::Hello(hello) => {
                assert!(hello.authentication.is_none(), "authentication がないこと");
            }
            other => panic!("Hello メッセージであること: {:?}", other),
        }
    }

    #[test]
    fn parse_server_message_parses_identified() {
        let raw = r#"{"op":2,"d":{"negotiatedRpcVersion":1}}"#;
        let message = parse_server_message(raw).expect("パースに失敗しないこと");
        match message {
            ServerMessage::Identified(identified) => {
                assert_eq!(identified.negotiated_rpc_version, 1);
            }
            other => panic!("Identified メッセージであること: {:?}", other),
        }
    }

    #[test]
    fn parse_server_message_parses_event() {
        let raw = r#"{"op":5,"d":{"eventType":"CurrentProgramSceneChanged","eventIntent":4,"eventData":{"sceneName":"Scene 1"}}}"#;
        let message = parse_server_message(raw).expect("パースに失敗しないこと");
        match message {
            ServerMessage::Event(event) => {
                assert_eq!(event.event_type, "CurrentProgramSceneChanged");
                assert_eq!(event.event_intent, 4);
                let event_data = event.event_data.expect("eventData があること");
                let scene_name: String = event_data
                    .value()
                    .to_member("sceneName")
                    .and_then(|member| member.required())
                    .and_then(|v| v.try_into())
                    .expect("sceneName があること");
                assert_eq!(scene_name, "Scene 1");
            }
            other => panic!("Event メッセージであること: {:?}", other),
        }
    }

    #[test]
    fn parse_server_message_parses_request_response() {
        let raw = r#"{"op":7,"d":{"requestType":"GetSceneList","requestId":"test-1","requestStatus":{"result":true,"code":100},"responseData":{"scenes":[]}}}"#;
        let message = parse_server_message(raw).expect("パースに失敗しないこと");
        match message {
            ServerMessage::RequestResponse(response) => {
                assert_eq!(response.request_type, "GetSceneList");
                assert_eq!(response.request_id, "test-1");
                assert!(response.request_status.result, "result が true であること");
                assert_eq!(response.request_status.code, 100);
            }
            other => panic!("RequestResponse メッセージであること: {:?}", other),
        }
    }

    #[test]
    fn parse_server_message_rejects_invalid_json() {
        let result = parse_server_message("not json");
        assert!(result.is_err(), "不正な JSON はエラーになること");
    }

    #[test]
    fn parse_server_message_rejects_missing_op() {
        let result = parse_server_message(r#"{"d":{}}"#);
        assert!(result.is_err(), "op フィールドがない場合はエラーになること");
    }

    #[test]
    fn parse_server_message_rejects_missing_d() {
        let result = parse_server_message(r#"{"op":0}"#);
        assert!(result.is_err(), "d フィールドがない場合はエラーになること");
    }

    #[test]
    fn serialize_client_message_serializes_identify() {
        let message = serialize_client_message(&ClientMessage::Identify(IdentifyData {
            rpc_version: 1,
            authentication: Some("auth-string".to_owned()),
            event_subscriptions: Some(33),
        }));
        let parsed = nojson::RawJson::parse(&message).expect("パースに失敗しないこと");
        let value = parsed.value();
        let op: i32 = value
            .to_member("op")
            .and_then(|member| member.required())
            .and_then(|v| v.try_into())
            .expect("op があること");
        assert_eq!(op, 1);
        let d = value
            .to_member("d")
            .and_then(|m| m.required())
            .expect("d があること");
        let rpc_version: i32 = d
            .to_member("rpcVersion")
            .and_then(|member| member.required())
            .and_then(|v| v.try_into())
            .expect("rpcVersion があること");
        assert_eq!(rpc_version, 1);
        let authentication: String = d
            .to_member("authentication")
            .and_then(|member| member.required())
            .and_then(|v| v.try_into())
            .expect("authentication があること");
        assert_eq!(authentication, "auth-string");
    }

    #[test]
    fn serialize_client_message_serializes_request() {
        let message = serialize_client_message(&ClientMessage::Request(RequestData {
            request_type: "GetSceneList".to_owned(),
            request_id: "req-1".to_owned(),
            request_data: None,
        }));
        let parsed = nojson::RawJson::parse(&message).expect("パースに失敗しないこと");
        let value = parsed.value();
        let op: i32 = value
            .to_member("op")
            .and_then(|member| member.required())
            .and_then(|v| v.try_into())
            .expect("op があること");
        assert_eq!(op, 6);
        let d = value
            .to_member("d")
            .and_then(|m| m.required())
            .expect("d があること");
        let request_type: String = d
            .to_member("requestType")
            .and_then(|member| member.required())
            .and_then(|v| v.try_into())
            .expect("requestType があること");
        assert_eq!(request_type, "GetSceneList");
    }
}
