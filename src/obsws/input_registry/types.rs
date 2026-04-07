use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use nojson::DisplayJson as _;

use crate::types::NonNegFiniteF64;

use crate::types::PositiveFiniteF64;
use crate::{ProcessorId, TrackId};

pub(crate) const OBSWS_SUPPORTED_INPUT_KINDS: [&str; 10] = [
    "image_source",
    "color_source",
    "video_capture_device",
    "audio_capture_device",
    "mp4_file_source",
    "rtmp_inbound",
    "srt_inbound",
    "rtsp_subscriber",
    "webrtc_source",
    "sora_source",
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
/// シーンコレクションにデフォルトで存在するトランジションインスタンス。
/// OBS は fade_transition と cut_transition のみをデフォルトで作成する。
pub(crate) const OBSWS_DEFAULT_TRANSITION_INSTANCES: [&str; 2] =
    ["fade_transition", "cut_transition"];
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
            f.member("inputKindCaps", self.input.settings.input_kind_caps())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsInput {
    pub settings: ObswsInputSettings,
    /// ミュート状態
    pub input_muted: bool,
    /// 音量乗算係数（0.0 以上の有限値、デフォルト 1.0 = 0dB）
    pub input_volume_mul: NonNegFiniteF64,
}

impl ObswsInput {
    pub fn from_kind_and_settings(
        input_kind: &str,
        input_settings: nojson::RawJsonValue<'_, '_>,
    ) -> Result<Self, ParseInputSettingsError> {
        Ok(Self {
            settings: ObswsInputSettings::from_kind_and_settings(input_kind, input_settings)?,
            input_muted: false,
            input_volume_mul: NonNegFiniteF64::ONE,
        })
    }

    pub fn kind_name(&self) -> &'static str {
        self.settings.kind_name()
    }

    /// 音量を dB 値で取得する。
    ///
    /// mul == 0.0 の場合は `f64::NEG_INFINITY` を返す。
    /// nojson は非有限値を JSON `null` として出力するため、
    /// `inputVolumeDb: null` は「音量ゼロ（-∞ dB）」を意味する。
    pub fn input_volume_db(&self) -> f64 {
        let mul = self.input_volume_mul.get();
        if mul <= 0.0 {
            f64::NEG_INFINITY
        } else {
            20.0 * mul.log10()
        }
    }

    /// 音量を dB 値から mul に変換して設定する
    pub fn set_volume_from_db(&mut self, db: f64) {
        let mul = 10.0_f64.powf(db / 20.0);
        // dB が有限なら mul も有限かつ正になる
        self.input_volume_mul = NonNegFiniteF64::new(mul).unwrap_or(NonNegFiniteF64::ZERO);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObswsInputSettings {
    ImageSource(ObswsImageSourceSettings),
    ColorSource(ObswsColorSourceSettings),
    VideoCaptureDevice(ObswsVideoCaptureDeviceSettings),
    AudioCaptureDevice(ObswsAudioCaptureDeviceSettings),
    Mp4FileSource(ObswsMp4FileSourceSettings),
    RtmpInbound(ObswsRtmpInboundSettings),
    SrtInbound(ObswsSrtInboundSettings),
    RtspSubscriber(ObswsRtspSubscriberSettings),
    WebRtcSource(ObswsWebRtcSourceSettings),
    SoraSource(ObswsSoraSourceInputSettings),
}

impl ObswsInputSettings {
    pub fn default_for_kind(input_kind: &str) -> Result<Self, ParseInputSettingsError> {
        match input_kind {
            "image_source" => Ok(Self::ImageSource(ObswsImageSourceSettings::default())),
            "color_source" => Ok(Self::ColorSource(ObswsColorSourceSettings::default())),
            "video_capture_device" => Ok(Self::VideoCaptureDevice(
                ObswsVideoCaptureDeviceSettings::default(),
            )),
            "audio_capture_device" => Ok(Self::AudioCaptureDevice(
                ObswsAudioCaptureDeviceSettings::default(),
            )),
            "mp4_file_source" => Ok(Self::Mp4FileSource(ObswsMp4FileSourceSettings::default())),
            "rtmp_inbound" => Ok(Self::RtmpInbound(ObswsRtmpInboundSettings::default())),
            "srt_inbound" => Ok(Self::SrtInbound(ObswsSrtInboundSettings::default())),
            "rtsp_subscriber" => Ok(Self::RtspSubscriber(ObswsRtspSubscriberSettings::default())),
            "webrtc_source" => Ok(Self::WebRtcSource(ObswsWebRtcSourceSettings::default())),
            "sora_source" => Ok(Self::SoraSource(ObswsSoraSourceInputSettings::default())),
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
            "color_source" => {
                let color = parse_optional_string_setting(input_settings, "color")?;
                validate_hex_color(&color)?;
                Ok(Self::ColorSource(ObswsColorSourceSettings { color }))
            }
            "video_capture_device" => {
                let device_id = parse_optional_string_setting(input_settings, "device_id")?;
                let pixel_format = parse_optional_string_setting(input_settings, "pixel_format")?;
                validate_video_capture_pixel_format(&pixel_format)?;
                let fps = parse_optional_i32_setting(input_settings, "fps")?;
                validate_video_capture_fps(fps)?;
                Ok(Self::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                    device_id,
                    pixel_format,
                    fps,
                }))
            }
            "audio_capture_device" => {
                let device_id = parse_optional_string_setting(input_settings, "device_id")?;
                let sample_rate = parse_optional_i32_setting(input_settings, "sampleRate")?;
                let channels = parse_optional_i32_setting(input_settings, "channels")?;
                Ok(Self::AudioCaptureDevice(ObswsAudioCaptureDeviceSettings {
                    device_id,
                    sample_rate,
                    channels,
                }))
            }
            "mp4_file_source" => {
                let path = parse_optional_string_setting(input_settings, "path")?;
                let loop_playback = parse_optional_bool_setting(input_settings, "loopPlayback")?;
                Ok(Self::Mp4FileSource(ObswsMp4FileSourceSettings {
                    path,
                    loop_playback: loop_playback.unwrap_or(true),
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
            "webrtc_source" => {
                // trackId は Attach/Detach で制御するため、CreateInput 時は無視する
                let background_key_color =
                    parse_optional_string_setting(input_settings, "backgroundKeyColor")?;
                let background_key_tolerance =
                    parse_optional_i32_setting(input_settings, "backgroundKeyTolerance")?;
                validate_background_key_tolerance(background_key_tolerance)?;
                Ok(Self::WebRtcSource(ObswsWebRtcSourceSettings {
                    track_id: None,
                    background_key_color,
                    background_key_tolerance,
                }))
            }
            "sora_source" => {
                // trackId は Attach/Detach で制御するため、CreateInput 時は無視する
                Ok(Self::SoraSource(ObswsSoraSourceInputSettings::default()))
            }
            _ => Err(ParseInputSettingsError::UnsupportedInputKind),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::ImageSource(_) => "image_source",
            Self::ColorSource(_) => "color_source",
            // TODO: `video_capture_device` は将来的に `video_device_source` へ rename して、
            // `*_source` 命名へ統一する。今回は既存 API 影響を避けるため据え置く。
            Self::VideoCaptureDevice(_) => "video_capture_device",
            Self::AudioCaptureDevice(_) => "audio_capture_device",
            Self::Mp4FileSource(_) => "mp4_file_source",
            Self::RtmpInbound(_) => "rtmp_inbound",
            Self::SrtInbound(_) => "srt_inbound",
            Self::RtspSubscriber(_) => "rtsp_subscriber",
            Self::WebRtcSource(_) => "webrtc_source",
            Self::SoraSource(_) => "sora_source",
        }
    }

    /// OBS WebSocket の `inputKindCaps` に相当するビットフラグを返す。
    ///
    /// OBS の `obs_source_info::output_flags` に対応する。
    /// - bit 0 (`OBS_SOURCE_VIDEO`): 映像出力を持つ
    /// - bit 1 (`OBS_SOURCE_AUDIO`): 音声出力を持つ
    pub fn input_kind_caps(&self) -> u32 {
        const VIDEO: u32 = 1;
        const AUDIO: u32 = 2;
        match self {
            Self::ImageSource(_) => VIDEO,
            Self::ColorSource(_) => VIDEO,
            Self::VideoCaptureDevice(_) => VIDEO,
            Self::AudioCaptureDevice(_) => AUDIO,
            Self::Mp4FileSource(_) => VIDEO | AUDIO,
            Self::RtmpInbound(_) => VIDEO | AUDIO,
            Self::SrtInbound(_) => VIDEO | AUDIO,
            Self::RtspSubscriber(_) => VIDEO | AUDIO,
            Self::WebRtcSource(_) => VIDEO,
            Self::SoraSource(_) => VIDEO | AUDIO,
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
            Self::ColorSource(existing) => {
                let color = parse_overlay_string_setting(input_settings, "color", &existing.color)?;
                validate_hex_color(&color)?;
                Ok(Self::ColorSource(ObswsColorSourceSettings { color }))
            }
            Self::VideoCaptureDevice(existing) => {
                let device_id =
                    parse_overlay_string_setting(input_settings, "device_id", &existing.device_id)?;
                let pixel_format = parse_overlay_string_setting(
                    input_settings,
                    "pixel_format",
                    &existing.pixel_format,
                )?;
                validate_video_capture_pixel_format(&pixel_format)?;
                let fps = parse_overlay_i32_setting(input_settings, "fps", &existing.fps)?;
                validate_video_capture_fps(fps)?;
                Ok(Self::VideoCaptureDevice(ObswsVideoCaptureDeviceSettings {
                    device_id,
                    pixel_format,
                    fps,
                }))
            }
            Self::AudioCaptureDevice(existing) => {
                let device_id =
                    parse_overlay_string_setting(input_settings, "device_id", &existing.device_id)?;
                let sample_rate =
                    parse_overlay_i32_setting(input_settings, "sampleRate", &existing.sample_rate)?;
                let channels =
                    parse_overlay_i32_setting(input_settings, "channels", &existing.channels)?;
                Ok(Self::AudioCaptureDevice(ObswsAudioCaptureDeviceSettings {
                    device_id,
                    sample_rate,
                    channels,
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
            Self::WebRtcSource(existing) => {
                // trackId は Attach/Detach で制御するため overlay 対象外
                let background_key_color = parse_overlay_string_setting(
                    input_settings,
                    "backgroundKeyColor",
                    &existing.background_key_color,
                )?;
                let background_key_tolerance = parse_overlay_i32_setting(
                    input_settings,
                    "backgroundKeyTolerance",
                    &existing.background_key_tolerance,
                )?;
                validate_background_key_tolerance(background_key_tolerance)?;
                Ok(Self::WebRtcSource(ObswsWebRtcSourceSettings {
                    track_id: existing.track_id.clone(),
                    background_key_color,
                    background_key_tolerance,
                }))
            }
            Self::SoraSource(existing) => {
                // trackId は Attach/Detach で制御するため overlay 対象外
                Ok(Self::SoraSource(existing.clone()))
            }
        }
    }
}

impl nojson::DisplayJson for ObswsInputSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::ImageSource(settings) => settings.fmt(f),
            Self::ColorSource(settings) => settings.fmt(f),
            Self::VideoCaptureDevice(settings) => settings.fmt(f),
            Self::AudioCaptureDevice(settings) => settings.fmt(f),
            Self::Mp4FileSource(settings) => settings.fmt(f),
            Self::RtmpInbound(settings) => settings.fmt(f),
            Self::SrtInbound(settings) => settings.fmt(f),
            Self::RtspSubscriber(settings) => settings.fmt(f),
            Self::WebRtcSource(settings) => settings.fmt(f),
            Self::SoraSource(settings) => settings.fmt(f),
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
    pub video: ObswsRecordTrackRun,
    pub audio: ObswsRecordTrackRun,
    pub publisher_processor_id: ProcessorId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsRtmpOutboundRun {
    pub video: ObswsRecordTrackRun,
    pub audio: ObswsRecordTrackRun,
    pub endpoint_processor_id: ProcessorId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsRecordRun {
    pub video: ObswsRecordTrackRun,
    pub audio: ObswsRecordTrackRun,
    pub writer_processor_id: ProcessorId,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsHlsRun {
    pub destination: HlsDestination,
    /// バリアントごとの実行情報
    pub variant_runs: Vec<ObswsHlsVariantRun>,
}

impl ObswsHlsRun {
    /// ABR（マスタープレイリストあり）かどうかを返す
    pub fn is_abr(&self) -> bool {
        self.variant_runs.len() > 1
    }
}

/// HLS ABR バリアントごとの実行情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsHlsVariantRun {
    pub video: ObswsRecordTrackRun,
    pub audio: ObswsRecordTrackRun,
    /// 解像度変換が必要なバリアントのスケーラープロセッサ ID
    pub scaler_processor_id: Option<ProcessorId>,
    /// スケーラー出力トラック ID
    pub scaled_track_id: Option<TrackId>,
    pub writer_processor_id: ProcessorId,
    /// バリアントの出力パス（filesystem: ディレクトリパス、S3: prefix）
    pub variant_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsSoraPublisherRun {
    pub publisher_processor_id: ProcessorId,
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
                "output:{output_kind}:{media_kind}_encoder:{run_id}"
            )),
            source_track_id: source_track_id.clone(),
            encoded_track_id: TrackId::new(format!(
                "output:{output_kind}:encoded_{media_kind}:{run_id}"
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
pub(crate) struct ObswsSoraPublisherRuntimeState {
    pub(crate) active: bool,
    pub(crate) started_at: Option<Instant>,
    pub(crate) run: Option<ObswsSoraPublisherRun>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ObswsRecordRuntimeState {
    pub(crate) active: bool,
    pub(crate) started_at: Option<Instant>,
    pub(crate) run: Option<ObswsRecordRun>,
}

#[derive(Debug, Default)]
pub(crate) struct ObswsHlsRuntimeState {
    pub(crate) active: bool,
    pub(crate) started_at: Option<Instant>,
    pub(crate) run: Option<ObswsHlsRun>,
    /// ABR マスタープレイリスト書き出しタスクの JoinHandle。
    /// 出力停止時に abort() でキャンセルする。
    pub(crate) master_playlist_task: Option<tokio::task::JoinHandle<()>>,
}

impl Clone for ObswsHlsRuntimeState {
    fn clone(&self) -> Self {
        Self {
            active: self.active,
            started_at: self.started_at,
            run: self.run.clone(),
            master_playlist_task: None,
        }
    }
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

fn validate_hex_color(color: &Option<String>) -> Result<(), ParseInputSettingsError> {
    if let Some(c) = color
        && crate::obsws::source::webrtc_source::parse_hex_color(c).is_none()
    {
        return Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "invalid color format: expected #RRGGBB, got {c}"
        )));
    }
    Ok(())
}

fn validate_video_capture_pixel_format(
    pixel_format: &Option<String>,
) -> Result<(), ParseInputSettingsError> {
    match pixel_format.as_deref() {
        None | Some("NV12" | "YUY2" | "I420") => Ok(()),
        Some(value) => Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.pixel_format field: unsupported pixel format: {value}"
        ))),
    }
}

fn validate_video_capture_fps(fps: Option<i32>) -> Result<(), ParseInputSettingsError> {
    if let Some(value) = fps
        && value <= 0
    {
        return Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.fps field: positive integer is required, got {value}"
        )));
    }
    Ok(())
}

fn validate_background_key_tolerance(value: Option<i32>) -> Result<(), ParseInputSettingsError> {
    if let Some(v) = value
        && !(0..=255).contains(&v)
    {
        return Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "backgroundKeyTolerance must be 0-255, got {v}"
        )));
    }
    Ok(())
}

fn parse_optional_i32_setting(
    settings: nojson::RawJsonValue<'_, '_>,
    key: &str,
) -> Result<Option<i32>, ParseInputSettingsError> {
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

    if value.kind() != nojson::JsonValueKind::Integer {
        return Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: integer is required"
        )));
    }
    let value: i64 = value.try_into().map_err(|e| {
        ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: {e}"
        ))
    })?;
    let value = i32::try_from(value).map_err(|_| {
        ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: value out of i32 range"
        ))
    })?;
    Ok(Some(value))
}

