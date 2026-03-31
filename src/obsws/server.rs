use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use shiguredo_http11::uri::Uri;
use shiguredo_http11::{Request, RequestDecoder, Response, ResponseDecoder};
use shiguredo_websocket::{
    CloseCode, ConnectionEvent, ConnectionOutput, ConnectionState, ServerConnectionOptions,
    WebSocketServerConnection,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpListener;

use crate::endpoint_http_bootstrap::BootstrapEndpoint;
use crate::obsws::auth::ObswsAuthentication;
use crate::obsws::input_registry::ObswsInputRegistry;
use crate::obsws::message::ObswsSessionStats;
use crate::obsws::protocol::OBSWS_SUBPROTOCOL;
use crate::obsws::session::{ObswsSession, SessionAction};
use crate::tcp::{ServerTcpOrTlsStream, TcpOrTlsStream, create_server_tls_acceptor};

type TlsAcceptor = Arc<tokio_rustls::TlsAcceptor>;

/// Program 出力の状態。常駐ミキサーのプロセッサ ID と固定トラック ID を保持する。
pub struct ProgramOutputState {
    pub scene_uuid: String,
    pub video_track_id: crate::TrackId,
    pub audio_track_id: crate::TrackId,
    pub video_mixer_processor_id: crate::ProcessorId,
    pub audio_mixer_processor_id: crate::ProcessorId,
    pub source_processor_ids: Vec<crate::ProcessorId>,
}

/// upstream リバースプロキシ設定
struct UpstreamConfig {
    host: String,
    port: u16,
    tls: bool,
    /// upstream URL のパス部分（プレフィックスとして使用）
    path_prefix: String,
}

/// hop-by-hop ヘッダーリスト（RFC 9110）
const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// クライアント切断かどうかを判定する
fn is_client_disconnect(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
    )
}

fn request_path(uri: &str) -> &str {
    uri.split_once('?').map_or(uri, |(path, _)| path)
}

