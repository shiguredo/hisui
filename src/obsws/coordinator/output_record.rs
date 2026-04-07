//! Record (MP4 録画) の output エンジン。
//! Program 出力を MP4 ファイルに録画するための processor 起動・停止を行う。

use std::time::Duration;

use super::ObswsCoordinator;
use super::output::{OutputOperationOutcome, terminate_and_wait, wait_or_terminate};

impl ObswsCoordinator {
    pub(crate) async fn handle_start_record(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{
            ActivateRecordError, ObswsRecordRun, ObswsRecordTrackRun,
        };
        use std::time::{SystemTime, UNIX_EPOCH};
        let run_id = match self.input_registry.next_record_run_id() {
            Ok(run_id) => run_id,
            Err(_) => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Record run ID overflow",
                    ),
                );
            }
        };
        let video = ObswsRecordTrackRun::new(
            "record",
            run_id,
            "video",
            &self.program_output.video_track_id,
        );
        let audio = ObswsRecordTrackRun::new(
            "record",
            run_id,
            "audio",
            &self.program_output.audio_track_id,
        );
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis();
        let output_path = self
            .input_registry
            .record_directory()
            .join(format!("obsws-record-{timestamp}.mp4"));
        let run = ObswsRecordRun {
            video,
            audio,
            writer_processor_id: crate::ProcessorId::new(format!(
                "output:record:mp4_writer:{run_id}"
            )),
            output_path: output_path.clone(),
        };
        if let Err(ActivateRecordError::AlreadyActive) =
            self.input_registry.activate_record(run.clone())
        {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "Record is already active",
                ),
            );
        }
        if let Some(parent) = output_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            self.input_registry.deactivate_record();
            let error_comment = format!("Failed to create record directory: {e}");
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            self.input_registry.deactivate_record();
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        let frame_rate = self.input_registry.frame_rate();
        if let Err(e) =
            start_record_processors(pipeline_handle, &output_path, &run, frame_rate).await
        {
            self.input_registry.deactivate_record();
            let _ = stop_processors_staged_record(pipeline_handle, &run).await;
            let error_comment = format!("Failed to start record: {}", e.display());
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        let output_path_str = output_path.display().to_string();
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_record_response(request_id),
            Some(output_path_str),
        )
    }

    pub(crate) async fn handle_stop_record(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        let run = match self.input_registry.record_run() {
            Some(run) => run.clone(),
            None => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                        "Record is not active",
                    ),
                );
            }
        };
        let output_path = run.output_path.display().to_string();
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_record(pipeline_handle, &run).await
        {
            // プロセッサ停止に失敗してもレコード状態は解除する。
            // MP4 ファイルの finalize を優先し、クライアントには成功を返す。
            tracing::warn!("failed to stop record processors: {}", e.display());
        }
        self.input_registry.deactivate_record();
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_record_response(request_id, &output_path),
            Some(output_path),
        )
    }
}

/// レコード用プロセッサを起動する: エンコーダー → MP4 ライター
/// program mixer の出力トラックを直接エンコーダーに入力するため、ミキサーとソースの起動は不要。
async fn start_record_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_path: &std::path::Path,
    run: &crate::obsws::input_registry::ObswsRecordRun,
    frame_rate: crate::video::FrameRate,
) -> crate::Result<()> {
    // レコードは Opus エンコーディングを使用する
    super::output::start_encoder_processors(
        pipeline_handle,
        &run.video,
        &run.audio,
        crate::types::CodecName::Opus,
        frame_rate,
    )
    .await?;
    crate::mp4::hybrid_writer::create_processor(
        pipeline_handle,
        output_path.to_path_buf(),
        Some(run.audio.encoded_track_id.clone()),
        Some(run.video.encoded_track_id.clone()),
        Some(run.writer_processor_id.clone()),
    )
    .await?;
    Ok(())
}

/// レコード用プロセッサを段階的に停止する。
/// エンコーダーを terminate し、ライターは EOS 伝播で自然終了させる。
async fn stop_processors_staged_record(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsRecordRun,
) -> crate::Result<()> {
    // NOTE:
    // この経路は terminate_processor() ベースで encoder を停止するため、
    // encoder の inner.finish() / drain を保証しない。
    // その結果、AAC や遅延出力を持つ video encoder では、
    // 停止直前の数サンプル / 数フレームが最終 MP4 に含まれない可能性がある。
    // また、encoder 停止完了直後でも writer 側の購読チャネルには終端付近の
    // データや Eos が未処理で残りうるが、現状はその時点で Finish RPC を送って
    // finalize を促すため、それらを読み切る前に末尾の一部を捨てるレースもある。
    // 現時点では StopRecord の応答性と実装単純性を優先し、この挙動を許容する。
    //
    // NOTE:
    // writer に Finish を送るのと同時に encoder へ非同期 finish RPC を送る方式は採用しない。
    // writer 側の Finish は入力トラックを即座に閉じるため、
    // encoder の drain 完了前に writer が finalize へ進み、かえって末尾欠損を固定化しうる。
    // 1. エンコーダーを停止して writer へ EOS を流す
    terminate_and_wait(
        pipeline_handle,
        &[
            run.video.encoder_processor_id.clone(),
            run.audio.encoder_processor_id.clone(),
        ],
    )
    .await?;

    // 2. 上流が止まった時点で writer に finalize を促す
    finish_mp4_writer_rpc(pipeline_handle, &run.writer_processor_id).await;

    // 3. writer の自然終了を待ち、タイムアウト時は強制停止
    wait_or_terminate(
        pipeline_handle,
        std::slice::from_ref(&run.writer_processor_id),
        Duration::from_secs(5),
    )
    .await?;

    Ok(())
}

/// MP4 writer に Finish RPC を送り、finalize を促す。
async fn finish_mp4_writer_rpc(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: &crate::ProcessorId,
) {
    const RETRY_TIMEOUT: Duration = Duration::from_millis(500);
    const RETRY_INTERVAL: Duration = Duration::from_millis(10);
    let deadline = tokio::time::Instant::now() + RETRY_TIMEOUT;

    loop {
        match pipeline_handle
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::mp4::writer::Mp4WriterRpcMessage,
            >>(processor_id)
            .await
        {
            Ok(sender) => {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                let _ =
                    sender.send(crate::mp4::writer::Mp4WriterRpcMessage::Finish { reply_tx });
                let _ = reply_rx.await;
                return;
            }
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(_) => {
                let _ = pipeline_handle
                    .terminate_processor(processor_id.clone())
                    .await;
                return;
            }
        }
    }
}
