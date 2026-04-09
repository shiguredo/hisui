//! output の統一管理。
//! 全 output を名前付きインスタンスとして BTreeMap で管理し、
//! HisuiCreateOutput / HisuiRemoveOutput で動的に追加・削除する。
//!
//! TODO: モジュール名・構成の整理
//! - このモジュール名 `output_dynamic` は、全 output が動的管理に統一された現在では不適切。
//!   リネーム候補: `output_registry`, `output_manager`, `output_instance` 等。
//! - `input_registry` も実際には Input / Scene / SceneItem / Transition / グローバル設定を
//!   管理しており「input 専用レジストリ」ではないため、命名の見直しが必要。
//! - 両モジュールの責務と命名を合わせて検討すること。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use super::{CommandResult, ObswsCoordinator};
use crate::obsws::input_registry::{
    ObswsDashSettings, ObswsHlsSettings, ObswsRtmpOutboundSettings, ObswsSoraPublisherSettings,
    ObswsStreamServiceSettings,
};
use crate::obsws::protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_FIELD,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS, REQUEST_STATUS_RESOURCE_NOT_FOUND,
};

// -----------------------------------------------------------------------
// 型定義
// -----------------------------------------------------------------------

/// output インスタンスの状態
pub(crate) struct OutputState {
    pub(crate) output_kind: OutputKind,
    pub(crate) settings: OutputSettings,
    pub(crate) runtime: OutputRuntimeState,
}

/// output の稼働状態
#[derive(Default)]
pub(crate) struct OutputRuntimeState {
    pub(crate) active: bool,
    pub(crate) started_at: Option<Instant>,
    pub(crate) run: Option<OutputRun>,
    /// HLS/DASH の ABR マスタープレイリスト等の非同期タスクハンドル
    pub(crate) background_task: Option<tokio::task::JoinHandle<()>>,
}

/// output の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputKind {
    /// RTMP 配信 (OBS 互換の主配信)
    Stream,
    /// MP4 録画
    Record,
    /// HLS ライブ出力
    Hls,
    /// MPEG-DASH ライブ出力
    MpegDash,
    /// RTMP 再配信
    RtmpOutbound,
    /// Sora WebRTC Publisher
    Sora,
}

impl OutputKind {
    /// OBS WebSocket の outputKind 文字列からパースする
    pub(crate) fn from_kind_str(s: &str) -> Option<Self> {
        match s {
            "rtmp_output" => Some(Self::Stream),
            "mp4_output" => Some(Self::Record),
            "hls_output" => Some(Self::Hls),
            "mpeg_dash_output" => Some(Self::MpegDash),
            "rtmp_outbound_output" => Some(Self::RtmpOutbound),
            "sora_webrtc_output" => Some(Self::Sora),
            _ => None,
        }
    }

    /// OBS WebSocket の outputKind 文字列に変換する
    pub(crate) fn as_kind_str(self) -> &'static str {
        match self {
            Self::Stream => "rtmp_output",
            Self::Record => "mp4_output",
            Self::Hls => "hls_output",
            Self::MpegDash => "mpeg_dash_output",
            Self::RtmpOutbound => "rtmp_outbound_output",
            Self::Sora => "sora_webrtc_output",
        }
    }
}

/// output の種別固有の設定
pub(crate) enum OutputSettings {
    Stream(ObswsStreamServiceSettings),
    Record { record_directory: PathBuf },
    Hls(ObswsHlsSettings),
    MpegDash(ObswsDashSettings),
    RtmpOutbound(ObswsRtmpOutboundSettings),
    Sora(ObswsSoraPublisherSettings),
}

/// output の稼働中の実行情報
pub(crate) enum OutputRun {
    Stream(crate::obsws::input_registry::ObswsStreamRun),
    Record(crate::obsws::input_registry::ObswsRecordRun),
    Hls(crate::obsws::input_registry::ObswsHlsRun),
    MpegDash(crate::obsws::input_registry::ObswsDashRun),
    RtmpOutbound(crate::obsws::input_registry::ObswsRtmpOutboundRun),
    Sora(crate::obsws::input_registry::ObswsSoraPublisherRun),
}

// -----------------------------------------------------------------------
// デフォルト output の初期化
// -----------------------------------------------------------------------

