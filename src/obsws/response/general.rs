use crate::obsws_input_registry::ObswsInputRegistry;
use crate::obsws_message::ObswsSessionStats;
use crate::obsws_protocol::{
    OBSWS_OP_REQUEST_RESPONSE, OBSWS_RPC_VERSION, OBSWS_SUPPORTED_IMAGE_FORMATS, OBSWS_VERSION,
    REQUEST_STATUS_SUCCESS,
};
#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

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
                                "GetSourceActive",
                                "GetSceneList",
                                "CreateScene",
                                "SetSceneName",
                                "RemoveScene",
                                "GetCurrentProgramScene",
                                "SetCurrentProgramScene",
                                "GetCurrentPreviewScene",
                                "SetCurrentPreviewScene",
                                "GetSceneSceneTransitionOverride",
                                "SetSceneSceneTransitionOverride",
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
                                "GetOutputList",
                                "GetOutputStatus",
                                "ToggleOutput",
                                "StartOutput",
                                "StopOutput",
                                "GetOutputSettings",
                                "SetOutputSettings",
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

pub fn build_get_stats_response(
    request_id: &str,
    session_stats: &ObswsSessionStats,
    input_registry: &ObswsInputRegistry,
    pipeline_handle: Option<&crate::MediaPipelineHandle>,
) -> String {
    let outgoing_messages = session_stats.outgoing_messages.saturating_add(1);
    let runtime_stats = collect_runtime_stats(input_registry);
    let output_stats = super::collect_output_runtime_stats(input_registry, pipeline_handle);
    let active_fps = calculate_active_fps(input_registry, &output_stats);

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
                        f.member("memoryUsage", runtime_stats.memory_usage_mb)?;
                        f.member("availableDiskSpace", runtime_stats.available_disk_space_mb)?;
                        f.member("activeFps", active_fps)?;
                        f.member("averageFrameRenderTime", 0.0)?;
                        f.member("renderSkippedFrames", 0)?;
                        f.member("renderTotalFrames", 0)?;
                        f.member(
                            "outputSkippedFrames",
                            output_stats
                                .stream_skipped_frames
                                .saturating_add(output_stats.record_skipped_frames),
                        )?;
                        f.member(
                            "outputTotalFrames",
                            output_stats
                                .stream_total_frames
                                .saturating_add(output_stats.record_total_frames),
                        )?;
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

struct ObswsRuntimeStats {
    memory_usage_mb: f64,
    available_disk_space_mb: f64,
}

fn collect_runtime_stats(input_registry: &ObswsInputRegistry) -> ObswsRuntimeStats {
    ObswsRuntimeStats {
        memory_usage_mb: current_process_memory_usage_mb(),
        available_disk_space_mb: available_disk_space_mb(input_registry.record_directory()),
    }
}

fn calculate_active_fps(
    input_registry: &ObswsInputRegistry,
    output_stats: &super::ObswsOutputRuntimeStats,
) -> f64 {
    if input_registry.is_stream_active() {
        return frames_per_second(
            output_stats.stream_total_frames,
            input_registry.stream_uptime(),
        );
    }
    if input_registry.is_record_active() {
        return frames_per_second(
            output_stats.record_total_frames,
            input_registry.record_uptime(),
        );
    }
    0.0
}

fn frames_per_second(total_frames: u64, duration: std::time::Duration) -> f64 {
    let seconds = duration.as_secs_f64();
    if seconds <= 0.0 {
        return 0.0;
    }
    total_frames as f64 / seconds
}

#[cfg(unix)]
fn current_process_memory_usage_mb() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // 現在プロセスの最大 RSS を取得する。
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc != 0 {
        return 0.0;
    }
    let usage = unsafe { usage.assume_init() };
    #[cfg(target_os = "linux")]
    let rss_bytes = (usage.ru_maxrss as i128).saturating_mul(1024);
    #[cfg(not(target_os = "linux"))]
    let rss_bytes = usage.ru_maxrss as i128;
    if rss_bytes <= 0 {
        return 0.0;
    }
    rss_bytes as f64 / (1024.0 * 1024.0)
}

#[cfg(not(unix))]
fn current_process_memory_usage_mb() -> f64 {
    0.0
}

#[cfg(unix)]
fn available_disk_space_mb(path: &std::path::Path) -> f64 {
    let path_bytes = path.as_os_str().as_bytes();
    let Ok(path_cstr) = CString::new(path_bytes) else {
        return 0.0;
    };
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // 録画先ディレクトリが属するファイルシステムの空き容量を取得する。
    let rc = unsafe { libc::statfs(path_cstr.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return 0.0;
    }
    let stat = unsafe { stat.assume_init() };
    let block_size = stat.f_bsize as u128;
    let available_blocks = stat.f_bavail as u128;
    let available_bytes = available_blocks.saturating_mul(block_size);
    available_bytes as f64 / (1024.0 * 1024.0)
}

#[cfg(not(unix))]
fn available_disk_space_mb(_path: &std::path::Path) -> f64 {
    0.0
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
