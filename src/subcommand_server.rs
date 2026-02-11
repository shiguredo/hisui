use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use orfail::OrFail;
use shiguredo_http11::uri::Uri;
use shiguredo_http11::{Request, RequestDecoder, Response, ResponseDecoder};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpListener;

use crate::tcp::{ServerTcpOrTlsStream, TcpOrTlsStream, create_server_tls_acceptor};

/// クライアント切断かどうかを判定する
fn is_client_disconnect(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
    )
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

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    // デフォルトポートは 8919 (H=8, I=9, S=19 で "His")
    let http_port: u16 = noargs::opt("http-port")
        .doc("HTTP サーバーのリッスンポート")
        .default("8919")
        .take(&mut args)
        .then(|o| o.value().parse())?;

    let https_cert_path: Option<PathBuf> = noargs::opt("https-cert-path")
        .doc("HTTPS 用の証明書ファイルパス（PEM 形式）")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;

    let https_key_path: Option<PathBuf> = noargs::opt("https-key-path")
        .doc("HTTPS 用の秘密鍵ファイルパス（PEM 形式）")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;

    let ui_remote_url: Option<String> = noargs::opt("ui-remote-url")
        .doc("UI 用リモートサーバーの URL（GET リクエストをリバースプロキシする）")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, std::convert::Infallible>(o.value().to_string()))?;

    // 片方のみ指定はエラー
    match (&https_cert_path, &https_key_path) {
        (Some(_), None) => {
            return Err(noargs::Error::other(
                &args,
                "--https-cert-path requires --https-key-path",
            ));
        }
        (None, Some(_)) => {
            return Err(noargs::Error::other(
                &args,
                "--https-key-path requires --https-cert-path",
            ));
        }
        _ => {}
    }

    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .or_fail()?;

    runtime.block_on(async {
        // upstream 設定をパースする
        let upstream_config = match &ui_remote_url {
            Some(url) => {
                let uri = Uri::parse(url).map_err(|e| {
                    orfail::Failure::new(format!("Failed to parse --ui-remote-url: {e}"))
                })?;
                let is_https = uri.scheme() == Some("https");
                let host = uri
                    .host()
                    .ok_or_else(|| orfail::Failure::new("--ui-remote-url has no host".to_string()))?
                    .to_string();
                let port = uri.port().unwrap_or(if is_https { 443 } else { 80 });
                let path_prefix = uri.path().to_string();
                tracing::info!("Reverse proxy upstream: {url}");
                Some(Arc::new(UpstreamConfig {
                    host,
                    port,
                    tls: is_https,
                    path_prefix,
                }))
            }
            None => None,
        };

        // TLS が指定されている場合は TlsAcceptor を作成する
        let tls_acceptor = match (&https_cert_path, &https_key_path) {
            (Some(cert_path), Some(key_path)) => Some(
                create_server_tls_acceptor(cert_path, key_path)
                    .await
                    .or_fail()?,
            ),
            _ => None,
        };

        let scheme = if tls_acceptor.is_some() {
            "https"
        } else {
            "http"
        };

        let addr = format!("0.0.0.0:{http_port}");
        let listener = TcpListener::bind(&addr).await.or_fail()?;
        tracing::info!("{scheme} server listening on {scheme}://{addr}");

        loop {
            let (stream, peer_addr) = listener.accept().await.or_fail()?;
            let tls_acceptor = tls_acceptor.clone();
            let upstream_config = upstream_config.clone();
            tokio::spawn(async move {
                // TLS ハンドシェイクを行う
                let stream = match ServerTcpOrTlsStream::accept_with_tls(
                    stream,
                    tls_acceptor.as_ref(),
                )
                .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("TLS handshake error from {peer_addr}: {e}");
                        return;
                    }
                };

                if let Err(e) = handle_connection(stream, peer_addr, upstream_config).await {
                    tracing::warn!("Client error from {peer_addr}: {e}");
                }
            });
        }
    })
}

async fn handle_connection(
    stream: ServerTcpOrTlsStream,
    peer_addr: std::net::SocketAddr,
    upstream_config: Option<Arc<UpstreamConfig>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = tokio::io::BufReader::with_capacity(8192, reader);
    let mut writer = BufWriter::with_capacity(65536, writer);

    let mut decoder = RequestDecoder::new();
    let mut buf = [0u8; 8192];

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }

        decoder.feed(&buf[..n])?;

        while let Some(request) = decoder.decode()? {
            let keep_alive = request.is_keep_alive();

            // ローカルエンドポイント
            let local_response = match request.uri.as_str() {
                "/.ok" => Some(Response::new(204, "No Content")),
                "/rpc" => Some(Response::new(204, "No Content")),
                "/bootstrap" => Some(Response::new(204, "No Content")),
                _ => None,
            };

            if let Some(response) = local_response {
                if let Err(e) = write_response(&mut writer, &response).await {
                    if is_client_disconnect(&e) {
                        tracing::warn!("499 Client Closed Request from {peer_addr}");
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
                    let response = Response::new(404, "Not Found");
                    if let Err(e) = write_response(&mut writer, &response).await {
                        if is_client_disconnect(&e) {
                            tracing::warn!("499 Client Closed Request from {peer_addr}");
                            return Ok(());
                        }
                        return Err(e.into());
                    }
                }
            } else {
                let response = Response::new(404, "Not Found");
                if let Err(e) = write_response(&mut writer, &response).await {
                    if is_client_disconnect(&e) {
                        tracing::warn!("499 Client Closed Request from {peer_addr}");
                        return Ok(());
                    }
                    return Err(e.into());
                }
            }

            if !keep_alive {
                tracing::debug!("Connection close requested by {peer_addr}");
                return Ok(());
            }
        }
    }

    Ok(())
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