/// 起動時のデフォルト output を生成する。
/// 各 output 設定はデフォルト値で初期化する。
pub(crate) fn create_default_outputs(record_directory: PathBuf) -> BTreeMap<String, OutputState> {
    let mut outputs = BTreeMap::new();
    outputs.insert(
        "stream".to_owned(),
        OutputState {
            output_kind: OutputKind::Stream,
            settings: OutputSettings::Stream(ObswsStreamServiceSettings::default()),
            runtime: OutputRuntimeState::default(),
        },
    );
    outputs.insert(
        "record".to_owned(),
        OutputState {
            output_kind: OutputKind::Record,
            settings: OutputSettings::Record { record_directory },
            runtime: OutputRuntimeState::default(),
        },
    );
    outputs.insert(
        "rtmp_outbound".to_owned(),
        OutputState {
            output_kind: OutputKind::RtmpOutbound,
            settings: OutputSettings::RtmpOutbound(ObswsRtmpOutboundSettings::default()),
            runtime: OutputRuntimeState::default(),
        },
    );
    outputs.insert(
        "sora".to_owned(),
        OutputState {
            output_kind: OutputKind::Sora,
            settings: OutputSettings::Sora(ObswsSoraPublisherSettings::default()),
            runtime: OutputRuntimeState::default(),
        },
    );
    outputs.insert(
        "hls".to_owned(),
        OutputState {
            output_kind: OutputKind::Hls,
            settings: OutputSettings::Hls(ObswsHlsSettings::default()),
            runtime: OutputRuntimeState::default(),
        },
    );
    outputs.insert(
        "mpeg_dash".to_owned(),
        OutputState {
            output_kind: OutputKind::MpegDash,
            settings: OutputSettings::MpegDash(ObswsDashSettings::default()),
            runtime: OutputRuntimeState::default(),
        },
    );
    outputs
}

// -----------------------------------------------------------------------
// output status レスポンス構築
// -----------------------------------------------------------------------

/// OutputState から GetOutputStatus / GetStreamStatus / GetRecordStatus 用の共通情報を取得する。
pub(crate) fn output_active_and_uptime(state: &OutputState) -> (bool, std::time::Duration) {
    let active = state.runtime.active;
    let duration = if active {
        state
            .runtime
            .started_at
            .map(|t| t.elapsed())
            .unwrap_or(std::time::Duration::ZERO)
    } else {
        std::time::Duration::ZERO
    };
    (active, duration)
}

/// OutputRun::Record から output_path を取得する。
pub(crate) fn record_output_path(state: &OutputState) -> Option<String> {
    state.runtime.run.as_ref().and_then(|r| match r {
        OutputRun::Record(run) => Some(run.output_path.display().to_string()),
        _ => None,
    })
}

// -----------------------------------------------------------------------
// GetOutputSettings / SetOutputSettings ハンドラ
// -----------------------------------------------------------------------

