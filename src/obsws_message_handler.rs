use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::ObswsInputRegistry;
use crate::obsws_protocol::{
    OBSWS_OP_HELLO, OBSWS_OP_IDENTIFIED, OBSWS_OP_IDENTIFY, OBSWS_OP_REQUEST,
    OBSWS_OP_REQUEST_RESPONSE, OBSWS_RPC_VERSION, OBSWS_SUPPORTED_IMAGE_FORMATS, OBSWS_VERSION,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_TYPE,
    REQUEST_STATUS_RESOURCE_NOT_FOUND, REQUEST_STATUS_SUCCESS, REQUEST_STATUS_UNKNOWN_REQUEST_TYPE,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ClientMessage {
    Identify(IdentifyMessage),
    Request(RequestMessage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IdentifyMessage {
    pub(crate) rpc_version: u32,
    pub(crate) authentication: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RequestMessage {
    pub(crate) request_id: Option<String>,
    pub(crate) request_type: Option<String>,
    pub(crate) request_data: Option<crate::json::JsonValue>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ObswsSessionStats {
    pub(crate) incoming_messages: u64,
    pub(crate) outgoing_messages: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestResponsePayload {
    pub(crate) message: String,
}

pub(crate) fn is_supported_rpc_version(rpc_version: u32) -> bool {
    rpc_version >= 1 && rpc_version <= OBSWS_RPC_VERSION
}

pub(crate) fn parse_client_message(text: &str) -> crate::Result<ClientMessage> {
    let json = nojson::RawJson::parse(text)?;
    let value = json.value();
    let op_value = value.to_member("op")?.required()?;
    let op: i64 = op_value.try_into()?;

    match op {
        OBSWS_OP_REQUEST => {
            let d_value = value.to_member("d")?.required()?;

            let request_id: Option<String> = d_value.to_member("requestId")?.try_into()?;
            let request_type: Option<String> = d_value.to_member("requestType")?.try_into()?;
            let request_data: Option<crate::json::JsonValue> =
                d_value.to_member("requestData")?.try_into()?;

            Ok(ClientMessage::Request(RequestMessage {
                request_id,
                request_type,
                request_data,
            }))
        }
        OBSWS_OP_IDENTIFY => {
            let d_value = value.to_member("d")?.required()?;
            let rpc_version: u32 = d_value.to_member("rpcVersion")?.required()?.try_into()?;
            let authentication: Option<String> = d_value.to_member("authentication")?.try_into()?;
            Ok(ClientMessage::Identify(IdentifyMessage {
                rpc_version,
                authentication,
            }))
        }
        _ => Err(crate::Error::new(format!(
            "unsupported message opcode: {op}"
        ))),
    }
}

pub(crate) fn handle_request_message(
    request: RequestMessage,
    session_stats: &ObswsSessionStats,
    input_registry: &ObswsInputRegistry,
) -> RequestResponsePayload {
    let request_id = request.request_id.unwrap_or_default();
    let request_type = request.request_type.unwrap_or_default();
    if request_id.is_empty() {
        return RequestResponsePayload {
            message: build_request_response_error(
                &request_type,
                &request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required requestId field",
            ),
        };
    }

    if request_type.is_empty() {
        return RequestResponsePayload {
            message: build_request_response_error(
                &request_type,
                &request_id,
                REQUEST_STATUS_MISSING_REQUEST_TYPE,
                "Missing required requestType field",
            ),
        };
    }

    let message = match request_type.as_str() {
        "GetVersion" => build_get_version_response(&request_id),
        "GetStats" => build_get_stats_response(&request_id, session_stats),
        "GetCanvasList" => build_get_canvas_list_response(&request_id),
        "GetInputList" => build_get_input_list_response(&request_id, input_registry),
        "GetInputKindList" => build_get_input_kind_list_response(&request_id, input_registry),
        "GetInputSettings" => build_get_input_settings_response(
            &request_id,
            request.request_data.as_ref(),
            input_registry,
        ),
        _ => build_request_response_error(
            &request_type,
            &request_id,
            REQUEST_STATUS_UNKNOWN_REQUEST_TYPE,
            "Unknown request type",
        ),
    };
    RequestResponsePayload { message }
}

fn parse_input_lookup_fields(
    request_data: Option<&crate::json::JsonValue>,
) -> Result<(Option<&str>, Option<&str>), &'static str> {
    let Some(request_data) = request_data else {
        return Err("Missing required requestData field");
    };
    let crate::json::JsonValue::Object(request_data) = request_data else {
        return Err("Invalid requestData field");
    };

    let input_name = request_data
        .get("inputName")
        .and_then(|v| {
            let crate::json::JsonValue::String(v) = v else {
                return None;
            };
            Some(v.as_str())
        })
        .filter(|v| !v.is_empty());
    let input_uuid = request_data
        .get("inputUuid")
        .and_then(|v| {
            let crate::json::JsonValue::String(v) = v else {
                return None;
            };
            Some(v.as_str())
        })
        .filter(|v| !v.is_empty());

    if input_name.is_none() && input_uuid.is_none() {
        return Err("Missing required inputName or inputUuid field");
    }

    Ok((input_uuid, input_name))
}

pub(crate) fn build_hello_message(authentication: Option<&ObswsAuthentication>) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_HELLO)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("obsWebSocketVersion", OBSWS_VERSION)?;
                f.member("rpcVersion", OBSWS_RPC_VERSION)?;
                if let Some(authentication) = authentication {
                    f.member(
                        "authentication",
                        nojson::object(|f| {
                            f.member("challenge", &authentication.challenge)?;
                            f.member("salt", &authentication.salt)
                        }),
                    )?;
                }
                Ok(())
            }),
        )
    })
    .to_string()
}