fn parse_overlay_i32_setting(
    settings: nojson::RawJsonValue<'_, '_>,
    key: &str,
    current: &Option<i32>,
) -> Result<Option<i32>, ParseInputSettingsError> {
    let Some(value) = settings
        .to_member(key)
        .map_err(|e| {
            ParseInputSettingsError::InvalidInputSettings(format!(
                "Invalid inputSettings field: {e}"
            ))
        })?
        .optional()
    else {
        return Ok(*current);
    };

    if value.kind() != nojson::JsonValueKind::Integer {
        return Err(ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: integer is required"
        )));
    }
    let value: i64 = value.try_into().map_err(|e| {
        ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: {e}"
        ))
    })?;
    let value = i32::try_from(value).map_err(|_| {
        ParseInputSettingsError::InvalidInputSettings(format!(
            "Invalid inputSettings.{key} field: value out of i32 range"
        ))
    })?;
    Ok(Some(value))
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
pub struct ObswsColorSourceSettings {
    /// `#RRGGBB` 形式の色指定。未指定時は実行時に `#000000`（黒）として扱う
    pub color: Option<String>,
}

impl nojson::DisplayJson for ObswsColorSourceSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(color) = &self.color {
                f.member("color", color)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsVideoCaptureDeviceSettings {
    // OBS 互換のため、video_capture_device は device_id 未指定でも入力としては受理する
    pub device_id: Option<String>,
    pub pixel_format: Option<String>,
    pub fps: Option<i32>,
}

impl nojson::DisplayJson for ObswsVideoCaptureDeviceSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(device_id) = &self.device_id {
                f.member("device_id", device_id)?;
            }
            if let Some(pixel_format) = &self.pixel_format {
                f.member("pixel_format", pixel_format)?;
            }
            if let Some(fps) = &self.fps {
                f.member("fps", fps)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsAudioCaptureDeviceSettings {
    // OBS 互換のため、audio_capture_device は device_id 未指定でも入力としては受理する
    pub device_id: Option<String>,
    pub sample_rate: Option<i32>,
    pub channels: Option<i32>,
}

impl nojson::DisplayJson for ObswsAudioCaptureDeviceSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(device_id) = &self.device_id {
                f.member("device_id", device_id)?;
            }
            if let Some(sample_rate) = self.sample_rate {
                f.member("sampleRate", i64::from(sample_rate))?;
            }
            if let Some(channels) = self.channels {
                f.member("channels", i64::from(channels))?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsMp4FileSourceSettings {
    // OBS 互換ではなく hisui 独自 input として扱うため、path 未指定も保持可能にする。
    // 実行時には path 必須とする。
    pub path: Option<String>,
    pub loop_playback: bool,
}

impl Default for ObswsMp4FileSourceSettings {
    fn default() -> Self {
        Self {
            path: None,
            loop_playback: true,
        }
    }
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
pub struct ObswsWebRtcSourceSettings {
    // trackId は Attach/Detach Request で制御する。SetInputSettings では変更不可。
    pub track_id: Option<String>,
    // 透過対象の背景色。#RRGGBB 形式。null は透過なし。
    pub background_key_color: Option<String>,
    // key color 許容差。0 以上 255 以下の整数。null は透過なし。
    pub background_key_tolerance: Option<i32>,
}

impl nojson::DisplayJson for ObswsWebRtcSourceSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(track_id) = &self.track_id {
                f.member("trackId", track_id)?;
            }
            if let Some(background_key_color) = &self.background_key_color {
                f.member("backgroundKeyColor", background_key_color)?;
            }
            if let Some(background_key_tolerance) = self.background_key_tolerance {
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsSoraSourceInputSettings {
    // video/audio の trackId は Attach/Detach で制御する。SetInputSettings では変更不可。
    pub video_track_id: Option<String>,
    pub audio_track_id: Option<String>,
}

impl nojson::DisplayJson for ObswsSoraSourceInputSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(video_track_id) = &self.video_track_id {
                f.member("videoTrackId", video_track_id)?;
            }
            if let Some(audio_track_id) = &self.audio_track_id {
                f.member("audioTrackId", audio_track_id)?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

/// SoraSubscriber の接続設定。CreateSoraSubscriber で登録する。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsSoraSubscriberSettings {
    pub signaling_urls: Vec<String>,
    pub channel_id: Option<String>,
    pub client_id: Option<String>,
    pub bundle_id: Option<String>,
    pub metadata: Option<nojson::RawJsonOwned>,
}

impl nojson::DisplayJson for ObswsSoraSubscriberSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if !self.signaling_urls.is_empty() {
                f.member("signalingUrls", &self.signaling_urls)?;
            }
            if let Some(channel_id) = &self.channel_id {
                f.member("channelId", channel_id)?;
            }
            if let Some(client_id) = &self.client_id {
                f.member("clientId", client_id)?;
            }
            if let Some(bundle_id) = &self.bundle_id {
                f.member("bundleId", bundle_id)?;
            }
            if let Some(metadata) = &self.metadata {
                f.member("metadata", metadata)?;
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

/// HLS 出力の設定。
/// SetOutputSettings で各フィールドを変更可能。
pub const DEFAULT_HLS_SEGMENT_DURATION_SECS: f64 = 2.0;
pub const DEFAULT_HLS_MAX_RETAINED_SEGMENTS: usize = 6;
pub const DEFAULT_HLS_VIDEO_BITRATE_BPS: usize = 2_000_000;
pub const DEFAULT_HLS_AUDIO_BITRATE_BPS: usize = 128_000;

/// HLS ABR のバリアント定義。
/// バリアントごとにビットレートと解像度を指定する。
#[derive(Debug, Clone, PartialEq)]
pub struct HlsVariant {
    /// ビデオビットレート (bps)
    pub video_bitrate_bps: usize,
    /// オーディオビットレート (bps)
    pub audio_bitrate_bps: usize,
    /// ビデオ幅（省略時はミキサーのキャンバスサイズを使用）
    pub width: Option<crate::types::EvenUsize>,
    /// ビデオ高さ（省略時はミキサーのキャンバスサイズを使用）
    pub height: Option<crate::types::EvenUsize>,
}

impl Default for HlsVariant {
    fn default() -> Self {
        Self {
            video_bitrate_bps: DEFAULT_HLS_VIDEO_BITRATE_BPS,
            audio_bitrate_bps: DEFAULT_HLS_AUDIO_BITRATE_BPS,
            width: None,
            height: None,
        }
    }
}

impl nojson::DisplayJson for HlsVariant {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("videoBitrate", self.video_bitrate_bps)?;
            f.member("audioBitrate", self.audio_bitrate_bps)?;
            if let Some(width) = self.width {
                f.member("width", width.get())?;
            }
            if let Some(height) = self.height {
                f.member("height", height.get())?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

/// HLS セグメントのフォーマット
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HlsSegmentFormat {
    /// MPEG-TS (.ts)
    #[default]
    MpegTs,
    /// Fragmented MP4 (.m4s + init.mp4)
    Fmp4,
}

impl HlsSegmentFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MpegTs => "mpegts",
            Self::Fmp4 => "fmp4",
        }
    }
}

impl std::str::FromStr for HlsSegmentFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mpegts" => Ok(Self::MpegTs),
            "fmp4" => Ok(Self::Fmp4),
            _ => Err(format!(
                "segmentFormat must be \"mpegts\" or \"fmp4\", got \"{s}\""
            )),
        }
    }
}

/// HLS 出力先の設定
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HlsDestination {
    /// ローカルファイルシステムへの出力
    Filesystem { directory: String },
    /// S3 互換オブジェクトストレージへの出力
    S3 {
        bucket: String,
        prefix: String,
        region: String,
        endpoint: Option<String>,
        use_path_style: bool,
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
        /// オブジェクトのライフタイム（日数）。
        /// 指定時はバケットに lifecycle ルールを設定する（セーフティネット用途）。
        /// 明示的な DeleteObject は常に実行する。
        lifetime_days: Option<u32>,
    },
}

impl nojson::DisplayJson for HlsDestination {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::Filesystem { directory } => nojson::object(|f| {
                f.member("type", "filesystem")?;
                f.member("directory", directory)
            })
            .fmt(f),
            Self::S3 {
                bucket,
                prefix,
                region,
                endpoint,
                use_path_style,
                // 認証情報はレスポンスに含めない
                access_key_id: _,
                secret_access_key: _,
                session_token: _,
                lifetime_days,
            } => nojson::object(|f| {
                f.member("type", "s3")?;
                f.member("bucket", bucket)?;
                f.member("prefix", prefix)?;
                f.member("region", region)?;
                if let Some(endpoint) = endpoint {
                    f.member("endpoint", endpoint)?;
                }
                f.member("usePathStyle", *use_path_style)?;
                if let Some(days) = lifetime_days {
                    f.member("lifetimeDays", *days)?;
                }
                Ok(())
            })
            .fmt(f),
        }
    }
}