impl ObswsCoordinator {
    /// GetOutputSettings: outputName で指定された output の設定を返す。
    pub(crate) fn handle_get_output_settings(
        &self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> nojson::RawJsonOwned {
        let Some(state) = self.outputs.get(output_name) else {
            return crate::obsws::response::build_request_response_error(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Output not found",
            );
        };
        let output_kind = state.output_kind.as_kind_str();
        crate::obsws::response::build_request_response_success(request_type, request_id, |f| {
            f.member("outputName", output_name)?;
            f.member("outputKind", output_kind)?;
            f.member("outputSettings", OutputSettingsJson(&state.settings))
        })
    }

    /// GetOutputSettings リクエスト（outputName をリクエストデータからパース）
    pub(crate) fn handle_get_output_settings_request(
        &self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> nojson::RawJsonOwned {
        let Some(output_name) =
            super::parse_required_non_empty_string_field(request_data, "outputName")
        else {
            return crate::obsws::response::build_request_response_error(
                "GetOutputSettings",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            );
        };
        // player の特別扱い
        #[cfg(feature = "player")]
        if output_name == "player" {
            return crate::obsws::response::build_request_response_success(
                "GetOutputSettings",
                request_id,
                |f| f.member("outputSettings", nojson::object(|_| Ok(()))),
            );
        }
        self.handle_get_output_settings("GetOutputSettings", request_id, &output_name)
    }

    /// SetOutputSettings リクエスト
    pub(crate) fn handle_set_output_settings_request(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> nojson::RawJsonOwned {
        // 不正な型のフィールドに対する共通エラーレスポンス
        let invalid_field = |comment: &str| {
            crate::obsws::response::build_request_response_error(
                "SetOutputSettings",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                comment,
            )
        };
        let Some(request_data) = request_data else {
            return crate::obsws::response::build_request_response_error(
                "SetOutputSettings",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };
        let Some(output_name) = parse_required_string(request_data, "outputName") else {
            return crate::obsws::response::build_request_response_error(
                "SetOutputSettings",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputName field",
            );
        };
        let output_settings_raw = request_data
            .value()
            .to_member("outputSettings")
            .ok()
            .and_then(|v| v.optional());
        let Some(output_settings) = output_settings_raw else {
            return crate::obsws::response::build_request_response_error(
                "SetOutputSettings",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_FIELD,
                "Missing required outputSettings field",
            );
        };
        // player の特別扱い
        #[cfg(feature = "player")]
        if output_name == "player" {
            return crate::obsws::response::build_request_response_success_no_data(
                "SetOutputSettings",
                request_id,
            );
        }
        let Some(state) = self.outputs.get_mut(&output_name) else {
            return crate::obsws::response::build_request_response_error(
                "SetOutputSettings",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Output not found",
            );
        };
        // 種別に応じて settings を更新する。
        // 各フィールドは「キーが存在し値が non-null」なら更新、「値が null」なら None にクリア、
        // 「キーが存在しない」なら既存値を維持する。不正な型は INVALID_REQUEST_FIELD を返す。
        match &mut state.settings {
            OutputSettings::Stream(settings) => {
                if let Ok(v) = output_settings.to_member("streamServiceType")
                    && let Some(v) = v.optional()
                {
                    if v.kind().is_null() {
                        settings.stream_service_type =
                            crate::obsws::input_registry::OBSWS_DEFAULT_STREAM_SERVICE_TYPE
                                .to_owned();
                    } else {
                        match <String>::try_from(v) {
                            Ok(s) => settings.stream_service_type = s,
                            Err(_) => return invalid_field("streamServiceType must be a string"),
                        }
                    }
                }
                let ss = output_settings
                    .to_member("streamServiceSettings")
                    .ok()
                    .and_then(|v| v.optional());
                let source = ss.as_ref().unwrap_or(&output_settings);
                if let Ok(v) = source.to_member("server")
                    && let Some(v) = v.optional()
                {
                    if v.kind().is_null() {
                        settings.server = None;
                    } else {
                        match <String>::try_from(v) {
                            Ok(s) => settings.server = Some(s),
                            Err(_) => return invalid_field("server must be a string"),
                        }
                    }
                }
                if let Ok(v) = source.to_member("key")
                    && let Some(v) = v.optional()
                {
                    if v.kind().is_null() {
                        settings.key = None;
                    } else {
                        match <String>::try_from(v) {
                            Ok(s) => settings.key = Some(s),
                            Err(_) => return invalid_field("key must be a string"),
                        }
                    }
                }
            }
            OutputSettings::Record { record_directory } => {
                if let Ok(v) = output_settings.to_member("recordDirectory")
                    && let Some(v) = v.optional()
                {
                    if v.kind().is_null() {
                        return invalid_field("recordDirectory cannot be null");
                    }
                    match <String>::try_from(v) {
                        Ok(dir) => *record_directory = PathBuf::from(dir),
                        Err(_) => return invalid_field("recordDirectory must be a string"),
                    }
                }
            }
            OutputSettings::RtmpOutbound(settings) => {
                if let Ok(v) = output_settings.to_member("outputUrl")
                    && let Some(v) = v.optional()
                {
                    if v.kind().is_null() {
                        settings.output_url = None;
                    } else {
                        match <String>::try_from(v) {
                            Ok(url) => settings.output_url = Some(url),
                            Err(_) => return invalid_field("outputUrl must be a string"),
                        }
                    }
                }
                if let Ok(v) = output_settings.to_member("streamName")
                    && let Some(v) = v.optional()
                {
                    if v.kind().is_null() {
                        settings.stream_name = None;
                    } else {
                        match <String>::try_from(v) {
                            Ok(name) => settings.stream_name = Some(name),
                            Err(_) => return invalid_field("streamName must be a string"),
                        }
                    }
                }
            }
            OutputSettings::Sora(settings) => {
                if let Ok(v) = output_settings.to_member("soraSdkSettings")
                    && let Some(sdk) = v.optional()
                    && !sdk.kind().is_null()
                {
                    if let Ok(v) = sdk.to_member("signalingUrls")
                        && let Some(v) = v.optional()
                    {
                        if v.kind().is_null() {
                            settings.signaling_urls = Vec::new();
                        } else {
                            match <Vec<String>>::try_from(v) {
                                Ok(urls) => settings.signaling_urls = urls,
                                Err(_) => {
                                    return invalid_field(
                                        "signalingUrls must be an array of strings",
                                    );
                                }
                            }
                        }
                    }
                    if let Ok(v) = sdk.to_member("channelId")
                        && let Some(v) = v.optional()
                    {
                        if v.kind().is_null() {
                            settings.channel_id = None;
                        } else {
                            match <String>::try_from(v) {
                                Ok(ch) => settings.channel_id = Some(ch),
                                Err(_) => return invalid_field("channelId must be a string"),
                            }
                        }
                    }
                    if let Ok(v) = sdk.to_member("clientId")
                        && let Some(v) = v.optional()
                    {
                        if v.kind().is_null() {
                            settings.client_id = None;
                        } else {
                            match <String>::try_from(v) {
                                Ok(ci) => settings.client_id = Some(ci),
                                Err(_) => return invalid_field("clientId must be a string"),
                            }
                        }
                    }
                    if let Ok(v) = sdk.to_member("bundleId")
                        && let Some(v) = v.optional()
                    {
                        if v.kind().is_null() {
                            settings.bundle_id = None;
                        } else {
                            match <String>::try_from(v) {
                                Ok(bi) => settings.bundle_id = Some(bi),
                                Err(_) => return invalid_field("bundleId must be a string"),
                            }
                        }
                    }
                    if let Ok(v) = sdk.to_member("metadata")
                        && let Some(v) = v.optional()
                    {
                        if v.kind().is_null() {
                            settings.metadata = None;
                        } else if !v.kind().is_object() {
                            return invalid_field("metadata must be an object");
                        } else {
                            settings.metadata = Some(v.extract().into_owned());
                        }
                    }
                }
            }
            OutputSettings::Hls(settings) => {
                match crate::obsws::response::parse_hls_settings_update(&output_settings, settings)
                {
                    Ok(new_settings) => *settings = new_settings,
                    Err(error) => {
                        return crate::obsws::response::build_request_response_error(
                            "SetOutputSettings",
                            request_id,
                            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                            &error,
                        );
                    }
                }
            }
            OutputSettings::MpegDash(settings) => {
                match crate::obsws::response::parse_dash_settings_update(&output_settings, settings)
                {
                    Ok(new_settings) => *settings = new_settings,
                    Err(error) => {
                        return crate::obsws::response::build_request_response_error(
                            "SetOutputSettings",
                            request_id,
                            crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                            &error,
                        );
                    }
                }
            }
        }
        crate::obsws::response::build_request_response_success_no_data(
            "SetOutputSettings",
            request_id,
        )
    }

    /// SetStreamServiceSettings（stream 専用の設定変更）
    pub(crate) fn handle_set_stream_service_settings(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> nojson::RawJsonOwned {
        let fields = match crate::obsws::response::parse_request_data_or_error_response(
            "SetStreamServiceSettings",
            request_id,
            request_data,
            crate::obsws::response::parse_set_stream_service_settings_fields,
        ) {
            Ok(fields) => fields,
            Err(response) => return response,
        };
        let new_settings = ObswsStreamServiceSettings {
            stream_service_type: fields.stream_service_type,
            server: Some(fields.server),
            key: fields.key,
        };
        if let Some(output) = self.outputs.get_mut("stream") {
            output.settings = OutputSettings::Stream(new_settings);
        }
        crate::obsws::response::build_request_response_success_no_data(
            "SetStreamServiceSettings",
            request_id,
        )
    }

    /// GetStreamServiceSettings（stream 専用の OBS 互換レスポンス形式）
    pub(crate) fn handle_get_stream_service_settings(
        &self,
        request_id: &str,
    ) -> nojson::RawJsonOwned {
        let Some(state) = self.outputs.get("stream") else {
            return crate::obsws::response::build_request_response_error(
                "GetStreamServiceSettings",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Output not found",
            );
        };
        let OutputSettings::Stream(settings) = &state.settings else {
            return crate::obsws::response::build_request_response_error(
                "GetStreamServiceSettings",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Output is not a stream output",
            );
        };
        crate::obsws::response::build_request_response_success(
            "GetStreamServiceSettings",
            request_id,
            |f| {
                f.member("streamServiceType", &settings.stream_service_type)?;
                f.member(
                    "streamServiceSettings",
                    nojson::object(|f| {
                        f.member("bwtest", false)?;
                        if let Some(server) = &settings.server {
                            f.member("server", server)?;
                        }
                        f.member("key", settings.key.as_deref().unwrap_or(""))?;
                        f.member("use_auth", false)
                    }),
                )
            },
        )
    }

    /// GetRecordDirectory
    pub(crate) fn handle_get_record_directory(&self, request_id: &str) -> nojson::RawJsonOwned {
        let record_directory = self
            .outputs
            .get("record")
            .and_then(|o| match &o.settings {
                OutputSettings::Record { record_directory } => {
                    Some(record_directory.display().to_string())
                }
                _ => None,
            })
            .unwrap_or_default();
        crate::obsws::response::build_request_response_success(
            "GetRecordDirectory",
            request_id,
            |f| f.member("recordDirectory", &record_directory),
        )
    }

    /// SetRecordDirectory
    pub(crate) fn handle_set_record_directory(
        &mut self,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> nojson::RawJsonOwned {
        let fields = match crate::obsws::response::parse_request_data_or_error_response(
            "SetRecordDirectory",
            request_id,
            request_data,
            crate::obsws::response::parse_set_record_directory_fields,
        ) {
            Ok(fields) => fields,
            Err(response) => return response,
        };
        let record_directory =
            match crate::obsws::response::resolve_record_directory_path(&fields.record_directory) {
                Ok(path) => path,
                Err(e) => {
                    return crate::obsws::response::build_request_response_error(
                        "SetRecordDirectory",
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                        &e,
                    );
                }
            };
        if let Some(output) = self.outputs.get_mut("record") {
            output.settings = OutputSettings::Record {
                record_directory: record_directory.clone(),
            };
        }
        // HisuiCreateOutput で mp4_output を省略作成した場合の既定値も更新する
        self.default_record_directory = record_directory;
        crate::obsws::response::build_request_response_success_no_data(
            "SetRecordDirectory",
            request_id,
        )
    }
}

// -----------------------------------------------------------------------
// OutputSettings の JSON シリアライズ
// -----------------------------------------------------------------------

/// OutputSettings を JSON として出力するためのラッパー
struct OutputSettingsJson<'a>(&'a OutputSettings);

impl nojson::DisplayJson for OutputSettingsJson<'_> {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self.0 {
            OutputSettings::Stream(s) => s.fmt(f),
            OutputSettings::Record { record_directory } => nojson::object(|f| {
                f.member("recordDirectory", record_directory.display().to_string())
            })
            .fmt(f),
            OutputSettings::Hls(s) => s.fmt(f),
            OutputSettings::MpegDash(s) => s.fmt(f),
            OutputSettings::RtmpOutbound(s) => s.fmt(f),
            OutputSettings::Sora(s) => s.fmt(f),
        }
    }
}

// -----------------------------------------------------------------------
// HisuiCreateOutput / HisuiRemoveOutput ハンドラ
// -----------------------------------------------------------------------

/// state file の outputs セクションから outputs BTreeMap を復元する。
/// パースに失敗した output はスキップしてログに記録する。
pub(crate) fn restore_outputs_from_state(
    state_outputs: Vec<crate::obsws::state_file::StateFileOutput>,
) -> BTreeMap<String, OutputState> {
    let mut outputs = BTreeMap::new();
    for entry in state_outputs {
        let Some(kind) = OutputKind::from_kind_str(&entry.output_kind) else {
            tracing::warn!(
                "state file: unknown outputKind \"{}\" for output \"{}\"; skipping",
                entry.output_kind,
                entry.output_name,
            );
            continue;
        };
        let settings = match restore_output_settings(kind, &entry.output_settings) {
            Ok(s) => s,
            Err(msg) => {
                tracing::warn!(
                    "state file: failed to parse outputSettings for output \"{}\": {}; skipping",
                    entry.output_name,
                    msg,
                );
                continue;
            }
        };
        outputs.insert(
            entry.output_name,
            OutputState {
                output_kind: kind,
                settings,
                runtime: OutputRuntimeState::default(),
            },
        );
    }
    outputs
}

/// state file の outputSettings JSON から OutputSettings を復元する。
fn restore_output_settings(
    kind: OutputKind,
    raw: &nojson::RawJsonOwned,
) -> Result<OutputSettings, String> {
    let v = raw.value();
    match kind {
        OutputKind::Stream => {
            let mut settings = ObswsStreamServiceSettings::default();
            // state file の stream settings は StreamServiceSettings 形式
            let sst: Option<String> = v
                .to_member("streamServiceType")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok());
            if let Some(s) = sst {
                settings.stream_service_type = s;
            }
            // streamServiceSettings のネストもフラットも対応
            let ss = v
                .to_member("streamServiceSettings")
                .ok()
                .and_then(|v| v.optional());
            let source = ss.as_ref().unwrap_or(&v);
            let server: Option<String> = source
                .to_member("server")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok());
            settings.server = server;
            let key: Option<String> = source
                .to_member("key")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok());
            settings.key = key;
            Ok(OutputSettings::Stream(settings))
        }
        OutputKind::Record => {
            let dir: Option<String> = v
                .to_member("recordDirectory")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok());
            let record_directory = dir
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"));
            Ok(OutputSettings::Record { record_directory })
        }
        OutputKind::Hls => {
            // HLS 設定を state file から復元する
            let existing = ObswsHlsSettings::default();
            match crate::obsws::response::parse_hls_settings_update(&v, &existing) {
                Ok(settings) => Ok(OutputSettings::Hls(settings)),
                Err(_) => Ok(OutputSettings::Hls(existing)),
            }
        }
        OutputKind::MpegDash => {
            let existing = ObswsDashSettings::default();
            match crate::obsws::response::parse_dash_settings_update(&v, &existing) {
                Ok(settings) => Ok(OutputSettings::MpegDash(settings)),
                Err(_) => Ok(OutputSettings::MpegDash(existing)),
            }
        }
        OutputKind::RtmpOutbound => {
            let mut settings = ObswsRtmpOutboundSettings::default();
            let url: Option<String> = v
                .to_member("outputUrl")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok());
            settings.output_url = url;
            let name: Option<String> = v
                .to_member("streamName")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok());
            settings.stream_name = name;
            Ok(OutputSettings::RtmpOutbound(settings))
        }
        OutputKind::Sora => {
            let mut settings = ObswsSoraPublisherSettings::default();
            // soraSdkSettings のネストもトップレベルも対応
            let sdk = v
                .to_member("soraSdkSettings")
                .ok()
                .and_then(|v| v.optional());
            let source = sdk.as_ref().unwrap_or(&v);
            let urls: Vec<String> = source
                .to_member("signalingUrls")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok())
                .unwrap_or_default();
            settings.signaling_urls = urls;
            let ch: Option<String> = source
                .to_member("channelId")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok());
            settings.channel_id = ch;
            let ci: Option<String> = source
                .to_member("clientId")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok());
            settings.client_id = ci;
            let bi: Option<String> = source
                .to_member("bundleId")
                .ok()
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok());
            settings.bundle_id = bi;
            // metadata の復元（object のみ）
            if let Ok(m) = source.to_member("metadata")
                && let Some(v) = m.optional()
                && v.kind().is_object()
            {
                settings.metadata = Some(v.extract().into_owned());
            }
            Ok(OutputSettings::Sora(settings))
        }
    }
}