#[expect(clippy::too_many_arguments)]
pub async fn run_server(
    addr: SocketAddr,
    password: Option<String>,
    default_record_dir: PathBuf,
    ui_remote_url: Option<String>,
    https_cert_path: Option<PathBuf>,
    https_key_path: Option<PathBuf>,
    pipeline_config: crate::MediaPipelineConfig,
    canvas_width: crate::types::EvenUsize,
    canvas_height: crate::types::EvenUsize,
    frame_rate: crate::video::FrameRate,
    state_file_path: Option<PathBuf>,
) -> crate::Result<()> {
    let upstream_config = parse_upstream_config(ui_remote_url.as_deref())?;

    // TLS が指定されている場合は TlsAcceptor を作成する
    let tls_acceptor: Option<TlsAcceptor> = if let Some((cert_path, key_path)) =
        https_cert_path.zip(https_key_path)
    {
        Some(
            create_server_tls_acceptor(&cert_path, &key_path)
                .await
                .map_err(|e| crate::Error::new(format!("failed to create TLS acceptor: {e}")))?,
        )
    } else {
        None
    };

    let scheme = if tls_acceptor.is_some() {
        "https"
    } else {
        "http"
    };

    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| crate::Error::new(format!("failed to bind obsws listener: {e}")))?;
    tracing::info!("obsws server listening on {scheme}://{addr}");

    // state file の読み込みと初期値への反映
    let (effective_record_dir, loaded_state, resolved_state_file_path) =
        if let Some(ref path) = state_file_path {
            let abs_path = std::path::absolute(path).map_err(|e| {
                crate::Error::new(format!(
                    "failed to resolve state file path {}: {e}",
                    path.display()
                ))
            })?;
            let state = crate::obsws::state_file::load_state_file(&abs_path)?;
            let record_dir = state
                .record
                .as_ref()
                .map(|r| r.record_directory.clone())
                .unwrap_or(default_record_dir);
            (record_dir, Some(state), Some(abs_path))
        } else {
            (default_record_dir, None, None)
        };

    let mut input_registry = ObswsInputRegistry::new(
        effective_record_dir,
        canvas_width,
        canvas_height,
        frame_rate,
        resolved_state_file_path,
    );

    // state file から読み込んだ設定を反映する
    if let Some(state) = loaded_state {
        if let Some(stream) = &state.stream {
            input_registry.set_stream_service_settings(stream.to_stream_service_settings());
        }
        if let Some(settings) = state.rtmp_outbound {
            input_registry.set_rtmp_outbound_settings(settings);
        }
        if let Some(settings) = state.sora {
            input_registry.set_sora_publisher_settings(settings);
        }
        if let Some(settings) = state.hls {
            input_registry.set_hls_settings(settings);
        }
        if let Some(settings) = state.dash {
            input_registry.set_dash_settings(settings);
        }
        // scene / input の復元
        if let Some(scenes) = state.scenes {
            let inputs = state.inputs.unwrap_or_default();
            input_registry.restore_scenes_and_inputs(
                scenes,
                inputs,
                state.current_program_scene,
                state.current_preview_scene,
                state.next_input_id,
                state.next_scene_id,
                state.next_scene_item_id,
            )?;
        }
    }

    let pipeline = crate::MediaPipeline::new_with_config(pipeline_config)?;
    let pipeline_handle = pipeline.handle();
    tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    if !started {
        tracing::debug!("obsws initial start trigger was already completed");
    }

    // Program 出力を初期化する（常駐ミキサー）
    let program_output = {
        let scene_inputs = input_registry.list_current_program_scene_input_entries();
        let output_plan = crate::obsws::output_plan::build_composed_output_plan(
            &scene_inputs,
            crate::obsws::source::ObswsOutputKind::Program,
            0,
            input_registry.canvas_width(),
            input_registry.canvas_height(),
            input_registry.frame_rate(),
        )
        .map_err(|e| {
            crate::Error::new(format!(
                "failed to build program output plan: {}",
                e.message()
            ))
        })?;

        crate::obsws::session::output::start_mixer_processors(&pipeline_handle, &output_plan)
            .await?;

        tracing::info!(
            "program output initialized: video={}, audio={}",
            output_plan.video_track_id,
            output_plan.audio_track_id,
        );

        let scene_uuid = input_registry
            .current_program_scene()
            .map(|s| s.scene_uuid)
            .unwrap_or_default();
        ProgramOutputState {
            scene_uuid,
            video_track_id: output_plan.video_track_id,
            audio_track_id: output_plan.audio_track_id,
            video_mixer_processor_id: output_plan.video_mixer_processor_id,
            audio_mixer_processor_id: output_plan.audio_mixer_processor_id,
            source_processor_ids: output_plan.source_processor_ids,
        }
    };

    // runtime actor を起動する
    // source processor は入力ライフサイクルで管理するため、coordinator 経由で初期起動する
    let (mut actor, coordinator_handle, shutdown_rx) =
        crate::obsws::coordinator::ObswsCoordinator::new(
            input_registry,
            program_output,
            Some(pipeline_handle.clone()),
        );
    actor.start_initial_input_source_processors().await?;
    tokio::task::spawn_local(actor.run());

    let bootstrap_endpoint = Rc::new(
        BootstrapEndpoint::new(pipeline_handle.clone(), coordinator_handle.clone())
            .map_err(|e| e.with_context("Failed to init /bootstrap"))?,
    );

    run_accept_loop(
        listener,
        tls_acceptor,
        upstream_config,
        password,
        coordinator_handle,
        pipeline_handle,
        bootstrap_endpoint,
        shutdown_rx,
    )
    .await
}

/// 受信バイト列に "upgrade:" ヘッダーが含まれるかを case-insensitive で判定する
fn contains_upgrade_header(buf: &[u8]) -> bool {
    let needle = b"\r\nupgrade:";
    let buf_lower: Vec<u8> = buf.iter().map(|b| b.to_ascii_lowercase()).collect();
    buf_lower
        .windows(needle.len())
        .any(|window| window == needle)
}

