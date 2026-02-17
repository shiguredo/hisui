use shiguredo_http11::{Request, Response, ResponseDecoder, uri::Uri};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::Error;

pub(crate) struct ParsedLinkHeader {
    pub(crate) urls: Vec<String>,
    pub(crate) username: Option<String>,
    pub(crate) credential: Option<String>,
}

pub(crate) fn parse_link_header(header: &str) -> ParsedLinkHeader {
    let mut urls = Vec::new();
    let mut username = None;
    let mut credential = None;

    for part in header.split(',') {
        let part = part.trim();
        if let Some(start) = part.find('<')
            && let Some(end) = part[start + 1..].find('>')
        {
            urls.push(part[start + 1..start + 1 + end].to_owned());
        }

        if username.is_none() {
            username = extract_quoted_param(part, "username");
        }
        if credential.is_none() {
            credential = extract_quoted_param(part, "credential");
        }
    }

    ParsedLinkHeader {
        urls,
        username,
        credential,
    }
}

fn extract_quoted_param(text: &str, key: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let pattern = format!("{key}=\"");
    let pos = lower.find(&pattern)?;
    let start = pos + pattern.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

pub(crate) fn resolve_resource_url(base_url: &str, location: &str) -> crate::Result<String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        Uri::parse(location).map_err(|e| Error::new(format!("invalid resource URL: {e}")))?;
        return Ok(location.to_owned());
    }

    let base = Uri::parse(base_url).map_err(|e| Error::new(format!("invalid base URL: {e}")))?;
    let scheme = base
        .scheme()
        .ok_or_else(|| Error::new("base URL must contain URL scheme"))?;
    let host = base
        .host()
        .ok_or_else(|| Error::new("base URL must contain host"))?;
    let default_port = if scheme == "https" { 443 } else { 80 };
    let port = base.port().unwrap_or(default_port);
    let authority = if (scheme == "http" && port != 80) || (scheme == "https" && port != 443) {
        format!("{host}:{port}")
    } else {
        host.to_owned()
    };

    let path_and_query = if location.starts_with('/') {
        location.to_owned()
    } else {
        let mut base_path = base.path().to_owned();
        if base_path.is_empty() {
            base_path = "/".to_owned();
        }
        let parent_end = base_path.rfind('/').unwrap_or(0);
        let parent = &base_path[..=parent_end];
        format!("{parent}{location}")
    };
    Uri::parse(&format!("{scheme}://{authority}{path_and_query}"))
        .map_err(|e| Error::new(format!("invalid resolved resource URL: {e}")))?;
    Ok(format!("{scheme}://{authority}{path_and_query}"))
}

pub(crate) struct RequestTarget {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) path_and_query: String,
    pub(crate) host_header: String,
    pub(crate) tls: bool,
}

pub(crate) fn build_request_target(
    url: &str,
    invalid_url_label: &str,
    url_field_label: &str,
) -> crate::Result<RequestTarget> {
    let uri =
        Uri::parse(url).map_err(|e| Error::new(format!("invalid {invalid_url_label}: {e}")))?;

    let scheme = uri
        .scheme()
        .ok_or_else(|| Error::new(format!("{url_field_label} must contain URL scheme")))?;
    let tls = match scheme {
        "http" => false,
        "https" => true,
        _ => {
            return Err(Error::new(format!(
                "{url_field_label} scheme must be http or https"
            )));
        }
    };

    let host = uri
        .host()
        .ok_or_else(|| Error::new(format!("{url_field_label} must contain host")))?
        .to_owned();
    let default_port = if tls { 443 } else { 80 };
    let port = uri.port().unwrap_or(default_port);

    let mut path_and_query = uri.path().to_owned();
    if path_and_query.is_empty() {
        path_and_query = "/".to_owned();
    }
    if let Some(query) = uri.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }

    let host_header = if (!tls && port != 80) || (tls && port != 443) {
        format!("{host}:{port}")
    } else {
        host.clone()
    };

    Ok(RequestTarget {
        host,
        port,
        path_and_query,
        host_header,
        tls,
    })
}

