use crate::obsws::protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
    REQUEST_STATUS_RESOURCE_NOT_FOUND,
};
use crate::obsws::state::{
    CreateSceneItemError, DuplicateSceneItemError, GetSceneItemIdError, GetSceneItemListError,
    ObswsSessionState, SceneItemLookupError, SetSceneItemIndexError,
};

use super::{
    CreateSceneItemExecution, DuplicateSceneItemExecution, SetSceneItemIndexEventContext,
    SetSceneItemIndexExecution, SetSceneItemLockedEventContext, SetSceneItemLockedExecution,
    SetSceneItemTransformEventContext, SetSceneItemTransformExecution,
    parse_create_scene_item_fields, parse_duplicate_scene_item_fields,
    parse_get_scene_item_enabled_fields, parse_get_scene_item_id_fields,
    parse_get_scene_item_list_fields, parse_remove_scene_item_fields,
    parse_request_data_or_error_response, parse_scene_item_lookup_fields,
    parse_set_scene_item_blend_mode_fields, parse_set_scene_item_enabled_fields,
    parse_set_scene_item_index_fields, parse_set_scene_item_locked_fields,
    parse_set_scene_item_transform_fields, resolve_scene_name_or_error,
};

pub fn build_get_scene_item_id_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemId",
        request_id,
        request_data,
        parse_get_scene_item_id_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "GetSceneItemId",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };

    let scene_item_id = match state.get_scene_item_id(
        &scene_name,
        fields.source_name.as_deref(),
        fields.source_uuid.as_deref(),
        fields.search_offset,
    ) {
        Ok(scene_item_id) => scene_item_id,
        Err(GetSceneItemIdError::SceneNotFound) => {
            unreachable!("resolved scene name must exist in input registry")
        }
        Err(GetSceneItemIdError::SourceNotFound) => {
            return super::build_request_response_error(
                "GetSceneItemId",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Source not found in scene",
            );
        }
        Err(GetSceneItemIdError::SearchOffsetUnsupported) => {
            return super::build_request_response_error(
                "GetSceneItemId",
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Unsupported searchOffset field: only 0 is supported",
            );
        }
    };

    super::build_request_response_success("GetSceneItemId", request_id, |f| {
        f.member("sceneItemId", scene_item_id)
    })
}

pub fn build_get_scene_item_list_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemList",
        request_id,
        request_data,
        parse_get_scene_item_list_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "GetSceneItemList",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let scene_items = state
        .list_scene_items(&scene_name)
        .unwrap_or_else(|error| match error {
            GetSceneItemListError::SceneNotFound => {
                unreachable!("resolved scene name must exist in input registry")
            }
        });

    super::build_request_response_success("GetSceneItemList", request_id, |f| {
        f.member("sceneItems", &scene_items)
    })
}