impl ObswsCoordinator {
    pub(crate) fn handle_create_output(
        &mut self,
        request_type: &str,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };

        // outputName のパース
        let output_name = match parse_required_string(request_data, "outputName") {
            Some(v) => v,
            None => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing or empty outputName field",
                );
            }
        };

        // outputKind のパース
        let kind_str = match parse_required_string(request_data, "outputKind") {
            Some(v) => v,
            None => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing or empty outputKind field",
                );
            }
        };
        let output_kind = match OutputKind::from_kind_str(&kind_str) {
            Some(k) => k,
            None => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &format!("Unknown outputKind: {kind_str}"),
                );
            }
        };

        // 名前の重複チェック
        if self.outputs.contains_key(&output_name) {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_ALREADY_EXISTS,
                &format!("Output already exists: {output_name}"),
            );
        }

        // outputSettings のパース（種別に応じたデフォルト値で初期化）
        let settings = match parse_output_settings(
            output_kind,
            request_data,
            &self.default_record_directory,
        ) {
            Ok(s) => s,
            Err(msg) => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    &msg,
                );
            }
        };

        // 登録
        self.outputs.insert(
            output_name,
            OutputState {
                output_kind,
                settings,
                runtime: OutputRuntimeState::default(),
            },
        );

        let response = crate::obsws::response::build_request_response_success_no_data(
            request_type,
            request_id,
        );
        self.build_result_from_response(response, Vec::new())
    }

    pub(crate) fn handle_remove_output(
        &mut self,
        request_type: &str,
        request_id: &str,
        request_data: Option<&nojson::RawJsonOwned>,
    ) -> CommandResult {
        let Some(request_data) = request_data else {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_DATA,
                "Missing required requestData field",
            );
        };

        let output_name = match parse_required_string(request_data, "outputName") {
            Some(v) => v,
            None => {
                return self.build_error_result(
                    request_type,
                    request_id,
                    REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing or empty outputName field",
                );
            }
        };

        // 存在チェック
        if !self.outputs.contains_key(&output_name) {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                &format!("Output not found: {output_name}"),
            );
        }

        // 稼働中の output は削除できない（先に StopOutput で停止する必要がある）
        let is_active = self
            .outputs
            .get(&output_name)
            .is_some_and(|o| o.runtime.active);
        if is_active {
            return self.build_error_result(
                request_type,
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                "Output is currently running. Stop it before removing.",
            );
        }

        self.outputs.remove(&output_name);

        let response = crate::obsws::response::build_request_response_success_no_data(
            request_type,
            request_id,
        );
        self.build_result_from_response(response, Vec::new())
    }
}