#[expect(clippy::too_many_arguments)]
async fn run_accept_loop(
    listener: TcpListener,
    tls_acceptor: Option<TlsAcceptor>,
    upstream_config: Option<Arc<UpstreamConfig>>,
    password: Option<String>,
    coordinator_handle: crate::obsws::coordinator::ObswsCoordinatorHandle,
    pipeline_handle: crate::MediaPipelineHandle,
    bootstrap_endpoint: Rc<BootstrapEndpoint>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> crate::Result<()> {
    loop {
        let (stream, peer_addr) = tokio::select! {
            result = listener.accept() => {
                result.map_err(|e| crate::Error::new(format!("failed to accept obsws connection: {e}")))?
            }
            _ = shutdown_rx.changed() => {
                tracing::info!("obsws server shutting down due to coordinator fatal error");
                return Err(crate::Error::new(
                    "obsws server terminated: state file write failed"
                ));
            }
        };
        let tls_acceptor = tls_acceptor.clone();
        let upstream_config = upstream_config.clone();
        let password = password.clone();
        let coordinator_handle = coordinator_handle.clone();
        let pipeline_handle = pipeline_handle.clone();
        let bootstrap_endpoint = bootstrap_endpoint.clone();
        tokio::task::spawn_local(async move {
            // WebSocket はフレーム単位の低遅延配信が必要なため、
            // Nagle アルゴリズムを無効化する。TLS ハンドシェイク前に設定する。
            if let Err(e) = stream.set_nodelay(true) {
                tracing::warn!("failed to set TCP_NODELAY on obsws socket: {e}");
                return;
            }

            // TLS ハンドシェイクを行う
            let stream =
                match ServerTcpOrTlsStream::accept_with_tls(stream, tls_acceptor.as_ref()).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("TLS handshake error from {peer_addr}: {e}");
                        return;
                    }
                };

            if let Err(e) = route_connection(
                stream,
                peer_addr,
                upstream_config,
                password,
                coordinator_handle,
                pipeline_handle,
                bootstrap_endpoint,
            )
            .await
            {
                tracing::warn!(
                    "obsws connection handler failed from {peer_addr}: {}",
                    e.display()
                );
            }
        });
    }
}

/// 接続の最初のデータを読み取り、WebSocket Upgrade か HTTP かをルーティングする
async fn route_connection(
    mut stream: ServerTcpOrTlsStream,
    peer_addr: SocketAddr,
    upstream_config: Option<Arc<UpstreamConfig>>,
    password: Option<String>,
    coordinator_handle: crate::obsws::coordinator::ObswsCoordinatorHandle,
    pipeline_handle: crate::MediaPipelineHandle,
    bootstrap_endpoint: Rc<BootstrapEndpoint>,
) -> crate::Result<()> {
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await.map_err(|e| {
        crate::Error::new(format!(
            "failed to read obsws connection from {peer_addr}: {e}"
        ))
    })?;
    if n == 0 {
        return Ok(());
    }
    buf.truncate(n);

    if contains_upgrade_header(&buf) {
        handle_ws_connection(stream, buf, peer_addr, password, coordinator_handle).await
    } else {
        handle_http_connection(
            stream,
            buf,
            peer_addr,
            upstream_config,
            pipeline_handle,
            bootstrap_endpoint,
        )
        .await
        .map_err(|e| crate::Error::new(format!("obsws http handler error from {peer_addr}: {e}")))
    }
}

