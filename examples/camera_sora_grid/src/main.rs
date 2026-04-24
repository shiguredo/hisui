use std::sync::atomic::{AtomicU64, Ordering};

use shiguredo_websocket::{
    ClientConnectionOptions, ConnectionEvent, ConnectionOutput, ConnectionState,
    WebSocketClientConnection,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// obsws リクエスト ID の連番生成
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

fn next_request_id() -> String {
    NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed).to_string()
}

// キャンバスサイズ（固定）
const CANVAS_WIDTH: f64 = 1920.0;
const CANVAS_HEIGHT: f64 = 1080.0;
// 2 カメラ + 最大 2 Sora トラックを 2x2 に並べるため、列数の上限を 2 にする
const MAX_COLUMNS: usize = 2;

// --- RandomSource ---

struct SecureRandom;

impl shiguredo_websocket::RandomSource for SecureRandom {
    fn masking_key(&mut self) -> [u8; 4] {
        let mut key = [0u8; 4];
        key.iter_mut().for_each(|b| *b = rand_byte());
        key
    }

    fn nonce(&mut self) -> [u8; 16] {
        let mut nonce = [0u8; 16];
        nonce.iter_mut().for_each(|b| *b = rand_byte());
        nonce
    }
}

fn rand_byte() -> u8 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // 起動ごとに異なるシーケンスを生成するため、時刻で seed を設定する
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        COUNTER.store(seed, Ordering::Relaxed);
    });
    let v = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mixed = v
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (mixed >> 33) as u8
}

// --- グリッド状態管理 ---

/// Scene 上の 1 ソース（カメラまたは Sora トラック）
struct SceneItem {
    input_name: String,
    scene_item_id: i64,
}

/// Sora からアタッチ済みの映像トラック
struct SoraTrack {
    track_id: String,
    item: SceneItem,
}

struct GridState {
    cameras: Vec<SceneItem>,
    sora_tracks: Vec<SoraTrack>,
    max_sora_tracks: usize,
}

impl GridState {
    fn new(max_sora_tracks: usize) -> Self {
        Self {
            cameras: Vec::new(),
            sora_tracks: Vec::new(),
            max_sora_tracks,
        }
    }

    /// 全ソースをカメラ → Sora の順に一列に並べる
    fn all_items(&self) -> Vec<&SceneItem> {
        self.cameras
            .iter()
            .chain(self.sora_tracks.iter().map(|t| &t.item))
            .collect()
    }
}

// --- obsws メッセージ生成 ---

fn make_identify_message() -> String {
    // SoraSource イベント (1 << 20) と標準イベントを subscribe する
    let event_subscriptions = (1 << 20) | ((1 << 10) - 1);
    nojson::object(|f| {
        f.member("op", 1)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("rpcVersion", 1)?;
                f.member("eventSubscriptions", event_subscriptions)
            }),
        )
    })
    .to_string()
}

