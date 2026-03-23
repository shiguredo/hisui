use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use crate::types::PositiveFiniteF64;
use crate::{ProcessorId, TrackId};

pub(crate) const OBSWS_SUPPORTED_INPUT_KINDS: [&str; 6] = [
    "image_source",
    "video_capture_device",
    "mp4_file_source",
    "rtmp_inbound",
    "srt_inbound",
    "rtsp_subscriber",
];
pub(crate) const OBSWS_SUPPORTED_TRANSITION_KINDS: [&str; 7] = [
    "fade_transition",
    "cut_transition",
    "swipe_transition",
    "slide_transition",
    "obs_stinger_transition",
    "fade_to_color_transition",
    "wipe_transition",
];
pub(crate) const OBSWS_MAX_INPUT_ID_FOR_UUID_SUFFIX: u64 = (1 << 48) - 1;
pub(crate) const OBSWS_MAX_SCENE_ID_FOR_UUID_SUFFIX: u64 = (1 << 48) - 1;
pub(crate) const OBSWS_DEFAULT_STREAM_SERVICE_TYPE: &str = "rtmp_custom";
pub(crate) const OBSWS_DEFAULT_TRANSITION_NAME: &str = "fade_transition";
pub(crate) const OBSWS_DEFAULT_TRANSITION_DURATION_MS: i64 = 500;
pub(crate) const OBSWS_DEFAULT_TBAR_POSITION: f64 = 0.0;
pub(crate) const OBSWS_MIN_TRANSITION_DURATION_MS: i64 = 50;
pub(crate) const OBSWS_MAX_TRANSITION_DURATION_MS: i64 = 20_000;
pub(crate) const OBSWS_MIN_TBAR_POSITION: f64 = 0.0;
pub(crate) const OBSWS_MAX_TBAR_POSITION: f64 = 1.0;

#[derive(Debug, Clone)]
pub struct ObswsSceneInputEntry {
    pub input: ObswsInputEntry,
    pub scene_item_index: usize,
    pub transform: ObswsSceneItemTransform,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsInputEntry {
    pub input_uuid: String,
    pub input_name: String,
    pub input: ObswsInput,
}

impl ObswsInputEntry {
    #[cfg(test)]
    pub fn new_for_test(
        input_uuid: impl Into<String>,
        input_name: impl Into<String>,
        input: ObswsInput,
    ) -> Self {
        Self {
            input_uuid: input_uuid.into(),
            input_name: input_name.into(),
            input,
        }
    }
}

impl nojson::DisplayJson for ObswsInputEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("inputName", &self.input_name)?;
            f.member("inputKind", self.input.kind_name())?;
            // 現状の hisui は OBS の *_v2 / *_v3 のようなバージョン付き input kind を
            // 使っていないため、unversionedInputKind は inputKind と同値になる。
            f.member("unversionedInputKind", self.input.kind_name())?;
            f.member("inputUuid", &self.input_uuid)?;
            f.member("inputKindCaps", 0)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsInput {
    pub settings: ObswsInputSettings,
}

impl ObswsInput {
    pub fn from_kind_and_settings(
        input_kind: &str,
        input_settings: nojson::RawJsonValue<'_, '_>,
    ) -> Result<Self, ParseInputSettingsError> {
        Ok(Self {
            settings: ObswsInputSettings::from_kind_and_settings(input_kind, input_settings)?,
        })
    }

    pub fn kind_name(&self) -> &'static str {
        self.settings.kind_name()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObswsInputSettings {
    ImageSource(ObswsImageSourceSettings),
    VideoCaptureDevice(ObswsVideoCaptureDeviceSettings),
    Mp4FileSource(ObswsMp4FileSourceSettings),
    RtmpInbound(ObswsRtmpInboundSettings),
    SrtInbound(ObswsSrtInboundSettings),
    RtspSubscriber(ObswsRtspSubscriberSettings),
}

impl ObswsInputSettings {
    pub fn default_for_kind(input_kind: &str) -> Result<Self, ParseInputSettingsError> {
        match input_kind {
            "image_source" => Ok(Self::ImageSource(ObswsImageSourceSettings::default())),
            "video_capture_device" => Ok(Self::VideoCaptureDevice(
                ObswsVideoCaptureDeviceSettings::default(),
            )),
            "mp4_file_source" => Ok(Self::Mp4FileSource(ObswsMp4FileSourceSettings::default())),
            "rtmp_inbound" => Ok(Self::RtmpInbound(ObswsRtmpInboundSettings::default())),
            "srt_inbound" => Ok(Self::SrtInbound(ObswsSrtInboundSettings::default())),
            "rtsp_subscriber" => Ok(Self::RtspSubscriber(ObswsRtspSubscriberSettings::default())),
            _ => Err(ParseInputSettingsError::UnsupportedInputKind),
        }
    }

