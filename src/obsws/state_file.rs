//! obsws の永続 state file の読み書きを行うモジュール。
//!
//! state file は obsws の output 設定を再起動後も復元するための JSONC ファイルである。
//! 永続化対象: stream / record / rtmp_outbound / sora / hls / mpeg_dash / scenes / inputs / persistentData

use std::path::{Path, PathBuf};

use crate::obsws::coordinator::output_registry::{
    DEFAULT_DASH_MAX_RETAINED_SEGMENTS, DEFAULT_DASH_SEGMENT_DURATION_SECS,
    DEFAULT_HLS_MAX_RETAINED_SEGMENTS, DEFAULT_HLS_SEGMENT_DURATION_SECS, DashDestination,
    DashVariant, HlsDestination, HlsSegmentFormat, HlsVariant, ObswsDashSettings, ObswsHlsSettings,
    ObswsRtmpOutboundSettings, ObswsSoraPublisherSettings, ObswsStreamServiceSettings,
};
use crate::obsws::state::{
    ObswsSceneItemBlendMode, ObswsSceneItemTransform, ObswsSessionState, ObswsSrtInboundSettings,
    ObswsWebRtcSourceSettings,
};

/// state file の output エントリ
pub struct StateFileOutput {
    pub output_name: String,
    pub output_kind: String,
    pub output_settings: nojson::RawJsonOwned,
}

/// state file のトップレベル構造
pub struct ObswsStateFile {
    pub stream: Option<ObswsStateFileStream>,
    pub record: Option<ObswsStateFileRecord>,
    pub rtmp_outbound: Option<ObswsRtmpOutboundSettings>,
    pub sora: Option<ObswsSoraPublisherSettings>,
    pub hls: Option<ObswsHlsSettings>,
    pub dash: Option<ObswsDashSettings>,
    /// 動的 output リスト（新形式）
    pub outputs: Option<Vec<StateFileOutput>>,
    pub scenes: Option<Vec<StateFileScene>>,
    pub inputs: Option<Vec<StateFileInput>>,
    pub current_program_scene: Option<String>,
    pub current_preview_scene: Option<String>,
    pub next_input_id: Option<u64>,
    pub next_scene_id: Option<u64>,
    pub next_scene_item_id: Option<i64>,
    pub persistent_data: Option<std::collections::BTreeMap<String, nojson::RawJsonOwned>>,
}

/// state file のシーン定義
pub struct StateFileScene {
    pub scene_name: String,
    pub scene_uuid: String,
    pub items: Vec<StateFileSceneItem>,
    pub transition_override: Option<StateFileTransitionOverride>,
}

/// state file のシーンアイテム定義
pub struct StateFileSceneItem {
    pub scene_item_id: i64,
    pub input_uuid: String,
    pub enabled: bool,
    pub locked: bool,
    pub blend_mode: String,
    pub transform: ObswsSceneItemTransform,
}

/// state file のトランジションオーバーライド
pub struct StateFileTransitionOverride {
    pub transition_name: Option<String>,
    pub transition_duration: Option<i64>,
}

/// state file のインプット定義
pub struct StateFileInput {
    pub input_uuid: String,
    pub input_name: String,
    pub input_kind: String,
    pub input_settings: nojson::RawJsonOwned,
    pub input_muted: bool,
    pub input_volume_mul: f64,
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

        // outputs セクション
        let outputs = parse_optional_outputs(value)?;

        // scene / input セクション
        let scenes = parse_optional_scenes(value)?;
        let inputs = parse_optional_inputs(value)?;
        let current_program_scene: Option<String> =
            value.to_member("currentProgramScene")?.try_into()?;
        let current_preview_scene: Option<String> =
            value.to_member("currentPreviewScene")?.try_into()?;
        let next_input_id: Option<u64> = value.to_member("nextInputId")?.try_into()?;
        let next_scene_id: Option<u64> = value.to_member("nextSceneId")?.try_into()?;
        let next_scene_item_id: Option<i64> = value.to_member("nextSceneItemId")?.try_into()?;

        // persistentData セクション: { slotName: slotValue, ... } 形式のオブジェクト
        let persistent_data = parse_optional_persistent_data(value)?;

