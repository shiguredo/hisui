use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(crate) const OBSWS_SUPPORTED_INPUT_KINDS: [&str; 3] =
    ["image_source", "video_capture_device", "mp4_file_input"];
pub(crate) const OBSWS_SUPPORTED_TRANSITION_KINDS: [&str; 2] = ["Cut", "Fade"];
pub(crate) const OBSWS_MAX_INPUT_ID_FOR_UUID_SUFFIX: u64 = (1 << 48) - 1;
pub(crate) const OBSWS_MAX_SCENE_ID_FOR_UUID_SUFFIX: u64 = (1 << 48) - 1;
pub(crate) const OBSWS_DEFAULT_STREAM_SERVICE_TYPE: &str = "rtmp_custom";
pub(crate) const OBSWS_DEFAULT_TRANSITION_NAME: &str = "Cut";
pub(crate) const OBSWS_DEFAULT_TRANSITION_DURATION_MS: i64 = 300;
pub(crate) const OBSWS_DEFAULT_TRANSITION_SETTINGS_JSON: &str = "{}";
pub(crate) const OBSWS_DEFAULT_TBAR_POSITION: f64 = 0.0;
pub(crate) const OBSWS_MIN_TRANSITION_DURATION_MS: i64 = 50;
pub(crate) const OBSWS_MAX_TRANSITION_DURATION_MS: i64 = 20_000;
pub(crate) const OBSWS_MIN_TBAR_POSITION: f64 = 0.0;
pub(crate) const OBSWS_MAX_TBAR_POSITION: f64 = 1.0;

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
            f.member("inputUuid", &self.input_uuid)
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
    Mp4FileInput(ObswsMp4FileInputSettings),
}

impl ObswsInputSettings {
    pub fn default_for_kind(input_kind: &str) -> Result<Self, ParseInputSettingsError> {
        match input_kind {
            "image_source" => Ok(Self::ImageSource(ObswsImageSourceSettings::default())),
            "video_capture_device" => Ok(Self::VideoCaptureDevice(
                ObswsVideoCaptureDeviceSettings::default(),
            )),
            "mp4_file_input" => Ok(Self::Mp4FileInput(ObswsMp4FileInputSettings::default())),
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
            "mp4_file_input" => {
                let path = parse_optional_string_setting(input_settings, "path")?;
                let loop_playback = parse_optional_bool_setting(input_settings, "loopPlayback")?;
                Ok(Self::Mp4FileInput(ObswsMp4FileInputSettings {
                    path,
                    loop_playback: loop_playback.unwrap_or(false),
                }))
            }
            _ => Err(ParseInputSettingsError::UnsupportedInputKind),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::ImageSource(_) => "image_source",
            Self::VideoCaptureDevice(_) => "video_capture_device",
            Self::Mp4FileInput(_) => "mp4_file_input",
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
            Self::Mp4FileInput(existing) => {
                let path = parse_overlay_string_setting(input_settings, "path", &existing.path)?;
                let loop_playback = parse_overlay_bool_setting(
                    input_settings,
                    "loopPlayback",
                    existing.loop_playback,
                )?;
                Ok(Self::Mp4FileInput(ObswsMp4FileInputSettings {
                    path,
                    loop_playback,
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
            Self::Mp4FileInput(settings) => settings.fmt(f),
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
    pub source_processor_id: String,
    pub encoder_processor_id: String,
    pub endpoint_processor_id: String,
    pub source_track_id: String,
    pub encoded_track_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsRecordRun {
    pub source_processor_id: String,
    pub video_encoder_processor_id: Option<String>,
    pub audio_encoder_processor_id: Option<String>,
    pub writer_processor_id: String,
    pub source_video_track_id: Option<String>,
    pub source_audio_track_id: Option<String>,
    pub encoded_video_track_id: Option<String>,
    pub encoded_audio_track_id: Option<String>,
    pub output_path: PathBuf,
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
    pub scale_x: f64,
    pub scale_y: f64,
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
            scale_x: 1.0,
            scale_y: 1.0,
            alignment: 0,
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
    pub scale_x: Option<f64>,
    pub scale_y: Option<f64>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsSceneItemEntry {
    pub scene_item_id: i64,
    pub source_name: String,
    pub source_uuid: String,
    pub scene_item_enabled: bool,
    pub scene_item_locked: bool,
    pub scene_item_blend_mode: String,
    pub scene_item_index: i64,
    pub is_group: bool,
}

impl nojson::DisplayJson for ObswsSceneItemEntry {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("sceneItemId", self.scene_item_id)?;
            f.member("sourceName", &self.source_name)?;
            f.member("sourceUuid", &self.source_uuid)?;
            f.member("sceneItemEnabled", self.scene_item_enabled)?;
            f.member("sceneItemLocked", self.scene_item_locked)?;
            f.member("sceneItemBlendMode", &self.scene_item_blend_mode)?;
            f.member("sceneItemIndex", self.scene_item_index)?;
            f.member("isGroup", self.is_group)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub(crate) struct ObswsRecordRuntimeState {
    pub(crate) active: bool,
    pub(crate) started_at: Option<Instant>,
    pub(crate) paused: bool,
    pub(crate) paused_at: Option<Instant>,
    pub(crate) total_paused_duration: Duration,
    pub(crate) run: Option<ObswsRecordRun>,
}

#[derive(Debug, Clone)]
pub(crate) struct ObswsTransitionRuntimeState {
    pub(crate) current_transition_name: String,
    pub(crate) current_transition_duration_ms: i64,
    pub(crate) current_transition_settings: nojson::RawJsonOwned,
    pub(crate) current_tbar_position: f64,
}

impl Default for ObswsTransitionRuntimeState {
    fn default() -> Self {
        Self {
            current_transition_name: OBSWS_DEFAULT_TRANSITION_NAME.to_owned(),
            current_transition_duration_ms: OBSWS_DEFAULT_TRANSITION_DURATION_MS,
            current_transition_settings: nojson::RawJsonOwned::parse(
                OBSWS_DEFAULT_TRANSITION_SETTINGS_JSON,
            )
            .expect("BUG: default transition settings json must be valid"),
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
pub struct ObswsMp4FileInputSettings {
    // OBS 互換ではなく hisui 独自 input として扱うため、path 未指定も保持可能にする。
    // 実行時には path 必須とする。
    pub path: Option<String>,
    pub loop_playback: bool,
}

impl nojson::DisplayJson for ObswsMp4FileInputSettings {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseInputSettingsError {
    UnsupportedInputKind,
    InvalidInputSettings(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateInputError {
    UnsupportedSceneName,
    InputNameAlreadyExists,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateStreamError {
    AlreadyActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateRecordError {
    AlreadyActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseRecordError {
    RecordNotActive,
    AlreadyPaused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeRecordError {
    RecordNotActive,
    NotPaused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemIdError {
    SceneNotFound,
    SourceNotFound,
    SearchOffsetUnsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemEnabledError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemLockedError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSceneItemLockedError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetSceneItemLockedResult {
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemBlendModeError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSceneItemBlendModeError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetSceneItemBlendModeResult {
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemTransformError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSceneItemTransformError {
    SceneNotFound,
    SceneItemNotFound,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveSceneItemError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemSourceError {
    SceneNotFound,
    SceneItemNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetSceneItemIndexError {
    SceneNotFound,
    SceneItemNotFound,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSceneItemEnabledError {
    SceneNotFound,
    SceneItemNotFound,
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
    pub(crate) record_directory: PathBuf,
    pub(crate) record_runtime: ObswsRecordRuntimeState,
}