fn make_request(
    request_type: &str,
    request_data: impl Fn(&mut nojson::JsonObjectFormatter<'_, '_, '_>) -> std::fmt::Result,
) -> (String, String) {
    let request_id = next_request_id();
    let rid = request_id.clone();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", request_type)?;
                f.member("requestId", rid.as_str())?;
                f.member("requestData", nojson::object(|f| request_data(f)))
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_create_camera_input_request(input_name: &str, device_id: Option<&str>) -> (String, String) {
    let iname = input_name.to_owned();
    let did = device_id.map(|s| s.to_owned());
    make_request("CreateInput", move |f| {
        f.member("sceneName", "Scene")?;
        f.member("inputName", iname.as_str())?;
        f.member("inputKind", "video_capture_device")?;
        f.member(
            "inputSettings",
            nojson::object(|f| {
                if let Some(d) = did.as_deref() {
                    f.member("device_id", d)?;
                }
                Ok(())
            }),
        )?;
        f.member("sceneItemEnabled", true)
    })
}

fn make_create_sora_source_input_request(input_name: &str) -> (String, String) {
    let name = input_name.to_owned();
    make_request("CreateInput", move |f| {
        f.member("sceneName", "Scene")?;
        f.member("inputName", name.as_str())?;
        f.member("inputKind", "sora_source")?;
        f.member("inputSettings", nojson::object(|_| Ok(())))?;
        f.member("sceneItemEnabled", true)
    })
}

fn make_attach_sora_source_track_request(
    input_name: &str,
    connection_id: &str,
    track_kind: &str,
) -> (String, String) {
    let iname = input_name.to_owned();
    let cid = connection_id.to_owned();
    let kind = track_kind.to_owned();
    make_request("HisuiAttachSoraSourceTrack", move |f| {
        f.member("inputName", iname.as_str())?;
        f.member("connectionId", cid.as_str())?;
        f.member("trackKind", kind.as_str())
    })
}

fn make_remove_input_request(input_name: &str) -> (String, String) {
    let name = input_name.to_owned();
    make_request("RemoveInput", move |f| f.member("inputName", name.as_str()))
}

fn make_set_scene_item_transform_request(
    scene_item_id: i64,
    pos_x: f64,
    pos_y: f64,
    bounds_width: f64,
    bounds_height: f64,
) -> (String, String) {
    make_request("SetSceneItemTransform", move |f| {
        f.member("sceneName", "Scene")?;
        f.member("sceneItemId", scene_item_id)?;
        f.member(
            "sceneItemTransform",
            nojson::object(|f| {
                f.member("positionX", pos_x)?;
                f.member("positionY", pos_y)?;
                f.member("boundsType", "OBS_BOUNDS_SCALE_INNER")?;
                f.member("boundsWidth", bounds_width)?;
                f.member("boundsHeight", bounds_height)
            }),
        )
    })
}

fn make_start_sora_subscriber_request(
    subscriber_name: &str,
    signaling_url: &str,
    channel_id: &str,
) -> (String, String) {
    let name = subscriber_name.to_owned();
    let url = signaling_url.to_owned();
    let ch = channel_id.to_owned();
    make_request("HisuiStartSoraSubscriber", move |f| {
        f.member("subscriberName", name.as_str())?;
        f.member("signalingUrls", [url.as_str()])?;
        f.member("channelId", ch.as_str())
    })
}

fn make_stop_sora_subscriber_request(subscriber_name: &str) -> (String, String) {
    let name = subscriber_name.to_owned();
    make_request("HisuiStopSoraSubscriber", move |f| {
        f.member("subscriberName", name.as_str())
    })
}

fn make_start_player_request() -> (String, String) {
    make_request("StartOutput", |f| f.member("outputName", "player"))
}

fn make_stop_player_request() -> (String, String) {
    make_request("StopOutput", |f| f.member("outputName", "player"))
}

// --- obsws レスポンス / イベントパース ---

/// op=7 のレスポンスから requestId と成否（成功時は responseData の JSON 文字列）を取得する
fn parse_request_response(text: &str) -> Option<(String, Result<String, String>)> {
    let json = nojson::RawJson::parse(text).ok()?;
    let root = json.value();
    let op: i64 = root
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if op != 7 {
        return None;
    }

    let d = root.to_member("d").ok()?.required().ok()?;
    let request_id: String = d
        .to_member("requestId")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    let request_status = d.to_member("requestStatus").ok()?.required().ok()?;
    let result: bool = request_status
        .to_member("result")
        .and_then(|v| v.required()?.try_into())
        .ok()?;

    if result {
        let response_data = d
            .to_member("responseData")
            .ok()
            .and_then(|v| v.optional())
            .map(|v| v.extract().to_string())
            .unwrap_or_default();
        Some((request_id, Ok(response_data)))
    } else {
        let comment: Option<String> = request_status
            .to_member("comment")
            .and_then(|v| v.try_into())
            .ok()
            .flatten();
        Some((
            request_id,
            Err(comment.unwrap_or_else(|| "unknown error".to_owned())),
        ))
    }
}

/// obsws イベントの種別（今回必要な分のみ）
enum ObswsEvent {
    SoraSourceTrackPublished {
        connection_id: String,
        track_kind: String,
        track_id: String,
    },
    SoraSourceTrackUnpublished {
        track_id: String,
    },
    Other,
}

fn parse_event(text: &str) -> Option<ObswsEvent> {
    let json = nojson::RawJson::parse(text).ok()?;
    let root = json.value();
    let op: i64 = root
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if op != 5 {
        return None;
    }

    let d = root.to_member("d").ok()?.required().ok()?;
    let event_type: String = d
        .to_member("eventType")
        .and_then(|v| v.required()?.try_into())
        .ok()?;

    match event_type.as_str() {
        "SoraSourceTrackPublished" => {
            let data = d.to_member("eventData").ok()?.required().ok()?;
            let connection_id: String = data
                .to_member("connectionId")
                .and_then(|v| v.required()?.try_into())
                .ok()?;
            let track_kind: String = data
                .to_member("trackKind")
                .and_then(|v| v.required()?.try_into())
                .ok()?;
            let track_id: String = data
                .to_member("trackId")
                .and_then(|v| v.required()?.try_into())
                .ok()?;
            Some(ObswsEvent::SoraSourceTrackPublished {
                connection_id,
                track_kind,
                track_id,
            })
        }
        "SoraSourceTrackUnpublished" => {
            let data = d.to_member("eventData").ok()?.required().ok()?;
            let track_id: String = data
                .to_member("trackId")
                .and_then(|v| v.required()?.try_into())
                .ok()?;
            Some(ObswsEvent::SoraSourceTrackUnpublished { track_id })
        }
        _ => Some(ObswsEvent::Other),
    }
}

/// CreateInput レスポンスから sceneItemId を取得する
fn parse_scene_item_id(response_data: &str) -> Option<i64> {
    let json = nojson::RawJson::parse(response_data).ok()?;
    let root = json.value();
    root.to_member("sceneItemId")
        .and_then(|v| v.required()?.try_into())
        .ok()
}

// --- WebSocket 通信ヘルパー ---

async fn flush_ws_output(
    ws: &mut WebSocketClientConnection<SecureRandom>,
    stream: &mut TcpStream,
) -> Result<(), String> {
    while let Some(output) = ws.poll_output() {
        match output {
            ConnectionOutput::SendData(data) => {
                stream
                    .write_all(&data)
                    .await
                    .map_err(|e| format!("failed to write: {e}"))?;
            }
            ConnectionOutput::CloseConnection => {
                return Err("connection closed by server".to_owned());
            }
            _ => {}
        }
    }
    Ok(())
}

async fn recv_text(
    ws: &mut WebSocketClientConnection<SecureRandom>,
    stream: &mut TcpStream,
) -> Result<String, String> {
    let mut buf = [0u8; 8192];
    loop {
        while let Some(event) = ws.poll_event() {
            match event {
                ConnectionEvent::TextMessage(text) => return Ok(text),
                ConnectionEvent::Close { code, reason } => {
                    return Err(format!("connection closed: code={code:?}, reason={reason}"));
                }
                ConnectionEvent::Error(e) => {
                    return Err(format!("websocket error: {e}"));
                }
                _ => {}
            }
        }
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("read error: {e}"))?;
        if n == 0 {
            return Err("connection closed".to_owned());
        }
        ws.feed_recv_buf(&buf[..n], shiguredo_websocket::Timestamp::from_millis(0))
            .map_err(|e| format!("websocket error: {e}"))?;
        flush_ws_output(ws, stream).await?;
    }
}

async fn recv_text_timeout(
    ws: &mut WebSocketClientConnection<SecureRandom>,
    stream: &mut TcpStream,
    timeout: std::time::Duration,
) -> Result<Option<String>, String> {
    match tokio::time::timeout(timeout, recv_text(ws, stream)).await {
        Ok(result) => result.map(Some),
        Err(_) => Ok(None),
    }
}

/// リクエストを送信し、対応するレスポンスを待つ。成功時は responseData の JSON 文字列を返す。
/// 待機中に受信したイベントは event_queue に蓄積する。
async fn send_request_and_wait(
    ws: &mut WebSocketClientConnection<SecureRandom>,
    stream: &mut TcpStream,
    request_id: &str,
    message: &str,
    event_queue: &mut Vec<String>,
) -> Result<String, String> {
    ws.send_text(message)
        .map_err(|e| format!("failed to send request: {e}"))?;
    flush_ws_output(ws, stream).await?;

    loop {
        let text = recv_text(ws, stream).await?;
        if let Some((resp_id, result)) = parse_request_response(&text)
            && resp_id == request_id
        {
            return result;
        }
        event_queue.push(text);
    }
}

// --- グリッドレイアウト / イベント処理 ---

/// 全ソースを 2x2 グリッドに並び替える
async fn update_grid_layout(
    ws: &mut WebSocketClientConnection<SecureRandom>,
    stream: &mut TcpStream,
    state: &GridState,
    event_queue: &mut Vec<String>,
) -> Result<(), String> {
    let items = state.all_items();
    let count = items.len();
    if count == 0 {
        return Ok(());
    }

    let cols = count.min(MAX_COLUMNS);
    let rows = count.div_ceil(cols);
    let cell_width = CANVAS_WIDTH / cols as f64;
    let cell_height = CANVAS_HEIGHT / rows as f64;

    for (i, item) in items.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let pos_x = col as f64 * cell_width;
        let pos_y = row as f64 * cell_height;

        let (req_id, msg) = make_set_scene_item_transform_request(
            item.scene_item_id,
            pos_x,
            pos_y,
            cell_width,
            cell_height,
        );
        send_request_and_wait(ws, stream, &req_id, &msg, event_queue).await?;
    }

    tracing::info!("grid layout updated: {count} items, {cols}x{rows}");

    Ok(())
}