async fn handle_ws_connection(
    mut stream: ServerTcpOrTlsStream,
    initial_buf: Vec<u8>,
    peer_addr: SocketAddr,
    password: Option<String>,
    coordinator_handle: crate::obsws::coordinator::ObswsCoordinatorHandle,
) -> crate::Result<()> {
    tracing::debug!("obsws peer connected: {peer_addr}");
    let mut ws = WebSocketServerConnection::new(
        ServerConnectionOptions::new()
            .protocol(OBSWS_SUBPROTOCOL)
            .ping_interval(0),
    );
    // 受信チャンクサイズのみを規定する固定バッファ。
    // メッセージ境界の再構成は shiguredo_websocket 側の内部バッファで処理される。
    let mut buf = [0_u8; 8192];
    let auth = password
        .as_deref()
        .map(ObswsAuthentication::new)
        .transpose()?;
    let mut session = ObswsSession::new(auth, coordinator_handle);

    // route_connection で読み取った最初のデータを先に処理する
    let mut pending_initial: Option<Vec<u8>> = Some(initial_buf);

    loop {
        if let Some(data) = pending_initial.take() {
            if let Err(e) = ws.feed_recv_buf(&data) {
                tracing::warn!("obsws protocol error from {peer_addr}: {e}");
                break;
            }
        } else {
            let n = stream
                .read(&mut buf)
                .await
                .map_err(|e| crate::Error::new(format!("failed to read from obsws socket: {e}")))?;
            if n == 0 {
                break;
            }
            if let Err(e) = ws.feed_recv_buf(&buf[..n]) {
                tracing::warn!("obsws protocol error from {peer_addr}: {e}");
                break;
            }
        }

        if ws.state() == ConnectionState::Connecting
            && let Some(request) = ws.handshake_request()
        {
            if !request.protocols.iter().any(|p| p == OBSWS_SUBPROTOCOL) {
                tracing::warn!(
                    "obsws handshake rejected: missing required subprotocol: {OBSWS_SUBPROTOCOL}"
                );
                ws.reject_handshake(400, "Bad Request", &[]).map_err(|e| {
                    crate::Error::new(format!("failed to reject websocket handshake: {e}"))
                })?;
            } else {
                ws.accept_handshake_auto().map_err(|e| {
                    crate::Error::new(format!("failed to accept websocket handshake: {e}"))
                })?;
            }
        }

        let mut should_terminate = false;
        while let Some(event) = ws.poll_event() {
            match event {
                ConnectionEvent::Connected {
                    protocol,
                    extensions,
                } => {
                    tracing::info!(
                        "obsws websocket connected: peer={peer_addr}, protocol={protocol:?}, extensions={extensions:?}"
                    );
                    let action = session.on_connected();
                    should_terminate = apply_session_action(&mut ws, action, session.stats_mut())?;
                }
                ConnectionEvent::TextMessage(text) => {
                    let action = match session.on_text_message(&text).await {
                        Ok(action) => action,
                        Err(e) => {
                            tracing::warn!("obsws invalid client message: {}", e.display());
                            SessionAction::Close {
                                code: CloseCode::INVALID_PAYLOAD,
                                reason: "invalid message",
                                close_error_context: "failed to close websocket for invalid message",
                            }
                        }
                    };
                    should_terminate = apply_session_action(&mut ws, action, session.stats_mut())?;
                }
                ConnectionEvent::Close { code, reason } => {
                    tracing::debug!(
                        "obsws close received: peer={peer_addr}, code={code:?}, reason={reason}"
                    );
                    should_terminate = apply_session_action(
                        &mut ws,
                        session.on_close_event(),
                        session.stats_mut(),
                    )?;
                }
                ConnectionEvent::Error(reason) => {
                    tracing::warn!("obsws event error from {peer_addr}: {reason}");
                    should_terminate = apply_session_action(
                        &mut ws,
                        session.on_error_event(),
                        session.stats_mut(),
                    )?;
                }
                ConnectionEvent::StateChanged(ConnectionState::Closed) => {
                    should_terminate = apply_session_action(
                        &mut ws,
                        session.on_close_event(),
                        session.stats_mut(),
                    )?;
                }
                _ => {}
            }

            if should_terminate {
                break;
            }
        }

        if !flush_ws_output(&mut ws, &mut stream).await? {
            break;
        }
        if should_terminate {
            break;
        }
    }

    let _ = stream.shutdown().await;
    tracing::debug!("obsws peer disconnected: {peer_addr}");
    Ok(())
}

