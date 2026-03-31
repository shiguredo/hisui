//! obsws の永続 state file の読み書きを行うモジュール。
//!
//! state file は obsws の output 設定を再起動後も復元するための JSONC ファイルである。
//! 永続化対象: stream / record / rtmp_outbound / sora / hls / mpeg_dash

use std::path::{Path, PathBuf};

use crate::obsws::input_registry::{
    DashDestination, DashVariant, HlsDestination, HlsSegmentFormat, HlsVariant, ObswsDashSettings,
    ObswsHlsSettings, ObswsInputRegistry, ObswsRtmpOutboundSettings, ObswsSoraPublisherSettings,
    ObswsStreamServiceSettings,
};

/// state file のトップレベル構造
pub struct ObswsStateFile {
    pub stream: Option<ObswsStateFileStream>,
    pub record: Option<ObswsStateFileRecord>,
    pub rtmp_outbound: Option<ObswsRtmpOutboundSettings>,
    pub sora: Option<ObswsSoraPublisherSettings>,
    pub hls: Option<ObswsHlsSettings>,
    pub dash: Option<ObswsDashSettings>,
}

/// state file の stream セクション
pub struct ObswsStateFileStream {
    pub stream_service_type: String,
    pub server: Option<String>,
    pub key: Option<String>,
}

/// state file の record セクション
pub struct ObswsStateFileRecord {
    pub record_directory: PathBuf,
}

