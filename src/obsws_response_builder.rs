use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::{
    CreateInputError, CreateSceneError, ObswsInput, ObswsInputRegistry, ObswsStreamServiceSettings,
    ParseInputSettingsError, SetCurrentProgramSceneError,
};
use crate::obsws_message::ObswsSessionStats;
use crate::obsws_protocol::{
    OBSWS_OP_HELLO, OBSWS_OP_IDENTIFIED, OBSWS_OP_REQUEST_RESPONSE, OBSWS_RPC_VERSION,
    OBSWS_SUPPORTED_IMAGE_FORMATS, OBSWS_VERSION, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
    REQUEST_STATUS_RESOURCE_NOT_FOUND, REQUEST_STATUS_SUCCESS,
};
use std::path::PathBuf;

struct CreateInputFields {
    scene_name: String,
    input_name: String,
    input: ObswsInput,
    scene_item_enabled: bool,
}

struct CreateSceneFields {
    scene_name: String,
}

struct SetCurrentProgramSceneFields {
    scene_name: String,
}

struct SetStreamServiceSettingsFields {
    stream_service_type: String,
    server: String,
    key: Option<String>,
}

struct SetRecordDirectoryFields {
    record_directory: String,
}

fn parse_input_lookup_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<(Option<String>, Option<String>), nojson::JsonParseError> {
    let input_name = optional_non_empty_string_member(request_data, "inputName")?;
    let input_uuid = optional_non_empty_string_member(request_data, "inputUuid")?;

    if input_name.is_none() && input_uuid.is_none() {
        return Err(request_data.invalid("required member 'inputName or inputUuid' is missing"));
    }

    Ok((input_uuid, input_name))
}

fn optional_non_empty_string_member(
    object: nojson::RawJsonValue<'_, '_>,
    member_name: &str,
) -> Result<Option<String>, nojson::JsonParseError> {
    let value = object.to_member(member_name)?.get();
    let Some(value) = value else {
        return Ok(None);
    };
    let value: String = value.try_into()?;
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some(value))
}

fn parse_create_input_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<CreateInputFields, nojson::JsonParseError> {
    let scene_name = required_non_empty_string_member(request_data, "sceneName")?;
    let input_name = required_non_empty_string_member(request_data, "inputName")?;
    let input_kind = required_non_empty_string_member(request_data, "inputKind")?;
    let input_settings = request_data.to_member("inputSettings")?.required()?;

    let input = match ObswsInput::from_kind_and_settings(&input_kind, input_settings) {
        Ok(input) => input,
        Err(ParseInputSettingsError::UnsupportedInputKind) => {
            return Err(request_data
                .to_member("inputKind")?
                .required()?
                .invalid("Unsupported inputKind field"));
        }
        Err(ParseInputSettingsError::InvalidInputSettings(message)) => {
            return Err(input_settings.invalid(message));
        }
    };
    let scene_item_enabled: Option<bool> =
        request_data.to_member("sceneItemEnabled")?.try_into()?;

    Ok(CreateInputFields {
        scene_name,
        input_name,
        input,
        scene_item_enabled: scene_item_enabled.unwrap_or(true),
    })
}

fn parse_create_scene_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<CreateSceneFields, nojson::JsonParseError> {
    let scene_name = required_non_empty_string_member(request_data, "sceneName")?;
    Ok(CreateSceneFields { scene_name })
}

fn parse_set_current_program_scene_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetCurrentProgramSceneFields, nojson::JsonParseError> {
    let scene_name = required_non_empty_string_member(request_data, "sceneName")?;
    Ok(SetCurrentProgramSceneFields { scene_name })
}

fn parse_set_stream_service_settings_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetStreamServiceSettingsFields, nojson::JsonParseError> {
    let stream_service_type = required_non_empty_string_member(request_data, "streamServiceType")?;
    let stream_service_settings = request_data
        .to_member("streamServiceSettings")?
        .required()?;
    let server = required_non_empty_string_member(stream_service_settings, "server")?;
    let key = optional_non_empty_string_member(stream_service_settings, "key")?;

    Ok(SetStreamServiceSettingsFields {
        stream_service_type,
        server,
        key,
    })
}

fn parse_set_record_directory_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetRecordDirectoryFields, nojson::JsonParseError> {
    let record_directory = required_non_empty_string_member(request_data, "recordDirectory")?;
    Ok(SetRecordDirectoryFields { record_directory })
}

fn required_non_empty_string_member(
    object: nojson::RawJsonValue<'_, '_>,
    member_name: &str,
) -> Result<String, nojson::JsonParseError> {
    let raw_value = object.to_member(member_name)?.required()?;
    let value: String = raw_value.try_into()?;
    if value.is_empty() {
        return Err(raw_value.invalid("string must not be empty"));
    }
    Ok(value)
}

