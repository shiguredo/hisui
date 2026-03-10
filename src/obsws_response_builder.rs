use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::{
    CreateInputError, CreateSceneError, CreateSceneItemError, DuplicateSceneItemError,
    GetSceneItemBlendModeError, GetSceneItemEnabledError, GetSceneItemIdError,
    GetSceneItemIndexError, GetSceneItemListError, GetSceneItemLockedError,
    GetSceneItemSourceError, GetSceneItemTransformError, ObswsInput, ObswsInputRegistry,
    ObswsInputSettings, ObswsSceneItemBlendMode, ObswsSceneItemIndexEntry, ObswsSceneItemRef,
    ObswsSceneItemTransform, ObswsSceneItemTransformPatch, ObswsStreamServiceSettings,
    ParseInputSettingsError, RemoveSceneError, RemoveSceneItemError, SetCurrentPreviewSceneError,
    SetCurrentProgramSceneError, SetCurrentSceneTransitionDurationError,
    SetCurrentSceneTransitionError, SetCurrentSceneTransitionSettingsError, SetInputNameError,
    SetInputSettingsError, SetSceneItemBlendModeError, SetSceneItemEnabledError,
    SetSceneItemIndexError, SetSceneItemIndexResult, SetSceneItemLockedError,
    SetSceneItemLockedResult, SetSceneItemTransformError, SetSceneItemTransformResult,
    SetTBarPositionError,
};
use crate::obsws_message::ObswsSessionStats;
use crate::obsws_protocol::{
    OBSWS_EVENT_SUB_INPUTS, OBSWS_EVENT_SUB_OUTPUTS, OBSWS_EVENT_SUB_SCENES, OBSWS_OP_EVENT,
    OBSWS_OP_HELLO, OBSWS_OP_IDENTIFIED, OBSWS_OP_REQUEST_BATCH_RESPONSE,
    OBSWS_OP_REQUEST_RESPONSE, OBSWS_RPC_VERSION, OBSWS_SUPPORTED_IMAGE_FORMATS, OBSWS_VERSION,
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_FIELD,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS, REQUEST_STATUS_RESOURCE_NOT_FOUND,
    REQUEST_STATUS_SUCCESS,
};
use std::path::PathBuf;

struct CreateInputFields {
    scene_name: String,
    input_name: String,
    input: ObswsInput,
    scene_item_enabled: bool,
}

struct SetInputSettingsFields {
    input_uuid: Option<String>,
    input_name: Option<String>,
    input_settings: nojson::RawJsonOwned,
    overlay: bool,
}

struct SetInputNameFields {
    input_uuid: Option<String>,
    input_name: Option<String>,
    new_input_name: String,
}

struct GetInputDefaultSettingsFields {
    input_kind: String,
}

struct CreateSceneFields {
    scene_name: String,
}

struct SetCurrentProgramSceneFields {
    scene_name: String,
}

struct SetCurrentPreviewSceneFields {
    scene_name: String,
}

struct SetCurrentSceneTransitionFields {
    transition_name: String,
}

struct SetCurrentSceneTransitionDurationFields {
    transition_duration: i64,
}

struct SetCurrentSceneTransitionSettingsFields {
    transition_settings: nojson::RawJsonOwned,
}

struct SetTBarPositionFields {
    position: f64,
}

struct RemoveSceneFields {
    scene_name: String,
}

struct GetSceneItemIdFields {
    scene_name: String,
    source_name: String,
    search_offset: i64,
}

struct GetSceneItemListFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
}

struct CreateSceneItemFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    source_name: Option<String>,
    source_uuid: Option<String>,
    scene_item_enabled: bool,
}

struct RemoveSceneItemFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
}

struct DuplicateSceneItemFields {
    from_scene_name: Option<String>,
    from_scene_uuid: Option<String>,
    to_scene_name: Option<String>,
    to_scene_uuid: Option<String>,
    scene_item_id: i64,
}

struct GetSceneItemSourceFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
}

struct GetSceneItemIndexFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
}

struct SetSceneItemIndexFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
    scene_item_index: i64,
}

struct GetSceneItemEnabledFields {
    scene_name: String,
    scene_item_id: i64,
}

struct SetSceneItemEnabledFields {
    scene_name: String,
    scene_item_id: i64,
    scene_item_enabled: bool,
}

struct GetSceneItemLockedFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
}

struct SetSceneItemLockedFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
    scene_item_locked: bool,
}

struct GetSceneItemBlendModeFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
}

struct SetSceneItemBlendModeFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
    scene_item_blend_mode: ObswsSceneItemBlendMode,
}

struct GetSceneItemTransformFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
}

struct SetSceneItemTransformFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
    scene_item_transform: ObswsSceneItemTransformPatch,
}

struct SetStreamServiceSettingsFields {
    stream_service_type: String,
    server: String,
    key: Option<String>,
}

struct SetRecordDirectoryFields {
    record_directory: String,
}

struct ObswsSceneTransitionEntry {
    transition_name: String,
    transition_kind: String,
    transition_fixed: bool,
    transition_configurable: bool,
}

impl nojson::DisplayJson for ObswsSceneTransitionEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("transitionName", &self.transition_name)?;
            f.member("transitionKind", &self.transition_kind)?;
            f.member("transitionFixed", self.transition_fixed)?;
            f.member("transitionConfigurable", self.transition_configurable)
        })
        .fmt(f)
    }
}

fn is_fixed_transition(transition_name: &str) -> bool {
    matches!(transition_name, "Cut")
}

#[derive(Debug, Clone)]
pub struct RequestBatchResult {
    pub request_type: String,
    pub request_status_result: bool,
    pub request_status_code: i64,
    pub request_status_comment: Option<String>,
    pub response_data: Option<nojson::RawJsonOwned>,
}

#[derive(Debug, Clone)]
pub struct SetSceneItemIndexExecution {
    pub response_text: String,
    pub scene_name: Option<String>,
    pub set_result: Option<SetSceneItemIndexResult>,
}

#[derive(Debug, Clone)]
pub struct SetSceneItemLockedExecution {
    pub response_text: String,
    pub scene_name: Option<String>,
    pub scene_item_id: Option<i64>,
    pub scene_item_locked: Option<bool>,
    pub set_result: Option<SetSceneItemLockedResult>,
}

#[derive(Debug, Clone)]
pub struct SetSceneItemTransformExecution {
    pub response_text: String,
    pub scene_name: Option<String>,
    pub scene_item_id: Option<i64>,
    pub set_result: Option<SetSceneItemTransformResult>,
}

#[derive(Debug, Clone)]
pub struct CreateSceneItemExecution {
    pub response_text: String,
    pub created: Option<ObswsSceneItemRef>,
}

#[derive(Debug, Clone)]
pub struct DuplicateSceneItemExecution {
    pub response_text: String,
    pub duplicated: Option<ObswsSceneItemRef>,
}

#[derive(Debug, Clone)]
pub struct SetInputSettingsExecution {
    pub response_text: String,
    pub request_succeeded: bool,
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
    let value = object.to_member(member_name)?.optional();
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

fn parse_set_input_settings_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetInputSettingsFields, nojson::JsonParseError> {
    let (input_uuid, input_name) = parse_input_lookup_fields(request_data)?;
    let input_settings = request_data.to_member("inputSettings")?.required()?;
    let overlay: Option<bool> = request_data.to_member("overlay")?.try_into()?;
    Ok(SetInputSettingsFields {
        input_uuid,
        input_name,
        input_settings: nojson::RawJsonOwned::try_from(input_settings)?,
        overlay: overlay.unwrap_or(true),
    })
}

fn parse_set_input_name_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetInputNameFields, nojson::JsonParseError> {
    let (input_uuid, input_name) = parse_input_lookup_fields(request_data)?;
    let new_input_name = required_non_empty_string_member(request_data, "newInputName")?;
    Ok(SetInputNameFields {
        input_uuid,
        input_name,
        new_input_name,
    })
}

fn parse_get_input_default_settings_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetInputDefaultSettingsFields, nojson::JsonParseError> {
    let input_kind = required_non_empty_string_member(request_data, "inputKind")?;
    Ok(GetInputDefaultSettingsFields { input_kind })
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

fn parse_set_current_preview_scene_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetCurrentPreviewSceneFields, nojson::JsonParseError> {
    let scene_name = required_non_empty_string_member(request_data, "sceneName")?;
    Ok(SetCurrentPreviewSceneFields { scene_name })
}

fn parse_set_current_scene_transition_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetCurrentSceneTransitionFields, nojson::JsonParseError> {
    let transition_name = required_non_empty_string_member(request_data, "transitionName")?;
    Ok(SetCurrentSceneTransitionFields { transition_name })
}

fn parse_set_current_scene_transition_duration_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetCurrentSceneTransitionDurationFields, nojson::JsonParseError> {
    let transition_duration: i64 = request_data
        .to_member("transitionDuration")?
        .required()?
        .try_into()?;
    Ok(SetCurrentSceneTransitionDurationFields {
        transition_duration,
    })
}

fn parse_set_current_scene_transition_settings_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetCurrentSceneTransitionSettingsFields, nojson::JsonParseError> {
    let transition_settings = request_data.to_member("transitionSettings")?.required()?;
    if transition_settings.kind() != nojson::JsonValueKind::Object {
        return Err(transition_settings.invalid("object is required"));
    }
    Ok(SetCurrentSceneTransitionSettingsFields {
        transition_settings: nojson::RawJsonOwned::try_from(transition_settings)?,
    })
}

fn parse_set_tbar_position_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetTBarPositionFields, nojson::JsonParseError> {
    let position: f64 = request_data.to_member("position")?.required()?.try_into()?;
    Ok(SetTBarPositionFields { position })
}

fn parse_remove_scene_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<RemoveSceneFields, nojson::JsonParseError> {
    let scene_name = required_non_empty_string_member(request_data, "sceneName")?;
    Ok(RemoveSceneFields { scene_name })
}

fn parse_get_scene_item_id_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneItemIdFields, nojson::JsonParseError> {
    let scene_name = required_non_empty_string_member(request_data, "sceneName")?;
    let source_name = required_non_empty_string_member(request_data, "sourceName")?;
    let search_offset: Option<i64> = request_data.to_member("searchOffset")?.try_into()?;
    Ok(GetSceneItemIdFields {
        scene_name,
        source_name,
        search_offset: search_offset.unwrap_or(0),
    })
}

fn parse_scene_lookup_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
    scene_name_field: &str,
    scene_uuid_field: &str,
) -> Result<(Option<String>, Option<String>), nojson::JsonParseError> {
    let scene_name = optional_non_empty_string_member(request_data, scene_name_field)?;
    let scene_uuid = optional_non_empty_string_member(request_data, scene_uuid_field)?;
    if scene_name.is_none() && scene_uuid.is_none() {
        return Err(request_data.invalid(format!(
            "required member '{} or {}' is missing",
            scene_name_field, scene_uuid_field
        )));
    }
    Ok((scene_name, scene_uuid))
}

fn parse_source_lookup_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<(Option<String>, Option<String>), nojson::JsonParseError> {
    let source_name = optional_non_empty_string_member(request_data, "sourceName")?;
    let source_uuid = optional_non_empty_string_member(request_data, "sourceUuid")?;
    if source_name.is_none() && source_uuid.is_none() {
        return Err(request_data.invalid("required member 'sourceName or sourceUuid' is missing"));
    }
    Ok((source_name, source_uuid))
}

fn parse_get_scene_item_list_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneItemListFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    Ok(GetSceneItemListFields {
        scene_name,
        scene_uuid,
    })
}

