//! Player (SDL3 ウィンドウ表示) の output エンジン。
//! Program 出力をリアルタイムで SDL3 ウィンドウに表示するための処理を行う。

use super::ObswsCoordinator;
use super::output::OutputOperationOutcome;
use super::output_dynamic::OutputRun;

impl ObswsCoordinator {
    pub(crate) async fn handle_start_player(
        &mut self,
        request_type: &str,
        request_id: &str,
    ) -> OutputOperationOutcome {
        if self.outputs.get("player").is_some_and(|o| o.runtime.active) {
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

        // active 状態に遷移する
        let player_state = self
            .outputs
            .get_mut("player")
            .expect("player output entry must exist");
        player_state.runtime.active = true;
        player_state.runtime.started_at = Some(std::time::Instant::now());
        player_state.runtime.run = Some(OutputRun::Player {
            subscriber_handle: None,
        });

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
            self.deactivate_player();
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
                self.deactivate_player();
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
                self.deactivate_player();
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
                // subscriber_handle を OutputRun::Player に保存する
                if let Some(OutputRun::Player {
                    subscriber_handle, ..
                }) = self
                    .outputs
                    .get_mut("player")
                    .and_then(|o| o.runtime.run.as_mut())
                {
                    *subscriber_handle = Some(handle);
                }
            }
            Ok(Err(message)) => {
                handle.abort();
                self.deactivate_player();
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
                self.deactivate_player();
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
        if !self.outputs.get("player").is_some_and(|o| o.runtime.active) {
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
        self.abort_player_subscriber();

        // メインスレッドにウィンドウ閉じを指示する
        let _ = self
            .player_command_tx
            .send(crate::obsws::player::PlayerCommand::Stop);

        self.deactivate_player();

        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }

    /// player の稼働状態を非アクティブにリセットする。
    pub(crate) fn deactivate_player(&mut self) {
        if let Some(state) = self.outputs.get_mut("player") {
            state.runtime.active = false;
            state.runtime.started_at = None;
            state.runtime.run = None;
        }
    }

    /// player の subscriber タスクを abort する。
    pub(crate) fn abort_player_subscriber(&mut self) {
        if let Some(OutputRun::Player {
            subscriber_handle, ..
        }) = self
            .outputs
            .get_mut("player")
            .and_then(|o| o.runtime.run.as_mut())
            && let Some(handle) = subscriber_handle.take()
        {
            handle.abort();
        }
    }
}
