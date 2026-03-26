use std::path::PathBuf;

use crate::obsws::input_registry::{ObswsInputRegistry, ObswsStreamServiceSettings};
use crate::obsws::protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_RESOURCE_NOT_FOUND,
};

use super::{
    parse_get_output_settings_fields, parse_get_output_status_fields,
    parse_request_data_or_error_response, parse_set_output_settings_fields,
    parse_set_record_directory_fields, parse_set_stream_service_settings_fields,
};

// `stream` は OBS 互換の主配信 Output として扱う。
// 現状は `streamServiceSettings` を使う RTMP 系の配信実装に結び付いているため、
// raw frame をそのまま WebRTC SDK に渡す `sora` とは分離している。
// 将来的に `stream` を多プロトコル化する余地はあるが、現時点では API の意味を
// 明確に保つことを優先して独立 Output 名を維持する。
const OBSWS_STREAM_OUTPUT_NAME: &str = "stream";
const OBSWS_RECORD_OUTPUT_NAME: &str = "record";
const OBSWS_RTMP_OUTBOUND_OUTPUT_NAME: &str = "rtmp_outbound";
const OBSWS_SORA_OUTPUT_NAME: &str = "sora";
const OBSWS_HLS_OUTPUT_NAME: &str = "hls";
const OBSWS_STREAM_OUTPUT_KIND: &str = "rtmp_output";
const OBSWS_RECORD_OUTPUT_KIND: &str = "mp4_output";
const OBSWS_RTMP_OUTBOUND_OUTPUT_KIND: &str = "rtmp_outbound_output";
const OBSWS_SORA_OUTPUT_KIND: &str = "sora_webrtc_output";
const OBSWS_HLS_OUTPUT_KIND: &str = "hls_output";

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
) -> nojson::RawJsonOwned {
    let settings = input_registry.stream_service_settings();
    super::build_request_response_success("GetStreamServiceSettings", request_id, |f| {
        f.member("streamServiceType", &settings.stream_service_type)?;
        f.member(
            "streamServiceSettings",
            nojson::object(|f| {
                // OBS 互換のデフォルト値を含める
                f.member("bwtest", false)?;
                if let Some(server) = &settings.server {
                    f.member("server", server)?;
                }
                // OBS は key が未設定でも空文字列を返す
                f.member("key", settings.key.as_deref().unwrap_or(""))?;
                f.member("use_auth", false)
            }),
        )
    })
}