impl HlsDestination {
    /// state file 用: 認証情報を含めて JSON オブジェクトとして出力する
    pub fn fmt_with_credentials(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::Filesystem { directory } => nojson::object(|f| {
                f.member("type", "filesystem")?;
                f.member("directory", directory)
            })
            .fmt(f),
            Self::S3 {
                bucket,
                prefix,
                region,
                endpoint,
                use_path_style,
                access_key_id,
                secret_access_key,
                session_token,
                lifetime_days,
            } => nojson::object(|f| {
                f.member("type", "s3")?;
                f.member("bucket", bucket)?;
                f.member("prefix", prefix)?;
                f.member("region", region)?;
                if let Some(endpoint) = endpoint {
                    f.member("endpoint", endpoint)?;
                }
                f.member("usePathStyle", *use_path_style)?;
                f.member(
                    "credentials",
                    nojson::object(|f| {
                        f.member("accessKeyId", access_key_id)?;
                        f.member("secretAccessKey", secret_access_key)?;
                        if let Some(token) = session_token {
                            f.member("sessionToken", token)?;
                        }
                        Ok(())
                    }),
                )?;
                if let Some(days) = lifetime_days {
                    f.member("lifetimeDays", *days)?;
                }
                Ok(())
            })
            .fmt(f),
        }
    }

    /// GetOutputStatus 用の outputPath を生成する
    pub fn output_path(&self) -> String {
        match self {
            Self::Filesystem { directory } => {
                let path = std::path::PathBuf::from(directory).join("playlist.m3u8");
                path.display().to_string()
            }
            Self::S3 { bucket, prefix, .. } => {
                if prefix.is_empty() {
                    format!("s3://{bucket}/playlist.m3u8")
                } else {
                    format!("s3://{bucket}/{prefix}/playlist.m3u8")
                }
            }
        }
    }

    /// バリアント用のサブパスを生成する
    pub fn variant_path(&self, index: usize) -> String {
        match self {
            Self::Filesystem { directory } => std::path::PathBuf::from(directory)
                .join(format!("variant_{index}"))
                .display()
                .to_string(),
            Self::S3 { prefix, .. } => {
                if prefix.is_empty() {
                    format!("variant_{index}")
                } else {
                    format!("{prefix}/variant_{index}")
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsHlsSettings {
    // StartOutput 時に必須。登録時点では未指定も許容する。
    pub destination: Option<HlsDestination>,
    /// セグメントの目標尺（秒）
    pub segment_duration: f64,
    /// プレイリストに保持するセグメント数
    pub max_retained_segments: usize,
    /// セグメントフォーマット
    pub segment_format: HlsSegmentFormat,
    /// ABR バリアント定義。
    /// 要素が 1 つの場合は non-ABR（マスタープレイリストを生成しない）。
    pub variants: Vec<HlsVariant>,
}

impl Default for ObswsHlsSettings {
    fn default() -> Self {
        Self {
            destination: None,
            segment_duration: DEFAULT_HLS_SEGMENT_DURATION_SECS,
            max_retained_segments: DEFAULT_HLS_MAX_RETAINED_SEGMENTS,
            segment_format: HlsSegmentFormat::default(),
            variants: vec![HlsVariant::default()],
        }
    }
}

impl nojson::DisplayJson for ObswsHlsSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(destination) = &self.destination {
                f.member("destination", destination)?;
            }
            f.member("segmentDuration", self.segment_duration)?;
            f.member("maxRetainedSegments", self.max_retained_segments)?;
            f.member("segmentFormat", self.segment_format.as_str())?;
            f.member(
                "variants",
                nojson::array(|f| {
                    for variant in &self.variants {
                        f.element(variant)?;
                    }
                    Ok(())
                }),
            )
        })
        .fmt(f)
    }
}

