use std::path::PathBuf;

/// outputs BTreeMap から output リストを構築する。
pub(crate) fn build_get_output_list_response(
    request_id: &str,
    outputs: &std::collections::BTreeMap<
        String,
        crate::obsws::coordinator::output_registry::OutputState,
    >,
) -> nojson::RawJsonOwned {
    super::build_request_response_success("GetOutputList", request_id, |f| {
        f.member(
            "outputs",
            nojson::array(|f| {
                for (name, state) in outputs {
                    f.element(nojson::object(|f| {
                        f.member("outputName", name.as_str())?;
                        f.member("outputKind", state.output_kind.as_kind_str())
                    }))?;
                }
                Ok(())
            }),
        )
    })
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
    build_output_active_response("ToggleRecord", request_id, output_active)
}

pub fn build_start_record_response(request_id: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success_no_data("StartRecord", request_id)
}

pub fn build_stop_record_response(request_id: &str, output_path: &str) -> nojson::RawJsonOwned {
    super::build_request_response_success("StopRecord", request_id, |f| {
        f.member("outputPath", output_path)
    })
}

pub(crate) fn format_timecode(duration: std::time::Duration) -> String {
    let total_millis = duration.as_millis();
    let millis = total_millis % 1_000;
    let total_secs = total_millis / 1_000;
    let secs = total_secs % 60;
    let total_minutes = total_secs / 60;
    let minutes = total_minutes % 60;
    let hours = total_minutes / 60;
    format!("{hours:02}:{minutes:02}:{secs:02}.{millis:03}")
}

pub(crate) fn resolve_record_directory_path(record_directory: &str) -> Result<PathBuf, String> {
    std::path::absolute(record_directory)
        .map_err(|e| format!("Failed to resolve absolute record directory path: {e}"))
}
