use std::path::PathBuf;

use crate::obsws_input_registry::{ObswsInputRegistry, ObswsStreamServiceSettings};
use crate::obsws_protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_RESOURCE_NOT_FOUND,
};

use super::{
    parse_get_output_settings_fields, parse_get_output_status_fields,
    parse_request_data_or_error_response, parse_set_output_settings_fields,
    parse_set_record_directory_fields, parse_set_stream_service_settings_fields,
};

const OBSWS_STREAM_OUTPUT_NAME: &str = "stream";
const OBSWS_RECORD_OUTPUT_NAME: &str = "record";
const OBSWS_STREAM_OUTPUT_KIND: &str = "rtmp_output";
const OBSWS_RECORD_OUTPUT_KIND: &str = "mp4_output";

#[derive(Debug, Clone, Copy)]
struct ObswsOutputEntry {
    output_name: &'static str,
    output_kind: &'static str,
}

impl nojson::DisplayJson for ObswsOutputEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("outputName", self.output_name)?;
            f.member("outputKind", self.output_kind)
        })
        .fmt(f)
    }
}

pub fn build_get_stream_service_settings_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let settings = input_registry.stream_service_settings();
    super::build_request_response_success("GetStreamServiceSettings", request_id, |f| {
        f.member("streamServiceType", &settings.stream_service_type)?;
        f.member(
            "streamServiceSettings",
            nojson::object(|f| {
                if let Some(server) = &settings.server {
                    f.member("server", server)?;
                }
                if let Some(key) = &settings.key {
                    f.member("key", key)?;
                }
                Ok(())
            }),
        )
    })
}

pub fn build_set_stream_service_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetStreamServiceSettings",
        request_id,
        request_data,
        parse_set_stream_service_settings_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    input_registry.set_stream_service_settings(ObswsStreamServiceSettings {
        stream_service_type: fields.stream_service_type,
        server: Some(fields.server),
        key: fields.key,
    });
    super::build_request_response_success_no_data("SetStreamServiceSettings", request_id)
}

pub fn build_get_stream_status_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
    pipeline_handle: Option<&crate::MediaPipelineHandle>,
) -> String {
    let active = input_registry.is_stream_active();
    let duration = if active {
        input_registry.stream_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    let output_stats = super::collect_output_runtime_stats(input_registry, pipeline_handle);
    super::build_request_response_success("GetStreamStatus", request_id, |f| {
        f.member("outputActive", active)?;
        f.member("outputReconnecting", false)?;
        f.member("outputTimecode", &output_timecode)?;
        f.member("outputDuration", output_duration)?;
        f.member("outputCongestion", 0.0)?;
        f.member("outputBytes", output_stats.stream_output_bytes)?;
        f.member("outputSkippedFrames", output_stats.stream_skipped_frames)?;
        f.member("outputTotalFrames", output_stats.stream_total_frames)
    })
}

pub fn build_get_output_list_response(request_id: &str) -> String {
    let outputs = [
        ObswsOutputEntry {
            output_name: OBSWS_STREAM_OUTPUT_NAME,
            output_kind: OBSWS_STREAM_OUTPUT_KIND,
        },
        ObswsOutputEntry {
            output_name: OBSWS_RECORD_OUTPUT_NAME,
            output_kind: OBSWS_RECORD_OUTPUT_KIND,
        },
    ];
    super::build_request_response_success("GetOutputList", request_id, |f| {
        f.member("outputs", outputs)
    })
}

pub fn build_get_output_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetOutputSettings",
        request_id,
        request_data,
        parse_get_output_settings_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };

    match fields.output_name.as_str() {
        OBSWS_STREAM_OUTPUT_NAME => {
            build_stream_output_settings_response(request_id, input_registry)
        }
        OBSWS_RECORD_OUTPUT_NAME => {
            build_record_output_settings_response(request_id, input_registry)
        }
        _ => super::build_request_response_error(
            "GetOutputSettings",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Output not found",
        ),
    }
}

