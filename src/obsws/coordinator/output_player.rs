//! Player (SDL3 ウィンドウ表示) の output エンジン。
//! Program 出力をリアルタイムで SDL3 ウィンドウに表示するための処理を行う。

use super::ObswsCoordinator;
use super::output::OutputOperationOutcome;

impl ObswsCoordinator {
    pub(crate) async fn handle_start_player(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        if self.input_registry.is_player_active() {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "Player is already running",
                ),
            );
        }

        let canvas_width = self.input_registry.canvas_width().get() as i32;
        let canvas_height = self.input_registry.canvas_height().get() as i32;
        let pipeline_handle = match self.pipeline_handle.as_ref() {
            Some(h) => h.clone(),
            None => {
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Pipeline is not initialized",
                    ),
                );
            }
        };
        self.player_generation = self.player_generation.wrapping_add(1);
        let generation = self.player_generation;
        let (window_start_reply_tx, window_start_reply_rx) = tokio::sync::oneshot::channel();

        if let Err(()) = self.input_registry.activate_player() {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "Player is already running",
                ),
            );
        }

        // メインスレッドにウィンドウ作成を指示する
        if self
            .player_command_tx
            .send(crate::obsws::player::PlayerCommand::Start {
                canvas_width,
                canvas_height,
                generation,
                reply_tx: window_start_reply_tx,
            })
            .is_err()
        {
            self.input_registry.deactivate_player();
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Player control channel is closed",
                ),
            );
        }

        match window_start_reply_rx.await {
            Ok(Ok(())) => {}
            Ok(Err(message)) => {
                self.input_registry.deactivate_player();
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        &message,
                    ),
                );
            }
            Err(_) => {
                self.input_registry.deactivate_player();
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Player startup reply channel is closed",
                    ),
                );
            }
        }

        let video_track_id = self.program_output.video_track_id.clone();
        let audio_track_id = self.program_output.audio_track_id.clone();
        let media_tx = self.player_media_tx.clone();
        let (subscriber_startup_reply_tx, subscriber_startup_reply_rx) =
            tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            crate::obsws::player::run_player_subscriber(
                pipeline_handle,
                video_track_id,
                audio_track_id,
                media_tx,
                subscriber_startup_reply_tx,
            )
            .await;
        });
        match subscriber_startup_reply_rx.await {
            Ok(Ok(())) => {
                self.player_subscriber_handle = Some(handle);
            }
            Ok(Err(message)) => {
                handle.abort();
                self.input_registry.deactivate_player();
                let _ = self
                    .player_command_tx
                    .send(crate::obsws::player::PlayerCommand::Stop);
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        &message,
                    ),
                );
            }
            Err(_) => {
                handle.abort();
                self.input_registry.deactivate_player();
                let _ = self
                    .player_command_tx
                    .send(crate::obsws::player::PlayerCommand::Stop);
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        "Player subscriber startup reply channel is closed",
                    ),
                );
            }
        }

        OutputOperationOutcome::success(
            crate::obsws::response::build_start_output_response(request_id),
            None,
        )
    }

    pub(crate) async fn handle_stop_player(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        if !self.input_registry.is_player_active() {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "Player is not running",
                ),
            );
        }

        // サブスクライバタスクを停止する
        if let Some(handle) = self.player_subscriber_handle.take() {
            handle.abort();
        }

        // メインスレッドにウィンドウ閉じを指示する
        let _ = self
            .player_command_tx
            .send(crate::obsws::player::PlayerCommand::Stop);

        self.input_registry.deactivate_player();

        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }
}