async fn handle_http_connection(
    stream: ServerTcpOrTlsStream,
    initial_buf: Vec<u8>,
    peer_addr: SocketAddr,
    upstream_config: Option<Arc<UpstreamConfig>>,
    pipeline_handle: crate::MediaPipelineHandle,
    bootstrap_endpoint: Rc<BootstrapEndpoint>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = tokio::io::BufReader::with_capacity(8192, reader);
    let mut writer = BufWriter::with_capacity(65536, writer);
    let mut decoder = RequestDecoder::new();
    let mut buf = [0_u8; 8192];

    // route_connection で読み取った最初のデータを先に処理する
    decoder.feed(&initial_buf)?;

    loop {
        while let Some(request) = decoder.decode()? {
            let keep_alive = request.is_keep_alive();

            // ローカルエンドポイント
            let local_response = match request_path(request.uri.as_str()) {
                "/.ok" => Some(Response::new(204, "No Content")),
                "/bootstrap" => Some(bootstrap_endpoint.handle_request(&request).await),
                "/metrics" => Some(
                    crate::endpoint_http_metrics::handle_request(&request, &pipeline_handle).await,
                ),
                _ => None,
            };

            if let Some(response) = local_response {
                if let Err(e) = write_response(&mut writer, &response).await {
                    if is_client_disconnect(&e) {
                        tracing::warn!("obsws http 499 Client Closed Request from {peer_addr}");
                        return Ok(());
                    }
                    return Err(e.into());
                }
            } else if let Some(upstream) = &upstream_config {
                if request.method == "GET" {
                    if let Err(e) =
                        proxy_to_upstream(&mut writer, &request, upstream, peer_addr).await
                    {
                        tracing::warn!("Reverse proxy error for {peer_addr}: {e}");
                        let error_response = Response::new(502, "Bad Gateway");
                        // 502 送信失敗は無視する（クライアントが切断している可能性がある）
                        let _ = write_response(&mut writer, &error_response).await;
                    }
                } else {
                    let response = Response::new(405, "Method Not Allowed");
                    if let Err(e) = write_response(&mut writer, &response).await {
                        if is_client_disconnect(&e) {
                            tracing::warn!("obsws http 499 Client Closed Request from {peer_addr}");
                            return Ok(());
                        }
                        return Err(e.into());
                    }
                }
            } else {
                let response = Response::new(404, "Not Found");
                if let Err(e) = write_response(&mut writer, &response).await {
                    if is_client_disconnect(&e) {
                        tracing::warn!("obsws http 499 Client Closed Request from {peer_addr}");
                        return Ok(());
                    }
                    return Err(e.into());
                }
            }

            if !keep_alive {
                return Ok(());
            }
        }

        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        decoder.feed(&buf[..n])?;
    }

    Ok(())
}

fn apply_session_action(
    ws: &mut WebSocketServerConnection,
    action: SessionAction,
    session_stats: &mut ObswsSessionStats,
) -> crate::Result<bool> {
    match action {
        SessionAction::SendTexts { messages } => {
            for (text, message_name) in messages {
                send_ws_text(ws, text.text(), session_stats, message_name)?;
            }
            Ok(false)
        }
        SessionAction::SendText { text, message_name } => {
            send_ws_text(ws, text.text(), session_stats, message_name)?;
            Ok(false)
        }
        SessionAction::Close {
            code,
            reason,
            close_error_context,
        } => {
            ws.close(code, reason)
                .map_err(|e| crate::Error::new(format!("{close_error_context}: {e}")))?;
            Ok(true)
        }
        SessionAction::Terminate => Ok(true),
    }
}

fn send_ws_text(
    ws: &mut WebSocketServerConnection,
    text: &str,
    session_stats: &mut ObswsSessionStats,
    message_name: &str,
) -> crate::Result<()> {
    ws.send_text(text)
        .map_err(|e| crate::Error::new(format!("failed to send {message_name}: {e}")))?;
    session_stats.outgoing_messages = session_stats.outgoing_messages.saturating_add(1);
    Ok(())
}

async fn flush_ws_output(
    ws: &mut WebSocketServerConnection,
    stream: &mut ServerTcpOrTlsStream,
) -> crate::Result<bool> {
    while let Some(output) = ws.poll_output() {
        match output {
            ConnectionOutput::SendData(data) => {
                stream.write_all(&data).await.map_err(|e| {
                    crate::Error::new(format!("failed to write to obsws socket: {e}"))
                })?;
            }
            ConnectionOutput::CloseConnection => {
                return Ok(false);
            }
            ConnectionOutput::SetTimer { .. } | ConnectionOutput::ClearTimer { .. } => {
                // タイマー駆動は未実装。現状は ping_interval=0 で運用する。
            }
        }
    }

    Ok(true)
}

