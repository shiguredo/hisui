use crate::obsws::message::ObswsSessionStats;
use crate::obsws::protocol::{
    OBSWS_RPC_VERSION, OBSWS_SUPPORTED_IMAGE_FORMATS, OBSWS_VERSION,
    REQUEST_STATUS_INVALID_REQUEST_FIELD,
};
use crate::obsws::state::ObswsSessionState;
#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

use super::{parse_persistent_data_fields, parse_request_data_or_error_response};

/// 基本の availableRequests 一覧
const BASE_AVAILABLE_REQUESTS: &[&str] = &[
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
    "GetInputPropertiesListPropertyItems",
    "CreateInput",
    "RemoveInput",
    "GetPersistentData",
    "SetPersistentData",
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
    "GetMediaInputStatus",
    "SetMediaInputCursor",
    "OffsetMediaInputCursor",
    "TriggerMediaInputAction",
    // SoraSubscriber / sora_source
    "HisuiStartSoraSubscriber",
    "HisuiStopSoraSubscriber",
    "HisuiListSoraSubscribers",
    "HisuiListSoraSourceTracks",
    "HisuiAttachSoraSourceTrack",
    "HisuiDetachSoraSourceTrack",
    // Output 管理
    "HisuiCreateOutput",
    "HisuiRemoveOutput",
];

pub fn build_get_version_response(
    request_id: &str,
    extra_available_requests: &[&str],
) -> nojson::RawJsonOwned {
    super::build_request_response_success("GetVersion", request_id, |f| {
        // hisui は OBS ではないため、obsVersion には hisui 自身のバージョンを返す。
        f.member("obsVersion", env!("CARGO_PKG_VERSION"))?;
        f.member("obsWebSocketVersion", OBSWS_VERSION)?;
        f.member("rpcVersion", OBSWS_RPC_VERSION)?;
        f.member(
            "availableRequests",
            nojson::array(|f| {
                for request in BASE_AVAILABLE_REQUESTS {
                    f.element(request)?;
                }
                for request in extra_available_requests {
                    f.element(request)?;
                }
                Ok(())
            }),
        )?;
        f.member("supportedImageFormats", OBSWS_SUPPORTED_IMAGE_FORMATS)?;
        f.member("platform", std::env::consts::OS)?;
        f.member(
            "platformDescription",
            format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        )
    })
}

pub(crate) fn build_get_stats_response(
    request_id: &str,
    session_stats: &ObswsSessionStats,
    outputs: &std::collections::BTreeMap<
        String,
        crate::obsws::coordinator::output_registry::OutputState,
    >,
    pipeline_handle: Option<&crate::MediaPipelineHandle>,
) -> nojson::RawJsonOwned {
    let outgoing_messages = session_stats.outgoing_messages.saturating_add(1);
    let runtime_stats = collect_runtime_stats(outputs);
    let output_stats = super::collect_output_runtime_stats_from_outputs(outputs, pipeline_handle);
    let active_fps = calculate_active_fps_from_outputs(outputs, &output_stats);

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

fn collect_runtime_stats(
    outputs: &std::collections::BTreeMap<
        String,
        crate::obsws::coordinator::output_registry::OutputState,
    >,
) -> ObswsRuntimeStats {
    use crate::obsws::coordinator::output_registry::OutputSettings;
    // record output の record_directory からディスク容量を取得する
    let record_dir = outputs.get("record").and_then(|o| match &o.settings {
        OutputSettings::Record(s) => Some(s.record_directory.as_path()),
        _ => None,
    });
    let disk_space = record_dir.map(available_disk_space_mb).unwrap_or(0.0);
    ObswsRuntimeStats {
        memory_usage_mb: current_process_memory_usage_mb(),
        available_disk_space_mb: disk_space,
    }
}

fn calculate_active_fps_from_outputs(
    outputs: &std::collections::BTreeMap<
        String,
        crate::obsws::coordinator::output_registry::OutputState,
    >,
    output_stats: &super::ObswsOutputRuntimeStats,
) -> f64 {
    use crate::obsws::coordinator::output_registry::output_active_and_uptime;
    if let Some(stream) = outputs.get("stream") {
        let (active, uptime) = output_active_and_uptime(stream);
        if active {
            return frames_per_second(output_stats.stream_total_frames, uptime);
        }
    }
    if let Some(record) = outputs.get("record") {
        let (active, uptime) = output_active_and_uptime(record);
        if active {
            return frames_per_second(output_stats.record_total_frames, uptime);
        }
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
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let scenes = state.list_scenes();
    let current_program_scene = state.current_program_scene();
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

pub fn build_get_persistent_data_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &ObswsSessionState,
) -> nojson::RawJsonOwned {
    let fields = match parse_request_data_or_error_response(
        "GetPersistentData",
        request_id,
        request_data,
        parse_persistent_data_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    if let Err(response) = validate_realm("GetPersistentData", request_id, &fields.realm) {
        return response;
    }

    let slot_value = state.get_persistent_data(&fields.slot_name);
    super::build_request_response_success("GetPersistentData", request_id, |f| match slot_value {
        Some(value) => f.member("slotValue", value),
        None => f.member("slotValue", Option::<&str>::None),
    })
}

pub fn build_set_persistent_data_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    state: &mut ObswsSessionState,
) -> nojson::RawJsonOwned {
    let (fields, slot_value) = match parse_request_data_or_error_response(
        "SetPersistentData",
        request_id,
        request_data,
        parse_set_persistent_data_fields,
    ) {
        Ok(v) => v,
        Err(response) => return response,
    };
    if let Err(response) = validate_realm("SetPersistentData", request_id, &fields.realm) {
        return response;
    }

    state.set_persistent_data(fields.slot_name, slot_value);
    super::build_request_response_success_no_data("SetPersistentData", request_id)
}

/// SetPersistentData 用のフィールドをパースする。
/// slotValue は任意の JSON 値（null 以外）を受け付ける。
/// OBS 本家では slotValue が null の場合は MissingRequestField エラーを返す。
fn parse_set_persistent_data_fields(
    request_data: nojson::RawJsonValue<'_, '_>,
) -> Result<(super::PersistentDataFields, nojson::RawJsonOwned), nojson::JsonParseError> {
    let fields = parse_persistent_data_fields(request_data)?;
    let slot_raw = request_data.to_member("slotValue")?.required()?;
    if slot_raw.kind().is_null() {
        return Err(slot_raw.invalid("required member 'slotValue' is missing"));
    }
    let slot_value: nojson::RawJsonOwned = slot_raw.try_into()?;
    Ok((fields, slot_value))
}

/// realm の値を検証する。GLOBAL のみ対応。
fn validate_realm(
    request_type: &str,
    request_id: &str,
    realm: &str,
) -> Result<(), nojson::RawJsonOwned> {
    match realm {
        "OBS_WEBSOCKET_DATA_REALM_GLOBAL" => Ok(()),
        "OBS_WEBSOCKET_DATA_REALM_PROFILE" => Err(super::build_request_response_error(
            request_type,
            request_id,
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "Unsupported realm: only OBS_WEBSOCKET_DATA_REALM_GLOBAL is supported",
        )),
        _ => Err(super::build_request_response_error(
            request_type,
            request_id,
            REQUEST_STATUS_INVALID_REQUEST_FIELD,
            "Invalid realm value",
        )),
    }
}
