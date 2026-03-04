use crate::obsws_input_registry::ObswsInputRegistry;
use crate::obsws_protocol::{
    OBSWS_OP_IDENTIFY, OBSWS_OP_REQUEST, OBSWS_RPC_VERSION, REQUEST_STATUS_MISSING_REQUEST_FIELD,
    REQUEST_STATUS_MISSING_REQUEST_TYPE, REQUEST_STATUS_UNKNOWN_REQUEST_TYPE,
};

pub use crate::obsws_response_builder::{build_hello_message, build_identified_message};

#[derive(Debug, Clone, PartialEq)]
pub enum ClientMessage {
    Identify(IdentifyMessage),
    Request(RequestMessage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifyMessage {
    pub rpc_version: u32,
    pub authentication: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestMessage {
    pub request_id: Option<String>,
    pub request_type: Option<String>,
    pub request_data: Option<nojson::RawJsonOwned>,
}

#[derive(Debug, Clone, Default)]
pub struct ObswsSessionStats {
    pub incoming_messages: u64,
    pub outgoing_messages: u64,
}

#[derive(Debug, Clone)]
pub struct RequestResponsePayload {
    pub message: String,
}

pub fn is_supported_rpc_version(rpc_version: u32) -> bool {
    rpc_version >= 1 && rpc_version <= OBSWS_RPC_VERSION
}

pub fn parse_client_message(text: &str) -> crate::Result<ClientMessage> {
    let json = nojson::RawJson::parse(text)?;
    let value = json.value();
    let op_value = value.to_member("op")?.required()?;
    let op: i64 = op_value.try_into()?;

    match op {
        OBSWS_OP_REQUEST => {
            let d_value = value.to_member("d")?.required()?;

            let request_id: Option<String> = d_value.to_member("requestId")?.try_into()?;
            let request_type: Option<String> = d_value.to_member("requestType")?.try_into()?;
            let request_data: Option<nojson::RawJsonOwned> = d_value
                .to_member("requestData")?
                .map(nojson::RawJsonOwned::try_from)?;

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

pub fn handle_request_message(
    request: RequestMessage,
    session_stats: &ObswsSessionStats,
    input_registry: &mut ObswsInputRegistry,
) -> RequestResponsePayload {
    let request_id = request.request_id.unwrap_or_default();
    let request_type = request.request_type.unwrap_or_default();
    if request_id.is_empty() {
        return RequestResponsePayload {
            message: crate::obsws_response_builder::build_request_response_error(
                &request_type,
                &request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required requestId field",
            ),
        };
    }

    if request_type.is_empty() {
        return RequestResponsePayload {
            message: crate::obsws_response_builder::build_request_response_error(
                &request_type,
                &request_id,
                REQUEST_STATUS_MISSING_REQUEST_TYPE,
                "Missing required requestType field",
            ),
        };
    }

    let message = match request_type.as_str() {
        "GetVersion" => crate::obsws_response_builder::build_get_version_response(&request_id),
        "GetStats" => {
            crate::obsws_response_builder::build_get_stats_response(&request_id, session_stats)
        }
        "GetCanvasList" => {
            crate::obsws_response_builder::build_get_canvas_list_response(&request_id)
        }
        "GetInputList" => crate::obsws_response_builder::build_get_input_list_response(
            &request_id,
            input_registry,
        ),
        "GetInputKindList" => crate::obsws_response_builder::build_get_input_kind_list_response(
            &request_id,
            input_registry,
        ),
        "GetInputSettings" => crate::obsws_response_builder::build_get_input_settings_response(
            &request_id,
            request.request_data.as_ref(),
            input_registry,
        ),
        "CreateInput" => crate::obsws_response_builder::build_create_input_response(
            &request_id,
            request.request_data.as_ref(),
            input_registry,
        ),
        "RemoveInput" => crate::obsws_response_builder::build_remove_input_response(
            &request_id,
            request.request_data.as_ref(),
            input_registry,
        ),
        _ => crate::obsws_response_builder::build_request_response_error(
            &request_type,
            &request_id,
            REQUEST_STATUS_UNKNOWN_REQUEST_TYPE,
            "Unknown request type",
        ),
    };
    RequestResponsePayload { message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsws_auth::{ObswsAuthentication, build_authentication_response};
    use crate::obsws_input_registry::{
        ObswsInput, ObswsInputEntry, ObswsInputRegistry, ObswsInputSettings,
        ObswsVideoCaptureDeviceSettings,
    };
    use crate::obsws_protocol::{
        OBSWS_OP_HELLO, OBSWS_OP_REQUEST_RESPONSE, REQUEST_STATUS_INVALID_REQUEST_FIELD,
        REQUEST_STATUS_RESOURCE_ALREADY_EXISTS, REQUEST_STATUS_RESOURCE_NOT_FOUND,
        REQUEST_STATUS_SUCCESS,
    };

    fn input_registry() -> ObswsInputRegistry {
        let mut registry = ObswsInputRegistry::new();
        registry.insert_for_test(ObswsInputEntry::new_for_test(
            "input-uuid-1",
            "input-name-1",
            ObswsInput {
                settings: ObswsInputSettings::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                    device_id: Some("camera-1".to_owned()),
                }),
            },
        ));
        registry
    }

    fn request_data(json: &str) -> nojson::RawJsonOwned {
        nojson::RawJsonOwned::parse(json).expect("requestData must be valid json")
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
                request_data: Some(request_data("{}")),
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
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);

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
        let available_requests: Vec<String> = response_data
            .to_member("availableRequests")?
            .required()?
            .try_into()?;
        assert_eq!(op, OBSWS_OP_REQUEST_RESPONSE);
        assert!(supported_image_formats.iter().any(|f| f == "png"));
        assert!(available_requests.iter().any(|r| r == "CreateInput"));
        assert!(available_requests.iter().any(|r| r == "RemoveInput"));
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
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);

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
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);

        let json = nojson::RawJson::parse(&response.message)?;
        let response_data = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("responseData")?
            .required()?;
        let mut inputs = response_data.to_member("inputs")?.required()?.to_array()?;
        let first_input = inputs.next().expect("first input must exist");
        let input_name: String = first_input.to_member("inputName")?.required()?.try_into()?;
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
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);

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
        assert!(
            input_kinds
                .iter()
                .any(|kind| kind == "video_capture_device")
        );
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_input_settings_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("GetInputSettings".to_owned()),
            request_data: Some(request_data(r#"{"inputName":"input-name-1"}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);

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
        assert_eq!(input_kind, "video_capture_device");
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
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);

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

    #[test]
    fn handle_request_message_returns_create_input_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-create-1".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(request_data(
                r#"{"sceneName":"Scene","inputName":"camera-2","inputKind":"video_capture_device","inputSettings":{},"sceneItemEnabled":true}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);
        let json = nojson::RawJson::parse(&response.message)?;
        let status = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("requestStatus")?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        let input_uuid: String = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("responseData")?
            .required()?
            .to_member("inputUuid")?
            .required()?
            .try_into()?;
        assert!(result);
        assert_eq!(code, REQUEST_STATUS_SUCCESS);
        assert!(!input_uuid.is_empty());
        assert!(input_registry.find_input(Some(&input_uuid), None).is_some());
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_duplicate_error_for_create_input()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-create-dup".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(request_data(
                r#"{"sceneName":"Scene","inputName":"input-name-1","inputKind":"video_capture_device","inputSettings":{}}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);
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
        assert_eq!(code, REQUEST_STATUS_RESOURCE_ALREADY_EXISTS);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_invalid_field_error_for_unsupported_scene_name()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-create-invalid-scene".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(request_data(
                r#"{"sceneName":"custom-scene","inputName":"input-name-2","inputKind":"video_capture_device","inputSettings":{}}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);
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
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
        Ok(())
    }

    #[test]
    fn handle_request_message_rejects_non_object_input_settings_for_create_input()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-create-invalid-settings-1".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(request_data(
                r#"{"sceneName":"Scene","inputName":"camera-invalid","inputKind":"video_capture_device","inputSettings":null}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);
        let json = nojson::RawJson::parse(&response.message)?;
        let status = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("requestStatus")?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        let comment: String = status.to_member("comment")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
        assert!(comment.contains("Invalid inputSettings field"));
        Ok(())
    }

    #[test]
    fn handle_request_message_rejects_invalid_known_input_settings_field_type()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-create-invalid-settings-2".to_owned()),
            request_type: Some("CreateInput".to_owned()),
            request_data: Some(request_data(
                r#"{"sceneName":"Scene","inputName":"camera-invalid-2","inputKind":"video_capture_device","inputSettings":{"device_id":1}}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);
        let json = nojson::RawJson::parse(&response.message)?;
        let status = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("requestStatus")?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        let comment: String = status.to_member("comment")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
        assert!(comment.contains("inputSettings.device_id"));
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_remove_input_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-remove-1".to_owned()),
            request_type: Some("RemoveInput".to_owned()),
            request_data: Some(request_data(r#"{"inputName":"input-name-1"}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);
        let json = nojson::RawJson::parse(&response.message)?;
        let status = json
            .value()
            .to_member("d")?
            .required()?
            .to_member("requestStatus")?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(result);
        assert_eq!(code, REQUEST_STATUS_SUCCESS);
        assert!(
            input_registry
                .find_input(None, Some("input-name-1"))
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_not_found_error_for_remove_input()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-remove-2".to_owned()),
            request_type: Some("RemoveInput".to_owned()),
            request_data: Some(request_data(r#"{"inputName":"not-found"}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut input_registry = input_registry();
        let response = handle_request_message(request, &session_stats, &mut input_registry);
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
        assert_eq!(code, REQUEST_STATUS_RESOURCE_NOT_FOUND);
        Ok(())
    }
}