fn parse_create_scene_item_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<CreateSceneItemFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let (source_name, source_uuid) = parse_source_lookup_fields(request_data)?;
    let scene_item_enabled: Option<bool> =
        request_data.to_member("sceneItemEnabled")?.try_into()?;
    Ok(CreateSceneItemFields {
        scene_name,
        scene_uuid,
        source_name,
        source_uuid,
        scene_item_enabled: scene_item_enabled.unwrap_or(true),
    })
}

fn parse_remove_scene_item_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<RemoveSceneItemFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(RemoveSceneItemFields {
        scene_name,
        scene_uuid,
        scene_item_id,
    })
}

fn parse_duplicate_scene_item_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<DuplicateSceneItemFields, nojson::JsonParseError> {
    let (from_scene_name, from_scene_uuid) =
        parse_scene_lookup_fields(request_data, "fromSceneName", "fromSceneUuid")?;
    let (to_scene_name, to_scene_uuid) =
        parse_scene_lookup_fields(request_data, "toSceneName", "toSceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(DuplicateSceneItemFields {
        from_scene_name,
        from_scene_uuid,
        to_scene_name,
        to_scene_uuid,
        scene_item_id,
    })
}

fn parse_get_scene_item_source_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneItemSourceFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(GetSceneItemSourceFields {
        scene_name,
        scene_uuid,
        scene_item_id,
    })
}

fn parse_get_scene_item_index_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneItemIndexFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(GetSceneItemIndexFields {
        scene_name,
        scene_uuid,
        scene_item_id,
    })
}

fn parse_set_scene_item_index_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetSceneItemIndexFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    let scene_item_index: i64 = request_data
        .to_member("sceneItemIndex")?
        .required()?
        .try_into()?;
    Ok(SetSceneItemIndexFields {
        scene_name,
        scene_uuid,
        scene_item_id,
        scene_item_index,
    })
}

fn parse_set_scene_item_enabled_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetSceneItemEnabledFields, nojson::JsonParseError> {
    let scene_name = required_non_empty_string_member(request_data, "sceneName")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    let scene_item_enabled: bool = request_data
        .to_member("sceneItemEnabled")?
        .required()?
        .try_into()?;
    Ok(SetSceneItemEnabledFields {
        scene_name,
        scene_item_id,
        scene_item_enabled,
    })
}

fn parse_get_scene_item_enabled_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneItemEnabledFields, nojson::JsonParseError> {
    let scene_name = required_non_empty_string_member(request_data, "sceneName")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(GetSceneItemEnabledFields {
        scene_name,
        scene_item_id,
    })
}

fn parse_get_scene_item_locked_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneItemLockedFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(GetSceneItemLockedFields {
        scene_name,
        scene_uuid,
        scene_item_id,
    })
}

fn parse_set_scene_item_locked_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetSceneItemLockedFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    let scene_item_locked: bool = request_data
        .to_member("sceneItemLocked")?
        .required()?
        .try_into()?;
    Ok(SetSceneItemLockedFields {
        scene_name,
        scene_uuid,
        scene_item_id,
        scene_item_locked,
    })
}

fn parse_get_scene_item_blend_mode_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneItemBlendModeFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(GetSceneItemBlendModeFields {
        scene_name,
        scene_uuid,
        scene_item_id,
    })
}

fn parse_set_scene_item_blend_mode_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetSceneItemBlendModeFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    let raw_blend_mode = request_data.to_member("sceneItemBlendMode")?.required()?;
    let blend_mode_str: String = raw_blend_mode.try_into()?;
    let Some(scene_item_blend_mode) = ObswsSceneItemBlendMode::parse(&blend_mode_str) else {
        return Err(raw_blend_mode.invalid("Invalid sceneItemBlendMode field"));
    };
    Ok(SetSceneItemBlendModeFields {
        scene_name,
        scene_uuid,
        scene_item_id,
        scene_item_blend_mode,
    })
}

fn parse_get_scene_item_transform_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneItemTransformFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(GetSceneItemTransformFields {
        scene_name,
        scene_uuid,
        scene_item_id,
    })
}

fn parse_set_scene_item_transform_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetSceneItemTransformFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    let raw_scene_item_transform = request_data.to_member("sceneItemTransform")?.required()?;
    if raw_scene_item_transform.kind() != nojson::JsonValueKind::Object {
        return Err(raw_scene_item_transform.invalid("object is required"));
    }
    let scene_item_transform = parse_scene_item_transform_patch(raw_scene_item_transform)?;
    Ok(SetSceneItemTransformFields {
        scene_name,
        scene_uuid,
        scene_item_id,
        scene_item_transform,
    })
}

fn parse_scene_item_transform_patch(
    raw_scene_item_transform: nojson::RawJsonValue<'_, '_>,
) -> Result<ObswsSceneItemTransformPatch, nojson::JsonParseError> {
    let bounds_type = optional_non_empty_string_member(raw_scene_item_transform, "boundsType")?;
    if let Some(bounds_type) = &bounds_type
        && !matches!(
            bounds_type.as_str(),
            "OBS_BOUNDS_NONE"
                | "OBS_BOUNDS_STRETCH"
                | "OBS_BOUNDS_SCALE_INNER"
                | "OBS_BOUNDS_SCALE_OUTER"
                | "OBS_BOUNDS_SCALE_TO_WIDTH"
                | "OBS_BOUNDS_SCALE_TO_HEIGHT"
                | "OBS_BOUNDS_MAX_ONLY"
        )
    {
        return Err(raw_scene_item_transform
            .to_member("boundsType")?
            .required()?
            .invalid("Invalid sceneItemTransform.boundsType field"));
    }
    let alignment: Option<i64> = raw_scene_item_transform
        .to_member("alignment")?
        .try_into()?;
    if let Some(alignment) = alignment
        && !is_valid_scene_item_alignment(alignment)
    {
        return Err(raw_scene_item_transform
            .to_member("alignment")?
            .required()?
            .invalid("Invalid sceneItemTransform.alignment field"));
    }
    let bounds_alignment: Option<i64> = raw_scene_item_transform
        .to_member("boundsAlignment")?
        .try_into()?;
    if let Some(bounds_alignment) = bounds_alignment
        && !is_valid_scene_item_alignment(bounds_alignment)
    {
        return Err(raw_scene_item_transform
            .to_member("boundsAlignment")?
            .required()?
            .invalid("Invalid sceneItemTransform.boundsAlignment field"));
    }

    Ok(ObswsSceneItemTransformPatch {
        position_x: raw_scene_item_transform
            .to_member("positionX")?
            .try_into()?,
        position_y: raw_scene_item_transform
            .to_member("positionY")?
            .try_into()?,
        rotation: raw_scene_item_transform.to_member("rotation")?.try_into()?,
        scale_x: raw_scene_item_transform.to_member("scaleX")?.try_into()?,
        scale_y: raw_scene_item_transform.to_member("scaleY")?.try_into()?,
        alignment,
        bounds_type,
        bounds_alignment,
        bounds_width: raw_scene_item_transform
            .to_member("boundsWidth")?
            .try_into()?,
        bounds_height: raw_scene_item_transform
            .to_member("boundsHeight")?
            .try_into()?,
        crop_top: raw_scene_item_transform.to_member("cropTop")?.try_into()?,
        crop_bottom: raw_scene_item_transform
            .to_member("cropBottom")?
            .try_into()?,
        crop_left: raw_scene_item_transform.to_member("cropLeft")?.try_into()?,
        crop_right: raw_scene_item_transform
            .to_member("cropRight")?
            .try_into()?,
        crop_to_bounds: raw_scene_item_transform
            .to_member("cropToBounds")?
            .try_into()?,
    })
}

