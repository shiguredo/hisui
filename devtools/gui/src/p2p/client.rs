//! HTTP Bootstrap + DataChannel シグナリングによる P2P WebRTC クライアント。
//!
//! `devtools/src/p2p/client.ts` の `createP2PClient` の Rust 移植。
//! PeerConnection は !Send のため、クライアントは単一スレッドの tokio LocalSet 上で
//! セッションを所有し、libwebrtc のコールバックは mpsc チャネル経由で受け取る。
//! 状態変更・ログ・映像フレームは GUI 側へ mpsc チャネルで送信する。

use std::collections::HashMap;
use std::sync::Arc;

use shiguredo_webrtc::{
    DataChannel, DataChannelInit, DataChannelObserver, DataChannelObserverHandler,
    DataChannelState as RtcDataChannelState, IceCandidateRef, IceGatheringState, PeerConnection,
    PeerConnectionDependencies, PeerConnectionFactory, PeerConnectionObserver,
    PeerConnectionObserverHandler, PeerConnectionRtcConfiguration, PeerConnectionState,
    RtpReceiver, RtpTransceiver, VideoSink, VideoSinkHandler, VideoSinkWants,
};
use tokio::sync::{mpsc, oneshot, watch};

use crate::obsdc::{
    EventData, RequestData, RequestResponseData, parse_server_message as parse_obsdc_message,
    serialize_client_message as serialize_obsdc_message,
};
use crate::p2p::signaling::{
    parse_server_message as parse_signaling_message,
    serialize_client_message as serialize_signaling_message,
};
use crate::p2p::types::{
    BootstrapConfig, CloseCode, ConnectionState, DataChannelState as ClientDcState, IceServer,
    LogCategory, LogEntry, LogLevel, ServerMessage,
};

/// GUI からクライアントへの操作コマンド。
pub enum ClientCommand {
    /// HTTP Bootstrap からの P2P 接続を開始する
    Connect(BootstrapConfig),
    /// 切断する
    Disconnect,
    /// obsdc DataChannel で OBS WebSocket リクエストを送信する。
    /// `response_tx` にはレスポンス受信時に結果が送られる。
    SendObsdcRequest {
        request_type: String,
        request_data: Option<nojson::RawJsonOwned>,
        response_tx: Option<oneshot::Sender<RequestResponseData>>,
    },
    /// WebRTC stats を取得する
    GetStats,
    /// クライアントを終了する
    Shutdown,
}

/// クライアントから GUI への状態通知。
pub enum ClientEvent {
    ConnectionStateChanged(ConnectionState),
    DataChannelStateChanged {
        label: &'static str,
        state: ClientDcState,
    },
    TrackAdded {
        track_id: String,
        kind: String,
    },
    TrackRemoved {
        track_id: String,
    },
    CloseReceived {
        code: CloseCode,
        reason: String,
    },
    Log {
        entry: LogEntry,
    },
    ObsdcEvent(EventData),
    ObsdcRequestResponse(RequestResponseData),
    Stats {
        json: String,
    },
}

/// 映像フレーム (I420)。
#[derive(Debug, Clone)]
pub struct OwnedVideoFrame {
    pub track_id: String,
    pub width: i32,
    pub height: i32,
    pub timestamp_us: i64,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub stride_y: i32,
    pub stride_u: i32,
    pub stride_v: i32,
}

