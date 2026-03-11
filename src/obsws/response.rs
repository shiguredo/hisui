use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::{
    ObswsInput, ObswsInputRegistry, ObswsSceneItemBlendMode, ObswsSceneItemRef,
    ObswsSceneItemTransformPatch, ParseInputSettingsError, SetSceneItemIndexResult,
    SetSceneItemLockedResult, SetSceneItemTransformResult,
};
use crate::obsws_protocol::{
    OBSWS_OP_HELLO, OBSWS_OP_IDENTIFIED, OBSWS_OP_REQUEST_BATCH_RESPONSE,
    OBSWS_OP_REQUEST_RESPONSE, OBSWS_RPC_VERSION, OBSWS_VERSION,
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_FIELD,
    REQUEST_STATUS_RESOURCE_NOT_FOUND,
};
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

mod event;
mod general;
mod input;
mod output;
mod scene;
mod scene_item;

pub use event::*;
pub use general::*;
pub use input::*;
pub use output::*;
pub use scene::*;
pub use scene_item::*;

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

#[cfg(test)]
#[path = "response/tests.rs"]
mod tests;