pub fn build_set_output_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetOutputSettings",
        request_id,
        request_data,
        parse_set_output_settings_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };

    match fields.output_name.as_str() {
        OBSWS_STREAM_OUTPUT_NAME => {
            let settings = match super::parse_set_stream_service_settings_fields(
                fields.output_settings.value(),
            ) {
                Ok(settings) => settings,
                Err(error) => {
                    return super::build_request_response_error(
                        "SetOutputSettings",
                        request_id,
                        super::request_status_code_for_parse_error(&error),
                        &error.to_string(),
                    );
                }
            };
            input_registry.set_stream_service_settings(ObswsStreamServiceSettings {
                stream_service_type: settings.stream_service_type,
                server: Some(settings.server),
                key: settings.key,
            });
            super::build_request_response_success_no_data("SetOutputSettings", request_id)
        }
        OBSWS_RECORD_OUTPUT_NAME => {
            let settings =
                match super::parse_set_record_directory_fields(fields.output_settings.value()) {
                    Ok(settings) => settings,
                    Err(error) => {
                        return super::build_request_response_error(
                            "SetOutputSettings",
                            request_id,
                            super::request_status_code_for_parse_error(&error),
                            &error.to_string(),
                        );
                    }
                };
            let record_directory = match resolve_record_directory_path(&settings.record_directory) {
                Ok(path) => path,
                Err(e) => {
                    return super::build_request_response_error(
                        "SetOutputSettings",
                        request_id,
                        REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        &e,
                    );
                }
            };
            input_registry.set_record_directory(record_directory);
            super::build_request_response_success_no_data("SetOutputSettings", request_id)
        }
        _ => super::build_request_response_error(
            "SetOutputSettings",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Output not found",
        ),
    }
}

pub fn build_get_record_directory_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let record_directory = input_registry.record_directory().display().to_string();
    super::build_request_response_success("GetRecordDirectory", request_id, |f| {
        f.member("recordDirectory", &record_directory)
    })
}

pub fn build_set_record_directory_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "SetRecordDirectory",
        request_id,
        request_data,
        parse_set_record_directory_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };
    let record_directory = match resolve_record_directory_path(&fields.record_directory) {
        Ok(path) => path,
        Err(e) => {
            return super::build_request_response_error(
                "SetRecordDirectory",
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                &e,
            );
        }
    };
    input_registry.set_record_directory(record_directory);
    super::build_request_response_success_no_data("SetRecordDirectory", request_id)
}

pub fn build_get_record_status_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
    pipeline_handle: Option<&crate::MediaPipelineHandle>,
) -> String {
    let active = input_registry.is_record_active();
    let paused = input_registry.is_record_paused();
    let duration = if active {
        input_registry.record_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    let output_path = input_registry
        .record_output_path()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let output_bytes = input_registry
        .record_output_path()
        .map(read_file_size_bytes)
        .unwrap_or(0);
    let output_stats = super::collect_output_runtime_stats(input_registry, pipeline_handle);
    super::build_request_response_success("GetRecordStatus", request_id, |f| {
        f.member("outputActive", active)?;
        f.member("outputPaused", paused)?;
        f.member("outputTimecode", &output_timecode)?;
        f.member("outputDuration", output_duration)?;
        f.member("outputCongestion", 0.0)?;
        f.member("outputBytes", output_bytes)?;
        f.member("outputSkippedFrames", output_stats.record_skipped_frames)?;
        f.member("outputTotalFrames", output_stats.record_total_frames)?;
        f.member("outputPath", &output_path)
    })
}

pub fn build_get_output_status_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
    pipeline_handle: Option<&crate::MediaPipelineHandle>,
) -> String {
    let fields = match parse_request_data_or_error_response(
        "GetOutputStatus",
        request_id,
        request_data,
        parse_get_output_status_fields,
    ) {
        Ok(fields) => fields,
        Err(response) => return response,
    };

    match fields.output_name.as_str() {
        OBSWS_STREAM_OUTPUT_NAME => {
            build_get_stream_status_as_output_response(request_id, input_registry, pipeline_handle)
        }
        OBSWS_RECORD_OUTPUT_NAME => {
            build_get_record_status_as_output_response(request_id, input_registry, pipeline_handle)
        }
        _ => super::build_request_response_error(
            "GetOutputStatus",
            request_id,
            REQUEST_STATUS_RESOURCE_NOT_FOUND,
            "Output not found",
        ),
    }
}

fn build_output_active_response(
    request_type: &str,
    request_id: &str,
    output_active: bool,
) -> String {
    super::build_request_response_success(request_type, request_id, |f| {
        f.member("outputActive", output_active)
    })
}

fn build_record_output_state_response(
    request_type: &str,
    request_id: &str,
    output_active: bool,
    output_paused: bool,
) -> String {
    super::build_request_response_success(request_type, request_id, |f| {
        f.member("outputActive", output_active)?;
        f.member("outputPaused", output_paused)
    })
}

fn build_stream_output_settings_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let settings = input_registry.stream_service_settings();
    super::build_request_response_success("GetOutputSettings", request_id, |f| {
        f.member("outputName", OBSWS_STREAM_OUTPUT_NAME)?;
        f.member("outputKind", OBSWS_STREAM_OUTPUT_KIND)?;
        f.member("outputSettings", settings)
    })
}