async fn handle_event_message(
    text: &str,
    state: &mut GridState,
    ws: &mut WebSocketClientConnection<SecureRandom>,
    stream: &mut TcpStream,
    event_queue: &mut Vec<String>,
) -> Result<(), String> {
    let Some(event) = parse_event(text) else {
        return Ok(());
    };

    match event {
        ObswsEvent::SoraSourceTrackPublished {
            connection_id,
            track_kind,
            track_id,
        } => {
            if track_kind != "video" {
                tracing::debug!("skipping non-video track: kind={track_kind}, id={track_id}");
                return Ok(());
            }
            if state.sora_tracks.len() >= state.max_sora_tracks {
                tracing::info!(
                    "Sora track limit reached ({}): dropping track id={track_id}",
                    state.max_sora_tracks
                );
                return Ok(());
            }

            tracing::info!(
                "track arrived: connection={connection_id}, kind={track_kind}, id={track_id}"
            );

            let input_name = format!("sora_track_{}", state.sora_tracks.len());
            let (req_id, msg) = make_create_sora_source_input_request(&input_name);
            let response_data =
                send_request_and_wait(ws, stream, &req_id, &msg, event_queue).await?;
            let scene_item_id = parse_scene_item_id(&response_data).unwrap_or(0);
            tracing::info!(
                "CreateInput sora_source succeeded: {input_name} (sceneItemId={scene_item_id})"
            );

            let (req_id, msg) =
                make_attach_sora_source_track_request(&input_name, &connection_id, "video");
            send_request_and_wait(ws, stream, &req_id, &msg, event_queue).await?;
            tracing::info!("HisuiAttachSoraSourceTrack succeeded: {input_name} <- {connection_id}");

            state.sora_tracks.push(SoraTrack {
                track_id,
                item: SceneItem {
                    input_name,
                    scene_item_id,
                },
            });

            update_grid_layout(ws, stream, state, event_queue).await?;
        }
        ObswsEvent::SoraSourceTrackUnpublished { track_id } => {
            if let Some(pos) = state
                .sora_tracks
                .iter()
                .position(|t| t.track_id == track_id)
            {
                let entry = state.sora_tracks.remove(pos);
                tracing::info!(
                    "track removed: id={track_id}, input={}",
                    entry.item.input_name
                );

                let (req_id, msg) = make_remove_input_request(&entry.item.input_name);
                if let Err(e) = send_request_and_wait(ws, stream, &req_id, &msg, event_queue).await
                {
                    tracing::warn!("RemoveInput failed: {e}");
                }

                update_grid_layout(ws, stream, state, event_queue).await?;
            }
        }
        ObswsEvent::Other => {}
    }

    Ok(())
}

