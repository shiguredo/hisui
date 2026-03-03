use std::net::{IpAddr, SocketAddr};

use base64::Engine as _;
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
const OBSWS_CLOSE_AUTHENTICATION_FAILED: CloseCode = CloseCode(4009);
const AUTH_RANDOM_BYTE_LEN: usize = 32;

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

    run_internal(host, port, password).map_err(noargs::Error::from)
}

fn run_internal(host: IpAddr, port: u16, password: Option<String>) -> crate::Result<()> {
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
                    ws.send_text(&build_hello_message(auth.as_ref()))
                        .map_err(|e| {
                            crate::Error::new(format!("failed to send hello message: {e}"))
                        })?;
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
                        Ok(ClientMessage::Identify(identify)) => {
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientMessage {
    Identify(IdentifyMessage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IdentifyMessage {
    authentication: Option<String>,
}

#[derive(Debug, Clone)]
struct ObswsAuthentication {
    salt: String,
    challenge: String,
    expected_response: String,
}

impl ObswsAuthentication {
    fn new(password: &str) -> crate::Result<Self> {
        let salt = generate_random_base64(AUTH_RANDOM_BYTE_LEN)?;
        let challenge = generate_random_base64(AUTH_RANDOM_BYTE_LEN)?;
        let expected_response = build_authentication_response(password, &salt, &challenge);
        Ok(Self {
            salt,
            challenge,
            expected_response,
        })
    }

    fn is_valid_response(&self, response: Option<&str>) -> bool {
        let Some(response) = response else {
            return false;
        };
        aws_lc_rs::constant_time::verify_slices_are_equal(
            response.as_bytes(),
            self.expected_response.as_bytes(),
        )
        .is_ok()
    }
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

    let authentication: Option<String> = d_value
        .to_member("authentication")
        .map_err(|e| crate::Error::new(format!("invalid identify payload: {e}")))?
        .try_into()
        .map_err(|e| crate::Error::new(format!("invalid identify payload: {e}")))?;

    Ok(ClientMessage::Identify(IdentifyMessage { authentication }))
}

fn generate_random_base64(len: usize) -> crate::Result<String> {
    let mut bytes = vec![0_u8; len];
    aws_lc_rs::rand::fill(&mut bytes)
        .map_err(|_| crate::Error::new("failed to generate random bytes"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn build_authentication_response(password: &str, salt: &str, challenge: &str) -> String {
    let secret_hash = aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA256,
        format!("{password}{salt}").as_bytes(),
    );
    let secret = base64::engine::general_purpose::STANDARD.encode(secret_hash.as_ref());
    let response_hash = aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA256,
        format!("{secret}{challenge}").as_bytes(),
    );
    base64::engine::general_purpose::STANDARD.encode(response_hash.as_ref())
}

fn build_hello_message(authentication: Option<&ObswsAuthentication>) -> String {
    nojson::json(|f| {
        f.object(|f| {
            f.member("op", OBSWS_OP_HELLO)?;
            f.member(
                "d",
                nojson::json(|f| {
                    f.object(|f| {
                        f.member("obsWebSocketVersion", OBSWS_VERSION)?;
                        f.member("rpcVersion", OBSWS_RPC_VERSION)?;
                        if let Some(authentication) = authentication {
                            f.member(
                                "authentication",
                                nojson::json(|f| {
                                    f.object(|f| {
                                        f.member("challenge", &authentication.challenge)?;
                                        f.member("salt", &authentication.salt)
                                    })
                                }),
                            )?;
                        }
                        Ok(())
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
        let message = build_hello_message(None);
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
        assert_eq!(
            parsed,
            ClientMessage::Identify(IdentifyMessage {
                authentication: None,
            })
        );
    }

    #[test]
    fn parse_client_message_accepts_identify_with_authentication() {
        let message = r#"{"op":1,"d":{"rpcVersion":1,"authentication":"test-auth"}}"#;
        let parsed = parse_client_message(message).expect("identify message must be accepted");
        assert_eq!(
            parsed,
            ClientMessage::Identify(IdentifyMessage {
                authentication: Some("test-auth".to_owned()),
            })
        );
    }

    #[test]
    fn parse_client_message_rejects_non_identify() {
        let message = r#"{"op":9,"d":{}}"#;
        let error = parse_client_message(message).expect_err("non identify must be rejected");
        assert!(error.display().contains("unsupported message opcode"));
    }

    #[test]
    fn build_authentication_response_matches_expected_value() {
        let response = build_authentication_response("test-password", "c2FsdA==", "Y2hhbGxlbmdl");
        assert_eq!(response, "692yhXm+ZMl25QzSnVANJIg265Xtpfqja0A08Opeiv8=");
    }

    #[test]
    fn build_hello_message_contains_authentication() {
        let auth = ObswsAuthentication {
            salt: "test-salt".to_owned(),
            challenge: "test-challenge".to_owned(),
            expected_response: "unused".to_owned(),
        };
        let message = build_hello_message(Some(&auth));
        let json = nojson::RawJson::parse(&message).expect("hello message must be valid JSON");
        let d_value = json
            .value()
            .to_member("d")
            .expect("d member access must succeed")
            .required()
            .expect("d must exist");
        let authentication = d_value
            .to_member("authentication")
            .expect("authentication member access must succeed")
            .required()
            .expect("authentication must exist");
        let challenge: String = authentication
            .to_member("challenge")
            .expect("challenge member access must succeed")
            .required()
            .expect("challenge must exist")
            .try_into()
            .expect("challenge must be string");
        let salt: String = authentication
            .to_member("salt")
            .expect("salt member access must succeed")
            .required()
            .expect("salt must exist")
            .try_into()
            .expect("salt must be string");
        assert_eq!(challenge, "test-challenge");
        assert_eq!(salt, "test-salt");
    }
}
