//! DevTools の GPUI UI。
//!
//! ブラウザ版 devtools の P2P ページ (`devtools/src/pages/P2PPage.tsx`) 相当の UI を提供する。
//! 接続状態・DataChannel 状態・映像表示・ログ表示・ソース管理 (OBS WebSocket リクエスト) を実装する。

use std::collections::VecDeque;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    Context, Entity, ImageSource, ObjectFit, Render, RenderImage, SharedString, Window, div, img,
    rgb,
};

use hisui_devtools_gui::obsdc::RequestResponseData;
use hisui_devtools_gui::p2p::{
    BootstrapConfig, ClientEvent, ConnectionState, DataChannelState as ClientDcState, IceServer,
    LogCategory, LogEntry, LogLevel, OwnedVideoFrame, P2PClientHandle, spawn_client,
};

/// 映像トラックの情報。
struct TrackInfo {
    track_id: String,
    kind: String,
}

/// ソース (入力) の情報。
#[derive(Debug, Clone)]
struct SourceInfo {
    input_name: String,
    input_kind: String,
}

/// カメラ (ビデオデバイス) の情報。
#[derive(Debug, Clone)]
struct CameraInfo {
    name: String,
    device_id: String,
}

/// UI の状態。
pub struct DevToolsApp {
    client: P2PClientHandle,
    connection_state: ConnectionState,
    signaling_dc_state: ClientDcState,
    obsdc_dc_state: ClientDcState,
    tracks: Vec<TrackInfo>,
    logs: VecDeque<LogEntry>,
    /// トラックごとの最新映像フレーム (BGRA)。グリッド表示される。
    videos: std::collections::BTreeMap<String, Arc<RenderImage>>,
    last_error: Option<String>,
    /// 接続中かどうか (映像プレースホルダの表示切り替え用)
    connecting: bool,
    /// 現在のシーン名
    current_scene: String,
    /// ソース一覧
    sources: Vec<SourceInfo>,
    /// カメラ追加ダイアログの表示中かどうか
    show_camera_dialog: bool,
    /// カメラ列挙中かどうか
    camera_loading: bool,
    /// カメラ一覧
    cameras: Vec<CameraInfo>,
    /// カメラ選択中のインデックス
    selected_camera: Option<usize>,
    /// 選択中のソースのインデックス
    selected_source: Option<usize>,
    /// 一時 probe input の名前 (カメラ列挙用)
    probe_input_name: Option<String>,
}