fn is_valid_scene_item_alignment(alignment: i64) -> bool {
    // OBS の alignment は bitmask（left=1, right=2, top=4, bottom=8）として扱う。
    // 有効値: center(0), left/right, top/bottom, およびそれらの組み合わせ。
    matches!(alignment, 0 | 1 | 2 | 4 | 5 | 6 | 8 | 9 | 10)
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

pub(crate) fn parse_input_lookup_fields_for_session(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<(Option<String>, Option<String>), nojson::JsonParseError> {
    parse_input_lookup_fields(request_data)
}

pub(crate) fn parse_scene_lookup_fields_for_session(
    request_data: nojson::RawJsonValue<'_, '_>,
    scene_name_field: &str,
    scene_uuid_field: &str,
) -> Result<(Option<String>, Option<String>), nojson::JsonParseError> {
    parse_scene_lookup_fields(request_data, scene_name_field, scene_uuid_field)
}

pub(crate) fn parse_required_i64_field_for_session(
    request_data: nojson::RawJsonValue<'_, '_>,
    field_name: &str,
) -> Result<i64, nojson::JsonParseError> {
    request_data.to_member(field_name)?.required()?.try_into()
}

pub(crate) fn parse_set_scene_item_enabled_fields_for_session(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<(String, i64, bool), nojson::JsonParseError> {
    let fields = parse_set_scene_item_enabled_fields(request_data)?;
    Ok((
        fields.scene_name,
        fields.scene_item_id,
        fields.scene_item_enabled,
    ))
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

fn resolve_scene_name_or_error(
    request_type: &str,
    request_id: &str,
    input_registry: &ObswsInputRegistry,
    scene_name: Option<&str>,
    scene_uuid: Option<&str>,
) -> Result<String, String> {
    input_registry
        .resolve_scene_name(scene_name, scene_uuid)
        .ok_or_else(|| {
            build_request_response_error(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
            )
        })
}

pub(crate) fn request_status_code_for_parse_error(error: &nojson::JsonParseError) -> i64 {
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

pub fn build_stream_state_changed_event(output_active: bool) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "StreamStateChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_OUTPUTS)?;
                f.member(
                    "eventData",
                    nojson::object(|f| f.member("outputActive", output_active)),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_record_state_changed_event(
    output_active: bool,
    output_paused: bool,
    output_path: Option<&str>,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "RecordStateChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_OUTPUTS)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("outputActive", output_active)?;
                        f.member("outputPaused", output_paused)?;
                        if let Some(output_path) = output_path {
                            f.member("outputPath", output_path)?;
                        }
                        Ok(())
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_current_program_scene_changed_event(scene_name: &str, scene_uuid: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "CurrentProgramSceneChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENES)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_current_preview_scene_changed_event(scene_name: &str, scene_uuid: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "CurrentPreviewSceneChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENES)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_scene_created_event(scene_name: &str, scene_uuid: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneCreated")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENES)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)?;
                        f.member("isGroup", false)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_scene_removed_event(scene_name: &str, scene_uuid: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneRemoved")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENES)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)?;
                        f.member("isGroup", false)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_input_created_event(input_name: &str, input_uuid: &str, input_kind: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "InputCreated")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_INPUTS)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("inputName", input_name)?;
                        f.member("inputUuid", input_uuid)?;
                        f.member("inputKind", input_kind)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_input_removed_event(input_name: &str, input_uuid: &str, input_kind: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "InputRemoved")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_INPUTS)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("inputName", input_name)?;
                        f.member("inputUuid", input_uuid)?;
                        f.member("inputKind", input_kind)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_input_settings_changed_event(
    input_name: &str,
    input_uuid: &str,
    input_kind: &str,
    input_settings: &ObswsInputSettings,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "InputSettingsChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_INPUTS)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("inputName", input_name)?;
                        f.member("inputUuid", input_uuid)?;
                        f.member("inputKind", input_kind)?;
                        f.member("inputSettings", input_settings)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_input_name_changed_event(
    input_name: &str,
    old_input_name: &str,
    input_uuid: &str,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "InputNameChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_INPUTS)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("inputName", input_name)?;
                        f.member("oldInputName", old_input_name)?;
                        f.member("inputUuid", input_uuid)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_scene_item_enable_state_changed_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_item_id: i64,
    scene_item_enabled: bool,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemEnableStateChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENES)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)?;
                        f.member("sceneItemId", scene_item_id)?;
                        f.member("sceneItemEnabled", scene_item_enabled)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_scene_item_lock_state_changed_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_item_id: i64,
    scene_item_locked: bool,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemLockStateChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENES)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)?;
                        f.member("sceneItemId", scene_item_id)?;
                        f.member("sceneItemLocked", scene_item_locked)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_scene_item_transform_changed_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_item_id: i64,
    scene_item_transform: &ObswsSceneItemTransform,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemTransformChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENES)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)?;
                        f.member("sceneItemId", scene_item_id)?;
                        f.member("sceneItemTransform", scene_item_transform)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_scene_item_created_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_item_id: i64,
    source_name: &str,
    source_uuid: &str,
    scene_item_index: i64,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemCreated")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENES)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)?;
                        f.member("sceneItemId", scene_item_id)?;
                        f.member("sourceName", source_name)?;
                        f.member("sourceUuid", source_uuid)?;
                        f.member("sceneItemIndex", scene_item_index)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_scene_item_removed_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_item_id: i64,
    source_name: &str,
    source_uuid: &str,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemRemoved")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENES)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)?;
                        f.member("sceneItemId", scene_item_id)?;
                        f.member("sourceName", source_name)?;
                        f.member("sourceUuid", source_uuid)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_scene_item_list_reindexed_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_items: &[ObswsSceneItemIndexEntry],
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemListReindexed")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENES)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("sceneName", scene_name)?;
                        f.member("sceneUuid", scene_uuid)?;
                        f.member("sceneItems", scene_items)
                    }),
                )
            }),
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
                                "RemoveScene",
                                "GetCurrentProgramScene",
                                "SetCurrentProgramScene",
                                "GetCurrentPreviewScene",
                                "SetCurrentPreviewScene",
                                "GetTransitionKindList",
                                "GetSceneTransitionList",
                                "GetCurrentSceneTransition",
                                "SetCurrentSceneTransition",
                                "SetCurrentSceneTransitionDuration",
                                "SetCurrentSceneTransitionSettings",
                                "GetCurrentSceneTransitionCursor",
                                "SetTBarPosition",
                                "GetSceneItemId",
                                "GetSceneItemList",
                                "CreateSceneItem",
                                "RemoveSceneItem",
                                "DuplicateSceneItem",
                                "GetSceneItemSource",
                                "GetSceneItemEnabled",
                                "SetSceneItemEnabled",
                                "GetSceneItemLocked",
                                "SetSceneItemLocked",
                                "GetSceneItemIndex",
                                "SetSceneItemIndex",
                                "GetSceneItemBlendMode",
                                "SetSceneItemBlendMode",
                                "GetSceneItemTransform",
                                "SetSceneItemTransform",
                                "GetInputList",
                                "GetInputKindList",
                                "GetInputSettings",
                                "SetInputSettings",
                                "SetInputName",
                                "GetInputDefaultSettings",
                                "CreateInput",
                                "RemoveInput",
                                "GetStreamServiceSettings",
                                "SetStreamServiceSettings",
                                "GetStreamStatus",
                                "ToggleStream",
                                "StartStream",
                                "StopStream",
                                "GetRecordDirectory",
                                "SetRecordDirectory",
                                "GetRecordStatus",
                                "ToggleRecord",
                                "StartRecord",
                                "StopRecord",
                                "ToggleRecordPause",
                                "PauseRecord",
                                "ResumeRecord",
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
    let current_preview_scene = input_registry.current_preview_scene();
    let current_program_scene_name = current_program_scene
        .as_ref()
        .map(|scene| scene.scene_name.as_str())
        .unwrap_or_default();
    let current_program_scene_uuid = current_program_scene
        .as_ref()
        .map(|scene| scene.scene_uuid.as_str())
        .unwrap_or_default();
    let current_preview_scene_name = current_preview_scene
        .as_ref()
        .map(|scene| scene.scene_name.as_str())
        .unwrap_or_default();
    let current_preview_scene_uuid = current_preview_scene
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
                        f.member("currentPreviewSceneName", current_preview_scene_name)?;
                        f.member("currentPreviewSceneUuid", current_preview_scene_uuid)?;
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

