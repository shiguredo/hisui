use crate::obsws::auth::ObswsAuthentication;
use crate::obsws::input_registry::{
    ObswsInput, ObswsInputEntry, ObswsInputRegistry, ObswsInputSettings, ObswsSceneItemBlendMode,
    ObswsSceneItemRef, ObswsSceneItemTransformPatch, ParseInputSettingsError,
    SetSceneItemIndexResult, SetSceneItemLockedResult, SetSceneItemTransformResult,
};
use crate::obsws::protocol::{
    OBS_STUDIO_VERSION, OBSWS_OP_HELLO, OBSWS_OP_IDENTIFIED, OBSWS_OP_REQUEST_BATCH_RESPONSE,
    OBSWS_OP_REQUEST_RESPONSE, OBSWS_RPC_VERSION, OBSWS_VERSION,
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_DATA,
    REQUEST_STATUS_MISSING_REQUEST_FIELD, REQUEST_STATUS_RESOURCE_NOT_FOUND,
    REQUEST_STATUS_SUCCESS,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ObswsOutputRuntimeStats {
    pub(crate) stream_output_bytes: u64,
    pub(crate) stream_total_frames: u64,
    pub(crate) stream_skipped_frames: u64,
    pub(crate) record_total_frames: u64,
    pub(crate) record_skipped_frames: u64,
}

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

struct SetSceneNameFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    new_scene_name: String,
}

struct SetCurrentProgramSceneFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
}

struct GetSceneSceneTransitionOverrideFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
}

struct SetSceneSceneTransitionOverrideFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    transition_name: Option<String>,
    transition_duration: Option<i64>,
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

struct RemoveSceneFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
}

struct GetSceneItemIdFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    source_name: Option<String>,
    source_uuid: Option<String>,
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
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    destination_scene_name: Option<String>,
    destination_scene_uuid: Option<String>,
    scene_item_id: i64,
}

/// sceneName / sceneUuid + sceneItemId の共通フィールド。
/// Get/Set の SceneItemSource, SceneItemIndex, SceneItemLocked,
/// SceneItemBlendMode, SceneItemTransform で共有する。
struct SceneItemLookupFields {
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
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
}

struct SetSceneItemEnabledFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
    scene_item_enabled: bool,
}

struct SetSceneItemLockedFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
    scene_item_locked: bool,
}

struct SetSceneItemBlendModeFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
    scene_item_blend_mode: ObswsSceneItemBlendMode,
}

struct SetSceneItemTransformFields {
    scene_name: Option<String>,
    scene_uuid: Option<String>,
    scene_item_id: i64,
    scene_item_transform: ObswsSceneItemTransformPatch,
}

pub(crate) struct SetStreamServiceSettingsFields {
    pub(crate) stream_service_type: String,
    pub(crate) server: String,
    pub(crate) key: Option<String>,
}

pub(crate) struct SetRecordDirectoryFields {
    pub(crate) record_directory: String,
}

#[derive(Debug, Clone)]
pub struct RequestBatchResult {
    pub request_id: String,
    pub request_type: String,
    pub request_status_result: bool,
    pub request_status_code: i64,
    pub request_status_comment: Option<String>,
    pub response_data: Option<nojson::RawJsonOwned>,
}

#[derive(Debug, Clone)]
pub struct SetSceneItemIndexExecution {
    pub response_text: nojson::RawJsonOwned,
    pub event_context: Option<SetSceneItemIndexEventContext>,
}

#[derive(Debug, Clone)]
pub struct SetSceneItemIndexEventContext {
    pub scene_name: String,
    pub scene_uuid: String,
    pub set_result: SetSceneItemIndexResult,
}

#[derive(Debug, Clone)]
pub struct SetSceneItemLockedExecution {
    pub response_text: nojson::RawJsonOwned,
    pub event_context: Option<SetSceneItemLockedEventContext>,
}

#[derive(Debug, Clone)]
pub struct SetSceneItemLockedEventContext {
    pub scene_name: String,
    pub scene_uuid: String,
    pub scene_item_id: i64,
    pub scene_item_locked: bool,
    pub set_result: SetSceneItemLockedResult,
}

