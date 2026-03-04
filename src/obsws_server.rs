use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use shiguredo_http11::{RequestDecoder, Response};
use shiguredo_websocket::{
    CloseCode, ConnectionEvent, ConnectionOutput, ConnectionState, ServerConnectionOptions,
    WebSocketServerConnection,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_input_registry::ObswsInputRegistry;
use crate::obsws_message::ObswsSessionStats;
use crate::obsws_protocol::OBSWS_SUBPROTOCOL;
use crate::obsws_session::{ObswsSession, SessionAction};

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

pub async fn run_server(
    ws_host: IpAddr,
    ws_port: u16,
    http_host: IpAddr,
    http_port: u16,
    password: Option<String>,
    pipeline_config: crate::MediaPipelineConfig,
) -> crate::Result<()> {
    let ws_listen_addr = SocketAddr::new(ws_host, ws_port);
    let ws_listener = TcpListener::bind(ws_listen_addr)
        .await
        .map_err(|e| crate::Error::new(format!("failed to bind obsws websocket listener: {e}")))?;
    tracing::info!("obsws server listening on ws://{ws_listen_addr}");

    let http_listen_addr = SocketAddr::new(http_host, http_port);
    let http_listener = TcpListener::bind(http_listen_addr)
        .await
        .map_err(|e| crate::Error::new(format!("failed to bind obsws http listener: {e}")))?;
    tracing::info!("obsws http server listening on http://{http_listen_addr}");
    let input_registry = Arc::new(RwLock::new(ObswsInputRegistry::new()));

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

    let ws_task = tokio::spawn(run_ws_accept_loop(
        ws_listener,
        password,
        input_registry,
        pipeline_handle.clone(),
    ));
    let http_task = tokio::spawn(run_http_accept_loop(http_listener, pipeline_handle));

    tokio::select! {
        ws_result = ws_task => {
            ws_result
                .map_err(|e| crate::Error::new(format!("obsws websocket accept loop task failed: {e}")))?
        }
        http_result = http_task => {
            http_result
                .map_err(|e| crate::Error::new(format!("obsws http accept loop task failed: {e}")))?
        }
    }
}

async fn run_ws_accept_loop(
    listener: TcpListener,
    password: Option<String>,
    input_registry: Arc<RwLock<ObswsInputRegistry>>,
    pipeline_handle: crate::MediaPipelineHandle,
) -> crate::Result<()> {
    loop {
        let (stream, peer_addr) = listener
            .accept()
            .await
            .map_err(|e| crate::Error::new(format!("failed to accept obsws connection: {e}")))?;
        let password = password.clone();
        let input_registry = input_registry.clone();
        let pipeline_handle = pipeline_handle.clone();
        tokio::spawn(async move {
            if let Err(e) =
                handle_ws_connection(stream, peer_addr, password, input_registry, pipeline_handle)
                    .await
            {
                tracing::warn!("obsws connection handler failed: {}", e.display());
            }
        });
    }
}

async fn run_http_accept_loop(
    listener: TcpListener,
    pipeline_handle: crate::MediaPipelineHandle,
) -> crate::Result<()> {
    loop {
        let (stream, peer_addr) = listener.accept().await.map_err(|e| {
            crate::Error::new(format!("failed to accept obsws http connection: {e}"))
        })?;
        let pipeline_handle = pipeline_handle.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_http_connection(stream, peer_addr, pipeline_handle).await {
                tracing::warn!("obsws http connection handler failed from {peer_addr}: {e}");
            }
        });
    }
}

async fn handle_ws_connection(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    password: Option<String>,
    input_registry: Arc<RwLock<ObswsInputRegistry>>,
    pipeline_handle: crate::MediaPipelineHandle,
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
    let mut session = ObswsSession::new(auth, input_registry, Some(pipeline_handle));

    loop {
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

        if ws.state() == ConnectionState::Connecting
            && let Some(request) = ws.handshake_request()
        {
            if !request.protocols.iter().any(|p| p == OBSWS_SUBPROTOCOL) {
                tracing::warn!(
                    "obsws handshake rejected: missing required subprotocol: {OBSWS_SUBPROTOCOL}"
                );
                ws.reject_handshake(400, "Bad Request").map_err(|e| {
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

        if !flush_connection_output(&mut ws, &mut stream).await? {
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
    stream: TcpStream,
    peer_addr: SocketAddr,
    pipeline_handle: crate::MediaPipelineHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = tokio::io::BufReader::with_capacity(8192, reader);
    let mut writer = BufWriter::with_capacity(65536, writer);
    let mut decoder = RequestDecoder::new();
    let mut buf = [0_u8; 8192];

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        decoder.feed(&buf[..n])?;

        while let Some(request) = decoder.decode()? {
            let keep_alive = request.is_keep_alive();
            let response = match request_path(request.uri.as_str()) {
                "/.ok" => Response::new(204, "No Content"),
                "/metrics" => {
                    crate::endpoint_http_metrics::handle_request(&request, &pipeline_handle).await
                }
                _ => Response::new(404, "Not Found"),
            };

            if let Err(e) = write_response(&mut writer, &response).await {
                if is_client_disconnect(&e) {
                    tracing::warn!("obsws http 499 Client Closed Request from {peer_addr}");
                    return Ok(());
                }
                return Err(e.into());
            }

            if !keep_alive {
                return Ok(());
            }
        }
    }

    Ok(())
}

fn apply_session_action(
    ws: &mut WebSocketServerConnection,
    action: SessionAction,
    session_stats: &mut ObswsSessionStats,
) -> crate::Result<bool> {
    match action {
        SessionAction::SendText { text, message_name } => {
            send_ws_text(ws, &text, session_stats, message_name)?;
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

async fn flush_connection_output(
    ws: &mut WebSocketServerConnection,
    stream: &mut TcpStream,
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