pub fn build_get_current_preview_scene_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let current_preview_scene = input_registry.current_preview_scene();
    let scene_name = current_preview_scene
        .as_ref()
        .map(|scene| scene.scene_name.as_str())
        .unwrap_or_default();
    let scene_uuid = current_preview_scene
        .as_ref()
        .map(|scene| scene.scene_uuid.as_str())
        .unwrap_or_default();
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetCurrentPreviewScene")?;
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
                        f.member("currentPreviewSceneName", scene_name)?;
                        f.member("currentPreviewSceneUuid", scene_uuid)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_set_current_preview_scene_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetCurrentPreviewScene",
        request_id,
        request_data,
        parse_set_current_preview_scene_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    if let Err(SetCurrentPreviewSceneError::SceneNotFound) =
        input_registry.set_current_preview_scene(&fields.scene_name)
    {
        return build_request_response_error(
            "SetCurrentPreviewScene",
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
                f.member("requestType", "SetCurrentPreviewScene")?;
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

pub fn build_get_transition_kind_list_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetTransitionKindList")?;
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
                            "transitionKinds",
                            input_registry.supported_transition_kinds(),
                        )
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_get_scene_transition_list_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let transitions: Vec<ObswsSceneTransitionEntry> = input_registry
        .supported_transition_kinds()
        .iter()
        .map(|name| ObswsSceneTransitionEntry {
            transition_name: (*name).to_owned(),
            transition_kind: (*name).to_owned(),
            transition_fixed: is_fixed_transition(name),
            transition_configurable: false,
        })
        .collect();
    let current_transition_name = input_registry.current_scene_transition_name();

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSceneTransitionList")?;
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
                        f.member("currentSceneTransitionName", current_transition_name)?;
                        f.member("currentSceneTransitionKind", current_transition_name)?;
                        f.member("transitions", &transitions)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_get_current_scene_transition_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let current_transition_name = input_registry.current_scene_transition_name();
    let current_transition_duration_ms = input_registry.current_scene_transition_duration_ms();
    let transition_settings = input_registry.current_scene_transition_settings();
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetCurrentSceneTransition")?;
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
                        f.member("transitionName", current_transition_name)?;
                        f.member("transitionKind", current_transition_name)?;
                        f.member(
                            "transitionFixed",
                            is_fixed_transition(current_transition_name),
                        )?;
                        f.member("transitionConfigurable", false)?;
                        f.member("transitionSettings", transition_settings)?;
                        f.member("transitionDuration", current_transition_duration_ms)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_set_current_scene_transition_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetCurrentSceneTransition",
        request_id,
        request_data,
        parse_set_current_scene_transition_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    if let Err(SetCurrentSceneTransitionError::TransitionNotFound) =
        input_registry.set_current_scene_transition(&fields.transition_name)
    {
        return build_request_response_error(
            "SetCurrentSceneTransition",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Transition not found",
        );
    }
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetCurrentSceneTransition")?;
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

pub fn build_set_current_scene_transition_duration_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetCurrentSceneTransitionDuration",
        request_id,
        request_data,
        parse_set_current_scene_transition_duration_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    if let Err(SetCurrentSceneTransitionDurationError::InvalidTransitionDuration) =
        input_registry.set_current_scene_transition_duration_ms(fields.transition_duration)
    {
        return build_request_response_error(
            "SetCurrentSceneTransitionDuration",
            request_id,
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "Invalid transitionDuration field",
        );
    }
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetCurrentSceneTransitionDuration")?;
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

pub fn build_set_current_scene_transition_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetCurrentSceneTransitionSettings",
        request_id,
        request_data,
        parse_set_current_scene_transition_settings_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    if let Err(SetCurrentSceneTransitionSettingsError::InvalidTransitionSettings) =
        input_registry.set_current_scene_transition_settings(fields.transition_settings)
    {
        return build_request_response_error(
            "SetCurrentSceneTransitionSettings",
            request_id,
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "Invalid transitionSettings field",
        );
    }
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetCurrentSceneTransitionSettings")?;
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

pub fn build_get_current_scene_transition_cursor_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let transition_cursor = input_registry.current_tbar_position();
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetCurrentSceneTransitionCursor")?;
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
                    nojson::object(|f| f.member("transitionCursor", transition_cursor)),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_set_tbar_position_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetTBarPosition",
        request_id,
        request_data,
        parse_set_tbar_position_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    if let Err(SetTBarPositionError::InvalidTBarPosition) =
        input_registry.set_tbar_position(fields.position)
    {
        return build_request_response_error(
            "SetTBarPosition",
            request_id,
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "Invalid position field",
        );
    }
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetTBarPosition")?;
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

pub fn build_remove_scene_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "RemoveScene",
        request_id,
        request_data,
        parse_remove_scene_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    if let Err(error) = input_registry.remove_scene(&fields.scene_name) {
        return match error {
            RemoveSceneError::SceneNotFound => build_request_response_error(
                "RemoveScene",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
            ),
            RemoveSceneError::LastSceneNotRemovable => build_request_response_error(
                "RemoveScene",
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "At least one scene must remain",
            ),
        };
    }

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "RemoveScene")?;
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

pub fn build_get_scene_item_id_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemId",
        request_id,
        request_data,
        parse_get_scene_item_id_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };

    let scene_item_id = match input_registry.get_scene_item_id(
        &fields.scene_name,
        &fields.source_name,
        fields.search_offset,
    ) {
        Ok(scene_item_id) => scene_item_id,
        Err(GetSceneItemIdError::SceneNotFound) => {
            return build_request_response_error(
                "GetSceneItemId",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
            );
        }
        Err(GetSceneItemIdError::SourceNotFound) => {
            return build_request_response_error(
                "GetSceneItemId",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Source not found in scene",
            );
        }
        Err(GetSceneItemIdError::SearchOffsetUnsupported) => {
            return build_request_response_error(
                "GetSceneItemId",
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Unsupported searchOffset field: only 0 is supported",
            );
        }
    };

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSceneItemId")?;
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
                    nojson::object(|f| f.member("sceneItemId", scene_item_id)),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_get_scene_item_list_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemList",
        request_id,
        request_data,
        parse_get_scene_item_list_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let scene_name = match resolve_scene_name_or_error(
        "GetSceneItemList",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => return response,
    };
    let scene_items = input_registry
        .list_scene_items(&scene_name)
        .unwrap_or_else(|error| match error {
            GetSceneItemListError::SceneNotFound => {
                unreachable!("resolved scene name must exist in input registry")
            }
        });

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSceneItemList")?;
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
                    nojson::object(|f| f.member("sceneItems", &scene_items)),
                )
            }),
        )
    })
    .to_string()
}

pub fn execute_create_scene_item(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> CreateSceneItemExecution {
    let fields = match parse_request_data_or_error_response(
        "CreateSceneItem",
        request_id,
        request_data,
        parse_create_scene_item_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => {
            return CreateSceneItemExecution {
                response_text: response,
                created: None,
            };
        }
    };
    let scene_name = match resolve_scene_name_or_error(
        "CreateSceneItem",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => {
            return CreateSceneItemExecution {
                response_text: response,
                created: None,
            };
        }
    };
    let created = match input_registry.create_scene_item(
        &scene_name,
        fields.source_uuid.as_deref(),
        fields.source_name.as_deref(),
        fields.scene_item_enabled,
    ) {
        Ok(created) => created,
        Err(CreateSceneItemError::SourceNotFound) => {
            return CreateSceneItemExecution {
                response_text: build_request_response_error(
                    "CreateSceneItem",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Source not found",
                ),
                created: None,
            };
        }
        Err(CreateSceneItemError::SceneNotFound) => {
            unreachable!("resolved scene name must exist in input registry")
        }
    };

    let response_text = nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "CreateSceneItem")?;
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
                    nojson::object(|f| f.member("sceneItemId", created.scene_item.scene_item_id)),
                )
            }),
        )
    })
    .to_string();
    CreateSceneItemExecution {
        response_text,
        created: Some(created),
    }
}

pub fn build_remove_scene_item_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "RemoveSceneItem",
        request_id,
        request_data,
        parse_remove_scene_item_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let scene_name = match resolve_scene_name_or_error(
        "RemoveSceneItem",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => return response,
    };
    if let Err(error) = input_registry.remove_scene_item(&scene_name, fields.scene_item_id) {
        return match error {
            RemoveSceneItemError::SceneNotFound => {
                unreachable!("resolved scene name must exist in input registry")
            }
            RemoveSceneItemError::SceneItemNotFound => build_request_response_error(
                "RemoveSceneItem",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene item not found",
            ),
        };
    }

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "RemoveSceneItem")?;
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

pub fn execute_duplicate_scene_item(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> DuplicateSceneItemExecution {
    let fields = match parse_request_data_or_error_response(
        "DuplicateSceneItem",
        request_id,
        request_data,
        parse_duplicate_scene_item_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => {
            return DuplicateSceneItemExecution {
                response_text: response,
                duplicated: None,
            };
        }
    };
    let from_scene_name = match resolve_scene_name_or_error(
        "DuplicateSceneItem",
        request_id,
        input_registry,
        fields.from_scene_name.as_deref(),
        fields.from_scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => {
            return DuplicateSceneItemExecution {
                response_text: response,
                duplicated: None,
            };
        }
    };
    let to_scene_name = match resolve_scene_name_or_error(
        "DuplicateSceneItem",
        request_id,
        input_registry,
        fields.to_scene_name.as_deref(),
        fields.to_scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => {
            return DuplicateSceneItemExecution {
                response_text: response,
                duplicated: None,
            };
        }
    };
    let duplicated = match input_registry.duplicate_scene_item(
        &from_scene_name,
        &to_scene_name,
        fields.scene_item_id,
    ) {
        Ok(duplicated) => duplicated,
        Err(DuplicateSceneItemError::SourceScene) => {
            unreachable!("resolved source scene name must exist in input registry")
        }
        Err(DuplicateSceneItemError::DestinationScene) => {
            unreachable!("resolved destination scene name must exist in input registry")
        }
        Err(DuplicateSceneItemError::SourceSceneItem) => {
            return DuplicateSceneItemExecution {
                response_text: build_request_response_error(
                    "DuplicateSceneItem",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                ),
                duplicated: None,
            };
        }
    };

    let response_text = nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "DuplicateSceneItem")?;
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
                        f.member("sceneItemId", duplicated.scene_item.scene_item_id)
                    }),
                )
            }),
        )
    })
    .to_string();
    DuplicateSceneItemExecution {
        response_text,
        duplicated: Some(duplicated),
    }
}

pub fn build_get_scene_item_source_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemSource",
        request_id,
        request_data,
        parse_get_scene_item_source_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let scene_name = match resolve_scene_name_or_error(
        "GetSceneItemSource",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => return response,
    };
    let (source_name, source_uuid) =
        match input_registry.get_scene_item_source(&scene_name, fields.scene_item_id) {
            Ok(source) => source,
            Err(GetSceneItemSourceError::SceneItemNotFound) => {
                return build_request_response_error(
                    "GetSceneItemSource",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                );
            }
            Err(GetSceneItemSourceError::SceneNotFound) => {
                unreachable!("resolved scene name must exist in input registry")
            }
        };

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSceneItemSource")?;
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
                        f.member("sourceName", &source_name)?;
                        f.member("sourceUuid", &source_uuid)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_get_scene_item_index_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemIndex",
        request_id,
        request_data,
        parse_get_scene_item_index_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let scene_name = match resolve_scene_name_or_error(
        "GetSceneItemIndex",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => return response,
    };
    let scene_item_index =
        match input_registry.get_scene_item_index(&scene_name, fields.scene_item_id) {
            Ok(scene_item_index) => scene_item_index,
            Err(GetSceneItemIndexError::SceneItemNotFound) => {
                return build_request_response_error(
                    "GetSceneItemIndex",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                );
            }
            Err(GetSceneItemIndexError::SceneNotFound) => {
                unreachable!("resolved scene name must exist in input registry")
            }
        };

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSceneItemIndex")?;
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
                    nojson::object(|f| f.member("sceneItemIndex", scene_item_index)),
                )
            }),
        )
    })
    .to_string()
}