pub(crate) fn build_identified_message(negotiated_rpc_version: u32) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_IDENTIFIED)?;
        f.member(
            "d",
            nojson::object(|f| f.member("negotiatedRpcVersion", negotiated_rpc_version)),
        )
    })
    .to_string()
}

fn build_get_version_response(request_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetVersion")?;
                f.member("requestId", request_id)?;
                f.member(
                    "requestStatus",
                    nojson::object(|f| {
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member(
                    "responseData",
                    nojson::object(|f| {
                        f.member("obsVersion", env!("CARGO_PKG_VERSION"))?;
                        f.member("obsWebSocketVersion", OBSWS_VERSION)?;
                        f.member("rpcVersion", OBSWS_RPC_VERSION)?;
                        f.member(
                            "availableRequests",
                            [
                                "GetVersion",
                                "GetStats",
                                "GetCanvasList",
                                "GetInputList",
                                "GetInputKindList",
                                "GetInputSettings",
                            ],
                        )?;
                        f.member("supportedImageFormats", OBSWS_SUPPORTED_IMAGE_FORMATS)?;
                        f.member("platform", std::env::consts::OS)?;
                        f.member(
                            "platformDescription",
                            format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
                        )
                    }),
                )
            }),
        )
    })
    .to_string()
}

fn build_get_stats_response(request_id: &str, session_stats: &ObswsSessionStats) -> String {
    let outgoing_messages = session_stats.outgoing_messages.saturating_add(1);

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetStats")?;
                f.member("requestId", request_id)?;
                f.member(
                    "requestStatus",
                    nojson::object(|f| {
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member(
                    "responseData",
                    nojson::object(|f| {
                        f.member("cpuUsage", 0.0)?;
                        f.member("memoryUsage", 0.0)?;
                        f.member("availableDiskSpace", 0.0)?;
                        f.member("activeFps", 0.0)?;
                        f.member("averageFrameRenderTime", 0.0)?;
                        f.member("renderSkippedFrames", 0)?;
                        f.member("renderTotalFrames", 0)?;
                        f.member("outputSkippedFrames", 0)?;
                        f.member("outputTotalFrames", 0)?;
                        f.member(
                            "webSocketSessionIncomingMessages",
                            session_stats.incoming_messages,
                        )?;
                        f.member("webSocketSessionOutgoingMessages", outgoing_messages)
                    }),
                )
            }),
        )
    })
    .to_string()
}

fn build_get_canvas_list_response(request_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetCanvasList")?;
                f.member("requestId", request_id)?;
                f.member(
                    "requestStatus",
                    nojson::object(|f| {
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member(
                    "responseData",
                    nojson::object(|f| {
                        f.member(
                            "canvases",
                            [nojson::object(|f| {
                                f.member("canvasName", "hisui-main")?;
                                f.member("canvasWidth", 0)?;
                                f.member("canvasHeight", 0)
                            })],
                        )
                    }),
                )
            }),
        )
    })
    .to_string()
}

fn build_get_input_list_response(request_id: &str, input_registry: &ObswsInputRegistry) -> String {
    let inputs = input_registry.list_inputs();
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetInputList")?;
                f.member("requestId", request_id)?;
                f.member(
                    "requestStatus",
                    nojson::object(|f| {
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member(
                    "responseData",
                    nojson::object(|f| f.member("inputs", &inputs)),
                )
            }),
        )
    })
    .to_string()
}

fn build_get_input_kind_list_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetInputKindList")?;
                f.member("requestId", request_id)?;
                f.member(
                    "requestStatus",
                    nojson::object(|f| {
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member(
                    "responseData",
                    nojson::object(|f| {
                        f.member("inputKinds", input_registry.supported_input_kinds())
                    }),
                )
            }),
        )
    })
    .to_string()
}

