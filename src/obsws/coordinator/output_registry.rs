//! output の統一管理。
//! 全 output を名前付きインスタンスとして BTreeMap で管理し、
//! HisuiCreateOutput / HisuiRemoveOutput で動的に追加・削除する。
//!
//! TODO: output 関連型の所属を再整理する
//!
//! 現状は output の Settings / Run / Destination / Variant 型の一部が
//! coordinator 配下にあり、state_file / writer / response など
//! coordinator 外のモジュールもそれらを参照している。
//!
//! そのため output_registry は output 集合の管理に加えて、
//! 外部参照用の型の入口（re-export）も兼ねている。
//!
//! 将来的には以下の形へ整理する:
//! - output 関連の Settings / Destination / Variant / 設定パースを
//!   `obsws::output::*` のような coordinator 外の層へ移す
//! - 必要なら Run 型も同じ層へ移し、coordinator は start / stop と
//!   processor 制御に専念させる
//! - それにより output_registry から re-export を取り除き、
//!   registry 本来の責務に限定する

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use super::{CommandResult, ObswsCoordinator};
use crate::obsws::protocol::{
    REQUEST_STATUS_INVALID_REQUEST_FIELD, REQUEST_STATUS_MISSING_REQUEST_FIELD,
    REQUEST_STATUS_RESOURCE_ALREADY_EXISTS, REQUEST_STATUS_RESOURCE_NOT_FOUND,
};
use crate::{ProcessorId, TrackId};

// output モジュールの Settings 型を re-export する
pub(crate) use super::output_dash::{
    DEFAULT_DASH_MAX_RETAINED_SEGMENTS, DEFAULT_DASH_SEGMENT_DURATION_SECS, DashDestination,
    DashVariant, ObswsDashSettings,
};
pub(crate) use super::output_hls::{
    DEFAULT_HLS_MAX_RETAINED_SEGMENTS, DEFAULT_HLS_SEGMENT_DURATION_SECS, HlsDestination,
    HlsSegmentFormat, HlsVariant, ObswsHlsSettings,
};
pub(crate) use super::output_rtmp::ObswsRtmpOutboundSettings;
pub(crate) use super::output_sora::ObswsSoraPublisherSettings;
pub(crate) use super::output_stream::ObswsStreamServiceSettings;

// -----------------------------------------------------------------------
// 型定義
// -----------------------------------------------------------------------

/// output の共通トラック実行情報。
/// Record / Stream / HLS / DASH / RTMP 各 output でエンコーダー + トラックの組として使われる。
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
    /// SDL3 ウィンドウ表示（ビルトイン output）
    #[cfg(feature = "player")]
    Player,
}

impl OutputKind {
    /// OBS WebSocket の outputKind 文字列からパースする。
    /// ビルトイン output（Player 等）はここでは返さない。
    /// ビルトイン output は create_default_outputs() でのみ作成される。
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
            #[cfg(feature = "player")]
            Self::Player => "player_output",
        }
    }
}

/// output の種別固有の設定
pub(crate) enum OutputSettings {
    Stream(ObswsStreamServiceSettings),
    Record(super::output_record::RecordOutputSettings),
    Hls(ObswsHlsSettings),
    MpegDash(ObswsDashSettings),
    RtmpOutbound(ObswsRtmpOutboundSettings),
    Sora(ObswsSoraPublisherSettings),
    /// Player は設定を持たない
    #[cfg(feature = "player")]
    Player,
}

/// output の稼働中の実行情報
pub(crate) enum OutputRun {
    Stream(super::output_stream::ObswsStreamRun),
    Record(super::output_record::ObswsRecordRun),
    Hls(super::output_hls::ObswsHlsRun),
    MpegDash(super::output_dash::ObswsDashRun),
    RtmpOutbound(super::output_rtmp::ObswsRtmpOutboundRun),
    Sora(super::output_sora::ObswsSoraPublisherRun),
    #[cfg(feature = "player")]
    Player {
        subscriber_handle: Option<tokio::task::JoinHandle<()>>,
    },
}

// -----------------------------------------------------------------------
// デフォルト output の初期化
// -----------------------------------------------------------------------

