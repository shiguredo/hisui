use orfail::OrFail;
use shiguredo_http11::{RequestDecoder, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpListener;

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    // デフォルトポートは 8919 (H=8, I=9, S=19 で "His")
    let http_port: u16 = noargs::opt("http-port")
        .doc("HTTP サーバーのリッスンポート")
        .default("8919")
        .take(&mut args)
        .then(|o| o.value().parse())?;

    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .or_fail()?;

    runtime.block_on(async {
        let addr = format!("0.0.0.0:{http_port}");
        let listener = TcpListener::bind(&addr).await.or_fail()?;
        log::info!("HTTP server listening on http://{addr}");

        loop {
            let (stream, peer_addr) = listener.accept().await.or_fail()?;
            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, peer_addr).await {
                    log::warn!("Client error from {peer_addr}: {e}");
                }
            });
        }
    })
}

async fn handle_client(
    stream: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, writer) = stream.into_split();
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
                "/rpc" | "/bootstrap" | "/.ok" => Response::new(204, "No Content"),
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