pub(crate) async fn read_http_response<T>(
    stream: &mut T,
    response_label: &str,
) -> crate::Result<Response>
where
    T: AsyncRead + Unpin,
{
    let mut decoder = ResponseDecoder::new();
    let mut buf = [0u8; 4096];

    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| Error::new(format!("failed to read {response_label} response: {e}")))?;
        if n == 0 {
            return Err(Error::new(format!(
                "connection closed before a complete {response_label} response was received",
            )));
        }

        decoder
            .feed(&buf[..n])
            .map_err(|e| Error::new(format!("failed to decode {response_label} response: {e}")))?;
        if let Some(response) = decoder
            .decode()
            .map_err(|e| Error::new(format!("failed to decode {response_label} response: {e}")))?
        {
            return Ok(response);
        }
    }
}

pub(crate) async fn send_delete_resource(
    resource_url: &str,
    bearer_token: Option<&str>,
    user_agent: &str,
    invalid_url_label: &str,
    url_field_label: &str,
    response_label: &str,
) -> crate::Result<()> {
    let target = build_request_target(resource_url, invalid_url_label, url_field_label)?;
    let mut request = Request::new("DELETE", &target.path_and_query)
        .header("Host", &target.host_header)
        .header("Connection", "close")
        .header("User-Agent", user_agent);
    let authorization = bearer_token.map(|token| format!("Bearer {token}"));
    if let Some(value) = authorization.as_deref() {
        request = request.header("Authorization", value);
    }
    let request = request.body(Vec::new());

    let mut stream = crate::tcp::TcpOrTlsStream::connect(&target.host, target.port, target.tls)
        .await
        .map_err(|e| Error::new(format!("failed to connect resource endpoint: {e}")))?;
    stream
        .write_all(&request.encode())
        .await
        .map_err(|e| Error::new(format!("failed to send resource DELETE request: {e}")))?;
    stream
        .flush()
        .await
        .map_err(|e| Error::new(format!("failed to flush resource DELETE request: {e}")))?;

    let response = read_http_response(&mut stream, response_label).await?;
    if !(200..300).contains(&response.status_code) {
        return Err(Error::new(format!(
            "resource endpoint returned unexpected status code for DELETE: {}",
            response.status_code
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_link_header_extracts_urls_and_credentials() {
        let parsed = parse_link_header(
            r#"<turn:turn.example.com?transport=udp>; rel="ice-server"; username="user"; credential="pass""#,
        );
        assert_eq!(parsed.urls.len(), 1);
        assert_eq!(parsed.urls[0], "turn:turn.example.com?transport=udp");
        assert_eq!(parsed.username.as_deref(), Some("user"));
        assert_eq!(parsed.credential.as_deref(), Some("pass"));
    }

    #[test]
    fn build_request_target_preserves_query() {
        let target = build_request_target(
            "https://example.com:8443/whip/live?foo=bar",
            "output URL",
            "outputUrl",
        )
        .expect("build");
        assert_eq!(target.host, "example.com");
        assert_eq!(target.port, 8443);
        assert_eq!(target.path_and_query, "/whip/live?foo=bar");
        assert_eq!(target.host_header, "example.com:8443");
        assert!(target.tls);
    }

    #[test]
    fn resolve_resource_url_supports_relative_location() {
        let resolved = resolve_resource_url(
            "https://example.com/whip/live/channel",
            "/resource/abc?token=xyz",
        )
        .expect("resolve");
        assert_eq!(resolved, "https://example.com/resource/abc?token=xyz");
    }

    #[test]
    fn resolve_resource_url_supports_absolute_location() {
        let resolved = resolve_resource_url(
            "https://example.com/whip/live/channel",
            "https://resource.example.com/session/123",
        )
        .expect("resolve");
        assert_eq!(resolved, "https://resource.example.com/session/123");
    }
}
