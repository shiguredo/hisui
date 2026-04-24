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
    use std::sync::Once;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    static INIT: Once = Once::new();
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

// --- obsws メッセージ生成 ---

fn make_identify_message() -> String {
    nojson::object(|f| {
        f.member("op", 1)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("rpcVersion", 1)?;
                f.member("eventSubscriptions", 0)
            }),
        )
    })
    .to_string()
}

fn make_set_record_directory_request(record_directory: &str) -> (String, String) {
    let request_id = next_request_id();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetRecordDirectory")?;
                f.member("requestId", request_id.as_str())?;
                f.member(
                    "requestData",
                    nojson::object(|f| f.member("recordDirectory", record_directory)),
                )
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_create_camera_input_request(input_name: &str, device_id: Option<&str>) -> (String, String) {
    let request_id = next_request_id();
    let rid = request_id.clone();
    let iname = input_name.to_owned();
    let did = device_id.map(|s| s.to_owned());
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "CreateInput")?;
                f.member("requestId", rid.as_str())?;
                f.member(
                    "requestData",
                    nojson::object(|f| {
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
                    }),
                )
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_create_microphone_input_request(
    input_name: &str,
    device_id: Option<&str>,
) -> (String, String) {
    let request_id = next_request_id();
    let rid = request_id.clone();
    let iname = input_name.to_owned();
    let did = device_id.map(|s| s.to_owned());
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "CreateInput")?;
                f.member("requestId", rid.as_str())?;
                f.member(
                    "requestData",
                    nojson::object(|f| {
                        f.member("sceneName", "Scene")?;
                        f.member("inputName", iname.as_str())?;
                        f.member("inputKind", "audio_capture_device")?;
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
                    }),
                )
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_get_device_id_items_request(input_name: &str) -> (String, String) {
    let request_id = next_request_id();
    let rid = request_id.clone();
    let iname = input_name.to_owned();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetInputPropertiesListPropertyItems")?;
                f.member("requestId", rid.as_str())?;
                f.member(
                    "requestData",
                    nojson::object(|f| {
                        f.member("inputName", iname.as_str())?;
                        f.member("propertyName", "device_id")
                    }),
                )
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_remove_input_request(input_name: &str) -> (String, String) {
    let request_id = next_request_id();
    let rid = request_id.clone();
    let iname = input_name.to_owned();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "RemoveInput")?;
                f.member("requestId", rid.as_str())?;
                f.member(
                    "requestData",
                    nojson::object(|f| f.member("inputName", iname.as_str())),
                )
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_start_record_request() -> (String, String) {
    let request_id = next_request_id();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "StartRecord")?;
                f.member("requestId", request_id.as_str())
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_stop_record_request() -> (String, String) {
    let request_id = next_request_id();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "StopRecord")?;
                f.member("requestId", request_id.as_str())
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_start_player_request() -> (String, String) {
    let request_id = next_request_id();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "StartOutput")?;
                f.member("requestId", request_id.as_str())?;
                f.member(
                    "requestData",
                    nojson::object(|f| f.member("outputName", "player")),
                )
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_stop_player_request() -> (String, String) {
    let request_id = next_request_id();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "StopOutput")?;
                f.member("requestId", request_id.as_str())?;
                f.member(
                    "requestData",
                    nojson::object(|f| f.member("outputName", "player")),
                )
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

// --- obsws レスポンスパース ---

/// op=7 のレスポンスから requestId と成否を取得する。
/// 成功時は responseData の JSON 文字列、失敗時はエラーコメント。
/// op=7 以外のメッセージは None を返す。
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

/// GetInputPropertiesListPropertyItems レスポンスから itemValue の配列を取り出す
fn parse_property_item_values(response_data: &str) -> Vec<String> {
    let Ok(json) = nojson::RawJson::parse(response_data) else {
        return Vec::new();
    };
    let root = json.value();
    let Ok(items_member) = root.to_member("propertyItems") else {
        return Vec::new();
    };
    let Ok(items_value) = items_member.required() else {
        return Vec::new();
    };
    let Ok(items) = items_value.to_array() else {
        return Vec::new();
    };
    items
        .filter_map(|item| {
            let value = item.to_member("itemValue").ok()?.required().ok()?;
            value.try_into().ok()
        })
        .collect()
}

// --- WebSocket 通信ヘルパー ---

/// WebSocket の出力バッファを TCP に書き出す
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

/// WebSocket 経由でテキストメッセージを受信するまで待つ
async fn recv_text(
    ws: &mut WebSocketClientConnection<SecureRandom>,
    stream: &mut TcpStream,
) -> Result<String, String> {
    let mut buf = [0u8; 8192];
    loop {
        // まずイベントキューを確認
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
        // データを読み込む
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

/// obsws リクエストを送信し、対応するレスポンスを待つ。
/// 成功時は responseData の JSON 文字列（空の場合もある）を返す。
async fn send_request_and_wait(
    ws: &mut WebSocketClientConnection<SecureRandom>,
    stream: &mut TcpStream,
    request_id: &str,
    message: &str,
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
        // 対象外のメッセージは無視して続行
    }
}

/// カメラデバイスの device_id 一覧を取得する。
/// 列挙用のダミー video_capture_device input を作成して照会し、終わったら削除する。
async fn enumerate_camera_device_ids(
    ws: &mut WebSocketClientConnection<SecureRandom>,
    stream: &mut TcpStream,
) -> Result<Vec<String>, String> {
    let dummy_name = "__camera_record_enumerate_camera__";

    let (req_id, msg) = make_create_camera_input_request(dummy_name, None);
    send_request_and_wait(ws, stream, &req_id, &msg).await?;

    let (req_id, msg) = make_get_device_id_items_request(dummy_name);
    let response_data = send_request_and_wait(ws, stream, &req_id, &msg).await?;
    let device_ids = parse_property_item_values(&response_data);

    let (req_id, msg) = make_remove_input_request(dummy_name);
    if let Err(e) = send_request_and_wait(ws, stream, &req_id, &msg).await {
        tracing::warn!("RemoveInput for enumerate dummy failed: {e}");
    }

    Ok(device_ids)
}

/// マイクデバイスの device_id 一覧を取得する。
/// 列挙用のダミー audio_capture_device input を作成して照会し、終わったら削除する。
async fn enumerate_microphone_device_ids(
    ws: &mut WebSocketClientConnection<SecureRandom>,
    stream: &mut TcpStream,
) -> Result<Vec<String>, String> {
    let dummy_name = "__camera_record_enumerate_mic__";

    let (req_id, msg) = make_create_microphone_input_request(dummy_name, None);
    send_request_and_wait(ws, stream, &req_id, &msg).await?;

    let (req_id, msg) = make_get_device_id_items_request(dummy_name);
    let response_data = send_request_and_wait(ws, stream, &req_id, &msg).await?;
    let device_ids = parse_property_item_values(&response_data);

    let (req_id, msg) = make_remove_input_request(dummy_name);
    if let Err(e) = send_request_and_wait(ws, stream, &req_id, &msg).await {
        tracing::warn!("RemoveInput for enumerate dummy failed: {e}");
    }

    Ok(device_ids)
}

// --- main ---

fn main() -> noargs::Result<()> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = "camera_record";
    args.metadata_mut().app_description =
        "カメラとマイクを入力として MP4 ファイルに録画するサンプル";
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
    let record_directory: String = noargs::opt("record-directory")
        .doc("録画先ディレクトリ")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let camera_only = noargs::flag("camera-only")
        .doc("カメラのみ使用する（マイクなし）")
        .take(&mut args)
        .is_present();
    let camera_device_id: Option<String> = noargs::opt("camera-device-id")
        .doc("カメラデバイス ID（省略時は接続中のカメラから自動選択）")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let mic_device_id: Option<String> = noargs::opt("mic-device-id")
        .doc("マイクデバイス ID（未指定かつ --camera-only なしで録画するとマイク作成が失敗する点に注意）")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let player = noargs::flag("player")
        .doc("player output を起動してウィンドウ表示する")
        .take(&mut args)
        .is_present();

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
        &record_directory,
        camera_only,
        camera_device_id.as_deref(),
        mic_device_id.as_deref(),
        player,
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
    record_directory: &str,
    camera_only: bool,
    camera_device_id: Option<&str>,
    mic_device_id: Option<&str>,
    player: bool,
) -> Result<(), String> {
    // TCP 接続
    let addr = format!("{host}:{port}");
    let mut stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("failed to connect to {addr}: {e}"))?;
    tracing::info!("TCP 接続完了: {addr}");

    // WebSocket ハンドシェイク
    let host_port = format!("{host}:{port}");
    let options = ClientConnectionOptions::new(&host_port, "/")
        .protocol("obswebsocket.json")
        .ping_interval(0);
    let mut ws = WebSocketClientConnection::new(options, SecureRandom);
    ws.connect()
        .map_err(|e| format!("websocket connect error: {e}"))?;
    flush_ws_output(&mut ws, &mut stream).await?;

    // ハンドシェイク完了を待つ
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
    tracing::info!("WebSocket 接続完了");

    // Hello (op=0) を受信
    let hello = recv_text(&mut ws, &mut stream).await?;
    tracing::debug!("Hello 受信: {hello}");

    // Identify (op=1) を送信
    let identify = make_identify_message();
    ws.send_text(&identify)
        .map_err(|e| format!("failed to send Identify: {e}"))?;
    flush_ws_output(&mut ws, &mut stream).await?;

    // Identified (op=2) を受信
    let identified = recv_text(&mut ws, &mut stream).await?;
    tracing::debug!("Identified 受信: {identified}");
    tracing::info!("obsws セッション確立");

    // 1. SetRecordDirectory: 録画先ディレクトリを設定
    let (req_id, msg) = make_set_record_directory_request(record_directory);
    tracing::info!("SetRecordDirectory 送信: {record_directory}");
    send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
    tracing::info!("SetRecordDirectory 成功");

    // 2. カメラ device_id を決定する。
    //    hisui の video_capture_device は device_id 未指定だと source processor が起動せず
    //    録画映像が黒になるため、必ず明示的に指定する。
    let resolved_camera_device_id = if let Some(d) = camera_device_id {
        Some(d.to_owned())
    } else {
        let device_ids = enumerate_camera_device_ids(&mut ws, &mut stream).await?;
        tracing::info!("available camera device_ids: {} found", device_ids.len());
        device_ids.into_iter().next()
    };
    let resolved_camera_device_id = resolved_camera_device_id
        .ok_or_else(|| "no camera device found; specify --camera-device-id manually".to_owned())?;

    // 3. CreateInput: カメラ入力を追加
    let (req_id, msg) =
        make_create_camera_input_request("camera", Some(resolved_camera_device_id.as_str()));
    tracing::info!(
        "CreateInput 送信: video_capture_device (device_id={resolved_camera_device_id})"
    );
    send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
    tracing::info!("CreateInput 成功: camera");

    // 4. CreateInput: マイク入力を追加（--camera-only でない場合）
    //    hisui の audio_capture_device も device_id 未指定では source processor が起動しないため、
    //    CLI 指定がなければ GetInputPropertiesListPropertyItems で列挙して 1 件目を使う。
    //    検出できなかった場合はマイク無しで録画を続行する。
    if !camera_only {
        let resolved_mic_device_id = if let Some(d) = mic_device_id {
            Some(d.to_owned())
        } else {
            let ids = enumerate_microphone_device_ids(&mut ws, &mut stream).await?;
            tracing::info!("available microphone device_ids: {} found", ids.len());
            ids.into_iter().next()
        };
        if let Some(mic_id) = resolved_mic_device_id {
            let (req_id, msg) = make_create_microphone_input_request("microphone", Some(&mic_id));
            tracing::info!("CreateInput 送信: audio_capture_device (device_id={mic_id})");
            send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
            tracing::info!("CreateInput 成功: microphone");
        } else {
            tracing::warn!(
                "no microphone device found: recording without audio (use --camera-only to silence this warning)"
            );
        }
    }

    // 4. StartOutput player: player ウィンドウ表示（--player 指定時）
    if player {
        let (req_id, msg) = make_start_player_request();
        tracing::info!("StartOutput player 送信");
        send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
        tracing::info!("player 開始");
    }

    // 5. StartRecord: 録画開始
    let (req_id, msg) = make_start_record_request();
    tracing::info!("StartRecord 送信");
    send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
    tracing::info!("録画開始");

    // Ctrl+C を待つ
    tracing::info!("Ctrl+C で停止します");
    tokio::signal::ctrl_c()
        .await
        .map_err(|e| format!("failed to wait for Ctrl+C: {e}"))?;

    // 6. StopRecord: 録画停止
    let (req_id, msg) = make_stop_record_request();
    tracing::info!("StopRecord 送信");
    send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
    tracing::info!("録画停止");

    // 7. StopOutput player: player ウィンドウ閉じ（--player 指定時）
    if player {
        let (req_id, msg) = make_stop_player_request();
        tracing::info!("StopOutput player 送信");
        send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
        tracing::info!("player 停止");
    }

    // WebSocket を閉じる
    let _ = ws.close(shiguredo_websocket::CloseCode::NORMAL, "bye");
    flush_ws_output(&mut ws, &mut stream).await.ok();

    Ok(())
}