// ---------------------------------------------------------------------------
// TryFrom 実装
// ---------------------------------------------------------------------------

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ObswsStateFile {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let version: i64 = value.to_member("version")?.required()?.try_into()?;
        if version != 1 {
            return Err(value.to_member("version")?.required()?.invalid(format!(
                "unsupported state file version: {version}, expected 1"
            )));
        }
        let stream: Option<ObswsStateFileStream> = value.to_member("stream")?.try_into()?;
        let record: Option<ObswsStateFileRecord> = value.to_member("record")?.try_into()?;

        // 新規 section
        let rtmp_outbound = parse_optional_rtmp_outbound(value)?;
        let sora = parse_optional_sora(value)?;
        let hls = parse_optional_hls(value)?;
        let dash = parse_optional_dash(value)?;

        Ok(Self {
            stream,
            record,
            rtmp_outbound,
            sora,
            hls,
            dash,
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ObswsStateFileStream {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let stream_service_type: String = value
            .to_member("streamServiceType")?
            .required()?
            .try_into()?;
        if stream_service_type != "rtmp_custom" {
            return Err(value
                .to_member("streamServiceType")?
                .required()?
                .invalid(format!(
                    "unsupported streamServiceType: \"{stream_service_type}\", expected \"rtmp_custom\""
                )));
        }

        let settings_member = value.to_member("streamServiceSettings")?;
        let settings_value: Option<nojson::RawJsonOwned> = settings_member.try_into()?;
        let (server, key) = if let Some(ref settings) = settings_value {
            let sv = settings.value();
            let server: Option<String> = sv.to_member("server")?.try_into()?;
            let key: Option<String> = sv.to_member("key")?.try_into()?;
            (server, key)
        } else {
            (None, None)
        };

        Ok(Self {
            stream_service_type,
            server,
            key,
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ObswsStateFileRecord {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let record_directory: String =
            value.to_member("recordDirectory")?.required()?.try_into()?;
        if record_directory.is_empty() {
            return Err(value
                .to_member("recordDirectory")?
                .required()?
                .invalid("recordDirectory must not be empty"));
        }
        Ok(Self {
            record_directory: PathBuf::from(record_directory),
        })
    }
}

// ---------------------------------------------------------------------------
// rtmpOutbound parse
// ---------------------------------------------------------------------------

fn parse_optional_rtmp_outbound(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<ObswsRtmpOutboundSettings>, nojson::JsonParseError> {
    let member: Option<nojson::RawJsonOwned> = value.to_member("rtmpOutbound")?.try_into()?;
    let Some(ref section) = member else {
        return Ok(None);
    };
    let v = section.value();
    let output_url: Option<String> = v.to_member("outputUrl")?.try_into()?;
    let stream_name: Option<String> = v.to_member("streamName")?.try_into()?;
    Ok(Some(ObswsRtmpOutboundSettings {
        output_url,
        stream_name,
    }))
}

// ---------------------------------------------------------------------------
// sora parse
// ---------------------------------------------------------------------------

fn parse_optional_sora(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<ObswsSoraPublisherSettings>, nojson::JsonParseError> {
    let member: Option<nojson::RawJsonOwned> = value.to_member("sora")?.try_into()?;
    let Some(ref section) = member else {
        return Ok(None);
    };
    let v = section.value();

    // sora セクションは soraSdkSettings を直接含む構造
    let signaling_urls: Option<Vec<String>> = v.to_member("signalingUrls")?.try_into()?;
    let channel_id: Option<String> = v.to_member("channelId")?.try_into()?;
    let client_id: Option<String> = v.to_member("clientId")?.try_into()?;
    let bundle_id: Option<String> = v.to_member("bundleId")?.try_into()?;

    // metadata は object のみ受理する
    let metadata_member: Option<nojson::RawJsonOwned> = v.to_member("metadata")?.try_into()?;
    if let Some(ref m) = metadata_member {
        // to_object() が成功すれば JSON object である
        if m.value().to_object().is_err() {
            return Err(v
                .to_member("metadata")?
                .required()?
                .invalid("metadata must be a JSON object"));
        }
    }

    Ok(Some(ObswsSoraPublisherSettings {
        signaling_urls: signaling_urls.unwrap_or_default(),
        channel_id,
        client_id,
        bundle_id,
        metadata: metadata_member,
    }))
}

// ---------------------------------------------------------------------------
// hls parse
// ---------------------------------------------------------------------------

fn parse_optional_hls(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<ObswsHlsSettings>, nojson::JsonParseError> {
    let member: Option<nojson::RawJsonOwned> = value.to_member("hls")?.try_into()?;
    let Some(ref section) = member else {
        return Ok(None);
    };
    let v = section.value();

    let destination = parse_optional_hls_destination(v)?;

    let segment_duration: Option<f64> = v.to_member("segmentDuration")?.try_into()?;
    let segment_duration =
        segment_duration.unwrap_or(crate::obsws::input_registry::DEFAULT_HLS_SEGMENT_DURATION_SECS);
    // NaN / Infinity は JSON 仕様上パーサが弾くため、ここでは正値チェックのみ行う
    if segment_duration <= 0.0 {
        return Err(v
            .to_member("segmentDuration")?
            .required()?
            .invalid("segmentDuration must be positive"));
    }

    let max_retained_segments: Option<usize> = v.to_member("maxRetainedSegments")?.try_into()?;
    let max_retained_segments = max_retained_segments
        .unwrap_or(crate::obsws::input_registry::DEFAULT_HLS_MAX_RETAINED_SEGMENTS);
    if max_retained_segments == 0 {
        return Err(v
            .to_member("maxRetainedSegments")?
            .required()?
            .invalid("maxRetainedSegments must be at least 1"));
    }

    let segment_format_str: Option<String> = v.to_member("segmentFormat")?.try_into()?;
    let segment_format = match segment_format_str {
        Some(s) => s.parse::<HlsSegmentFormat>().map_err(|e| {
            v.to_member("segmentFormat")
                .expect("already accessed")
                .required()
                .expect("already accessed")
                .invalid(e)
        })?,
        None => HlsSegmentFormat::default(),
    };

    let variants = parse_hls_variants(v)?;

    Ok(Some(ObswsHlsSettings {
        destination,
        segment_duration,
        max_retained_segments,
        segment_format,
        variants,
    }))
}

fn parse_hls_variants(
    v: nojson::RawJsonValue<'_, '_>,
) -> Result<Vec<HlsVariant>, nojson::JsonParseError> {
    let variants_member: Option<nojson::RawJsonOwned> = v.to_member("variants")?.try_into()?;
    let Some(ref variants_json) = variants_member else {
        return Ok(vec![HlsVariant::default()]);
    };
    let mut arr = variants_json.value().to_array()?;
    let mut variants = Vec::new();
    for elem in arr.by_ref() {
        let video_bitrate: usize = elem.to_member("videoBitrate")?.required()?.try_into()?;
        let audio_bitrate: usize = elem.to_member("audioBitrate")?.required()?.try_into()?;
        if video_bitrate == 0 {
            return Err(elem
                .to_member("videoBitrate")?
                .required()?
                .invalid("videoBitrate must be positive"));
        }
        if audio_bitrate == 0 {
            return Err(elem
                .to_member("audioBitrate")?
                .required()?
                .invalid("audioBitrate must be positive"));
        }
        let width: Option<usize> = elem.to_member("width")?.try_into()?;
        let height: Option<usize> = elem.to_member("height")?.try_into()?;
        let (width, height) = parse_variant_dimensions(elem, width, height)?;
        variants.push(HlsVariant {
            video_bitrate_bps: video_bitrate,
            audio_bitrate_bps: audio_bitrate,
            width,
            height,
        });
    }
    if variants.is_empty() {
        return Err(v
            .to_member("variants")?
            .required()?
            .invalid("variants must not be empty"));
    }
    Ok(variants)
}

fn parse_variant_dimensions(
    elem: nojson::RawJsonValue<'_, '_>,
    width: Option<usize>,
    height: Option<usize>,
) -> Result<
    (
        Option<crate::types::EvenUsize>,
        Option<crate::types::EvenUsize>,
    ),
    nojson::JsonParseError,
> {
    match (width, height) {
        (Some(w), Some(h)) => {
            let w = crate::types::EvenUsize::new(w).ok_or_else(|| {
                elem.to_member("width")
                    .expect("already accessed")
                    .required()
                    .expect("already accessed")
                    .invalid("width must be a positive even number")
            })?;
            let h = crate::types::EvenUsize::new(h).ok_or_else(|| {
                elem.to_member("height")
                    .expect("already accessed")
                    .required()
                    .expect("already accessed")
                    .invalid("height must be a positive even number")
            })?;
            Ok((Some(w), Some(h)))
        }
        (None, None) => Ok((None, None)),
        _ => Err(elem.invalid("width and height must be both specified or both omitted")),
    }
}

// ---------------------------------------------------------------------------
// mpegDash parse
// ---------------------------------------------------------------------------

fn parse_optional_dash(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<ObswsDashSettings>, nojson::JsonParseError> {
    let member: Option<nojson::RawJsonOwned> = value.to_member("mpegDash")?.try_into()?;
    let Some(ref section) = member else {
        return Ok(None);
    };
    let v = section.value();

    let destination = parse_optional_dash_destination(v)?;

    let segment_duration: Option<f64> = v.to_member("segmentDuration")?.try_into()?;
    let segment_duration = segment_duration
        .unwrap_or(crate::obsws::input_registry::DEFAULT_DASH_SEGMENT_DURATION_SECS);
    // NaN / Infinity は JSON 仕様上パーサが弾くため、ここでは正値チェックのみ行う
    if segment_duration <= 0.0 {
        return Err(v
            .to_member("segmentDuration")?
            .required()?
            .invalid("segmentDuration must be positive"));
    }

    let max_retained_segments: Option<usize> = v.to_member("maxRetainedSegments")?.try_into()?;
    let max_retained_segments = max_retained_segments
        .unwrap_or(crate::obsws::input_registry::DEFAULT_DASH_MAX_RETAINED_SEGMENTS);
    if max_retained_segments == 0 {
        return Err(v
            .to_member("maxRetainedSegments")?
            .required()?
            .invalid("maxRetainedSegments must be at least 1"));
    }

    let variants = parse_dash_variants(v)?;

    let video_codec_str: Option<String> = v.to_member("videoCodec")?.try_into()?;
    let video_codec = match video_codec_str {
        Some(s) => crate::types::CodecName::parse_video(&s).map_err(|e| {
            v.to_member("videoCodec")
                .expect("already accessed")
                .required()
                .expect("already accessed")
                .invalid(e)
        })?,
        None => crate::types::CodecName::H264,
    };

    let audio_codec_str: Option<String> = v.to_member("audioCodec")?.try_into()?;
    let audio_codec = match audio_codec_str {
        Some(s) => crate::types::CodecName::parse_audio(&s).map_err(|e| {
            v.to_member("audioCodec")
                .expect("already accessed")
                .required()
                .expect("already accessed")
                .invalid(e)
        })?,
        None => crate::types::CodecName::Aac,
    };

    Ok(Some(ObswsDashSettings {
        destination,
        segment_duration,
        max_retained_segments,
        variants,
        video_codec,
        audio_codec,
    }))
}

fn parse_dash_variants(
    v: nojson::RawJsonValue<'_, '_>,
) -> Result<Vec<DashVariant>, nojson::JsonParseError> {
    let variants_member: Option<nojson::RawJsonOwned> = v.to_member("variants")?.try_into()?;
    let Some(ref variants_json) = variants_member else {
        return Ok(vec![DashVariant::default()]);
    };
    let mut arr = variants_json.value().to_array()?;
    let mut variants = Vec::new();
    for elem in arr.by_ref() {
        let video_bitrate: usize = elem.to_member("videoBitrate")?.required()?.try_into()?;
        let audio_bitrate: usize = elem.to_member("audioBitrate")?.required()?.try_into()?;
        if video_bitrate == 0 {
            return Err(elem
                .to_member("videoBitrate")?
                .required()?
                .invalid("videoBitrate must be positive"));
        }
        if audio_bitrate == 0 {
            return Err(elem
                .to_member("audioBitrate")?
                .required()?
                .invalid("audioBitrate must be positive"));
        }
        let width: Option<usize> = elem.to_member("width")?.try_into()?;
        let height: Option<usize> = elem.to_member("height")?.try_into()?;
        let (width, height) = parse_variant_dimensions(elem, width, height)?;
        variants.push(DashVariant {
            video_bitrate_bps: video_bitrate,
            audio_bitrate_bps: audio_bitrate,
            width,
            height,
        });
    }
    if variants.is_empty() {
        return Err(v
            .to_member("variants")?
            .required()?
            .invalid("variants must not be empty"));
    }
    Ok(variants)
}

// ---------------------------------------------------------------------------
// destination parse（HLS / DASH 共通パターン）
// ---------------------------------------------------------------------------

fn parse_optional_hls_destination(
    v: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<HlsDestination>, nojson::JsonParseError> {
    let member: Option<nojson::RawJsonOwned> = v.to_member("destination")?.try_into()?;
    let Some(ref dest_json) = member else {
        return Ok(None);
    };
    let d = dest_json.value();
    let dest_type: String = d.to_member("type")?.required()?.try_into()?;
    match dest_type.as_str() {
        "filesystem" => {
            let directory: String = d.to_member("directory")?.required()?.try_into()?;
            if directory.is_empty() {
                return Err(d
                    .to_member("directory")?
                    .required()?
                    .invalid("directory must not be empty"));
            }
            Ok(Some(HlsDestination::Filesystem { directory }))
        }
        "s3" => {
            let s3 = parse_s3_fields(d)?;
            Ok(Some(HlsDestination::S3 {
                bucket: s3.bucket,
                prefix: s3.prefix,
                region: s3.region,
                endpoint: s3.endpoint,
                use_path_style: s3.use_path_style,
                access_key_id: s3.access_key_id,
                secret_access_key: s3.secret_access_key,
                session_token: s3.session_token,
                lifetime_days: s3.lifetime_days,
            }))
        }
        _ => Err(d.to_member("type")?.required()?.invalid(format!(
            "destination type must be \"filesystem\" or \"s3\", got \"{dest_type}\""
        ))),
    }
}

fn parse_optional_dash_destination(
    v: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<DashDestination>, nojson::JsonParseError> {
    let member: Option<nojson::RawJsonOwned> = v.to_member("destination")?.try_into()?;
    let Some(ref dest_json) = member else {
        return Ok(None);
    };
    let d = dest_json.value();
    let dest_type: String = d.to_member("type")?.required()?.try_into()?;
    match dest_type.as_str() {
        "filesystem" => {
            let directory: String = d.to_member("directory")?.required()?.try_into()?;
            if directory.is_empty() {
                return Err(d
                    .to_member("directory")?
                    .required()?
                    .invalid("directory must not be empty"));
            }
            Ok(Some(DashDestination::Filesystem { directory }))
        }
        "s3" => {
            let s3 = parse_s3_fields(d)?;
            Ok(Some(DashDestination::S3 {
                bucket: s3.bucket,
                prefix: s3.prefix,
                region: s3.region,
                endpoint: s3.endpoint,
                use_path_style: s3.use_path_style,
                access_key_id: s3.access_key_id,
                secret_access_key: s3.secret_access_key,
                session_token: s3.session_token,
                lifetime_days: s3.lifetime_days,
            }))
        }
        _ => Err(d.to_member("type")?.required()?.invalid(format!(
            "destination type must be \"filesystem\" or \"s3\", got \"{dest_type}\""
        ))),
    }
}

/// S3 destination の共通フィールドをパースする
struct S3Fields {
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

fn parse_s3_fields(d: nojson::RawJsonValue<'_, '_>) -> Result<S3Fields, nojson::JsonParseError> {
    let bucket: String = d.to_member("bucket")?.required()?.try_into()?;
    if bucket.is_empty() {
        return Err(d
            .to_member("bucket")?
            .required()?
            .invalid("bucket must not be empty"));
    }
    let prefix: Option<String> = d.to_member("prefix")?.try_into()?;
    let prefix = prefix.unwrap_or_default();
    let region: String = d.to_member("region")?.required()?.try_into()?;
    if region.is_empty() {
        return Err(d
            .to_member("region")?
            .required()?
            .invalid("region must not be empty"));
    }
    let endpoint: Option<String> = d.to_member("endpoint")?.try_into()?;
    let use_path_style: Option<bool> = d.to_member("usePathStyle")?.try_into()?;

    // credentials オブジェクトのパース
    let creds_member: nojson::RawJsonOwned = d.to_member("credentials")?.required()?.try_into()?;
    let c = creds_member.value();
    let access_key_id: String = c.to_member("accessKeyId")?.required()?.try_into()?;
    let secret_access_key: String = c.to_member("secretAccessKey")?.required()?.try_into()?;
    let session_token: Option<String> = c.to_member("sessionToken")?.try_into()?;

    let lifetime_days: Option<u32> = d.to_member("lifetimeDays")?.try_into()?;
    if let Some(days) = lifetime_days {
        if days == 0 {
            return Err(d
                .to_member("lifetimeDays")?
                .required()?
                .invalid("lifetimeDays must be positive"));
        }
        if prefix.is_empty() {
            return Err(d
                .to_member("lifetimeDays")?
                .required()?
                .invalid("lifetimeDays requires a non-empty prefix"));
        }
    }

    Ok(S3Fields {
        bucket,
        prefix,
        region,
        endpoint,
        use_path_style: use_path_style.unwrap_or(false),
        access_key_id,
        secret_access_key,
        session_token,
        lifetime_days,
    })
}

// ---------------------------------------------------------------------------
// DisplayJson 実装
// ---------------------------------------------------------------------------

/// HlsDestination を credentials 込みで出力するための wrapper
struct HlsDestinationWithCredentials<'a>(&'a HlsDestination);

impl nojson::DisplayJson for HlsDestinationWithCredentials<'_> {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        self.0.fmt_with_credentials(f)
    }
}

/// DashDestination を credentials 込みで出力するための wrapper
struct DashDestinationWithCredentials<'a>(&'a DashDestination);

impl nojson::DisplayJson for DashDestinationWithCredentials<'_> {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        self.0.fmt_with_credentials(f)
    }
}

/// sora セクション: soraSdkSettings ラッパーなしで直接フィールドを出力する
struct SoraSection<'a>(&'a ObswsSoraPublisherSettings);

impl nojson::DisplayJson for SoraSection<'_> {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        let sora = self.0;
        nojson::object(|f| {
            if !sora.signaling_urls.is_empty() {
                f.member("signalingUrls", &sora.signaling_urls)?;
            }
            if let Some(channel_id) = &sora.channel_id {
                f.member("channelId", channel_id)?;
            }
            if let Some(client_id) = &sora.client_id {
                f.member("clientId", client_id)?;
            }
            if let Some(bundle_id) = &sora.bundle_id {
                f.member("bundleId", bundle_id)?;
            }
            if let Some(metadata) = &sora.metadata {
                f.member("metadata", metadata)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

/// hls セクション: destination に credentials を含めて出力する
struct HlsSection<'a>(&'a ObswsHlsSettings);

impl nojson::DisplayJson for HlsSection<'_> {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        let hls = self.0;
        nojson::object(|f| {
            if let Some(destination) = &hls.destination {
                f.member("destination", HlsDestinationWithCredentials(destination))?;
            }
            f.member("segmentDuration", hls.segment_duration)?;
            f.member("maxRetainedSegments", hls.max_retained_segments)?;
            f.member("segmentFormat", hls.segment_format.as_str())?;
            f.member(
                "variants",
                nojson::array(|f| {
                    for variant in &hls.variants {
                        f.element(variant)?;
                    }
                    Ok(())
                }),
            )
        })
        .fmt(f)
    }
}

/// mpegDash セクション: destination に credentials を含めて出力する
struct DashSection<'a>(&'a ObswsDashSettings);

impl nojson::DisplayJson for DashSection<'_> {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        let dash = self.0;
        nojson::object(|f| {
            if let Some(destination) = &dash.destination {
                f.member("destination", DashDestinationWithCredentials(destination))?;
            }
            f.member("segmentDuration", dash.segment_duration)?;
            f.member("maxRetainedSegments", dash.max_retained_segments)?;
            f.member(
                "variants",
                nojson::array(|f| {
                    for variant in &dash.variants {
                        f.element(variant)?;
                    }
                    Ok(())
                }),
            )?;
            f.member("videoCodec", dash.video_codec)?;
            f.member("audioCodec", dash.audio_codec)
        })
        .fmt(f)
    }
}

impl nojson::DisplayJson for ObswsStateFile {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("version", 1)?;
            if let Some(stream) = &self.stream {
                f.member("stream", stream)?;
            }
            if let Some(record) = &self.record {
                f.member("record", record)?;
            }
            if let Some(rtmp_outbound) = &self.rtmp_outbound {
                f.member("rtmpOutbound", rtmp_outbound)?;
            }
            if let Some(sora) = &self.sora {
                f.member("sora", SoraSection(sora))?;
            }
            if let Some(hls) = &self.hls {
                f.member("hls", HlsSection(hls))?;
            }
            if let Some(dash) = &self.dash {
                f.member("mpegDash", DashSection(dash))?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

impl nojson::DisplayJson for ObswsStateFileStream {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("streamServiceType", &self.stream_service_type)?;
            f.member(
                "streamServiceSettings",
                nojson::object(|f| {
                    if let Some(server) = &self.server {
                        f.member("server", server)?;
                    }
                    if let Some(key) = &self.key {
                        f.member("key", key)?;
                    }
                    Ok(())
                }),
            )
        })
        .fmt(f)
    }
}

impl nojson::DisplayJson for ObswsStateFileRecord {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member(
                "recordDirectory",
                self.record_directory.display().to_string(),
            )
        })
        .fmt(f)
    }
}

