//! HTTP Bootstrap 用の最小限の HTTP/1.1 クライアント。
//!
//! 対象は hisui server の `/bootstrap` エンドポイントのみで、
//! レスポンスは必ず `Connection: close` + `Content-Length` 付きの
//! プレーンなテキストボディ (SDP) が返るため、汎用クライアントは使わない。
//! hisui server 以外へのリクエストは想定していない。

use std::net::IpAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::Error;

/// Bootstrap URL のパース結果。
struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
}

/// `http://host:port/path` 形式の URL をパースする。
fn parse_url(url: &str) -> crate::Result<ParsedUrl> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| Error::new(format!("unsupported URL scheme: {url}")))?;
    let (host_port, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) => {
            let port = port
                .parse::<u16>()
                .map_err(|_| Error::new(format!("invalid port in URL: {url}")))?;
            (host, port)
        }
        None => (host_port, 80),
    };
    if host.is_empty() {
        return Err(Error::new(format!("missing host in URL: {url}")));
    }
    Ok(ParsedUrl {
        host: host.to_owned(),
        port,
        path: path.to_owned(),
    })
}

/// HTTP レスポンスのパース結果。
struct ParsedResponse {
    status: u16,
    body: Vec<u8>,
}

/// HTTP レスポンスをパースする。`Connection: close` + `Content-Length` のみ対応。
fn parse_response(bytes: &[u8]) -> crate::Result<ParsedResponse> {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| Error::new("invalid HTTP response: missing header terminator"))?;
    let header_text = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| Error::new("invalid HTTP response: header is not UTF-8"))?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| Error::new("invalid HTTP response: missing status line"))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| Error::new(format!("invalid status line: {status_line}")))?;

    let mut content_length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("Content-Length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| Error::new("invalid Content-Length header"))?,
            );
        }
    }
    let body_start = header_end + 4;
    let body = match content_length {
        Some(length) => bytes[body_start..body_start + length].to_vec(),
        None => bytes[body_start..].to_vec(),
    };
    Ok(ParsedResponse { status, body })
}

/// `POST http://host:port/path` を実行する。
/// ボディは `Content-Type: application/sdp` で送信し、レスポンスボディを返す。
pub async fn post_sdp(url: &str, body: &str) -> crate::Result<(u16, String)> {
    let parsed = parse_url(url)?;

    // IPv6 アドレスはブラケット形式で来るため、そのままホストとして扱う
    let address = if parsed.host.starts_with('[') && parsed.host.ends_with(']') {
        let host = &parsed.host[1..parsed.host.len() - 1];
        let ip = host
            .parse::<IpAddr>()
            .map_err(|_| Error::new(format!("invalid host: {}", parsed.host)))?;
        tokio::net::lookup_host((ip, parsed.port))
            .await
            .map_err(|e| Error::new(format!("failed to resolve host: {e}")))?
            .next()
            .ok_or_else(|| Error::new(format!("failed to resolve host: {}", parsed.host)))?
    } else {
        tokio::net::lookup_host((parsed.host.as_str(), parsed.port))
            .await
            .map_err(|e| Error::new(format!("failed to resolve host: {e}")))?
            .next()
            .ok_or_else(|| Error::new(format!("failed to resolve host: {}", parsed.host)))?
    };

    let mut stream = TcpStream::connect(address)
        .await
        .map_err(|e| Error::new(format!("failed to connect to {url}: {e}")))?;

    let request = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: {length}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        path = parsed.path,
        host = parsed.host,
        port = parsed.port,
        length = body.len(),
        body = body,
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| Error::new(format!("failed to send bootstrap request: {e}")))?;

    let mut response_bytes = Vec::new();
    stream
        .read_to_end(&mut response_bytes)
        .await
        .map_err(|e| Error::new(format!("failed to read bootstrap response: {e}")))?;

    let response = parse_response(&response_bytes)?;
    let response_body = String::from_utf8(response.body)
        .map_err(|_| Error::new("bootstrap response body is not UTF-8"))?;
    Ok((response.status, response_body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_url_parses_host_port_and_path() {
        let parsed = parse_url("http://127.0.0.1:4455/bootstrap").expect("パースに失敗しないこと");
        assert_eq!(parsed.host, "127.0.0.1");
        assert_eq!(parsed.port, 4455);
        assert_eq!(parsed.path, "/bootstrap");
    }

    #[test]
    fn parse_url_defaults_port_to_80() {
        let parsed = parse_url("http://example.com/").expect("パースに失敗しないこと");
        assert_eq!(parsed.host, "example.com");
        assert_eq!(parsed.port, 80);
        assert_eq!(parsed.path, "/");
    }

    #[test]
    fn parse_url_rejects_non_http_scheme() {
        let result = parse_url("https://example.com/");
        assert!(result.is_err(), "https はエラーになること");
    }

    #[test]
    fn parse_response_extracts_status_and_body() {
        let bytes = b"HTTP/1.1 201 Created\r\nContent-Type: application/sdp\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
        let response = parse_response(bytes).expect("パースに失敗しないこと");
        assert_eq!(response.status, 201);
        assert_eq!(response.body, b"hello");
    }

    #[test]
    fn parse_response_handles_body_without_content_length() {
        let bytes = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nbody";
        let response = parse_response(bytes).expect("パースに失敗しないこと");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"body");
    }
}