// MPEG-DASH 設定
pub const DEFAULT_DASH_SEGMENT_DURATION_SECS: f64 = 2.0;
pub const DEFAULT_DASH_MAX_RETAINED_SEGMENTS: usize = 6;
pub const DEFAULT_DASH_VIDEO_BITRATE_BPS: usize = 2_000_000;
pub const DEFAULT_DASH_AUDIO_BITRATE_BPS: usize = 128_000;

#[derive(Debug, Clone, PartialEq)]
pub struct DashVariant {
    /// ビデオビットレート (bps)
    pub video_bitrate_bps: usize,
    /// オーディオビットレート (bps)
    pub audio_bitrate_bps: usize,
    /// ビデオ幅（省略時はミキサーのキャンバスサイズを使用）
    pub width: Option<crate::types::EvenUsize>,
    /// ビデオ高さ（省略時はミキサーのキャンバスサイズを使用）
    pub height: Option<crate::types::EvenUsize>,
}

impl Default for DashVariant {
    fn default() -> Self {
        Self {
            video_bitrate_bps: DEFAULT_DASH_VIDEO_BITRATE_BPS,
            audio_bitrate_bps: DEFAULT_DASH_AUDIO_BITRATE_BPS,
            width: None,
            height: None,
        }
    }
}

impl nojson::DisplayJson for DashVariant {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("videoBitrate", self.video_bitrate_bps)?;
            f.member("audioBitrate", self.audio_bitrate_bps)?;
            if let Some(width) = self.width {
                f.member("width", width.get())?;
            }
            if let Some(height) = self.height {
                f.member("height", height.get())?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

/// MPEG-DASH 出力先の設定
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashDestination {
    /// ローカルファイルシステムへの出力
    Filesystem { directory: String },
    /// S3 互換オブジェクトストレージへの出力
    S3 {
        bucket: String,
        prefix: String,
        region: String,
        endpoint: Option<String>,
        use_path_style: bool,
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
        /// オブジェクトのライフタイム（日数）。
        /// 指定時はバケットに lifecycle ルールを設定する（セーフティネット用途）。
        /// 明示的な DeleteObject は常に実行する。
        lifetime_days: Option<u32>,
    },
}

impl nojson::DisplayJson for DashDestination {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::Filesystem { directory } => nojson::object(|f| {
                f.member("type", "filesystem")?;
                f.member("directory", directory)
            })
            .fmt(f),
            Self::S3 {
                bucket,
                prefix,
                region,
                endpoint,
                use_path_style,
                // 認証情報はレスポンスに含めない
                access_key_id: _,
                secret_access_key: _,
                session_token: _,
                lifetime_days,
            } => nojson::object(|f| {
                f.member("type", "s3")?;
                f.member("bucket", bucket)?;
                f.member("prefix", prefix)?;
                f.member("region", region)?;
                if let Some(endpoint) = endpoint {
                    f.member("endpoint", endpoint)?;
                }
                f.member("usePathStyle", *use_path_style)?;
                if let Some(days) = lifetime_days {
                    f.member("lifetimeDays", *days)?;
                }
                Ok(())
            })
            .fmt(f),
        }
    }
}