pub fn build_set_stream_service_settings_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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
) -> nojson::RawJsonOwned {
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

pub fn build_get_output_list_response(request_id: &str) -> nojson::RawJsonOwned {
    let outputs = [
        ObswsOutputEntry {
            output_name: OBSWS_STREAM_OUTPUT_NAME,
            output_kind: OBSWS_STREAM_OUTPUT_KIND,
        },
        ObswsOutputEntry {
            output_name: OBSWS_RECORD_OUTPUT_NAME,
            output_kind: OBSWS_RECORD_OUTPUT_KIND,
        },
        ObswsOutputEntry {
            output_name: OBSWS_RTMP_OUTBOUND_OUTPUT_NAME,
            output_kind: OBSWS_RTMP_OUTBOUND_OUTPUT_KIND,
        },
        ObswsOutputEntry {
            output_name: OBSWS_SORA_OUTPUT_NAME,
            output_kind: OBSWS_SORA_OUTPUT_KIND,
        },
        ObswsOutputEntry {
            output_name: OBSWS_HLS_OUTPUT_NAME,
            output_kind: OBSWS_HLS_OUTPUT_KIND,
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
) -> nojson::RawJsonOwned {
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
        OBSWS_RTMP_OUTBOUND_OUTPUT_NAME => {
            build_rtmp_outbound_output_settings_response(request_id, input_registry)
        }
        OBSWS_SORA_OUTPUT_NAME => build_sora_output_settings_response(request_id, input_registry),
        OBSWS_HLS_OUTPUT_NAME => build_hls_output_settings_response(request_id, input_registry),
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
) -> nojson::RawJsonOwned {
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
        OBSWS_RTMP_OUTBOUND_OUTPUT_NAME => {
            let output_settings = fields.output_settings.value();
            let output_url =
                match super::optional_non_empty_string_member(output_settings, "outputUrl") {
                    Ok(v) => v,
                    Err(error) => {
                        return super::build_request_response_error(
                            "SetOutputSettings",
                            request_id,
                            super::request_status_code_for_parse_error(&error),
                            &error.to_string(),
                        );
                    }
                };
            let stream_name =
                match super::optional_non_empty_string_member(output_settings, "streamName") {
                    Ok(v) => v,
                    Err(error) => {
                        return super::build_request_response_error(
                            "SetOutputSettings",
                            request_id,
                            super::request_status_code_for_parse_error(&error),
                            &error.to_string(),
                        );
                    }
                };
            let existing = input_registry.rtmp_outbound_settings().clone();
            input_registry.set_rtmp_outbound_settings(
                crate::obsws::input_registry::ObswsRtmpOutboundSettings {
                    output_url: output_url.or(existing.output_url),
                    stream_name: stream_name.or(existing.stream_name),
                },
            );
            super::build_request_response_success_no_data("SetOutputSettings", request_id)
        }
        OBSWS_SORA_OUTPUT_NAME => {
            let output_settings = fields.output_settings.value();
            match parse_sora_publisher_settings(output_settings, input_registry) {
                Ok(()) => {
                    super::build_request_response_success_no_data("SetOutputSettings", request_id)
                }
                Err(error) => super::build_request_response_error(
                    "SetOutputSettings",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &error,
                ),
            }
        }
        OBSWS_HLS_OUTPUT_NAME => {
            let output_settings = fields.output_settings.value();
            match parse_hls_settings(output_settings, input_registry) {
                Ok(()) => {
                    super::build_request_response_success_no_data("SetOutputSettings", request_id)
                }
                Err(error) => super::build_request_response_error(
                    "SetOutputSettings",
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &error,
                ),
            }
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
) -> nojson::RawJsonOwned {
    let record_directory = input_registry.record_directory().display().to_string();
    super::build_request_response_success("GetRecordDirectory", request_id, |f| {
        f.member("recordDirectory", &record_directory)
    })
}

pub fn build_set_record_directory_response(
    request_id: &str,
    request_data: Option<&nojson::RawJsonOwned>,
    input_registry: &mut ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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
) -> nojson::RawJsonOwned {
    let active = input_registry.is_record_active();
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
) -> nojson::RawJsonOwned {
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
        OBSWS_RTMP_OUTBOUND_OUTPUT_NAME => {
            build_get_rtmp_outbound_status_as_output_response(request_id, input_registry)
        }
        OBSWS_SORA_OUTPUT_NAME => {
            build_get_sora_status_as_output_response(request_id, input_registry)
        }
        OBSWS_HLS_OUTPUT_NAME => {
            build_get_hls_status_as_output_response(request_id, input_registry)
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
) -> nojson::RawJsonOwned {
    super::build_request_response_success(request_type, request_id, |f| {
        f.member("outputActive", output_active)
    })
}

fn build_output_active_record_response(
    request_type: &str,
    request_id: &str,
    output_active: bool,
) -> nojson::RawJsonOwned {
    super::build_request_response_success(request_type, request_id, |f| {
        f.member("outputActive", output_active)
    })
}

fn build_stream_output_settings_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
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
) -> nojson::RawJsonOwned {
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

pub fn build_start_stream_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success_no_data("StartStream", request_id)
}

pub fn build_start_output_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success_no_data("StartOutput", request_id)
}

pub fn build_toggle_stream_response(request_id: &str, output_active: bool) -> nojson::RawJsonOwned {
    build_output_active_response("ToggleStream", request_id, output_active)
}

pub fn build_toggle_output_response(request_id: &str, output_active: bool) -> nojson::RawJsonOwned {
    build_output_active_response("ToggleOutput", request_id, output_active)
}

pub fn build_stop_stream_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success_no_data("StopStream", request_id)
}

pub fn build_stop_output_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success_no_data("StopOutput", request_id)
}

pub fn build_toggle_record_response(request_id: &str, output_active: bool) -> nojson::RawJsonOwned {
    build_output_active_record_response("ToggleRecord", request_id, output_active)
}

pub fn build_start_record_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success_no_data("StartRecord", request_id)
}

