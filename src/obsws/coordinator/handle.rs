//! ObswsCoordinatorHandle を定義するモジュール。
//! coordinator actor にコマンドを送信するための非同期 RPC インターフェースを提供する。
//! セッションや bootstrap ハンドラがこのハンドルを保持する。

use super::{
    BatchCommandResult, BootstrapInputEvent, BootstrapInputSnapshot, CommandResult,
    ObswsCoordinatorCommand, ProgramTrackIds, ResolvedInputInfo,
};
use crate::obsws::event::TaggedEvent;
use crate::obsws::message::ObswsSessionStats;

/// coordinator への handle。セッションや bootstrap が保持する。
#[derive(Clone)]
pub struct ObswsCoordinatorHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<ObswsCoordinatorCommand>,
    program_track_ids: ProgramTrackIds,
    bootstrap_event_tx: tokio::sync::broadcast::Sender<BootstrapInputEvent>,
    obsws_event_tx: tokio::sync::broadcast::Sender<TaggedEvent>,
}

impl ObswsCoordinatorHandle {
    pub(super) fn new(
        command_tx: tokio::sync::mpsc::UnboundedSender<ObswsCoordinatorCommand>,
        program_track_ids: ProgramTrackIds,
        bootstrap_event_tx: tokio::sync::broadcast::Sender<BootstrapInputEvent>,
        obsws_event_tx: tokio::sync::broadcast::Sender<TaggedEvent>,
    ) -> Self {
        Self {
            command_tx,
            program_track_ids,
            bootstrap_event_tx,
            obsws_event_tx,
        }
    }

    /// 単一リクエストを actor に送信し、結果を待つ
    pub async fn process_request(
        &self,
        request: crate::obsws::message::RequestMessage,
        session_stats: ObswsSessionStats,
    ) -> crate::Result<CommandResult> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send(ObswsCoordinatorCommand::ProcessRequest {
                request,
                session_stats,
                reply_tx,
            })
            .map_err(|_| crate::Error::new("coordinator has terminated"))?;
        reply_rx
            .await
            .map_err(|_| crate::Error::new("coordinator dropped reply channel"))
    }

    /// RequestBatch を actor に送信し、結果を待つ
    pub async fn process_request_batch(
        &self,
        requests: Vec<crate::obsws::message::RequestMessage>,
        session_stats: ObswsSessionStats,
        halt_on_failure: bool,
    ) -> crate::Result<BatchCommandResult> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send(ObswsCoordinatorCommand::ProcessRequestBatch {
                requests,
                session_stats,
                halt_on_failure,
                reply_tx,
            })
            .map_err(|_| crate::Error::new("coordinator has terminated"))?;
        reply_rx
            .await
            .map_err(|_| crate::Error::new("coordinator dropped reply channel"))
    }

    /// coordinator が保持する固定 Program 出力の video track ID を取得する
    pub fn program_video_track_id(&self) -> crate::TrackId {
        self.program_track_ids.video_track_id.clone()
    }

    /// coordinator が保持する固定 Program 出力の audio track ID を取得する
    pub fn program_audio_track_id(&self) -> crate::TrackId {
        self.program_track_ids.audio_track_id.clone()
    }

    /// bootstrap 用の入力 snapshot を取得する
    pub async fn get_bootstrap_snapshot(&self) -> crate::Result<Vec<BootstrapInputSnapshot>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send(ObswsCoordinatorCommand::GetBootstrapSnapshot { reply_tx })
            .map_err(|_| crate::Error::new("coordinator has terminated"))?;
        reply_rx
            .await
            .map_err(|_| crate::Error::new("coordinator dropped reply channel"))
    }

    /// webrtc_source の settings を取得する
    pub async fn get_webrtc_source_settings(
        &self,
        input_name: &str,
    ) -> crate::Result<Option<crate::obsws::input_registry::ObswsWebRtcSourceSettings>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send(ObswsCoordinatorCommand::GetWebRtcSourceSettings {
                input_name: input_name.to_owned(),
                reply_tx,
            })
            .map_err(|_| crate::Error::new("coordinator has terminated"))?;
        reply_rx
            .await
            .map_err(|_| crate::Error::new("coordinator dropped reply channel"))
    }

    /// webrtc_source の trackId を更新する
    pub fn update_webrtc_source_track_id(&self, input_name: &str, track_id: Option<String>) {
        let _ = self
            .command_tx
            .send(ObswsCoordinatorCommand::UpdateWebRtcSourceTrackId {
                input_name: input_name.to_owned(),
                track_id,
            });
    }

    /// inputName から最新の input 情報を解決する
    pub async fn resolve_input_by_name(
        &self,
        input_name: &str,
    ) -> crate::Result<Option<ResolvedInputInfo>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.command_tx
            .send(ObswsCoordinatorCommand::ResolveInputByName {
                input_name: input_name.to_owned(),
                reply_tx,
            })
            .map_err(|_| crate::Error::new("coordinator has terminated"))?;
        reply_rx
            .await
            .map_err(|_| crate::Error::new("coordinator dropped reply channel"))
    }

    /// obsws イベント broadcast を購読する
    pub fn subscribe_obsws_events(&self) -> tokio::sync::broadcast::Receiver<TaggedEvent> {
        self.obsws_event_tx.subscribe()
    }

    /// bootstrap 用の差分イベントを購読する
    pub fn subscribe_bootstrap_events(
        &self,
    ) -> tokio::sync::broadcast::Receiver<BootstrapInputEvent> {
        self.bootstrap_event_tx.subscribe()
    }
}
