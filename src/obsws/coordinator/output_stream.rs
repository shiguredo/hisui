//! Stream (RTMP 配信) の output エンジン。
//! Program 出力を RTMP でライブ配信するための processor 起動・停止を行う。

use super::ObswsCoordinator;
use super::output::{OutputOperationOutcome, terminate_and_wait};
use super::output_registry::{ObswsRecordTrackRun, OutputRun, OutputSettings};
use crate::ProcessorId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsStreamRun {
    pub video: ObswsRecordTrackRun,
    pub audio: ObswsRecordTrackRun,
    pub publisher_processor_id: ProcessorId,
}

impl ObswsCoordinator {
    /// 指定された output_name の stream output を開始する。
    /// outputs BTreeMap から設定・ランタイム状態を読み書きする。
    pub(crate) async fn handle_start_stream(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        // output の存在チェックと設定取得
        let Some(output) = self.outputs.get(output_name) else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                ),
            );
        };
        let OutputSettings::Stream(stream_settings) = &output.settings else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Output is not a stream output",
                ),
            );
        };
        let stream_settings = stream_settings.clone();

        // 稼働中チェック
        if output.runtime.active {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_STREAM_RUNNING,
                    "Stream is already active",
                ),
            );
        }

        // rtmp_custom 以外は非対応
        if stream_settings.stream_service_type != "rtmp_custom" {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Unsupported streamServiceType field",
                ),
            );
        }
        let Some(output_url) = stream_settings.server else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing streamServiceSettings.server field",
                ),
            );
        };

        // run_id の発行
        let run_id = self.next_output_run_id;
        self.next_output_run_id = self.next_output_run_id.wrapping_add(1);

        let video = ObswsRecordTrackRun::new(
            output_name,
            run_id,
            "video",
            &self.program_output.video_track_id,
        );
        let audio = ObswsRecordTrackRun::new(
            output_name,
            run_id,
            "audio",
            &self.program_output.audio_track_id,
        );
        let run = ObswsStreamRun {
            video,
            audio,
            publisher_processor_id: crate::ProcessorId::new(format!(
                "output:{output_name}:rtmp_publisher:{run_id}"
            )),
        };

        // ランタイム状態を active にする
        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = true;
            output.runtime.started_at = Some(std::time::Instant::now());
            output.runtime.run = Some(OutputRun::Stream(run.clone()));
        }

        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            // ロールバック
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        let frame_rate = self.state.frame_rate();
        if let Err(e) = start_stream_processors(
            pipeline_handle,
            &output_url,
            stream_settings.key.as_deref(),
            &run,
            frame_rate,
        )
        .await
        {
            // ロールバック
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
            let _ = stop_processors_staged_stream(pipeline_handle, &run).await;
            let error_comment = format!("Failed to start stream: {}", e.display());
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_stream_response(request_id),
            None,
        )
    }

    /// 指定された output_name の stream output を停止する。
    pub(crate) async fn handle_stop_stream(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        // output から run を取得
        let run = self
            .outputs
            .get(output_name)
            .and_then(|o| o.runtime.run.as_ref())
            .and_then(|r| match r {
                OutputRun::Stream(run) => Some(run.clone()),
                _ => None,
            });
        let Some(run) = run else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_STREAM_NOT_RUNNING,
                    "Stream is not active",
                ),
            );
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_stream(pipeline_handle, &run).await
        {
            let error_comment = format!("Failed to stop stream: {}", e.display());
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        // ランタイム状態をリセット
        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = false;
            output.runtime.started_at = None;
            output.runtime.run = None;
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_stream_response(request_id),
            None,
        )
    }
}

// -----------------------------------------------------------------------
// ObswsStreamServiceSettings: stream service の設定型
// -----------------------------------------------------------------------

/// OBS WebSocket の StreamServiceSettings に対応する設定。
/// GetStreamServiceSettings / SetStreamServiceSettings で使用する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsStreamServiceSettings {
    pub stream_service_type: String,
    pub server: Option<String>,
    pub key: Option<String>,
}