pub fn execute_set_scene_item_index(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> SetSceneItemIndexExecution {
    let fields = match parse_request_data_or_error_response(
        "SetSceneItemIndex",
        request_id,
        request_data,
        parse_set_scene_item_index_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => {
            return SetSceneItemIndexExecution {
                response_text: response,
                scene_name: None,
                set_result: None,
            };
        }
    };
    let scene_name = match resolve_scene_name_or_error(
        "SetSceneItemIndex",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => {
            return SetSceneItemIndexExecution {
                response_text: response,
                scene_name: None,
                set_result: None,
            };
        }
    };
    let set_result = match input_registry.set_scene_item_index(
        &scene_name,
        fields.scene_item_id,
        fields.scene_item_index,
    ) {
        Ok(set_result) => set_result,
        Err(error) => {
            let response_text = match error {
                SetSceneItemIndexError::SceneItemNotFound => build_request_response_error(
                    "SetSceneItemIndex",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                ),
                SetSceneItemIndexError::InvalidSceneItemIndex => build_request_response_error(
                    "SetSceneItemIndex",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Invalid sceneItemIndex field",
                ),
                SetSceneItemIndexError::SceneNotFound => {
                    unreachable!("resolved scene name must exist in input registry")
                }
            };
            return SetSceneItemIndexExecution {
                response_text,
                scene_name: None,
                set_result: None,
            };
        }
    };

    let response_text = nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetSceneItemIndex")?;
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
    SetSceneItemIndexExecution {
        response_text,
        scene_name: Some(scene_name),
        set_result: Some(set_result),
    }
}

pub fn build_set_scene_item_enabled_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetSceneItemEnabled",
        request_id,
        request_data,
        parse_set_scene_item_enabled_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };

    if let Err(error) = input_registry.set_scene_item_enabled(
        &fields.scene_name,
        fields.scene_item_id,
        fields.scene_item_enabled,
    ) {
        return match error {
            SetSceneItemEnabledError::SceneNotFound => build_request_response_error(
                "SetSceneItemEnabled",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
            ),
            SetSceneItemEnabledError::SceneItemNotFound => build_request_response_error(
                "SetSceneItemEnabled",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene item not found",
            ),
        };
    }

    build_set_scene_item_enabled_success_response(request_id)
}

pub fn build_get_scene_item_enabled_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemEnabled",
        request_id,
        request_data,
        parse_get_scene_item_enabled_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };

    let scene_item_enabled =
        match input_registry.get_scene_item_enabled(&fields.scene_name, fields.scene_item_id) {
            Ok(scene_item_enabled) => scene_item_enabled,
            Err(GetSceneItemEnabledError::SceneNotFound) => {
                return build_request_response_error(
                    "GetSceneItemEnabled",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene not found",
                );
            }
            Err(GetSceneItemEnabledError::SceneItemNotFound) => {
                return build_request_response_error(
                    "GetSceneItemEnabled",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                );
            }
        };

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSceneItemEnabled")?;
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
                    nojson::object(|f| f.member("sceneItemEnabled", scene_item_enabled)),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_set_scene_item_enabled_success_response(request_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetSceneItemEnabled")?;
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

pub fn build_get_scene_item_locked_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemLocked",
        request_id,
        request_data,
        parse_get_scene_item_locked_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let scene_name = match resolve_scene_name_or_error(
        "GetSceneItemLocked",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => return response,
    };
    let scene_item_locked =
        match input_registry.get_scene_item_locked(&scene_name, fields.scene_item_id) {
            Ok(scene_item_locked) => scene_item_locked,
            Err(GetSceneItemLockedError::SceneNotFound) => {
                unreachable!("resolved scene name must exist in input registry")
            }
            Err(GetSceneItemLockedError::SceneItemNotFound) => {
                return build_request_response_error(
                    "GetSceneItemLocked",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                );
            }
        };

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSceneItemLocked")?;
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
                    nojson::object(|f| f.member("sceneItemLocked", scene_item_locked)),
                )
            }),
        )
    })
    .to_string()
}

pub fn execute_set_scene_item_locked(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> SetSceneItemLockedExecution {
    let fields = match parse_request_data_or_error_response(
        "SetSceneItemLocked",
        request_id,
        request_data,
        parse_set_scene_item_locked_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => {
            return SetSceneItemLockedExecution {
                response_text: response,
                scene_name: None,
                scene_item_id: None,
                scene_item_locked: None,
                set_result: None,
            };
        }
    };
    let scene_name = match resolve_scene_name_or_error(
        "SetSceneItemLocked",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => {
            return SetSceneItemLockedExecution {
                response_text: response,
                scene_name: None,
                scene_item_id: None,
                scene_item_locked: None,
                set_result: None,
            };
        }
    };
    let set_result = match input_registry.set_scene_item_locked(
        &scene_name,
        fields.scene_item_id,
        fields.scene_item_locked,
    ) {
        Ok(set_result) => set_result,
        Err(SetSceneItemLockedError::SceneNotFound) => {
            unreachable!("resolved scene name must exist in input registry")
        }
        Err(SetSceneItemLockedError::SceneItemNotFound) => {
            return SetSceneItemLockedExecution {
                response_text: build_request_response_error(
                    "SetSceneItemLocked",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                ),
                scene_name: None,
                scene_item_id: None,
                scene_item_locked: None,
                set_result: None,
            };
        }
    };

    let response_text = nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetSceneItemLocked")?;
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

    SetSceneItemLockedExecution {
        response_text,
        scene_name: Some(scene_name),
        scene_item_id: Some(fields.scene_item_id),
        scene_item_locked: Some(fields.scene_item_locked),
        set_result: Some(set_result),
    }
}

pub fn build_get_scene_item_blend_mode_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemBlendMode",
        request_id,
        request_data,
        parse_get_scene_item_blend_mode_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let scene_name = match resolve_scene_name_or_error(
        "GetSceneItemBlendMode",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => return response,
    };
    let scene_item_blend_mode =
        match input_registry.get_scene_item_blend_mode(&scene_name, fields.scene_item_id) {
            Ok(scene_item_blend_mode) => scene_item_blend_mode,
            Err(GetSceneItemBlendModeError::SceneNotFound) => {
                unreachable!("resolved scene name must exist in input registry")
            }
            Err(GetSceneItemBlendModeError::SceneItemNotFound) => {
                return build_request_response_error(
                    "GetSceneItemBlendMode",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                );
            }
        };

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSceneItemBlendMode")?;
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
                        f.member("sceneItemBlendMode", scene_item_blend_mode.as_str())
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_set_scene_item_blend_mode_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetSceneItemBlendMode",
        request_id,
        request_data,
        parse_set_scene_item_blend_mode_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let scene_name = match resolve_scene_name_or_error(
        "SetSceneItemBlendMode",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => return response,
    };
    if let Err(error) = input_registry.set_scene_item_blend_mode(
        &scene_name,
        fields.scene_item_id,
        fields.scene_item_blend_mode,
    ) {
        return match error {
            SetSceneItemBlendModeError::SceneNotFound => {
                unreachable!("resolved scene name must exist in input registry")
            }
            SetSceneItemBlendModeError::SceneItemNotFound => build_request_response_error(
                "SetSceneItemBlendMode",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene item not found",
            ),
        };
    }
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetSceneItemBlendMode")?;
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

pub fn build_get_scene_item_transform_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemTransform",
        request_id,
        request_data,
        parse_get_scene_item_transform_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let scene_name = match resolve_scene_name_or_error(
        "GetSceneItemTransform",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => return response,
    };
    let scene_item_transform =
        match input_registry.get_scene_item_transform(&scene_name, fields.scene_item_id) {
            Ok(scene_item_transform) => scene_item_transform,
            Err(GetSceneItemTransformError::SceneNotFound) => {
                unreachable!("resolved scene name must exist in input registry")
            }
            Err(GetSceneItemTransformError::SceneItemNotFound) => {
                return build_request_response_error(
                    "GetSceneItemTransform",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                );
            }
        };

    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetSceneItemTransform")?;
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
                    nojson::object(|f| f.member("sceneItemTransform", &scene_item_transform)),
                )
            }),
        )
    })
    .to_string()
}

pub fn execute_set_scene_item_transform(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> SetSceneItemTransformExecution {
    let fields = match parse_request_data_or_error_response(
        "SetSceneItemTransform",
        request_id,
        request_data,
        parse_set_scene_item_transform_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => {
            return SetSceneItemTransformExecution {
                response_text: response,
                scene_name: None,
                scene_item_id: None,
                set_result: None,
            };
        }
    };
    let scene_name = match resolve_scene_name_or_error(
        "SetSceneItemTransform",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(scene_name) => scene_name,
        Err(response) => {
            return SetSceneItemTransformExecution {
                response_text: response,
                scene_name: None,
                scene_item_id: None,
                set_result: None,
            };
        }
    };
    let set_result = match input_registry.set_scene_item_transform(
        &scene_name,
        fields.scene_item_id,
        fields.scene_item_transform,
    ) {
        Ok(set_result) => set_result,
        Err(SetSceneItemTransformError::SceneNotFound) => {
            unreachable!("resolved scene name must exist in input registry")
        }
        Err(SetSceneItemTransformError::SceneItemNotFound) => {
            return SetSceneItemTransformExecution {
                response_text: build_request_response_error(
                    "SetSceneItemTransform",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                ),
                scene_name: None,
                scene_item_id: None,
                set_result: None,
            };
        }
    };

    let response_text = nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetSceneItemTransform")?;
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

    SetSceneItemTransformExecution {
        response_text,
        scene_name: Some(scene_name),
        scene_item_id: Some(fields.scene_item_id),
        set_result: Some(set_result),
    }
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
    let paused = input_registry.is_record_paused();
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
                        f.member("outputPaused", paused)?;
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

fn build_output_active_response(
    request_type: &str,
    request_id: &str,
    output_active: bool,
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
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member(
                    "responseData",
                    nojson::object(|f| f.member("outputActive", output_active)),
                )
            }),
        )
    })
    .to_string()
}

