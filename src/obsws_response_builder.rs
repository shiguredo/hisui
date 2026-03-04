use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::{
    CreateInputError, ObswsInput, ObswsInputRegistry, ParseInputSettingsError,
};
use crate::obsws_message::ObswsSessionStats;
use crate::obsws_protocol::{
    OBSWS_DEFAULT_SCENE_NAME, OBSWS_OP_HELLO, OBSWS_OP_IDENTIFIED, OBSWS_OP_REQUEST_RESPONSE,
    OBSWS_RPC_VERSION, OBSWS_SUPPORTED_IMAGE_FORMATS, OBSWS_VERSION,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
    REQUEST_STATUS_RESOURCE_NOT_FOUND, REQUEST_STATUS_SUCCESS,
};

struct CreateInputFields {
    scene_name: String,
    input_name: String,
    input: ObswsInput,
}

fn parse_input_lookup_fields(
    request_data: Option<&nojson::RawJsonOwned>,
) -> Result<(Option<String>, Option<String>), &'static str> {
    let Some(request_data) = request_data else {
        return Err("Missing required requestData field");
    };
    let request_data = request_data.value();
    if request_data.kind() != nojson::JsonValueKind::Object {
        return Err("Invalid requestData field");
    }

    let input_name = optional_non_empty_string_member(request_data, "inputName")?;
    let input_uuid = optional_non_empty_string_member(request_data, "inputUuid")?;

    if input_name.is_none() && input_uuid.is_none() {
        return Err("Missing required inputName or inputUuid field");
    }

    Ok((input_uuid, input_name))
}

fn optional_non_empty_string_member(
    object: nojson::RawJsonValue<'_, '_>,
    member_name: &str,
) -> Result<Option<String>, &'static str> {
    let value = object
        .to_member(member_name)
        .map_err(|_| "Invalid requestData field")?
        .get();
    let Some(value) = value else {
        return Ok(None);
    };
    if value.kind() != nojson::JsonValueKind::String {
        return Ok(None);
    }
    let value: String = value.try_into().map_err(|_| "Invalid requestData field")?;
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some(value))
}

fn parse_create_input_fields(
    request_data: Option<&nojson::RawJsonOwned>,
) -> Result<CreateInputFields, String> {
    let Some(request_data) = request_data else {
        return Err("Missing required requestData field".to_owned());
    };
    let request_data = request_data.value();
    if request_data.kind() != nojson::JsonValueKind::Object {
        return Err("Invalid requestData field".to_owned());
    }

    let scene_name = required_non_empty_string_member(request_data, "sceneName")?;
    let input_name = required_non_empty_string_member(request_data, "inputName")?;
    let input_kind = required_non_empty_string_member(request_data, "inputKind")?;
    let input_settings = request_data
        .to_member("inputSettings")
        .map_err(|_| "Invalid requestData field".to_owned())?
        .required()
        .map_err(|_| "Missing required inputSettings field".to_owned())?;
    let input =
        ObswsInput::from_kind_and_settings(&input_kind, input_settings).map_err(|e| match e {
            ParseInputSettingsError::UnsupportedInputKind => {
                "Unsupported inputKind field".to_owned()
            }
            ParseInputSettingsError::InvalidInputSettings(message) => message,
        })?;

    Ok(CreateInputFields {
        scene_name,
        input_name,
        input,
    })
}

fn required_non_empty_string_member(
    object: nojson::RawJsonValue<'_, '_>,
    member_name: &str,
) -> Result<String, String> {
    let value = object
        .to_member(member_name)
        .map_err(|_| "Invalid requestData field".to_owned())?
        .required()
        .map_err(|_| format!("Missing required {member_name} field"))?;
    if value.kind() != nojson::JsonValueKind::String {
        return Err(format!("Invalid {member_name} field: string is required"));
    }
    let value: String = value
        .try_into()
        .map_err(|_| format!("Invalid {member_name} field"))?;
    if value.is_empty() {
        return Err(format!("Missing required {member_name} field"));
    }
    Ok(value)
}

pub fn build_hello_message(authentication: Option<&ObswsAuthentication>) -> String {
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

pub fn build_identified_message(negotiated_rpc_version: u32) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_IDENTIFIED)?;
        f.member(
            "d",
            nojson::object(|f| f.member("negotiatedRpcVersion", negotiated_rpc_version)),
        )
    })
    .to_string()
}

pub fn build_get_version_response(request_id: &str) -> String {
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
                                "CreateInput",
                                "RemoveInput",
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

pub fn build_get_stats_response(request_id: &str, session_stats: &ObswsSessionStats) -> String {
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

pub fn build_get_canvas_list_response(request_id: &str) -> String {
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

pub fn build_get_input_list_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
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

pub fn build_get_input_kind_list_response(
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

pub fn build_get_input_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
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

    let Some(input) = input_registry.find_input(input_uuid.as_deref(), input_name.as_deref())
    else {
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
                        f.member("inputKind", input.input.kind_name())?;
                        f.member("inputSettings", &input.input.settings)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_create_input_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_create_input_fields(request_data) {
        Ok(fields) => fields,
        Err(message) => {
            return build_request_response_error(
                "CreateInput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                &message,
            );
        }
    };

    let created = match input_registry.create_input(
        &fields.scene_name,
        &fields.input_name,
        fields.input,
    ) {
        Ok(created) => created,
        Err(CreateInputError::UnsupportedSceneName) => {
            return build_request_response_error(
                "CreateInput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                &format!(
                    "Unsupported sceneName field: only '{OBSWS_DEFAULT_SCENE_NAME}' is supported"
                ),
            );
        }
        Err(CreateInputError::InputNameAlreadyExists) => {
            return build_request_response_error(
                "CreateInput",
                request_id,
                REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
                "Input already exists",
            );
        }
    };
    let input_uuid = created.input_uuid;

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "CreateInput")?;
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
                    nojson::object(|f| f.member("inputUuid", &input_uuid)),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_remove_input_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let (input_uuid, input_name) = match parse_input_lookup_fields(request_data) {
        Ok(v) => v,
        Err(message) => {
            return build_request_response_error(
                "RemoveInput",
                request_id,
                REQUEST_STATUS_MISSING_REQUEST_FIELD,
                message,
            );
        }
    };
    let Some(_removed) = input_registry.remove_input(input_uuid.as_deref(), input_name.as_deref())
    else {
        return build_request_response_error(
            "RemoveInput",
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
                f.member("requestType", "RemoveInput")?;
                f.member("requestId", request_id)?;
                f.member(
                    "requestStatus",
                    nojson::object(|f| {
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member("responseData", nojson::object(|_| Ok(())))
            }),
        )
    })
    .to_string()
}

pub fn build_request_response_error(
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