/// ストリームサービスタイプのデフォルト値
pub(crate) const OBSWS_DEFAULT_STREAM_SERVICE_TYPE: &str = "rtmp_custom";

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

impl ObswsStreamServiceSettings {
    /// JSON から設定を更新する（SetOutputSettings 用）。
    /// 各フィールドは「キーが存在し値が non-null」なら更新、「値が null」なら既定値/None にクリア、
    /// 「キーが存在しない」なら既存値を維持する。
    pub(crate) fn update_from_json(
        &mut self,
        output_settings: &nojson::RawJsonValue<'_, '_>,
    ) -> Result<(), String> {
        if let Ok(v) = output_settings.to_member("streamServiceType")
            && let Some(v) = v.optional()
        {
            if v.kind().is_null() {
                self.stream_service_type = OBSWS_DEFAULT_STREAM_SERVICE_TYPE.to_owned();
            } else {
                match <String>::try_from(v) {
                    Ok(s) => self.stream_service_type = s,
                    Err(_) => return Err("streamServiceType must be a string".to_owned()),
                }
            }
        }
        let ss = output_settings
            .to_member("streamServiceSettings")
            .ok()
            .and_then(|v| v.optional());
        let source = ss.as_ref().unwrap_or(output_settings);
        if let Ok(v) = source.to_member("server")
            && let Some(v) = v.optional()
        {
            if v.kind().is_null() {
                self.server = None;
            } else {
                match <String>::try_from(v) {
                    Ok(s) => self.server = Some(s),
                    Err(_) => return Err("server must be a string".to_owned()),
                }
            }
        }
        if let Ok(v) = source.to_member("key")
            && let Some(v) = v.optional()
        {
            if v.kind().is_null() {
                self.key = None;
            } else {
                match <String>::try_from(v) {
                    Ok(s) => self.key = Some(s),
                    Err(_) => return Err("key must be a string".to_owned()),
                }
            }
        }
        Ok(())
    }

    /// JSON から設定をパースする（HisuiCreateOutput / state file 復元用）。
    pub(crate) fn parse_from_json(
        settings_value: Option<&nojson::RawJsonValue<'_, '_>>,
    ) -> Result<Self, String> {
        use super::output_registry::parse_optional_string_strict;

        let mut settings = Self::default();
        if let Some(v) = settings_value {
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
        Ok(settings)
    }
}

/// ストリーム用プロセッサを起動する: エンコーダー → パブリッシャー
/// program mixer の出力トラックを直接エンコーダーに入力するため、ミキサーとソースの起動は不要。
async fn start_stream_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_url: &str,
    stream_key: Option<&str>,
    run: &ObswsStreamRun,
    frame_rate: crate::video::FrameRate,
) -> crate::Result<()> {
    super::output::start_encoder_processors(
        pipeline_handle,
        &run.video,
        &run.audio,
        crate::types::CodecName::Aac,
        frame_rate,
    )
    .await?;
    // RTMP パブリッシャーを起動する
    let publisher = crate::rtmp::publisher::RtmpPublisher {
        output_url: output_url.to_owned(),
        stream_name: stream_key.map(|s| s.to_owned()),
        input_audio_track_id: Some(run.audio.encoded_track_id.clone()),
        input_video_track_id: Some(run.video.encoded_track_id.clone()),
        options: Default::default(),
    };
    crate::rtmp::publisher::create_processor(
        pipeline_handle,
        publisher,
        Some(run.publisher_processor_id.clone()),
    )
    .await?;
    Ok(())
}

/// ストリーム用プロセッサを段階的に停止する: エンコーダー → パブリッシャー
async fn stop_processors_staged_stream(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &ObswsStreamRun,
) -> crate::Result<()> {
    terminate_and_wait(
        pipeline_handle,
        &[
            run.video.encoder_processor_id.clone(),
            run.audio.encoder_processor_id.clone(),
        ],
    )
    .await?;
    terminate_and_wait(
        pipeline_handle,
        std::slice::from_ref(&run.publisher_processor_id),
    )
    .await?;
    Ok(())
}
