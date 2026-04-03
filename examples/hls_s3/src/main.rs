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

fn make_create_input_request(input_mp4_path: &str) -> (String, String) {
    let request_id = next_request_id();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "CreateInput")?;
                f.member("requestId", request_id.as_str())?;
                f.member(
                    "requestData",
                    nojson::object(|f| {
                        f.member("sceneName", "Scene")?;
                        f.member("inputName", "hls-s3-input")?;
                        f.member("inputKind", "mp4_file_source")?;
                        f.member(
                            "inputSettings",
                            nojson::object(|f| {
                                f.member("path", input_mp4_path)?;
                                f.member("loopPlayback", true)
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

/// HLS 出力の S3 destination 設定を構築する
fn make_set_output_settings_request(s3: &S3Params) -> (String, String) {
    let request_id = next_request_id();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SetOutputSettings")?;
                f.member("requestId", request_id.as_str())?;
                f.member(
                    "requestData",
                    nojson::object(|f| {
                        f.member("outputName", "hls")?;
                        f.member(
                            "outputSettings",
                            nojson::object(|f| {
                                f.member(
                                    "destination",
                                    nojson::object(|f| {
                                        f.member("type", "s3")?;
                                        f.member("bucket", s3.bucket.as_str())?;
                                        f.member("prefix", s3.prefix.as_str())?;
                                        f.member("region", s3.region.as_str())?;
                                        f.member("endpoint", s3.endpoint.as_str())?;
                                        f.member("usePathStyle", s3.use_path_style)?;
                                        f.member(
                                            "credentials",
                                            nojson::object(|f| {
                                                f.member("accessKeyId", s3.access_key_id.as_str())?;
                                                f.member(
                                                    "secretAccessKey",
                                                    s3.secret_access_key.as_str(),
                                                )
                                            }),
                                        )
                                    }),
                                )?;
                                f.member("segmentFormat", s3.segment_format.as_str())
                            }),
                        )
                    }),
                )
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_start_output_request() -> (String, String) {
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
                    nojson::object(|f| f.member("outputName", "hls")),
                )
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

fn make_stop_output_request() -> (String, String) {
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
                    nojson::object(|f| f.member("outputName", "hls")),
                )
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

fn make_get_output_status_request() -> (String, String) {
    let request_id = next_request_id();
    let msg = nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetOutputStatus")?;
                f.member("requestId", request_id.as_str())?;
                f.member(
                    "requestData",
                    nojson::object(|f| f.member("outputName", "hls")),
                )
            }),
        )
    })
    .to_string();
    (request_id, msg)
}

// --- obsws レスポンスパース ---

/// op=7 のレスポンスから requestId と成否を取得する。
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
        // responseData があれば JSON 文字列として返す
        let response_data = d
            .to_member("responseData")
            .ok()
            .and_then(|v| v.optional())
            .map(|v| v.to_string())
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
    }
}

// --- CLI パラメータ ---

struct S3Params {
    bucket: String,
    prefix: String,
    region: String,
    endpoint: String,
    use_path_style: bool,
    access_key_id: String,
    secret_access_key: String,
    segment_format: String,
}

// --- main ---