pub fn build_stop_record_response(request_id: &str, output_path: &str) -> nojson::RawJsonOwned {
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
) -> nojson::RawJsonOwned {
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
) -> nojson::RawJsonOwned {
    let active = input_registry.is_record_active();
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
        f.member("outputTimecode", &output_timecode)?;
        f.member("outputDuration", output_duration)?;
        f.member("outputCongestion", 0.0)?;
        f.member("outputBytes", output_bytes)?;
        f.member("outputSkippedFrames", output_stats.record_skipped_frames)?;
        f.member("outputTotalFrames", output_stats.record_total_frames)?;
        f.member("outputPath", &output_path)
    })
}

fn build_rtmp_outbound_output_settings_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let settings = input_registry.rtmp_outbound_settings();
    super::build_request_response_success("GetOutputSettings", request_id, |f| {
        f.member("outputName", OBSWS_RTMP_OUTBOUND_OUTPUT_NAME)?;
        f.member("outputKind", OBSWS_RTMP_OUTBOUND_OUTPUT_KIND)?;
        f.member("outputSettings", settings)
    })
}

fn build_get_rtmp_outbound_status_as_output_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let active = input_registry.is_rtmp_outbound_active();
    let duration = if active {
        input_registry.rtmp_outbound_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    super::build_request_response_success("GetOutputStatus", request_id, |f| {
        f.member("outputActive", active)?;
        f.member("outputReconnecting", false)?;
        f.member("outputTimecode", &output_timecode)?;
        f.member("outputDuration", output_duration)?;
        f.member("outputCongestion", 0.0)?;
        f.member("outputBytes", 0)?;
        f.member("outputSkippedFrames", 0)?;
        f.member("outputTotalFrames", 0)
    })
}

fn build_sora_output_settings_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let settings = input_registry.sora_publisher_settings();
    super::build_request_response_success("GetOutputSettings", request_id, |f| {
        f.member("outputName", OBSWS_SORA_OUTPUT_NAME)?;
        f.member("outputKind", OBSWS_SORA_OUTPUT_KIND)?;
        f.member("outputSettings", settings)
    })
}

fn build_get_sora_status_as_output_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let active = input_registry.is_sora_publisher_active();
    let duration = if active {
        input_registry.sora_publisher_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    // TODO: outputBytes / outputSkippedFrames / outputTotalFrames は sora-rust-sdk に
    // フレーム単位の統計 API が追加されたら対応する。現時点では 0 固定。
    super::build_request_response_success("GetOutputStatus", request_id, |f| {
        f.member("outputActive", active)?;
        f.member("outputReconnecting", false)?;
        f.member("outputTimecode", &output_timecode)?;
        f.member("outputDuration", output_duration)?;
        f.member("outputCongestion", 0.0)?;
        f.member("outputBytes", 0)?;
        f.member("outputSkippedFrames", 0)?;
        f.member("outputTotalFrames", 0)
    })
}

/// soraSdkSettings をパースして registry に保存する。
fn parse_sora_publisher_settings(
    output_settings: nojson::RawJsonValue<'_, '_>,
    input_registry: &mut ObswsInputRegistry,
) -> Result<(), String> {
    let sora_sdk_settings = output_settings
        .to_member("soraSdkSettings")
        .map_err(|e| e.to_string())?
        .required()
        .map_err(|e| e.to_string())?;

    let signaling_urls: Option<Vec<String>> = sora_sdk_settings
        .to_member("signalingUrls")
        .map_err(|e| e.to_string())?
        .optional()
        .map(|v| v.try_into())
        .transpose()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;

    let channel_id: Option<String> =
        super::optional_non_empty_string_member(sora_sdk_settings, "channelId")
            .map_err(|e| e.to_string())?;

    let client_id: Option<String> =
        super::optional_non_empty_string_member(sora_sdk_settings, "clientId")
            .map_err(|e| e.to_string())?;

    let bundle_id: Option<String> =
        super::optional_non_empty_string_member(sora_sdk_settings, "bundleId")
            .map_err(|e| e.to_string())?;

    // metadata は object として保持する
    let metadata: Option<nojson::RawJsonOwned> = {
        let member = sora_sdk_settings
            .to_member("metadata")
            .map_err(|e| e.to_string())?;
        match member.optional() {
            Some(value) => {
                if !value.kind().is_object() {
                    return Err("metadata must be a JSON object".to_owned());
                }
                Some(value.extract().into_owned())
            }
            None => None,
        }
    };

    input_registry.set_sora_publisher_settings(
        crate::obsws::input_registry::ObswsSoraPublisherSettings {
            signaling_urls: signaling_urls.unwrap_or_default(),
            channel_id,
            client_id,
            bundle_id,
            metadata,
        },
    );
    Ok(())
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

fn build_hls_output_settings_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let settings = input_registry.hls_settings();
    super::build_request_response_success("GetOutputSettings", request_id, |f| {
        f.member("outputName", OBSWS_HLS_OUTPUT_NAME)?;
        f.member("outputKind", OBSWS_HLS_OUTPUT_KIND)?;
        f.member("outputSettings", settings)
    })
}