fn build_get_input_settings_response(
    request_id: &str,
    request_data: Option<&crate::json::JsonValue>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let (input_uuid, input_name) = match parse_input_lookup_fields(request_data) {
        Ok(v) => v,
        Err(message) => {
            return build_request_response_error(
                "GetInputSettings",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                message,
            );
        }
    };

    let Some(input) = input_registry.find_input(input_uuid, input_name) else {
        return build_request_response_error(
            "GetInputSettings",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Input not found",
        );
    };

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetInputSettings")?;
                f.member("requestId", request_id)?;
                f.member(
                    "requestStatus",
                    nojson::object(|f| {
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member(
                    "responseData",
                    nojson::object(|f| {
                        f.member("inputName", &input.input_name)?;
                        f.member("inputKind", &input.input_kind)?;
                        f.member("inputSettings", &input.settings)
                    }),
                )
            }),
        )
    })
    .to_string()
}

fn build_request_response_error(
    request_type: &str,
    request_id: &str,
    code: i64,
    comment: &str,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", request_type)?;
                f.member("requestId", request_id)?;
                f.member(
                    "requestStatus",
                    nojson::object(|f| {
                        f.member("result", false)?;
                        f.member("code", code)?;
                        f.member("comment", comment)
                    }),
                )
            }),
        )
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsws_auth::build_authentication_response;
    use crate::obsws_input_registry::{ObswsInputEntry, ObswsInputRegistry};

    fn input_registry() -> ObswsInputRegistry {
        let mut registry = ObswsInputRegistry::new();
        registry.insert_for_test(ObswsInputEntry::new_for_test(
            "input-uuid-1",
            "input-name-1",
            "ffmpeg_source",
            crate::json::JsonValue::Object(
                [(
                    "input".to_owned(),
                    crate::json::JsonValue::String("sample.mp4".to_owned()),
                )]
                .into_iter()
                .collect(),
            ),
        ));
        registry
    }

    #[test]
    fn build_hello_message_contains_expected_fields() {
        let message = build_hello_message(None);
        let json = nojson::RawJson::parse(&message).expect("hello message must be valid JSON");
        let op_value = json
            .value()
            .to_member("op")
            .expect("op member access must succeed")
            .required()
            .expect("op must exist");
        let op: i64 = op_value.try_into().expect("op must be i64");
        assert_eq!(op, OBSWS_OP_HELLO);
    }

    #[test]
    fn parse_client_message_accepts_identify() {
        let message = r#"{"op":1,"d":{"rpcVersion":1}}"#;
        let parsed = parse_client_message(message).expect("identify message must be accepted");
        assert_eq!(
            parsed,
            ClientMessage::Identify(IdentifyMessage {
                rpc_version: 1,
                authentication: None,
            })
        );
    }

    #[test]
    fn parse_client_message_accepts_identify_with_authentication() {
        let message = r#"{"op":1,"d":{"rpcVersion":1,"authentication":"test-auth"}}"#;
        let parsed = parse_client_message(message).expect("identify message must be accepted");
        assert_eq!(
            parsed,
            ClientMessage::Identify(IdentifyMessage {
                rpc_version: 1,
                authentication: Some("test-auth".to_owned()),
            })
        );
    }

    #[test]
    fn parse_client_message_rejects_identify_without_rpc_version() {
        let message = r#"{"op":1,"d":{}}"#;
        let error = parse_client_message(message).expect_err("identify without rpcVersion");
        assert!(error.display().contains("rpcVersion"));
    }

    #[test]
    fn is_supported_rpc_version_accepts_only_range_one_to_server_version() {
        assert!(!is_supported_rpc_version(0));
        assert!(is_supported_rpc_version(1));
        assert!(!is_supported_rpc_version(
            OBSWS_RPC_VERSION.saturating_add(1)
        ));
    }

    #[test]
    fn parse_client_message_accepts_request() {
        let message =
            r#"{"op":6,"d":{"requestType":"GetVersion","requestId":"req-1","requestData":{}}}"#;
        let parsed = parse_client_message(message).expect("request message must be accepted");
        assert_eq!(
            parsed,
            ClientMessage::Request(RequestMessage {
                request_id: Some("req-1".to_owned()),
                request_type: Some("GetVersion".to_owned()),
                request_data: Some(crate::json::JsonValue::Object(Default::default())),
            })
        );
    }

    #[test]
    fn parse_client_message_rejects_non_identify() {
        let message = r#"{"op":9,"d":{}}"#;
        let error = parse_client_message(message).expect_err("non identify must be rejected");
        assert!(error.display().contains("unsupported message opcode"));
    }

    #[test]
    fn build_authentication_response_matches_expected_value() {
        let response = build_authentication_response("test-password", "c2FsdA==", "Y2hhbGxlbmdl");
        assert_eq!(response, "692yhXm+ZMl25QzSnVANJIg265Xtpfqja0A08Opeiv8=");
    }

    #[test]
    fn build_hello_message_contains_authentication() {
        let auth = ObswsAuthentication {
            salt: "test-salt".to_owned(),
            challenge: "test-challenge".to_owned(),
            expected_response: "unused".to_owned(),
        };
        let message = build_hello_message(Some(&auth));
        let json = nojson::RawJson::parse(&message).expect("hello message must be valid JSON");
        let d_value = json
            .value()
            .to_member("d")
            .expect("d member access must succeed")
            .required()
            .expect("d must exist");
        let authentication = d_value
            .to_member("authentication")
            .expect("authentication member access must succeed")
            .required()
            .expect("authentication must exist");
        let challenge: String = authentication
            .to_member("challenge")
            .expect("challenge member access must succeed")
            .required()
            .expect("challenge must exist")
            .try_into()
            .expect("challenge must be string");
        let salt: String = authentication
            .to_member("salt")
            .expect("salt member access must succeed")
            .required()
            .expect("salt must exist")
            .try_into()
            .expect("salt must be string");
        assert_eq!(challenge, "test-challenge");
        assert_eq!(salt, "test-salt");
    }

    #[test]
    fn handle_request_message_returns_get_version_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("GetVersion".to_owned()),
            request_data: None,
        };
        let session_stats = ObswsSessionStats::default();
        let response = handle_request_message(request, &session_stats, &input_registry());

        let json = nojson::RawJson::parse(&response.message)?;
        let op: i64 = json.value().to_member("op")?.required()?.try_into()?;
        let response_data = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("responseData")?
            .required()?;
        let supported_image_formats: Vec<String> = response_data
            .to_member("supportedImageFormats")?
            .required()?
            .try_into()?;
        assert_eq!(op, OBSWS_OP_REQUEST_RESPONSE);
        assert!(supported_image_formats.iter().any(|f| f == "png"));
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_unknown_request_type_error()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("UnknownRequest".to_owned()),
            request_data: None,
        };
        let session_stats = ObswsSessionStats::default();
        let response = handle_request_message(request, &session_stats, &input_registry());

        let json = nojson::RawJson::parse(&response.message)?;
        let status = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("requestStatus")?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_UNKNOWN_REQUEST_TYPE);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_input_list_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("GetInputList".to_owned()),
            request_data: None,
        };
        let session_stats = ObswsSessionStats::default();
        let response = handle_request_message(request, &session_stats, &input_registry());

        let json = nojson::RawJson::parse(&response.message)?;
        let response_data = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("responseData")?
            .required()?;
        let inputs: Vec<crate::json::JsonValue> =
            response_data.to_member("inputs")?.required()?.try_into()?;
        let Some(crate::json::JsonValue::Object(first_input)) = inputs.first() else {
            panic!("first input must be object");
        };
        let Some(crate::json::JsonValue::String(input_name)) = first_input.get("inputName") else {
            panic!("inputName must exist");
        };
        assert_eq!(input_name, "input-name-1");
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_input_kind_list_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("GetInputKindList".to_owned()),
            request_data: None,
        };
        let session_stats = ObswsSessionStats::default();
        let response = handle_request_message(request, &session_stats, &input_registry());

        let json = nojson::RawJson::parse(&response.message)?;
        let response_data = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("responseData")?
            .required()?;
        let input_kinds: Vec<String> = response_data
            .to_member("inputKinds")?
            .required()?
            .try_into()?;
        assert!(input_kinds.iter().any(|kind| kind == "ffmpeg_source"));
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_input_settings_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("GetInputSettings".to_owned()),
            request_data: Some(crate::json::JsonValue::Object(
                [(
                    "inputName".to_owned(),
                    crate::json::JsonValue::String("input-name-1".to_owned()),
                )]
                .into_iter()
                .collect(),
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let response = handle_request_message(request, &session_stats, &input_registry());

        let json = nojson::RawJson::parse(&response.message)?;
        let response_data = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("responseData")?
            .required()?;
        let input_kind: String = response_data
            .to_member("inputKind")?
            .required()?
            .try_into()?;
        assert_eq!(input_kind, "ffmpeg_source");
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_error_when_get_input_settings_is_missing_lookup_fields()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("GetInputSettings".to_owned()),
            request_data: None,
        };
        let session_stats = ObswsSessionStats::default();
        let response = handle_request_message(request, &session_stats, &input_registry());

        let json = nojson::RawJson::parse(&response.message)?;
        let status = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("requestStatus")?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_MISSING_REQUEST_FIELD);
        Ok(())
    }
}
