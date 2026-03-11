use crate::obsws_input_registry::{
    CreateSceneItemError, DuplicateSceneItemError, GetSceneItemBlendModeError,
    GetSceneItemEnabledError, GetSceneItemIdError, GetSceneItemIndexError, GetSceneItemListError,
    GetSceneItemLockedError, GetSceneItemSourceError, GetSceneItemTransformError,
    ObswsInputRegistry, SetSceneItemBlendModeError, SetSceneItemEnabledError,
    SetSceneItemIndexError, SetSceneItemLockedError, SetSceneItemTransformError,
};
use crate::obsws_protocol::{
    OBSWS_OP_REQUEST_RESPONSE, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_RESOURCE_NOT_FOUND, REQUEST_STATUS_SUCCESS,
};

use super::{
    CreateSceneItemExecution, DuplicateSceneItemExecution, SetSceneItemIndexExecution,
    SetSceneItemLockedExecution, SetSceneItemTransformExecution, parse_create_scene_item_fields,
    parse_duplicate_scene_item_fields, parse_get_scene_item_blend_mode_fields,
    parse_get_scene_item_enabled_fields, parse_get_scene_item_id_fields,
    parse_get_scene_item_index_fields, parse_get_scene_item_list_fields,
    parse_get_scene_item_locked_fields, parse_get_scene_item_source_fields,
    parse_get_scene_item_transform_fields, parse_remove_scene_item_fields,
    parse_request_data_or_error_response, parse_set_scene_item_blend_mode_fields,
    parse_set_scene_item_enabled_fields, parse_set_scene_item_index_fields,
    parse_set_scene_item_locked_fields, parse_set_scene_item_transform_fields,
    resolve_scene_name_or_error,
};

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
            return super::build_request_response_error(
                "GetSceneItemId",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
            );
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
            crate::obsws_input_registry::RemoveSceneItemError::SceneNotFound => {
                unreachable!("resolved scene name must exist in input registry")
            }
            crate::obsws_input_registry::RemoveSceneItemError::SceneItemNotFound => {
                super::build_request_response_error(
                    "RemoveSceneItem",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene item not found",
                )
            }
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
                response_text: super::build_request_response_error(
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
                return super::build_request_response_error(
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
                return super::build_request_response_error(
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
            SetSceneItemEnabledError::SceneNotFound => super::build_request_response_error(
                "SetSceneItemEnabled",
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Scene not found",
            ),
            SetSceneItemEnabledError::SceneItemNotFound => super::build_request_response_error(
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
                return super::build_request_response_error(
                    "GetSceneItemEnabled",
                    request_id,
                    REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Scene not found",
                );
            }
            Err(GetSceneItemEnabledError::SceneItemNotFound) => {
                return super::build_request_response_error(
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
                return super::build_request_response_error(
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
                response_text: super::build_request_response_error(
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
                return super::build_request_response_error(
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
            SetSceneItemBlendModeError::SceneItemNotFound => super::build_request_response_error(
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
                return super::build_request_response_error(
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
                response_text: super::build_request_response_error(
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