#[derive(Debug, Clone)]
pub struct SetSceneItemTransformExecution {
    pub response_text: nojson::RawJsonOwned,
    pub event_context: Option<SetSceneItemTransformEventContext>,
}

#[derive(Debug, Clone)]
pub struct SetSceneItemTransformEventContext {
    pub scene_name: String,
    pub scene_uuid: String,
    pub scene_item_id: i64,
    pub set_result: SetSceneItemTransformResult,
}

#[derive(Debug, Clone)]
pub struct CreateSceneItemExecution {
    pub response_text: nojson::RawJsonOwned,
    pub created: Option<ObswsSceneItemRef>,
}

#[derive(Debug, Clone)]
pub struct DuplicateSceneItemExecution {
    pub response_text: nojson::RawJsonOwned,
    pub duplicated: Option<ObswsSceneItemRef>,
}

#[derive(Debug, Clone)]
pub struct CreateInputExecution {
    pub response_text: nojson::RawJsonOwned,
    pub created: Option<CreateInputCreated>,
}

#[derive(Debug, Clone)]
pub struct CreateInputCreated {
    pub input_entry: ObswsInputEntry,
    pub default_settings: ObswsInputSettings,
    pub scene_item_ref: ObswsSceneItemRef,
}

#[derive(Debug, Clone)]
pub struct SetInputSettingsExecution {
    pub response_text: nojson::RawJsonOwned,
    pub request_succeeded: bool,
}

pub(crate) fn parse_input_lookup_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<(Option<String>, Option<String>), nojson::JsonParseError> {
    let input_name = optional_non_empty_string_member(request_data, "inputName")?;
    let input_uuid = optional_non_empty_string_member(request_data, "inputUuid")?;

    if input_name.is_none() && input_uuid.is_none() {
        return Err(request_data.invalid("required member 'inputName or inputUuid' is missing"));
    }

    Ok((input_uuid, input_name))
}

/// TriggerMediaInputAction のリクエストフィールドをパースする。
/// (input_uuid, input_name, media_action) を返す。
pub(crate) fn parse_trigger_media_input_action_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<(Option<String>, Option<String>, String), nojson::JsonParseError> {
    let (input_uuid, input_name) = parse_input_lookup_fields(request_data)?;
    let media_action = optional_non_empty_string_member(request_data, "mediaAction")?;
    let Some(media_action) = media_action else {
        return Err(request_data.invalid("required member 'mediaAction' is missing"));
    };
    Ok((input_uuid, input_name, media_action))
}

/// SetMediaInputCursor のリクエストフィールドをパースする。
/// (input_uuid, input_name, mediaCursor) を返す。
pub(crate) fn parse_set_media_input_cursor_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<(Option<String>, Option<String>, i64), nojson::JsonParseError> {
    let (input_uuid, input_name) = parse_input_lookup_fields(request_data)?;
    let cursor: Option<i64> = request_data.to_member("mediaCursor")?.try_into()?;
    let Some(cursor) = cursor else {
        return Err(request_data.invalid("required member 'mediaCursor' is missing"));
    };
    Ok((input_uuid, input_name, cursor))
}

/// OffsetMediaInputCursor のリクエストフィールドをパースする。
/// (input_uuid, input_name, mediaCursorOffset) を返す。
pub(crate) fn parse_offset_media_input_cursor_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<(Option<String>, Option<String>, i64), nojson::JsonParseError> {
    let (input_uuid, input_name) = parse_input_lookup_fields(request_data)?;
    let offset: Option<i64> = request_data.to_member("mediaCursorOffset")?.try_into()?;
    let Some(offset) = offset else {
        return Err(request_data.invalid("required member 'mediaCursorOffset' is missing"));
    };
    Ok((input_uuid, input_name, offset))
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

pub(crate) fn parse_get_input_properties_list_property_items_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetInputPropertiesListPropertyItemsFields, nojson::JsonParseError> {
    let (input_uuid, input_name) = parse_input_lookup_fields(request_data)?;
    let property_name = required_non_empty_string_member(request_data, "propertyName")?;
    Ok(GetInputPropertiesListPropertyItemsFields {
        input_uuid,
        input_name,
        property_name,
    })
}

pub(crate) struct GetInputPropertiesListPropertyItemsFields {
    pub input_uuid: Option<String>,
    pub input_name: Option<String>,
    pub property_name: String,
}

fn parse_create_scene_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<CreateSceneFields, nojson::JsonParseError> {
    let scene_name = required_non_empty_string_member(request_data, "sceneName")?;
    Ok(CreateSceneFields { scene_name })
}

fn parse_set_scene_name_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetSceneNameFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let new_scene_name = required_non_empty_string_member(request_data, "newSceneName")?;
    Ok(SetSceneNameFields {
        scene_name,
        scene_uuid,
        new_scene_name,
    })
}

