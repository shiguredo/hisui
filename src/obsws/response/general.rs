use crate::obsws_input_registry::ObswsInputRegistry;
use crate::obsws_message::ObswsSessionStats;
use crate::obsws_protocol::{
    OBSWS_OP_REQUEST_RESPONSE, OBSWS_RPC_VERSION, OBSWS_SUPPORTED_IMAGE_FORMATS, OBSWS_VERSION,
    REQUEST_STATUS_SUCCESS,
};

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
                                "BroadcastCustomEvent",
                                "GetGroupList",
                                "GetCanvasList",
                                "GetSceneList",
                                "CreateScene",
                                "SetSceneName",
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
                                "Sleep",
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

pub fn build_get_group_list_response(request_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetGroupList")?;
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
                    nojson::object(|f| f.member("groups", nojson::array(|_| Ok(())))),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_broadcast_custom_event_response(request_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "BroadcastCustomEvent")?;
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

pub fn build_sleep_response(request_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "Sleep")?;
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
