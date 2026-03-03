use std::net::{IpAddr, SocketAddr};

use shiguredo_websocket::{
    CloseCode, ConnectionEvent, ConnectionOutput, ConnectionState, ServerConnectionOptions,
    WebSocketServerConnection,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_message_handler::{
    ClientMessage, ObswsSessionStats, build_hello_message, build_identified_message,
    handle_request_message, is_supported_rpc_version, parse_client_message,
};
use crate::obsws_protocol::{
    OBSWS_CLOSE_ALREADY_IDENTIFIED, OBSWS_CLOSE_AUTHENTICATION_FAILED, OBSWS_CLOSE_NOT_IDENTIFIED,
    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION, OBSWS_SUBPROTOCOL,
};

pub(crate) fn run_internal(host: IpAddr, port: u16, password: Option<String>) -> crate::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(crate::Error::from)?;

    runtime.block_on(async move { run_server(host, port, password).await })
}

async fn run_server(host: IpAddr, port: u16, password: Option<String>) -> crate::Result<()> {
    let listen_addr = SocketAddr::new(host, port);
    let listener = TcpListener::bind(listen_addr)
        .await
        .map_err(|e| crate::Error::new(format!("failed to bind obsws listener: {e}")))?;
    tracing::info!("obsws server listening on ws://{listen_addr}");

    loop {
        let (stream, peer_addr) = listener
            .accept()
            .await
            .map_err(|e| crate::Error::new(format!("failed to accept obsws connection: {e}")))?;
        let password = password.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer_addr, password).await {
                tracing::warn!("obsws connection handler failed: {}", e.display());
            }
        });
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    password: Option<String>,
) -> crate::Result<()> {
    tracing::debug!("obsws peer connected: {peer_addr}");
    let mut ws = WebSocketServerConnection::new(
        ServerConnectionOptions::new()
            .protocol(OBSWS_SUBPROTOCOL)
            .ping_interval(0),
    );
    let mut buf = [0_u8; 8192];
    let mut identified = false;
    let auth = password
        .as_deref()
        .map(ObswsAuthentication::new)
        .transpose()?;
    let mut session_stats = ObswsSessionStats::default();

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

        if ws.state() == ConnectionState::Connecting && ws.handshake_request().is_some() {
            let request = ws
                .handshake_request()
                .expect("BUG: handshake request must exist");
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

        let mut should_close = false;
        while let Some(event) = ws.poll_event() {
            match event {
                ConnectionEvent::Connected {
                    protocol,
                    extensions,
                } => {
                    tracing::info!(
                        "obsws websocket connected: peer={peer_addr}, protocol={protocol:?}, extensions={extensions:?}"
                    );
                    send_ws_text(
                        &mut ws,
                        &build_hello_message(auth.as_ref()),
                        &mut session_stats,
                        "hello message",
                    )?;
                }
                ConnectionEvent::TextMessage(text) => {
                    session_stats.incoming_messages =
                        session_stats.incoming_messages.saturating_add(1);
                    // ws.close() は即時切断ではなく close frame を送るための要求なので、
                    // ここで break せず should_close を立てて continue し、
                    // このループ内の残イベント処理と外側での flush を行ってから抜ける。

                    match parse_client_message(&text) {
                        Ok(ClientMessage::Identify(identify)) => {
                            if identified {
                                ws.close(OBSWS_CLOSE_ALREADY_IDENTIFIED, "already identified")
                                    .map_err(|e| {
                                        crate::Error::new(format!(
                                            "failed to close websocket for duplicated identify: {e}"
                                        ))
                                    })?;
                                should_close = true;
                                continue;
                            }
                            if !is_supported_rpc_version(identify.rpc_version) {
                                ws.close(
                                    OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION,
                                    "unsupported rpc version",
                                )
                                .map_err(|e| {
                                    crate::Error::new(format!(
                                        "failed to close websocket for unsupported rpc version: {e}"
                                    ))
                                })?;
                                should_close = true;
                                continue;
                            }
                            if let Some(auth) = auth.as_ref()
                                && !auth.is_valid_response(identify.authentication.as_deref())
                            {
                                ws.close(
                                    OBSWS_CLOSE_AUTHENTICATION_FAILED,
                                    "authentication failed",
                                )
                                .map_err(|e| {
                                    crate::Error::new(format!(
                                        "failed to close websocket for authentication failure: {e}"
                                    ))
                                })?;
                                should_close = true;
                                continue;
                            }
                            send_ws_text(
                                &mut ws,
                                &build_identified_message(identify.rpc_version),
                                &mut session_stats,
                                "identified message",
                            )?;
                            identified = true;
                        }
                        Ok(ClientMessage::Request(request)) => {
                            if !identified {
                                ws.close(OBSWS_CLOSE_NOT_IDENTIFIED, "identify is required")
                                    .map_err(|e| {
                                        crate::Error::new(format!(
                                            "failed to close websocket for unidentified request: {e}"
                                        ))
                                    })?;
                                should_close = true;
                                continue;
                            }

                            let response = handle_request_message(request, &session_stats);
                            send_ws_text(
                                &mut ws,
                                &response.message,
                                &mut session_stats,
                                "request response message",
                            )?;
                        }
                        Err(e) => {
                            tracing::warn!("obsws invalid client message: {}", e.display());
                            ws.close(CloseCode::INVALID_PAYLOAD, "invalid message")
                                .map_err(|close_err| {
                                    crate::Error::new(format!(
                                        "failed to close websocket for invalid message: {close_err}"
                                    ))
                                })?;
                            should_close = true;
                        }
                    }
                }
                ConnectionEvent::Close { code, reason } => {
                    tracing::debug!(
                        "obsws close received: peer={peer_addr}, code={code:?}, reason={reason}"
                    );
                    should_close = true;
                }
                ConnectionEvent::Error(reason) => {
                    tracing::warn!("obsws event error from {peer_addr}: {reason}");
                    should_close = true;
                }
                ConnectionEvent::StateChanged(ConnectionState::Closed) => {
                    should_close = true;
                }
                _ => {}
            }
        }

        if !flush_connection_output(&mut ws, &mut stream).await? {
            break;
        }
        if should_close {
            break;
        }
    }

    let _ = stream.shutdown().await;
    tracing::debug!("obsws peer disconnected: {peer_addr}");
    Ok(())
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