impl DevToolsApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        tracing::info!("DevToolsApp::new called");
        let (client, event_rx, frame_rx) =
            spawn_client().expect("P2P クライアントの起動に失敗しました");

        let mut app = Self {
            client,
            connection_state: ConnectionState::Idle,
            signaling_dc_state: ClientDcState::NotCreated,
            obsdc_dc_state: ClientDcState::NotCreated,
            tracks: Vec::new(),
            logs: VecDeque::new(),
            videos: std::collections::BTreeMap::new(),
            last_error: None,
            connecting: false,
            current_scene: String::new(),
            sources: Vec::new(),
            show_camera_dialog: false,
            camera_loading: false,
            cameras: Vec::new(),
            selected_camera: None,
            selected_source: None,
            probe_input_name: None,
        };

        app.start_event_tasks(cx, event_rx, frame_rx);
        app
    }

    /// クライアントからのイベント・映像フレームを UI に反映するタスクを起動する。
    ///
    /// tokio のチャネルは waker ベースで tokio ランタイムが無くても await できるため、
    /// GPUI のフォアグラウンドエグゼキュータ (メインスレッド) 上で受信する。
    fn start_event_tasks(
        &mut self,
        cx: &mut Context<Self>,
        mut event_rx: tokio::sync::mpsc::UnboundedReceiver<ClientEvent>,
        mut frame_rx: tokio::sync::watch::Receiver<Option<OwnedVideoFrame>>,
    ) {
        let app = cx.entity();

        // イベント受信タスク
        cx.spawn(async move |_weak, cx| {
            while let Some(event) = event_rx.recv().await {
                let _ = app.update::<_, gpui::AsyncApp>(cx, |app, cx| {
                    app.handle_client_event(event, cx);
                    cx.notify();
                });
            }
        })
        .detach();

        // 映像フレーム受信タスク
        let frame_app = cx.entity();
        cx.spawn(async move |_weak, cx| {
            // トラックごとの表示レートを 30fps に制限する。
            // サーバーは複数の映像トラック (Program 出力 + bootstrap 入力) を
            // それぞれ 30fps で送信するため、トラック単位で制限しないと
            // 片方のトラックの表示が間引かれてカクカクする。
            let mut last_display: std::collections::HashMap<String, std::time::Instant> =
                std::collections::HashMap::new();
            let mut frame_count: u64 = 0;
            let mut last_log = std::time::Instant::now();
            while frame_rx.changed().await.is_ok() {
                // watch チャネルは最新フレームだけを保持するため、borrow で最新を取得する
                let frame = frame_rx.borrow().clone();
                let Some(frame) = frame else {
                    continue;
                };
                frame_count += 1;
                // フレームレート確認用のログ (5 秒ごと)
                if last_log.elapsed() >= std::time::Duration::from_secs(5) {
                    tracing::info!(
                        "frame received: {}x{} track={} rate={:.1} fps",
                        frame.width,
                        frame.height,
                        frame.track_id,
                        frame_count as f64 / last_log.elapsed().as_secs_f64(),
                    );
                    last_log = std::time::Instant::now();
                    frame_count = 0;
                }
                // トラックごとの表示レートを制限する。制限を超えたフレームは
                // watch チャネルで最新フレームに置き換えられる
                let track_id = frame.track_id.clone();
                let last = last_display.entry(track_id.clone()).or_insert_with(|| {
                    std::time::Instant::now()
                        .checked_sub(std::time::Duration::from_secs(1))
                        .unwrap_or_else(std::time::Instant::now)
                });
                if last.elapsed() < std::time::Duration::from_millis(33) {
                    continue;
                }
                *last = std::time::Instant::now();
                // 色変換はバックグラウンドエグゼキュータで行い、メインスレッドをブロックしない
                let render_image = cx
                    .background_executor()
                    .spawn(async move { to_render_image(&frame) })
                    .await;
                if let Some(render_image) = render_image {
                    let _ = frame_app.update::<_, gpui::AsyncApp>(cx, |app, cx| {
                        app.videos.insert(track_id, render_image);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn handle_client_event(&mut self, event: ClientEvent, cx: &mut Context<Self>) {
        match event {
            ClientEvent::ConnectionStateChanged(state) => {
                self.connection_state = state;
                self.connecting = matches!(
                    state,
                    ConnectionState::Bootstrapping | ConnectionState::Connecting
                );
                if state == ConnectionState::Connected {
                    // 接続直後にソース一覧を取得する
                    self.refresh_sources(cx);
                }
                if state == ConnectionState::Closed {
                    // 切断時はソース一覧をクリアする
                    self.sources.clear();
                    self.current_scene.clear();
                    self.show_camera_dialog = false;
                    self.camera_loading = false;
                    self.cameras.clear();
                    self.selected_source = None;
                    self.probe_input_name = None;
                }
            }
            ClientEvent::DataChannelStateChanged { label, state } => match label {
                "signaling" => self.signaling_dc_state = state,
                "obsdc" => self.obsdc_dc_state = state,
                _ => {}
            },
            ClientEvent::TrackAdded { track_id, kind } => {
                self.tracks.push(TrackInfo { track_id, kind });
            }
            ClientEvent::TrackRemoved { track_id } => {
                self.tracks.retain(|track| track.track_id != track_id);
            }
            ClientEvent::CloseReceived { code, reason } => {
                self.last_error = Some(format!("{code:?}: {reason}"));
                self.connection_state = ConnectionState::Closed;
                self.connecting = false;
            }
            ClientEvent::Log { entry } => {
                tracing::info!("[{}] {:?} {}", entry.category, entry.level, entry.message);
                self.push_log(entry);
            }
            ClientEvent::ObsdcEvent(data) => {
                tracing::info!("ObsdcEvent: {}", data.event_type);
                self.push_log(LogEntry {
                    timestamp_ms: now_ms(),
                    level: LogLevel::Info,
                    category: LogCategory::Obsdc,
                    message: format!("Event: {}", data.event_type),
                });
            }
            ClientEvent::ObsdcRequestResponse(data) => {
                tracing::info!(
                    "ObsdcRequestResponse: {} result={}",
                    data.request_type,
                    data.request_status.result
                );
                self.push_log(LogEntry {
                    timestamp_ms: now_ms(),
                    level: if data.request_status.result {
                        LogLevel::Info
                    } else {
                        LogLevel::Error
                    },
                    category: LogCategory::Obsdc,
                    message: format!("Response: {}", data.request_type),
                });
            }
            ClientEvent::Stats { .. } => {}
        }
    }

    fn push_log(&mut self, entry: LogEntry) {
        self.logs.push_back(entry);
        if self.logs.len() > 500 {
            self.logs.pop_front();
        }
    }

    fn connect(&self) {
        self.client.connect(BootstrapConfig {
            bootstrap_url: default_bootstrap_url().to_string(),
            ice_servers: vec![IceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                username: None,
                credential: None,
            }],
            data_channel_only: true,
        });
    }

    fn disconnect(&self) {
        self.client.disconnect();
    }

    /// ソース一覧と現在のシーンを取得する。
    fn refresh_sources(&mut self, cx: &mut Context<Self>) {
        if !self.is_connected() {
            return;
        }
        // 現在のシーン名を取得してからソース一覧を取得する
        let scene_app = cx.entity();
        let client = self.client.clone();
        let rx = client.send_obsdc_request("GetCurrentProgramScene", None);
        cx.spawn(async move |_weak, cx| {
            let Ok(scene_response) = rx.await else {
                return;
            };
            let scene_name: String = scene_response
                .response_data
                .as_ref()
                .and_then(|data| {
                    data.value()
                        .to_member("sceneName")
                        .ok()?
                        .required()
                        .ok()?
                        .try_into()
                        .ok()
                })
                .unwrap_or_default();
            let _ = scene_app.update::<_, gpui::AsyncApp>(cx, |app, _| {
                app.current_scene = scene_name;
            });
            let list_rx = client.send_obsdc_request("GetInputList", None);
            if let Ok(list_response) = list_rx.await {
                let sources = parse_input_list(&list_response);
                let _ = scene_app.update::<_, gpui::AsyncApp>(cx, |app, _| {
                    app.sources = sources;
                    app.selected_source = None;
                });
            }
        })
        .detach();
    }

    /// カメラ追加ダイアログを開き、カメラを列挙する。
    ///
    /// デバイス列挙 API が既存 input を必要とするため、
    /// 一時 input (probe) を作成してから列挙し、ダイアログを閉じるときに削除する。
    ///
    /// シーン名は self の状態に依存せず、その場で GetCurrentProgramScene を
    /// 送信して取得する (接続直後で current_scene が未設定でも動作するようにする)。
    fn open_camera_dialog(&mut self, cx: &mut Context<Self>) {
        if self.camera_loading {
            return;
        }
        self.camera_loading = true;
        self.cameras.clear();
        self.selected_camera = None;
        self.show_camera_dialog = true;

        let client = self.client.clone();
        let app = cx.entity();
        let probe_name = format!("__probe_camera_{}", now_ms());
        cx.spawn(async move |_weak, cx| {
            // 現在のシーン名を取得する
            let scene_name = match client
                .send_obsdc_request("GetCurrentProgramScene", None)
                .await
            {
                Ok(response) => response
                    .response_data
                    .as_ref()
                    .and_then(|data| {
                        data.value()
                            .to_member("sceneName")
                            .ok()?
                            .required()
                            .ok()?
                            .try_into()
                            .ok()
                    })
                    .unwrap_or_default(),
                Err(_) => String::new(),
            };
            let _ = app.update::<_, gpui::AsyncApp>(cx, |app, _| {
                app.current_scene = scene_name.clone();
            });
            if scene_name.is_empty() {
                tracing::warn!("GetCurrentProgramScene returned empty scene name");
                let _ = app.update::<_, gpui::AsyncApp>(cx, |app, _| {
                    app.camera_loading = false;
                });
                return;
            }
            // 一時 input を作成する (inputSettings は必須フィールドのため空オブジェクトを渡す)
            let create_data = nojson::RawJsonOwned::object(|f| {
                f.member("inputName", probe_name.as_str())?;
                f.member("inputKind", "video_capture_device")?;
                f.member("sceneName", scene_name.as_str())?;
                f.member("inputSettings", nojson::object(|_f| Ok(())))
            });
            let _ = client
                .send_obsdc_request("CreateInput", Some(create_data))
                .await;
            // デバイス一覧を列挙する
            let device_data =
                build_object_data(&[("inputName", &probe_name), ("propertyName", "device_id")]);
            let Ok(response) = client
                .send_obsdc_request("GetInputPropertiesListPropertyItems", device_data)
                .await
            else {
                tracing::warn!("GetInputPropertiesListPropertyItems failed");
                let _ = app.update::<_, gpui::AsyncApp>(cx, |app, _| {
                    app.camera_loading = false;
                    app.probe_input_name = Some(probe_name);
                });
                return;
            };
            let cameras = parse_camera_list(&response);
            let _ = app.update::<_, gpui::AsyncApp>(cx, |app, _| {
                app.cameras = cameras;
                app.camera_loading = false;
                app.probe_input_name = Some(probe_name);
            });
        })
        .detach();
    }

    /// カメラ追加ダイアログを閉じる。一時 input を削除する。
    fn close_camera_dialog(&mut self) {
        self.show_camera_dialog = false;
        let probe_name = self.probe_input_name.take();
        if let Some(probe_name) = probe_name {
            self.remove_probe_input(probe_name);
        }
    }

    /// 一時 input を削除する。
    fn remove_probe_input(&self, probe_name: String) {
        let remove_data = build_object_data(&[("inputName", &probe_name)]);
        // レスポンスは不要なので drop する
        drop(self.client.send_obsdc_request("RemoveInput", remove_data));
    }

    /// 選択したカメラでソースを追加する。
    fn add_camera(&mut self, cx: &mut Context<Self>, camera: CameraInfo) {
        if self.current_scene.is_empty() {
            return;
        }
        // 既存のソース名と重複しない名前を生成する
        let input_name = generate_unique_input_name("Video Capture Device", &self.sources);
        let probe_name = self.probe_input_name.clone();
        let scene_name = self.current_scene.clone();
        let client = self.client.clone();
        let app = cx.entity();

        // デバイス ID を settings に含めて CreateInput を送信する
        let settings = nojson::object(|f| f.member("device_id", camera.device_id.as_str()));
        let settings = nojson::RawJsonOwned::parse(settings.to_string())
            .expect("settings JSON のパースに失敗しました");
        let request_data = nojson::RawJsonOwned::object(|f| {
            f.member("inputName", input_name.as_str())?;
            f.member("inputKind", "video_capture_device")?;
            f.member("sceneName", scene_name.as_str())?;
            f.member("inputSettings", settings.clone())?;
            f.member("sceneItemEnabled", true)
        });

        self.show_camera_dialog = false;
        self.probe_input_name = None;

        cx.spawn(async move |_weak, cx| {
            let _ = client
                .send_obsdc_request("CreateInput", Some(request_data))
                .await;
            // 一時 input を削除する
            if let Some(probe_name) = probe_name {
                let remove_data = build_object_data(&[("inputName", &probe_name)]);
                let _ = client.send_obsdc_request("RemoveInput", remove_data).await;
            }
            let _ = app.update::<_, gpui::AsyncApp>(cx, |app, _| {
                app.camera_loading = false;
            });
        })
        .detach();
    }

    /// 選択中のソースを削除する。
    fn remove_selected_source(&mut self) {
        let Some(index) = self.selected_source else {
            return;
        };
        let Some(source) = self.sources.get(index).cloned() else {
            return;
        };
        self.selected_source = None;
        let remove_data = build_object_data(&[("inputName", &source.input_name)]);
        // レスポンスは不要なので drop する
        drop(self.client.send_obsdc_request("RemoveInput", remove_data));
    }

    fn connection_state_text(&self) -> &'static str {
        match self.connection_state {
            ConnectionState::Idle => "idle",
            ConnectionState::Bootstrapping => "bootstrapping",
            ConnectionState::Connecting => "connecting",
            ConnectionState::Connected => "connected",
            ConnectionState::Disconnecting => "disconnecting",
            ConnectionState::Closed => "closed",
        }
    }

    fn dc_state_text(state: ClientDcState) -> &'static str {
        match state {
            ClientDcState::NotCreated => "not-created",
            ClientDcState::Connecting => "connecting",
            ClientDcState::Open => "open",
            ClientDcState::Closing => "closing",
            ClientDcState::Closed => "closed",
        }
    }

    fn is_connected(&self) -> bool {
        self.connection_state == ConnectionState::Connected
    }
}

impl Drop for DevToolsApp {
    fn drop(&mut self) {
        // サーバー側のセッションを解放してからクライアントをシャットダウンする。
        // disconnect を送らずに終了すると、サーバーにセッションが残り、
        // 次回接続時に 409 Conflict になるため。
        self.client.disconnect();
        self.client.shutdown_and_join();
    }
}

/// GetInputList レスポンスからソース一覧をパースする。
fn parse_input_list(response: &RequestResponseData) -> Vec<SourceInfo> {
    let Some(data) = &response.response_data else {
        return Vec::new();
    };
    let Ok(inputs_value) = data.value().to_member("inputs").and_then(|m| m.required()) else {
        return Vec::new();
    };
    let Ok(inputs) = inputs_value.to_array() else {
        return Vec::new();
    };
    let mut sources = Vec::new();
    for input in inputs {
        let input_name: Option<String> = match input.to_member("inputName") {
            Ok(member) => member.required().ok().and_then(|v| v.try_into().ok()),
            Err(_) => None,
        };
        let input_kind: Option<String> = match input.to_member("inputKind") {
            Ok(member) => member.required().ok().and_then(|v| v.try_into().ok()),
            Err(_) => None,
        };
        if let (Some(input_name), Some(input_kind)) = (input_name, input_kind) {
            sources.push(SourceInfo {
                input_name,
                input_kind,
            });
        }
    }
    sources
}

/// GetInputPropertiesListPropertyItems レスポンスからカメラ一覧をパースする。
fn parse_camera_list(response: &RequestResponseData) -> Vec<CameraInfo> {
    let Some(data) = &response.response_data else {
        return Vec::new();
    };
    let Ok(items_value) = data
        .value()
        .to_member("propertyItems")
        .and_then(|m| m.required())
    else {
        return Vec::new();
    };
    let Ok(items) = items_value.to_array() else {
        return Vec::new();
    };
    let mut cameras = Vec::new();
    for item in items {
        let name: Option<String> = match item.to_member("itemName") {
            Ok(member) => member.required().ok().and_then(|v| v.try_into().ok()),
            Err(_) => None,
        };
        let device_id: Option<String> = match item.to_member("itemValue") {
            Ok(member) => member.required().ok().and_then(|v| v.try_into().ok()),
            Err(_) => None,
        };
        if let (Some(name), Some(device_id)) = (name, device_id) {
            cameras.push(CameraInfo { name, device_id });
        }
    }
    cameras
}

/// キー・バリューのペアから JSON オブジェクトを構築する。
fn build_object_data(pairs: &[(&str, &str)]) -> Option<nojson::RawJsonOwned> {
    let json = nojson::object(|f| {
        for (key, value) in pairs {
            f.member(*key, *value)?;
        }
        Ok(())
    });
    nojson::RawJsonOwned::parse(json.to_string()).ok()
}

/// 既存のソース名と重複しない入力名を生成する。
fn generate_unique_input_name(base_name: &str, sources: &[SourceInfo]) -> String {
    let existing: std::collections::HashSet<&str> =
        sources.iter().map(|s| s.input_name.as_str()).collect();
    if !existing.contains(base_name) {
        return base_name.to_owned();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base_name} {suffix}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
        suffix += 1;
    }
}

impl Render for DevToolsApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_connected = self.is_connected();
        let connection_text = self.connection_state_text();
        let error_text = self.last_error.clone();
        let bootstrap_url = default_bootstrap_url();
        let tracks: Vec<SharedString> = self
            .tracks
            .iter()
            .map(|track| format!("{}: {}", track.kind, track.track_id).into())
            .collect();
        let logs = self.logs.iter().map(log_line).collect::<Vec<_>>();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xd4d4d4))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_4()
                    .p_3()
                    .border_b_1()
                    .border_color(rgb(0x333333))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().child("State:"))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(connection_color(self.connection_state))
                                    .child(connection_text),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().child("Bootstrap URL:"))
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .bg(rgb(0x2d2d2d))
                                    .rounded_md()
                                    .text_sm()
                                    .child(bootstrap_url),
                            ),
                    )
                    .child(if is_connected {
                        div()
                            .id("disconnect-button")
                            .px_3()
                            .py_1()
                            .bg(rgb(0x8a2b2b))
                            .rounded_md()
                            .text_sm()
                            .text_color(rgb(0xffffff))
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _window, cx| {
                                this.disconnect();
                                cx.notify();
                            }))
                            .child("Disconnect")
                    } else {
                        div()
                            .id("connect-button")
                            .px_3()
                            .py_1()
                            .bg(rgb(0x2b5a2b))
                            .rounded_md()
                            .text_sm()
                            .text_color(rgb(0xffffff))
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _window, cx| {
                                this.connect();
                                cx.notify();
                            }))
                            .child("Connect")
                    }),
            )
            .child(if let Some(error) = error_text {
                div().p_2().bg(rgb(0x5a2b2b)).text_sm().child(error)
            } else {
                div()
            })
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(
                        // 左サイドバー: 接続情報
                        div()
                            .w_64()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .p_3()
                            .border_r_1()
                            .border_color(rgb(0x333333))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Connection"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .text_xs()
                                    .child(div().child(
                                        "signaling DataChannel: ".to_string()
                                            + Self::dc_state_text(self.signaling_dc_state),
                                    ))
                                    .child(div().child(
                                        "obsdc DataChannel: ".to_string()
                                            + Self::dc_state_text(self.obsdc_dc_state),
                                    )),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Tracks"),
                            )
                            .child(if tracks.is_empty() {
                                div().text_xs().child("no tracks")
                            } else {
                                div().flex().flex_col().gap_1().text_xs().children(tracks)
                            })
                            .child(sources_panel(self, is_connected, cx)),
                    )
                    .child(
                        // メイン: 映像表示
                        div()
                            .flex_1()
                            .m_3()
                            .bg(rgb(0x111111))
                            .rounded_md()
                            .overflow_hidden()
                            .child(video_area(&self.videos, self.connecting).into_any_element()),
                    ),
            )
            .child(
                // ログ表示
                div()
                    .id("logs")
                    .h_48()
                    .border_t_1()
                    .border_color(rgb(0x333333))
                    .p_2()
                    .overflow_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(logs),
            )
    }
}