/// クライアントの操作ハンドルと終了待ちハンドル。
#[derive(Clone)]
pub struct P2PClientHandle {
    command_tx: mpsc::UnboundedSender<ClientCommand>,
    /// クライアントスレッドの終了待ちハンドル
    ///
    /// `shutdown_and_join` を一度だけ実行するためのフラグも兼ねる。
    join_handle: Arc<std::sync::Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl P2PClientHandle {
    pub fn connect(&self, config: BootstrapConfig) {
        let _ = self.command_tx.send(ClientCommand::Connect(config));
    }

    pub fn disconnect(&self) {
        let _ = self.command_tx.send(ClientCommand::Disconnect);
    }

    pub fn send_obsdc_request(
        &self,
        request_type: &str,
        request_data: Option<nojson::RawJsonOwned>,
    ) -> oneshot::Receiver<RequestResponseData> {
        let (tx, rx) = oneshot::channel();
        let _ = self.command_tx.send(ClientCommand::SendObsdcRequest {
            request_type: request_type.to_owned(),
            request_data,
            response_tx: Some(tx),
        });
        rx
    }

    pub fn get_stats(&self) {
        let _ = self.command_tx.send(ClientCommand::GetStats);
    }

    pub fn shutdown(&self) {
        let _ = self.command_tx.send(ClientCommand::Shutdown);
    }

    /// クライアントスレッドをシャットダウンし、終了を待つ。
    ///
    /// アプリ終了時に一度だけ呼ぶこと。プロセス終了時に libwebrtc のスレッドが
    /// 残ったままにならないようにするため。
    pub fn shutdown_and_join(&self) {
        let _ = self.command_tx.send(ClientCommand::Shutdown);
        if let Some(handle) = self.join_handle.lock().expect("lock").take() {
            let _ = handle.join();
        }
    }
}

/// セッションのイベント (libwebrtc コールバック由来)。
enum SessionEvent {
    ConnectionChange(PeerConnectionState),
    DataChannel(DataChannel),
    DataChannelStateChange { label: &'static str },
    DcMessage { label: &'static str, data: Vec<u8> },
    RemoteTrack(RtpTransceiver),
    RemoteTrackRemoved { track_id: String },
}

enum IceObserverEvent {
    Candidate {
        sdp_mid: String,
        sdp_mline_index: i32,
        candidate: String,
    },
    Complete,
}

struct PcObserverHandler {
    event_tx: mpsc::UnboundedSender<SessionEvent>,
    ice_tx: mpsc::UnboundedSender<IceObserverEvent>,
}

impl PeerConnectionObserverHandler for PcObserverHandler {
    fn on_connection_change(&mut self, state: PeerConnectionState) {
        let _ = self.event_tx.send(SessionEvent::ConnectionChange(state));
    }

    fn on_data_channel(&mut self, dc: DataChannel) {
        let _ = self.event_tx.send(SessionEvent::DataChannel(dc));
    }

    fn on_ice_gathering_change(&mut self, state: IceGatheringState) {
        if state == IceGatheringState::Complete {
            let _ = self.ice_tx.send(IceObserverEvent::Complete);
        }
    }

    fn on_ice_candidate(&mut self, candidate: IceCandidateRef<'_>) {
        let Ok(sdp_mid) = candidate.sdp_mid() else {
            return;
        };
        let sdp_mline_index = candidate.sdp_mline_index();
        let Ok(candidate) = candidate.to_string() else {
            return;
        };
        let _ = self.ice_tx.send(IceObserverEvent::Candidate {
            sdp_mid,
            sdp_mline_index,
            candidate,
        });
    }

    fn on_track(&mut self, transceiver: RtpTransceiver) {
        let _ = self.event_tx.send(SessionEvent::RemoteTrack(transceiver));
    }

    fn on_remove_track(&mut self, receiver: RtpReceiver) {
        let track_id = receiver.track().id().unwrap_or_default();
        let _ = self
            .event_tx
            .send(SessionEvent::RemoteTrackRemoved { track_id });
    }
}

struct DcObserverHandler {
    event_tx: mpsc::UnboundedSender<SessionEvent>,
    label: &'static str,
}

impl DataChannelObserverHandler for DcObserverHandler {
    fn on_state_change(&mut self) {
        let _ = self
            .event_tx
            .send(SessionEvent::DataChannelStateChange { label: self.label });
    }

    fn on_message(&mut self, data: &[u8], _is_binary: bool) {
        let _ = self.event_tx.send(SessionEvent::DcMessage {
            label: self.label,
            data: data.to_vec(),
        });
    }
}

struct FrameSinkHandler {
    frame_tx: watch::Sender<Option<OwnedVideoFrame>>,
    track_id: String,
}

impl VideoSinkHandler for FrameSinkHandler {
    fn on_frame(&mut self, frame: shiguredo_webrtc::VideoFrameRef<'_>) {
        // I420 に変換して所有バッファへコピーする
        let Some(buffer) = frame.buffer().as_i420() else {
            return;
        };
        let width = buffer.width();
        let height = buffer.height();
        if width <= 0 || height <= 0 {
            return;
        }
        let owned = OwnedVideoFrame {
            track_id: self.track_id.clone(),
            width,
            height,
            timestamp_us: frame.timestamp_us(),
            y: buffer.y_data().to_vec(),
            u: buffer.u_data().to_vec(),
            v: buffer.v_data().to_vec(),
            stride_y: buffer.stride_y(),
            stride_u: buffer.stride_u(),
            stride_v: buffer.stride_v(),
        };
        // watch チャネルは最新のフレームだけを保持する。
        // UI の処理が追いつかない場合は古いフレームが置き換えられ、常に最新が表示される。
        let _ = self.frame_tx.send(Some(owned));
    }
}

/// 接続確立後のセッション。
struct Session {
    pc: PeerConnection,
    _pc_observer: PeerConnectionObserver,
    /// セッション内のイベント送信チャネル (DataChannel observer 作成に使う)
    event_tx: mpsc::UnboundedSender<SessionEvent>,
    signaling_dc: Option<DataChannel>,
    obsdc_dc: Option<DataChannel>,
    _dc_observer: Option<DataChannelObserver>,
    _obsdc_dc_observer: Option<DataChannelObserver>,
    connection_state: ConnectionState,
    ice_rx: mpsc::UnboundedReceiver<IceObserverEvent>,
    ice_candidates: Vec<GatheredIceCandidate>,
    pending_requests: HashMap<String, oneshot::Sender<RequestResponseData>>,
    next_request_id: u64,
    /// Program トラックを購読済みかどうか
    program_tracks_subscribed: bool,
    client_event_tx: mpsc::UnboundedSender<ClientEvent>,
    frame_tx: watch::Sender<Option<OwnedVideoFrame>>,
    video_sinks: HashMap<String, VideoSink>,
}

impl Drop for Session {
    fn drop(&mut self) {
        tracing::warn!(
            "Closing PeerConnection\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        self.pc.close();
    }
}

#[derive(Clone)]
struct GatheredIceCandidate {
    sdp_mid: String,
    sdp_mline_index: i32,
    candidate: String,
}

impl Session {
    fn add_log(&self, category: LogCategory, level: LogLevel, message: String) {
        let entry = LogEntry {
            timestamp_ms: current_timestamp_ms(),
            level,
            category,
            message,
        };
        let _ = self.client_event_tx.send(ClientEvent::Log { entry });
    }

    fn set_connection_state(&mut self, state: ConnectionState) {
        self.connection_state = state;
        let _ = self
            .client_event_tx
            .send(ClientEvent::ConnectionStateChanged(state));
    }

    /// セッションの終了処理。true を返した場合はセッションを破棄する。
    fn handle_connection_change(&mut self, state: PeerConnectionState) -> bool {
        tracing::info!("handle_connection_change: {:?}", state);
        self.add_log(
            LogCategory::Pc,
            LogLevel::Info,
            format!("State changed: {state:?}"),
        );
        match state {
            PeerConnectionState::Connected => {
                self.set_connection_state(ConnectionState::Connected);
                false
            }
            PeerConnectionState::New | PeerConnectionState::Connecting => false,
            PeerConnectionState::Disconnected
            | PeerConnectionState::Failed
            | PeerConnectionState::Closed => {
                self.set_connection_state(ConnectionState::Closed);
                true
            }
            PeerConnectionState::Unknown(value) => {
                self.add_log(
                    LogCategory::Pc,
                    LogLevel::Warn,
                    format!("Unknown PeerConnection state: {value}"),
                );
                false
            }
        }
    }

    fn handle_data_channel(&mut self, mut dc: DataChannel) {
        let label = dc.label().unwrap_or_default();
        tracing::info!("handle_data_channel: label={label} state={:?}", dc.state());
        self.add_log(
            LogCategory::Pc,
            LogLevel::Info,
            format!("DataChannel received: label={label}"),
        );
        match label.as_str() {
            "signaling" => {
                let dc_observer =
                    DataChannelObserver::new_with_handler(Box::new(DcObserverHandler {
                        event_tx: self.event_tx.clone(),
                        label: "signaling",
                    }));
                dc.register_observer(&dc_observer);
                self.signaling_dc = Some(dc);
                self._dc_observer = Some(dc_observer);
            }
            "obsdc" => {
                let dc_observer =
                    DataChannelObserver::new_with_handler(Box::new(DcObserverHandler {
                        event_tx: self.event_tx.clone(),
                        label: "obsdc",
                    }));
                dc.register_observer(&dc_observer);
                self.obsdc_dc = Some(dc);
                self._obsdc_dc_observer = Some(dc_observer);
            }
            other => {
                self.add_log(
                    LogCategory::Pc,
                    LogLevel::Warn,
                    format!("Unknown DataChannel \"{other}\" ignored"),
                );
            }
        }

        // 受け取り時点で既に open の場合、DataChannelStateChange イベントは
        // 観測されない (observer 登録前に状態遷移が完了している) ため、
        // ここで state を通知して Program トラック購読を開始する
        if label == "obsdc" {
            let state = self
                .obsdc_dc
                .as_ref()
                .map(|dc| dc.state())
                .unwrap_or(RtcDataChannelState::Unknown(-1));
            if state == RtcDataChannelState::Open {
                self.handle_dc_state_change("obsdc");
            }
        }
    }

    fn handle_dc_state_change(&mut self, label: &'static str) {
        self.add_log(
            LogCategory::Pc,
            LogLevel::Info,
            format!("DataChannel state changed: label={label}"),
        );
        let state = match label {
            "signaling" => self
                .signaling_dc
                .as_ref()
                .map(|dc| dc.state())
                .unwrap_or(RtcDataChannelState::Unknown(-1)),
            "obsdc" => self
                .obsdc_dc
                .as_ref()
                .map(|dc| dc.state())
                .unwrap_or(RtcDataChannelState::Unknown(-1)),
            _ => return,
        };
        let client_state = match state {
            RtcDataChannelState::Connecting => ClientDcState::Connecting,
            RtcDataChannelState::Open => ClientDcState::Open,
            RtcDataChannelState::Closing => ClientDcState::Closing,
            RtcDataChannelState::Closed => ClientDcState::Closed,
            RtcDataChannelState::Unknown(_) => ClientDcState::Closed,
        };
        let _ = self
            .client_event_tx
            .send(ClientEvent::DataChannelStateChanged {
                label,
                state: client_state,
            });

        // obsdc が open になったら Program トラックを購読して映像を受信する
        if label == "obsdc"
            && client_state == ClientDcState::Open
            && !self.program_tracks_subscribed
        {
            self.subscribe_program_tracks();
        }
    }

    /// Program トラックを購読する。
    ///
    /// 購読後、サーバーが renegotiation offer を送信し、
    /// 映像・音声トラックが受信できるようになる。
    fn subscribe_program_tracks(&mut self) {
        let request_id = self.next_request_id.to_string();
        self.next_request_id += 1;
        let message = serialize_obsdc_message(&crate::obsdc::ClientMessage::Request(RequestData {
            request_type: "HisuiSubscribeProgramTracks".to_owned(),
            request_id: request_id.clone(),
            request_data: None,
        }));
        self.add_log(
            LogCategory::Obsdc,
            LogLevel::Info,
            format!("Sent: {message}"),
        );
        if let Some(dc) = &self.obsdc_dc {
            dc.send(message.as_bytes(), false);
        }
        self.program_tracks_subscribed = true;
    }

    /// signaling メッセージを処理する。true を返した場合はセッションを終了する。
    async fn handle_signaling_message(&mut self, data: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(data) else {
            return false;
        };
        self.add_log(
            LogCategory::Signaling,
            LogLevel::Info,
            format!("Received: {text}"),
        );
        match parse_signaling_message(text) {
            Ok(ServerMessage::Offer { sdp }) => {
                self.add_log(
                    LogCategory::Signaling,
                    LogLevel::Info,
                    "Received offer, starting re-negotiation".to_owned(),
                );
                if let Err(e) = self.handle_renegotiation(&sdp).await {
                    self.add_log(
                        LogCategory::Pc,
                        LogLevel::Error,
                        format!("Re-negotiation failed: {}", e.display()),
                    );
                }
                false
            }
            Ok(ServerMessage::Close(message)) => {
                self.add_log(
                    LogCategory::Signaling,
                    LogLevel::Warn,
                    format!(
                        "Received close: code={:?}, reason={}",
                        message.code, message.reason
                    ),
                );
                let _ = self.client_event_tx.send(ClientEvent::CloseReceived {
                    code: message.code,
                    reason: message.reason.clone(),
                });
                true
            }
            Err(e) => {
                self.add_log(
                    LogCategory::Signaling,
                    LogLevel::Error,
                    format!("Failed to parse message: {}", e.0),
                );
                false
            }
        }
    }

    /// サーバー主導の renegotiation offer に answer する
    async fn handle_renegotiation(&mut self, sdp: &str) -> crate::Result<()> {
        tracing::info!("setRemoteDescription(offer)");
        crate::webrtc::set_remote_offer(&self.pc, sdp)?;
        tracing::info!("createAnswer()");
        let answer_sdp = crate::webrtc::create_answer_sdp(&self.pc)?;
        tracing::info!("setLocalDescription(answer)");
        crate::webrtc::set_local_answer(&self.pc, &answer_sdp)?;

        // ICE candidate を answer SDP に追加してから送信する
        let answer_sdp = self.finalize_local_sdp(answer_sdp).await?;

        let answer_message =
            serialize_signaling_message(&crate::p2p::types::ClientMessage::Answer {
                sdp: answer_sdp,
            });
        self.add_log(
            LogCategory::Signaling,
            LogLevel::Info,
            format!("Sent: {answer_message}"),
        );
        if let Some(dc) = &self.signaling_dc {
            dc.send(answer_message.as_bytes(), false);
        }
        Ok(())
    }

    /// obsws メッセージを処理する
    fn handle_obsdc_message(&mut self, data: &[u8]) {
        let Ok(text) = std::str::from_utf8(data) else {
            return;
        };
        self.add_log(
            LogCategory::Obsdc,
            LogLevel::Info,
            format!("Received: {text}"),
        );
        match parse_obsdc_message(text) {
            Ok(message) => match message {
                crate::obsdc::ServerMessage::Hello(_) => {
                    self.add_log(
                        LogCategory::Obsdc,
                        LogLevel::Info,
                        "Unhandled OpCode: Hello".to_owned(),
                    );
                }
                crate::obsdc::ServerMessage::Identified(_) => {
                    self.add_log(
                        LogCategory::Obsdc,
                        LogLevel::Info,
                        "Unhandled OpCode: Identified".to_owned(),
                    );
                }
                crate::obsdc::ServerMessage::RequestResponse(data) => {
                    if data.request_status.result {
                        self.add_log(
                            LogCategory::Obsdc,
                            LogLevel::Info,
                            format!("Response: {} success", data.request_type),
                        );
                    } else {
                        self.add_log(
                            LogCategory::Obsdc,
                            LogLevel::Error,
                            format!(
                                "Response: {} failed (code={}{})",
                                data.request_type,
                                data.request_status.code,
                                data.request_status
                                    .comment
                                    .as_ref()
                                    .map(|comment| format!(", {comment}"))
                                    .unwrap_or_default(),
                            ),
                        );
                    }
                    if let Some(pending) = self.pending_requests.remove(&data.request_id) {
                        let _ = pending.send(data.clone());
                    }
                    let _ = self
                        .client_event_tx
                        .send(ClientEvent::ObsdcRequestResponse(data));
                }
                crate::obsdc::ServerMessage::Event(data) => {
                    self.add_log(
                        LogCategory::Obsdc,
                        LogLevel::Info,
                        format!("Event: {}{}", data.event_type, {
                            match &data.event_data {
                                Some(event_data) => format!(" {}", event_data.text()),
                                None => String::new(),
                            }
                        }),
                    );
                    let _ = self.client_event_tx.send(ClientEvent::ObsdcEvent(data));
                }
            },
            Err(e) => {
                self.add_log(
                    LogCategory::Obsdc,
                    LogLevel::Error,
                    format!("Failed to parse message: {}", e.0),
                );
            }
        }
    }

    fn send_obsdc_request(
        &mut self,
        request_type: &str,
        request_data: Option<nojson::RawJsonOwned>,
        response_tx: Option<oneshot::Sender<RequestResponseData>>,
    ) -> Result<(), String> {
        let Some(dc) = &self.obsdc_dc else {
            let message = "obsdc datachannel is not open".to_owned();
            self.add_log(LogCategory::Obsdc, LogLevel::Error, message.clone());
            return Err(message);
        };
        if dc.state() != RtcDataChannelState::Open {
            let message = "obsdc datachannel is not open".to_owned();
            self.add_log(LogCategory::Obsdc, LogLevel::Error, message.clone());
            return Err(message);
        }
        let request_id = self.next_request_id.to_string();
        self.next_request_id += 1;
        let message = serialize_obsdc_message(&crate::obsdc::ClientMessage::Request(RequestData {
            request_type: request_type.to_owned(),
            request_id: request_id.clone(),
            request_data,
        }));
        self.add_log(
            LogCategory::Obsdc,
            LogLevel::Info,
            format!("Sent: {message}"),
        );
        dc.send(message.as_bytes(), false);

        if let Some(tx) = response_tx {
            self.pending_requests.insert(request_id, tx);
        }
        Ok(())
    }

    /// 受信トラックを登録する
    fn handle_remote_track(&mut self, transceiver: RtpTransceiver) {
        let receiver = transceiver.receiver();
        let track = receiver.track();
        let kind = track.kind().unwrap_or_default();
        let track_id = track.id().unwrap_or_default();
        self.add_log(
            LogCategory::Pc,
            LogLevel::Info,
            format!("Track received: kind={kind}, id={track_id}"),
        );

        if kind != "video" {
            // 初版は映像のみ表示する。audio はトラック一覧に載せるだけにする
            self.add_log(
                LogCategory::Pc,
                LogLevel::Info,
                format!("Ignoring non-video track for rendering: kind={kind}, id={track_id}"),
            );
            return;
        }

        // すべてのビデオトラックに VideoSink を登録する。
        // サーバーは Program 出力と bootstrap 入力の生トラックの両方を送信するため、
        // 両方のフレームが UI 側でグリッド表示される。
        let mut video_track = track.cast_to_video_track();
        let sink_handler = FrameSinkHandler {
            frame_tx: self.frame_tx.clone(),
            track_id: track_id.clone(),
        };
        let sink = VideoSink::new_with_handler(Box::new(sink_handler));
        let wants = VideoSinkWants::new();
        video_track.add_or_update_sink(&sink, &wants);
        self.video_sinks.insert(track_id.clone(), sink);

        let _ = self.client_event_tx.send(ClientEvent::TrackAdded {
            track_id: track_id.clone(),
            kind: kind.clone(),
        });
    }

    fn handle_remote_track_removed(&mut self, track_id: &str) {
        self.add_log(
            LogCategory::Pc,
            LogLevel::Info,
            format!("Track removed: id={track_id}"),
        );
        self.video_sinks.remove(track_id);
        let _ = self.client_event_tx.send(ClientEvent::TrackRemoved {
            track_id: track_id.to_owned(),
        });
    }

    /// ICE gathering が完了した SDP を返す。タイムアウト時はキャッシュ済み candidate を追加する
    async fn finalize_local_sdp(&mut self, initial_sdp: String) -> crate::Result<String> {
        if initial_sdp.contains("\r\na=candidate:") {
            return Ok(initial_sdp);
        }

        let mut candidates = Vec::new();
        let mut complete = false;
        // まずノンブロッキングで既に到着しているイベントを処理する
        while let Ok(event) = self.ice_rx.try_recv() {
            match event {
                IceObserverEvent::Candidate {
                    sdp_mid,
                    sdp_mline_index,
                    candidate,
                } => {
                    candidates.push(GatheredIceCandidate {
                        sdp_mid,
                        sdp_mline_index,
                        candidate,
                    });
                }
                IceObserverEvent::Complete => {
                    complete = true;
                }
            }
        }

        if !complete && candidates.is_empty() && !self.ice_candidates.is_empty() {
            return Ok(append_ice_candidates_to_sdp(
                &initial_sdp,
                &self.ice_candidates,
            ));
        }

        // タイムアウト付きで ICE gathering 完了を待機する。
        // srflx (STUN) candidate の収集が完了しない環境があるため、
        // ブラウザ版と同じく短いタイムアウトで諦め、収集済み candidate を SDP に追加する。
        let timeout_duration = std::time::Duration::from_secs(1);
        let deadline = tokio::time::Instant::now() + timeout_duration;
        while !complete {
            match tokio::time::timeout_at(deadline, self.ice_rx.recv()).await {
                Ok(Some(IceObserverEvent::Candidate {
                    sdp_mid,
                    sdp_mline_index,
                    candidate,
                })) => {
                    candidates.push(GatheredIceCandidate {
                        sdp_mid,
                        sdp_mline_index,
                        candidate,
                    });
                }
                Ok(Some(IceObserverEvent::Complete)) => {
                    complete = true;
                }
                Ok(None) => {
                    // チャネルが切断された
                    return Err(crate::Error::new("ICE gathering channel closed"));
                }
                Err(_) => {
                    // タイムアウト。収集済み candidate を SDP に追加して返す
                    if !candidates.is_empty() {
                        return Ok(append_ice_candidates_to_sdp(&initial_sdp, &candidates));
                    }
                    if !self.ice_candidates.is_empty() {
                        return Ok(append_ice_candidates_to_sdp(
                            &initial_sdp,
                            &self.ice_candidates,
                        ));
                    }
                    return Err(crate::Error::new("ICE gathering timed out"));
                }
            }
        }

        if !candidates.is_empty() {
            self.ice_candidates = candidates.clone();
        }

        Ok(append_ice_candidates_to_sdp(
            &initial_sdp,
            if candidates.is_empty() {
                &self.ice_candidates
            } else {
                &candidates
            },
        ))
    }
}

fn append_ice_candidates_to_sdp(sdp: &str, candidates: &[GatheredIceCandidate]) -> String {
    let mut sections: Vec<Vec<String>> = Vec::new();
    let mut current_section = Vec::new();

    for line in sdp.split("\r\n").filter(|line| !line.is_empty()) {
        if line.starts_with("m=") && !current_section.is_empty() {
            sections.push(current_section);
            current_section = Vec::new();
        }
        current_section.push(line.to_owned());
    }
    if !current_section.is_empty() {
        sections.push(current_section);
    }

    let mut output = Vec::new();
    for (index, section) in sections.into_iter().enumerate() {
        let is_media_section = section.first().is_some_and(|line| line.starts_with("m="));
        let sdp_mid = section
            .iter()
            .find_map(|line| line.strip_prefix("a=mid:"))
            .unwrap_or_default();

        for line in &section {
            output.push(line.clone());
        }

        if is_media_section {
            let section_candidates: Vec<&GatheredIceCandidate> = candidates
                .iter()
                .filter(|candidate| {
                    candidate.sdp_mid == sdp_mid || candidate.sdp_mline_index == index as i32 - 1
                })
                .collect();
            if !section_candidates.is_empty() {
                for candidate in section_candidates {
                    output.push(format!("a={}", candidate.candidate));
                }
                output.push("a=end-of-candidates".to_owned());
            }
        }
    }

    output.join("\r\n") + "\r\n"
}

fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// HTTP Bootstrap から P2P 接続を確立し、セッションとイベント受信チャネルを返す。
async fn connect_session(
    factory: Arc<PeerConnectionFactory>,
    config: BootstrapConfig,
    client_event_tx: mpsc::UnboundedSender<ClientEvent>,
    frame_tx: watch::Sender<Option<OwnedVideoFrame>>,
) -> crate::Result<(Session, mpsc::UnboundedReceiver<SessionEvent>)> {
    let (session_event_tx, session_event_rx) = mpsc::unbounded_channel::<SessionEvent>();
    let (ice_tx, ice_rx) = mpsc::unbounded_channel::<IceObserverEvent>();

    // PeerConnectionObserver の作成
    let pc_observer = PeerConnectionObserver::new_with_handler(Box::new(PcObserverHandler {
        event_tx: session_event_tx.clone(),
        ice_tx,
    }));

    let mut deps = PeerConnectionDependencies::new(&pc_observer);
    let mut rtc_config = PeerConnectionRtcConfiguration::new();
    rtc_config.set_always_negotiate_data_channels(config.data_channel_only);
    for server in &config.ice_servers {
        add_ice_server(&mut rtc_config, server);
    }

    let pc = PeerConnection::create(factory.as_ref(), &mut rtc_config, &mut deps)
        .map_err(|e| crate::Error::new(format!("Failed to create PeerConnection: {e}")))?;

    let mut session = Session {
        pc,
        _pc_observer: pc_observer,
        event_tx: session_event_tx,
        signaling_dc: None,
        obsdc_dc: None,
        _dc_observer: None,
        _obsdc_dc_observer: None,
        connection_state: ConnectionState::Bootstrapping,
        ice_rx,
        ice_candidates: Vec::new(),
        pending_requests: HashMap::new(),
        next_request_id: 1,
        program_tracks_subscribed: false,
        client_event_tx,
        frame_tx,
        video_sinks: HashMap::new(),
    };

    session.set_connection_state(ConnectionState::Bootstrapping);
    session.add_log(
        LogCategory::Pc,
        LogLevel::Info,
        format!("Starting bootstrap: {}", config.bootstrap_url),
    );
    session.add_log(
        LogCategory::Pc,
        LogLevel::Info,
        format!("DataChannel only: {}", config.data_channel_only),
    );

    if config.data_channel_only {
        session.add_log(
            LogCategory::Pc,
            LogLevel::Info,
            "Created PeerConnection (alwaysNegotiateDataChannels: SDP contains m=application only)"
                .to_owned(),
        );
    } else {
        let mut dc_init = DataChannelInit::new();
        dc_init.set_ordered(true);
        let dummy_dc = session
            .pc
            .create_data_channel("dummy", &mut dc_init)
            .map_err(|e| crate::Error::new(format!("Failed to create dummy DataChannel: {e}")))?;
        dummy_dc.close();
        session.add_log(
            LogCategory::Pc,
            LogLevel::Info,
            "Created PeerConnection (createDataChannel fallback: dummy DataChannel created)"
                .to_owned(),
        );
    }

    session.add_log(LogCategory::Pc, LogLevel::Info, "createOffer()".to_owned());
    let offer_sdp = crate::webrtc::create_offer_sdp(&session.pc)?;
    crate::webrtc::set_local_offer(&session.pc, &offer_sdp)?;

    session.add_log(
        LogCategory::Pc,
        LogLevel::Info,
        "Waiting for ICE gathering".to_owned(),
    );
    let offer_sdp = session.finalize_local_sdp(offer_sdp).await?;
    session.add_log(
        LogCategory::Pc,
        LogLevel::Info,
        "ICE gathering complete".to_owned(),
    );

    session.add_log(
        LogCategory::Pc,
        LogLevel::Info,
        format!(
            "POST {} (Content-Type: application/sdp)",
            config.bootstrap_url
        ),
    );
    session.add_log(
        LogCategory::Pc,
        LogLevel::Info,
        format!("offer SDP:\n{offer_sdp}"),
    );
    let (status, answer_sdp) = crate::webrtc::post_sdp(&config.bootstrap_url, &offer_sdp).await?;
    session.add_log(
        LogCategory::Pc,
        LogLevel::Info,
        format!("Bootstrap response: {status}"),
    );
    if status != 201 {
        return Err(crate::Error::new(format!(
            "bootstrap failed with status: {status}"
        )));
    }
    session.add_log(
        LogCategory::Pc,
        LogLevel::Info,
        format!("answer SDP:\n{answer_sdp}"),
    );

    session.add_log(
        LogCategory::Pc,
        LogLevel::Info,
        "setRemoteDescription(answer)".to_owned(),
    );
    crate::webrtc::set_remote_answer(&session.pc, &answer_sdp)?;

    session.set_connection_state(ConnectionState::Connecting);
    session.add_log(
        LogCategory::Pc,
        LogLevel::Info,
        "Connecting (waiting for WebRTC handshake)".to_owned(),
    );

    Ok((session, session_event_rx))
}

/// ICE サーバーを設定に追加する。
fn add_ice_server(config: &mut PeerConnectionRtcConfiguration, server: &IceServer) {
    let mut ice_server = shiguredo_webrtc::IceServer::new();
    for url in &server.urls {
        ice_server.add_url(url);
    }
    if let Some(username) = &server.username {
        ice_server.set_username(username);
    }
    if let Some(credential) = &server.credential {
        ice_server.set_password(credential);
    }
    config.servers().push(&ice_server);
}

/// クライアントを起動する。バックグラウンドスレッドで tokio LocalSet を実行する。
///
/// 戻り値は (操作ハンドル, イベント受信チャネル, 映像フレーム受信チャネル)。
pub fn spawn_client() -> crate::Result<(
    P2PClientHandle,
    mpsc::UnboundedReceiver<ClientEvent>,
    watch::Receiver<Option<OwnedVideoFrame>>,
)> {
    let (command_tx, command_rx) = mpsc::unbounded_channel::<ClientCommand>();
    let (client_event_tx, client_event_rx) = mpsc::unbounded_channel::<ClientEvent>();
    // watch チャネルは最新のフレームだけを保持する
    let (frame_tx, frame_rx) = watch::channel(None);

    let factory_bundle = crate::webrtc::WebRtcFactoryBundle::new()?;
    let factory = factory_bundle.factory();

    let join_handle = std::thread::Builder::new()
        .name("hisui-devtools-client".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime の作成に失敗しました");
            let local = tokio::task::LocalSet::new();
            local.block_on(&runtime, async move {
                run_client_loop(
                    factory_bundle,
                    factory,
                    command_rx,
                    client_event_tx,
                    frame_tx,
                )
                .await;
            });
        })
        .map_err(|e| crate::Error::new(format!("failed to spawn client thread: {e}")))?;

    Ok((
        P2PClientHandle {
            command_tx,
            join_handle: Arc::new(std::sync::Mutex::new(Some(join_handle))),
        },
        client_event_rx,
        frame_rx,
    ))
}

/// クライアントのメインループ。
async fn run_client_loop(
    _factory_bundle: crate::webrtc::WebRtcFactoryBundle,
    factory: Arc<PeerConnectionFactory>,
    mut command_rx: mpsc::UnboundedReceiver<ClientCommand>,
    client_event_tx: mpsc::UnboundedSender<ClientEvent>,
    frame_tx: watch::Sender<Option<OwnedVideoFrame>>,
) {
    // コマンド処理中もイベント送信に使えるよう、先にクローンを確保しておく
    let client_event_tx = client_event_tx;
    let event_tx_for_log = client_event_tx.clone();
    let mut session: Option<Session> = None;
    let mut session_event_rx: Option<mpsc::UnboundedReceiver<SessionEvent>> = None;
    // Connecting 状態のタイムアウト
    let mut connect_deadline: Option<tokio::time::Instant> = None;

    loop {
        tokio::select! {
            command = command_rx.recv() => {
                let Some(command) = command else {
                    // コマンドチャネルが閉じられたら終了する
                    break;
                };
                match command {
                    ClientCommand::Connect(config) => {
                        if session.is_some() {
                            send_error_log(&client_event_tx, "session already exists");
                            continue;
                        }
                        match connect_session(factory.clone(), config, event_tx_for_log.clone(), frame_tx.clone()).await {
                            Ok((sess, rx)) => {
                                connect_deadline = Some(tokio::time::Instant::now() + std::time::Duration::from_secs(5));
                                session = Some(sess);
                                session_event_rx = Some(rx);
                            }
                            Err(e) => {
                                send_error_log(&client_event_tx, format!("Bootstrap failed: {}", e.display()).as_str());
                                let _ = client_event_tx.send(ClientEvent::ConnectionStateChanged(ConnectionState::Closed));
                            }
                        }
                    }
                    ClientCommand::Disconnect => {
                        let Some(sess) = session.as_mut() else {
                            continue;
                        };
                        sess.add_log(LogCategory::Pc, LogLevel::Info, "Disconnecting".to_owned());
                        if let Some(dc) = &sess.signaling_dc
                            && dc.state() == RtcDataChannelState::Open {
                            let disconnect_message = serialize_signaling_message(&crate::p2p::types::ClientMessage::Disconnect);
                            sess.add_log(LogCategory::Signaling, LogLevel::Info, format!("Sent: {disconnect_message}"));
                            dc.send(disconnect_message.as_bytes(), false);
                        }
                        // セッションを破棄する (Drop で pc.close() される)
                        session = None;
                        session_event_rx = None;
                        connect_deadline = None;
                        let _ = client_event_tx.send(ClientEvent::ConnectionStateChanged(ConnectionState::Closed));
                        send_info_log(&client_event_tx, "Disconnected");
                    }
                    ClientCommand::SendObsdcRequest {
                        request_type,
                        request_data,
                        response_tx,
                    } => {
                        if let Some(sess) = session.as_mut() {
                            let _ = sess.send_obsdc_request(&request_type, request_data, response_tx);
                        } else if let Some(tx) = response_tx {
                            // セッションが無い場合はエラーとして通知する
                            let _ = tx.send(RequestResponseData {
                                request_type,
                                request_id: String::new(),
                                request_status: crate::obsdc::RequestStatus {
                                    result: false,
                                    code: 0,
                                    comment: Some("not connected".to_owned()),
                                },
                                response_data: None,
                            });
                        }
                    }
                    ClientCommand::GetStats => {
                        if let Some(sess) = session.as_mut() {
                            let stats_tx = client_event_tx.clone();
                            sess.pc.get_stats(move |report| {
                                let json = report.to_json().unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"));
                                let _ = stats_tx.send(ClientEvent::Stats { json });
                            });
                        }
                    }
                    ClientCommand::Shutdown => {
                        // セッションはスコープ終了時の drop に任せる
                        break;
                    }
                }
            }
            event = async {
                session_event_rx.as_mut().expect("セッションのイベント受信チャネルは存在する").recv().await
            }, if session_event_rx.is_some() => {
                let Some(event) = event else {
                    continue;
                };
                let Some(sess) = session.as_mut() else {
                    continue;
                };
                let session_closed = match event {
                    SessionEvent::ConnectionChange(state) => {
                        // 接続が確立したらタイムアウトを解除する
                        if state == PeerConnectionState::Connected {
                            connect_deadline = None;
                        }
                        sess.handle_connection_change(state)
                    }
                    SessionEvent::DataChannel(dc) => {
                        sess.handle_data_channel(dc);
                        false
                    }
                    SessionEvent::DataChannelStateChange { label } => {
                        sess.handle_dc_state_change(label);
                        false
                    }
                    SessionEvent::DcMessage { label, data } => {
                        match label {
                            "signaling" => sess.handle_signaling_message(&data).await,
                            "obsdc" => {
                                sess.handle_obsdc_message(&data);
                                false
                            }
                            _ => false,
                        }
                    }
                    SessionEvent::RemoteTrack(transceiver) => {
                        sess.handle_remote_track(transceiver);
                        false
                    }
                    SessionEvent::RemoteTrackRemoved { track_id } => {
                        sess.handle_remote_track_removed(&track_id);
                        false
                    }
                };
                if session_closed {
                    // セッションを破棄する
                    tracing::warn!("session closed by session event");
                    let _ = sess.client_event_tx.send(ClientEvent::ConnectionStateChanged(ConnectionState::Closed));
                    session = None;
                    session_event_rx = None;
                    connect_deadline = None;
                }
            }
            _ = async {
                // precondition が false のときも将来式は評価されるため、async ブロックで遅延評価する
                tokio::time::sleep_until(
                    connect_deadline.expect("connect_deadline は is_some の場合のみ有効"),
                )
                .await;
            }, if connect_deadline.is_some() => {
                // 接続タイムアウト
                let Some(sess) = session.as_mut() else {
                    connect_deadline = None;
                    continue;
                };
                tracing::warn!("connection timeout");
                sess.add_log(LogCategory::Pc, LogLevel::Error, "Connection timed out".to_owned());
                session = None;
                session_event_rx = None;
                connect_deadline = None;
                let _ = client_event_tx.send(ClientEvent::ConnectionStateChanged(ConnectionState::Closed));
            }
        }
    }
}

fn send_error_log(event_tx: &mpsc::UnboundedSender<ClientEvent>, message: &str) {
    let _ = event_tx.send(ClientEvent::Log {
        entry: LogEntry {
            timestamp_ms: current_timestamp_ms(),
            level: LogLevel::Error,
            category: LogCategory::Pc,
            message: message.to_owned(),
        },
    });
}

fn send_info_log(event_tx: &mpsc::UnboundedSender<ClientEvent>, message: &str) {
    let _ = event_tx.send(ClientEvent::Log {
        entry: LogEntry {
            timestamp_ms: current_timestamp_ms(),
            level: LogLevel::Info,
            category: LogCategory::Pc,
            message: message.to_owned(),
        },
    });
}
