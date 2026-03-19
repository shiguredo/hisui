use crate::obsws_input_registry::{
    CreateSceneError, GetSceneSceneTransitionOverrideError, ObswsInputRegistry,
    SetCurrentProgramSceneError, SetCurrentSceneTransitionDurationError,
    SetCurrentSceneTransitionError, SetSceneNameError, SetSceneSceneTransitionOverrideError,
};
use crate::obsws_protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
    REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED, REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
    REQUEST_STATUS_RESOURCE_NOT_FOUND, REQUEST_STATUS_STUDIO_MODE_NOT_ACTIVE,
};

use super::{
    parse_create_scene_fields, parse_get_scene_scene_transition_override_fields,
    parse_remove_scene_fields, parse_request_data_or_error_response,
    parse_set_current_program_scene_fields, parse_set_current_scene_transition_duration_fields,
    parse_set_current_scene_transition_fields, parse_set_current_scene_transition_settings_fields,
    parse_set_scene_name_fields, parse_set_scene_scene_transition_override_fields,
};

struct ObswsSceneTransitionEntry {
    transition_name: String,
    transition_uuid: String,
    transition_kind: String,
    transition_fixed: bool,
    transition_configurable: bool,
}

impl nojson::DisplayJson for ObswsSceneTransitionEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("transitionName", &self.transition_name)?;
            f.member("transitionUuid", &self.transition_uuid)?;
            f.member("transitionKind", &self.transition_kind)?;
            f.member("transitionFixed", self.transition_fixed)?;
            f.member("transitionConfigurable", self.transition_configurable)
        })
        .fmt(f)
    }
}

fn is_fixed_transition(_transition_name: &str) -> bool {
    // OBS のビルトイントランジションはすべてカスタム設定非対応
    true
}

/// トランジション名から決定的な UUID を生成する
fn transition_uuid(transition_name: &str) -> String {
    match transition_name {
        "cut_transition" => "20000000-0000-0000-0000-000000000000".to_owned(),
        "fade_transition" => "20000000-0000-0000-0000-000000000001".to_owned(),
        other => format!("20000000-0000-0000-0000-{:012x}", {
            let mut hash: u64 = 5381;
            for byte in other.bytes() {
                hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
            }
            hash & 0xFFFF_FFFF_FFFF
        }),
    }
}

pub fn build_get_current_program_scene_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let current_program_scene = input_registry.current_program_scene();
    let scene_name = current_program_scene
        .as_ref()
        .map(|scene| scene.scene_name.as_str())
        .unwrap_or_default();
    let scene_uuid = current_program_scene
        .as_ref()
        .map(|scene| scene.scene_uuid.as_str())
        .unwrap_or_default();
    super::build_request_response_success("GetCurrentProgramScene", request_id, |f| {
        f.member("sceneName", scene_name)?;
        f.member("sceneUuid", scene_uuid)?;
        f.member("currentProgramSceneName", scene_name)?;
        f.member("currentProgramSceneUuid", scene_uuid)
    })
}

pub fn build_set_current_program_scene_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "SetCurrentProgramScene",
        request_id,
        request_data,
        parse_set_current_program_scene_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match super::resolve_scene_name_or_error(
        "SetCurrentProgramScene",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if let Err(SetCurrentProgramSceneError::SceneNotFound) =
        input_registry.set_current_program_scene(&scene_name)
    {
        return super::build_request_response_error(
            "SetCurrentProgramScene",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Scene not found",
        );
    }
    super::build_request_response_success_no_data("SetCurrentProgramScene", request_id)
}

pub fn build_get_current_preview_scene_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_error(
        "GetCurrentPreviewScene",
        request_id,
        REQUEST_STATUS_STUDIO_MODE_NOT_ACTIVE,
        "Studio mode is not enabled",
    )
}

pub fn build_set_current_preview_scene_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_error(
        "SetCurrentPreviewScene",
        request_id,
        REQUEST_STATUS_STUDIO_MODE_NOT_ACTIVE,
        "Studio mode is not enabled",
    )
}