// ---------------------------------------------------------------------------
// 公開関数
// ---------------------------------------------------------------------------

/// state file を読み込む。
///
/// ファイルが存在しない場合は空の state を返す（初回起動対応）。
/// パースエラーや読み取り権限エラーの場合は起動エラーとする。
pub fn load_state_file(path: &Path) -> crate::Result<ObswsStateFile> {
    if !path.exists() {
        return Ok(ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: None,
        });
    }
    crate::json::parse_file(path)
}

/// state file を保存する。
///
/// 一時ファイルへ書き込み後に rename する atomic write を行う。
/// 親ディレクトリが存在しない場合は自動作成する。
// TODO: 将来的にコメント保持更新を検討する
pub fn save_state_file(path: &Path, state: &ObswsStateFile) -> crate::Result<()> {
    let content = crate::json::to_pretty_string(state);

    let dir = path
        .parent()
        .ok_or_else(|| crate::Error::new("state file path has no parent directory"))?;

    // 親ディレクトリが存在しない場合は自動作成する
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|e| {
            crate::Error::new(format!(
                "failed to create state file directory {}: {e}",
                dir.display()
            ))
        })?;
    }

    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp_path = dir.join(format!(".{file_name}.tmp.{}", std::process::id()));

    std::fs::write(&tmp_path, content.as_bytes()).map_err(|e| {
        crate::Error::new(format!(
            "failed to write temporary state file {}: {e}",
            tmp_path.display()
        ))
    })?;

    std::fs::rename(&tmp_path, path).map_err(|e| {
        // 一時ファイルのクリーンアップを試みる
        let _ = std::fs::remove_file(&tmp_path);
        crate::Error::new(format!(
            "failed to rename state file to {}: {e}",
            path.display()
        ))
    })?;

    Ok(())
}