fn parse_set_current_program_scene_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetCurrentProgramSceneFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    Ok(SetCurrentProgramSceneFields {
        scene_name,
        scene_uuid,
    })
}

fn parse_get_scene_scene_transition_override_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneSceneTransitionOverrideFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    Ok(GetSceneSceneTransitionOverrideFields {
        scene_name,
        scene_uuid,
    })
}

fn parse_set_scene_scene_transition_override_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SetSceneSceneTransitionOverrideFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let transition_name = optional_non_empty_string_member(request_data, "transitionName")?;
    let transition_duration: Option<i64> =
        request_data.to_member("transitionDuration")?.try_into()?;
    Ok(SetSceneSceneTransitionOverrideFields {
        scene_name,
        scene_uuid,
        transition_name,
        transition_duration,
    })
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

fn parse_remove_scene_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<RemoveSceneFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    Ok(RemoveSceneFields {
        scene_name,
        scene_uuid,
    })
}

fn parse_get_scene_item_id_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneItemIdFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let (source_name, source_uuid) = parse_source_lookup_fields(request_data)?;
    let search_offset: Option<i64> = request_data.to_member("searchOffset")?.try_into()?;
    Ok(GetSceneItemIdFields {
        scene_name,
        scene_uuid,
        source_name,
        source_uuid,
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
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let (destination_scene_name, destination_scene_uuid) =
        parse_scene_lookup_fields(request_data, "destinationSceneName", "destinationSceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(DuplicateSceneItemFields {
        scene_name,
        scene_uuid,
        destination_scene_name,
        destination_scene_uuid,
        scene_item_id,
    })
}

fn parse_scene_item_lookup_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<SceneItemLookupFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(SceneItemLookupFields {
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
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
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
        scene_uuid,
        scene_item_id,
        scene_item_enabled,
    })
}