pub fn execute_create_scene_item(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
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
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "CreateSceneItem",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => {
            return CreateSceneItemExecution {
                response_text: response,
                created: None,
            };
        }
    };
    let created = match state.create_scene_item(
        &scene_name,
        fields.source_uuid.as_deref(),
        fields.source_name.as_deref(),
        fields.scene_item_enabled,
    ) {
        Ok(created) => created,
        Err(CreateSceneItemError::SourceNotFound) => {
            return CreateSceneItemExecution {
                response_text: super::build_request_response_error(
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
        Err(CreateSceneItemError::SceneItemIdOverflow) => {
            return CreateSceneItemExecution {
                response_text: super::build_request_response_error(
                    "CreateSceneItem",
                    request_id,
                    REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Scene item ID overflow",
                ),
                created: None,
            };
        }
    };

    let response_text = super::build_request_response_success("CreateSceneItem", request_id, |f| {
        f.member("sceneItemId", created.scene_item.scene_item_id)
    });
    CreateSceneItemExecution {
        response_text,
        created: Some(created),
    }
}

pub fn build_remove_scene_item_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "RemoveSceneItem",
        request_id,
        request_data,
        parse_remove_scene_item_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "RemoveSceneItem",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if let Err(error) = state.remove_scene_item(&scene_name, fields.scene_item_id) {
        return match error {
            SceneItemLookupError::SceneNotFound => {
                unreachable!("resolved scene name must exist in input registry")
            }
            SceneItemLookupError::SceneItemNotFound => super::build_request_response_error(
                "RemoveSceneItem",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene item not found",
            ),
        };
    }

    super::build_request_response_success_no_data("RemoveSceneItem", request_id)
}

pub fn execute_duplicate_scene_item(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
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
    let (from_scene_name, _from_scene_uuid) = match resolve_scene_name_or_error(
        "DuplicateSceneItem",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => {
            return DuplicateSceneItemExecution {
                response_text: response,
                duplicated: None,
            };
        }
    };
    let (to_scene_name, _to_scene_uuid) = match resolve_scene_name_or_error(
        "DuplicateSceneItem",
        request_id,
        state,
        fields.destination_scene_name.as_deref(),
        fields.destination_scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => {
            return DuplicateSceneItemExecution {
                response_text: response,
                duplicated: None,
            };
        }
    };
    let duplicated =
        match state.duplicate_scene_item(&from_scene_name, &to_scene_name, fields.scene_item_id) {
            Ok(duplicated) => duplicated,
            Err(DuplicateSceneItemError::SourceScene) => {
                unreachable!("resolved source scene name must exist in input registry")
            }
            Err(DuplicateSceneItemError::DestinationScene) => {
                unreachable!("resolved destination scene name must exist in input registry")
            }
            Err(DuplicateSceneItemError::SourceSceneItem) => {
                return DuplicateSceneItemExecution {
                    response_text: super::build_request_response_error(
                        "DuplicateSceneItem",
                        request_id,
                        REQUEST_STATUS_RESOURCE_NOT_FOUND,
                        "Scene item not found",
                    ),
                    duplicated: None,
                };
            }
            Err(DuplicateSceneItemError::SceneItemIdOverflow) => {
                return DuplicateSceneItemExecution {
                    response_text: super::build_request_response_error(
                        "DuplicateSceneItem",
                        request_id,
                        REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Scene item ID overflow",
                    ),
                    duplicated: None,
                };
            }
        };

    let response_text =
        super::build_request_response_success("DuplicateSceneItem", request_id, |f| {
            f.member("sceneItemId", duplicated.scene_item.scene_item_id)
        });
    DuplicateSceneItemExecution {
        response_text,
        duplicated: Some(duplicated),
    }
}

pub fn build_get_scene_item_source_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemSource",
        request_id,
        request_data,
        parse_scene_item_lookup_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "GetSceneItemSource",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let (source_name, source_uuid) =
        match state.get_scene_item_source(&scene_name, fields.scene_item_id) {
            Ok(source) => source,
            Err(SceneItemLookupError::SceneItemNotFound) => {
                return super::build_request_response_error(
                    "GetSceneItemSource",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                );
            }
            Err(SceneItemLookupError::SceneNotFound) => {
                unreachable!("resolved scene name must exist in input registry")
            }
        };

    super::build_request_response_success("GetSceneItemSource", request_id, |f| {
        f.member("sourceName", &source_name)?;
        f.member("sourceUuid", &source_uuid)
    })
}

pub fn build_get_scene_item_index_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemIndex",
        request_id,
        request_data,
        parse_scene_item_lookup_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "GetSceneItemIndex",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let scene_item_index = match state.get_scene_item_index(&scene_name, fields.scene_item_id) {
        Ok(scene_item_index) => scene_item_index,
        Err(SceneItemLookupError::SceneItemNotFound) => {
            return super::build_request_response_error(
                "GetSceneItemIndex",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene item not found",
            );
        }
        Err(SceneItemLookupError::SceneNotFound) => {
            unreachable!("resolved scene name must exist in input registry")
        }
    };

    super::build_request_response_success("GetSceneItemIndex", request_id, |f| {
        f.member("sceneItemIndex", scene_item_index)
    })
}