// --- main ---

fn main() -> noargs::Result<()> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = "camera_sora_grid";
    args.metadata_mut().app_description =
        "カメラ 2 台と Sora 受信トラック 2 本をグリッド合成して player に表示するサンプル";
    noargs::HELP_FLAG.take_help(&mut args);

    let verbose = noargs::flag("verbose")
        .short('v')
        .doc("詳細ログを出力する")
        .take(&mut args)
        .is_present();

    let host: String = noargs::opt("host")
        .default("127.0.0.1")
        .doc("hisui obsws 接続先ホスト")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let port: u16 = noargs::opt("port")
        .default("4455")
        .doc("hisui obsws 接続先ポート")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let camera_device_id_1: Option<String> = noargs::opt("camera-device-id-1")
        .doc("1 台目のカメラデバイス ID（省略時はデフォルトデバイス）")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let camera_device_id_2: Option<String> = noargs::opt("camera-device-id-2")
        .doc("2 台目のカメラデバイス ID（省略時はデフォルトデバイス）")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let signaling_url: String = noargs::opt("signaling-url")
        .doc("Sora シグナリング URL")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let channel_id: String = noargs::opt("channel-id")
        .default("sora")
        .doc("Sora チャネル ID")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let max_sora_tracks: usize = noargs::opt("max-sora-tracks")
        .default("2")
        .doc("Sora から受信する映像トラックの最大数")
        .take(&mut args)
        .then(|o| o.value().parse())?;

    args.finish()?;

    if verbose {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_writer(std::io::stderr)
            .init();
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let result = runtime.block_on(run(
        &host,
        port,
        camera_device_id_1.as_deref(),
        camera_device_id_2.as_deref(),
        &signaling_url,
        &channel_id,
        max_sora_tracks,
    ));

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

async fn run(
    host: &str,
    port: u16,
    camera_device_id_1: Option<&str>,
    camera_device_id_2: Option<&str>,
    signaling_url: &str,
    channel_id: &str,
    max_sora_tracks: usize,
) -> Result<(), String> {
    let subscriber_name = "camera_sora_grid_example";

    // TCP + WebSocket 接続
    let addr = format!("{host}:{port}");
    let mut stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("failed to connect to {addr}: {e}"))?;
    tracing::info!("TCP connected: {addr}");

    let host_port = format!("{host}:{port}");
    let options = ClientConnectionOptions::new(&host_port, "/")
        .protocol("obswebsocket.json")
        .ping_interval(0);
    let mut ws = WebSocketClientConnection::new(options, SecureRandom);
    ws.connect()
        .map_err(|e| format!("websocket connect error: {e}"))?;
    flush_ws_output(&mut ws, &mut stream).await?;

    let mut buf = [0u8; 8192];
    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("read error: {e}"))?;
        if n == 0 {
            return Err("connection closed during handshake".to_owned());
        }
        ws.feed_recv_buf(&buf[..n], shiguredo_websocket::Timestamp::from_millis(0))
            .map_err(|e| format!("websocket error: {e}"))?;
        flush_ws_output(&mut ws, &mut stream).await?;
        if ws.state() == ConnectionState::Connected {
            break;
        }
    }
    tracing::info!("WebSocket connected");

    // Hello / Identify / Identified
    let _hello = recv_text(&mut ws, &mut stream).await?;
    let identify = make_identify_message();
    ws.send_text(&identify)
        .map_err(|e| format!("failed to send Identify: {e}"))?;
    flush_ws_output(&mut ws, &mut stream).await?;
    let _identified = recv_text(&mut ws, &mut stream).await?;
    tracing::info!("obsws session established");

    // send_request_and_wait 中に受信したイベントを保持するキュー
    let mut event_queue: Vec<String> = Vec::new();
    let mut state = GridState::new(max_sora_tracks);

    // カメラ入力 2 台を作成する
    let cameras = [
        ("camera_0", camera_device_id_1),
        ("camera_1", camera_device_id_2),
    ];
    for (name, device_id) in cameras {
        let (req_id, msg) = make_create_camera_input_request(name, device_id);
        let response_data =
            send_request_and_wait(&mut ws, &mut stream, &req_id, &msg, &mut event_queue).await?;
        let scene_item_id = parse_scene_item_id(&response_data).unwrap_or(0);
        tracing::info!(
            "CreateInput camera succeeded: {name} (sceneItemId={scene_item_id}, device_id={:?})",
            device_id
        );
        state.cameras.push(SceneItem {
            input_name: name.to_owned(),
            scene_item_id,
        });
    }

    // カメラのみで先に 2x1 レイアウトを決める。Sora トラック到着時に再計算される。
    update_grid_layout(&mut ws, &mut stream, &state, &mut event_queue).await?;

    // player 起動
    let (req_id, msg) = make_start_player_request();
    send_request_and_wait(&mut ws, &mut stream, &req_id, &msg, &mut event_queue).await?;
    tracing::info!("player started");

    // Sora サブスクライバ起動
    let (req_id, msg) =
        make_start_sora_subscriber_request(subscriber_name, signaling_url, channel_id);
    send_request_and_wait(&mut ws, &mut stream, &req_id, &msg, &mut event_queue).await?;
    tracing::info!(
        "HisuiStartSoraSubscriber succeeded: signaling={signaling_url}, channel={channel_id}"
    );

    tracing::info!("waiting for Sora video tracks (max {max_sora_tracks}). Press Ctrl+C to stop.");

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        // キューに溜まっているイベントを先に処理する
        let queued = std::mem::take(&mut event_queue);
        for text in &queued {
            handle_event_message(text, &mut state, &mut ws, &mut stream, &mut event_queue).await?;
        }

        tokio::select! {
            _ = &mut ctrl_c => {
                tracing::info!("Ctrl+C received");
                break;
            }
            result = recv_text_timeout(&mut ws, &mut stream, std::time::Duration::from_millis(100)) => {
                let text = match result {
                    Ok(Some(text)) => text,
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::warn!("recv error: {e}");
                        break;
                    }
                };
                handle_event_message(&text, &mut state, &mut ws, &mut stream, &mut event_queue).await?;
            }
        }
    }

    // クリーンアップ
    let (req_id, msg) = make_stop_sora_subscriber_request(subscriber_name);
    if let Err(e) =
        send_request_and_wait(&mut ws, &mut stream, &req_id, &msg, &mut event_queue).await
    {
        tracing::warn!("HisuiStopSoraSubscriber failed: {e}");
    }

    let (req_id, msg) = make_stop_player_request();
    if let Err(e) =
        send_request_and_wait(&mut ws, &mut stream, &req_id, &msg, &mut event_queue).await
    {
        tracing::warn!("StopOutput player failed: {e}");
    }

    let _ = ws.close(shiguredo_websocket::CloseCode::NORMAL, "bye");
    flush_ws_output(&mut ws, &mut stream).await.ok();

    tracing::info!("done");
    Ok(())
}
