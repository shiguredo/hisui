//! output の統一管理。
//! 全 output を名前付きインスタンスとして BTreeMap で管理し、
//! HisuiCreateOutput / HisuiRemoveOutput で動的に追加・削除する。

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
    #[expect(dead_code, reason = "Phase 2 以降で参照予定")]
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
    #[expect(dead_code, reason = "Phase 2 以降で参照予定")]
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
    #[expect(dead_code, reason = "Phase 2 以降で使用予定")]
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
    Record {
        record_directory: PathBuf,
    },
    #[expect(dead_code, reason = "Phase 2 以降で参照予定")]
    Hls(ObswsHlsSettings),
    #[expect(dead_code, reason = "Phase 2 以降で参照予定")]
    MpegDash(ObswsDashSettings),
    RtmpOutbound(ObswsRtmpOutboundSettings),
    Sora(ObswsSoraPublisherSettings),
}

/// output の稼働中の実行情報
pub(crate) enum OutputRun {
    Stream(crate::obsws::input_registry::ObswsStreamRun),
    Record(crate::obsws::input_registry::ObswsRecordRun),
    #[expect(dead_code, reason = "Phase 2 以降で使用予定")]
    Hls(crate::obsws::input_registry::ObswsHlsRun),
    #[expect(dead_code, reason = "Phase 2 以降で使用予定")]
    MpegDash(crate::obsws::input_registry::ObswsDashRun),
    RtmpOutbound(crate::obsws::input_registry::ObswsRtmpOutboundRun),
    Sora(crate::obsws::input_registry::ObswsSoraPublisherRun),
}

// -----------------------------------------------------------------------
// デフォルト output の初期化
// -----------------------------------------------------------------------

/// 起動時のデフォルト output を生成する。
/// input_registry の既存設定を初期値として使用する。
pub(crate) fn create_default_outputs(
    registry: &crate::obsws::input_registry::ObswsInputRegistry,
) -> BTreeMap<String, OutputState> {
    let mut outputs = BTreeMap::new();
    outputs.insert(
        "stream".to_owned(),
        OutputState {
            output_kind: OutputKind::Stream,
            settings: OutputSettings::Stream(registry.stream_service_settings().clone()),
            runtime: OutputRuntimeState::default(),
        },
    );
    outputs.insert(
        "record".to_owned(),
        OutputState {
            output_kind: OutputKind::Record,
            settings: OutputSettings::Record {
                record_directory: registry.record_directory().to_path_buf(),
            },
            runtime: OutputRuntimeState::default(),
        },
    );
    outputs
}

// -----------------------------------------------------------------------
// HisuiCreateOutput / HisuiRemoveOutput ハンドラ
// -----------------------------------------------------------------------

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
        let settings = match parse_output_settings(output_kind, request_data) {
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

    pub(crate) async fn handle_remove_output(
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

        // 稼働中なら停止する
        let is_active = self
            .outputs
            .get(&output_name)
            .is_some_and(|o| o.runtime.active);
        if is_active {
            // TODO: Phase 2 以降で各 output kind の停止処理を実装
            // 現時点ではエラーを返す
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

/// outputKind に応じて outputSettings をパースする。
/// outputSettings が省略された場合はデフォルト値を使用する。
fn parse_output_settings(
    kind: OutputKind,
    request_data: &nojson::RawJsonOwned,
) -> Result<OutputSettings, String> {
    let json = request_data.value();
    // outputSettings フィールドの取得（オプション）
    let settings_value = json
        .to_member("outputSettings")
        .ok()
        .and_then(|v| v.optional());

    match kind {
        OutputKind::Stream => {
            let mut settings = ObswsStreamServiceSettings::default();
            if let Some(v) = &settings_value {
                let server: Option<String> = v
                    .to_member("server")
                    .ok()
                    .and_then(|v| v.optional())
                    .and_then(|v| v.try_into().ok());
                settings.server = server;
                let key: Option<String> = v
                    .to_member("key")
                    .ok()
                    .and_then(|v| v.optional())
                    .and_then(|v| v.try_into().ok());
                settings.key = key;
                let sst: Option<String> = v
                    .to_member("streamServiceType")
                    .ok()
                    .and_then(|v| v.optional())
                    .and_then(|v| v.try_into().ok());
                if let Some(s) = sst {
                    settings.stream_service_type = s;
                }
            }
            Ok(OutputSettings::Stream(settings))
        }
        OutputKind::Record => {
            let record_directory: Option<String> = settings_value
                .as_ref()
                .and_then(|v| v.to_member("recordDirectory").ok())
                .and_then(|v| v.optional())
                .and_then(|v| v.try_into().ok());
            let record_directory = record_directory
                .map(PathBuf::from)
                .ok_or("outputSettings with recordDirectory is required for mp4_output")?;
            Ok(OutputSettings::Record { record_directory })
        }
        OutputKind::Hls => {
            // HLS 設定はデフォルト値で初期化し、SetOutputSettings で後から変更可能
            let settings = ObswsHlsSettings::default();
            Ok(OutputSettings::Hls(settings))
        }
        OutputKind::MpegDash => {
            let settings = ObswsDashSettings::default();
            Ok(OutputSettings::MpegDash(settings))
        }
        OutputKind::RtmpOutbound => {
            let mut settings = ObswsRtmpOutboundSettings::default();
            if let Some(v) = &settings_value {
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
            }
            Ok(OutputSettings::RtmpOutbound(settings))
        }
        OutputKind::Sora => {
            let mut settings = ObswsSoraPublisherSettings::default();
            if let Some(v) = &settings_value {
                let urls: Vec<String> = v
                    .to_member("signalingUrls")
                    .ok()
                    .and_then(|v| v.optional())
                    .and_then(|v| v.try_into().ok())
                    .unwrap_or_default();
                settings.signaling_urls = urls;
                let ch: Option<String> = v
                    .to_member("channelId")
                    .ok()
                    .and_then(|v| v.optional())
                    .and_then(|v| v.try_into().ok());
                settings.channel_id = ch;
                let ci: Option<String> = v
                    .to_member("clientId")
                    .ok()
                    .and_then(|v| v.optional())
                    .and_then(|v| v.try_into().ok());
                settings.client_id = ci;
                let bi: Option<String> = v
                    .to_member("bundleId")
                    .ok()
                    .and_then(|v| v.optional())
                    .and_then(|v| v.try_into().ok());
                settings.bundle_id = bi;
            }
            Ok(OutputSettings::Sora(settings))
        }
    }
}