/// ObswsInputRegistry の現在値から ObswsStateFile を構築する。
pub fn build_state_from_registry(registry: &ObswsInputRegistry) -> ObswsStateFile {
    let settings = registry.stream_service_settings();
    let stream = Some(ObswsStateFileStream {
        stream_service_type: settings.stream_service_type.clone(),
        server: settings.server.clone(),
        key: settings.key.clone(),
    });
    let record = Some(ObswsStateFileRecord {
        record_directory: registry.record_directory().to_path_buf(),
    });
    let rtmp_outbound = Some(registry.rtmp_outbound_settings().clone());
    let sora = Some(registry.sora_publisher_settings().clone());
    let hls = Some(registry.hls_settings().clone());
    let dash = Some(registry.dash_settings().clone());
    ObswsStateFile {
        stream,
        record,
        rtmp_outbound,
        sora,
        hls,
        dash,
    }
}

impl ObswsStateFileStream {
    /// ObswsStreamServiceSettings に変換する。
    pub fn to_stream_service_settings(&self) -> ObswsStreamServiceSettings {
        ObswsStreamServiceSettings {
            stream_service_type: self.stream_service_type.clone(),
            server: self.server.clone(),
            key: self.key.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_state_file() {
        let json = r#"{
            "version": 1,
            "stream": {
                "streamServiceType": "rtmp_custom",
                "streamServiceSettings": {
                    "server": "rtmp://127.0.0.1:1935/live",
                    "key": "stream-main"
                }
            },
            "record": {
                "recordDirectory": "/tmp/recordings"
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        let stream = state.stream.expect("stream must be present");
        assert_eq!(stream.stream_service_type, "rtmp_custom");
        assert_eq!(stream.server.as_deref(), Some("rtmp://127.0.0.1:1935/live"));
        assert_eq!(stream.key.as_deref(), Some("stream-main"));
        let record = state.record.expect("record must be present");
        assert_eq!(record.record_directory, PathBuf::from("/tmp/recordings"));
        // 新規 section は未指定なので None
        assert!(state.rtmp_outbound.is_none());
        assert!(state.sora.is_none());
        assert!(state.hls.is_none());
        assert!(state.dash.is_none());
    }

    #[test]
    fn parse_stream_only() {
        let json = r#"{
            "version": 1,
            "stream": {
                "streamServiceType": "rtmp_custom",
                "streamServiceSettings": {
                    "server": "rtmp://localhost/live"
                }
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        assert!(state.stream.is_some());
        assert!(state.record.is_none());
    }

    #[test]
    fn parse_record_only() {
        let json = r#"{
            "version": 1,
            "record": {
                "recordDirectory": "/tmp/rec"
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        assert!(state.stream.is_none());
        assert!(state.record.is_some());
    }

    #[test]
    fn parse_empty_state() {
        let json = r#"{ "version": 1 }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        assert!(state.stream.is_none());
        assert!(state.record.is_none());
    }

    #[test]
    fn reject_unsupported_version() {
        let json = r#"{ "version": 2 }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reject_unsupported_stream_service_type() {
        let json = r#"{
            "version": 1,
            "stream": {
                "streamServiceType": "srt_custom",
                "streamServiceSettings": {}
            }
        }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reject_record_without_record_directory() {
        let json = r#"{
            "version": 1,
            "record": {}
        }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reject_record_with_empty_record_directory() {
        let json = r#"{
            "version": 1,
            "record": {
                "recordDirectory": ""
            }
        }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    #[test]
    fn roundtrip_display_and_parse() {
        let state = ObswsStateFile {
            stream: Some(ObswsStateFileStream {
                stream_service_type: "rtmp_custom".to_owned(),
                server: Some("rtmp://127.0.0.1:1935/live".to_owned()),
                key: Some("my-key".to_owned()),
            }),
            record: Some(ObswsStateFileRecord {
                record_directory: PathBuf::from("/tmp/recordings"),
            }),
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: None,
        };

        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip parse must succeed");

        let stream = parsed.stream.expect("stream must be present");
        assert_eq!(stream.stream_service_type, "rtmp_custom");
        assert_eq!(stream.server.as_deref(), Some("rtmp://127.0.0.1:1935/live"));
        assert_eq!(stream.key.as_deref(), Some("my-key"));
        let record = parsed.record.expect("record must be present");
        assert_eq!(record.record_directory, PathBuf::from("/tmp/recordings"));
    }

    #[test]
    fn save_and_load_state_file() {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let path = dir.path().join("state.jsonc");

        let state = ObswsStateFile {
            stream: Some(ObswsStateFileStream {
                stream_service_type: "rtmp_custom".to_owned(),
                server: Some("rtmp://localhost/live".to_owned()),
                key: None,
            }),
            record: Some(ObswsStateFileRecord {
                record_directory: PathBuf::from("/tmp/rec"),
            }),
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: None,
        };

        save_state_file(&path, &state).expect("save must succeed");
        assert!(path.exists());

        let loaded: ObswsStateFile = load_state_file(&path).expect("load must succeed");
        let stream = loaded.stream.expect("stream must be present");
        assert_eq!(stream.server.as_deref(), Some("rtmp://localhost/live"));
        assert!(stream.key.is_none());
    }

    #[test]
    fn load_nonexistent_file_returns_empty_state() {
        let path = Path::new("/tmp/nonexistent-hisui-state-file-test.jsonc");
        let state = load_state_file(path).expect("load must succeed for nonexistent file");
        assert!(state.stream.is_none());
        assert!(state.record.is_none());
    }

    #[test]
    fn save_creates_parent_directories() {
        let dir = tempfile::tempdir().expect("tempdir must be created");
        let path = dir.path().join("nested").join("dir").join("state.jsonc");

        let state = ObswsStateFile {
            stream: None,
            record: Some(ObswsStateFileRecord {
                record_directory: PathBuf::from("/tmp/rec"),
            }),
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: None,
        };

        save_state_file(&path, &state).expect("save must succeed");
        assert!(path.exists());
    }

    #[test]
    fn parse_jsonc_with_comments() {
        let json = r#"{
            // state file のバージョン
            "version": 1,
            "stream": {
                "streamServiceType": "rtmp_custom",
                "streamServiceSettings": {
                    "server": "rtmp://127.0.0.1:1935/live"
                    // "key": "secret-key"
                }
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("JSONC parse must succeed");
        let stream = state.stream.expect("stream must be present");
        assert_eq!(stream.server.as_deref(), Some("rtmp://127.0.0.1:1935/live"));
        assert!(stream.key.is_none());
    }

    // --- rtmpOutbound ---

    #[test]
    fn parse_rtmp_outbound() {
        let json = r#"{
            "version": 1,
            "rtmpOutbound": {
                "outputUrl": "rtmp://relay:1935/live",
                "streamName": "backup"
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        let rtmp = state.rtmp_outbound.expect("rtmpOutbound must be present");
        assert_eq!(rtmp.output_url.as_deref(), Some("rtmp://relay:1935/live"));
        assert_eq!(rtmp.stream_name.as_deref(), Some("backup"));
    }

    #[test]
    fn roundtrip_rtmp_outbound() {
        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: Some(ObswsRtmpOutboundSettings {
                output_url: Some("rtmp://test/live".to_owned()),
                stream_name: Some("name".to_owned()),
            }),
            sora: None,
            hls: None,
            dash: None,
        };
        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip must succeed");
        let rtmp = parsed.rtmp_outbound.expect("rtmpOutbound must be present");
        assert_eq!(rtmp.output_url.as_deref(), Some("rtmp://test/live"));
        assert_eq!(rtmp.stream_name.as_deref(), Some("name"));
    }

    // --- sora ---

    #[test]
    fn parse_sora() {
        let json = r#"{
            "version": 1,
            "sora": {
                "signalingUrls": ["wss://example.com/signaling"],
                "channelId": "test-ch",
                "metadata": {"key": "value"}
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        let sora = state.sora.expect("sora must be present");
        assert_eq!(sora.signaling_urls, vec!["wss://example.com/signaling"]);
        assert_eq!(sora.channel_id.as_deref(), Some("test-ch"));
        assert!(sora.metadata.is_some());
    }

    #[test]
    fn roundtrip_sora_with_metadata() {
        let metadata = {
            let raw = nojson::RawJson::parse(r#"{"foo":"bar"}"#).expect("valid json");
            nojson::RawJsonOwned::try_from(raw.value()).expect("conversion must succeed")
        };
        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: Some(ObswsSoraPublisherSettings {
                signaling_urls: vec!["wss://s.example.com/signaling".to_owned()],
                channel_id: Some("ch".to_owned()),
                client_id: Some("cli".to_owned()),
                bundle_id: None,
                metadata: Some(metadata),
            }),
            hls: None,
            dash: None,
        };
        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip must succeed");
        let sora = parsed.sora.expect("sora must be present");
        assert_eq!(sora.signaling_urls, vec!["wss://s.example.com/signaling"]);
        assert_eq!(sora.channel_id.as_deref(), Some("ch"));
        assert_eq!(sora.client_id.as_deref(), Some("cli"));
        assert!(sora.metadata.is_some());
    }

    #[test]
    fn reject_sora_non_object_metadata() {
        let json = r#"{
            "version": 1,
            "sora": {
                "metadata": "not-an-object"
            }
        }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    // --- hls ---

    #[test]
    fn parse_hls_filesystem() {
        let json = r#"{
            "version": 1,
            "hls": {
                "destination": {
                    "type": "filesystem",
                    "directory": "/tmp/hls"
                },
                "segmentDuration": 3.0,
                "maxRetainedSegments": 10,
                "segmentFormat": "fmp4",
                "variants": [
                    {"videoBitrate": 1000000, "audioBitrate": 64000}
                ]
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        let hls = state.hls.expect("hls must be present");
        assert!(matches!(
            hls.destination,
            Some(HlsDestination::Filesystem { .. })
        ));
        assert_eq!(hls.segment_duration, 3.0);
        assert_eq!(hls.max_retained_segments, 10);
        assert_eq!(hls.segment_format, HlsSegmentFormat::Fmp4);
        assert_eq!(hls.variants.len(), 1);
        assert_eq!(hls.variants[0].video_bitrate_bps, 1_000_000);
    }

    #[test]
    fn parse_hls_s3_with_credentials() {
        let json = r#"{
            "version": 1,
            "hls": {
                "destination": {
                    "type": "s3",
                    "bucket": "my-bucket",
                    "prefix": "hls-out",
                    "region": "us-east-1",
                    "usePathStyle": false,
                    "credentials": {
                        "accessKeyId": "AKID",
                        "secretAccessKey": "SECRET",
                        "sessionToken": "TOKEN"
                    },
                    "lifetimeDays": 7
                },
                "variants": [
                    {"videoBitrate": 2000000, "audioBitrate": 128000}
                ]
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        let hls = state.hls.expect("hls must be present");
        match &hls.destination {
            Some(HlsDestination::S3 {
                bucket,
                access_key_id,
                secret_access_key,
                session_token,
                lifetime_days,
                ..
            }) => {
                assert_eq!(bucket, "my-bucket");
                assert_eq!(access_key_id, "AKID");
                assert_eq!(secret_access_key, "SECRET");
                assert_eq!(session_token.as_deref(), Some("TOKEN"));
                assert_eq!(*lifetime_days, Some(7));
            }
            _ => panic!("expected S3 destination"),
        }
    }

    #[test]
    fn roundtrip_hls_filesystem() {
        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: None,
            hls: Some(ObswsHlsSettings {
                destination: Some(HlsDestination::Filesystem {
                    directory: "/tmp/hls".to_owned(),
                }),
                segment_duration: 2.0,
                max_retained_segments: 6,
                segment_format: HlsSegmentFormat::MpegTs,
                variants: vec![HlsVariant {
                    video_bitrate_bps: 2_000_000,
                    audio_bitrate_bps: 128_000,
                    width: None,
                    height: None,
                }],
            }),
            dash: None,
        };
        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip must succeed");
        let hls = parsed.hls.expect("hls must be present");
        assert!(matches!(
            hls.destination,
            Some(HlsDestination::Filesystem { .. })
        ));
        assert_eq!(hls.variants[0].video_bitrate_bps, 2_000_000);
    }

    #[test]
    fn roundtrip_hls_s3() {
        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: None,
            hls: Some(ObswsHlsSettings {
                destination: Some(HlsDestination::S3 {
                    bucket: "bucket".to_owned(),
                    prefix: "pfx".to_owned(),
                    region: "us-west-2".to_owned(),
                    endpoint: None,
                    use_path_style: false,
                    access_key_id: "AK".to_owned(),
                    secret_access_key: "SK".to_owned(),
                    session_token: Some("ST".to_owned()),
                    lifetime_days: Some(3),
                }),
                segment_duration: 2.0,
                max_retained_segments: 6,
                segment_format: HlsSegmentFormat::MpegTs,
                variants: vec![HlsVariant::default()],
            }),
            dash: None,
        };
        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip must succeed");
        let hls = parsed.hls.expect("hls must be present");
        match &hls.destination {
            Some(HlsDestination::S3 {
                access_key_id,
                secret_access_key,
                session_token,
                ..
            }) => {
                assert_eq!(access_key_id, "AK");
                assert_eq!(secret_access_key, "SK");
                assert_eq!(session_token.as_deref(), Some("ST"));
            }
            _ => panic!("expected S3 destination"),
        }
    }

    #[test]
    fn reject_hls_empty_variants() {
        let json = r#"{
            "version": 1,
            "hls": {
                "variants": []
            }
        }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    // --- mpegDash ---

    #[test]
    fn parse_dash_filesystem() {
        let json = r#"{
            "version": 1,
            "mpegDash": {
                "destination": {
                    "type": "filesystem",
                    "directory": "/tmp/dash"
                },
                "segmentDuration": 4.0,
                "maxRetainedSegments": 8,
                "variants": [
                    {"videoBitrate": 3000000, "audioBitrate": 192000}
                ],
                "videoCodec": "H265",
                "audioCodec": "OPUS"
            }
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        let dash = state.dash.expect("dash must be present");
        assert!(matches!(
            dash.destination,
            Some(DashDestination::Filesystem { .. })
        ));
        assert_eq!(dash.segment_duration, 4.0);
        assert_eq!(dash.max_retained_segments, 8);
        assert_eq!(dash.variants[0].video_bitrate_bps, 3_000_000);
        assert_eq!(dash.video_codec, crate::types::CodecName::H265);
        assert_eq!(dash.audio_codec, crate::types::CodecName::Opus);
    }

    #[test]
    fn roundtrip_dash_s3() {
        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: Some(ObswsDashSettings {
                destination: Some(DashDestination::S3 {
                    bucket: "dash-bucket".to_owned(),
                    prefix: "dash".to_owned(),
                    region: "ap-northeast-1".to_owned(),
                    endpoint: Some("https://s3.custom.example.com".to_owned()),
                    use_path_style: true,
                    access_key_id: "DAK".to_owned(),
                    secret_access_key: "DSK".to_owned(),
                    session_token: None,
                    lifetime_days: None,
                }),
                segment_duration: 2.0,
                max_retained_segments: 6,
                variants: vec![DashVariant::default()],
                video_codec: crate::types::CodecName::H264,
                audio_codec: crate::types::CodecName::Aac,
            }),
        };
        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip must succeed");
        let dash = parsed.dash.expect("dash must be present");
        match &dash.destination {
            Some(DashDestination::S3 {
                bucket,
                access_key_id,
                endpoint,
                use_path_style,
                ..
            }) => {
                assert_eq!(bucket, "dash-bucket");
                assert_eq!(access_key_id, "DAK");
                assert_eq!(endpoint.as_deref(), Some("https://s3.custom.example.com"));
                assert!(*use_path_style);
            }
            _ => panic!("expected S3 destination"),
        }
    }

    #[test]
    fn reject_dash_empty_variants() {
        let json = r#"{
            "version": 1,
            "mpegDash": {
                "variants": []
            }
        }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }
}