    pub fn from_kind_and_settings(
        input_kind: &str,
        input_settings: nojson::RawJsonValue<'_, '_>,
    ) -> Result<Self, ParseInputSettingsError> {
        if input_settings.kind() != nojson::JsonValueKind::Object {
            return Err(ParseInputSettingsError::InvalidInputSettings(
                "Invalid inputSettings field: object is required".to_owned(),
            ));
        }

        match input_kind {
            "image_source" => {
                let file = parse_optional_string_setting(input_settings, "file")?;
                Ok(Self::ImageSource(ObswsImageSourceSettings { file }))
            }
            "video_capture_device" => {
                let device_id = parse_optional_string_setting(input_settings, "device_id")?;
                Ok(Self::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                    device_id,
                }))
            }
            "mp4_file_source" => {
                let path = parse_optional_string_setting(input_settings, "path")?;
                let loop_playback = parse_optional_bool_setting(input_settings, "loopPlayback")?;
                Ok(Self::Mp4FileSource(ObswsMp4FileSourceSettings {
                    path,
                    loop_playback: loop_playback.unwrap_or(false),
                }))
            }
            "rtmp_inbound" => {
                let input_url = parse_optional_string_setting(input_settings, "inputUrl")?;
                let stream_name = parse_optional_string_setting(input_settings, "streamName")?;
                Ok(Self::RtmpInbound(ObswsRtmpInboundSettings {
                    input_url,
                    stream_name,
                }))
            }
            "srt_inbound" => {
                let input_url = parse_optional_string_setting(input_settings, "inputUrl")?;
                let stream_id = parse_optional_string_setting(input_settings, "streamId")?;
                let passphrase = parse_optional_string_setting(input_settings, "passphrase")?;
                Ok(Self::SrtInbound(ObswsSrtInboundSettings {
                    input_url,
                    stream_id,
                    passphrase,
                }))
            }
            "rtsp_subscriber" => {
                let input_url = parse_optional_string_setting(input_settings, "inputUrl")?;
                Ok(Self::RtspSubscriber(ObswsRtspSubscriberSettings {
                    input_url,
                }))
            }
            _ => Err(ParseInputSettingsError::UnsupportedInputKind),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::ImageSource(_) => "image_source",
            // TODO: `video_capture_device` は将来的に `video_device_source` へ rename して、
            // `*_source` 命名へ統一する。今回は既存 API 影響を避けるため据え置く。
            Self::VideoCaptureDevice(_) => "video_capture_device",
            Self::Mp4FileSource(_) => "mp4_file_source",
            Self::RtmpInbound(_) => "rtmp_inbound",
            Self::SrtInbound(_) => "srt_inbound",
            Self::RtspSubscriber(_) => "rtsp_subscriber",
        }
    }