impl DashDestination {
    /// state file 用: 認証情報を含めて JSON オブジェクトとして出力する
    pub fn fmt_with_credentials(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::Filesystem { directory } => nojson::object(|f| {
                f.member("type", "filesystem")?;
                f.member("directory", directory)
            })
            .fmt(f),
            Self::S3 {
                bucket,
                prefix,
                region,
                endpoint,
                use_path_style,
                access_key_id,
                secret_access_key,
                session_token,
                lifetime_days,
            } => nojson::object(|f| {
                f.member("type", "s3")?;
                f.member("bucket", bucket)?;
                f.member("prefix", prefix)?;
                f.member("region", region)?;
                if let Some(endpoint) = endpoint {
                    f.member("endpoint", endpoint)?;
                }
                f.member("usePathStyle", *use_path_style)?;
                f.member(
                    "credentials",
                    nojson::object(|f| {
                        f.member("accessKeyId", access_key_id)?;
                        f.member("secretAccessKey", secret_access_key)?;
                        if let Some(token) = session_token {
                            f.member("sessionToken", token)?;
                        }
                        Ok(())
                    }),
                )?;
                if let Some(days) = lifetime_days {
                    f.member("lifetimeDays", *days)?;
                }
                Ok(())
            })
            .fmt(f),
        }
    }

    /// GetOutputStatus 用の outputPath を生成する
    pub fn output_path(&self) -> String {
        match self {
            Self::Filesystem { directory } => {
                let path = std::path::PathBuf::from(directory).join("manifest.mpd");
                path.display().to_string()
            }
            Self::S3 { bucket, prefix, .. } => {
                if prefix.is_empty() {
                    format!("s3://{bucket}/manifest.mpd")
                } else {
                    format!("s3://{bucket}/{prefix}/manifest.mpd")
                }
            }
        }
    }

    /// バリアント用のサブパスを生成する
    pub fn variant_path(&self, index: usize) -> String {
        match self {
            Self::Filesystem { directory } => std::path::PathBuf::from(directory)
                .join(format!("variant_{index}"))
                .display()
                .to_string(),
            Self::S3 { prefix, .. } => {
                if prefix.is_empty() {
                    format!("variant_{index}")
                } else {
                    format!("{prefix}/variant_{index}")
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsDashSettings {
    // StartOutput 時に必須。登録時点では未指定も許容する。
    pub destination: Option<DashDestination>,
    /// セグメントの目標尺（秒）
    pub segment_duration: f64,
    /// マニフェストに保持するセグメント数
    pub max_retained_segments: usize,
    /// ABR バリアント定義。
    /// 要素が 1 つの場合は non-ABR。
    pub variants: Vec<DashVariant>,
    /// ビデオコーデック。全バリアント共通。
    pub video_codec: crate::types::CodecName,
    /// オーディオコーデック。全バリアント共通。
    pub audio_codec: crate::types::CodecName,
}

impl Default for ObswsDashSettings {
    fn default() -> Self {
        Self {
            destination: None,
            segment_duration: DEFAULT_DASH_SEGMENT_DURATION_SECS,
            max_retained_segments: DEFAULT_DASH_MAX_RETAINED_SEGMENTS,
            variants: vec![DashVariant::default()],
            video_codec: crate::types::CodecName::H264,
            audio_codec: crate::types::CodecName::Aac,
        }
    }
}

impl nojson::DisplayJson for ObswsDashSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(destination) = &self.destination {
                f.member("destination", destination)?;
            }
            f.member("segmentDuration", self.segment_duration)?;
            f.member("maxRetainedSegments", self.max_retained_segments)?;
            f.member(
                "variants",
                nojson::array(|f| {
                    for variant in &self.variants {
                        f.element(variant)?;
                    }
                    Ok(())
                }),
            )?;
            f.member("videoCodec", self.video_codec)?;
            f.member("audioCodec", self.audio_codec)
        })
        .fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsDashRun {
    pub destination: DashDestination,
    /// バリアントごとの実行情報
    pub variant_runs: Vec<ObswsDashVariantRun>,
}

impl ObswsDashRun {
    /// ABR かどうかを返す
    pub fn is_abr(&self) -> bool {
        self.variant_runs.len() > 1
    }
}

/// MPEG-DASH ABR バリアントごとの実行情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsDashVariantRun {
    pub video: ObswsRecordTrackRun,
    pub audio: ObswsRecordTrackRun,
    /// 解像度変換が必要なバリアントのスケーラープロセッサ ID
    pub scaler_processor_id: Option<ProcessorId>,
    /// スケーラー出力トラック ID
    pub scaled_track_id: Option<TrackId>,
    pub writer_processor_id: ProcessorId,
    /// バリアントの出力パス（filesystem: ディレクトリパス、S3: prefix）
    pub variant_path: String,
}

#[derive(Debug, Default)]
pub(crate) struct ObswsDashRuntimeState {
    pub(crate) active: bool,
    pub(crate) started_at: Option<Instant>,
    pub(crate) run: Option<ObswsDashRun>,
    /// ABR 結合 MPD 書き出しタスクの JoinHandle。
    /// 出力停止時に abort() でキャンセルする。
    pub(crate) combined_mpd_task: Option<tokio::task::JoinHandle<()>>,
}

impl Clone for ObswsDashRuntimeState {
    fn clone(&self) -> Self {
        Self {
            active: self.active,
            started_at: self.started_at,
            run: self.run.clone(),
            // JoinHandle は clone できないため、clone 時は None にする。
            // ObswsInputRegistry の clone は coordinator の初期化時のみで、
            // DASH 出力がアクティブな状態で clone されることはない。
            combined_mpd_task: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObswsSoraPublisherSettings {
    // StartOutput 時に必須。登録時点では未指定も許容する。
    pub signaling_urls: Vec<String>,
    pub channel_id: Option<String>,
    pub client_id: Option<String>,
    pub bundle_id: Option<String>,
    pub metadata: Option<nojson::RawJsonOwned>,
}

impl nojson::DisplayJson for ObswsSoraPublisherSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member(
                "soraSdkSettings",
                nojson::object(|f| {
                    if !self.signaling_urls.is_empty() {
                        f.member("signalingUrls", &self.signaling_urls)?;
                    }
                    if let Some(channel_id) = &self.channel_id {
                        f.member("channelId", channel_id)?;
                    }
                    if let Some(client_id) = &self.client_id {
                        f.member("clientId", client_id)?;
                    }
                    if let Some(bundle_id) = &self.bundle_id {
                        f.member("bundleId", bundle_id)?;
                    }
                    if let Some(metadata) = &self.metadata {
                        f.member("metadata", metadata)?;
                    }
                    Ok(())
                }),
            )
        })
        .fmt(f)
    }
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

impl std::fmt::Display for ParseInputSettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedInputKind => write!(f, "unsupported input kind"),
            Self::InvalidInputSettings(msg) => write!(f, "{msg}"),
        }
    }
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
    SoraPublisher,
    Hls,
    MpegDash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateRtmpOutboundError {
    AlreadyActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateSoraPublisherError {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateHlsError {
    AlreadyActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateDashError {
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
    pub(crate) sora_publisher_settings: ObswsSoraPublisherSettings,
    pub(crate) sora_publisher_runtime: ObswsSoraPublisherRuntimeState,
    pub(crate) next_sora_publisher_run_id: u64,
    pub(crate) record_directory: PathBuf,
    pub(crate) record_runtime: ObswsRecordRuntimeState,
    pub(crate) hls_settings: ObswsHlsSettings,
    pub(crate) hls_runtime: ObswsHlsRuntimeState,
    pub(crate) next_hls_run_id: u64,
    pub(crate) dash_settings: ObswsDashSettings,
    pub(crate) dash_runtime: ObswsDashRuntimeState,
    pub(crate) next_dash_run_id: u64,
    #[cfg(feature = "player")]
    pub(crate) player_active: bool,
    pub(crate) canvas_width: crate::types::EvenUsize,
    pub(crate) canvas_height: crate::types::EvenUsize,
    pub(crate) frame_rate: crate::video::FrameRate,
    pub(crate) state_file_path: Option<PathBuf>,
    pub(crate) persistent_data: BTreeMap<String, nojson::RawJsonOwned>,
}
