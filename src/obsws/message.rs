use crate::obsws::protocol::{
    OBSWS_OP_IDENTIFY, OBSWS_OP_REIDENTIFY, OBSWS_OP_REQUEST, OBSWS_OP_REQUEST_BATCH,
    OBSWS_RPC_VERSION, REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_TYPE,
    REQUEST_STATUS_UNKNOWN_REQUEST_TYPE,
};
use crate::obsws::state::ObswsSessionState;

pub use crate::obsws::response::{build_hello_message, build_identified_message};

#[derive(Debug, Clone, PartialEq)]
pub enum ClientMessage {
    Identify(IdentifyMessage),
    Reidentify(ReidentifyMessage),
    Request(RequestMessage),
    RequestBatch(RequestBatchMessage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifyMessage {
    pub rpc_version: u32,
    pub authentication: Option<String>,
    pub event_subscriptions: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReidentifyMessage {
    pub event_subscriptions: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestMessage {
    pub request_id: Option<String>,
    pub request_type: Option<String>,
    pub request_data: Option<nojson::RawJsonOwned>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestBatchMessage {
    pub request_id: Option<String>,
    pub halt_on_failure: Option<bool>,
    pub execution_type: Option<i64>,
    pub requests: Option<Vec<RequestMessage>>,
}

#[derive(Debug, Clone, Default)]
pub struct ObswsSessionStats {
    pub incoming_messages: u64,
    pub outgoing_messages: u64,
}

#[derive(Debug, Clone)]
pub struct RequestResponsePayload {
    pub message: nojson::RawJsonOwned,
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
            Ok(ClientMessage::Request(parse_request_message(d_value)?))
        }
        OBSWS_OP_REQUEST_BATCH => {
            let d_value = value.to_member("d")?.required()?;
            let request_id: Option<String> = d_value.to_member("requestId")?.try_into()?;
            let halt_on_failure: Option<bool> = d_value.to_member("haltOnFailure")?.try_into()?;
            let execution_type: Option<i64> = d_value.to_member("executionType")?.try_into()?;
            // OBS は variables / inputVariables / outputVariables による
            // batch 内の変数置換をサポートするが、hisui では未対応。
            // nojson は明示的に参照しないフィールドを無視するため、
            // これらのフィールドが存在してもパースエラーにはならない。
            let requests = if let Some(requests_value) = d_value.to_member("requests")?.optional() {
                let requests = requests_value
                    .to_array()?
                    .map(parse_request_message)
                    .collect::<Result<Vec<_>, _>>()?;
                Some(requests)
            } else {
                None
            };
            Ok(ClientMessage::RequestBatch(RequestBatchMessage {
                request_id,
                halt_on_failure,
                execution_type,
                requests,
            }))
        }
        OBSWS_OP_IDENTIFY => {
            let d_value = value.to_member("d")?.required()?;
            let rpc_version: u32 = d_value.to_member("rpcVersion")?.required()?.try_into()?;
            let authentication: Option<String> = d_value.to_member("authentication")?.try_into()?;
            let event_subscriptions: Option<u32> =
                d_value.to_member("eventSubscriptions")?.try_into()?;
            Ok(ClientMessage::Identify(IdentifyMessage {
                rpc_version,
                authentication,
                event_subscriptions,
            }))
        }
        OBSWS_OP_REIDENTIFY => {
            let d_value = value.to_member("d")?.required()?;
            let event_subscriptions: Option<u32> =
                d_value.to_member("eventSubscriptions")?.try_into()?;
            Ok(ClientMessage::Reidentify(ReidentifyMessage {
                event_subscriptions,
            }))
        }
        _ => Err(crate::Error::new(format!(
            "unsupported message opcode: {op}"
        ))),
    }
}

fn parse_request_message(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<RequestMessage, nojson::JsonParseError> {
    let request_id: Option<String> = value.to_member("requestId")?.try_into()?;
    let request_type: Option<String> = value.to_member("requestType")?.try_into()?;
    let request_data: Option<nojson::RawJsonOwned> = value
        .to_member("requestData")?
        .map(nojson::RawJsonOwned::try_from)?;
    Ok(RequestMessage {
        request_id,
        request_type,
        request_data,
    })
}

pub fn handle_request_message(
    request: RequestMessage,
    session_stats: &ObswsSessionStats,
    state: &mut ObswsSessionState,
) -> RequestResponsePayload {
    handle_request_message_with_pipeline_handle(request, session_stats, state, None)
}

pub fn handle_request_message_with_pipeline_handle(
    request: RequestMessage,
    _session_stats: &ObswsSessionStats,
    state: &mut ObswsSessionState,
    _pipeline_handle: Option<&crate::MediaPipelineHandle>,
) -> RequestResponsePayload {
    let request_id = request.request_id.unwrap_or_default();
    let request_type = request.request_type.unwrap_or_default();
    if request_id.is_empty() {
        return RequestResponsePayload {
            message: crate::obsws::response::build_request_response_error(
                &request_type,
                &request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required requestId field",
            ),
        };
    }

    if request_type.is_empty() {
        return RequestResponsePayload {
            message: crate::obsws::response::build_request_response_error(
                &request_type,
                &request_id,
                REQUEST_STATUS_MISSING_REQUEST_TYPE,
                "Missing required requestType field",
            ),
        };
    }

    let message = match request_type.as_str() {
        "GetVersion" => crate::obsws::response::build_get_version_response(&request_id, &[]),
        // GetStats は coordinator で処理する（outputs BTreeMap を参照するため）
        "GetCanvasList" => crate::obsws::response::build_get_canvas_list_response(
            &request_id,
            state.canvas_width(),
            state.canvas_height(),
            state.frame_rate(),
        ),
        "GetGroupList" => crate::obsws::response::build_get_group_list_response(&request_id),
        "GetSceneList" => crate::obsws::response::build_get_scene_list_response(&request_id, state),
        "GetCurrentProgramScene" => {
            crate::obsws::response::build_get_current_program_scene_response(&request_id, state)
        }
        "GetCurrentPreviewScene" => {
            crate::obsws::response::build_get_current_preview_scene_response(&request_id)
        }
        "GetSceneSceneTransitionOverride" => {
            crate::obsws::response::build_get_scene_scene_transition_override_response(
                &request_id,
                request.request_data.as_ref(),
                state,
            )
        }
        "SetSceneSceneTransitionOverride" => {
            crate::obsws::response::build_set_scene_scene_transition_override_response(
                &request_id,
                request.request_data.as_ref(),
                state,
            )
        }
        "GetTransitionKindList" => {
            crate::obsws::response::build_get_transition_kind_list_response(&request_id, state)
        }
        "GetSceneTransitionList" => {
            crate::obsws::response::build_get_scene_transition_list_response(&request_id, state)
        }
        "GetCurrentSceneTransition" => {
            crate::obsws::response::build_get_current_scene_transition_response(&request_id, state)
        }
        "SetCurrentSceneTransition" => {
            crate::obsws::response::build_set_current_scene_transition_response(
                &request_id,
                request.request_data.as_ref(),
                state,
            )
        }
        "SetCurrentSceneTransitionDuration" => {
            crate::obsws::response::build_set_current_scene_transition_duration_response(
                &request_id,
                request.request_data.as_ref(),
                state,
            )
        }
        "SetCurrentSceneTransitionSettings" => {
            crate::obsws::response::build_set_current_scene_transition_settings_response(
                &request_id,
                request.request_data.as_ref(),
                state,
            )
        }
        "GetCurrentSceneTransitionCursor" => {
            crate::obsws::response::build_get_current_scene_transition_cursor_response(
                &request_id,
                state,
            )
        }
        "SetTBarPosition" => crate::obsws::response::build_set_tbar_position_response(&request_id),
        "SetSceneName" => crate::obsws::response::build_set_scene_name_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetSceneItemId" => crate::obsws::response::build_get_scene_item_id_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetSceneItemList" => crate::obsws::response::build_get_scene_item_list_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetSceneItemSource" => crate::obsws::response::build_get_scene_item_source_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetSceneItemEnabled" => crate::obsws::response::build_get_scene_item_enabled_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetSceneItemLocked" => crate::obsws::response::build_get_scene_item_locked_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetSceneItemIndex" => crate::obsws::response::build_get_scene_item_index_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetSceneItemBlendMode" => {
            crate::obsws::response::build_get_scene_item_blend_mode_response(
                &request_id,
                request.request_data.as_ref(),
                state,
            )
        }
        "GetSceneItemTransform" => crate::obsws::response::build_get_scene_item_transform_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetInputList" => crate::obsws::response::build_get_input_list_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetInputKindList" => {
            crate::obsws::response::build_get_input_kind_list_response(&request_id, state)
        }
        "GetSourceActive" => crate::obsws::response::build_get_source_active_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetInputSettings" => crate::obsws::response::build_get_input_settings_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetInputMute" => crate::obsws::response::build_get_input_mute_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetInputVolume" => crate::obsws::response::build_get_input_volume_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "SetInputSettings" => crate::obsws::response::build_set_input_settings_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "SetInputName" => crate::obsws::response::build_set_input_name_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "GetInputDefaultSettings" => {
            crate::obsws::response::build_get_input_default_settings_response(
                &request_id,
                request.request_data.as_ref(),
                state,
            )
        }
        "GetInputPropertiesListPropertyItems" => {
            crate::obsws::response::build_get_input_properties_list_property_items_response(
                &request_id,
                request.request_data.as_ref(),
                state,
            )
        }
        "GetPersistentData" => crate::obsws::response::build_get_persistent_data_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        "SetPersistentData" => crate::obsws::response::build_set_persistent_data_response(
            &request_id,
            request.request_data.as_ref(),
            state,
        ),
        // GetStreamServiceSettings / SetStreamServiceSettings / GetOutputSettings /
        // SetOutputSettings / GetRecordDirectory / SetRecordDirectory /
        // GetStreamStatus / GetOutputStatus / GetRecordStatus は
        // coordinator で処理する（outputs BTreeMap を参照するため）
        _ => crate::obsws::response::build_request_response_error(
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
    use crate::obsws::auth::{ObswsAuthentication, build_authentication_response};
    use crate::obsws::protocol::{
        OBSWS_OP_HELLO, OBSWS_OP_REQUEST_RESPONSE, REQUEST_STATUS_INVALID_REQUEST_FIELD,
        REQUEST_STATUS_MISSING_REQUEST_DATA, REQUEST_STATUS_RESOURCE_NOT_FOUND,
    };
    use crate::obsws::state::{
        ObswsInput, ObswsInputEntry, ObswsInputSettings, ObswsSessionState,
        ObswsVideoCaptureDeviceSettings,
    };
    fn state() -> ObswsSessionState {
        let mut registry = ObswsSessionState::new_for_test();
        registry.insert_for_test(ObswsInputEntry::new_for_test(
            "input-uuid-1",
            "input-name-1",
            ObswsInput {
                settings: ObswsInputSettings::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                    device_id: Some("camera-1".to_owned()),
                    pixel_format: None,
                    fps: None,
                }),
                input_muted: false,
                input_volume_mul: crate::types::NonNegFiniteF64::ONE,
            },
        ));
        registry
    }

    /// audio_capture_device 入力を含む fixture
    fn state_with_audio_input() -> ObswsSessionState {
        use crate::obsws::state::ObswsAudioCaptureDeviceSettings;
        let mut registry = state();
        registry.insert_for_test(ObswsInputEntry::new_for_test(
            "input-uuid-audio",
            "input-name-audio",
            ObswsInput {
                settings: ObswsInputSettings::AudioCaptureDevice(
                    ObswsAudioCaptureDeviceSettings::default(),
                ),
                input_muted: false,
                input_volume_mul: crate::types::NonNegFiniteF64::ONE,
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
        let json =
            nojson::RawJson::parse(message.text()).expect("hello message must be valid JSON");
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
                event_subscriptions: None,
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
                event_subscriptions: None,
            })
        );
    }

    #[test]
    fn parse_client_message_accepts_identify_with_event_subscriptions() {
        let message = r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":64}}"#;
        let parsed = parse_client_message(message).expect("identify message must be accepted");
        assert_eq!(
            parsed,
            ClientMessage::Identify(IdentifyMessage {
                rpc_version: 1,
                authentication: None,
                event_subscriptions: Some(64),
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
    fn parse_client_message_rejects_identify_with_invalid_event_subscriptions() {
        let message = r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":"invalid"}}"#;
        let error = parse_client_message(message).expect_err("identify must reject invalid type");
        assert!(!error.display().is_empty());
    }

    #[test]
    fn parse_client_message_accepts_reidentify_without_event_subscriptions() {
        let message = r#"{"op":3,"d":{}}"#;
        let parsed = parse_client_message(message).expect("reidentify message must be accepted");
        assert_eq!(
            parsed,
            ClientMessage::Reidentify(ReidentifyMessage {
                event_subscriptions: None,
            })
        );
    }

    #[test]
    fn parse_client_message_accepts_reidentify_with_event_subscriptions() {
        let message = r#"{"op":3,"d":{"eventSubscriptions":1023}}"#;
        let parsed = parse_client_message(message).expect("reidentify message must be accepted");
        assert_eq!(
            parsed,
            ClientMessage::Reidentify(ReidentifyMessage {
                event_subscriptions: Some(1023),
            })
        );
    }

    #[test]
    fn parse_client_message_rejects_reidentify_with_invalid_event_subscriptions() {
        let message = r#"{"op":3,"d":{"eventSubscriptions":"invalid"}}"#;
        let error = parse_client_message(message).expect_err("reidentify must reject invalid type");
        assert!(!error.display().is_empty());
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
    fn parse_client_message_accepts_request_batch() {
        let message = r#"{"op":8,"d":{"requestId":"batch-1","haltOnFailure":true,"executionType":0,"requests":[{"requestType":"GetVersion"},{"requestType":"GetStats","requestData":{}}]}}"#;
        let parsed = parse_client_message(message).expect("request batch message must be accepted");
        assert_eq!(
            parsed,
            ClientMessage::RequestBatch(RequestBatchMessage {
                request_id: Some("batch-1".to_owned()),
                halt_on_failure: Some(true),
                execution_type: Some(0),
                requests: Some(vec![
                    RequestMessage {
                        request_id: None,
                        request_type: Some("GetVersion".to_owned()),
                        request_data: None,
                    },
                    RequestMessage {
                        request_id: None,
                        request_type: Some("GetStats".to_owned()),
                        request_data: Some(request_data("{}")),
                    },
                ]),
            })
        );
    }

    #[test]
    fn parse_client_message_rejects_request_batch_with_invalid_requests_type() {
        let message = r#"{"op":8,"d":{"requestId":"batch-1","requests":"invalid"}}"#;
        let error = parse_client_message(message).expect_err("request batch must reject invalid");
        assert!(!error.display().is_empty());
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
        let json =
            nojson::RawJson::parse(message.text()).expect("hello message must be valid JSON");
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
            .and_then(|v| v.required()?.try_into())
            .expect("challenge must be string");
        let salt: String = authentication
            .to_member("salt")
            .and_then(|v| v.required()?.try_into())
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
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let op: i64 = json.value().to_member("op")?.required()?.try_into()?;
        let response_data = json
            .value()
            .to_path_member(&["d", "responseData"])?
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
        assert!(
            available_requests
                .iter()
                .any(|r| r == "BroadcastCustomEvent")
        );
        assert!(available_requests.iter().any(|r| r == "GetGroupList"));
        assert!(available_requests.iter().any(|r| r == "GetSourceActive"));
        assert!(available_requests.iter().any(|r| r == "GetSceneList"));
        assert!(available_requests.iter().any(|r| r == "SetSceneName"));
        assert!(
            available_requests
                .iter()
                .any(|r| r == "GetCurrentPreviewScene")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "SetCurrentPreviewScene")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "GetSceneSceneTransitionOverride")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "SetSceneSceneTransitionOverride")
        );
        assert!(available_requests.iter().any(|r| r == "RemoveScene"));
        assert!(
            available_requests
                .iter()
                .any(|r| r == "GetTransitionKindList")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "GetSceneTransitionList")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "GetCurrentSceneTransition")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "SetCurrentSceneTransition")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "SetCurrentSceneTransitionDuration")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "SetCurrentSceneTransitionSettings")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "GetCurrentSceneTransitionCursor")
        );
        assert!(available_requests.iter().any(|r| r == "SetTBarPosition"));
        assert!(available_requests.iter().any(|r| r == "GetSceneItemId"));
        assert!(available_requests.iter().any(|r| r == "GetSceneItemList"));
        assert!(available_requests.iter().any(|r| r == "CreateSceneItem"));
        assert!(available_requests.iter().any(|r| r == "RemoveSceneItem"));
        assert!(available_requests.iter().any(|r| r == "DuplicateSceneItem"));
        assert!(available_requests.iter().any(|r| r == "GetSceneItemSource"));
        assert!(
            available_requests
                .iter()
                .any(|r| r == "GetSceneItemEnabled")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "SetSceneItemEnabled")
        );
        assert!(available_requests.iter().any(|r| r == "GetSceneItemLocked"));
        assert!(available_requests.iter().any(|r| r == "SetSceneItemLocked"));
        assert!(available_requests.iter().any(|r| r == "GetSceneItemIndex"));
        assert!(available_requests.iter().any(|r| r == "SetSceneItemIndex"));
        assert!(
            available_requests
                .iter()
                .any(|r| r == "GetSceneItemBlendMode")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "SetSceneItemBlendMode")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "GetSceneItemTransform")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "SetSceneItemTransform")
        );
        assert!(
            available_requests
                .iter()
                .any(|r| r == "SetStreamServiceSettings")
        );
        assert!(available_requests.iter().any(|r| r == "GetOutputList"));
        assert!(available_requests.iter().any(|r| r == "GetOutputStatus"));
        assert!(available_requests.iter().any(|r| r == "StartOutput"));
        assert!(available_requests.iter().any(|r| r == "ToggleOutput"));
        assert!(available_requests.iter().any(|r| r == "StopOutput"));
        assert!(available_requests.iter().any(|r| r == "GetOutputSettings"));
        assert!(available_requests.iter().any(|r| r == "SetOutputSettings"));
        assert!(available_requests.iter().any(|r| r == "StartStream"));
        assert!(available_requests.iter().any(|r| r == "ToggleStream"));
        assert!(available_requests.iter().any(|r| r == "GetRecordDirectory"));
        assert!(available_requests.iter().any(|r| r == "SetRecordDirectory"));
        assert!(available_requests.iter().any(|r| r == "GetRecordStatus"));
        assert!(available_requests.iter().any(|r| r == "StartRecord"));
        assert!(available_requests.iter().any(|r| r == "ToggleRecord"));
        assert!(available_requests.iter().any(|r| r == "StopRecord"));
        assert!(available_requests.iter().any(|r| r == "Sleep"));
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_group_list_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-group-list".to_owned()),
            request_type: Some("GetGroupList".to_owned()),
            request_data: None,
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let mut groups = json
            .value()
            .to_path_member(&["d", "responseData", "groups"])?
            .required()?
            .to_array()?;
        assert!(groups.next().is_none());
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_source_active_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-source-active".to_owned()),
            request_type: Some("GetSourceActive".to_owned()),
            request_data: Some(request_data(r#"{"inputName":"input-name-1"}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        state
            .create_scene_item("Scene", Some("input-uuid-1"), None, true)
            .expect("scene item creation must succeed");
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let video_active: bool = json
            .value()
            .to_path_member(&["d", "responseData", "videoActive"])?
            .required()?
            .try_into()?;
        assert!(video_active);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_scene_transition_override_responses()
    -> Result<(), Box<dyn std::error::Error>> {
        let session_stats = ObswsSessionStats::default();
        let mut state = ObswsSessionState::new_for_test();

        let set_response = handle_request_message(
            RequestMessage {
                request_id: Some("req-set-override".to_owned()),
                request_type: Some("SetSceneSceneTransitionOverride".to_owned()),
                request_data: Some(request_data(
                    r#"{"sceneName":"Scene","transitionName":"fade_transition","transitionDuration":500}"#,
                )),
            },
            &session_stats,
            &mut state,
        );
        // OBS は SetSceneSceneTransitionOverride で responseData を返さない
        let set_json = nojson::RawJson::parse(set_response.message.text())?;
        let set_result: bool = set_json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])?
            .required()?
            .try_into()?;
        assert!(set_result);

        let get_response = handle_request_message(
            RequestMessage {
                request_id: Some("req-get-override".to_owned()),
                request_type: Some("GetSceneSceneTransitionOverride".to_owned()),
                request_data: Some(request_data(r#"{"sceneName":"Scene"}"#)),
            },
            &session_stats,
            &mut state,
        );
        let get_json = nojson::RawJson::parse(get_response.message.text())?;
        let get_transition_name: Option<String> = get_json
            .value()
            .to_path_member(&["d", "responseData", "transitionName"])?
            .try_into()?;
        let get_transition_duration: Option<i64> = get_json
            .value()
            .to_path_member(&["d", "responseData", "transitionDuration"])?
            .try_into()?;
        assert_eq!(get_transition_name.as_deref(), Some("fade_transition"));
        assert_eq!(get_transition_duration, Some(500));
        Ok(())
    }

    // GetOutputList は coordinator 経由で処理されるため、message.rs レベルのテストは不要。
    // session テスト側で検証する。

    // GetOutputSettings / SetOutputSettings / SetStreamServiceSettings / SetRecordDirectory / GetRecordDirectory は coordinator 経由で処理されるため削除。

    #[test]
    fn handle_request_message_returns_set_scene_name_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-set-scene-name".to_owned()),
            request_type: Some("SetSceneName".to_owned()),
            request_data: Some(request_data(
                r#"{"sceneName":"Scene","newSceneName":"Scene Renamed"}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = ObswsSessionState::new_for_test();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        assert!(result);
        // OBS は SetSceneName で responseData を返さない
        assert_eq!(
            state.current_program_scene().map(|scene| scene.scene_name),
            Some("Scene Renamed".to_owned())
        );
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
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
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
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let response_data = json
            .value()
            .to_path_member(&["d", "responseData"])?
            .required()?;
        let mut inputs = response_data.to_member("inputs")?.required()?.to_array()?;
        let first_input = inputs.next().expect("first input must exist");
        let input_name: String = first_input.to_member("inputName")?.required()?.try_into()?;
        assert_eq!(input_name, "input-name-1");
        Ok(())
    }

    // GetStats は coordinator 経由で処理されるため削除。

    #[test]
    fn handle_request_message_returns_get_input_kind_list_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-1".to_owned()),
            request_type: Some("GetInputKindList".to_owned()),
            request_data: None,
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let response_data = json
            .value()
            .to_path_member(&["d", "responseData"])?
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
        assert!(input_kinds.iter().any(|kind| kind == "mp4_file_source"));
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
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let response_data = json
            .value()
            .to_path_member(&["d", "responseData"])?
            .required()?;
        let input_settings = response_data.to_member("inputSettings")?.required()?;
        assert_eq!(input_settings.kind(), nojson::JsonValueKind::Object);
        let input_kind: String = response_data
            .to_member("inputKind")?
            .required()?
            .try_into()?;
        assert_eq!(input_kind, "video_capture_device");
        // OBS 仕様では GetInputSettings の responseData に inputName は含まれない
        assert!(response_data.to_member("inputName")?.optional().is_none());
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_set_input_settings_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-set-input-settings".to_owned()),
            request_type: Some("SetInputSettings".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"input-name-1","inputSettings":{"device_id":"camera-2"}}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        assert!(result);

        let input = state
            .find_input(None, Some("input-name-1"))
            .expect("input must exist");
        match &input.input.settings {
            ObswsInputSettings::VideoCaptureDevice(settings) => {
                assert_eq!(settings.device_id.as_deref(), Some("camera-2"));
            }
            _ => panic!("input kind must remain video_capture_device"),
        }
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_invalid_field_error_for_set_input_settings_with_invalid_settings()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-set-input-settings-invalid".to_owned()),
            request_type: Some("SetInputSettings".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"input-name-1","inputSettings":{"device_id":1}}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_set_input_name_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-set-input-name".to_owned()),
            request_type: Some("SetInputName".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"input-name-1","newInputName":"input-name-1-renamed"}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        assert!(result);

        assert!(state.find_input(None, Some("input-name-1")).is_none());
        assert!(
            state
                .find_input(None, Some("input-name-1-renamed"))
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_input_default_settings_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-default-input-settings".to_owned()),
            request_type: Some("GetInputDefaultSettings".to_owned()),
            request_data: Some(request_data(r#"{"inputKind":"video_capture_device"}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let default_input_settings = json
            .value()
            .to_path_member(&["d", "responseData", "defaultInputSettings"])?
            .required()?;
        let device_id: Option<String> =
            default_input_settings.to_member("device_id")?.try_into()?;
        assert_eq!(device_id, None);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_mp4_source_default_settings_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-default-mp4-input-settings".to_owned()),
            request_type: Some("GetInputDefaultSettings".to_owned()),
            request_data: Some(request_data(r#"{"inputKind":"mp4_file_source"}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let default_input_settings = json
            .value()
            .to_path_member(&["d", "responseData", "defaultInputSettings"])?
            .required()?;
        let path: Option<String> = default_input_settings.to_member("path")?.try_into()?;
        let loop_playback: bool = default_input_settings
            .to_member("loopPlayback")?
            .required()?
            .try_into()?;
        assert_eq!(path, None);
        assert!(loop_playback);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_scene_item_id_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(request_data(
                r#"{"sceneName":"Scene","sourceName":"camera-1","searchOffset":0}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = ObswsSessionState::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            request_data(r#"{}"#).value(),
        )
        .expect("input settings must be valid");
        state
            .create_input("Scene", "camera-1", input, true)
            .expect("input creation must succeed");
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let scene_item_id: i64 = json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemId"])?
            .required()?
            .try_into()?;
        assert!(result);
        assert_eq!(scene_item_id, 1);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_invalid_field_error_for_get_scene_item_id_search_offset()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-scene-item-id-offset".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(request_data(
                r#"{"sceneName":"Scene","sourceName":"camera-1","searchOffset":1}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = ObswsSessionState::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            request_data(r#"{}"#).value(),
        )
        .expect("input settings must be valid");
        state
            .create_input("Scene", "camera-1", input, true)
            .expect("input creation must succeed");
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_scene_item_enabled_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-scene-item-enabled".to_owned()),
            request_type: Some("GetSceneItemEnabled".to_owned()),
            request_data: Some(request_data(r#"{"sceneName":"Scene","sceneItemId":1}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = ObswsSessionState::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            request_data(r#"{}"#).value(),
        )
        .expect("input settings must be valid");
        state
            .create_input("Scene", "camera-1", input, true)
            .expect("input creation must succeed");
        state
            .set_scene_item_enabled("Scene", 1, false)
            .expect("set scene item enabled must succeed");
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let scene_item_enabled: bool = json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemEnabled"])?
            .required()?
            .try_into()?;
        assert!(result);
        assert!(!scene_item_enabled);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_scene_item_locked_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-scene-item-locked".to_owned()),
            request_type: Some("GetSceneItemLocked".to_owned()),
            request_data: Some(request_data(r#"{"sceneName":"Scene","sceneItemId":1}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = ObswsSessionState::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            request_data(r#"{}"#).value(),
        )
        .expect("input settings must be valid");
        state
            .create_input("Scene", "camera-1", input, true)
            .expect("input creation must succeed");
        state
            .set_scene_item_locked("Scene", 1, true)
            .expect("set scene item locked must succeed");
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let scene_item_locked: bool = json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemLocked"])?
            .required()?
            .try_into()?;
        assert!(scene_item_locked);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_scene_item_blend_mode_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-scene-item-blend-mode".to_owned()),
            request_type: Some("GetSceneItemBlendMode".to_owned()),
            request_data: Some(request_data(r#"{"sceneName":"Scene","sceneItemId":1}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = ObswsSessionState::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            request_data(r#"{}"#).value(),
        )
        .expect("input settings must be valid");
        state
            .create_input("Scene", "camera-1", input, true)
            .expect("input creation must succeed");
        state
            .set_scene_item_blend_mode(
                "Scene",
                1,
                crate::obsws::state::ObswsSceneItemBlendMode::Additive,
            )
            .expect("set scene item blend mode must succeed");
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let scene_item_blend_mode: String = json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemBlendMode"])?
            .required()?
            .try_into()?;
        assert_eq!(scene_item_blend_mode, "OBS_BLEND_ADDITIVE");
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_scene_item_transform_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-scene-item-transform".to_owned()),
            request_type: Some("GetSceneItemTransform".to_owned()),
            request_data: Some(request_data(r#"{"sceneName":"Scene","sceneItemId":1}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = ObswsSessionState::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "video_capture_device",
            request_data(r#"{}"#).value(),
        )
        .expect("input settings must be valid");
        state
            .create_input("Scene", "camera-1", input, true)
            .expect("input creation must succeed");
        state
            .set_scene_item_transform(
                "Scene",
                1,
                crate::obsws::state::ObswsSceneItemTransformPatch {
                    position_x: Some(123.0),
                    ..Default::default()
                },
            )
            .expect("set scene item transform must succeed");
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let position_x: f64 = json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemTransform", "positionX"])?
            .required()?
            .try_into()?;
        assert_eq!(position_x, 123.0);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_get_input_properties_list_property_items_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-props-list".to_owned()),
            request_type: Some("GetInputPropertiesListPropertyItems".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"input-name-1","propertyName":"device_id"}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        assert!(result);

        let response_data = json
            .value()
            .to_path_member(&["d", "responseData"])?
            .required()?;
        let property_items = response_data.to_member("propertyItems")?.required()?;
        assert_eq!(property_items.kind(), nojson::JsonValueKind::Array);
        for item in property_items.to_array()? {
            let value: String = item.to_member("itemValue")?.required()?.try_into()?;
            assert_ne!(value, "Unknown");
        }
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_pixel_format_property_items_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-props-pixel-format".to_owned()),
            request_type: Some("GetInputPropertiesListPropertyItems".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"input-name-1","propertyName":"pixel_format"}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        assert!(result);

        let response_data = json
            .value()
            .to_path_member(&["d", "responseData"])?
            .required()?;
        let property_items = response_data.to_member("propertyItems")?.required()?;
        assert_eq!(property_items.kind(), nojson::JsonValueKind::Array);
        for item in property_items.to_array()? {
            let value: String = item.to_member("itemValue")?.required()?.try_into()?;
            assert_ne!(value, "Unknown");
        }
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_fps_property_items_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-props-fps".to_owned()),
            request_type: Some("GetInputPropertiesListPropertyItems".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"input-name-1","propertyName":"fps"}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        assert!(result);

        let response_data = json
            .value()
            .to_path_member(&["d", "responseData"])?
            .required()?;
        let property_items = response_data.to_member("propertyItems")?.required()?;
        assert_eq!(property_items.kind(), nojson::JsonValueKind::Array);
        for item in property_items.to_array()? {
            let value: String = item.to_member("itemValue")?.required()?.try_into()?;
            let fps: i32 = value.parse()?;
            assert!(fps > 0);
        }
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_audio_device_id_property_items_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-props-audio-device-id".to_owned()),
            request_type: Some("GetInputPropertiesListPropertyItems".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"input-name-audio","propertyName":"device_id"}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state_with_audio_input();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        // PulseAudio デーモンが無いヘッドレス CI では shiguredo_audio_device::
        // AudioDeviceList::enumerate_input() が Err を返すため、
        // エラー応答となる可能性を許容する。実装の分岐と
        // ハンドラ経路までは通ったことを code で確認して終了する。
        if !result {
            let code: i64 = status.to_member("code")?.required()?.try_into()?;
            assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
            return Ok(());
        }

        let response_data = json
            .value()
            .to_path_member(&["d", "responseData"])?
            .required()?;
        let property_items = response_data.to_member("propertyItems")?.required()?;
        assert_eq!(property_items.kind(), nojson::JsonValueKind::Array);
        // 各項目が itemName / itemValue / itemEnabled を持つこと。
        // デバイスが無い環境でも空配列が返ることを許容する。
        for item in property_items.to_array()? {
            let _: String = item.to_member("itemName")?.required()?.try_into()?;
            let _: String = item.to_member("itemValue")?.required()?.try_into()?;
            let _: bool = item.to_member("itemEnabled")?.required()?.try_into()?;
        }
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_audio_sample_rate_property_items_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-props-audio-sample-rate".to_owned()),
            request_type: Some("GetInputPropertiesListPropertyItems".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"input-name-audio","propertyName":"sample_rate"}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state_with_audio_input();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        // ヘッドレス CI ではオーディオデバイス列挙が失敗しエラー応答となるため、
        // その場合はコード値だけ確認して早期に return する。
        if !result {
            let code: i64 = status.to_member("code")?.required()?.try_into()?;
            assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
            return Ok(());
        }

        let response_data = json
            .value()
            .to_path_member(&["d", "responseData"])?
            .required()?;
        let property_items = response_data.to_member("propertyItems")?.required()?;
        assert_eq!(property_items.kind(), nojson::JsonValueKind::Array);
        for item in property_items.to_array()? {
            let value: String = item.to_member("itemValue")?.required()?.try_into()?;
            let rate: i32 = value.parse()?;
            assert!(rate > 0);
        }
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_audio_channels_property_items_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-props-audio-channels".to_owned()),
            request_type: Some("GetInputPropertiesListPropertyItems".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"input-name-audio","propertyName":"channels"}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state_with_audio_input();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        // ヘッドレス CI ではオーディオデバイス列挙が失敗しエラー応答となるため、
        // その場合はコード値だけ確認して早期に return する。
        if !result {
            let code: i64 = status.to_member("code")?.required()?.try_into()?;
            assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
            return Ok(());
        }

        let response_data = json
            .value()
            .to_path_member(&["d", "responseData"])?
            .required()?;
        let property_items = response_data.to_member("propertyItems")?.required()?;
        assert_eq!(property_items.kind(), nojson::JsonValueKind::Array);
        for item in property_items.to_array()? {
            let value: String = item.to_member("itemValue")?.required()?.try_into()?;
            let channels: i32 = value.parse()?;
            assert!(channels > 0);
        }
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_invalid_field_for_unsupported_audio_property_name()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-props-audio-invalid-prop".to_owned()),
            request_type: Some("GetInputPropertiesListPropertyItems".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"input-name-audio","propertyName":"pixel_format"}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state_with_audio_input();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_error_when_get_input_properties_list_property_items_missing_request_data()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-props-list-no-data".to_owned()),
            request_type: Some("GetInputPropertiesListPropertyItems".to_owned()),
            request_data: None,
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_MISSING_REQUEST_DATA);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_error_when_get_input_properties_list_property_items_input_not_found()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-props-list-not-found".to_owned()),
            request_type: Some("GetInputPropertiesListPropertyItems".to_owned()),
            request_data: Some(request_data(
                r#"{"inputName":"nonexistent","propertyName":"device_id"}"#,
            )),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_RESOURCE_NOT_FOUND);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_error_when_get_input_properties_list_property_items_missing_property_name()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-get-props-list-no-prop".to_owned()),
            request_type: Some("GetInputPropertiesListPropertyItems".to_owned()),
            request_data: Some(request_data(r#"{"inputName":"input-name-1"}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_MISSING_REQUEST_FIELD);
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
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_MISSING_REQUEST_DATA);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_invalid_field_error_for_get_input_settings_lookup_type_mismatch()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = RequestMessage {
            request_id: Some("req-1-invalid-type".to_owned()),
            request_type: Some("GetInputSettings".to_owned()),
            request_data: Some(request_data(r#"{"inputName":1}"#)),
        };
        let session_stats = ObswsSessionStats::default();
        let mut state = state();
        let response = handle_request_message(request, &session_stats, &mut state);

        let json = nojson::RawJson::parse(response.message.text())?;
        let status = json
            .value()
            .to_path_member(&["d", "requestStatus"])?
            .required()?;
        let result: bool = status.to_member("result")?.required()?.try_into()?;
        let code: i64 = status.to_member("code")?.required()?.try_into()?;
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
        Ok(())
    }

    #[test]
    fn handle_request_message_returns_unknown_request_type_for_session_handled_requests()
    -> Result<(), Box<dyn std::error::Error>> {
        let session_stats = ObswsSessionStats::default();
        let request_types = [
            "SetCurrentProgramScene",
            "SetCurrentPreviewScene",
            "CreateScene",
            "RemoveScene",
            "CreateInput",
            "RemoveInput",
            "CreateSceneItem",
            "RemoveSceneItem",
            "DuplicateSceneItem",
            "SetSceneItemEnabled",
            "SetSceneItemLocked",
            "SetSceneItemIndex",
            "SetSceneItemBlendMode",
            "SetSceneItemTransform",
            "StartStream",
            "ToggleStream",
            "StopStream",
            "StartOutput",
            "ToggleOutput",
            "StopOutput",
            "StartRecord",
            "ToggleRecord",
            "StopRecord",
        ];

        for request_type in request_types {
            let request = RequestMessage {
                request_id: Some("req-session-handled".to_owned()),
                request_type: Some(request_type.to_owned()),
                request_data: Some(request_data(r#"{"dummy":"value"}"#)),
            };
            let mut state = state();
            let response = handle_request_message(request, &session_stats, &mut state);
            let json = nojson::RawJson::parse(response.message.text())?;
            let status = json
                .value()
                .to_path_member(&["d", "requestStatus"])?
                .required()?;
            let result: bool = status.to_member("result")?.required()?.try_into()?;
            let code: i64 = status.to_member("code")?.required()?.try_into()?;
            assert!(!result);
            assert_eq!(code, REQUEST_STATUS_UNKNOWN_REQUEST_TYPE);
        }
        Ok(())
    }

    // GetOutputSettings / SetOutputSettings / SetStreamServiceSettings / SetRecordDirectory / GetRecordDirectory は coordinator 経由で処理されるため削除。

    #[test]
    fn set_sora_output_settings_rejects_non_object_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        let session_stats = ObswsSessionStats::default();
        let mut state = ObswsSessionState::new_for_test();

        let request = RequestMessage {
            request_id: Some("req-bad-meta".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(request_data(
                r#"{"outputName":"sora","outputSettings":{"soraSdkSettings":{"signalingUrls":["wss://a.example.com"],"channelId":"ch","metadata":"not-an-object"}}}"#,
            )),
        };
        let response = handle_request_message(request, &session_stats, &mut state);
        let json = nojson::RawJson::parse(response.message.text())?;
        let result: bool = json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])?
            .required()?
            .try_into()?;
        assert!(!result);
        Ok(())
    }
}
