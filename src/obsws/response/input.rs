use crate::obsws::protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS, REQUEST_STATUS_RESOURCE_NOT_FOUND,
};
use crate::obsws::state::{
    CreateInputError, ObswsSessionState, ParseInputSettingsError, SetInputNameError,
    SetInputSettingsError,
};

use super::{
    CreateInputCreated, CreateInputExecution, SetInputSettingsExecution, parse_create_input_fields,
    parse_get_input_default_settings_fields, parse_get_input_properties_list_property_items_fields,
    parse_input_lookup_fields, parse_request_data_or_error_response, parse_set_input_name_fields,
    parse_set_input_settings_fields,
};

pub fn build_get_input_list_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let inputs = state.list_inputs();
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
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    super::build_request_response_success("GetInputKindList", request_id, |f| {
        f.member("inputKinds", state.supported_input_kinds())
    })
}

pub fn build_get_input_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
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

    let Some(input) = state.find_input(input_uuid.as_deref(), input_name.as_deref()) else {
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
    state: &ObswsSessionState,
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

    let source_active = match state.is_source_active(input_uuid.as_deref(), input_name.as_deref()) {
        Ok(source_active) => source_active,
        Err(crate::obsws::state::GetSourceActiveError::SourceNotFound) => {
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
    state: &mut ObswsSessionState,
) -> nojson::RawJsonOwned {
    execute_set_input_settings(request_id, request_data, state).response_text
}

pub fn execute_set_input_settings(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
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

    if let Err(error) = state.set_input_settings(
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
    state: &mut ObswsSessionState,
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

    if let Err(error) = state.set_input_name(
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
    state: &ObswsSessionState,
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
    let default_input_settings = match state.get_input_default_settings(&fields.input_kind) {
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
    state: &mut ObswsSessionState,
) -> nojson::RawJsonOwned {
    execute_create_input(request_id, request_data, state).response_text
}

pub fn execute_create_input(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
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
    let (created_entry, scene_item_id) = match state.create_input(
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
    let scene_uuid = state.get_scene_uuid(&scene_name).unwrap_or_default();
    let scene_items = state.list_scene_items(&scene_name).unwrap_or_default();
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

    let default_settings = state
        .get_input_default_settings(created_entry.input.kind_name())
        .unwrap_or_else(|_| created_entry.input.settings.clone());

    let scene_item_ref = crate::obsws::state::ObswsSceneItemRef {
        scene_name: scene_name.clone(),
        scene_uuid,
        scene_item: crate::obsws::state::ObswsSceneItemEntry {
            scene_item_id,
            source_name: created_entry.input_name.clone(),
            source_uuid: created_entry.input_uuid.clone(),
            input_kind: created_entry.input.kind_name().to_owned(),
            source_type: "OBS_SOURCE_TYPE_INPUT".to_owned(),
            scene_item_enabled: fields.scene_item_enabled,
            scene_item_locked: false,
            scene_item_blend_mode: crate::obsws::state::ObswsSceneItemBlendMode::default()
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
    state: &mut ObswsSessionState,
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
    let Some(_removed) = state.remove_input(input_uuid.as_deref(), input_name.as_deref()) else {
        return super::build_request_response_error(
            "RemoveInput",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Input not found",
        );
    };

    super::build_request_response_success_no_data("RemoveInput", request_id)
}

// --- Mute / Volume ---

pub fn build_get_input_mute_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let (input_uuid, input_name) = match parse_request_data_or_error_response(
        "GetInputMute",
        request_id,
        request_data,
        parse_input_lookup_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };

    let Some(muted) = state.get_input_mute(input_uuid.as_deref(), input_name.as_deref()) else {
        return super::build_request_response_error(
            "GetInputMute",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Input not found",
        );
    };

    super::build_request_response_success("GetInputMute", request_id, |f| {
        f.member("inputMuted", muted)
    })
}

pub fn build_set_input_mute_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
) -> SetInputMuteExecution {
    let (input_uuid, input_name, input_muted) = match parse_request_data_or_error_response(
        "SetInputMute",
        request_id,
        request_data,
        parse_set_input_mute_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => {
            return SetInputMuteExecution {
                response_text: response,
                request_succeeded: false,
                input_uuid: None,
                input_name: None,
            };
        }
    };

    if state
        .set_input_mute(input_uuid.as_deref(), input_name.as_deref(), input_muted)
        .is_none()
    {
        return SetInputMuteExecution {
            response_text: super::build_request_response_error(
                "SetInputMute",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Input not found",
            ),
            request_succeeded: false,
            input_uuid,
            input_name,
        };
    }

    SetInputMuteExecution {
        response_text: super::build_request_response_success_no_data("SetInputMute", request_id),
        request_succeeded: true,
        input_uuid,
        input_name,
    }
}

pub struct SetInputMuteExecution {
    pub response_text: nojson::RawJsonOwned,
    pub request_succeeded: bool,
    pub input_uuid: Option<String>,
    pub input_name: Option<String>,
}

pub fn build_toggle_input_mute_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
) -> ToggleInputMuteExecution {
    let (input_uuid, input_name) = match parse_request_data_or_error_response(
        "ToggleInputMute",
        request_id,
        request_data,
        parse_input_lookup_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => {
            return ToggleInputMuteExecution {
                response_text: response,
                request_succeeded: false,
                input_uuid: None,
                input_name: None,
            };
        }
    };

    let Some(new_muted) = state.toggle_input_mute(input_uuid.as_deref(), input_name.as_deref())
    else {
        return ToggleInputMuteExecution {
            response_text: super::build_request_response_error(
                "ToggleInputMute",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Input not found",
            ),
            request_succeeded: false,
            input_uuid,
            input_name,
        };
    };

    ToggleInputMuteExecution {
        response_text: super::build_request_response_success("ToggleInputMute", request_id, |f| {
            f.member("inputMuted", new_muted)
        }),
        request_succeeded: true,
        input_uuid,
        input_name,
    }
}

pub struct ToggleInputMuteExecution {
    pub response_text: nojson::RawJsonOwned,
    pub request_succeeded: bool,
    pub input_uuid: Option<String>,
    pub input_name: Option<String>,
}

pub fn build_get_input_volume_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let (input_uuid, input_name) = match parse_request_data_or_error_response(
        "GetInputVolume",
        request_id,
        request_data,
        parse_input_lookup_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };

    let Some((volume_db, volume_mul)) =
        state.get_input_volume(input_uuid.as_deref(), input_name.as_deref())
    else {
        return super::build_request_response_error(
            "GetInputVolume",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Input not found",
        );
    };

    super::build_request_response_success("GetInputVolume", request_id, |f| {
        f.member("inputVolumeDb", volume_db)?;
        f.member("inputVolumeMul", volume_mul)
    })
}

pub fn build_set_input_volume_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
) -> SetInputVolumeExecution {
    let fields = match parse_request_data_or_error_response(
        "SetInputVolume",
        request_id,
        request_data,
        parse_set_input_volume_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => {
            return SetInputVolumeExecution {
                response_text: response,
                request_succeeded: false,
                input_uuid: None,
                input_name: None,
            };
        }
    };

    if state
        .set_input_volume(
            fields.input_uuid.as_deref(),
            fields.input_name.as_deref(),
            fields.volume_db,
            fields.volume_mul,
        )
        .is_none()
    {
        return SetInputVolumeExecution {
            response_text: super::build_request_response_error(
                "SetInputVolume",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Input not found",
            ),
            request_succeeded: false,
            input_uuid: fields.input_uuid,
            input_name: fields.input_name,
        };
    }

    SetInputVolumeExecution {
        response_text: super::build_request_response_success_no_data("SetInputVolume", request_id),
        request_succeeded: true,
        input_uuid: fields.input_uuid,
        input_name: fields.input_name,
    }
}

pub struct SetInputVolumeExecution {
    pub response_text: nojson::RawJsonOwned,
    pub request_succeeded: bool,
    pub input_uuid: Option<String>,
    pub input_name: Option<String>,
}

struct SetInputVolumeFields {
    input_uuid: Option<String>,
    input_name: Option<String>,
    volume_db: Option<f64>,
    volume_mul: Option<f64>,
}

fn parse_set_input_mute_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<(Option<String>, Option<String>, bool), nojson::JsonParseError> {
    let (input_uuid, input_name) = parse_input_lookup_fields(request_data)?;
    let input_muted: bool = request_data
        .to_member("inputMuted")?
        .required()?
        .try_into()?;
    Ok((input_uuid, input_name, input_muted))
}

fn parse_set_input_volume_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetInputVolumeFields, nojson::JsonParseError> {
    let (input_uuid, input_name) = parse_input_lookup_fields(request_data)?;

    let volume_db: Option<f64> = request_data
        .to_member("inputVolumeDb")?
        .optional()
        .map(|v| v.try_into())
        .transpose()?;
    let volume_mul: Option<f64> = request_data
        .to_member("inputVolumeMul")?
        .optional()
        .map(|v| v.try_into())
        .transpose()?;

    if volume_db.is_none() && volume_mul.is_none() {
        return Err(
            request_data.invalid("required member 'inputVolumeDb or inputVolumeMul' is missing")
        );
    }

    // 有限値チェック
    if let Some(db) = volume_db
        && !db.is_finite()
    {
        return Err(request_data.invalid("inputVolumeDb must be a finite number"));
    }
    if let Some(mul) = volume_mul
        && (!mul.is_finite() || mul < 0.0)
    {
        return Err(request_data.invalid("inputVolumeMul must be a finite non-negative number"));
    }

    Ok(SetInputVolumeFields {
        input_uuid,
        input_name,
        volume_db,
        volume_mul,
    })
}

pub fn build_get_input_properties_list_property_items_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetInputPropertiesListPropertyItems",
        request_id,
        request_data,
        parse_get_input_properties_list_property_items_fields,
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };

    let Some(input) = state.find_input(fields.input_uuid.as_deref(), fields.input_name.as_deref())
    else {
        return super::build_request_response_error(
            "GetInputPropertiesListPropertyItems",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Input not found",
        );
    };

    if input.input.settings.kind_name() != "video_capture_device" {
        return super::build_request_response_error(
            "GetInputPropertiesListPropertyItems",
            request_id,
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "property enumeration is only supported for video_capture_device",
        );
    }

    // video_capture_device の device_id を取得する
    let input_device_id = match &input.input.settings {
        crate::obsws::state::ObswsInputSettings::VideoCaptureDevice(settings) => {
            settings.device_id.as_deref()
        }
        _ => None,
    };

    let property_items =
        match enumerate_video_device_property_items(&fields.property_name, input_device_id) {
            Ok(items) => items,
            Err(error_message) => {
                return super::build_request_response_error(
                    "GetInputPropertiesListPropertyItems",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &error_message,
                );
            }
        };

    super::build_request_response_success("GetInputPropertiesListPropertyItems", request_id, |f| {
        f.member(
            "propertyItems",
            nojson::array(|f| {
                for item in &property_items {
                    f.element(nojson::object(|f| {
                        f.member("itemName", item.item_name.as_str())?;
                        f.member("itemValue", item.item_value.as_str())?;
                        f.member("itemEnabled", item.item_enabled)
                    }))?;
                }
                Ok(())
            }),
        )
    })
}

/// GetInputPropertiesListPropertyItems のレスポンスに含まれるプロパティアイテム
struct ObswsPropertyItem {
    item_name: String,
    item_value: String,
    item_enabled: bool,
}

/// video_capture_device の指定されたプロパティのアイテムを列挙する
fn enumerate_video_device_property_items(
    property_name: &str,
    device_id: Option<&str>,
) -> Result<Vec<ObswsPropertyItem>, String> {
    use std::collections::BTreeSet;

    let device_list = shiguredo_video_device::VideoDeviceList::enumerate()
        .map_err(|e| format!("failed to enumerate video devices: {e}"))?;

    match property_name {
        "device_id" => {
            let mut items = Vec::new();
            for device in device_list.devices() {
                let name = device.name().unwrap_or_else(|_| "Unknown".to_owned());
                let unique_id = device.unique_id().unwrap_or_else(|_| "unknown".to_owned());
                items.push(ObswsPropertyItem {
                    item_name: name,
                    item_value: unique_id,
                    item_enabled: true,
                });
            }
            Ok(items)
        }
        "formats" => {
            let mut items = Vec::new();
            for device in device_list.devices() {
                // device_id が指定されている場合はそのデバイスだけフィルタする
                if let Some(target_id) = device_id {
                    let unique_id = device.unique_id().unwrap_or_else(|_| "unknown".to_owned());
                    if unique_id != target_id {
                        continue;
                    }
                }
                for format in device.formats() {
                    let fps = format.max_fps.round() as i32;
                    let pixel_format_name = format.pixel_format.name();
                    let item_name = format!(
                        "{}x{} / {} fps / {}",
                        format.width, format.height, fps, pixel_format_name
                    );
                    let item_value = format!(
                        "{}x{}_{}_{}",
                        format.width, format.height, fps, pixel_format_name
                    );
                    items.push(ObswsPropertyItem {
                        item_name,
                        item_value,
                        item_enabled: true,
                    });
                }
            }
            Ok(items)
        }
        "pixel_format" => {
            let mut values = BTreeSet::new();
            for device in device_list.devices() {
                if let Some(target_id) = device_id {
                    let unique_id = device.unique_id().unwrap_or_else(|_| "unknown".to_owned());
                    if unique_id != target_id {
                        continue;
                    }
                }
                for format in device.formats() {
                    match format.pixel_format {
                        shiguredo_video_device::PixelFormat::Nv12 => {
                            values.insert("NV12".to_owned());
                        }
                        shiguredo_video_device::PixelFormat::Yuy2 => {
                            values.insert("YUY2".to_owned());
                        }
                        shiguredo_video_device::PixelFormat::I420 => {
                            values.insert("I420".to_owned());
                        }
                        shiguredo_video_device::PixelFormat::Unknown(_) => {}
                    }
                }
            }

            Ok(values
                .into_iter()
                .map(|value| ObswsPropertyItem {
                    item_name: value.clone(),
                    item_value: value,
                    item_enabled: true,
                })
                .collect())
        }
        "fps" => {
            let mut values = BTreeSet::new();
            for device in device_list.devices() {
                if let Some(target_id) = device_id {
                    let unique_id = device.unique_id().unwrap_or_else(|_| "unknown".to_owned());
                    if unique_id != target_id {
                        continue;
                    }
                }
                for format in device.formats() {
                    values.insert((format.max_fps.round() as i32).to_string());
                }
            }

            Ok(values
                .into_iter()
                .map(|value| ObswsPropertyItem {
                    item_name: value.clone(),
                    item_value: value,
                    item_enabled: true,
                })
                .collect())
        }
        _ => Err(format!(
            "unsupported property name for video_capture_device: {property_name}"
        )),
    }
}