fn parse_get_scene_item_enabled_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<GetSceneItemEnabledFields, nojson::JsonParseError> {
    let (scene_name, scene_uuid) =
        parse_scene_lookup_fields(request_data, "sceneName", "sceneUuid")?;
    let scene_item_id: i64 = request_data
        .to_member("sceneItemId")?
        .required()?
        .try_into()?;
    Ok(GetSceneItemEnabledFields {
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

pub(crate) fn parse_set_stream_service_settings_fields(
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

pub(crate) fn parse_set_record_directory_fields(
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

struct PersistentDataFields {
    realm: String,
    slot_name: String,
}

/// GetPersistentData / SetPersistentData 共通のフィールドをパースする。
/// realm の値の妥当性検証（GLOBAL のみ許可）は呼び出し側で行う。
fn parse_persistent_data_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<PersistentDataFields, nojson::JsonParseError> {
    let realm = required_non_empty_string_member(request_data, "realm")?;
    let slot_name = required_non_empty_string_member(request_data, "slotName")?;
    Ok(PersistentDataFields { realm, slot_name })
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
) -> Result<(Option<String>, Option<String>, i64, bool), nojson::JsonParseError> {
    let fields = parse_set_scene_item_enabled_fields(request_data)?;
    Ok((
        fields.scene_name,
        fields.scene_uuid,
        fields.scene_item_id,
        fields.scene_item_enabled,
    ))
}

pub(crate) fn parse_request_data_or_error_response<T, F>(
    request_type: &str,
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    parser: F,
) -> Result<T, nojson::RawJsonOwned>
where
    F: FnOnce(nojson::RawJsonValue<'_, '_>) -> Result<T, nojson::JsonParseError>,
{
    let Some(request_data) = request_data else {
        return Err(build_request_response_error(
            request_type,
            request_id,
            REQUEST_STATUS_MISSING_REQUEST_DATA,
            "Missing required requestData field",
        ));
    };

    parser(request_data.value()).map_err(|e| {
        let code = request_status_code_for_parse_error(&e);
        build_request_response_error(request_type, request_id, code, &e.to_string())
    })
}

/// シーン名とシーン UUID のペアを解決する。
/// resolve_scene_name が成功した直後に get_scene_uuid を呼ぶため、
/// UUID 取得失敗は内部エラーとして扱う。
fn resolve_scene_name_or_error(
    request_type: &str,
    request_id: &str,
    input_registry: &ObswsInputRegistry,
    scene_name: Option<&str>,
    scene_uuid: Option<&str>,
) -> Result<(String, String), nojson::RawJsonOwned> {
    let resolved_name = input_registry
        .resolve_scene_name(scene_name, scene_uuid)
        .ok_or_else(|| {
            build_request_response_error(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
            )
        })?;
    let resolved_uuid = input_registry
        .get_scene_uuid(&resolved_name)
        .ok_or_else(|| {
            build_request_response_error(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Internal error: resolved scene UUID not found",
            )
        })?;
    Ok((resolved_name, resolved_uuid))
}

pub(crate) fn request_status_code_for_parse_error(error: &nojson::JsonParseError) -> i64 {
    // OBS WebSocket の 300 / 400 の厳密分類は nojson のエラー種別だけでは判別しづらいため、
    // 現状は required member 欠落パターンのみ 300 として扱い、それ以外は 400 とする。
    //
    // TODO(nojson): この判定は nojson のエラーメッセージ文字列に依存しており、
    // nojson 側のメッセージ変更で壊れるリスクがある。
    // nojson に JsonParseError の種別（欠落 / 型不一致 / 範囲外など）を
    // 構造的に取得できる API が追加された場合は、文字列マッチを廃止すること。
    // 現状は response/tests.rs のテストで挙動を担保している。
    if let nojson::JsonParseError::InvalidValue { error, .. } = error {
        let reason = error.to_string();
        if reason.contains("required member") && reason.contains("is missing") {
            return REQUEST_STATUS_MISSING_REQUEST_FIELD;
        }
    }
    REQUEST_STATUS_INVALID_REQUEST_FIELD
}

/// Hello メッセージを構築する。
pub fn build_hello_message(authentication: Option<&ObswsAuthentication>) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_HELLO)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("obsWebSocketVersion", OBSWS_VERSION)?;
                f.member("rpcVersion", OBSWS_RPC_VERSION)?;
                f.member("obsStudioVersion", OBS_STUDIO_VERSION)?;
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
}

pub fn build_identified_message(negotiated_rpc_version: u32) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_IDENTIFIED)?;
        f.member(
            "d",
            nojson::object(|f| f.member("negotiatedRpcVersion", negotiated_rpc_version)),
        )
    })
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

