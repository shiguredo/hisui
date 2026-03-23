use crate::obsws_input_registry::{
    CreateInputError, ObswsInputRegistry, ParseInputSettingsError, SetInputNameError,
    SetInputSettingsError,
};
use crate::obsws_protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS, REQUEST_STATUS_RESOURCE_NOT_FOUND,
};

use super::{
    CreateInputCreated, CreateInputExecution, SetInputSettingsExecution, parse_create_input_fields,
    parse_get_input_default_settings_fields, parse_input_lookup_fields,
    parse_request_data_or_error_response, parse_set_input_name_fields,
    parse_set_input_settings_fields,
};

pub fn build_get_input_list_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let inputs = input_registry.list_inputs();
    // inputKind フィールドが指定されている場合、その kind でフィルタする
    let input_kind_filter: Option<String> = request_data.and_then(|data| {
        let value: Option<String> = data.value().to_member("inputKind").ok()?.try_into().ok()?;
        value
    });
    let filtered: Vec<_> = match input_kind_filter {
        Some(ref kind) => inputs
            .into_iter()
            .filter(|i| i.input.kind_name() == kind)
            .collect(),
        None => inputs,
    };
    super::build_request_response_success("GetInputList", request_id, |f| {
        f.member("inputs", &filtered)
    })
}

/// hisui にはバージョン付き input kind が存在しないため、
/// OBS の unversioned パラメータには対応しない。
pub fn build_get_input_kind_list_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    super::build_request_response_success("GetInputKindList", request_id, |f| {
        f.member("inputKinds", input_registry.supported_input_kinds())
    })
}

pub fn build_get_input_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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

    super::build_request_response_success("GetInputSettings", request_id, |f| {
        f.member("inputSettings", &input.input.settings)?;
        f.member("inputKind", input.input.settings.kind_name())
    })
}

pub fn build_get_source_active_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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

    super::build_request_response_success("GetSourceActive", request_id, |f| {
        f.member("videoActive", source_active)?;
        // videoShowing は hisui では videoActive と同値
        f.member("videoShowing", source_active)
    })
}

pub fn build_set_input_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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

    let response_text =
        super::build_request_response_success_no_data("SetInputSettings", request_id);
    SetInputSettingsExecution {
        response_text,
        request_succeeded: true,
    }
}

pub fn build_set_input_name_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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

    super::build_request_response_success_no_data("SetInputName", request_id)
}

pub fn build_get_input_default_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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

    super::build_request_response_success("GetInputDefaultSettings", request_id, |f| {
        f.member("defaultInputSettings", &default_input_settings)
    })
}

pub fn build_create_input_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    execute_create_input(request_id, request_data, input_registry).response_text
}

pub fn execute_create_input(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> CreateInputExecution {
    let fields = match parse_request_data_or_error_response(
        "CreateInput",
        request_id,
        request_data,
        parse_create_input_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => {
            return CreateInputExecution {
                response_text: response,
                created: None,
            };
        }
    };

    let scene_name = fields.scene_name.clone();
    let (created_entry, scene_item_id) = match input_registry.create_input(
        &fields.scene_name,
        &fields.input_name,
        fields.input,
        fields.scene_item_enabled,
    ) {
        Ok(result) => result,
        Err(CreateInputError::UnsupportedSceneName) => {
            return CreateInputExecution {
                response_text: super::build_request_response_error(
                    "CreateInput",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene not found",
                ),
                created: None,
            };
        }
        Err(CreateInputError::InputNameAlreadyExists) => {
            return CreateInputExecution {
                response_text: super::build_request_response_error(
                    "CreateInput",
                    request_id,
                    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
                    "Input already exists",
                ),
                created: None,
            };
        }
        Err(CreateInputError::InputIdOverflow) => {
            return CreateInputExecution {
                response_text: super::build_request_response_error(
                    "CreateInput",
                    request_id,
                    REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Input ID overflow",
                ),
                created: None,
            };
        }
    };

    let response_text = super::build_request_response_success("CreateInput", request_id, |f| {
        f.member("inputUuid", &created_entry.input_uuid)?;
        f.member("sceneItemId", scene_item_id)
    });

    // SceneItemCreated イベント用の情報を構築する
    let scene_uuid = input_registry
        .get_scene_uuid(&scene_name)
        .unwrap_or_default();
    let scene_items = input_registry
        .list_scene_items(&scene_name)
        .unwrap_or_default();
    // 追加直後のアイテムを scene_item_id で検索する
    let created_scene_item = scene_items
        .iter()
        .find(|item| item.scene_item_id == scene_item_id);
    let scene_item_index = created_scene_item
        .map(|item| item.scene_item_index)
        .unwrap_or(0);
    let scene_item_transform = created_scene_item
        .map(|item| item.scene_item_transform.clone())
        .unwrap_or_default();

    let default_settings = input_registry
        .get_input_default_settings(created_entry.input.kind_name())
        .unwrap_or_else(|_| created_entry.input.settings.clone());

    let scene_item_ref = crate::obsws_input_registry::ObswsSceneItemRef {
        scene_name: scene_name.clone(),
        scene_uuid,
        scene_item: crate::obsws_input_registry::ObswsSceneItemEntry {
            scene_item_id,
            source_name: created_entry.input_name.clone(),
            source_uuid: created_entry.input_uuid.clone(),
            input_kind: created_entry.input.kind_name().to_owned(),
            source_type: "OBS_SOURCE_TYPE_INPUT".to_owned(),
            scene_item_enabled: fields.scene_item_enabled,
            scene_item_locked: false,
            scene_item_blend_mode: crate::obsws_input_registry::ObswsSceneItemBlendMode::default()
                .as_str()
                .to_owned(),
            scene_item_index,
            scene_item_transform,
            is_group: None,
        },
    };

    CreateInputExecution {
        response_text,
        created: Some(CreateInputCreated {
            input_entry: created_entry,
            default_settings,
            scene_item_ref,
        }),
    }
}

pub fn build_remove_input_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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

    super::build_request_response_success_no_data("RemoveInput", request_id)
}
