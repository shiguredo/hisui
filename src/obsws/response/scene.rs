use crate::obsws_input_registry::{
    CreateSceneError, ObswsInputRegistry, SetCurrentPreviewSceneError, SetCurrentProgramSceneError,
    SetCurrentSceneTransitionDurationError, SetCurrentSceneTransitionError, SetTBarPositionError,
};
use crate::obsws_protocol::{
    OBSWS_OP_REQUEST_RESPONSE, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS, REQUEST_STATUS_RESOURCE_NOT_FOUND,
    REQUEST_STATUS_SUCCESS,
};

use super::{
    parse_create_scene_fields, parse_remove_scene_fields, parse_request_data_or_error_response,
    parse_set_current_preview_scene_fields, parse_set_current_program_scene_fields,
    parse_set_current_scene_transition_duration_fields, parse_set_current_scene_transition_fields,
    parse_set_current_scene_transition_settings_fields, parse_set_tbar_position_fields,
};

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
        return super::build_request_response_error(
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
        return super::build_request_response_error(
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
        return super::build_request_response_error(
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
        return super::build_request_response_error(
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
    input_registry
        .set_current_scene_transition_settings(fields.transition_settings)
        .expect("BUG: parser must validate transitionSettings as object");
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
        return super::build_request_response_error(
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
            return super::build_request_response_error(
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
            crate::obsws_input_registry::RemoveSceneError::SceneNotFound => {
                super::build_request_response_error(
                    "RemoveScene",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene not found",
                )
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

// Scene item handlers continue here.