fn parse_request_data_or_error_response<T, F>(
    request_type: &str,
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    parser: F,
) -> Result<T, String>
where
    F: FnOnce(nojson::RawJsonValue<'_, '_>) -> Result<T, nojson::JsonParseError>,
{
    let Some(request_data) = request_data else {
        return Err(build_request_response_error(
            request_type,
            request_id,
            REQUEST_STATUS_MISSING_REQUEST_FIELD,
            "Missing required requestData field",
        ));
    };

    parser(request_data.value()).map_err(|e| {
        let code = request_status_code_for_parse_error(&e);
        build_request_response_error(request_type, request_id, code, &e.to_string())
    })
}

fn request_status_code_for_parse_error(error: &nojson::JsonParseError) -> i64 {
    // OBS WebSocket の 300 / 400 の厳密分類は nojson のエラー種別だけでは判別しづらいため、
    // 現状は required member 欠落パターンのみ 300 として扱い、それ以外は 400 とする
    // 将来的に厳密化する場合は、パーサー側で欠落と型不一致を明示的に分離する
    if let nojson::JsonParseError::InvalidValue { error, .. } = error {
        let reason = error.to_string();
        if reason.contains("required member") && reason.contains("is missing") {
            return REQUEST_STATUS_MISSING_REQUEST_FIELD;
        }
    }
    REQUEST_STATUS_INVALID_REQUEST_FIELD
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
                                "GetSceneList",
                                "CreateScene",
                                "GetCurrentProgramScene",
                                "SetCurrentProgramScene",
                                "GetInputList",
                                "GetInputKindList",
                                "GetInputSettings",
                                "CreateInput",
                                "RemoveInput",
                                "GetStreamServiceSettings",
                                "SetStreamServiceSettings",
                                "GetStreamStatus",
                                "StartStream",
                                "StopStream",
                                "GetRecordDirectory",
                                "SetRecordDirectory",
                                "GetRecordStatus",
                                "StartRecord",
                                "StopRecord",
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

pub fn build_get_scene_list_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let scenes = input_registry.list_scenes();
    let current_program_scene = input_registry.current_program_scene();
    let current_program_scene_name = current_program_scene
        .as_ref()
        .map(|scene| scene.scene_name.as_str())
        .unwrap_or_default();
    let current_program_scene_uuid = current_program_scene
        .as_ref()
        .map(|scene| scene.scene_uuid.as_str())
        .unwrap_or_default();
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSceneList")?;
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
                        f.member("currentProgramSceneName", current_program_scene_name)?;
                        f.member("currentProgramSceneUuid", current_program_scene_uuid)?;
                        // 現時点は preview scene を独立管理していないため、program scene と同じ値を返す。
                        f.member("currentPreviewSceneName", current_program_scene_name)?;
                        f.member("currentPreviewSceneUuid", current_program_scene_uuid)?;
                        f.member("scenes", &scenes)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_get_current_program_scene_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let current_program_scene = input_registry.current_program_scene();
    let scene_name = current_program_scene
        .as_ref()
        .map(|scene| scene.scene_name.as_str())
        .unwrap_or_default();
    let scene_uuid = current_program_scene
        .as_ref()
        .map(|scene| scene.scene_uuid.as_str())
        .unwrap_or_default();
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetCurrentProgramScene")?;
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
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)?;
                        // 互換目的で currentProgramSceneName/currentProgramSceneUuid も返す。
                        f.member("currentProgramSceneName", scene_name)?;
                        f.member("currentProgramSceneUuid", scene_uuid)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_set_current_program_scene_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetCurrentProgramScene",
        request_id,
        request_data,
        parse_set_current_program_scene_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    if let Err(SetCurrentProgramSceneError::SceneNotFound) =
        input_registry.set_current_program_scene(&fields.scene_name)
    {
        return build_request_response_error(
            "SetCurrentProgramScene",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Scene not found",
        );
    }
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetCurrentProgramScene")?;
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

pub fn build_create_scene_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "CreateScene",
        request_id,
        request_data,
        parse_create_scene_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let created = match input_registry.create_scene(&fields.scene_name) {
        Ok(created) => created,
        Err(CreateSceneError::SceneNameAlreadyExists) => {
            return build_request_response_error(
                "CreateScene",
                request_id,
                REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
                "Scene already exists",
            );
        }
    };
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "CreateScene")?;
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
                        f.member("sceneName", &created.scene_name)?;
                        f.member("sceneUuid", &created.scene_uuid)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_get_stream_service_settings_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let settings = input_registry.stream_service_settings();
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetStreamServiceSettings")?;
                f.member("requestId", request_id)?;
                f.member(
                    "requestStatus",
                    nojson::object(|f| {
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member("responseData", settings)
            }),
        )
    })
    .to_string()
}