fn build_record_output_settings_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let record_directory = input_registry.record_directory().display().to_string();
    super::build_request_response_success("GetOutputSettings", request_id, |f| {
        f.member("outputName", OBSWS_RECORD_OUTPUT_NAME)?;
        f.member("outputKind", OBSWS_RECORD_OUTPUT_KIND)?;
        f.member(
            "outputSettings",
            nojson::object(|f| f.member("recordDirectory", &record_directory)),
        )
    })
}

pub fn build_start_stream_response(request_id: &str, output_active: bool) -> String {
    build_output_active_response("StartStream", request_id, output_active)
}

pub fn build_start_output_response(request_id: &str, output_active: bool) -> String {
    build_output_active_response("StartOutput", request_id, output_active)
}

pub fn build_toggle_stream_response(request_id: &str, output_active: bool) -> String {
    build_output_active_response("ToggleStream", request_id, output_active)
}

pub fn build_toggle_output_response(request_id: &str, output_active: bool) -> String {
    build_output_active_response("ToggleOutput", request_id, output_active)
}

pub fn build_stop_stream_response(request_id: &str) -> String {
    super::build_request_response_success_no_data("StopStream", request_id)
}

pub fn build_stop_output_response(request_id: &str) -> String {
    super::build_request_response_success_no_data("StopOutput", request_id)
}

pub fn build_toggle_record_response(request_id: &str, output_active: bool) -> String {
    build_record_output_state_response("ToggleRecord", request_id, output_active, false)
}

pub fn build_start_record_response(request_id: &str, output_active: bool) -> String {
    build_record_output_state_response("StartRecord", request_id, output_active, false)
}

pub fn build_toggle_record_pause_response(request_id: &str, output_paused: bool) -> String {
    build_record_output_state_response("ToggleRecordPause", request_id, true, output_paused)
}

pub fn build_pause_record_response(request_id: &str) -> String {
    build_record_output_state_response("PauseRecord", request_id, true, true)
}

pub fn build_resume_record_response(request_id: &str) -> String {
    build_record_output_state_response("ResumeRecord", request_id, true, false)
}

pub fn build_stop_record_response(request_id: &str, output_path: &str) -> String {
    super::build_request_response_success("StopRecord", request_id, |f| {
        f.member("outputPath", output_path)
    })
}

fn format_timecode(duration: std::time::Duration) -> String {
    let total_millis = duration.as_millis();
    let millis = total_millis % 1_000;
    let total_secs = total_millis / 1_000;
    let secs = total_secs % 60;
    let total_minutes = total_secs / 60;
    let minutes = total_minutes % 60;
    let hours = total_minutes / 60;
    format!("{hours:02}:{minutes:02}:{secs:02}.{millis:03}")
}

fn build_get_stream_status_as_output_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
    pipeline_handle: Option<&crate::MediaPipelineHandle>,
) -> String {
    let active = input_registry.is_stream_active();
    let duration = if active {
        input_registry.stream_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    let output_stats = super::collect_output_runtime_stats(input_registry, pipeline_handle);
    super::build_request_response_success("GetOutputStatus", request_id, |f| {
        f.member("outputActive", active)?;
        f.member("outputReconnecting", false)?;
        f.member("outputTimecode", &output_timecode)?;
        f.member("outputDuration", output_duration)?;
        f.member("outputCongestion", 0.0)?;
        f.member("outputBytes", output_stats.stream_output_bytes)?;
        f.member("outputSkippedFrames", output_stats.stream_skipped_frames)?;
        f.member("outputTotalFrames", output_stats.stream_total_frames)
    })
}

fn build_get_record_status_as_output_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
    pipeline_handle: Option<&crate::MediaPipelineHandle>,
) -> String {
    let active = input_registry.is_record_active();
    let paused = input_registry.is_record_paused();
    let duration = if active {
        input_registry.record_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    let output_path = input_registry
        .record_output_path()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let output_bytes = input_registry
        .record_output_path()
        .map(read_file_size_bytes)
        .unwrap_or(0);
    let output_stats = super::collect_output_runtime_stats(input_registry, pipeline_handle);
    super::build_request_response_success("GetOutputStatus", request_id, |f| {
        f.member("outputActive", active)?;
        f.member("outputPaused", paused)?;
        f.member("outputTimecode", &output_timecode)?;
        f.member("outputDuration", output_duration)?;
        f.member("outputCongestion", 0.0)?;
        f.member("outputBytes", output_bytes)?;
        f.member("outputSkippedFrames", output_stats.record_skipped_frames)?;
        f.member("outputTotalFrames", output_stats.record_total_frames)?;
        f.member("outputPath", &output_path)
    })
}

fn resolve_record_directory_path(record_directory: &str) -> Result<PathBuf, String> {
    std::path::absolute(record_directory)
        .map_err(|e| format!("Failed to resolve absolute record directory path: {e}"))
}

fn read_file_size_bytes(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}
