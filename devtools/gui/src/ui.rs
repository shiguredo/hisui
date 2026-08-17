//! DevTools の GPUI UI。
//!
//! ブラウザ版 devtools の P2P ページ (`devtools/src/pages/P2PPage.tsx`) 相当の UI を提供する。
//! 接続状態・DataChannel 状態・映像再生・ログ表示・ソース管理 (OBS WebSocket リクエスト) を実装する。
//! 映像の GPUI 表示は [`hisui_devtools_gui::video::VideoDisplay`] に委譲する。

mod frame;

use std::collections::VecDeque;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{Context, Entity, Render, SharedString, Window, div, rgb};

use hisui_devtools_gui::obsdc::RequestResponseData;
use hisui_devtools_gui::p2p::{
    BootstrapConfig, ClientEvent, ConnectionState, DataChannelState as ClientDcState, IceServer,
    LogCategory, LogEntry, LogLevel, OwnedVideoFrame, P2PClientHandle, spawn_client,
};
use hisui_devtools_gui::video::VideoDisplay;

use frame::to_render_image;

/// 表示更新間隔。サーバーは 30fps で送るため、これより細かく変換すると追いつかない。
const DISPLAY_INTERVAL: Duration = Duration::from_millis(33);

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
    /// ログカテゴリごとの表示/非表示 (true = 表示)
    log_filter_pc: bool,
    log_filter_signaling: bool,
    log_filter_obsdc: bool,
    /// 映像再生コンポーネント
    video_display: Entity<VideoDisplay>,
    last_error: Option<String>,
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
        let (client, event_rx) = spawn_client().expect("P2P クライアントの起動に失敗しました");
        let video_display = cx.new(VideoDisplay::new);

        let mut app = Self {
            client,
            connection_state: ConnectionState::Idle,
            signaling_dc_state: ClientDcState::NotCreated,
            obsdc_dc_state: ClientDcState::NotCreated,
            tracks: Vec::new(),
            logs: VecDeque::new(),
            log_filter_pc: true,
            log_filter_signaling: true,
            log_filter_obsdc: true,
            video_display,
            last_error: None,
            current_scene: String::new(),
            sources: Vec::new(),
            show_camera_dialog: false,
            camera_loading: false,
            cameras: Vec::new(),
            selected_camera: None,
            selected_source: None,
            probe_input_name: None,
        };

        app.start_event_tasks(cx, event_rx);
        app
    }

    /// クライアントからのイベントを UI に反映するタスクを起動する。
    ///
    /// tokio のチャネルは waker ベースで tokio ランタイムが無くても await できるため、
    /// GPUI のフォアグラウンドエグゼキュータ (メインスレッド) 上で受信する。
    /// 映像フレームは `TrackAdded` で受信タスクを起動し、変換後に [`VideoDisplay`] へ渡す。
    fn start_event_tasks(
        &mut self,
        cx: &mut Context<Self>,
        mut event_rx: tokio::sync::mpsc::UnboundedReceiver<ClientEvent>,
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
    }

    /// トラックの映像フレーム受信タスクを起動する。
    ///
    /// I420 を [`gpui::RenderImage`] に変換し、GPUI コンポーネントへ渡す。
    /// タイルの描画自体は [`VideoDisplay`] が担当する。
    fn start_frame_task(
        &mut self,
        cx: &mut Context<Self>,
        track_id: String,
        mut frame_rx: tokio::sync::watch::Receiver<Option<OwnedVideoFrame>>,
    ) {
        cx.spawn(async move |app, cx| {
            // 表示レートを 30fps に制限する。
            // サーバーは各映像トラックを 30fps で送信するため、
            // 制限しないと色変換が追いつかずカクカクする。
            let mut last_display = std::time::Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(std::time::Instant::now);
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
                if last_log.elapsed() >= Duration::from_secs(5) {
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
                // 表示レートを制限する。制限を超えたフレームは
                // watch チャネルで最新フレームに置き換えられる
                if last_display.elapsed() < DISPLAY_INTERVAL {
                    continue;
                }
                last_display = std::time::Instant::now();
                // 色変換はバックグラウンドエグゼキュータで行い、メインスレッドをブロックしない
                let frame_width = frame.width;
                let frame_height = frame.height;
                let render_image = cx
                    .background_executor()
                    .spawn(async move { to_render_image(&frame) })
                    .await;
                let Some(render_image) = render_image else {
                    continue;
                };
                if app
                    .update(cx, |app, cx| {
                        // 削除済みトラックのフレームは無視する。
                        // トラック削除後もサーバーからフレームが届き続けることがあり、
                        // 映像タイルが復活してしまうのを防ぐ。
                        if !app.tracks.iter().any(|track| track.track_id == track_id) {
                            return;
                        }
                        app.video_display.update(cx, |display, cx| {
                            display.show_frame(
                                track_id.clone(),
                                render_image,
                                frame_width,
                                frame_height,
                                cx,
                            );
                        });
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_client_event(&mut self, event: ClientEvent, cx: &mut Context<Self>) {
        match event {
            ClientEvent::ConnectionStateChanged(state) => {
                self.connection_state = state;
                let connecting = matches!(
                    state,
                    ConnectionState::Bootstrapping | ConnectionState::Connecting
                );
                self.video_display.update(cx, |display, cx| {
                    display.set_connecting(connecting, cx);
                });
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
            ClientEvent::TrackAdded {
                track_id,
                kind,
                frame_rx,
            } => {
                self.tracks.push(TrackInfo {
                    track_id: track_id.clone(),
                    kind,
                });
                self.start_frame_task(cx, track_id, frame_rx);
            }
            ClientEvent::TrackRemoved { track_id } => {
                self.tracks.retain(|track| track.track_id != track_id);
                self.video_display.update(cx, |display, cx| {
                    display.remove_track(&track_id, cx);
                });
            }
            ClientEvent::CloseReceived { code, reason } => {
                self.last_error = Some(format!("{code:?}: {reason}"));
                self.connection_state = ConnectionState::Closed;
                self.video_display.update(cx, |display, cx| {
                    display.set_connecting(false, cx);
                });
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

    /// 指定したログカテゴリの表示/非表示を切り替える。
    fn toggle_log_filter(&mut self, category: LogCategory) {
        let enabled = match category {
            LogCategory::Pc => &mut self.log_filter_pc,
            LogCategory::Signaling => &mut self.log_filter_signaling,
            LogCategory::Obsdc => &mut self.log_filter_obsdc,
        };
        *enabled = !*enabled;
    }

    /// 指定したログカテゴリが表示対象かどうかを返す。
    fn is_log_category_visible(&self, category: LogCategory) -> bool {
        match category {
            LogCategory::Pc => self.log_filter_pc,
            LogCategory::Signaling => self.log_filter_signaling,
            LogCategory::Obsdc => self.log_filter_obsdc,
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
            let _ = app.update::<_, gpui::AsyncApp>(cx, |app, cx| {
                app.camera_loading = false;
                // ソース一覧を更新して追加結果を反映する
                app.refresh_sources(cx);
            });
        })
        .detach();
    }

    /// 選択中のソースを削除する。
    fn remove_selected_source(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_source else {
            return;
        };
        let Some(source) = self.sources.get(index).cloned() else {
            return;
        };
        self.selected_source = None;
        let remove_data = build_object_data(&[("inputName", &source.input_name)]);
        let client = self.client.clone();
        let app = cx.entity();
        // 削除完了後にソース一覧を更新して結果を反映する
        cx.spawn(async move |_weak, cx| {
            let _ = client.send_obsdc_request("RemoveInput", remove_data).await;
            let _ = app.update::<_, gpui::AsyncApp>(cx, |app, cx| {
                app.refresh_sources(cx);
            });
        })
        .detach();
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
        let logs = self
            .logs
            .iter()
            .filter(|entry| self.is_log_category_visible(entry.category))
            .map(log_line)
            .collect::<Vec<_>>();

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
                            .child(div().text_sm().child("状態:"))
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
                            .child("切断")
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
                            .child("接続")
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
                                    .child("接続情報"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .text_xs()
                                    .child(div().child(
                                        "シグナリング DataChannel: ".to_string()
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
                                    .child("トラック"),
                            )
                            .child(if tracks.is_empty() {
                                div().text_xs().child("トラックなし")
                            } else {
                                div().flex().flex_col().gap_1().text_xs().children(tracks)
                            })
                            .child(sources_panel(self, is_connected, cx)),
                    )
                    .child(
                        // メイン: 映像再生
                        div()
                            .flex_1()
                            .m_3()
                            .bg(rgb(0x111111))
                            .rounded_md()
                            .overflow_hidden()
                            .child(self.video_display.clone()),
                    ),
            )
            .child(
                // ログ表示
                div()
                    .h_48()
                    .border_t_1()
                    .border_color(rgb(0x333333))
                    .flex()
                    .flex_col()
                    .child(log_filter_bar(self, cx))
                    .child(
                        div()
                            .id("logs")
                            .flex_1()
                            .p_2()
                            .overflow_scroll()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(logs),
                    ),
            )
    }
}

/// ログカテゴリのフィルターバー。
fn log_filter_bar(app: &DevToolsApp, cx: &mut Context<DevToolsApp>) -> gpui::AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .p_2()
        .border_b_1()
        .border_color(rgb(0x333333))
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x888888))
                .child("ログフィルター:"),
        )
        .child(log_filter_button(app, LogCategory::Pc, cx))
        .child(log_filter_button(app, LogCategory::Signaling, cx))
        .child(log_filter_button(app, LogCategory::Obsdc, cx))
        .into_any_element()
}

/// ログカテゴリ 1 つの表示/非表示トグルボタン。
fn log_filter_button(
    app: &DevToolsApp,
    category: LogCategory,
    cx: &mut Context<DevToolsApp>,
) -> impl IntoElement {
    let enabled = app.is_log_category_visible(category);
    div()
        .id(gpui::ElementId::Name(gpui::SharedString::from(format!(
            "log-filter-{category}"
        ))))
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .bg(if enabled {
            rgb(0x2d2d2d)
        } else {
            rgb(0x1e1e1e)
        })
        .text_color(if enabled {
            rgb(0xd4d4d4)
        } else {
            rgb(0x666666)
        })
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _window, cx| {
            this.toggle_log_filter(category);
            cx.notify();
        }))
        .child(category.to_string())
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
                        .child("ソース"),
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
                        .child("更新"),
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
                        .child("カメラ追加"),
                ),
        )
        .child(if sources.is_empty() {
            div()
                .text_xs()
                .text_color(rgb(0x888888))
                .child(if is_connected {
                    "ソースがありません"
                } else {
                    "未接続"
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
                    this.remove_selected_source(cx);
                    cx.notify();
                }))
                .child("ソース削除")
                .into_any_element()
        } else {
            div().into_any_element()
        })
        .child(if show_camera_dialog {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(rgb(0x888888)).child("カメラ:"))
                .child(if camera_loading {
                    div()
                        .text_xs()
                        .text_color(rgb(0x888888))
                        .child("読み込み中...")
                        .into_any_element()
                } else if camera_names.is_empty() {
                    div()
                        .text_xs()
                        .text_color(rgb(0xd46969))
                        .child("カメラが見つかりません")
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
                                .child("キャンセル"),
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
                                .child("追加"),
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