        Ok(Self {
            stream,
            record,
            rtmp_outbound,
            sora,
            hls,
            dash,
            outputs,
            scenes,
            inputs,
            current_program_scene,
            current_preview_scene,
            next_input_id,
            next_scene_id,
            next_scene_item_id,
            persistent_data,
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
    let segment_duration = segment_duration.unwrap_or(DEFAULT_HLS_SEGMENT_DURATION_SECS);
    // NaN / Infinity は JSON 仕様上パーサが弾くため、ここでは正値チェックのみ行う
    if segment_duration <= 0.0 {
        return Err(v
            .to_member("segmentDuration")?
            .required()?
            .invalid("segmentDuration must be positive"));
    }

    let max_retained_segments: Option<usize> = v.to_member("maxRetainedSegments")?.try_into()?;
    let max_retained_segments = max_retained_segments.unwrap_or(DEFAULT_HLS_MAX_RETAINED_SEGMENTS);
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
    let arr = variants_json.value().to_array()?;
    let mut variants = Vec::new();
    for elem in arr {
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
    let segment_duration = segment_duration.unwrap_or(DEFAULT_DASH_SEGMENT_DURATION_SECS);
    // NaN / Infinity は JSON 仕様上パーサが弾くため、ここでは正値チェックのみ行う
    if segment_duration <= 0.0 {
        return Err(v
            .to_member("segmentDuration")?
            .required()?
            .invalid("segmentDuration must be positive"));
    }

    let max_retained_segments: Option<usize> = v.to_member("maxRetainedSegments")?.try_into()?;
    let max_retained_segments = max_retained_segments.unwrap_or(DEFAULT_DASH_MAX_RETAINED_SEGMENTS);
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

fn parse_optional_outputs(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<Vec<StateFileOutput>>, nojson::JsonParseError> {
    let member: Option<nojson::RawJsonOwned> = value.to_member("outputs")?.try_into()?;
    let Some(ref section) = member else {
        return Ok(None);
    };
    let arr = section.value().to_array()?;
    let mut outputs = Vec::new();
    for item in arr {
        let output_name: String = item.to_member("outputName")?.required()?.try_into()?;
        let output_kind: String = item.to_member("outputKind")?.required()?.try_into()?;
        let output_settings: nojson::RawJsonOwned = item
            .to_member("outputSettings")?
            .required()?
            .extract()
            .into_owned();
        outputs.push(StateFileOutput {
            output_name,
            output_kind,
            output_settings,
        });
    }
    Ok(Some(outputs))
}

fn parse_dash_variants(
    v: nojson::RawJsonValue<'_, '_>,
) -> Result<Vec<DashVariant>, nojson::JsonParseError> {
    let variants_member: Option<nojson::RawJsonOwned> = v.to_member("variants")?.try_into()?;
    let Some(ref variants_json) = variants_member else {
        return Ok(vec![DashVariant::default()]);
    };
    let arr = variants_json.value().to_array()?;
    let mut variants = Vec::new();
    for elem in arr {
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
// scenes parse
// ---------------------------------------------------------------------------

fn parse_optional_scenes(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<Vec<StateFileScene>>, nojson::JsonParseError> {
    let member: Option<nojson::RawJsonOwned> = value.to_member("scenes")?.try_into()?;
    let Some(ref scenes_json) = member else {
        return Ok(None);
    };
    let arr = scenes_json.value().to_array()?;
    let mut scenes = Vec::new();
    for elem in arr {
        let scene_name: String = elem.to_member("sceneName")?.required()?.try_into()?;
        let scene_uuid: String = elem.to_member("sceneUuid")?.required()?.try_into()?;
        let items = parse_scene_items(elem)?;
        let transition_override = parse_optional_transition_override(elem)?;
        scenes.push(StateFileScene {
            scene_name,
            scene_uuid,
            items,
            transition_override,
        });
    }
    Ok(Some(scenes))
}

fn parse_scene_items(
    scene_value: nojson::RawJsonValue<'_, '_>,
) -> Result<Vec<StateFileSceneItem>, nojson::JsonParseError> {
    // items は省略可能。省略時は空配列として扱う。
    let member: Option<nojson::RawJsonOwned> = scene_value.to_member("items")?.try_into()?;
    let Some(ref items_json) = member else {
        return Ok(Vec::new());
    };
    let arr = items_json.value().to_array()?;
    let mut items = Vec::new();
    for elem in arr {
        let scene_item_id: i64 = elem.to_member("sceneItemId")?.required()?.try_into()?;
        let input_uuid: String = elem.to_member("inputUuid")?.required()?.try_into()?;
        let enabled: bool = elem.to_member("enabled")?.required()?.try_into()?;
        let locked: bool = elem.to_member("locked")?.required()?.try_into()?;
        let blend_mode: String = elem.to_member("blendMode")?.required()?.try_into()?;
        // blend_mode のバリデーション
        if ObswsSceneItemBlendMode::parse(&blend_mode).is_none() {
            return Err(elem
                .to_member("blendMode")?
                .required()?
                .invalid(format!("unknown blend mode: \"{blend_mode}\"")));
        }
        let transform_member: nojson::RawJsonOwned =
            elem.to_member("transform")?.required()?.try_into()?;
        let transform = parse_scene_item_transform(transform_member.value())?;
        items.push(StateFileSceneItem {
            scene_item_id,
            input_uuid,
            enabled,
            locked,
            blend_mode,
            transform,
        });
    }
    Ok(items)
}

fn parse_scene_item_transform(
    v: nojson::RawJsonValue<'_, '_>,
) -> Result<ObswsSceneItemTransform, nojson::JsonParseError> {
    let position_x: f64 = v.to_member("positionX")?.required()?.try_into()?;
    let position_y: f64 = v.to_member("positionY")?.required()?.try_into()?;
    let rotation: f64 = v.to_member("rotation")?.required()?.try_into()?;
    let scale_x_raw: f64 = v.to_member("scaleX")?.required()?.try_into()?;
    let scale_x = crate::types::PositiveFiniteF64::new(scale_x_raw).ok_or_else(|| {
        v.to_member("scaleX")
            .expect("already accessed")
            .required()
            .expect("already accessed")
            .invalid("scaleX must be a positive finite number")
    })?;
    let scale_y_raw: f64 = v.to_member("scaleY")?.required()?.try_into()?;
    let scale_y = crate::types::PositiveFiniteF64::new(scale_y_raw).ok_or_else(|| {
        v.to_member("scaleY")
            .expect("already accessed")
            .required()
            .expect("already accessed")
            .invalid("scaleY must be a positive finite number")
    })?;
    let alignment: i64 = v.to_member("alignment")?.required()?.try_into()?;
    let bounds_type: String = v.to_member("boundsType")?.required()?.try_into()?;
    let bounds_alignment: i64 = v.to_member("boundsAlignment")?.required()?.try_into()?;
    let bounds_width: f64 = v.to_member("boundsWidth")?.required()?.try_into()?;
    let bounds_height: f64 = v.to_member("boundsHeight")?.required()?.try_into()?;
    let crop_top: i64 = v.to_member("cropTop")?.required()?.try_into()?;
    let crop_bottom: i64 = v.to_member("cropBottom")?.required()?.try_into()?;
    let crop_left: i64 = v.to_member("cropLeft")?.required()?.try_into()?;
    let crop_right: i64 = v.to_member("cropRight")?.required()?.try_into()?;
    let crop_to_bounds: bool = v.to_member("cropToBounds")?.required()?.try_into()?;
    let source_width: f64 = v.to_member("sourceWidth")?.required()?.try_into()?;
    let source_height: f64 = v.to_member("sourceHeight")?.required()?.try_into()?;
    let width: f64 = v.to_member("width")?.required()?.try_into()?;
    let height: f64 = v.to_member("height")?.required()?.try_into()?;
    Ok(ObswsSceneItemTransform {
        position_x,
        position_y,
        rotation,
        scale_x,
        scale_y,
        alignment,
        bounds_type,
        bounds_alignment,
        bounds_width,
        bounds_height,
        crop_top,
        crop_bottom,
        crop_left,
        crop_right,
        crop_to_bounds,
        source_width,
        source_height,
        width,
        height,
    })
}

fn parse_optional_transition_override(
    scene_value: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<StateFileTransitionOverride>, nojson::JsonParseError> {
    let member: Option<nojson::RawJsonOwned> =
        scene_value.to_member("transitionOverride")?.try_into()?;
    let Some(ref override_json) = member else {
        return Ok(None);
    };
    let v = override_json.value();
    let transition_name: Option<String> = v.to_member("transitionName")?.try_into()?;
    let transition_duration: Option<i64> = v.to_member("transitionDuration")?.try_into()?;
    Ok(Some(StateFileTransitionOverride {
        transition_name,
        transition_duration,
    }))
}

// ---------------------------------------------------------------------------
// inputs parse
// ---------------------------------------------------------------------------

fn parse_optional_inputs(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<Vec<StateFileInput>>, nojson::JsonParseError> {
    let member: Option<nojson::RawJsonOwned> = value.to_member("inputs")?.try_into()?;
    let Some(ref inputs_json) = member else {
        return Ok(None);
    };
    let arr = inputs_json.value().to_array()?;
    let mut inputs = Vec::new();
    for elem in arr {
        let input_uuid: String = elem.to_member("inputUuid")?.required()?.try_into()?;
        let input_name: String = elem.to_member("inputName")?.required()?.try_into()?;
        let input_kind: String = elem.to_member("inputKind")?.required()?.try_into()?;
        let input_settings: nojson::RawJsonOwned =
            elem.to_member("inputSettings")?.required()?.try_into()?;
        let input_muted: Option<bool> = elem.to_member("inputMuted")?.try_into()?;
        let input_muted = input_muted.unwrap_or(false);
        let input_volume_mul: Option<crate::types::NonNegFiniteF64> =
            elem.to_member("inputVolumeMul")?.try_into()?;
        let input_volume_mul = input_volume_mul
            .unwrap_or(crate::types::NonNegFiniteF64::ONE)
            .get();
        inputs.push(StateFileInput {
            input_uuid,
            input_name,
            input_kind,
            input_settings,
            input_muted,
            input_volume_mul,
        });
    }
    Ok(Some(inputs))
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

/// persistentData セクションをパースする。
/// 各メンバーの値を RawJsonOwned としてそのまま保持する。
/// null 値のスロットはスキップする（API からは null を書き込めないため、
/// state file 手動編集で混入した場合に「存在しない」と同等に扱う）。
fn parse_optional_persistent_data(
    v: nojson::RawJsonValue<'_, '_>,
) -> Result<Option<std::collections::BTreeMap<String, nojson::RawJsonOwned>>, nojson::JsonParseError>
{
    let member: Option<nojson::RawJsonOwned> = v.to_member("persistentData")?.try_into()?;
    let Some(ref obj_json) = member else {
        return Ok(None);
    };
    let mut map = std::collections::BTreeMap::new();
    for (key, value) in obj_json.value().to_object()? {
        if value.kind().is_null() {
            continue;
        }
        let slot_name: String = key.to_unquoted_string_str()?.into_owned();
        let raw = nojson::RawJsonOwned::try_from(value)?;
        map.insert(slot_name, raw);
    }
    Ok(Some(map))
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
    if access_key_id.is_empty() {
        return Err(c
            .to_member("accessKeyId")?
            .required()?
            .invalid("accessKeyId must not be empty"));
    }
    let secret_access_key: String = c.to_member("secretAccessKey")?.required()?.try_into()?;
    if secret_access_key.is_empty() {
        return Err(c
            .to_member("secretAccessKey")?
            .required()?
            .invalid("secretAccessKey must not be empty"));
    }
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
            // 空配列は省略する。読み戻し時は unwrap_or_default() で空 Vec に復元されるため同義。
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

/// SRT inbound settings を passphrase 込みで出力するための wrapper
///
/// 通常の DisplayJson は passphrase を省略するが、state file には永続化する必要がある
struct SrtInboundSettingsWithPassphrase<'a>(&'a ObswsSrtInboundSettings);

impl nojson::DisplayJson for SrtInboundSettingsWithPassphrase<'_> {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        let s = self.0;
        nojson::object(|f| {
            if let Some(input_url) = &s.input_url {
                f.member("inputUrl", input_url)?;
            }
            if let Some(stream_id) = &s.stream_id {
                f.member("streamId", stream_id)?;
            }
            if let Some(passphrase) = &s.passphrase {
                f.member("passphrase", passphrase)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

/// WebRTC source settings を track_id なしで出力するための wrapper
///
/// track_id はランタイム専用のため state file には含めない
struct WebRtcSourceSettingsWithoutTrackId<'a>(&'a ObswsWebRtcSourceSettings);

impl nojson::DisplayJson for WebRtcSourceSettingsWithoutTrackId<'_> {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        let s = self.0;
        nojson::object(|f| {
            if let Some(background_key_color) = &s.background_key_color {
                f.member("backgroundKeyColor", background_key_color)?;
            }
            if let Some(background_key_tolerance) = s.background_key_tolerance {
                f.member(
                    "backgroundKeyTolerance",
                    i64::from(background_key_tolerance),
                )?;
            }
            Ok(())
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
            if let Some(outputs) = &self.outputs {
                f.member(
                    "outputs",
                    nojson::array(|f| {
                        for output in outputs {
                            f.element(output)?;
                        }
                        Ok(())
                    }),
                )?;
            }
            if let Some(scenes) = &self.scenes {
                f.member(
                    "scenes",
                    nojson::array(|f| {
                        for scene in scenes {
                            f.element(scene)?;
                        }
                        Ok(())
                    }),
                )?;
            }
            if let Some(inputs) = &self.inputs {
                f.member(
                    "inputs",
                    nojson::array(|f| {
                        for input in inputs {
                            f.element(input)?;
                        }
                        Ok(())
                    }),
                )?;
            }
            if let Some(current_program_scene) = &self.current_program_scene {
                f.member("currentProgramScene", current_program_scene)?;
            }
            if let Some(current_preview_scene) = &self.current_preview_scene {
                f.member("currentPreviewScene", current_preview_scene)?;
            }
            if let Some(next_input_id) = self.next_input_id {
                f.member("nextInputId", next_input_id)?;
            }
            if let Some(next_scene_id) = self.next_scene_id {
                f.member("nextSceneId", next_scene_id)?;
            }
            if let Some(next_scene_item_id) = self.next_scene_item_id {
                f.member("nextSceneItemId", next_scene_item_id)?;
            }
            if let Some(persistent_data) = &self.persistent_data
                && !persistent_data.is_empty()
            {
                f.member(
                    "persistentData",
                    nojson::object(|f| {
                        for (key, value) in persistent_data {
                            f.member(key, value)?;
                        }
                        Ok(())
                    }),
                )?;
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

impl nojson::DisplayJson for StateFileOutput {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("outputName", &self.output_name)?;
            f.member("outputKind", &self.output_kind)?;
            f.member("outputSettings", &self.output_settings)
        })
        .fmt(f)
    }
}

impl nojson::DisplayJson for StateFileScene {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("sceneName", &self.scene_name)?;
            f.member("sceneUuid", &self.scene_uuid)?;
            f.member(
                "items",
                nojson::array(|f| {
                    for item in &self.items {
                        f.element(item)?;
                    }
                    Ok(())
                }),
            )?;
            if let Some(transition_override) = &self.transition_override {
                f.member("transitionOverride", transition_override)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

impl nojson::DisplayJson for StateFileSceneItem {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("sceneItemId", self.scene_item_id)?;
            f.member("inputUuid", &self.input_uuid)?;
            f.member("enabled", self.enabled)?;
            f.member("locked", self.locked)?;
            f.member("blendMode", &self.blend_mode)?;
            f.member("transform", &self.transform)
        })
        .fmt(f)
    }
}

impl nojson::DisplayJson for StateFileTransitionOverride {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(transition_name) = &self.transition_name {
                f.member("transitionName", transition_name)?;
            }
            if let Some(transition_duration) = self.transition_duration {
                f.member("transitionDuration", transition_duration)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

impl nojson::DisplayJson for StateFileInput {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("inputUuid", &self.input_uuid)?;
            f.member("inputName", &self.input_name)?;
            f.member("inputKind", &self.input_kind)?;
            f.member("inputSettings", &self.input_settings)?;
            f.member("inputMuted", self.input_muted)?;
            f.member("inputVolumeMul", self.input_volume_mul)
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
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
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

/// ObswsSessionState と outputs BTreeMap の現在値から ObswsStateFile を構築する。
pub(crate) fn build_state_from_registry(
    registry: &ObswsSessionState,
    outputs_map: &std::collections::BTreeMap<
        String,
        crate::obsws::coordinator::output_registry::OutputState,
    >,
) -> ObswsStateFile {
    use crate::obsws::coordinator::output_registry::OutputSettings;

    // outputs BTreeMap から outputs セクションを構築する
    // ビルトイン output（Player 等）は永続設定を持たないためスキップする
    let outputs = {
        let mut output_list = Vec::new();
        for (name, state) in outputs_map {
            let settings_json = match &state.settings {
                OutputSettings::Stream(s) => crate::json::to_pretty_string(s),
                OutputSettings::Record(s) => crate::json::to_pretty_string(s),
                OutputSettings::Hls(s) => crate::json::to_pretty_string(s),
                OutputSettings::MpegDash(s) => crate::json::to_pretty_string(s),
                OutputSettings::RtmpOutbound(s) => crate::json::to_pretty_string(s),
                OutputSettings::Sora(s) => crate::json::to_pretty_string(s),
                #[cfg(feature = "player")]
                OutputSettings::Player => continue,
            };
            let raw =
                nojson::RawJson::parse(&settings_json).expect("serialized JSON must be valid");
            let output_settings = nojson::RawJsonOwned::try_from(raw.value())
                .expect("RawJsonOwned conversion must succeed");
            output_list.push(StateFileOutput {
                output_name: name.clone(),
                output_kind: state.output_kind.as_kind_str().to_owned(),
                output_settings,
            });
        }
        Some(output_list)
    };

    // scenes を scene_order の順序で構築する
    let scenes = {
        let mut scene_list = Vec::new();
        for scene_name in &registry.scene_order {
            let Some(scene_state) = registry.scenes_by_name.get(scene_name) else {
                // scene_order と scenes_by_name の不整合は実装バグ
                tracing::warn!(
                    "scene_order contains \"{}\" but scenes_by_name does not; skipping",
                    scene_name
                );
                continue;
            };
            let items = scene_state
                .items
                .iter()
                .map(|item| StateFileSceneItem {
                    scene_item_id: item.scene_item_id,
                    input_uuid: item.input_uuid.clone(),
                    enabled: item.enabled,
                    locked: item.locked,
                    blend_mode: item.blend_mode.as_str().to_owned(),
                    transform: item.transform.clone(),
                })
                .collect();
            let transition_override =
                registry
                    .scene_transition_overrides
                    .get(scene_name)
                    .map(|o| StateFileTransitionOverride {
                        transition_name: o.transition_name.clone(),
                        transition_duration: o.transition_duration,
                    });
            scene_list.push(StateFileScene {
                scene_name: scene_name.clone(),
                scene_uuid: scene_state.scene_uuid.clone(),
                items,
                transition_override,
            });
        }
        Some(scene_list)
    };

    // inputs を構築する
    let inputs = {
        let mut input_list = Vec::new();
        for (uuid, entry) in &registry.inputs_by_uuid {
            // input settings を適切な DisplayJson でシリアライズする
            let settings_json = match &entry.input.settings {
                crate::obsws::state::ObswsInputSettings::SrtInbound(srt) => {
                    // SRT inbound は passphrase を含めて出力する
                    crate::json::to_pretty_string(SrtInboundSettingsWithPassphrase(srt))
                }
                crate::obsws::state::ObswsInputSettings::WebRtcSource(webrtc) => {
                    // WebRTC source は track_id を除外する
                    crate::json::to_pretty_string(WebRtcSourceSettingsWithoutTrackId(webrtc))
                }
                other => crate::json::to_pretty_string(other),
            };
            let raw =
                nojson::RawJson::parse(&settings_json).expect("serialized JSON must be valid");
            let input_settings = nojson::RawJsonOwned::try_from(raw.value())
                .expect("RawJsonOwned conversion must succeed");
            input_list.push(StateFileInput {
                input_uuid: uuid.clone(),
                input_name: entry.input_name.clone(),
                input_kind: entry.input.kind_name().to_owned(),
                input_settings,
                input_muted: entry.input.input_muted,
                input_volume_mul: entry.input.input_volume_mul.get(),
            });
        }
        Some(input_list)
    };

    let current_program_scene = Some(registry.current_program_scene_name.clone());
    let current_preview_scene = Some(registry.current_preview_scene_name.clone());
    let next_input_id = Some(registry.next_input_id);
    let next_scene_id = Some(registry.next_scene_id);
    let next_scene_item_id = Some(registry.next_scene_item_id);

    ObswsStateFile {
        stream: None,
        record: None,
        rtmp_outbound: None,
        sora: None,
        hls: None,
        dash: None,
        outputs,
        scenes,
        inputs,
        current_program_scene,
        current_preview_scene,
        next_input_id,
        next_scene_id,
        next_scene_item_id,
        persistent_data: {
            let data = registry.persistent_data();
            if data.is_empty() {
                None
            } else {
                Some(data.clone())
            }
        },
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
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
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
    fn roundtrip_all_sections() {
        let state = ObswsStateFile {
            stream: Some(ObswsStateFileStream {
                stream_service_type: "rtmp_custom".to_owned(),
                server: Some("rtmp://127.0.0.1:1935/live".to_owned()),
                key: Some("key".to_owned()),
            }),
            record: Some(ObswsStateFileRecord {
                record_directory: PathBuf::from("/tmp/rec"),
            }),
            rtmp_outbound: Some(ObswsRtmpOutboundSettings {
                output_url: Some("rtmp://relay/live".to_owned()),
                stream_name: Some("backup".to_owned()),
            }),
            sora: Some(ObswsSoraPublisherSettings {
                signaling_urls: vec!["wss://s.example.com/signaling".to_owned()],
                channel_id: Some("ch".to_owned()),
                client_id: None,
                bundle_id: None,
                metadata: None,
            }),
            hls: Some(ObswsHlsSettings {
                destination: Some(HlsDestination::Filesystem {
                    directory: "/tmp/hls".to_owned(),
                }),
                segment_duration: 2.0,
                max_retained_segments: 6,
                segment_format: HlsSegmentFormat::MpegTs,
                variants: vec![HlsVariant::default()],
            }),
            dash: Some(ObswsDashSettings {
                destination: Some(DashDestination::Filesystem {
                    directory: "/tmp/dash".to_owned(),
                }),
                segment_duration: 2.0,
                max_retained_segments: 6,
                variants: vec![DashVariant::default()],
                video_codec: crate::types::CodecName::H264,
                audio_codec: crate::types::CodecName::Aac,
            }),
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
        };

        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip must succeed");

        let stream = parsed.stream.expect("stream must be present");
        assert_eq!(stream.key.as_deref(), Some("key"));

        let record = parsed.record.expect("record must be present");
        assert_eq!(record.record_directory, PathBuf::from("/tmp/rec"));

        let rtmp = parsed.rtmp_outbound.expect("rtmpOutbound must be present");
        assert_eq!(rtmp.output_url.as_deref(), Some("rtmp://relay/live"));
        assert_eq!(rtmp.stream_name.as_deref(), Some("backup"));

        let sora = parsed.sora.expect("sora must be present");
        assert_eq!(sora.signaling_urls, vec!["wss://s.example.com/signaling"]);
        assert_eq!(sora.channel_id.as_deref(), Some("ch"));

        let hls = parsed.hls.expect("hls must be present");
        assert!(
            matches!(hls.destination, Some(HlsDestination::Filesystem { ref directory }) if directory == "/tmp/hls")
        );
        assert_eq!(hls.segment_duration, 2.0);

        let dash = parsed.dash.expect("dash must be present");
        assert!(
            matches!(dash.destination, Some(DashDestination::Filesystem { ref directory }) if directory == "/tmp/dash")
        );
        assert_eq!(dash.video_codec, crate::types::CodecName::H264);
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
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
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
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
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
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
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
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
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
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
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
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
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
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
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

    // --- scenes / inputs ---

    #[test]
    fn parse_scenes_with_items() {
        let json = r#"{
            "version": 1,
            "scenes": [
                {
                    "sceneName": "Main",
                    "sceneUuid": "uuid-scene-1",
                    "items": [
                        {
                            "sceneItemId": 1,
                            "inputUuid": "uuid-input-1",
                            "enabled": true,
                            "locked": false,
                            "blendMode": "OBS_BLEND_NORMAL",
                            "transform": {
                                "positionX": 0.0,
                                "positionY": 0.0,
                                "rotation": 0.0,
                                "scaleX": 1.0,
                                "scaleY": 1.0,
                                "alignment": 5,
                                "boundsType": "OBS_BOUNDS_NONE",
                                "boundsAlignment": 0,
                                "boundsWidth": 0.0,
                                "boundsHeight": 0.0,
                                "cropTop": 0,
                                "cropBottom": 0,
                                "cropLeft": 0,
                                "cropRight": 0,
                                "cropToBounds": false,
                                "sourceWidth": 1920.0,
                                "sourceHeight": 1080.0,
                                "width": 1920.0,
                                "height": 1080.0
                            }
                        }
                    ]
                }
            ]
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        let scenes = state.scenes.expect("scenes must be present");
        assert_eq!(scenes.len(), 1);
        assert_eq!(scenes[0].scene_name, "Main");
        assert_eq!(scenes[0].scene_uuid, "uuid-scene-1");
        assert_eq!(scenes[0].items.len(), 1);
        assert_eq!(scenes[0].items[0].scene_item_id, 1);
        assert_eq!(scenes[0].items[0].input_uuid, "uuid-input-1");
        assert!(scenes[0].items[0].enabled);
        assert!(!scenes[0].items[0].locked);
        assert_eq!(scenes[0].items[0].blend_mode, "OBS_BLEND_NORMAL");
        assert_eq!(scenes[0].items[0].transform.source_width, 1920.0);
    }

    #[test]
    fn roundtrip_scenes_with_items() {
        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: None,
            outputs: None,
            scenes: Some(vec![StateFileScene {
                scene_name: "Scene1".to_owned(),
                scene_uuid: "uuid-s1".to_owned(),
                items: vec![StateFileSceneItem {
                    scene_item_id: 42,
                    input_uuid: "uuid-i1".to_owned(),
                    enabled: true,
                    locked: true,
                    blend_mode: "OBS_BLEND_ADDITIVE".to_owned(),
                    transform: ObswsSceneItemTransform::default(),
                }],
                transition_override: None,
            }]),
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
        };
        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip must succeed");
        let scenes = parsed.scenes.expect("scenes must be present");
        assert_eq!(scenes.len(), 1);
        assert_eq!(scenes[0].scene_name, "Scene1");
        assert_eq!(scenes[0].items[0].scene_item_id, 42);
        assert_eq!(scenes[0].items[0].blend_mode, "OBS_BLEND_ADDITIVE");
        assert!(scenes[0].items[0].locked);
    }

    #[test]
    fn parse_inputs_all_kinds() {
        // 8 つの input kind すべてをパースできることを確認する
        let json = r##"{
            "version": 1,
            "inputs": [
                {
                    "inputUuid": "uuid-1",
                    "inputName": "Image",
                    "inputKind": "image_source",
                    "inputSettings": {"file": "/tmp/test.png"}
                },
                {
                    "inputUuid": "uuid-2",
                    "inputName": "Camera",
                    "inputKind": "video_capture_device",
                    "inputSettings": {"device_id": "cam0"}
                },
                {
                    "inputUuid": "uuid-3",
                    "inputName": "Mic",
                    "inputKind": "audio_capture_device",
                    "inputSettings": {"device_id": "mic0"}
                },
                {
                    "inputUuid": "uuid-4",
                    "inputName": "MP4",
                    "inputKind": "mp4_file_source",
                    "inputSettings": {"path": "/tmp/video.mp4", "loopPlayback": true}
                },
                {
                    "inputUuid": "uuid-5",
                    "inputName": "RTMP",
                    "inputKind": "rtmp_inbound",
                    "inputSettings": {"inputUrl": "rtmp://localhost/live", "streamName": "test"}
                },
                {
                    "inputUuid": "uuid-6",
                    "inputName": "SRT",
                    "inputKind": "srt_inbound",
                    "inputSettings": {"inputUrl": "srt://localhost:9000", "passphrase": "secret12"}
                },
                {
                    "inputUuid": "uuid-7",
                    "inputName": "RTSP",
                    "inputKind": "rtsp_subscriber",
                    "inputSettings": {"inputUrl": "rtsp://localhost/stream"}
                },
                {
                    "inputUuid": "uuid-8",
                    "inputName": "WebRTC",
                    "inputKind": "webrtc_source",
                    "inputSettings": {"backgroundKeyColor": "#00FF00"}
                }
            ]
        }"##;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        let inputs = state.inputs.expect("inputs must be present");
        assert_eq!(inputs.len(), 8);
        assert_eq!(inputs[0].input_kind, "image_source");
        assert_eq!(inputs[1].input_kind, "video_capture_device");
        assert_eq!(inputs[2].input_kind, "audio_capture_device");
        assert_eq!(inputs[3].input_kind, "mp4_file_source");
        assert_eq!(inputs[4].input_kind, "rtmp_inbound");
        assert_eq!(inputs[5].input_kind, "srt_inbound");
        assert_eq!(inputs[6].input_kind, "rtsp_subscriber");
        assert_eq!(inputs[7].input_kind, "webrtc_source");
    }

    #[test]
    fn roundtrip_inputs() {
        let settings_json = r#"{"inputUrl": "rtmp://test/live"}"#;
        let raw = nojson::RawJson::parse(settings_json).expect("valid json");
        let input_settings =
            nojson::RawJsonOwned::try_from(raw.value()).expect("conversion must succeed");

        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: None,
            outputs: None,
            scenes: None,
            inputs: Some(vec![StateFileInput {
                input_uuid: "uuid-1".to_owned(),
                input_name: "TestInput".to_owned(),
                input_kind: "rtmp_inbound".to_owned(),
                input_settings,
                input_muted: false,
                input_volume_mul: 1.0,
            }]),
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: Some(5),
            next_scene_id: Some(3),
            next_scene_item_id: Some(10),
            persistent_data: None,
        };
        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip must succeed");
        let inputs = parsed.inputs.expect("inputs must be present");
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].input_uuid, "uuid-1");
        assert_eq!(inputs[0].input_name, "TestInput");
        assert_eq!(inputs[0].input_kind, "rtmp_inbound");
        assert_eq!(parsed.next_input_id, Some(5));
        assert_eq!(parsed.next_scene_id, Some(3));
        assert_eq!(parsed.next_scene_item_id, Some(10));
    }

    #[test]
    fn roundtrip_scene_item_transform() {
        use crate::types::PositiveFiniteF64;

        let transform = ObswsSceneItemTransform {
            position_x: 100.0,
            position_y: 200.0,
            rotation: 45.0,
            scale_x: PositiveFiniteF64::new(2.0).expect("valid"),
            scale_y: PositiveFiniteF64::new(0.5).expect("valid"),
            alignment: 5,
            bounds_type: "OBS_BOUNDS_STRETCH".to_owned(),
            bounds_alignment: 0,
            bounds_width: 1920.0,
            bounds_height: 1080.0,
            crop_top: 10,
            crop_bottom: 20,
            crop_left: 30,
            crop_right: 40,
            crop_to_bounds: true,
            source_width: 1920.0,
            source_height: 1080.0,
            width: 3840.0,
            height: 540.0,
        };

        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: None,
            outputs: None,
            scenes: Some(vec![StateFileScene {
                scene_name: "TransformTest".to_owned(),
                scene_uuid: "uuid-tf".to_owned(),
                items: vec![StateFileSceneItem {
                    scene_item_id: 1,
                    input_uuid: "uuid-input".to_owned(),
                    enabled: true,
                    locked: false,
                    blend_mode: "OBS_BLEND_NORMAL".to_owned(),
                    transform: transform.clone(),
                }],
                transition_override: None,
            }]),
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
        };
        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip must succeed");
        let scenes = parsed.scenes.expect("scenes must be present");
        let parsed_transform = &scenes[0].items[0].transform;
        assert_eq!(parsed_transform.position_x, 100.0);
        assert_eq!(parsed_transform.position_y, 200.0);
        assert_eq!(parsed_transform.rotation, 45.0);
        assert_eq!(
            parsed_transform.scale_x,
            PositiveFiniteF64::new(2.0).expect("valid")
        );
        assert_eq!(
            parsed_transform.scale_y,
            PositiveFiniteF64::new(0.5).expect("valid")
        );
        assert_eq!(parsed_transform.bounds_type, "OBS_BOUNDS_STRETCH");
        assert_eq!(parsed_transform.bounds_width, 1920.0);
        assert_eq!(parsed_transform.crop_top, 10);
        assert_eq!(parsed_transform.crop_bottom, 20);
        assert_eq!(parsed_transform.crop_left, 30);
        assert_eq!(parsed_transform.crop_right, 40);
        assert!(parsed_transform.crop_to_bounds);
        assert_eq!(parsed_transform.width, 3840.0);
        assert_eq!(parsed_transform.height, 540.0);
    }

    #[test]
    fn parse_scene_with_transition_override() {
        let json = r#"{
            "version": 1,
            "scenes": [
                {
                    "sceneName": "WithOverride",
                    "sceneUuid": "uuid-override",
                    "items": [],
                    "transitionOverride": {
                        "transitionName": "Cut",
                        "transitionDuration": 500
                    }
                }
            ]
        }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        let scenes = state.scenes.expect("scenes must be present");
        assert_eq!(scenes.len(), 1);
        let to = scenes[0]
            .transition_override
            .as_ref()
            .expect("transitionOverride must be present");
        assert_eq!(to.transition_name.as_deref(), Some("Cut"));
        assert_eq!(to.transition_duration, Some(500));
    }

    #[test]
    fn roundtrip_scene_with_transition_override() {
        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: None,
            outputs: None,
            scenes: Some(vec![StateFileScene {
                scene_name: "OverrideScene".to_owned(),
                scene_uuid: "uuid-os".to_owned(),
                items: Vec::new(),
                transition_override: Some(StateFileTransitionOverride {
                    transition_name: Some("Fade".to_owned()),
                    transition_duration: Some(300),
                }),
            }]),
            inputs: None,
            current_program_scene: Some("OverrideScene".to_owned()),
            current_preview_scene: Some("OverrideScene".to_owned()),
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
        };
        let json_text = crate::json::to_pretty_string(&state);
        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip must succeed");
        let scenes = parsed.scenes.expect("scenes must be present");
        let to = scenes[0]
            .transition_override
            .as_ref()
            .expect("transitionOverride must be present");
        assert_eq!(to.transition_name.as_deref(), Some("Fade"));
        assert_eq!(to.transition_duration, Some(300));
        assert_eq!(
            parsed.current_program_scene.as_deref(),
            Some("OverrideScene")
        );
        assert_eq!(
            parsed.current_preview_scene.as_deref(),
            Some("OverrideScene")
        );
    }

    #[test]
    fn reject_invalid_blend_mode() {
        let json = r#"{
            "version": 1,
            "scenes": [
                {
                    "sceneName": "Bad",
                    "sceneUuid": "uuid-bad",
                    "items": [
                        {
                            "sceneItemId": 1,
                            "inputUuid": "uuid-input",
                            "enabled": true,
                            "locked": false,
                            "blendMode": "OBS_BLEND_INVALID",
                            "transform": {
                                "positionX": 0.0,
                                "positionY": 0.0,
                                "rotation": 0.0,
                                "scaleX": 1.0,
                                "scaleY": 1.0,
                                "alignment": 5,
                                "boundsType": "OBS_BOUNDS_NONE",
                                "boundsAlignment": 0,
                                "boundsWidth": 0.0,
                                "boundsHeight": 0.0,
                                "cropTop": 0,
                                "cropBottom": 0,
                                "cropLeft": 0,
                                "cropRight": 0,
                                "cropToBounds": false,
                                "sourceWidth": 0.0,
                                "sourceHeight": 0.0,
                                "width": 0.0,
                                "height": 0.0
                            }
                        }
                    ]
                }
            ]
        }"#;
        let result = crate::json::parse_str::<ObswsStateFile>(json);
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_state_has_none_for_new_fields() {
        let json = r#"{ "version": 1 }"#;
        let state: ObswsStateFile = crate::json::parse_str(json).expect("parse must succeed");
        assert!(state.scenes.is_none());
        assert!(state.inputs.is_none());
        assert!(state.current_program_scene.is_none());
        assert!(state.current_preview_scene.is_none());
        assert!(state.next_input_id.is_none());
        assert!(state.next_scene_id.is_none());
        assert!(state.next_scene_item_id.is_none());
    }

    #[test]
    fn roundtrip_srt_inbound_with_passphrase() {
        // SrtInboundSettingsWithPassphrase が passphrase を含めて出力することを検証する
        let srt = ObswsSrtInboundSettings {
            input_url: Some("srt://127.0.0.1:9000".to_owned()),
            stream_id: Some("test".to_owned()),
            passphrase: Some("my-secret-pass".to_owned()),
        };
        let json_text =
            crate::json::to_pretty_string(super::SrtInboundSettingsWithPassphrase(&srt));
        assert!(json_text.contains("passphrase"));
        assert!(json_text.contains("my-secret-pass"));

        // 通常の DisplayJson では passphrase が含まれないことを確認する
        let normal_text = crate::json::to_pretty_string(&srt);
        assert!(!normal_text.contains("passphrase"));
    }

    #[test]
    fn roundtrip_webrtc_source_without_track_id() {
        // WebRtcSourceSettingsWithoutTrackId が track_id を除外することを検証する
        let webrtc = ObswsWebRtcSourceSettings {
            track_id: Some("runtime-track-id".to_owned()),
            background_key_color: Some("#00FF00".to_owned()),
            background_key_tolerance: Some(30),
        };
        let json_text =
            crate::json::to_pretty_string(super::WebRtcSourceSettingsWithoutTrackId(&webrtc));
        assert!(!json_text.contains("trackId"));
        assert!(!json_text.contains("runtime-track-id"));
        assert!(json_text.contains("backgroundKeyColor"));
        assert!(json_text.contains("#00FF00"));
    }

    #[test]
    fn roundtrip_persistent_data() {
        let mut persistent_data = std::collections::BTreeMap::new();
        let val1 = nojson::RawJson::parse(r#"{"key": "value", "num": 42}"#).expect("valid json");
        persistent_data.insert(
            "slot1".to_owned(),
            nojson::RawJsonOwned::try_from(val1.value()).expect("conversion must succeed"),
        );
        let val2 = nojson::RawJson::parse(r#""hello""#).expect("valid json");
        persistent_data.insert(
            "slot2".to_owned(),
            nojson::RawJsonOwned::try_from(val2.value()).expect("conversion must succeed"),
        );

        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: None,
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: Some(persistent_data),
        };

        let json_text = crate::json::to_pretty_string(&state);
        assert!(json_text.contains("persistentData"));
        assert!(json_text.contains("slot1"));
        assert!(json_text.contains("slot2"));

        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip parse must succeed");
        let data = parsed
            .persistent_data
            .expect("persistent_data must be present");
        assert_eq!(data.len(), 2);
        assert!(data.contains_key("slot1"));
        assert!(data.contains_key("slot2"));

        // slot1 の値がオブジェクトであることを確認
        let slot1_text = crate::json::to_pretty_string(data.get("slot1").unwrap());
        assert!(slot1_text.contains("\"key\""));
        assert!(slot1_text.contains("\"value\""));
        assert!(slot1_text.contains("42"));

        // slot2 の値が文字列であることを確認
        let slot2_text = crate::json::to_pretty_string(data.get("slot2").unwrap());
        assert!(slot2_text.contains("hello"));
    }

    #[test]
    fn roundtrip_persistent_data_empty() {
        let state = ObswsStateFile {
            stream: None,
            record: None,
            rtmp_outbound: None,
            sora: None,
            hls: None,
            dash: None,
            outputs: None,
            scenes: None,
            inputs: None,
            current_program_scene: None,
            current_preview_scene: None,
            next_input_id: None,
            next_scene_id: None,
            next_scene_item_id: None,
            persistent_data: None,
        };

        let json_text = crate::json::to_pretty_string(&state);
        // persistent_data が None の場合はフィールド自体を出力しない
        assert!(!json_text.contains("persistentData"));

        let parsed: ObswsStateFile =
            crate::json::parse_str(&json_text).expect("roundtrip parse must succeed");
        assert!(parsed.persistent_data.is_none());
    }

    #[test]
    fn restore_sora_output_preserves_metadata() {
        use crate::obsws::coordinator::output_registry::{
            OutputSettings, restore_outputs_from_state,
        };

        let state_outputs = vec![StateFileOutput {
            output_name: "sora_with_meta".to_owned(),
            output_kind: "sora_webrtc_output".to_owned(),
            output_settings: nojson::RawJsonOwned::parse(
                r#"{"soraSdkSettings":{"signalingUrls":["wss://example.com/signaling"],"channelId":"ch","metadata":{"key":"value"}}}"#,
            )
            .expect("settings json must be valid"),
        }];
        let outputs = restore_outputs_from_state(state_outputs).expect("restore must succeed");
        let state = outputs
            .get("sora_with_meta")
            .expect("sora output must exist");
        let OutputSettings::Sora(sora_settings) = &state.settings else {
            panic!("expected Sora settings");
        };
        assert_eq!(
            sora_settings.signaling_urls,
            vec!["wss://example.com/signaling"]
        );
        assert_eq!(sora_settings.channel_id.as_deref(), Some("ch"));
        let metadata = sora_settings
            .metadata
            .as_ref()
            .expect("metadata must be present");
        let key: String = metadata
            .value()
            .to_member("key")
            .expect("key access must succeed")
            .required()
            .expect("key must be present")
            .try_into()
            .expect("key must be string");
        assert_eq!(key, "value");
    }

    #[test]
    fn default_record_directory_uses_record_output_not_custom_mp4() {
        // record と別名の mp4_output が異なる recordDirectory を持つ state file から
        // 復元した場合、record 側の値が default_record_directory に使われるべき。
        // server.rs の復元ロジックと同じ抽出ルールをテストする。
        let state_outputs = [
            // カスタム mp4_output（先に来る）
            StateFileOutput {
                output_name: "custom_record".to_owned(),
                output_kind: "mp4_output".to_owned(),
                output_settings: nojson::RawJsonOwned::parse(
                    r#"{"recordDirectory":"/tmp/custom-recordings"}"#,
                )
                .expect("settings json must be valid"),
            },
            // 標準の record output
            StateFileOutput {
                output_name: "record".to_owned(),
                output_kind: "mp4_output".to_owned(),
                output_settings: nojson::RawJsonOwned::parse(
                    r#"{"recordDirectory":"/tmp/standard-recordings"}"#,
                )
                .expect("settings json must be valid"),
            },
        ];

        // server.rs と同じロジック: outputName == "record" && outputKind == "mp4_output" を優先
        let record_dir = state_outputs
            .iter()
            .find(|o| o.output_name == "record" && o.output_kind == "mp4_output")
            .and_then(|o| {
                o.output_settings
                    .value()
                    .to_member("recordDirectory")
                    .ok()
                    .and_then(|v| v.optional())
                    .and_then(|v| <Option<String>>::try_from(v).ok().flatten())
                    .map(std::path::PathBuf::from)
            });

        assert_eq!(
            record_dir,
            Some(std::path::PathBuf::from("/tmp/standard-recordings")),
        );
    }

    #[test]
    fn default_record_directory_falls_back_when_no_record_output() {
        // record output がない場合、カスタム mp4_output からは逆算しない
        let state_outputs = [StateFileOutput {
            output_name: "custom_only".to_owned(),
            output_kind: "mp4_output".to_owned(),
            output_settings: nojson::RawJsonOwned::parse(
                r#"{"recordDirectory":"/tmp/custom-only"}"#,
            )
            .expect("settings json must be valid"),
        }];

        let record_dir = state_outputs
            .iter()
            .find(|o| o.output_name == "record" && o.output_kind == "mp4_output")
            .and_then(|o| {
                o.output_settings
                    .value()
                    .to_member("recordDirectory")
                    .ok()
                    .and_then(|v| v.optional())
                    .and_then(|v| <Option<String>>::try_from(v).ok().flatten())
                    .map(std::path::PathBuf::from)
            });

        // record がないので None → CLI 既定値にフォールバック
        assert!(record_dir.is_none());
    }

    #[test]
    fn restore_outputs_from_state_fails_on_invalid_output_settings() {
        use crate::obsws::coordinator::output_registry::restore_outputs_from_state;

        let state_outputs = vec![StateFileOutput {
            output_name: "broken_stream".to_owned(),
            output_kind: "rtmp_output".to_owned(),
            output_settings: nojson::RawJsonOwned::parse(
                r#"{"streamServiceType":123,"streamServiceSettings":{"server":"rtmp://example.com/live"}}"#,
            )
            .expect("settings json must be valid"),
        }];

        let error = match restore_outputs_from_state(state_outputs) {
            Ok(_) => panic!("restore must fail for invalid state"),
            Err(error) => error,
        };
        let message = error.display().to_string();
        assert!(message.contains("broken_stream"));
        assert!(message.contains("streamServiceType must be a string"));
    }
}