fn build_get_hls_status_as_output_response(
    request_id: &str,
    input_registry: &ObswsInputRegistry,
) -> nojson::RawJsonOwned {
    let active = input_registry.is_hls_active();
    let duration = if active {
        input_registry.hls_uptime()
    } else {
        std::time::Duration::ZERO
    };
    let output_duration = duration.as_millis().min(i64::MAX as u128) as i64;
    let output_timecode = format_timecode(duration);
    let output_path = input_registry
        .hls_output_directory()
        .map(|p| p.join("playlist.m3u8").display().to_string())
        .unwrap_or_default();
    // TODO: outputBytes / outputSkippedFrames / outputTotalFrames は未対応。0 固定。
    super::build_request_response_success("GetOutputStatus", request_id, |f| {
        f.member("outputActive", active)?;
        f.member("outputReconnecting", false)?;
        f.member("outputTimecode", &output_timecode)?;
        f.member("outputDuration", output_duration)?;
        f.member("outputCongestion", 0.0)?;
        f.member("outputBytes", 0)?;
        f.member("outputSkippedFrames", 0)?;
        f.member("outputTotalFrames", 0)?;
        f.member("outputPath", &output_path)
    })
}

/// HLS 出力の設定をパースして registry に保存する。
/// 省略されたフィールドは既存値を維持する。
fn parse_hls_settings(
    output_settings: nojson::RawJsonValue<'_, '_>,
    input_registry: &mut ObswsInputRegistry,
) -> Result<(), String> {
    let output_directory: Option<String> =
        super::optional_non_empty_string_member(output_settings, "outputDirectory")
            .map_err(|e| e.to_string())?;

    let segment_duration: Option<f64> = output_settings
        .to_member("segmentDuration")
        .map_err(|e| e.to_string())?
        .optional()
        .map(|v| v.try_into())
        .transpose()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;

    let max_retained_segments: Option<usize> = output_settings
        .to_member("maxRetainedSegments")
        .map_err(|e| e.to_string())?
        .optional()
        .map(|v| v.try_into())
        .transpose()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;

    let segment_format_str: Option<String> =
        super::optional_non_empty_string_member(output_settings, "segmentFormat")
            .map_err(|e| e.to_string())?;

    if let Some(duration) = segment_duration
        && duration <= 0.0
    {
        return Err("segmentDuration must be positive".to_owned());
    }
    if let Some(count) = max_retained_segments
        && count == 0
    {
        return Err("maxRetainedSegments must be at least 1".to_owned());
    }
    let segment_format = match segment_format_str {
        Some(ref s) => crate::obsws::input_registry::HlsSegmentFormat::from_str(s)
            .ok_or_else(|| format!("segmentFormat must be \"mpegts\" or \"fmp4\", got \"{s}\""))?,
        None => input_registry.hls_settings().segment_format,
    };

    let existing = input_registry.hls_settings().clone();
    input_registry.set_hls_settings(crate::obsws::input_registry::ObswsHlsSettings {
        output_directory: output_directory.or(existing.output_directory),
        segment_duration: segment_duration.unwrap_or(existing.segment_duration),
        max_retained_segments: max_retained_segments.unwrap_or(existing.max_retained_segments),
        segment_format,
    });
    Ok(())
}
