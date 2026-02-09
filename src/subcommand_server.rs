use std::path::PathBuf;

use orfail::OrFail;
use shiguredo_http11::{RequestDecoder, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpListener;

use crate::tcp::{ServerTcpOrTlsStream, create_server_tls_acceptor};

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
        log::info!("{scheme} server listening on {scheme}://{addr}");

        loop {
            let (stream, peer_addr) = listener.accept().await.or_fail()?;
            let tls_acceptor = tls_acceptor.clone();
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
                        log::warn!("TLS handshake error from {peer_addr}: {e}");
                        return;
                    }
                };

                if let Err(e) = handle_connection(stream, peer_addr).await {
                    log::warn!("Client error from {peer_addr}: {e}");
                }
            });
        }
    })
}

async fn handle_connection(
    stream: ServerTcpOrTlsStream,
    peer_addr: std::net::SocketAddr,
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

            let response = match request.uri.as_str() {
                "/.ok" => Response::new(204, "No Content"),
                "/rpc" => Response::new(204, "No Content"),
                "/bootstrap" => Response::new(204, "No Content"),
                _ => Response::new(404, "Not Found"),
            };

            let response_bytes = response.encode();
            writer.write_all(&response_bytes).await?;
            writer.flush().await?;

            if !keep_alive {
                log::debug!("Connection close requested by {peer_addr}");
                return Ok(());
            }
        }
    }

    Ok(())
}