fn build_record_output_state_response(
    request_type: &str,
    request_id: &str,
    output_active: bool,
    output_paused: bool,
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
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member(
                    "responseData",
                    nojson::object(|f| {
                        f.member("outputActive", output_active)?;
                        f.member("outputPaused", output_paused)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_start_stream_response(request_id: &str, output_active: bool) -> String {
    build_output_active_response("StartStream", request_id, output_active)
}

pub fn build_toggle_stream_response(request_id: &str, output_active: bool) -> String {
    build_output_active_response("ToggleStream", request_id, output_active)
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

pub fn build_toggle_record_response(request_id: &str, output_active: bool) -> String {
    build_record_output_state_response("ToggleRecord", request_id, output_active, false)
}

pub fn build_start_record_response(request_id: &str, output_active: bool) -> String {
    build_record_output_state_response("StartRecord", request_id, output_active, false)
}

pub fn build_toggle_record_pause_response(request_id: &str, output_paused: bool) -> String {
    build_record_output_state_response("ToggleRecordPause", request_id, true, output_paused)
}

pub fn build_pause_record_response(request_id: &str) -> String {
    build_record_output_state_response("PauseRecord", request_id, true, true)
}

pub fn build_resume_record_response(request_id: &str) -> String {
    build_record_output_state_response("ResumeRecord", request_id, true, false)
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
            SetInputSettingsError::InputNotFound => build_request_response_error(
                "SetInputSettings",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Input not found",
            ),
            SetInputSettingsError::InvalidInputSettings(message) => build_request_response_error(
                "SetInputSettings",
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                &message,
            ),
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
            SetInputNameError::InputNotFound => build_request_response_error(
                "SetInputName",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Input not found",
            ),
            SetInputNameError::InputNameAlreadyExists => build_request_response_error(
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
            return build_request_response_error(
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

pub fn build_request_batch_response(request_id: &str, results: &[RequestBatchResult]) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_BATCH_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestId", request_id)?;
                f.member(
                    "results",
                    nojson::array(|f| {
                        for result in results {
                            f.element(nojson::object(|f| {
                                f.member("requestType", &result.request_type)?;
                                f.member(
                                    "requestStatus",
                                    nojson::object(|f| {
                                        f.member("result", result.request_status_result)?;
                                        f.member("code", result.request_status_code)?;
                                        if let Some(comment) =
                                            result.request_status_comment.as_deref()
                                        {
                                            f.member("comment", comment)?;
                                        }
                                        Ok(())
                                    }),
                                )?;
                                if let Some(response_data) = result.response_data.as_ref() {
                                    f.member("responseData", response_data)?;
                                }
                                Ok(())
                            }))?;
                        }
                        Ok(())
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn parse_request_response_for_batch_result(
    response_text: &str,
) -> crate::Result<RequestBatchResult> {
    let json = nojson::RawJson::parse(response_text)?;
    let d = json.value().to_member("d")?.required()?;
    let request_type: String = d.to_member("requestType")?.required()?.try_into()?;
    let request_status = d.to_member("requestStatus")?.required()?;
    let request_status_result: bool = request_status.to_member("result")?.required()?.try_into()?;
    let request_status_code: i64 = request_status.to_member("code")?.required()?.try_into()?;
    let request_status_comment: Option<String> = request_status.to_member("comment")?.try_into()?;
    let response_data: Option<nojson::RawJsonOwned> = d
        .to_member("responseData")?
        .map(nojson::RawJsonOwned::try_from)?;

    Ok(RequestBatchResult {
        request_type,
        request_status_result,
        request_status_code,
        request_status_comment,
        response_data,
    })
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
    std::path::absolute(record_directory)
        .map_err(|e| format!("Failed to resolve absolute record directory path: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsws_input_registry::ObswsInputRegistry;

    #[test]
    fn build_stream_state_changed_event_contains_expected_fields() {
        let event = build_stream_state_changed_event(true);
        let json = nojson::RawJson::parse(&event).expect("event must be valid json");
        let op: i64 = json
            .value()
            .to_member("op")
            .and_then(|v| v.required()?.try_into())
            .expect("op must be i64");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let event_intent: u32 = json
            .value()
            .to_path_member(&["d", "eventIntent"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventIntent must be u32");
        let output_active: bool = json
            .value()
            .to_path_member(&["d", "eventData", "outputActive"])
            .and_then(|v| v.required()?.try_into())
            .expect("outputActive must be bool");
        assert_eq!(op, OBSWS_OP_EVENT);
        assert_eq!(event_type, "StreamStateChanged");
        assert_eq!(event_intent, OBSWS_EVENT_SUB_OUTPUTS);
        assert!(output_active);
    }

    #[test]
    fn build_stop_record_response_includes_output_path() {
        let response = build_stop_record_response("req-stop-record", "/tmp/output.mp4");
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let output_path: String = json
            .value()
            .to_path_member(&["d", "responseData", "outputPath"])
            .and_then(|v| v.required()?.try_into())
            .expect("outputPath must be string");
        assert_eq!(output_path, "/tmp/output.mp4");
    }

    #[test]
    fn build_record_state_changed_event_includes_output_path_when_present() {
        let event = build_record_state_changed_event(false, false, Some("/tmp/record.mp4"));
        let json = nojson::RawJson::parse(&event).expect("event must be valid json");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let output_paused: bool = json
            .value()
            .to_path_member(&["d", "eventData", "outputPaused"])
            .and_then(|v| v.required()?.try_into())
            .expect("outputPaused must be bool");
        let output_path: String = json
            .value()
            .to_path_member(&["d", "eventData", "outputPath"])
            .and_then(|v| v.required()?.try_into())
            .expect("outputPath must be string");
        assert_eq!(event_type, "RecordStateChanged");
        assert!(!output_paused);
        assert_eq!(output_path, "/tmp/record.mp4");
    }

    #[test]
    fn build_pause_record_response_sets_output_paused_true() {
        let response = build_pause_record_response("req-pause-record");
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let output_paused: bool = json
            .value()
            .to_path_member(&["d", "responseData", "outputPaused"])
            .and_then(|v| v.required()?.try_into())
            .expect("outputPaused must be bool");
        assert!(output_paused);
    }

    #[test]
    fn build_scene_events_contain_expected_fields() {
        let created_event = build_scene_created_event("Scene A", "scene-uuid-a");
        let removed_event = build_scene_removed_event("Scene B", "scene-uuid-b");

        for (event, expected_type, expected_name) in [
            (created_event, "SceneCreated", "Scene A"),
            (removed_event, "SceneRemoved", "Scene B"),
        ] {
            let json = nojson::RawJson::parse(&event).expect("event must be valid json");
            let event_type: String = json
                .value()
                .to_path_member(&["d", "eventType"])
                .and_then(|v| v.required()?.try_into())
                .expect("eventType must be string");
            let scene_name: String = json
                .value()
                .to_path_member(&["d", "eventData", "sceneName"])
                .and_then(|v| v.required()?.try_into())
                .expect("sceneName must be string");
            assert_eq!(event_type, expected_type);
            assert_eq!(scene_name, expected_name);
        }
    }

    #[test]
    fn build_current_preview_scene_changed_event_contains_expected_fields() {
        let event = build_current_preview_scene_changed_event("Scene P", "scene-uuid-p");
        let json = nojson::RawJson::parse(&event).expect("event must be valid json");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let scene_name: String = json
            .value()
            .to_path_member(&["d", "eventData", "sceneName"])
            .and_then(|v| v.required()?.try_into())
            .expect("sceneName must be string");
        assert_eq!(event_type, "CurrentPreviewSceneChanged");
        assert_eq!(scene_name, "Scene P");
    }

    #[test]
    fn build_get_and_set_current_preview_scene_response_succeeds() {
        let mut registry = ObswsInputRegistry::new_for_test();
        registry
            .create_scene("Scene B")
            .expect("scene creation must succeed");
        let set_request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
            .expect("requestData must be valid json");
        let set_response = build_set_current_preview_scene_response(
            "req-set-preview-scene",
            Some(&set_request_data),
            &mut registry,
        );
        let set_json = nojson::RawJson::parse(&set_response).expect("response must be valid json");
        let set_result: bool = set_json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        assert!(set_result);

        let get_response =
            build_get_current_preview_scene_response("req-get-preview-scene", &registry);
        let get_json = nojson::RawJson::parse(&get_response).expect("response must be valid json");
        let scene_name: String = get_json
            .value()
            .to_path_member(&["d", "responseData", "sceneName"])
            .and_then(|v| v.required()?.try_into())
            .expect("sceneName must be string");
        assert_eq!(scene_name, "Scene B");
    }

    #[test]
    fn build_set_current_scene_transition_settings_and_tbar_position_responses_succeed() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let set_transition_settings_request_data =
            nojson::RawJsonOwned::parse(r#"{"transitionSettings":{"curve":"ease","power":2}}"#)
                .expect("requestData must be valid json");
        let set_transition_settings_response = build_set_current_scene_transition_settings_response(
            "req-set-transition-settings",
            Some(&set_transition_settings_request_data),
            &mut registry,
        );
        let set_transition_settings_json =
            nojson::RawJson::parse(&set_transition_settings_response)
                .expect("response must be valid json");
        let set_transition_settings_result: bool = set_transition_settings_json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        assert!(set_transition_settings_result);

        let get_current_transition_response =
            build_get_current_scene_transition_response("req-get-current-transition", &registry);
        let get_current_transition_json = nojson::RawJson::parse(&get_current_transition_response)
            .expect("response must be valid json");
        let transition_power: i64 = get_current_transition_json
            .value()
            .to_path_member(&["d", "responseData", "transitionSettings", "power"])
            .and_then(|v| v.required()?.try_into())
            .expect("power must be i64");
        assert_eq!(transition_power, 2);

        let set_tbar_position_request_data = nojson::RawJsonOwned::parse(r#"{"position":0.75}"#)
            .expect("requestData must be valid json");
        let set_tbar_position_response = build_set_tbar_position_response(
            "req-set-tbar-position",
            Some(&set_tbar_position_request_data),
            &mut registry,
        );
        let set_tbar_position_json = nojson::RawJson::parse(&set_tbar_position_response)
            .expect("response must be valid json");
        let set_tbar_position_result: bool = set_tbar_position_json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        assert!(set_tbar_position_result);

        let get_transition_cursor_response = build_get_current_scene_transition_cursor_response(
            "req-get-transition-cursor",
            &registry,
        );
        let get_transition_cursor_json = nojson::RawJson::parse(&get_transition_cursor_response)
            .expect("response must be valid json");
        let transition_cursor: f64 = get_transition_cursor_json
            .value()
            .to_path_member(&["d", "responseData", "transitionCursor"])
            .and_then(|v| v.required()?.try_into())
            .expect("transitionCursor must be f64");
        assert_eq!(transition_cursor, 0.75);
    }

    #[test]
    fn build_input_events_contain_expected_fields() {
        let created_event = build_input_created_event("camera-1", "input-uuid-1", "image_source");
        let removed_event = build_input_removed_event("camera-2", "input-uuid-2", "image_source");

        for (event, expected_type, expected_name, expected_uuid) in [
            (created_event, "InputCreated", "camera-1", "input-uuid-1"),
            (removed_event, "InputRemoved", "camera-2", "input-uuid-2"),
        ] {
            let json = nojson::RawJson::parse(&event).expect("event must be valid json");
            let event_type: String = json
                .value()
                .to_path_member(&["d", "eventType"])
                .and_then(|v| v.required()?.try_into())
                .expect("eventType must be string");
            let event_data = json
                .value()
                .to_path_member(&["d", "eventData"])
                .expect("eventData access must succeed")
                .required()
                .expect("eventData must exist");
            let input_name: String = event_data
                .to_member("inputName")
                .and_then(|v| v.required()?.try_into())
                .expect("inputName must be string");
            let input_uuid: String = event_data
                .to_member("inputUuid")
                .and_then(|v| v.required()?.try_into())
                .expect("inputUuid must be string");
            assert_eq!(event_type, expected_type);
            assert_eq!(input_name, expected_name);
            assert_eq!(input_uuid, expected_uuid);
        }
    }

    #[test]
    fn build_input_settings_changed_event_contains_expected_fields() {
        let input_settings = ObswsInputSettings::VideoCaptureDevice(
            crate::obsws_input_registry::ObswsVideoCaptureDeviceSettings {
                device_id: Some("camera-1".to_owned()),
            },
        );
        let event = build_input_settings_changed_event(
            "camera-source",
            "input-uuid-3",
            "video_capture_device",
            &input_settings,
        );
        let json = nojson::RawJson::parse(&event).expect("event must be valid json");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let input_name: String = json
            .value()
            .to_path_member(&["d", "eventData", "inputName"])
            .and_then(|v| v.required()?.try_into())
            .expect("inputName must be string");
        let input_kind: String = json
            .value()
            .to_path_member(&["d", "eventData", "inputKind"])
            .and_then(|v| v.required()?.try_into())
            .expect("inputKind must be string");
        let device_id: String = json
            .value()
            .to_path_member(&["d", "eventData", "inputSettings", "device_id"])
            .and_then(|v| v.required()?.try_into())
            .expect("device_id must be string");
        assert_eq!(event_type, "InputSettingsChanged");
        assert_eq!(input_name, "camera-source");
        assert_eq!(input_kind, "video_capture_device");
        assert_eq!(device_id, "camera-1");
    }

    #[test]
    fn build_input_name_changed_event_contains_expected_fields() {
        let event =
            build_input_name_changed_event("camera-renamed", "camera-before", "input-uuid-4");
        let json = nojson::RawJson::parse(&event).expect("event must be valid json");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let input_name: String = json
            .value()
            .to_path_member(&["d", "eventData", "inputName"])
            .and_then(|v| v.required()?.try_into())
            .expect("inputName must be string");
        let old_input_name: String = json
            .value()
            .to_path_member(&["d", "eventData", "oldInputName"])
            .and_then(|v| v.required()?.try_into())
            .expect("oldInputName must be string");
        let input_uuid: String = json
            .value()
            .to_path_member(&["d", "eventData", "inputUuid"])
            .and_then(|v| v.required()?.try_into())
            .expect("inputUuid must be string");
        assert_eq!(event_type, "InputNameChanged");
        assert_eq!(input_name, "camera-renamed");
        assert_eq!(old_input_name, "camera-before");
        assert_eq!(input_uuid, "input-uuid-4");
    }

    #[test]
    fn build_scene_item_enable_state_changed_event_contains_expected_fields() {
        let event = build_scene_item_enable_state_changed_event(
            "Scene",
            "10000000-0000-0000-0000-000000000000",
            10,
            false,
        );
        let json = nojson::RawJson::parse(&event).expect("event must be valid json");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let event_intent: u32 = json
            .value()
            .to_path_member(&["d", "eventIntent"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventIntent must be u32");
        let event_data = json
            .value()
            .to_path_member(&["d", "eventData"])
            .expect("eventData access must succeed")
            .required()
            .expect("eventData must exist");
        let scene_name: String = event_data
            .to_member("sceneName")
            .and_then(|v| v.required()?.try_into())
            .expect("sceneName must be string");
        let scene_uuid: String = event_data
            .to_member("sceneUuid")
            .and_then(|v| v.required()?.try_into())
            .expect("sceneUuid must be string");
        let scene_item_id: i64 = event_data
            .to_member("sceneItemId")
            .and_then(|v| v.required()?.try_into())
            .expect("sceneItemId must be i64");
        let scene_item_enabled: bool = event_data
            .to_member("sceneItemEnabled")
            .and_then(|v| v.required()?.try_into())
            .expect("sceneItemEnabled must be bool");
        assert_eq!(event_type, "SceneItemEnableStateChanged");
        assert_eq!(event_intent, OBSWS_EVENT_SUB_SCENES);
        assert_eq!(scene_name, "Scene");
        assert_eq!(scene_uuid, "10000000-0000-0000-0000-000000000000");
        assert_eq!(scene_item_id, 10);
        assert!(!scene_item_enabled);
    }

    #[test]
    fn build_scene_item_lock_state_changed_event_contains_expected_fields() {
        let event = build_scene_item_lock_state_changed_event(
            "Scene",
            "10000000-0000-0000-0000-000000000000",
            10,
            true,
        );
        let json = nojson::RawJson::parse(&event).expect("event must be valid json");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let scene_item_locked: bool = json
            .value()
            .to_path_member(&["d", "eventData", "sceneItemLocked"])
            .and_then(|v| v.required()?.try_into())
            .expect("sceneItemLocked must be bool");
        assert_eq!(event_type, "SceneItemLockStateChanged");
        assert!(scene_item_locked);
    }

    #[test]
    fn build_scene_item_transform_changed_event_contains_expected_fields() {
        let event = build_scene_item_transform_changed_event(
            "Scene",
            "10000000-0000-0000-0000-000000000000",
            10,
            &ObswsSceneItemTransform {
                position_x: 12.0,
                position_y: 34.0,
                ..Default::default()
            },
        );
        let json = nojson::RawJson::parse(&event).expect("event must be valid json");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let position_x: f64 = json
            .value()
            .to_path_member(&["d", "eventData", "sceneItemTransform", "positionX"])
            .and_then(|v| v.required()?.try_into())
            .expect("positionX must be f64");
        assert_eq!(event_type, "SceneItemTransformChanged");
        assert_eq!(position_x, 12.0);
    }

    #[test]
    fn build_get_scene_item_id_response_succeeds_when_scene_item_exists() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
                .expect("settings must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "input-1", input, true)
            .expect("input creation must succeed");
        let request_data = nojson::RawJsonOwned::parse(
            r#"{"sceneName":"Scene","sourceName":"input-1","searchOffset":0}"#,
        )
        .expect("request data must be valid json");

        let response = build_get_scene_item_id_response(
            "req-get-scene-item-id",
            Some(&request_data),
            &registry,
        );
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let result: bool = json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        let scene_item_id: i64 = json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemId"])
            .and_then(|v| v.required()?.try_into())
            .expect("sceneItemId must be i64");
        assert!(result);
        assert_eq!(scene_item_id, 1);
    }

    #[test]
    fn build_set_scene_item_enabled_response_succeeds_when_scene_item_exists() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
                .expect("settings must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "input-1", input, true)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id("Scene", "input-1", 0)
            .expect("scene item id must exist");
        let request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemEnabled":false}}"#,
            scene_item_id
        ))
        .expect("request data must be valid json");

        let response = build_set_scene_item_enabled_response(
            "req-set-scene-item-enabled",
            Some(&request_data),
            &mut registry,
        );
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let result: bool = json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        assert!(result);
        assert!(registry.list_current_program_scene_inputs().is_empty());
    }

    #[test]
    fn build_get_scene_item_enabled_response_succeeds_when_scene_item_exists() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
                .expect("settings must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "input-1", input, true)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id("Scene", "input-1", 0)
            .expect("scene item id must exist");
        registry
            .set_scene_item_enabled("Scene", scene_item_id, false)
            .expect("set scene item enabled must succeed");
        let request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sceneItemId":{}}}"#,
            scene_item_id
        ))
        .expect("request data must be valid json");

        let response = build_get_scene_item_enabled_response(
            "req-get-scene-item-enabled",
            Some(&request_data),
            &registry,
        );
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let result: bool = json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        let scene_item_enabled: bool = json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemEnabled"])
            .and_then(|v| v.required()?.try_into())
            .expect("sceneItemEnabled must be bool");
        assert!(result);
        assert!(!scene_item_enabled);
    }

    #[test]
    fn build_get_and_set_scene_item_locked_response_succeeds_when_scene_item_exists() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
                .expect("settings must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "input-1", input, true)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id("Scene", "input-1", 0)
            .expect("scene item id must exist");
        let set_request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemLocked":true}}"#,
            scene_item_id
        ))
        .expect("request data must be valid json");

        let set_response = execute_set_scene_item_locked(
            "req-set-scene-item-locked",
            Some(&set_request_data),
            &mut registry,
        )
        .response_text;
        let set_json = nojson::RawJson::parse(&set_response).expect("response must be valid json");
        let set_result: bool = set_json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        assert!(set_result);

        let get_request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sceneItemId":{}}}"#,
            scene_item_id
        ))
        .expect("request data must be valid json");
        let get_response = build_get_scene_item_locked_response(
            "req-get-scene-item-locked",
            Some(&get_request_data),
            &registry,
        );
        let get_json = nojson::RawJson::parse(&get_response).expect("response must be valid json");
        let locked: bool = get_json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemLocked"])
            .and_then(|v| v.required()?.try_into())
            .expect("sceneItemLocked must be bool");
        assert!(locked);
    }

    #[test]
    fn build_get_and_set_scene_item_blend_mode_response_succeeds_when_scene_item_exists() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
                .expect("settings must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "input-1", input, true)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id("Scene", "input-1", 0)
            .expect("scene item id must exist");
        let set_request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemBlendMode":"OBS_BLEND_ADDITIVE"}}"#,
            scene_item_id
        ))
        .expect("request data must be valid json");

        let set_response = build_set_scene_item_blend_mode_response(
            "req-set-scene-item-blend-mode",
            Some(&set_request_data),
            &mut registry,
        );
        let set_json = nojson::RawJson::parse(&set_response).expect("response must be valid json");
        let set_result: bool = set_json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        assert!(set_result);

        let get_request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sceneItemId":{}}}"#,
            scene_item_id
        ))
        .expect("request data must be valid json");
        let get_response = build_get_scene_item_blend_mode_response(
            "req-get-scene-item-blend-mode",
            Some(&get_request_data),
            &registry,
        );
        let get_json = nojson::RawJson::parse(&get_response).expect("response must be valid json");
        let blend_mode: String = get_json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemBlendMode"])
            .and_then(|v| v.required()?.try_into())
            .expect("sceneItemBlendMode must be string");
        assert_eq!(blend_mode, "OBS_BLEND_ADDITIVE");
    }

    #[test]
    fn build_get_and_set_scene_item_transform_response_succeeds_when_scene_item_exists() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
                .expect("settings must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "input-1", input, true)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id("Scene", "input-1", 0)
            .expect("scene item id must exist");
        let set_request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemTransform":{{"positionX":321.0}}}}"#,
            scene_item_id
        ))
        .expect("request data must be valid json");

        let set_response = execute_set_scene_item_transform(
            "req-set-scene-item-transform",
            Some(&set_request_data),
            &mut registry,
        )
        .response_text;
        let set_json = nojson::RawJson::parse(&set_response).expect("response must be valid json");
        let set_result: bool = set_json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        assert!(set_result);

        let get_request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sceneItemId":{}}}"#,
            scene_item_id
        ))
        .expect("request data must be valid json");
        let get_response = build_get_scene_item_transform_response(
            "req-get-scene-item-transform",
            Some(&get_request_data),
            &registry,
        );
        let get_json = nojson::RawJson::parse(&get_response).expect("response must be valid json");
        let position_x: f64 = get_json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemTransform", "positionX"])
            .and_then(|v| v.required()?.try_into())
            .expect("positionX must be f64");
        assert_eq!(position_x, 321.0);
    }

    #[test]
    fn execute_set_scene_item_transform_rejects_invalid_alignment_value() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
                .expect("settings must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "input-1", input, true)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id("Scene", "input-1", 0)
            .expect("scene item id must exist");
        let request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemTransform":{{"alignment":3}}}}"#,
            scene_item_id
        ))
        .expect("request data must be valid json");

        let response = execute_set_scene_item_transform(
            "req-set-scene-item-transform-invalid-alignment",
            Some(&request_data),
            &mut registry,
        )
        .response_text;
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let result: bool = json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        let code: i64 = json
            .value()
            .to_path_member(&["d", "requestStatus", "code"])
            .and_then(|v| v.required()?.try_into())
            .expect("code must be i64");
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
    }

    #[test]
    fn build_get_scene_item_list_response_succeeds_when_scene_exists() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
                .expect("settings must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "input-1", input, true)
            .expect("input creation must succeed");
        let request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene"}"#)
            .expect("request data must be valid json");

        let response = build_get_scene_item_list_response(
            "req-get-scene-item-list",
            Some(&request_data),
            &registry,
        );
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let result: bool = json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        let scene_items = json
            .value()
            .to_path_member(&["d", "responseData", "sceneItems"])
            .expect("sceneItems access must succeed")
            .required()
            .expect("sceneItems must exist")
            .to_array()
            .expect("sceneItems must be array");
        let scene_name = json
            .value()
            .to_path_member(&["d", "responseData", "sceneName"])
            .expect("sceneName access must succeed")
            .optional();
        assert!(result);
        assert!(scene_items.count() >= 1);
        assert!(scene_name.is_none());
    }

    #[test]
    fn build_create_scene_item_response_succeeds_when_source_exists() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
                .expect("settings must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        let created = registry
            .create_input("Scene", "input-1", input, false)
            .expect("input creation must succeed");
        let request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sourceUuid":"{}","sceneItemEnabled":true}}"#,
            created.input_uuid
        ))
        .expect("request data must be valid json");

        let response =
            execute_create_scene_item("req-create-scene-item", Some(&request_data), &mut registry)
                .response_text;
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let result: bool = json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        let scene_item_id: i64 = json
            .value()
            .to_path_member(&["d", "responseData", "sceneItemId"])
            .and_then(|v| v.required()?.try_into())
            .expect("sceneItemId must be i64");
        assert!(result);
        assert!(scene_item_id > 0);
    }

    #[test]
    fn build_set_scene_item_index_response_rejects_invalid_index() {
        let mut registry = ObswsInputRegistry::new_for_test();
        let input = ObswsInput::from_kind_and_settings(
            "image_source",
            nojson::RawJsonOwned::parse(r#"{"file":"/tmp/image.png"}"#)
                .expect("settings must be valid json")
                .value(),
        )
        .expect("input settings must be valid");
        registry
            .create_input("Scene", "input-1", input, true)
            .expect("input creation must succeed");
        let scene_item_id = registry
            .get_scene_item_id("Scene", "input-1", 0)
            .expect("scene item id must exist");
        let request_data = nojson::RawJsonOwned::parse(format!(
            r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemIndex":100}}"#,
            scene_item_id
        ))
        .expect("request data must be valid json");

        let response = execute_set_scene_item_index(
            "req-set-scene-item-index",
            Some(&request_data),
            &mut registry,
        )
        .response_text;
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let result: bool = json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        let code: i64 = json
            .value()
            .to_path_member(&["d", "requestStatus", "code"])
            .and_then(|v| v.required()?.try_into())
            .expect("code must be i64");
        assert!(!result);
        assert_eq!(code, REQUEST_STATUS_INVALID_REQUEST_FIELD);
    }

    #[test]
    fn build_scene_item_created_event_contains_expected_fields() {
        let event = build_scene_item_created_event(
            "Scene",
            "scene-uuid-1",
            10,
            "camera-1",
            "input-uuid-1",
            0,
        );
        let json = nojson::RawJson::parse(&event).expect("event must be valid json");
        let event_type: String = json
            .value()
            .to_path_member(&["d", "eventType"])
            .and_then(|v| v.required()?.try_into())
            .expect("eventType must be string");
        let scene_item_id: i64 = json
            .value()
            .to_path_member(&["d", "eventData", "sceneItemId"])
            .and_then(|v| v.required()?.try_into())
            .expect("sceneItemId must be i64");
        assert_eq!(event_type, "SceneItemCreated");
        assert_eq!(scene_item_id, 10);
    }

    #[test]
    fn build_remove_scene_response_succeeds_when_scene_exists() {
        let mut registry = ObswsInputRegistry::new_for_test();
        registry
            .create_scene("Scene B")
            .expect("scene creation must succeed");
        let request_data = nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
            .expect("requestData must be valid json");

        let response =
            build_remove_scene_response("req-remove-scene", Some(&request_data), &mut registry);
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let result: bool = json
            .value()
            .to_path_member(&["d", "requestStatus", "result"])
            .and_then(|v| v.required()?.try_into())
            .expect("result must be bool");
        assert!(result);
    }

    #[test]
    fn build_and_parse_request_batch_response_preserves_fields() {
        let response = build_request_batch_response(
            "batch-1",
            &[
                RequestBatchResult {
                    request_type: "GetVersion".to_owned(),
                    request_status_result: true,
                    request_status_code: REQUEST_STATUS_SUCCESS,
                    request_status_comment: None,
                    response_data: Some(
                        nojson::RawJsonOwned::parse(r#"{"rpcVersion":1}"#)
                            .expect("responseData must be valid json"),
                    ),
                },
                RequestBatchResult {
                    request_type: "CreateScene".to_owned(),
                    request_status_result: false,
                    request_status_code: REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
                    request_status_comment: Some("Scene already exists".to_owned()),
                    response_data: None,
                },
            ],
        );
        let json = nojson::RawJson::parse(&response).expect("response must be valid json");
        let op: i64 = json
            .value()
            .to_member("op")
            .and_then(|v| v.required()?.try_into())
            .expect("op must be i64");
        assert_eq!(op, OBSWS_OP_REQUEST_BATCH_RESPONSE);

        let results = json
            .value()
            .to_path_member(&["d", "results"])
            .expect("results access must succeed")
            .required()
            .expect("results must exist");
        let mut results = results.to_array().expect("results must be array");
        let first = results.next().expect("first result must exist");
        let first_request_type: String = first
            .to_member("requestType")
            .and_then(|v| v.required()?.try_into())
            .expect("requestType must be string");
        assert_eq!(first_request_type, "GetVersion");

        let source_response = build_get_version_response("req-1");
        let parsed = parse_request_response_for_batch_result(&source_response)
            .expect("request response must be parsed");
        assert_eq!(parsed.request_type, "GetVersion");
        assert!(parsed.request_status_result);
        assert_eq!(parsed.request_status_code, REQUEST_STATUS_SUCCESS);
        assert!(parsed.response_data.is_some());
    }
}
