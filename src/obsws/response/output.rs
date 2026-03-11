use std::path::PathBuf;

use crate::obsws_input_registry::{ObswsInputRegistry, ObswsStreamServiceSettings};
use crate::obsws_protocol::{
    OBSWS_OP_REQUEST_RESPONSE, REQUEST_STATUS_INVALID_REQUEST_FIELD,
    REQUEST_STATUS_RESOURCE_NOT_FOUND, REQUEST_STATUS_SUCCESS,
};

use super::{
    parse_get_output_status_fields, parse_request_data_or_error_response,
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
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetStreamServiceSettings")?;
                f.member("requestId", request_id)?;
                f.member(
                    "requestStatus",
                    nojson::object(|f| {
                        f.member("result", true)?;
                        f.member("code", REQUEST_STATUS_SUCCESS)
                    }),
                )?;
                f.member("responseData", settings)
            }),
        )
    })
    .to_string()
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
    empty_success_response("SetStreamServiceSettings", request_id)
}

pub fn build_get_stream_status_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let active = input_registry.is_stream_active();
    let duration = if active {
        input_registry.stream_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetStreamStatus")?;
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
                        f.member("outputActive", active)?;
                        f.member("outputReconnecting", false)?;
                        f.member("outputTimecode", &output_timecode)?;
                        f.member("outputDuration", output_duration)?;
                        f.member("outputCongestion", 0.0)?;
                        f.member("outputBytes", 0)?;
                        f.member("outputSkippedFrames", 0)?;
                        f.member("outputTotalFrames", 0)
                    }),
                )
            }),
        )
    })
    .to_string()
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
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetOutputList")?;
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
                    nojson::object(|f| f.member("outputs", outputs)),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_get_record_directory_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> String {
    let record_directory = input_registry.record_directory().display().to_string();
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetRecordDirectory")?;
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
                    nojson::object(|f| f.member("recordDirectory", &record_directory)),
                )
            }),
        )
    })
    .to_string()
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
    empty_success_response("SetRecordDirectory", request_id)
}

pub fn build_get_record_status_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
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
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetRecordStatus")?;
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
                        f.member("outputActive", active)?;
                        f.member("outputPaused", paused)?;
                        f.member("outputTimecode", &output_timecode)?;
                        f.member("outputDuration", output_duration)?;
                        f.member("outputCongestion", 0.0)?;
                        f.member("outputBytes", output_bytes)?;
                        f.member("outputSkippedFrames", 0)?;
                        f.member("outputTotalFrames", 0)?;
                        f.member("outputPath", &output_path)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn build_get_output_status_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &ObswsInputRegistry,
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
            build_get_stream_status_as_output_response(request_id, input_registry)
        }
        OBSWS_RECORD_OUTPUT_NAME => {
            build_get_record_status_as_output_response(request_id, input_registry)
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
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", request_type)?;
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
                    nojson::object(|f| f.member("outputActive", output_active)),
                )
            }),
        )
    })
    .to_string()
}

fn build_record_output_state_response(
    request_type: &str,
    request_id: &str,
    output_active: bool,
    output_paused: bool,
) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", request_type)?;
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
                        f.member("outputActive", output_active)?;
                        f.member("outputPaused", output_paused)
                    }),
                )
            }),
        )
    })
    .to_string()
}

fn empty_success_response(request_type: &str, request_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", request_type)?;
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

pub fn build_start_stream_response(request_id: &str, output_active: bool) -> String {
    build_output_active_response("StartStream", request_id, output_active)
}

pub fn build_toggle_stream_response(request_id: &str, output_active: bool) -> String {
    build_output_active_response("ToggleStream", request_id, output_active)
}

pub fn build_stop_stream_response(request_id: &str) -> String {
    empty_success_response("StopStream", request_id)
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
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "StopRecord")?;
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
                    nojson::object(|f| f.member("outputPath", output_path)),
                )
            }),
        )
    })
    .to_string()
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
) -> String {
    let active = input_registry.is_stream_active();
    let duration = if active {
        input_registry.stream_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetOutputStatus")?;
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
                        f.member("outputActive", active)?;
                        f.member("outputReconnecting", false)?;
                        f.member("outputTimecode", &output_timecode)?;
                        f.member("outputDuration", output_duration)?;
                        f.member("outputCongestion", 0.0)?;
                        f.member("outputBytes", 0)?;
                        f.member("outputSkippedFrames", 0)?;
                        f.member("outputTotalFrames", 0)
                    }),
                )
            }),
        )
    })
    .to_string()
}

fn build_get_record_status_as_output_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
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
    nojson::object(|f| {
        f.member("op", OBSWS_OP_REQUEST_RESPONSE)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetOutputStatus")?;
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
                        f.member("outputActive", active)?;
                        f.member("outputPaused", paused)?;
                        f.member("outputTimecode", &output_timecode)?;
                        f.member("outputDuration", output_duration)?;
                        f.member("outputBytes", output_bytes)?;
                        f.member("outputSkippedFrames", 0)?;
                        f.member("outputTotalFrames", 0)?;
                        f.member("outputPath", &output_path)
                    }),
                )
            }),
        )
    })
    .to_string()
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