fn main() -> noargs::Result<()> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = "hls_s3";
    args.metadata_mut().app_description = "S3 に HLS 出力を行うサンプル";
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
    let input_mp4_path: String = noargs::opt("input-mp4-path")
        .doc("入力 MP4 ファイルパス")
        .take(&mut args)
        .then(|o| o.value().parse())?;

    // S3 設定
    let bucket: String = noargs::opt("s3-bucket")
        .doc("S3 バケット名")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let prefix: String = noargs::opt("s3-prefix")
        .default("hls")
        .doc("S3 オブジェクトキーの prefix")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let region: String = noargs::opt("s3-region")
        .default("us-east-1")
        .doc("S3 リージョン")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let endpoint: String = noargs::opt("s3-endpoint")
        .default("http://127.0.0.1:9000")
        .doc("S3 エンドポイント URL")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let use_path_style = noargs::flag("s3-path-style")
        .doc("S3 パススタイル URL を使用する")
        .take(&mut args)
        .is_present();
    let access_key_id: String = noargs::opt("s3-access-key")
        .default("admin")
        .doc("S3 アクセスキー ID")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let secret_access_key: String = noargs::opt("s3-secret-key")
        .default("admin")
        .doc("S3 シークレットアクセスキー")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let segment_format: String = noargs::opt("segment-format")
        .default("fmp4")
        .doc("セグメントフォーマット (mpegts / fmp4)")
        .take(&mut args)
        .then(|o| o.value().parse())?;
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

    let s3 = S3Params {
        bucket,
        prefix,
        region,
        endpoint,
        use_path_style,
        access_key_id,
        secret_access_key,
        segment_format,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let result = runtime.block_on(run(&host, port, &input_mp4_path, &s3, player));

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
    input_mp4_path: &str,
    s3: &S3Params,
    player: bool,
) -> Result<(), String> {
    // TCP 接続
    let addr = format!("{host}:{port}");
    let mut stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("failed to connect to {addr}: {e}"))?;
    tracing::info!("TCP connected: {addr}");

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
    tracing::info!("WebSocket connected");

    // Hello (op=0) を受信
    let hello = recv_text(&mut ws, &mut stream).await?;
    tracing::debug!("Hello received: {hello}");

    // Identify (op=1) を送信
    let identify = make_identify_message();
    ws.send_text(&identify)
        .map_err(|e| format!("failed to send Identify: {e}"))?;
    flush_ws_output(&mut ws, &mut stream).await?;

    // Identified (op=2) を受信
    let identified = recv_text(&mut ws, &mut stream).await?;
    tracing::debug!("Identified received: {identified}");
    tracing::info!("obsws session established");

    // 1. CreateInput: MP4 ファイルを入力として追加
    let (req_id, msg) = make_create_input_request(input_mp4_path);
    tracing::info!("CreateInput: {input_mp4_path}");
    send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
    tracing::info!("CreateInput succeeded");

    // 2. SetOutputSettings: HLS S3 設定
    let (req_id, msg) = make_set_output_settings_request(s3);
    tracing::info!(
        "SetOutputSettings: bucket={}, prefix={}, endpoint={}",
        s3.bucket,
        s3.prefix,
        s3.endpoint,
    );
    send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
    tracing::info!("SetOutputSettings succeeded");

    // 3. StartOutput: player 表示開始（オプション）
    if player {
        let (req_id, msg) = make_start_player_request();
        tracing::info!("StartOutput (player) requested");
        send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
        tracing::info!("player output started");
    }

    // 4. StartOutput: HLS 出力開始
    let (req_id, msg) = make_start_output_request();
    tracing::info!("StartOutput requested");
    send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
    tracing::info!("HLS S3 output started");

    // 4. GetOutputStatus: 出力状態確認
    let (req_id, msg) = make_get_output_status_request();
    let response_data = send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
    tracing::info!("GetOutputStatus: {response_data}");

    // Ctrl+C を待つ
    tracing::info!("Press Ctrl+C to stop");
    tokio::signal::ctrl_c()
        .await
        .map_err(|e| format!("failed to wait for Ctrl+C: {e}"))?;

    // 6. StopOutput: HLS 出力停止
    let (req_id, msg) = make_stop_output_request();
    tracing::info!("StopOutput requested");
    send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
    tracing::info!("HLS S3 output stopped");

    // 7. StopOutput: player 表示停止（オプション）
    if player {
        let (req_id, msg) = make_stop_player_request();
        tracing::info!("StopOutput (player) requested");
        send_request_and_wait(&mut ws, &mut stream, &req_id, &msg).await?;
        tracing::info!("player output stopped");
    }

    // WebSocket を閉じる
    let _ = ws.close(shiguredo_websocket::CloseCode::NORMAL, "bye");
    flush_ws_output(&mut ws, &mut stream).await.ok();

    Ok(())
}