// -----------------------------------------------------------------------
// パースヘルパー
// -----------------------------------------------------------------------

fn parse_required_string(request_data: &nojson::RawJsonOwned, field: &str) -> Option<String> {
    let value: Option<String> = request_data
        .value()
        .to_member(field)
        .ok()?
        .try_into()
        .ok()?;
    let value = value?;
    if value.is_empty() {
        return None;
    }
    Some(value)
}

/// JSON メンバーから Option<String> を厳格に取得する。
/// キー不在 → Ok(None)、null → Ok(None)、string → Ok(Some(s))、それ以外 → Err
fn parse_optional_string_strict(
    v: &nojson::RawJsonValue<'_, '_>,
    field: &str,
    error_msg: &str,
) -> Result<Option<String>, String> {
    let Ok(member) = v.to_member(field) else {
        return Ok(None);
    };
    let Some(val) = member.optional() else {
        return Ok(None);
    };
    if val.kind().is_null() {
        return Ok(None);
    }
    match <String>::try_from(val) {
        Ok(s) => Ok(Some(s)),
        Err(_) => Err(error_msg.to_owned()),
    }
}

/// outputKind に応じて outputSettings をパースする。
/// outputSettings が省略された場合はデフォルト値を使用する。
/// 指定された値の型が不正な場合はエラーを返す。
fn parse_output_settings(
    kind: OutputKind,
    request_data: &nojson::RawJsonOwned,
    default_record_directory: &std::path::Path,
) -> Result<OutputSettings, String> {
    let json = request_data.value();
    let settings_value = json
        .to_member("outputSettings")
        .ok()
        .and_then(|v| v.optional());
    // outputSettings が存在する場合は object でなければエラー
    if let Some(ref v) = settings_value
        && !v.kind().is_object()
        && !v.kind().is_null()
    {
        return Err("outputSettings must be an object".to_owned());
    }

    match kind {
        OutputKind::Stream => {
            let mut settings = ObswsStreamServiceSettings::default();
            if let Some(v) = &settings_value {
                if let Some(s) = parse_optional_string_strict(
                    v,
                    "streamServiceType",
                    "streamServiceType must be a string",
                )? {
                    settings.stream_service_type = s;
                }
                let ss = v
                    .to_member("streamServiceSettings")
                    .ok()
                    .and_then(|v| v.optional());
                let source = ss.as_ref().unwrap_or(v);
                settings.server =
                    parse_optional_string_strict(source, "server", "server must be a string")?;
                settings.key = parse_optional_string_strict(source, "key", "key must be a string")?;
            }
            Ok(OutputSettings::Stream(settings))
        }
        OutputKind::Record => {
            if let Some(v) = &settings_value
                && let Ok(member) = v.to_member("recordDirectory")
                && let Some(val) = member.optional()
            {
                if val.kind().is_null() {
                    return Err("recordDirectory cannot be null".to_owned());
                }
                match <String>::try_from(val) {
                    Ok(dir) => {
                        return Ok(OutputSettings::Record {
                            record_directory: PathBuf::from(dir),
                        });
                    }
                    Err(_) => return Err("recordDirectory must be a string".to_owned()),
                }
            }
            Ok(OutputSettings::Record {
                record_directory: default_record_directory.to_path_buf(),
            })
        }
        OutputKind::Hls => {
            let existing = ObswsHlsSettings::default();
            if let Some(v) = &settings_value {
                crate::obsws::response::parse_hls_settings_update(v, &existing)
                    .map(OutputSettings::Hls)
            } else {
                Ok(OutputSettings::Hls(existing))
            }
        }
        OutputKind::MpegDash => {
            let existing = ObswsDashSettings::default();
            if let Some(v) = &settings_value {
                crate::obsws::response::parse_dash_settings_update(v, &existing)
                    .map(OutputSettings::MpegDash)
            } else {
                Ok(OutputSettings::MpegDash(existing))
            }
        }
        OutputKind::RtmpOutbound => {
            let mut settings = ObswsRtmpOutboundSettings::default();
            if let Some(v) = &settings_value {
                settings.output_url =
                    parse_optional_string_strict(v, "outputUrl", "outputUrl must be a string")?;
                settings.stream_name =
                    parse_optional_string_strict(v, "streamName", "streamName must be a string")?;
            }
            Ok(OutputSettings::RtmpOutbound(settings))
        }
        OutputKind::Sora => {
            let mut settings = ObswsSoraPublisherSettings::default();
            if let Some(v) = &settings_value {
                let sdk = v
                    .to_member("soraSdkSettings")
                    .ok()
                    .and_then(|v| v.optional())
                    .filter(|v| !v.kind().is_null());
                let source = sdk.as_ref().unwrap_or(v);
                // signalingUrls
                if let Ok(member) = source.to_member("signalingUrls")
                    && let Some(val) = member.optional()
                    && !val.kind().is_null()
                {
                    settings.signaling_urls = <Vec<String>>::try_from(val)
                        .map_err(|_| "signalingUrls must be an array of strings".to_owned())?;
                }
                settings.channel_id = parse_optional_string_strict(
                    source,
                    "channelId",
                    "channelId must be a string",
                )?;
                settings.client_id =
                    parse_optional_string_strict(source, "clientId", "clientId must be a string")?;
                settings.bundle_id =
                    parse_optional_string_strict(source, "bundleId", "bundleId must be a string")?;
                // metadata（object のみ）
                if let Ok(member) = source.to_member("metadata")
                    && let Some(val) = member.optional()
                    && !val.kind().is_null()
                {
                    if !val.kind().is_object() {
                        return Err("metadata must be an object".to_owned());
                    }
                    settings.metadata = Some(val.extract().into_owned());
                }
            }
            Ok(OutputSettings::Sora(settings))
        }
    }
}