pub fn execute_set_scene_item_index(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
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
                event_context: None,
            };
        }
    };
    let (scene_name, scene_uuid) = match resolve_scene_name_or_error(
        "SetSceneItemIndex",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => {
            return SetSceneItemIndexExecution {
                response_text: response,
                event_context: None,
            };
        }
    };
    let set_result = match state.set_scene_item_index(
        &scene_name,
        fields.scene_item_id,
        fields.scene_item_index,
    ) {
        Ok(set_result) => set_result,
        Err(error) => {
            let response_text = match error {
                SetSceneItemIndexError::SceneItemNotFound => super::build_request_response_error(
                    "SetSceneItemIndex",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                ),
                SetSceneItemIndexError::InvalidSceneItemIndex => {
                    super::build_request_response_error(
                        "SetSceneItemIndex",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        "Invalid sceneItemIndex field",
                    )
                }
                SetSceneItemIndexError::SceneNotFound => {
                    unreachable!("resolved scene name must exist in input registry")
                }
            };
            return SetSceneItemIndexExecution {
                response_text,
                event_context: None,
            };
        }
    };

    let response_text =
        super::build_request_response_success_no_data("SetSceneItemIndex", request_id);
    SetSceneItemIndexExecution {
        response_text,
        event_context: Some(SetSceneItemIndexEventContext {
            scene_name,
            scene_uuid,
            set_result,
        }),
    }
}

pub fn build_set_scene_item_enabled_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "SetSceneItemEnabled",
        request_id,
        request_data,
        parse_set_scene_item_enabled_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "SetSceneItemEnabled",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };

    if let Err(error) =
        state.set_scene_item_enabled(&scene_name, fields.scene_item_id, fields.scene_item_enabled)
    {
        return match error {
            SceneItemLookupError::SceneNotFound => {
                unreachable!("resolved scene name must exist in input registry")
            }
            SceneItemLookupError::SceneItemNotFound => super::build_request_response_error(
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
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemEnabled",
        request_id,
        request_data,
        parse_get_scene_item_enabled_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "GetSceneItemEnabled",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };

    let scene_item_enabled = match state.get_scene_item_enabled(&scene_name, fields.scene_item_id) {
        Ok(scene_item_enabled) => scene_item_enabled,
        Err(SceneItemLookupError::SceneNotFound) => {
            unreachable!("resolved scene name must exist in input registry")
        }
        Err(SceneItemLookupError::SceneItemNotFound) => {
            return super::build_request_response_error(
                "GetSceneItemEnabled",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene item not found",
            );
        }
    };

    super::build_request_response_success("GetSceneItemEnabled", request_id, |f| {
        f.member("sceneItemEnabled", scene_item_enabled)
    })
}

pub fn build_set_scene_item_enabled_success_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success_no_data("SetSceneItemEnabled", request_id)
}

pub fn build_get_scene_item_locked_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemLocked",
        request_id,
        request_data,
        parse_scene_item_lookup_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "GetSceneItemLocked",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let scene_item_locked = match state.get_scene_item_locked(&scene_name, fields.scene_item_id) {
        Ok(scene_item_locked) => scene_item_locked,
        Err(SceneItemLookupError::SceneNotFound) => {
            unreachable!("resolved scene name must exist in input registry")
        }
        Err(SceneItemLookupError::SceneItemNotFound) => {
            return super::build_request_response_error(
                "GetSceneItemLocked",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene item not found",
            );
        }
    };

    super::build_request_response_success("GetSceneItemLocked", request_id, |f| {
        f.member("sceneItemLocked", scene_item_locked)
    })
}

pub fn execute_set_scene_item_locked(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
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
                event_context: None,
            };
        }
    };
    let (scene_name, scene_uuid) = match resolve_scene_name_or_error(
        "SetSceneItemLocked",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => {
            return SetSceneItemLockedExecution {
                response_text: response,
                event_context: None,
            };
        }
    };
    let set_result = match state.set_scene_item_locked(
        &scene_name,
        fields.scene_item_id,
        fields.scene_item_locked,
    ) {
        Ok(set_result) => set_result,
        Err(SceneItemLookupError::SceneNotFound) => {
            unreachable!("resolved scene name must exist in input registry")
        }
        Err(SceneItemLookupError::SceneItemNotFound) => {
            return SetSceneItemLockedExecution {
                response_text: super::build_request_response_error(
                    "SetSceneItemLocked",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                ),
                event_context: None,
            };
        }
    };

    let response_text =
        super::build_request_response_success_no_data("SetSceneItemLocked", request_id);

    SetSceneItemLockedExecution {
        response_text,
        event_context: Some(SetSceneItemLockedEventContext {
            scene_name,
            scene_uuid,
            scene_item_id: fields.scene_item_id,
            scene_item_locked: fields.scene_item_locked,
            set_result,
        }),
    }
}

