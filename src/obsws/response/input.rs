use crate::obsws_input_registry::{
    CreateInputError, ObswsInputRegistry, ParseInputSettingsError, SetInputNameError,
    SetInputSettingsError,
};
use crate::obsws_protocol::{
    OBSWS_OP_REQUEST_RESPONSE, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS, REQUEST_STATUS_RESOURCE_NOT_FOUND,
    REQUEST_STATUS_SUCCESS,
};

use super::{
    SetInputSettingsExecution, parse_create_input_fields, parse_get_input_default_settings_fields,
    parse_input_lookup_fields, parse_request_data_or_error_response, parse_set_input_name_fields,
    parse_set_input_settings_fields,
};

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
        return super::build_request_response_error(
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

pub fn build_get_source_active_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let (input_uuid, input_name) = match parse_request_data_or_error_response(
        "GetSourceActive",
        request_id,
        request_data,
        parse_input_lookup_fields,
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };

    let source_active =
        match input_registry.is_source_active(input_uuid.as_deref(), input_name.as_deref()) {
            Ok(source_active) => source_active,
            Err(crate::obsws_input_registry::GetSourceActiveError::SourceNotFound) => {
                return super::build_request_response_error(
                    "GetSourceActive",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Source not found",
                );
            }
        };

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSourceActive")?;
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
                    nojson::object(|f| f.member("videoActive", source_active)),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_set_input_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    execute_set_input_settings(request_id, request_data, input_registry).response_text
}

pub fn execute_set_input_settings(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> SetInputSettingsExecution {
    let fields = match parse_request_data_or_error_response(
        "SetInputSettings",
        request_id,
        request_data,
        parse_set_input_settings_fields,
    ) {
        Ok(fields) => fields,
        Err(response_text) => {
            return SetInputSettingsExecution {
                response_text,
                request_succeeded: false,
            };
        }
    };

    if let Err(error) = input_registry.set_input_settings(
        fields.input_uuid.as_deref(),
        fields.input_name.as_deref(),
        fields.input_settings.value(),
        fields.overlay,
    ) {
        let response_text = match error {
            SetInputSettingsError::InputNotFound => super::build_request_response_error(
                "SetInputSettings",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Input not found",
            ),
            SetInputSettingsError::InvalidInputSettings(message) => {
                super::build_request_response_error(
                    "SetInputSettings",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &message,
                )
            }
        };
        return SetInputSettingsExecution {
            response_text,
            request_succeeded: false,
        };
    }

    let response_text = nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetInputSettings")?;
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
    .to_string();
    SetInputSettingsExecution {
        response_text,
        request_succeeded: true,
    }
}

pub fn build_set_input_name_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetInputName",
        request_id,
        request_data,
        parse_set_input_name_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };

    if let Err(error) = input_registry.set_input_name(
        fields.input_uuid.as_deref(),
        fields.input_name.as_deref(),
        &fields.new_input_name,
    ) {
        return match error {
            SetInputNameError::InputNotFound => super::build_request_response_error(
                "SetInputName",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Input not found",
            ),
            SetInputNameError::InputNameAlreadyExists => super::build_request_response_error(
                "SetInputName",
                request_id,
                REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
                "Input name already exists",
            ),
        };
    }

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetInputName")?;
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

pub fn build_get_input_default_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetInputDefaultSettings",
        request_id,
        request_data,
        parse_get_input_default_settings_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let default_input_settings = match input_registry.get_input_default_settings(&fields.input_kind)
    {
        Ok(settings) => settings,
        Err(ParseInputSettingsError::UnsupportedInputKind) => {
            return super::build_request_response_error(
                "GetInputDefaultSettings",
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Unsupported input kind",
            );
        }
        Err(ParseInputSettingsError::InvalidInputSettings(_)) => {
            unreachable!("BUG: default settings generation must not return invalid settings")
        }
    };

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetInputDefaultSettings")?;
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
                        f.member("inputKind", &fields.input_kind)?;
                        f.member("defaultInputSettings", &default_input_settings)
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
            return super::build_request_response_error(
                "CreateInput",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
            );
        }
        Err(CreateInputError::InputNameAlreadyExists) => {
            return super::build_request_response_error(
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
        return super::build_request_response_error(
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
