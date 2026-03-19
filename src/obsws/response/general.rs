use crate::obsws_input_registry::ObswsInputRegistry;
use crate::obsws_message::ObswsSessionStats;
use crate::obsws_protocol::{OBSWS_RPC_VERSION, OBSWS_SUPPORTED_IMAGE_FORMATS, OBSWS_VERSION};
#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

pub fn build_get_version_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success("GetVersion", request_id, |f| {
        // hisui は OBS ではないため、obsVersion には hisui 自身のバージョンを返す。
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
                "Sleep",
            ],
        )?;
        f.member("supportedImageFormats", OBSWS_SUPPORTED_IMAGE_FORMATS)?;
        f.member("platform", std::env::consts::OS)?;
        f.member(
            "platformDescription",
            format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        )
    })
}

pub fn build_get_stats_response(
    request_id: &str,
    session_stats: &ObswsSessionStats,
    input_registry: &ObswsInputRegistry,
    pipeline_handle: Option<&crate::MediaPipelineHandle>,
) -> nojson::RawJsonOwned {
    let outgoing_messages = session_stats.outgoing_messages.saturating_add(1);
    let runtime_stats = collect_runtime_stats(input_registry);
    let output_stats = super::collect_output_runtime_stats(input_registry, pipeline_handle);
    let active_fps = calculate_active_fps(input_registry, &output_stats);

    super::build_request_response_success("GetStats", request_id, |f| {
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
    })
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

pub fn build_get_canvas_list_response(
    request_id: &str,
    canvas_width: crate::types::EvenUsize,
    canvas_height: crate::types::EvenUsize,
    frame_rate: crate::video::FrameRate,
) -> nojson::RawJsonOwned {
    super::build_request_response_success("GetCanvasList", request_id, |f| {
        f.member(
            "canvases",
            [nojson::object(|f| {
                f.member("canvasName", "Main")?;
                f.member("canvasUuid", "00000000-0000-0000-0000-000000000001")?;
                // OBS 互換の object 形式で canvasFlags を返す。
                // hisui は単一の main canvas のみ持つため、MAIN / ACTIVATE / MIX_AUDIO を true とする。
                f.member(
                    "canvasFlags",
                    nojson::object(|f| {
                        f.member("MAIN", true)?;
                        f.member("ACTIVATE", true)?;
                        f.member("MIX_AUDIO", true)?;
                        f.member("SCENE_REF", false)?;
                        f.member("EPHEMERAL", false)
                    }),
                )?;
                f.member(
                    "canvasVideoSettings",
                    nojson::object(|f| {
                        f.member("baseWidth", canvas_width)?;
                        f.member("baseHeight", canvas_height)?;
                        f.member("outputWidth", canvas_width)?;
                        f.member("outputHeight", canvas_height)?;
                        f.member("fpsNumerator", frame_rate.numerator.get())?;
                        f.member("fpsDenominator", frame_rate.denumerator.get())
                    }),
                )
            })],
        )
    })
}

pub fn build_get_group_list_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success("GetGroupList", request_id, |f| {
        f.member("groups", nojson::array(|_| Ok(())))
    })
}

pub fn build_broadcast_custom_event_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success_no_data("BroadcastCustomEvent", request_id)
}

pub fn build_sleep_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success_no_data("Sleep", request_id)
}

/// hisui は単一 canvas 前提のため、OBS の canvasUuid によるシーン絞り込みには対応しない。
pub fn build_get_scene_list_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let scenes = input_registry.list_scenes();
    let current_program_scene = input_registry.current_program_scene();
    let current_program_scene_name = current_program_scene
        .as_ref()
        .map(|scene| scene.scene_name.as_str())
        .unwrap_or_default();
    let current_program_scene_uuid = current_program_scene
        .as_ref()
        .map(|scene| scene.scene_uuid.as_str())
        .unwrap_or_default();
    super::build_request_response_success("GetSceneList", request_id, |f| {
        f.member("currentProgramSceneName", current_program_scene_name)?;
        f.member("currentProgramSceneUuid", current_program_scene_uuid)?;
        f.member("currentPreviewSceneName", Option::<&str>::None)?;
        f.member("currentPreviewSceneUuid", Option::<&str>::None)?;
        f.member("scenes", &scenes)
    })
}