/// outputs BTreeMap から output 統計情報を収集する。
pub(crate) fn collect_output_runtime_stats_from_outputs(
    outputs: &std::collections::BTreeMap<
        String,
        crate::obsws::coordinator::output_dynamic::OutputState,
    >,
    pipeline_handle: Option<&crate::MediaPipelineHandle>,
) -> ObswsOutputRuntimeStats {
    use crate::obsws::coordinator::output_dynamic::OutputRun;

    let Some(pipeline_handle) = pipeline_handle else {
        return ObswsOutputRuntimeStats::default();
    };
    let Ok(entries) = pipeline_handle.stats().entries() else {
        return ObswsOutputRuntimeStats::default();
    };

    let stream_run = outputs.get("stream").and_then(|o| {
        o.runtime.run.as_ref().and_then(|r| match r {
            OutputRun::Stream(run) => Some(run),
            _ => None,
        })
    });
    let record_run = outputs.get("record").and_then(|o| {
        o.runtime.run.as_ref().and_then(|r| match r {
            OutputRun::Record(run) => Some(run),
            _ => None,
        })
    });

    let stream_total_frames = stream_run
        .map(|run| {
            find_counter_metric(
                &entries,
                &run.video.encoder_processor_id,
                "total_output_video_frame_count",
            )
        })
        .unwrap_or(0);
    let stream_output_bytes = stream_run
        .map(|run| find_counter_metric(&entries, &run.publisher_processor_id, "total_sent_bytes"))
        .unwrap_or(0);
    let stream_skipped_frames = stream_run
        .map(|run| {
            find_counter_metric(
                &entries,
                &run.publisher_processor_id,
                "total_waiting_keyframe_dropped_video_frame_count",
            )
        })
        .unwrap_or(0);
    let (record_total_frames, record_skipped_frames) = record_run
        .map(|run| {
            (
                find_counter_metric(
                    &entries,
                    &run.writer_processor_id,
                    "total_video_sample_count",
                ),
                find_counter_metric(
                    &entries,
                    &run.writer_processor_id,
                    "total_keyframe_wait_dropped_video_frame_count",
                ),
            )
        })
        .unwrap_or((0, 0));

    ObswsOutputRuntimeStats {
        stream_output_bytes,
        stream_total_frames,
        stream_skipped_frames,
        record_total_frames,
        record_skipped_frames,
    }
}

fn find_counter_metric(
    entries: &[crate::stats::StatsEntry],
    processor_id: &crate::ProcessorId,
    metric_name: &'static str,
) -> u64 {
    entries
        .iter()
        .find(|entry| {
            entry.metric_name == metric_name
                && entry.labels.get("processor_id").map(String::as_str) == Some(processor_id.get())
        })
        .and_then(|entry| entry.value.as_counter())
        .unwrap_or(0)
}

pub fn build_request_batch_response(
    request_id: &str,
    results: &[RequestBatchResult],
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
                                f.member("requestId", &result.request_id)?;
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
}

pub fn parse_request_response_for_batch_result(
    response: &nojson::RawJsonOwned,
) -> crate::Result<RequestBatchResult> {
    let d = response.value().to_member("d")?.required()?;
    let request_type: String = d.to_member("requestType")?.required()?.try_into()?;
    let request_id: String = d.to_member("requestId")?.required()?.try_into()?;
    let request_status = d.to_member("requestStatus")?.required()?;
    let request_status_result: bool = request_status.to_member("result")?.required()?.try_into()?;
    let request_status_code: i64 = request_status.to_member("code")?.required()?.try_into()?;
    let request_status_comment: Option<String> = request_status.to_member("comment")?.try_into()?;
    let response_data: Option<nojson::RawJsonOwned> = d
        .to_member("responseData")?
        .map(nojson::RawJsonOwned::try_from)?;

    Ok(RequestBatchResult {
        request_id,
        request_type,
        request_status_result,
        request_status_code,
        request_status_comment,
        response_data,
    })
}

/// responseData 付きの成功レスポンスを構築する共通ヘルパー。
/// `response_data` クロージャで responseData オブジェクトの中身を書き込む。
pub fn build_request_response_success<F>(
    request_type: &str,
    request_id: &str,
    response_data: F,
) -> nojson::RawJsonOwned
where
    F: Fn(&mut nojson::JsonObjectFormatter<'_, '_, '_>) -> std::fmt::Result,
{
    nojson::RawJsonOwned::object(|f| {
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
                f.member("responseData", nojson::object(|f| response_data(f)))
            }),
        )
    })
}

/// responseData が不要な成功レスポンスを構築する共通ヘルパー。
/// OBS 互換で responseData フィールド自体を省略する。
pub fn build_request_response_success_no_data(
    request_type: &str,
    request_id: &str,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
                )
            }),
        )
    })
}

/// comment なしのエラーレスポンスを構築する（OBS が comment を返さないリクエスト用）
pub fn build_request_response_error_without_comment(
    request_type: &str,
    request_id: &str,
    code: i64,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
                        f.member("code", code)
                    }),
                )
            }),
        )
    })
}

pub fn build_request_response_error(
    request_type: &str,
    request_id: &str,
    code: i64,
    comment: &str,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
}

#[cfg(test)]
#[path = "response/tests.rs"]
mod tests;