/// 起動時のデフォルト output を生成する。
/// OBS 互換として stream と record を自動作成する。
/// player feature 有効時は player も自動作成する。
/// hls / mpeg_dash / sora / rtmp_outbound は HisuiCreateOutput で明示的に作成する。
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
            settings: OutputSettings::Record(super::output_record::RecordOutputSettings {
                record_directory,
            }),
            runtime: OutputRuntimeState::default(),
        },
    );
    #[cfg(feature = "player")]
    outputs.insert(
        "player".to_owned(),
        OutputState {
            output_kind: OutputKind::Player,
            settings: OutputSettings::Player,
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
        let output_settings = match request_data.value().to_member("outputSettings") {
            Ok(v) => match v.optional() {
                Some(v) if v.kind().is_object() => v,
                Some(_) => return invalid_field("outputSettings must be an object"),
                None => return invalid_field("outputSettings must be an object"),
            },
            Err(_) => {
                return crate::obsws::response::build_request_response_error(
                    "SetOutputSettings",
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_MISSING_REQUEST_FIELD,
                    "Missing required outputSettings field",
                );
            }
        };
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
            OutputSettings::Stream(s) => {
                if let Err(e) = s.update_from_json(&output_settings) {
                    return invalid_field(&e);
                }
            }
            OutputSettings::Record(s) => {
                if let Err(e) = s.update_from_json(&output_settings) {
                    return invalid_field(&e);
                }
            }
            OutputSettings::RtmpOutbound(s) => {
                if let Err(e) = s.update_from_json(&output_settings) {
                    return invalid_field(&e);
                }
            }
            OutputSettings::Sora(s) => {
                if let Err(e) = s.update_from_json(&output_settings) {
                    return invalid_field(&e);
                }
            }
            OutputSettings::Hls(settings) => {
                match super::output_hls::parse_hls_settings_update(&output_settings, settings) {
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
                match super::output_dash::parse_dash_settings_update(&output_settings, settings) {
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
            // Player は設定を持たないため何もしない
            #[cfg(feature = "player")]
            OutputSettings::Player => {}
        }
        // record の recordDirectory 更新時は default_record_directory も同期する
        if output_name == "record"
            && let Some(state) = self.outputs.get("record")
            && let OutputSettings::Record(s) = &state.settings
        {
            self.default_record_directory = s.record_directory.clone();
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
        let Some(output) = self.outputs.get_mut("stream") else {
            return crate::obsws::response::build_request_response_error(
                "SetStreamServiceSettings",
                request_id,
                crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                "Output not found",
            );
        };
        output.settings = OutputSettings::Stream(new_settings);
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
                OutputSettings::Record(s) => Some(s.record_directory.display().to_string()),
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
            output.settings = OutputSettings::Record(super::output_record::RecordOutputSettings {
                record_directory: record_directory.clone(),
            });
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
            OutputSettings::Record(s) => s.fmt(f),
            OutputSettings::Hls(s) => s.fmt(f),
            OutputSettings::MpegDash(s) => s.fmt(f),
            OutputSettings::RtmpOutbound(s) => s.fmt(f),
            OutputSettings::Sora(s) => s.fmt(f),
            #[cfg(feature = "player")]
            OutputSettings::Player => {
                // Player は設定を持たないため空オブジェクトを返す
                Ok(())
            }
        }
    }
}

// -----------------------------------------------------------------------
// HisuiCreateOutput / HisuiRemoveOutput ハンドラ
// -----------------------------------------------------------------------

/// state file の outputs セクションから outputs BTreeMap を復元する。
/// パースに失敗した場合は起動失敗とする。
pub(crate) fn restore_outputs_from_state(
    state_outputs: Vec<crate::obsws::state_file::StateFileOutput>,
) -> Result<BTreeMap<String, OutputState>, crate::Error> {
    let mut outputs = BTreeMap::new();
    for entry in state_outputs {
        let Some(kind) = OutputKind::from_kind_str(&entry.output_kind) else {
            return Err(crate::Error::new(format!(
                "state file: unknown outputKind \"{}\" for output \"{}\"",
                entry.output_kind, entry.output_name,
            )));
        };
        let settings = match restore_output_settings(kind, &entry.output_settings) {
            Ok(s) => s,
            Err(msg) => {
                // TODO: src/json.rs のような行・列付きコンテキスト表示を state file の
                // outputSettings 復元エラーにも付与したいが、現状は String ベースの
                // パーサが多く位置情報を十分に保持していないため後続タスクで対応する。
                return Err(crate::Error::new(format!(
                    "state file: failed to parse outputSettings for output \"{}\" (kind: \"{}\"): {}",
                    entry.output_name, entry.output_kind, msg,
                )));
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
    // ビルトイン output を追加する（state file には保存されないため、常にここで挿入する）
    #[cfg(feature = "player")]
    outputs.insert(
        "player".to_owned(),
        OutputState {
            output_kind: OutputKind::Player,
            settings: OutputSettings::Player,
            runtime: OutputRuntimeState::default(),
        },
    );
    Ok(outputs)
}

/// state file の outputSettings JSON から OutputSettings を復元する。
fn restore_output_settings(
    kind: OutputKind,
    raw: &nojson::RawJsonOwned,
) -> Result<OutputSettings, String> {
    let v = raw.value();
    match kind {
        OutputKind::Stream => Ok(OutputSettings::Stream(
            ObswsStreamServiceSettings::parse_from_json(Some(&v))?,
        )),
        OutputKind::Record => {
            // state file 復元時のデフォルトは /tmp（state file にディレクトリ情報がない場合のフォールバック）
            Ok(OutputSettings::Record(
                super::output_record::RecordOutputSettings::parse_from_json(
                    Some(&v),
                    std::path::Path::new("/tmp"),
                )?,
            ))
        }
        OutputKind::Hls => {
            // HLS 設定を state file から復元する
            let existing = ObswsHlsSettings::default();
            match super::output_hls::parse_hls_settings_update(&v, &existing) {
                Ok(settings) => Ok(OutputSettings::Hls(settings)),
                Err(_) => Ok(OutputSettings::Hls(existing)),
            }
        }
        OutputKind::MpegDash => {
            let existing = ObswsDashSettings::default();
            match super::output_dash::parse_dash_settings_update(&v, &existing) {
                Ok(settings) => Ok(OutputSettings::MpegDash(settings)),
                Err(_) => Ok(OutputSettings::MpegDash(existing)),
            }
        }
        OutputKind::RtmpOutbound => Ok(OutputSettings::RtmpOutbound(
            ObswsRtmpOutboundSettings::parse_from_json(Some(&v))?,
        )),
        OutputKind::Sora => Ok(OutputSettings::Sora(
            ObswsSoraPublisherSettings::parse_from_json(Some(&v))?,
        )),
        // Player は from_kind_str で返されないため到達しない
        #[cfg(feature = "player")]
        OutputKind::Player => unreachable!(),
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
        let Some(state) = self.outputs.get(&output_name) else {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_RESOURCE_NOT_FOUND,
                &format!("Output not found: {output_name}"),
            );
        };

        // ビルトイン output は削除できない
        #[cfg(feature = "player")]
        if state.output_kind == OutputKind::Player {
            return self.build_error_result(
                request_type,
                request_id,
                REQUEST_STATUS_INVALID_REQUEST_FIELD,
                "Cannot remove builtin output",
            );
        }

        // 稼働中の output は削除できない（先に StopOutput で停止する必要がある）
        if state.runtime.active {
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
pub(super) fn parse_optional_string_strict(
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

/// オブジェクトのメンバーから非空文字列を取得する。
/// キー不在 → Ok(None)、null/空文字列 → Ok(None)、string → Ok(Some(s))、それ以外 → Err
///
/// HLS/DASH パーサ間で共有される。
pub(super) fn optional_non_empty_string_member(
    object: nojson::RawJsonValue<'_, '_>,
    member_name: &str,
) -> Result<Option<String>, nojson::JsonParseError> {
    let value = object.to_member(member_name)?.optional();
    let Some(value) = value else {
        return Ok(None);
    };
    let value: String = value.try_into()?;
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some(value))
}

/// OBS WebSocket 互換の S3 destination をパースした中間表現。
/// HLS/DASH の両方のパーサで使用する。
pub(super) struct ParsedObswsS3Destination {
    pub(super) bucket: String,
    pub(super) prefix: String,
    pub(super) region: String,
    pub(super) endpoint: Option<String>,
    pub(super) use_path_style: bool,
    pub(super) access_key_id: String,
    pub(super) secret_access_key: String,
    pub(super) session_token: Option<String>,
    pub(super) lifetime_days: Option<u32>,
}

/// `destination` オブジェクトから S3 互換の出力先情報をパースする。
/// HLS/DASH のパーサで共通に使用される。
pub(super) fn parse_obsws_s3_destination(
    dest_value: nojson::RawJsonValue<'_, '_>,
) -> Result<ParsedObswsS3Destination, String> {
    let bucket: String = dest_value
        .to_member("bucket")
        .map_err(|e| e.to_string())?
        .required()
        .map_err(|_| "destination.bucket is required for s3".to_owned())?
        .try_into()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;
    let prefix: String = optional_non_empty_string_member(dest_value, "prefix")
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let region: String = dest_value
        .to_member("region")
        .map_err(|e| e.to_string())?
        .required()
        .map_err(|_| "destination.region is required for s3".to_owned())?
        .try_into()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;
    let endpoint: Option<String> =
        optional_non_empty_string_member(dest_value, "endpoint").map_err(|e| e.to_string())?;
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
        optional_non_empty_string_member(creds_value, "sessionToken").map_err(|e| e.to_string())?;

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
        OutputKind::Stream => Ok(OutputSettings::Stream(
            ObswsStreamServiceSettings::parse_from_json(settings_value.as_ref())?,
        )),
        OutputKind::Record => Ok(OutputSettings::Record(
            super::output_record::RecordOutputSettings::parse_from_json(
                settings_value.as_ref(),
                default_record_directory,
            )?,
        )),
        OutputKind::Hls => {
            let existing = ObswsHlsSettings::default();
            if let Some(v) = &settings_value {
                super::output_hls::parse_hls_settings_update(v, &existing).map(OutputSettings::Hls)
            } else {
                Ok(OutputSettings::Hls(existing))
            }
        }
        OutputKind::MpegDash => {
            let existing = ObswsDashSettings::default();
            if let Some(v) = &settings_value {
                super::output_dash::parse_dash_settings_update(v, &existing)
                    .map(OutputSettings::MpegDash)
            } else {
                Ok(OutputSettings::MpegDash(existing))
            }
        }
        OutputKind::RtmpOutbound => Ok(OutputSettings::RtmpOutbound(
            ObswsRtmpOutboundSettings::parse_from_json(settings_value.as_ref())?,
        )),
        OutputKind::Sora => Ok(OutputSettings::Sora(
            ObswsSoraPublisherSettings::parse_from_json(settings_value.as_ref())?,
        )),
        // Player は from_kind_str で返されないため到達しない
        #[cfg(feature = "player")]
        OutputKind::Player => unreachable!(),
    }
}
