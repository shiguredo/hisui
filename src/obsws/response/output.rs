use std::path::PathBuf;

use crate::obsws::input_registry::ObswsInputRegistry;

#[cfg(feature = "player")]
const OBSWS_PLAYER_OUTPUT_NAME: &str = "player";
#[cfg(feature = "player")]
const OBSWS_PLAYER_OUTPUT_KIND: &str = "player_output";

struct ParsedObswsS3Destination {
    bucket: String,
    prefix: String,
    region: String,
    endpoint: Option<String>,
    use_path_style: bool,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    lifetime_days: Option<u32>,
}

/// outputs BTreeMap から動的に output リストを構築する。
pub(crate) fn build_get_output_list_response(
    request_id: &str,
    outputs: &std::collections::BTreeMap<
        String,
        crate::obsws::coordinator::output_dynamic::OutputState,
    >,
    #[cfg(feature = "player")] player_active: bool,
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
                #[cfg(feature = "player")]
                {
                    // player は outputs BTreeMap に含まれないため、別途追加する
                    let _ = player_active;
                    f.element(nojson::object(|f| {
                        f.member("outputName", OBSWS_PLAYER_OUTPUT_NAME)?;
                        f.member("outputKind", OBSWS_PLAYER_OUTPUT_KIND)
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

/// HLS 出力の設定をパースして registry に保存する。
/// 省略されたフィールドは既存値を維持する。
/// HLS 設定をパースして input_registry に適用する。
/// coordinator から呼び出し可能。
pub(crate) fn parse_and_apply_hls_settings(
    output_settings: &nojson::RawJsonValue<'_, '_>,
    input_registry: &mut ObswsInputRegistry,
) -> Result<(), String> {
    parse_hls_settings(*output_settings, input_registry)
}

fn parse_hls_settings(
    output_settings: nojson::RawJsonValue<'_, '_>,
    input_registry: &mut ObswsInputRegistry,
) -> Result<(), String> {
    // destination オブジェクトのパース
    let destination: Option<crate::obsws::input_registry::HlsDestination> =
        if let Some(dest_value) = output_settings
            .to_member("destination")
            .map_err(|e| e.to_string())?
            .optional()
        {
            let dest_type: String = dest_value
                .to_member("type")
                .map_err(|e| e.to_string())?
                .required()
                .map_err(|_| "destination.type is required".to_owned())?
                .try_into()
                .map_err(|e: nojson::JsonParseError| e.to_string())?;

            match dest_type.as_str() {
                "filesystem" => {
                    let directory: String = dest_value
                        .to_member("directory")
                        .map_err(|e| e.to_string())?
                        .required()
                        .map_err(|_| "destination.directory is required for filesystem".to_owned())?
                        .try_into()
                        .map_err(|e: nojson::JsonParseError| e.to_string())?;
                    if directory.is_empty() {
                        return Err("destination.directory must not be empty".to_owned());
                    }
                    Some(crate::obsws::input_registry::HlsDestination::Filesystem { directory })
                }
                "s3" => {
                    let parsed = parse_obsws_s3_destination(dest_value)?;
                    Some(crate::obsws::input_registry::HlsDestination::S3 {
                        bucket: parsed.bucket,
                        prefix: parsed.prefix,
                        region: parsed.region,
                        endpoint: parsed.endpoint,
                        use_path_style: parsed.use_path_style,
                        access_key_id: parsed.access_key_id,
                        secret_access_key: parsed.secret_access_key,
                        session_token: parsed.session_token,
                        lifetime_days: parsed.lifetime_days,
                    })
                }
                _ => {
                    return Err(format!(
                        "destination.type must be \"filesystem\" or \"s3\", got \"{dest_type}\""
                    ));
                }
            }
        } else {
            None
        };

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

    // variants 配列のパース
    let variants: Option<Vec<crate::obsws::input_registry::HlsVariant>> =
        if let Some(variants_value) = output_settings
            .to_member("variants")
            .map_err(|e| e.to_string())?
            .optional()
        {
            let mut variants = Vec::new();
            for item in variants_value.to_array().map_err(|e| e.to_string())? {
                let video_bitrate: usize = item
                    .to_member("videoBitrate")
                    .map_err(|e| e.to_string())?
                    .required()
                    .map_err(|_| "variants[].videoBitrate is required".to_owned())?
                    .try_into()
                    .map_err(|e: nojson::JsonParseError| e.to_string())?;
                let audio_bitrate: usize = item
                    .to_member("audioBitrate")
                    .map_err(|e| e.to_string())?
                    .required()
                    .map_err(|_| "variants[].audioBitrate is required".to_owned())?
                    .try_into()
                    .map_err(|e: nojson::JsonParseError| e.to_string())?;
                let width: Option<usize> = item
                    .to_member("width")
                    .map_err(|e| e.to_string())?
                    .optional()
                    .map(|v| v.try_into())
                    .transpose()
                    .map_err(|e: nojson::JsonParseError| e.to_string())?;
                let height: Option<usize> = item
                    .to_member("height")
                    .map_err(|e| e.to_string())?
                    .optional()
                    .map(|v| v.try_into())
                    .transpose()
                    .map_err(|e: nojson::JsonParseError| e.to_string())?;

                if video_bitrate == 0 {
                    return Err("variants[].videoBitrate must be positive".to_owned());
                }
                if audio_bitrate == 0 {
                    return Err("variants[].audioBitrate must be positive".to_owned());
                }
                let width = match width {
                    Some(0) => return Err("variants[].width must be positive".to_owned()),
                    Some(w) => Some(
                        crate::types::EvenUsize::new(w).ok_or("variants[].width must be even")?,
                    ),
                    None => None,
                };
                let height = match height {
                    Some(0) => return Err("variants[].height must be positive".to_owned()),
                    Some(h) => Some(
                        crate::types::EvenUsize::new(h).ok_or("variants[].height must be even")?,
                    ),
                    None => None,
                };
                // width と height は両方指定するか両方省略する必要がある
                if width.is_some() != height.is_some() {
                    return Err(
                    "variants[].width and variants[].height must both be specified or both omitted"
                        .to_owned(),
                );
                }

                variants.push(crate::obsws::input_registry::HlsVariant {
                    video_bitrate_bps: video_bitrate,
                    audio_bitrate_bps: audio_bitrate,
                    width,
                    height,
                });
            }
            if variants.is_empty() {
                return Err("variants must not be empty".to_owned());
            }
            Some(variants)
        } else {
            None
        };

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
        Some(ref s) => s.parse::<crate::obsws::input_registry::HlsSegmentFormat>()?,
        None => input_registry.hls_settings().segment_format,
    };

    let existing = input_registry.hls_settings().clone();
    input_registry.set_hls_settings(crate::obsws::input_registry::ObswsHlsSettings {
        destination: destination.or(existing.destination),
        segment_duration: segment_duration.unwrap_or(existing.segment_duration),
        max_retained_segments: max_retained_segments.unwrap_or(existing.max_retained_segments),
        segment_format,
        variants: variants.unwrap_or(existing.variants),
    });
    Ok(())
}

// --- MPEG-DASH 出力 ---

/// MPEG-DASH 出力の設定をパースして registry に保存する。
/// 省略されたフィールドは既存値を維持する。
/// DASH 設定をパースして input_registry に適用する。
/// coordinator から呼び出し可能。
pub(crate) fn parse_and_apply_dash_settings(
    output_settings: &nojson::RawJsonValue<'_, '_>,
    input_registry: &mut ObswsInputRegistry,
) -> Result<(), String> {
    parse_dash_settings(*output_settings, input_registry)
}

fn parse_dash_settings(
    output_settings: nojson::RawJsonValue<'_, '_>,
    input_registry: &mut ObswsInputRegistry,
) -> Result<(), String> {
    // destination オブジェクトのパース
    let destination: Option<crate::obsws::input_registry::DashDestination> =
        if let Some(dest_value) = output_settings
            .to_member("destination")
            .map_err(|e| e.to_string())?
            .optional()
        {
            let dest_type: String = dest_value
                .to_member("type")
                .map_err(|e| e.to_string())?
                .required()
                .map_err(|_| "destination.type is required".to_owned())?
                .try_into()
                .map_err(|e: nojson::JsonParseError| e.to_string())?;

            match dest_type.as_str() {
                "filesystem" => {
                    let directory: String = dest_value
                        .to_member("directory")
                        .map_err(|e| e.to_string())?
                        .required()
                        .map_err(|_| "destination.directory is required for filesystem".to_owned())?
                        .try_into()
                        .map_err(|e: nojson::JsonParseError| e.to_string())?;
                    if directory.is_empty() {
                        return Err("destination.directory must not be empty".to_owned());
                    }
                    Some(crate::obsws::input_registry::DashDestination::Filesystem { directory })
                }
                "s3" => {
                    let parsed = parse_obsws_s3_destination(dest_value)?;
                    Some(crate::obsws::input_registry::DashDestination::S3 {
                        bucket: parsed.bucket,
                        prefix: parsed.prefix,
                        region: parsed.region,
                        endpoint: parsed.endpoint,
                        use_path_style: parsed.use_path_style,
                        access_key_id: parsed.access_key_id,
                        secret_access_key: parsed.secret_access_key,
                        session_token: parsed.session_token,
                        lifetime_days: parsed.lifetime_days,
                    })
                }
                _ => {
                    return Err(format!(
                        "destination.type must be \"filesystem\" or \"s3\", got \"{dest_type}\""
                    ));
                }
            }
        } else {
            None
        };

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

    // variants 配列のパース
    let variants: Option<Vec<crate::obsws::input_registry::DashVariant>> =
        if let Some(variants_value) = output_settings
            .to_member("variants")
            .map_err(|e| e.to_string())?
            .optional()
        {
            let mut variants = Vec::new();
            for item in variants_value.to_array().map_err(|e| e.to_string())? {
                let video_bitrate: usize = item
                    .to_member("videoBitrate")
                    .map_err(|e| e.to_string())?
                    .required()
                    .map_err(|_| "variants[].videoBitrate is required".to_owned())?
                    .try_into()
                    .map_err(|e: nojson::JsonParseError| e.to_string())?;
                let audio_bitrate: usize = item
                    .to_member("audioBitrate")
                    .map_err(|e| e.to_string())?
                    .required()
                    .map_err(|_| "variants[].audioBitrate is required".to_owned())?
                    .try_into()
                    .map_err(|e: nojson::JsonParseError| e.to_string())?;
                let width: Option<usize> = item
                    .to_member("width")
                    .map_err(|e| e.to_string())?
                    .optional()
                    .map(|v| v.try_into())
                    .transpose()
                    .map_err(|e: nojson::JsonParseError| e.to_string())?;
                let height: Option<usize> = item
                    .to_member("height")
                    .map_err(|e| e.to_string())?
                    .optional()
                    .map(|v| v.try_into())
                    .transpose()
                    .map_err(|e: nojson::JsonParseError| e.to_string())?;

                if video_bitrate == 0 {
                    return Err("variants[].videoBitrate must be positive".to_owned());
                }
                if audio_bitrate == 0 {
                    return Err("variants[].audioBitrate must be positive".to_owned());
                }
                let width = match width {
                    Some(0) => return Err("variants[].width must be positive".to_owned()),
                    Some(w) => Some(
                        crate::types::EvenUsize::new(w).ok_or("variants[].width must be even")?,
                    ),
                    None => None,
                };
                let height = match height {
                    Some(0) => return Err("variants[].height must be positive".to_owned()),
                    Some(h) => Some(
                        crate::types::EvenUsize::new(h).ok_or("variants[].height must be even")?,
                    ),
                    None => None,
                };
                // width と height は両方指定するか両方省略する必要がある
                if width.is_some() != height.is_some() {
                    return Err(
                    "variants[].width and variants[].height must both be specified or both omitted"
                        .to_owned(),
                    );
                }

                variants.push(crate::obsws::input_registry::DashVariant {
                    video_bitrate_bps: video_bitrate,
                    audio_bitrate_bps: audio_bitrate,
                    width,
                    height,
                });
            }
            if variants.is_empty() {
                return Err("variants must not be empty".to_owned());
            }
            Some(variants)
        } else {
            None
        };

    // ビデオコーデックのパース
    let video_codec: Option<crate::types::CodecName> = output_settings
        .to_member("videoCodec")
        .map_err(|e| e.to_string())?
        .optional()
        .map(|v| -> Result<crate::types::CodecName, String> {
            let s: String = v
                .try_into()
                .map_err(|e: nojson::JsonParseError| e.to_string())?;
            crate::types::CodecName::parse_video(&s)
        })
        .transpose()?;

    // オーディオコーデックのパース
    let audio_codec: Option<crate::types::CodecName> = output_settings
        .to_member("audioCodec")
        .map_err(|e| e.to_string())?
        .optional()
        .map(|v| -> Result<crate::types::CodecName, String> {
            let s: String = v
                .try_into()
                .map_err(|e: nojson::JsonParseError| e.to_string())?;
            crate::types::CodecName::parse_audio(&s)
        })
        .transpose()?;

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

    let existing = input_registry.dash_settings().clone();
    input_registry.set_dash_settings(crate::obsws::input_registry::ObswsDashSettings {
        destination: destination.or(existing.destination),
        segment_duration: segment_duration.unwrap_or(existing.segment_duration),
        max_retained_segments: max_retained_segments.unwrap_or(existing.max_retained_segments),
        variants: variants.unwrap_or(existing.variants),
        video_codec: video_codec.unwrap_or(existing.video_codec),
        audio_codec: audio_codec.unwrap_or(existing.audio_codec),
    });
    Ok(())
}

fn parse_obsws_s3_destination(
    dest_value: nojson::RawJsonValue<'_, '_>,
) -> Result<ParsedObswsS3Destination, String> {
    let bucket: String = dest_value
        .to_member("bucket")
        .map_err(|e| e.to_string())?
        .required()
        .map_err(|_| "destination.bucket is required for s3".to_owned())?
        .try_into()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;
    let prefix: String = super::optional_non_empty_string_member(dest_value, "prefix")
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let region: String = dest_value
        .to_member("region")
        .map_err(|e| e.to_string())?
        .required()
        .map_err(|_| "destination.region is required for s3".to_owned())?
        .try_into()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;
    let endpoint: Option<String> = super::optional_non_empty_string_member(dest_value, "endpoint")
        .map_err(|e| e.to_string())?;
    let use_path_style: bool = dest_value
        .to_member("usePathStyle")
        .map_err(|e| e.to_string())?
        .optional()
        .map(|v| v.try_into())
        .transpose()
        .map_err(|e: nojson::JsonParseError| e.to_string())?
        .unwrap_or(false);

    let creds_value = dest_value
        .to_member("credentials")
        .map_err(|e| e.to_string())?
        .required()
        .map_err(|_| "destination.credentials is required for s3".to_owned())?;
    let access_key_id: String = creds_value
        .to_member("accessKeyId")
        .map_err(|e| e.to_string())?
        .required()
        .map_err(|_| "credentials.accessKeyId is required".to_owned())?
        .try_into()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;
    let secret_access_key: String = creds_value
        .to_member("secretAccessKey")
        .map_err(|e| e.to_string())?
        .required()
        .map_err(|_| "credentials.secretAccessKey is required".to_owned())?
        .try_into()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;
    let session_token: Option<String> =
        super::optional_non_empty_string_member(creds_value, "sessionToken")
            .map_err(|e| e.to_string())?;

    let lifetime_days: Option<u32> = dest_value
        .to_member("lifetimeDays")
        .map_err(|e| e.to_string())?
        .optional()
        .map(|v| v.try_into())
        .transpose()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;

    if bucket.is_empty() {
        return Err("destination.bucket must not be empty".to_owned());
    }
    if region.is_empty() {
        return Err("destination.region must not be empty".to_owned());
    }
    if let Some(days) = lifetime_days {
        if days == 0 {
            return Err("destination.lifetimeDays must be positive".to_owned());
        }
        if prefix.is_empty() {
            return Err("destination.prefix is required when lifetimeDays is set (empty prefix would apply lifecycle rules to the entire bucket)".to_owned());
        }
    }

    Ok(ParsedObswsS3Destination {
        bucket,
        prefix,
        region,
        endpoint,
        use_path_style,
        access_key_id,
        secret_access_key,
        session_token,
        lifetime_days,
    })
}