    pub fn overlay_with_settings(
        &self,
        input_settings: nojson::RawJsonValue<'_, '_>,
    ) -> Result<Self, ParseInputSettingsError> {
        if input_settings.kind() != nojson::JsonValueKind::Object {
            return Err(ParseInputSettingsError::InvalidInputSettings(
                "Invalid inputSettings field: object is required".to_owned(),
            ));
        }

        match self {
            Self::ImageSource(existing) => {
                let file = parse_overlay_string_setting(input_settings, "file", &existing.file)?;
                Ok(Self::ImageSource(ObswsImageSourceSettings { file }))
            }
            Self::VideoCaptureDevice(existing) => {
                let device_id =
                    parse_overlay_string_setting(input_settings, "device_id", &existing.device_id)?;
                Ok(Self::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                    device_id,
                }))
            }
            Self::Mp4FileSource(existing) => {
                let path = parse_overlay_string_setting(input_settings, "path", &existing.path)?;
                let loop_playback = parse_overlay_bool_setting(
                    input_settings,
                    "loopPlayback",
                    existing.loop_playback,
                )?;
                Ok(Self::Mp4FileSource(ObswsMp4FileSourceSettings {
                    path,
                    loop_playback,
                }))
            }
            Self::RtmpInbound(existing) => {
                let input_url =
                    parse_overlay_string_setting(input_settings, "inputUrl", &existing.input_url)?;
                let stream_name = parse_overlay_string_setting(
                    input_settings,
                    "streamName",
                    &existing.stream_name,
                )?;
                Ok(Self::RtmpInbound(ObswsRtmpInboundSettings {
                    input_url,
                    stream_name,
                }))
            }
            Self::SrtInbound(existing) => {
                let input_url =
                    parse_overlay_string_setting(input_settings, "inputUrl", &existing.input_url)?;
                let stream_id =
                    parse_overlay_string_setting(input_settings, "streamId", &existing.stream_id)?;
                let passphrase = parse_overlay_string_setting(
                    input_settings,
                    "passphrase",
                    &existing.passphrase,
                )?;
                Ok(Self::SrtInbound(ObswsSrtInboundSettings {
                    input_url,
                    stream_id,
                    passphrase,
                }))
            }
            Self::RtspSubscriber(existing) => {
                let input_url =
                    parse_overlay_string_setting(input_settings, "inputUrl", &existing.input_url)?;
                Ok(Self::RtspSubscriber(ObswsRtspSubscriberSettings {
                    input_url,
                }))
            }
        }
    }
}

impl nojson::DisplayJson for ObswsInputSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::ImageSource(settings) => settings.fmt(f),
            Self::VideoCaptureDevice(settings) => settings.fmt(f),
            Self::Mp4FileSource(settings) => settings.fmt(f),
            Self::RtmpInbound(settings) => settings.fmt(f),
            Self::SrtInbound(settings) => settings.fmt(f),
            Self::RtspSubscriber(settings) => settings.fmt(f),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsSceneEntry {
    pub scene_index: usize,
    pub scene_name: String,
    pub scene_uuid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsSceneTransitionOverride {
    pub transition_name: Option<String>,
    pub transition_duration: Option<i64>,
}

impl nojson::DisplayJson for ObswsSceneTransitionOverride {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("transitionName", &self.transition_name)?;
            f.member("transitionDuration", self.transition_duration)
        })
        .fmt(f)
    }
}

impl nojson::DisplayJson for ObswsSceneEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("sceneIndex", self.scene_index)?;
            f.member("sceneName", &self.scene_name)?;
            f.member("sceneUuid", &self.scene_uuid)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsStreamServiceSettings {
    pub stream_service_type: String,
    pub server: Option<String>,
    pub key: Option<String>,
}

impl Default for ObswsStreamServiceSettings {
    fn default() -> Self {
        Self {
            stream_service_type: OBSWS_DEFAULT_STREAM_SERVICE_TYPE.to_owned(),
            server: None,
            key: None,
        }
    }
}