/// 映像表示領域。トラックごとの最新フレームをグリッド表示する。
///
/// トラック数に応じて列数を変える (1 トラックは 1 列、2 トラックは 2 列)。
/// 各トラックはアスペクト比を維持してセル内に収める (ObjectFit::Contain)。
fn video_area(
    videos: &std::collections::BTreeMap<String, Arc<RenderImage>>,
    connecting: bool,
) -> gpui::AnyElement {
    if videos.is_empty() {
        return div()
            .size_full()
            .items_center()
            .justify_center()
            .flex()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x888888))
                    .child(if connecting {
                        "Connecting..."
                    } else {
                        "No video"
                    }),
            )
            .into_any_element();
    }
    div()
        .size_full()
        .flex()
        .flex_row()
        .gap_1()
        .children(videos.iter().map(|(track_id, video)| {
            div().flex_1().size_full().bg(rgb(0x000000)).child(
                img(ImageSource::Render(video.clone()))
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .id(gpui::ElementId::Name(gpui::SharedString::from(format!(
                        "video-{track_id}"
                    )))),
            )
        }))
        .into_any_element()
}

/// ソース管理パネル。
fn sources_panel(
    app: &DevToolsApp,
    is_connected: bool,
    cx: &mut Context<DevToolsApp>,
) -> gpui::AnyElement {
    let sources: Vec<SharedString> = app
        .sources
        .iter()
        .map(|source| {
            if source.input_kind == "video_capture_device" {
                format!("[camera] {}", source.input_name).into()
            } else {
                format!("{} ({})", source.input_name, source.input_kind).into()
            }
        })
        .collect();
    let selected = app.selected_source;
    let camera_names: Vec<SharedString> = app
        .cameras
        .iter()
        .map(|camera| camera.name.clone().into())
        .collect();
    let camera_selected = app.selected_camera;
    let show_camera_dialog = app.show_camera_dialog;
    let camera_loading = app.camera_loading;

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child("Sources"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("refresh-sources-button")
                        .px_2()
                        .py_0p5()
                        .bg(rgb(0x2d2d2d))
                        .rounded_md()
                        .text_xs()
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _window, cx| {
                            this.refresh_sources(cx);
                            cx.notify();
                        }))
                        .child("Refresh"),
                )
                .child(
                    div()
                        .id("add-camera-button")
                        .px_2()
                        .py_0p5()
                        .bg(rgb(0x2b5a2b))
                        .rounded_md()
                        .text_xs()
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _window, cx| {
                            this.open_camera_dialog(cx);
                            cx.notify();
                        }))
                        .child("Add Camera"),
                ),
        )
        .child(if sources.is_empty() {
            div()
                .text_xs()
                .text_color(rgb(0x888888))
                .child(if is_connected {
                    "no sources"
                } else {
                    "not connected"
                })
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .text_xs()
                .children(sources.iter().enumerate().map(|(index, name)| {
                    let is_selected = selected == Some(index);
                    div()
                        .id(gpui::ElementId::Name(gpui::SharedString::from(format!(
                            "source-{index}"
                        ))))
                        .px_2()
                        .py_0p5()
                        .rounded_md()
                        .bg(if is_selected {
                            rgb(0x3d5a3d)
                        } else {
                            rgb(0x2d2d2d)
                        })
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _window, cx| {
                            this.selected_source = Some(index);
                            cx.notify();
                        }))
                        .child(name.clone())
                }))
                .into_any_element()
        })
        .child(if selected.is_some() && is_connected {
            div()
                .id("remove-source-button")
                .px_2()
                .py_0p5()
                .bg(rgb(0x8a2b2b))
                .rounded_md()
                .text_xs()
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _window, cx| {
                    this.remove_selected_source();
                    cx.notify();
                }))
                .child("Remove Source")
                .into_any_element()
        } else {
            div().into_any_element()
        })
        .child(if show_camera_dialog {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(rgb(0x888888)).child("Camera:"))
                .child(if camera_loading {
                    div()
                        .text_xs()
                        .text_color(rgb(0x888888))
                        .child("Loading...")
                        .into_any_element()
                } else if camera_names.is_empty() {
                    div()
                        .text_xs()
                        .text_color(rgb(0xd46969))
                        .child("no camera found")
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .text_xs()
                        .children(camera_names.iter().enumerate().map(|(index, name)| {
                            let is_selected = camera_selected == Some(index);
                            div()
                                .id(gpui::ElementId::Name(gpui::SharedString::from(format!(
                                    "camera-{index}"
                                ))))
                                .px_2()
                                .py_0p5()
                                .rounded_md()
                                .bg(if is_selected {
                                    rgb(0x3d5a3d)
                                } else {
                                    rgb(0x2d2d2d)
                                })
                                .on_click(cx.listener(
                                    move |this, _: &gpui::ClickEvent, _window, cx| {
                                        this.selected_camera = Some(index);
                                        cx.notify();
                                    },
                                ))
                                .child(name.clone())
                        }))
                        .into_any_element()
                })
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(
                            div()
                                .id("cancel-camera-button")
                                .px_2()
                                .py_0p5()
                                .bg(rgb(0x2d2d2d))
                                .rounded_md()
                                .text_xs()
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _window, cx| {
                                    this.close_camera_dialog();
                                    cx.notify();
                                }))
                                .child("Cancel"),
                        )
                        .child(
                            div()
                                .id("add-camera-confirm-button")
                                .px_2()
                                .py_0p5()
                                .bg(rgb(0x2b5a2b))
                                .rounded_md()
                                .text_xs()
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _window, cx| {
                                    if let Some(index) = this.selected_camera
                                        && let Some(camera) = this.cameras.get(index).cloned()
                                    {
                                        this.add_camera(cx, camera);
                                    }
                                    cx.notify();
                                }))
                                .child("Add"),
                        ),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        })
        .into_any_element()
}