pub fn build_set_stream_service_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetStreamServiceSettings",
        request_id,
        request_data,
        parse_set_stream_service_settings_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    input_registry.set_stream_service_settings(ObswsStreamServiceSettings {
        stream_service_type: fields.stream_service_type,
        server: Some(fields.server),
        key: fields.key,
    });
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetStreamServiceSettings")?;
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

pub fn build_get_stream_status_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let active = input_registry.is_stream_active();
    let duration = if active {
        input_registry.stream_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetStreamStatus")?;
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
                        f.member("outputActive", active)?;
                        f.member("outputReconnecting", false)?;
                        f.member("outputTimecode", &output_timecode)?;
                        f.member("outputDuration", output_duration)?;
                        f.member("outputCongestion", 0.0)?;
                        f.member("outputBytes", 0)?;
                        f.member("outputSkippedFrames", 0)?;
                        f.member("outputTotalFrames", 0)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_get_record_directory_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let record_directory = input_registry.record_directory().display().to_string();
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetRecordDirectory")?;
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
                    nojson::object(|f| f.member("recordDirectory", &record_directory)),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_set_record_directory_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetRecordDirectory",
        request_id,
        request_data,
        parse_set_record_directory_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let record_directory = match resolve_record_directory_path(&fields.record_directory) {
        Ok(path) => path,
        Err(e) => {
            return build_request_response_error(
                "SetRecordDirectory",
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                &e,
            );
        }
    };
    input_registry.set_record_directory(record_directory);
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetRecordDirectory")?;
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

pub fn build_get_record_status_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let active = input_registry.is_record_active();
    let duration = if active {
        input_registry.record_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    let output_path = input_registry
        .record_output_path()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetRecordStatus")?;
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
                        f.member("outputActive", active)?;
                        f.member("outputPaused", false)?;
                        f.member("outputTimecode", &output_timecode)?;
                        f.member("outputDuration", output_duration)?;
                        f.member("outputBytes", 0)?;
                        f.member("outputSkippedFrames", 0)?;
                        f.member("outputTotalFrames", 0)?;
                        f.member("outputPath", &output_path)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_start_stream_response(request_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "StartStream")?;
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

pub fn build_stop_stream_response(request_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "StopStream")?;
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

pub fn build_start_record_response(request_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "StartRecord")?;
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

pub fn build_stop_record_response(request_id: &str, output_path: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "StopRecord")?;
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
                    nojson::object(|f| f.member("outputPath", output_path)),
                )
            }),
        )
    })
    .to_string()
}

fn format_timecode(duration: std::time::Duration) -> String {
    let total_millis = duration.as_millis();
    let millis = total_millis % 1_000;
    let total_secs = total_millis / 1_000;
    let secs = total_secs % 60;
    let total_minutes = total_secs / 60;
    let minutes = total_minutes % 60;
    let hours = total_minutes / 60;
    format!("{hours:02}:{minutes:02}:{secs:02}.{millis:03}")
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
    let (input_uuid, input_name) = match parse_request_data_or_error_response(
        "GetInputSettings",
        request_id,
        request_data,
        parse_input_lookup_fields,
    ) {
        Ok(v) => v,
        Err(response) => return response,
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
    let fields = match parse_request_data_or_error_response(
        "CreateInput",
        request_id,
        request_data,
        parse_create_input_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };

    let created = match input_registry.create_input(
        &fields.scene_name,
        &fields.input_name,
        fields.input,
        fields.scene_item_enabled,
    ) {
        Ok(created) => created,
        Err(CreateInputError::UnsupportedSceneName) => {
            return build_request_response_error(
                "CreateInput",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
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
    let (input_uuid, input_name) = match parse_request_data_or_error_response(
        "RemoveInput",
        request_id,
        request_data,
        parse_input_lookup_fields,
    ) {
        Ok(v) => v,
        Err(response) => return response,
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

fn resolve_record_directory_path(record_directory: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(record_directory);
    if path.is_absolute() {
        return Ok(path);
    }
    let current_dir =
        std::env::current_dir().map_err(|e| format!("Failed to resolve current directory: {e}"))?;
    Ok(current_dir.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_stop_record_response_includes_output_path() {
        let response = build_stop_record_response("req-stop-record", "/tmp/output.mp4");
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let output_path: String = json
            .value()
            .to_member("d")
            .expect("d access must succeed")
            .required()
            .expect("d must exist")
            .to_member("responseData")
            .expect("responseData access must succeed")
            .required()
            .expect("responseData must exist")
            .to_member("outputPath")
            .expect("outputPath access must succeed")
            .required()
            .expect("outputPath must exist")
            .try_into()
            .expect("outputPath must be string");
        assert_eq!(output_path, "/tmp/output.mp4");
    }
}
