use shiguredo_http11::{Request, ResponseDecoder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn http_bootstrap(host: &str, port: u16, offer_sdp: &str) -> Result<String, String> {
    tracing::info!("connecting to bootstrap endpoint: host={host}, port={port}");
    let mut stream = tokio::net::TcpStream::connect(format!("{host}:{port}"))
        .await
        .map_err(|e| format!("failed to connect: {e}"))?;

    let mut request = Request::new("POST", "/bootstrap");
    request.add_header("Content-Type", "application/sdp");
    request.add_header("Host", &format!("{host}:{port}"));
    request.add_header("Connection", "close");
    request.body = offer_sdp.as_bytes().to_vec();

    stream
        .write_all(&request.encode())
        .await
        .map_err(|e| format!("failed to send request: {e}"))?;
    stream
        .flush()
        .await
        .map_err(|e| format!("failed to flush: {e}"))?;
    let mut decoder = ResponseDecoder::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("failed to read response: {e}"))?;
        if n == 0 {
            return Err("server closed connection without response".to_owned());
        }
        decoder
            .feed(&buf[..n])
            .map_err(|e| format!("failed to decode response: {e}"))?;
        if let Some(response) = decoder
            .decode()
            .map_err(|e| format!("failed to decode response: {e}"))?
        {
            if response.status_code != 201 {
                return Err(format!(
                    "bootstrap failed: {} {}",
                    response.status_code, response.reason_phrase
                ));
            }
            return String::from_utf8(response.body)
                .map_err(|e| format!("invalid UTF-8 in answer SDP: {e}"));
        }
    }
}