/// レスポンスを downstream に書き込む
async fn write_response(
    writer: &mut BufWriter<impl tokio::io::AsyncWrite + Unpin>,
    response: &Response,
) -> io::Result<()> {
    writer.write_all(&response.encode()).await?;
    writer.flush().await?;
    Ok(())
}

fn parse_upstream_config(
    ui_remote_url: Option<&str>,
) -> crate::Result<Option<Arc<UpstreamConfig>>> {
    match ui_remote_url {
        Some(url) => {
            let uri = Uri::parse(url)
                .map_err(|e| crate::Error::new(format!("Failed to parse --ui-remote-url: {e}")))?;
            let is_https = uri.scheme() == Some("https");
            let host = uri
                .host()
                .ok_or_else(|| crate::Error::new("--ui-remote-url has no host".to_string()))?
                .to_string();
            let port = uri.port().unwrap_or(if is_https { 443 } else { 80 });
            let path_prefix = uri.path().to_string();
            tracing::info!("Reverse proxy upstream: {url}");
            Ok(Some(Arc::new(UpstreamConfig {
                host,
                port,
                tls: is_https,
                path_prefix,
            })))
        }
        None => Ok(None),
    }
}

/// upstream にリクエストをプロキシする
async fn proxy_to_upstream(
    downstream: &mut BufWriter<impl tokio::io::AsyncWrite + Unpin>,
    client_request: &Request,
    config: &UpstreamConfig,
    client_addr: std::net::SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // upstream URI を構築する
    let upstream_uri = if config.path_prefix == "/" || config.path_prefix.is_empty() {
        client_request.uri.clone()
    } else {
        let prefix = config.path_prefix.trim_end_matches('/');
        format!("{prefix}{}", client_request.uri)
    };

    // upstream リクエストを構築する
    let mut upstream_request = Request::new("GET", &upstream_uri);
    upstream_request.add_header("Host", &config.host);
    upstream_request.add_header("Connection", "close");

    // クライアントヘッダーを転送する（hop-by-hop と Host を除外）
    for (name, value) in &client_request.headers {
        let name_lower = name.to_ascii_lowercase();
        if name_lower == "host" {
            continue;
        }
        if HOP_BY_HOP_HEADERS.contains(&name_lower.as_str()) {
            continue;
        }
        upstream_request.add_header(name, value);
    }

    // X-Forwarded-For ヘッダーを追加する
    upstream_request.add_header("X-Forwarded-For", &client_addr.ip().to_string());

    // upstream に接続する
    let mut upstream = TcpOrTlsStream::connect(&config.host, config.port, config.tls).await?;

    // upstream にリクエストを送信する
    upstream.write_all(&upstream_request.encode()).await?;
    upstream.flush().await?;

    // upstream レスポンスを受信する
    let mut response_decoder = ResponseDecoder::new();
    let mut buf = vec![0u8; 8192];

    loop {
        let n = upstream.read(&mut buf).await?;
        if n == 0 {
            break;
        }

        response_decoder.feed(&buf[..n])?;

        if let Some(response) = response_decoder.decode()? {
            // レスポンスを downstream に転送する
            if let Err(e) = downstream.write_all(&response.encode()).await {
                if is_client_disconnect(&e) {
                    tracing::warn!("499 Client Closed Request from {client_addr}");
                    return Ok(());
                }
                return Err(e.into());
            }
            if let Err(e) = downstream.flush().await {
                if is_client_disconnect(&e) {
                    tracing::warn!("499 Client Closed Request from {client_addr}");
                    return Ok(());
                }
                return Err(e.into());
            }
            return Ok(());
        }
    }

    // upstream がレスポンスなしで切断した場合
    Err("Upstream closed connection without sending a response".into())
}