impl nojson::DisplayJson for ObswsStreamServiceSettings {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsStreamRun {
    pub source_processor_ids: Vec<ProcessorId>,
    pub video: ObswsRecordTrackRun,
    pub audio: ObswsRecordTrackRun,
    pub audio_mixer_processor_id: ProcessorId,
    pub video_mixer_processor_id: ProcessorId,
    pub publisher_processor_id: ProcessorId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsRtmpOutboundRun {
    pub source_processor_ids: Vec<ProcessorId>,
    pub video: ObswsRecordTrackRun,
    pub audio: ObswsRecordTrackRun,
    pub audio_mixer_processor_id: ProcessorId,
    pub video_mixer_processor_id: ProcessorId,
    pub endpoint_processor_id: ProcessorId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsRecordRun {
    pub source_processor_ids: Vec<ProcessorId>,
    pub video: ObswsRecordTrackRun,
    pub audio: ObswsRecordTrackRun,
    pub audio_mixer_processor_id: ProcessorId,
    pub video_mixer_processor_id: ProcessorId,
    pub writer_processor_id: ProcessorId,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsRecordTrackRun {
    pub encoder_processor_id: ProcessorId,
    pub source_track_id: TrackId,
    pub encoded_track_id: TrackId,
}

impl ObswsRecordTrackRun {
    /// output_kind ("stream" / "record") と media_kind ("video" / "audio") から構築する
    pub fn new(
        output_kind: &str,
        run_id: u64,
        media_kind: &str,
        source_track_id: &TrackId,
    ) -> Self {
        Self {
            encoder_processor_id: ProcessorId::new(format!(
                "obsws:{output_kind}:{run_id}:{media_kind}_encoder"
            )),
            source_track_id: source_track_id.clone(),
            encoded_track_id: TrackId::new(format!(
                "obsws:{output_kind}:{run_id}:encoded_{media_kind}"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObswsSceneItemBlendMode {
    #[default]
    Normal,
    Additive,
    Subtract,
    Screen,
    Multiply,
    Lighten,
    Darken,
}

impl ObswsSceneItemBlendMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "OBS_BLEND_NORMAL",
            Self::Additive => "OBS_BLEND_ADDITIVE",
            Self::Subtract => "OBS_BLEND_SUBTRACT",
            Self::Screen => "OBS_BLEND_SCREEN",
            Self::Multiply => "OBS_BLEND_MULTIPLY",
            Self::Lighten => "OBS_BLEND_LIGHTEN",
            Self::Darken => "OBS_BLEND_DARKEN",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "OBS_BLEND_NORMAL" => Some(Self::Normal),
            "OBS_BLEND_ADDITIVE" => Some(Self::Additive),
            "OBS_BLEND_SUBTRACT" => Some(Self::Subtract),
            "OBS_BLEND_SCREEN" => Some(Self::Screen),
            "OBS_BLEND_MULTIPLY" => Some(Self::Multiply),
            "OBS_BLEND_LIGHTEN" => Some(Self::Lighten),
            "OBS_BLEND_DARKEN" => Some(Self::Darken),
            _ => None,
        }
    }
}

impl nojson::DisplayJson for ObswsSceneItemBlendMode {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsSceneItemTransform {
    pub position_x: f64,
    pub position_y: f64,
    pub rotation: f64,
    pub scale_x: PositiveFiniteF64,
    pub scale_y: PositiveFiniteF64,
    pub alignment: i64,
    pub bounds_type: String,
    pub bounds_alignment: i64,
    pub bounds_width: f64,
    pub bounds_height: f64,
    pub crop_top: i64,
    pub crop_bottom: i64,
    pub crop_left: i64,
    pub crop_right: i64,
    pub crop_to_bounds: bool,
    pub source_width: f64,
    pub source_height: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for ObswsSceneItemTransform {
    fn default() -> Self {
        Self {
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            scale_x: PositiveFiniteF64::ONE,
            scale_y: PositiveFiniteF64::ONE,
            alignment: 5,
            bounds_type: "OBS_BOUNDS_NONE".to_owned(),
            bounds_alignment: 0,
            bounds_width: 0.0,
            bounds_height: 0.0,
            crop_top: 0,
            crop_bottom: 0,
            crop_left: 0,
            crop_right: 0,
            crop_to_bounds: false,
            source_width: 0.0,
            source_height: 0.0,
            width: 0.0,
            height: 0.0,
        }
    }
}

impl nojson::DisplayJson for ObswsSceneItemTransform {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("positionX", self.position_x)?;
            f.member("positionY", self.position_y)?;
            f.member("rotation", self.rotation)?;
            f.member("scaleX", self.scale_x)?;
            f.member("scaleY", self.scale_y)?;
            f.member("alignment", self.alignment)?;
            f.member("boundsType", &self.bounds_type)?;
            f.member("boundsAlignment", self.bounds_alignment)?;
            f.member("boundsWidth", self.bounds_width)?;
            f.member("boundsHeight", self.bounds_height)?;
            f.member("cropTop", self.crop_top)?;
            f.member("cropBottom", self.crop_bottom)?;
            f.member("cropLeft", self.crop_left)?;
            f.member("cropRight", self.crop_right)?;
            f.member("cropToBounds", self.crop_to_bounds)?;
            f.member("sourceWidth", self.source_width)?;
            f.member("sourceHeight", self.source_height)?;
            f.member("width", self.width)?;
            f.member("height", self.height)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObswsSceneItemTransformPatch {
    pub position_x: Option<f64>,
    pub position_y: Option<f64>,
    pub rotation: Option<f64>,
    pub scale_x: Option<PositiveFiniteF64>,
    pub scale_y: Option<PositiveFiniteF64>,
    pub alignment: Option<i64>,
    pub bounds_type: Option<String>,
    pub bounds_alignment: Option<i64>,
    pub bounds_width: Option<f64>,
    pub bounds_height: Option<f64>,
    pub crop_top: Option<i64>,
    pub crop_bottom: Option<i64>,
    pub crop_left: Option<i64>,
    pub crop_right: Option<i64>,
    pub crop_to_bounds: Option<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct ObswsSceneItemState {
    pub(crate) scene_item_id: i64,
    pub(crate) input_uuid: String,
    pub(crate) enabled: bool,
    pub(crate) locked: bool,
    pub(crate) blend_mode: ObswsSceneItemBlendMode,
    pub(crate) transform: ObswsSceneItemTransform,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsSceneItemEntry {
    pub scene_item_id: i64,
    pub source_name: String,
    pub source_uuid: String,
    pub input_kind: String,
    pub source_type: String,
    pub scene_item_enabled: bool,
    pub scene_item_locked: bool,
    pub scene_item_blend_mode: String,
    pub scene_item_index: i64,
    pub scene_item_transform: ObswsSceneItemTransform,
    pub is_group: Option<bool>,
}

impl nojson::DisplayJson for ObswsSceneItemEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("sceneItemId", self.scene_item_id)?;
            f.member("sourceName", &self.source_name)?;
            f.member("sourceUuid", &self.source_uuid)?;
            f.member("inputKind", &self.input_kind)?;
            f.member("sourceType", &self.source_type)?;
            f.member("sceneItemEnabled", self.scene_item_enabled)?;
            f.member("sceneItemLocked", self.scene_item_locked)?;
            f.member("sceneItemBlendMode", &self.scene_item_blend_mode)?;
            f.member("sceneItemIndex", self.scene_item_index)?;
            f.member("sceneItemTransform", &self.scene_item_transform)?;
            f.member("isGroup", self.is_group)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsSceneItemRef {
    pub scene_name: String,
    pub scene_uuid: String,
    pub scene_item: ObswsSceneItemEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSceneItemIndexResult {
    pub changed: bool,
    pub scene_items: Vec<ObswsSceneItemIndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsSceneItemIndexEntry {
    pub scene_item_id: i64,
    pub scene_item_index: i64,
}

impl nojson::DisplayJson for ObswsSceneItemIndexEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("sceneItemId", self.scene_item_id)?;
            f.member("sceneItemIndex", self.scene_item_index)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ObswsSceneState {
    pub(crate) scene_uuid: String,
    pub(crate) items: Vec<ObswsSceneItemState>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ObswsStreamRuntimeState {
    pub(crate) active: bool,
    pub(crate) started_at: Option<Instant>,
    pub(crate) run: Option<ObswsStreamRun>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ObswsRtmpOutboundRuntimeState {
    pub(crate) active: bool,
    pub(crate) started_at: Option<Instant>,
    pub(crate) run: Option<ObswsRtmpOutboundRun>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ObswsRecordRuntimeState {
    pub(crate) active: bool,
    pub(crate) started_at: Option<Instant>,
    pub(crate) run: Option<ObswsRecordRun>,
}

#[derive(Debug, Clone)]
pub(crate) struct ObswsTransitionRuntimeState {
    pub(crate) current_transition_name: String,
    pub(crate) current_transition_duration_ms: i64,
    pub(crate) current_transition_settings: Option<nojson::RawJsonOwned>,
    pub(crate) current_tbar_position: f64,
}

impl Default for ObswsTransitionRuntimeState {
    fn default() -> Self {
        Self {
            current_transition_name: OBSWS_DEFAULT_TRANSITION_NAME.to_owned(),
            current_transition_duration_ms: OBSWS_DEFAULT_TRANSITION_DURATION_MS,
            current_transition_settings: None,
            current_tbar_position: OBSWS_DEFAULT_TBAR_POSITION,
        }
    }
}

fn parse_optional_string_setting(
    settings: nojson::RawJsonValue<'_, '_>,
    key: &str,
) -> Result<Option<String>, ParseInputSettingsError> {
    let Some(value) = settings
        .to_member(key)
        .map_err(|e| {
            ParseInputSettingsError::InvalidInputSettings(format!(
                "Invalid inputSettings field: {e}"
            ))
        })?
        .optional()
    else {
        return Ok(None);
    };

    if value.kind() != nojson::JsonValueKind::String {
        return Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: string is required"
        )));
    }
    let value: String = value.try_into().map_err(|e| {
        ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: {e}"
        ))
    })?;
    Ok(Some(value))
}

fn parse_overlay_string_setting(
    settings: nojson::RawJsonValue<'_, '_>,
    key: &str,
    current: &Option<String>,
) -> Result<Option<String>, ParseInputSettingsError> {
    let Some(value) = settings
        .to_member(key)
        .map_err(|e| {
            ParseInputSettingsError::InvalidInputSettings(format!(
                "Invalid inputSettings field: {e}"
            ))
        })?
        .optional()
    else {
        return Ok(current.clone());
    };

    if value.kind() != nojson::JsonValueKind::String {
        return Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: string is required"
        )));
    }
    let value: String = value.try_into().map_err(|e| {
        ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: {e}"
        ))
    })?;
    Ok(Some(value))
}

fn parse_optional_bool_setting(
    settings: nojson::RawJsonValue<'_, '_>,
    key: &str,
) -> Result<Option<bool>, ParseInputSettingsError> {
    let Some(value) = settings
        .to_member(key)
        .map_err(|e| {
            ParseInputSettingsError::InvalidInputSettings(format!(
                "Invalid inputSettings field: {e}"
            ))
        })?
        .optional()
    else {
        return Ok(None);
    };

    if value.kind() != nojson::JsonValueKind::Boolean {
        return Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: boolean is required"
        )));
    }
    let value: bool = value.try_into().map_err(|e| {
        ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: {e}"
        ))
    })?;
    Ok(Some(value))
}

fn parse_overlay_bool_setting(
    settings: nojson::RawJsonValue<'_, '_>,
    key: &str,
    current: bool,
) -> Result<bool, ParseInputSettingsError> {
    let Some(value) = settings
        .to_member(key)
        .map_err(|e| {
            ParseInputSettingsError::InvalidInputSettings(format!(
                "Invalid inputSettings field: {e}"
            ))
        })?
        .optional()
    else {
        return Ok(current);
    };

    if value.kind() != nojson::JsonValueKind::Boolean {
        return Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: boolean is required"
        )));
    }
    value.try_into().map_err(|e| {
        ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: {e}"
        ))
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsImageSourceSettings {
    // OBS 互換のため、image_source は file 未指定の状態も有効として扱う
    pub file: Option<String>,
}

