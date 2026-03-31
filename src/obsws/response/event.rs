use crate::obsws::input_registry::{
    ObswsInputSettings, ObswsSceneItemIndexEntry, ObswsSceneItemTransform,
};
use crate::obsws::protocol::{
    OBSWS_EVENT_SUB_GENERAL, OBSWS_EVENT_SUB_INPUTS, OBSWS_EVENT_SUB_OUTPUTS,
    OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED, OBSWS_EVENT_SUB_SCENE_ITEMS,
    OBSWS_EVENT_SUB_SCENES, OBSWS_OP_EVENT,
};

pub fn build_stream_state_changed_event(
    output_active: bool,
    output_state: &str,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "StreamStateChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_OUTPUTS)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("outputActive", output_active)?;
                        f.member("outputState", output_state)
                    }),
                )
            }),
        )
    })
}

pub fn build_record_state_changed_event(
    output_active: bool,
    output_state: &str,
    output_path: Option<&str>,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
                        f.member("outputState", output_state)?;
                        f.member("outputPath", output_path)
                    }),
                )
            }),
        )
    })
}

pub fn build_current_program_scene_changed_event(
    scene_name: &str,
    scene_uuid: &str,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
}

pub fn build_current_preview_scene_changed_event(
    scene_name: &str,
    scene_uuid: &str,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
}

pub fn build_scene_created_event(scene_name: &str, scene_uuid: &str) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
}

pub fn build_scene_removed_event(scene_name: &str, scene_uuid: &str) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
}

pub fn build_input_created_event(
    input_name: &str,
    input_uuid: &str,
    input_kind: &str,
    input_settings: &ObswsInputSettings,
    default_input_settings: &ObswsInputSettings,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
                        f.member("inputKind", input_kind)?;
                        f.member("unversionedInputKind", input_kind)?;
                        f.member("inputKindCaps", 0)?;
                        f.member("inputSettings", input_settings)?;
                        f.member("defaultInputSettings", default_input_settings)
                    }),
                )
            }),
        )
    })
}

pub fn build_input_removed_event(input_name: &str, input_uuid: &str) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
                        f.member("inputUuid", input_uuid)
                    }),
                )
            }),
        )
    })
}

pub fn build_input_settings_changed_event(
    input_name: &str,
    input_uuid: &str,
    input_settings: &ObswsInputSettings,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
                        f.member("inputSettings", input_settings)
                    }),
                )
            }),
        )
    })
}

pub fn build_input_name_changed_event(
    input_name: &str,
    old_input_name: &str,
    input_uuid: &str,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
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
}

pub fn build_input_mute_state_changed_event(
    input_name: &str,
    input_uuid: &str,
    input_muted: bool,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "InputMuteStateChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_INPUTS)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("inputName", input_name)?;
                        f.member("inputUuid", input_uuid)?;
                        f.member("inputMuted", input_muted)
                    }),
                )
            }),
        )
    })
}

pub fn build_input_volume_changed_event(
    input_name: &str,
    input_uuid: &str,
    input_volume_db: f64,
    input_volume_mul: f64,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "InputVolumeChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_INPUTS)?;
                f.member(
                    "eventData",
                    nojson::object(|f| {
                        f.member("inputName", input_name)?;
                        f.member("inputUuid", input_uuid)?;
                        f.member("inputVolumeDb", input_volume_db)?;
                        f.member("inputVolumeMul", input_volume_mul)
                    }),
                )
            }),
        )
    })
}

pub fn build_custom_event(event_data: &nojson::RawJsonOwned) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "CustomEvent")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_GENERAL)?;
                f.member("eventData", event_data)
            }),
        )
    })
}

pub fn build_scene_item_enable_state_changed_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_item_id: i64,
    scene_item_enabled: bool,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemEnableStateChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENE_ITEMS)?;
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
}

pub fn build_scene_item_lock_state_changed_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_item_id: i64,
    scene_item_locked: bool,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemLockStateChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENE_ITEMS)?;
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
}

pub fn build_scene_item_transform_changed_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_item_id: i64,
    scene_item_transform: &ObswsSceneItemTransform,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemTransformChanged")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENE_ITEM_TRANSFORM_CHANGED)?;
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
}

pub fn build_scene_item_created_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_item_id: i64,
    source_name: &str,
    source_uuid: &str,
    scene_item_index: i64,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemCreated")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENE_ITEMS)?;
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
}

pub fn build_scene_item_removed_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_item_id: i64,
    source_name: &str,
    source_uuid: &str,
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemRemoved")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENE_ITEMS)?;
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
}

pub fn build_scene_item_list_reindexed_event(
    scene_name: &str,
    scene_uuid: &str,
    scene_items: &[ObswsSceneItemIndexEntry],
) -> nojson::RawJsonOwned {
    nojson::RawJsonOwned::object(|f| {
        f.member("op", OBSWS_OP_EVENT)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("eventType", "SceneItemListReindexed")?;
                f.member("eventIntent", OBSWS_EVENT_SUB_SCENE_ITEMS)?;
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
}
