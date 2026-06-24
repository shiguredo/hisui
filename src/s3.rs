use shiguredo_http11::{HttpHead, ResponseDecoder};
use shiguredo_s3::{Client, Config, S3Request, S3Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::tcp::TcpOrTlsStream;

/// Sans I/O な shiguredo_s3 と実際の HTTP 通信を橋渡しするクライアント
pub struct S3HttpClient {
    client: Client,
}

impl S3HttpClient {
    pub fn new(config: Config) -> Self {
        Self {
            client: Client::from_conf(config),
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// S3Request を HTTP/1.1 で送信し、S3Response を返す。
    /// リクエストごとに TCP 接続を新規作成する。
    /// HLS セグメントの書き出し頻度（数秒に 1 回）では接続プールの効果が限定的なため、
    /// 単純さを優先してこの方式を採用している。
    pub async fn execute(&self, s3_request: &S3Request) -> crate::Result<S3Response> {
        let mut stream =
            TcpOrTlsStream::connect(&s3_request.host, s3_request.port, s3_request.https).await?;

        // shiguredo_http11::Request に変換する。
        // Method の TryFrom<&str> は &'static str のみ実装されているので、
        // 動的文字列の S3Request.method からは Method::new 経由で構築する。
        let method = shiguredo_http11::Method::new(&s3_request.method)
            .map_err(|e| crate::Error::new(format!("invalid S3 request method: {e}")))?;
        let mut http_request = shiguredo_http11::Request::new(method, s3_request.uri.clone())
            .map_err(|e| crate::Error::new(format!("failed to build HTTP request: {e}")))?;
        for (name, value) in &s3_request.headers {
            // HeaderName の TryFrom も &'static str のみなので動的文字列は HeaderName::new を使う。
            let header_name = shiguredo_http11::HeaderName::new(name.as_str())
                .map_err(|e| crate::Error::new(format!("invalid S3 header name: {e}")))?;
            http_request
                .add_header(header_name, value.as_str())
                .map_err(|e| crate::Error::new(format!("failed to add S3 header: {e}")))?;
        }
        http_request
            .add_header("Connection", "close")
            .map_err(|e| crate::Error::new(format!("failed to add Connection header: {e}")))?;
        http_request.set_body(s3_request.body.clone());

        // リクエストを送信する
        let encoded = http_request
            .encode()
            .map_err(|e| crate::Error::new(format!("failed to encode S3 request: {e}")))?;
        stream.write_all(&encoded).await?;
        stream.flush().await?;

        // レスポンスを受信する
        let mut response_decoder = ResponseDecoder::new();
        let mut buf = vec![0u8; 8192];

        loop {
            let n = stream.read(&mut buf).await?;
            if n == 0 {
                break;
            }

            response_decoder.feed(&buf[..n])?;

            if let Some(response) = response_decoder.decode()? {
                let s3_response = S3Response {
                    status_code: response.status_code(),
                    headers: response
                        .headers()
                        .iter()
                        .map(|(k, v)| (k.as_str().to_owned(), v.clone()))
                        .collect(),
                    body: response.body_bytes().unwrap_or(&[]).to_vec(),
                };
                return Ok(s3_response);
            }
        }

        Err(crate::Error::new(
            "S3 server closed connection without sending a response",
        ))
    }
}