impl nojson::DisplayJson for ObswsImageSourceSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(file) = &self.file {
                f.member("file", file)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsVideoCaptureDeviceSettings {
    // OBS 互換のため、video_capture_device は device_id 未指定の状態も有効として扱う
    pub device_id: Option<String>,
}

impl nojson::DisplayJson for ObswsVideoCaptureDeviceSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(device_id) = &self.device_id {
                f.member("device_id", device_id)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsMp4FileSourceSettings {
    // OBS 互換ではなく hisui 独自 input として扱うため、path 未指定も保持可能にする。
    // 実行時には path 必須とする。
    pub path: Option<String>,
    pub loop_playback: bool,
}

impl nojson::DisplayJson for ObswsMp4FileSourceSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(path) = &self.path {
                f.member("path", path)?;
            }
            f.member("loopPlayback", self.loop_playback)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsRtmpInboundSettings {
    // 録画・配信開始時に必須。登録時点では未指定も許容する。
    pub input_url: Option<String>,
    pub stream_name: Option<String>,
}

impl nojson::DisplayJson for ObswsRtmpInboundSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(input_url) = &self.input_url {
                f.member("inputUrl", input_url)?;
            }
            if let Some(stream_name) = &self.stream_name {
                f.member("streamName", stream_name)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsSrtInboundSettings {
    // 録画・配信開始時に必須。登録時点では未指定も許容する。
    pub input_url: Option<String>,
    pub stream_id: Option<String>,
    pub passphrase: Option<String>,
}

impl nojson::DisplayJson for ObswsSrtInboundSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(input_url) = &self.input_url {
                f.member("inputUrl", input_url)?;
            }
            if let Some(stream_id) = &self.stream_id {
                f.member("streamId", stream_id)?;
            }
            // passphrase はセキュリティ上の理由で GetInputSettings に含めない
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsRtspSubscriberSettings {
    // 録画・配信開始時に必須。登録時点では未指定も許容する。
    pub input_url: Option<String>,
}

impl nojson::DisplayJson for ObswsRtspSubscriberSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(input_url) = &self.input_url {
                f.member("inputUrl", input_url)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsRtmpOutboundSettings {
    // StartOutput 時に必須。登録時点では未指定も許容する。
    pub output_url: Option<String>,
    pub stream_name: Option<String>,
}

impl nojson::DisplayJson for ObswsRtmpOutboundSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(output_url) = &self.output_url {
                f.member("outputUrl", output_url)?;
            }
            if let Some(stream_name) = &self.stream_name {
                f.member("streamName", stream_name)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseInputSettingsError {
    UnsupportedInputKind,
    InvalidInputSettings(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateInputError {
    UnsupportedSceneName,
    InputNameAlreadyExists,
    InputIdOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetInputSettingsError {
    InputNotFound,
    InvalidInputSettings(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetInputNameError {
    InputNotFound,
    InputNameAlreadyExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateSceneError {
    SceneNameAlreadyExists,
    SceneIdOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSceneNameError {
    SceneNotFound,
    SceneNameAlreadyExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetCurrentProgramSceneError {
    SceneNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetCurrentPreviewSceneError {
    SceneNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSourceActiveError {
    SourceNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneSceneTransitionOverrideError {
    SceneNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSceneSceneTransitionOverrideError {
    SceneNotFound,
    TransitionNotFound,
    InvalidTransitionDuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetCurrentSceneTransitionError {
    TransitionNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetCurrentSceneTransitionDurationError {
    InvalidTransitionDuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetCurrentSceneTransitionSettingsError {
    InvalidTransitionSettings,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SetTBarPositionError {
    InvalidTBarPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveSceneError {
    SceneNotFound,
    LastSceneNotRemovable,
}

/// input ID のオーバーフローを表す内部エラー型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputIdOverflowError;

/// scene item ID のオーバーフローを表す内部エラー型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SceneItemIdOverflowError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunIdOverflowError {
    Stream,
    Record,
    RtmpOutbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateRtmpOutboundError {
    AlreadyActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateStreamError {
    AlreadyActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateRecordError {
    AlreadyActive,
}

/// SceneItem の検索時に発生するエラー。
/// シーンが見つからない場合とシーンアイテムが見つからない場合を表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneItemLookupError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemIdError {
    SceneNotFound,
    SourceNotFound,
    SearchOffsetUnsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetSceneItemLockedResult {
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetSceneItemBlendModeResult {
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetSceneItemTransformResult {
    pub changed: bool,
    pub scene_item_transform: ObswsSceneItemTransform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemListError {
    SceneNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateSceneItemError {
    SceneNotFound,
    SourceNotFound,
    SceneItemIdOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSceneItemIndexError {
    SceneNotFound,
    SceneItemNotFound,
    InvalidSceneItemIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateSceneItemError {
    SourceScene,
    DestinationScene,
    SourceSceneItem,
    SceneItemIdOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetSceneItemEnabledResult {
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct ObswsInputRegistry {
    pub(crate) inputs_by_uuid: BTreeMap<String, ObswsInputEntry>,
    pub(crate) uuids_by_name: BTreeMap<String, String>,
    pub(crate) scenes_by_name: BTreeMap<String, ObswsSceneState>,
    pub(crate) scene_order: Vec<String>,
    pub(crate) current_program_scene_name: String,
    pub(crate) current_preview_scene_name: String,
    pub(crate) scene_transition_overrides: BTreeMap<String, ObswsSceneTransitionOverride>,
    pub(crate) next_input_id: u64,
    pub(crate) next_scene_id: u64,
    pub(crate) next_scene_item_id: i64,
    pub(crate) next_stream_run_id: u64,
    pub(crate) next_record_run_id: u64,
    pub(crate) stream_service_settings: ObswsStreamServiceSettings,
    pub(crate) transition_runtime: ObswsTransitionRuntimeState,
    pub(crate) stream_runtime: ObswsStreamRuntimeState,
    pub(crate) rtmp_outbound_settings: ObswsRtmpOutboundSettings,
    pub(crate) rtmp_outbound_runtime: ObswsRtmpOutboundRuntimeState,
    pub(crate) next_rtmp_outbound_run_id: u64,
    pub(crate) record_directory: PathBuf,
    pub(crate) record_runtime: ObswsRecordRuntimeState,
    pub(crate) canvas_width: crate::types::EvenUsize,
    pub(crate) canvas_height: crate::types::EvenUsize,
    pub(crate) frame_rate: crate::video::FrameRate,
}
