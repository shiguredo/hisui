use std::net::{IpAddr, SocketAddr};

use shiguredo_websocket::{
    ConnectionEvent, ConnectionOutput, ConnectionState, ServerConnectionOptions,
    WebSocketServerConnection,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

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
            if let Err(e) = handle_connection(stream, peer_addr).await {
                tracing::warn!("obsws connection handler failed: {}", e.display());
            }
        });
    }
}

async fn handle_connection(mut stream: TcpStream, peer_addr: SocketAddr) -> crate::Result<()> {
    tracing::debug!("obsws peer connected: {peer_addr}");
    let mut ws = WebSocketServerConnection::new(ServerConnectionOptions::new().ping_interval(0));
    let mut buf = [0_u8; 8192];

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
            ws.accept_handshake_auto().map_err(|e| {
                crate::Error::new(format!("failed to accept websocket handshake: {e}"))
            })?;
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
