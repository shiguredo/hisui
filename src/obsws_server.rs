use std::net::{IpAddr, SocketAddr};

use shiguredo_websocket::{
    CloseCode, ConnectionEvent, ConnectionOutput, ConnectionState, ServerConnectionOptions,
    WebSocketServerConnection,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::obsws_auth::ObswsAuthentication;
use crate::obsws_message_handler::ObswsSessionStats;
use crate::obsws_protocol::OBSWS_SUBPROTOCOL;
use crate::obsws_session::{ObswsSession, SessionAction};

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
    let auth = password
        .as_deref()
        .map(ObswsAuthentication::new)
        .transpose()?;
    let mut session = ObswsSession::new(auth);

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
                    let action = match session.on_text_message(&text) {
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