pub fn build_get_transition_kind_list_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    super::build_request_response_success("GetTransitionKindList", request_id, |f| {
        f.member(
            "transitionKinds",
            input_registry.supported_transition_kinds(),
        )
    })
}

pub fn build_get_scene_transition_list_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let transitions: Vec<ObswsSceneTransitionEntry> = input_registry
        .supported_transition_kinds()
        .iter()
        .map(|name| ObswsSceneTransitionEntry {
            transition_name: (*name).to_owned(),
            transition_uuid: transition_uuid(name),
            transition_kind: (*name).to_owned(),
            transition_fixed: is_fixed_transition(name),
            transition_configurable: false,
        })
        .collect();
    let current_transition_name = input_registry.current_scene_transition_name();
    let current_transition_uuid = transition_uuid(current_transition_name);

    super::build_request_response_success("GetSceneTransitionList", request_id, |f| {
        f.member("currentSceneTransitionName", current_transition_name)?;
        f.member("currentSceneTransitionUuid", &current_transition_uuid)?;
        f.member("currentSceneTransitionKind", current_transition_name)?;
        f.member("transitions", &transitions)
    })
}

pub fn build_get_current_scene_transition_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let current_transition_name = input_registry.current_scene_transition_name();
    let current_transition_uuid = transition_uuid(current_transition_name);
    let current_transition_duration_ms = input_registry.current_scene_transition_duration_ms();
    let fixed = is_fixed_transition(current_transition_name);
    super::build_request_response_success("GetCurrentSceneTransition", request_id, |f| {
        f.member("transitionName", current_transition_name)?;
        f.member("transitionUuid", &current_transition_uuid)?;
        f.member("transitionKind", current_transition_name)?;
        f.member("transitionFixed", fixed)?;
        f.member("transitionConfigurable", false)?;
        // OBS のビルトイントランジションは transitionSettings を常に null で返す
        f.member("transitionSettings", Option::<&str>::None)?;
        f.member("transitionDuration", current_transition_duration_ms)
    })
}

pub fn build_get_scene_scene_transition_override_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetSceneSceneTransitionOverride",
        request_id,
        request_data,
        parse_get_scene_scene_transition_override_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match super::resolve_scene_name_or_error(
        "GetSceneSceneTransitionOverride",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let override_entry = match input_registry.get_scene_transition_override(&scene_name) {
        Ok(override_entry) => override_entry,
        Err(GetSceneSceneTransitionOverrideError::SceneNotFound) => {
            return super::build_request_response_error(
                "GetSceneSceneTransitionOverride",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
            );
        }
    };

    super::build_request_response_success("GetSceneSceneTransitionOverride", request_id, |f| {
        f.member("transitionName", &override_entry.transition_name)?;
        f.member("transitionDuration", override_entry.transition_duration)
    })
}

pub fn build_set_current_scene_transition_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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
        return super::build_request_response_error(
            "SetCurrentSceneTransition",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Transition not found",
        );
    }
    super::build_request_response_success_no_data("SetCurrentSceneTransition", request_id)
}

pub fn build_set_current_scene_transition_duration_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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
        return super::build_request_response_error(
            "SetCurrentSceneTransitionDuration",
            request_id,
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "Invalid transitionDuration field",
        );
    }
    super::build_request_response_success_no_data("SetCurrentSceneTransitionDuration", request_id)
}

pub fn build_set_current_scene_transition_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "SetCurrentSceneTransitionSettings",
        request_id,
        request_data,
        parse_set_current_scene_transition_settings_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    // Cut 等の固定トランジションはカスタム設定をサポートしない
    if is_fixed_transition(input_registry.current_scene_transition_name()) {
        return super::build_request_response_error(
            "SetCurrentSceneTransitionSettings",
            request_id,
            REQUEST_STATUS_RESOURCE_ACTION_NOT_SUPPORTED,
            "Transition does not support custom settings",
        );
    }
    input_registry
        .set_current_scene_transition_settings(fields.transition_settings)
        .expect("BUG: parser must validate transitionSettings as object");
    super::build_request_response_success_no_data("SetCurrentSceneTransitionSettings", request_id)
}

