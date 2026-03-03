use std::net::{IpAddr, SocketAddr};

use shiguredo_websocket::{
    CloseCode, ConnectionEvent, ConnectionOutput, ConnectionState, ServerConnectionOptions,
    WebSocketServerConnection,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const OBSWS_SUBPROTOCOL: &str = "obswebsocket.json";
const OBSWS_VERSION: &str = "5.0.0";
const OBSWS_RPC_VERSION: u32 = 1;
const OBSWS_OP_HELLO: i64 = 0;
const OBSWS_OP_IDENTIFY: i64 = 1;
const OBSWS_OP_IDENTIFIED: i64 = 2;

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let host: IpAddr = noargs::opt("host")
        .ty("HOST")
        .env("HISUI_OBSWS_HOST")
        .doc("OBS WebSocket のリッスンアドレス")
        .default("127.0.0.1")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let port: u16 = noargs::opt("port")
        .ty("PORT")
        .env("HISUI_OBSWS_PORT")
        .doc("OBS WebSocket のリッスンポート")
        .default("4455")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let password: Option<String> = noargs::opt("password")
        .ty("PASSWORD")
        .env("HISUI_OBSWS_PASSWORD")
        .doc("OBS WebSocket の接続パスワード")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;

    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    run_internal(host, port, password.is_some()).map_err(noargs::Error::from)
}

fn run_internal(host: IpAddr, port: u16, has_password: bool) -> crate::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(crate::Error::from)?;

    runtime.block_on(async move { run_server(host, port, has_password).await })
}

async fn run_server(host: IpAddr, port: u16, has_password: bool) -> crate::Result<()> {
    if has_password {
        tracing::warn!("obsws password is set but authentication is not implemented yet");
    }

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
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer_addr, has_password).await {
                tracing::warn!("obsws connection handler failed: {}", e.display());
            }
        });
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    has_password: bool,
) -> crate::Result<()> {
    tracing::debug!("obsws peer connected: {peer_addr}");
    let mut ws = WebSocketServerConnection::new(
        ServerConnectionOptions::new()
            .protocol(OBSWS_SUBPROTOCOL)
            .ping_interval(0),
    );
    let mut buf = [0_u8; 8192];
    let mut identified = false;

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
                    if has_password {
                        tracing::warn!(
                            "obsws authentication is not implemented, closing authenticated connection"
                        );
                        ws.close(
                            CloseCode::POLICY_VIOLATION,
                            "authentication is not implemented",
                        )
                        .map_err(|e| {
                            crate::Error::new(format!(
                                "failed to close websocket for unimplemented auth: {e}"
                            ))
                        })?;
                        should_close = true;
                    } else {
                        ws.send_text(&build_hello_message()).map_err(|e| {
                            crate::Error::new(format!("failed to send hello message: {e}"))
                        })?;
                    }
                }
                ConnectionEvent::TextMessage(text) => {
                    if identified {
                        tracing::warn!("obsws received unsupported message after identify");
                        ws.close(CloseCode::UNSUPPORTED_DATA, "unsupported message")
                            .map_err(|e| {
                                crate::Error::new(format!(
                                    "failed to close websocket for unsupported message: {e}"
                                ))
                            })?;
                        should_close = true;
                        continue;
                    }

                    match parse_client_message(&text) {
                        Ok(ClientMessage::Identify) => {
                            ws.send_text(&build_identified_message()).map_err(|e| {
                                crate::Error::new(format!("failed to send identified message: {e}"))
                            })?;
                            identified = true;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientMessage {
    Identify,
}

fn parse_client_message(text: &str) -> crate::Result<ClientMessage> {
    let json = nojson::RawJson::parse(text)
        .map_err(|e| crate::Error::new(format!("invalid JSON: {e}")))?;
    let value = json.value();
    let op_value = value
        .to_member("op")
        .map_err(|e| crate::Error::new(format!("invalid message: {e}")))?
        .required()
        .map_err(|e| crate::Error::new(format!("invalid message: {e}")))?;
    let op: i64 = op_value
        .try_into()
        .map_err(|e| crate::Error::new(format!("invalid message: {e}")))?;

    if op != OBSWS_OP_IDENTIFY {
        return Err(crate::Error::new(format!(
            "unsupported message opcode: {op}"
        )));
    }

    let d_value = value
        .to_member("d")
        .map_err(|e| crate::Error::new(format!("invalid identify payload: {e}")))?
        .required()
        .map_err(|e| crate::Error::new(format!("invalid identify payload: {e}")))?;
    if d_value.kind() != nojson::JsonValueKind::Object {
        return Err(crate::Error::new(
            "invalid identify payload: d must be an object",
        ));
    }

    Ok(ClientMessage::Identify)
}

fn build_hello_message() -> String {
    nojson::json(|f| {
        f.object(|f| {
            f.member("op", OBSWS_OP_HELLO)?;
            f.member(
                "d",
                nojson::json(|f| {
                    f.object(|f| {
                        f.member("obsWebSocketVersion", OBSWS_VERSION)?;
                        f.member("rpcVersion", OBSWS_RPC_VERSION)
                    })
                }),
            )
        })
    })
    .to_string()
}

fn build_identified_message() -> String {
    nojson::json(|f| {
        f.object(|f| {
            f.member("op", OBSWS_OP_IDENTIFIED)?;
            f.member(
                "d",
                nojson::json(|f| f.object(|f| f.member("negotiatedRpcVersion", OBSWS_RPC_VERSION))),
            )
        })
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_hello_message_contains_expected_fields() {
        let message = build_hello_message();
        let json = nojson::RawJson::parse(&message).expect("hello message must be valid JSON");
        let op_value = json
            .value()
            .to_member("op")
            .expect("op member access must succeed")
            .required()
            .expect("op must exist");
        let op: i64 = op_value.try_into().expect("op must be i64");
        assert_eq!(op, OBSWS_OP_HELLO);
    }

    #[test]
    fn parse_client_message_accepts_identify() {
        let message = r#"{"op":1,"d":{"rpcVersion":1}}"#;
        let parsed = parse_client_message(message).expect("identify message must be accepted");
        assert_eq!(parsed, ClientMessage::Identify);
    }

    #[test]
    fn parse_client_message_rejects_non_identify() {
        let message = r#"{"op":9,"d":{}}"#;
        let error = parse_client_message(message).expect_err("non identify must be rejected");
        assert!(error.display().contains("unsupported message opcode"));
    }
}