/// 接続状態に応じた色
fn connection_color(state: ConnectionState) -> gpui::Hsla {
    let color = match state {
        ConnectionState::Connected => rgb(0x4ec94e),
        ConnectionState::Bootstrapping | ConnectionState::Connecting => rgb(0xe0b64e),
        ConnectionState::Closed | ConnectionState::Disconnecting => rgb(0xd46969),
        ConnectionState::Idle => rgb(0x9a9a9a),
    };
    color.into()
}

/// ログ 1 行の表示
fn log_line(entry: &LogEntry) -> impl IntoElement {
    let color = match entry.level {
        LogLevel::Info => rgb(0xd4d4d4),
        LogLevel::Warn => rgb(0xe0b64e),
        LogLevel::Error => rgb(0xd46969),
    };
    let time = format!("{:08}", entry.timestamp_ms % 100_000_000);
    let category = entry.category.to_string();
    let message = entry.message.clone();
    div()
        .flex()
        .flex_row()
        .gap_2()
        .text_xs()
        .child(div().text_color(rgb(0x6a6a6a)).child(time))
        .child(div().w_16().text_color(rgb(0x6a8ab4)).child(category))
        .child(div().flex_1().text_color(color).child(message))
}

/// I420 フレームを GPUI の RenderImage (BGRA バイト列) に変換する
///
/// GPUI は BGRA バイト列 ([B][G][R][A] 順) を期待する。
/// libyuv のピクセル形式名は「ビッグエンディアンの 32 ビット値」であり、
/// リトルエンディアン環境ではメモリ順が逆になる。
/// `I420ToARGB` の出力はメモリ上 [B][G][R][A] となり、GPUI の期待と一致する。
fn to_render_image(frame: &OwnedVideoFrame) -> Option<Arc<RenderImage>> {
    use shiguredo_libyuv::{ArgbImageMut, I420Image};

    let width = frame.width as usize;
    let height = frame.height as usize;
    if width == 0 || height == 0 {
        return None;
    }

    let src = I420Image {
        y: &frame.y,
        y_stride: frame.stride_y as usize,
        u: &frame.u,
        u_stride: frame.stride_u as usize,
        v: &frame.v,
        v_stride: frame.stride_v as usize,
    };
    let mut bgra = vec![0_u8; width * height * 4];
    let mut dst = ArgbImageMut {
        data: &mut bgra,
        stride: width * 4,
    };
    let size = shiguredo_libyuv::ImageSize { width, height };
    shiguredo_libyuv::i420_to_argb(&src, &mut dst, size).ok()?;

    let bgra_image = image::RgbaImage::from_raw(width as u32, height as u32, bgra)?;
    Some(Arc::new(RenderImage::new([image::Frame::new(bgra_image)])))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// デフォルトの Bootstrap URL
fn default_bootstrap_url() -> SharedString {
    std::env::var("HISUI_BOOTSTRAP_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:4455/bootstrap".to_owned())
        .into()
}

// Entity 型の警告を抑えるための参照 (後で接続タスクで使用する)
#[allow(dead_code)]
type _AppEntity = Entity<DevToolsApp>;