pub fn build_set_scene_scene_transition_override_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "SetSceneSceneTransitionOverride",
        request_id,
        request_data,
        parse_set_scene_scene_transition_override_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match super::resolve_scene_name_or_error(
        "SetSceneSceneTransitionOverride",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let override_entry = match input_registry.set_scene_transition_override(
        &scene_name,
        fields.transition_name.as_deref(),
        fields.transition_duration,
    ) {
        Ok(override_entry) => override_entry,
        Err(SetSceneSceneTransitionOverrideError::SceneNotFound) => {
            return super::build_request_response_error(
                "SetSceneSceneTransitionOverride",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
            );
        }
        Err(SetSceneSceneTransitionOverrideError::TransitionNotFound) => {
            return super::build_request_response_error(
                "SetSceneSceneTransitionOverride",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Transition not found",
            );
        }
        Err(SetSceneSceneTransitionOverrideError::InvalidTransitionDuration) => {
            return super::build_request_response_error(
                "SetSceneSceneTransitionOverride",
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Invalid transitionDuration field",
            );
        }
    };

    // OBS は SetSceneSceneTransitionOverride で responseData を返さない
    let _ = override_entry;
    super::build_request_response_success_no_data("SetSceneSceneTransitionOverride", request_id)
}

pub fn build_get_current_scene_transition_cursor_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let transition_cursor = input_registry.current_tbar_position();
    super::build_request_response_success("GetCurrentSceneTransitionCursor", request_id, |f| {
        f.member("transitionCursor", transition_cursor)
    })
}

pub fn build_set_tbar_position_response(request_id: &str) -> nojson::RawJsonOwned {
    // hisui は Studio Mode をサポートしていないため、常に 506 を返す
    super::build_request_response_error(
        "SetTBarPosition",
        request_id,
        REQUEST_STATUS_STUDIO_MODE_NOT_ACTIVE,
        "Studio mode is not enabled",
    )
}

pub fn build_create_scene_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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
            return super::build_request_response_error(
                "CreateScene",
                request_id,
                REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
                "Scene already exists",
            );
        }
        Err(CreateSceneError::SceneIdOverflow) => {
            return super::build_request_response_error(
                "CreateScene",
                request_id,
                REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                "Scene ID overflow",
            );
        }
    };
    // OBS 互換で sceneUuid のみ返す
    super::build_request_response_success("CreateScene", request_id, |f| {
        f.member("sceneUuid", &created.scene_uuid)
    })
}

pub fn build_remove_scene_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "RemoveScene",
        request_id,
        request_data,
        parse_remove_scene_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match super::resolve_scene_name_or_error(
        "RemoveScene",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if let Err(error) = input_registry.remove_scene(&scene_name) {
        return match error {
            crate::obsws_input_registry::RemoveSceneError::SceneNotFound => {
                unreachable!("resolved scene name must exist in input registry")
            }
            crate::obsws_input_registry::RemoveSceneError::LastSceneNotRemovable => {
                super::build_request_response_error(
                    "RemoveScene",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "At least one scene must remain",
                )
            }
        };
    }

    super::build_request_response_success_no_data("RemoveScene", request_id)
}

pub fn build_set_scene_name_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "SetSceneName",
        request_id,
        request_data,
        parse_set_scene_name_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match super::resolve_scene_name_or_error(
        "SetSceneName",
        request_id,
        input_registry,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let renamed = match input_registry.set_scene_name(&scene_name, &fields.new_scene_name) {
        Ok(renamed) => renamed,
        Err(SetSceneNameError::SceneNotFound) => {
            unreachable!("resolved scene name must exist in input registry")
        }
        Err(SetSceneNameError::SceneNameAlreadyExists) => {
            return super::build_request_response_error(
                "SetSceneName",
                request_id,
                REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
                "Scene already exists",
            );
        }
    };

    // OBS は SetSceneName で responseData を返さない
    let _ = renamed;
    super::build_request_response_success_no_data("SetSceneName", request_id)
}

// Scene item handlers continue here.