pub fn build_get_scene_item_blend_mode_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemBlendMode",
        request_id,
        request_data,
        parse_scene_item_lookup_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "GetSceneItemBlendMode",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let scene_item_blend_mode =
        match state.get_scene_item_blend_mode(&scene_name, fields.scene_item_id) {
            Ok(scene_item_blend_mode) => scene_item_blend_mode,
            Err(SceneItemLookupError::SceneNotFound) => {
                unreachable!("resolved scene name must exist in input registry")
            }
            Err(SceneItemLookupError::SceneItemNotFound) => {
                return super::build_request_response_error(
                    "GetSceneItemBlendMode",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                );
            }
        };

    super::build_request_response_success("GetSceneItemBlendMode", request_id, |f| {
        f.member("sceneItemBlendMode", scene_item_blend_mode.as_str())
    })
}

pub fn build_set_scene_item_blend_mode_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "SetSceneItemBlendMode",
        request_id,
        request_data,
        parse_set_scene_item_blend_mode_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "SetSceneItemBlendMode",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if let Err(error) = state.set_scene_item_blend_mode(
        &scene_name,
        fields.scene_item_id,
        fields.scene_item_blend_mode,
    ) {
        return match error {
            SceneItemLookupError::SceneNotFound => {
                unreachable!("resolved scene name must exist in input registry")
            }
            SceneItemLookupError::SceneItemNotFound => super::build_request_response_error(
                "SetSceneItemBlendMode",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene item not found",
            ),
        };
    }
    super::build_request_response_success_no_data("SetSceneItemBlendMode", request_id)
}

pub fn build_get_scene_item_transform_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetSceneItemTransform",
        request_id,
        request_data,
        parse_scene_item_lookup_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let (scene_name, _scene_uuid) = match resolve_scene_name_or_error(
        "GetSceneItemTransform",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    let scene_item_transform =
        match state.get_scene_item_transform(&scene_name, fields.scene_item_id) {
            Ok(scene_item_transform) => scene_item_transform,
            Err(SceneItemLookupError::SceneNotFound) => {
                unreachable!("resolved scene name must exist in input registry")
            }
            Err(SceneItemLookupError::SceneItemNotFound) => {
                return super::build_request_response_error(
                    "GetSceneItemTransform",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                );
            }
        };

    super::build_request_response_success("GetSceneItemTransform", request_id, |f| {
        f.member("sceneItemTransform", &scene_item_transform)
    })
}

pub fn execute_set_scene_item_transform(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
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
                event_context: None,
            };
        }
    };
    let (scene_name, scene_uuid) = match resolve_scene_name_or_error(
        "SetSceneItemTransform",
        request_id,
        state,
        fields.scene_name.as_deref(),
        fields.scene_uuid.as_deref(),
    ) {
        Ok(v) => v,
        Err(response) => {
            return SetSceneItemTransformExecution {
                response_text: response,
                event_context: None,
            };
        }
    };
    let set_result = match state.set_scene_item_transform(
        &scene_name,
        fields.scene_item_id,
        fields.scene_item_transform,
    ) {
        Ok(set_result) => set_result,
        Err(SceneItemLookupError::SceneNotFound) => {
            unreachable!("resolved scene name must exist in input registry")
        }
        Err(SceneItemLookupError::SceneItemNotFound) => {
            return SetSceneItemTransformExecution {
                response_text: super::build_request_response_error(
                    "SetSceneItemTransform",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                ),
                event_context: None,
            };
        }
    };

    let response_text =
        super::build_request_response_success_no_data("SetSceneItemTransform", request_id);

    SetSceneItemTransformExecution {
        response_text,
        event_context: Some(SetSceneItemTransformEventContext {
            scene_name,
            scene_uuid,
            scene_item_id: fields.scene_item_id,
            set_result,
        }),
    }
}
