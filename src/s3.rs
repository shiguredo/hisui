use shiguredo_http11::ResponseDecoder;
use shiguredo_s3::{S3Client, S3Config, S3Request, S3Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::tcp::TcpOrTlsStream;

/// Sans I/O な shiguredo_s3 と実際の HTTP 通信を橋渡しするクライアント
pub struct S3HttpClient {
    client: S3Client,
}

impl S3HttpClient {
    pub fn new(config: S3Config) -> Self {
        Self {
            client: S3Client::new(config),
        }
    }

    pub fn client(&self) -> &S3Client {
        &self.client
    }

    /// S3Request を HTTP/1.1 で送信し、S3Response を返す
    pub async fn execute(&self, s3_request: &S3Request) -> crate::Result<S3Response> {
        let mut stream =
            TcpOrTlsStream::connect(&s3_request.host, s3_request.port, s3_request.https).await?;

        // shiguredo_http11::Request に変換する
        let mut http_request = shiguredo_http11::Request::new(&s3_request.method, &s3_request.uri);
        for (name, value) in &s3_request.headers {
            http_request.add_header(name, value);
        }
        http_request.add_header("Connection", "close");
        http_request.body = s3_request.body.clone();

        // リクエストを送信する
        stream.write_all(&http_request.encode()).await?;
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
                    status_code: response.status_code,
                    headers: response
                        .headers
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    body: response.body,
                };
                return Ok(s3_response);
            }
        }

        Err(crate::Error::new(
            "S3 server closed connection without sending a response",
        ))
    }
}
