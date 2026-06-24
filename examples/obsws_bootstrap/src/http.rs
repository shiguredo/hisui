use shiguredo_http11::{Request, ResponseDecoder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn http_bootstrap(host: &str, port: u16, offer_sdp: &str) -> Result<String, String> {
    tracing::info!("connecting to bootstrap endpoint: host={host}, port={port}");
    let mut stream = tokio::net::TcpStream::connect(format!("{host}:{port}"))
        .await
        .map_err(|e| format!("failed to connect: {e}"))?;

    let mut request =
        Request::new("POST", "/bootstrap").map_err(|e| format!("failed to build request: {e}"))?;
    request
        .add_header("Content-Type", "application/sdp")
        .map_err(|e| format!("failed to add Content-Type: {e}"))?;
    request
        .add_header("Host", format!("{host}:{port}"))
        .map_err(|e| format!("failed to add Host: {e}"))?;
    request
        .add_header("Connection", "close")
        .map_err(|e| format!("failed to add Connection: {e}"))?;
    request.set_body(offer_sdp.as_bytes().to_vec());

    let encoded = request
        .encode()
        .map_err(|e| format!("failed to encode request: {e}"))?;
    stream
        .write_all(&encoded)
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
            if response.status_code() != 201 {
                return Err(format!(
                    "bootstrap failed: {} {}",
                    response.status_code(),
                    response.reason_phrase()
                ));
            }
            let body = response
                .body_bytes()
                .ok_or_else(|| "response has no body".to_owned())?
                .to_vec();
            return String::from_utf8(body)
                .map_err(|e| format!("invalid UTF-8 in answer SDP: {e}"));
        }
    }
}
