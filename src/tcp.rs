use std::sync::Arc;

use rustls_platform_verifier::ConfigVerifierExt;

#[derive(Debug)]
pub enum TcpOrTlsStream {
    Tcp(tokio::net::TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>),
}

impl TcpOrTlsStream {
    pub async fn connect_tcp<A: tokio::net::ToSocketAddrs>(addr: A) -> std::io::Result<Self> {
        let stream = tokio::net::TcpStream::connect(addr).await?;
        Ok(TcpOrTlsStream::Tcp(stream))
    }

    pub async fn connect_tls<A: tokio::net::ToSocketAddrs>(
        addr: A,
        server_name: &str,
    ) -> std::io::Result<Self> {
        // TLS設定をプラットフォームの証明書ストアを使用して作成
        let config = rustls::ClientConfig::with_platform_verifier().map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create TLS config: {e}"),
            )
        })?;

        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));

        // 最初にプレーンなTCP接続を確立
        let tcp_stream = tokio::net::TcpStream::connect(addr).await?;

        // TLS SNI用のサーバー名を作成
        let server_name_ref = rustls::pki_types::ServerName::try_from(server_name.to_string())
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Invalid server name: {e}"),
                )
            })?;

        // TLSハンドシェイクを実行
        let tls_stream = connector
            .connect(server_name_ref, tcp_stream)
            .await
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("TLS handshake failed: {e}"),
                )
            })?;

        Ok(TcpOrTlsStream::Tls(Box::new(tls_stream)))
    }
}

impl tokio::io::AsyncRead for TcpOrTlsStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            TcpOrTlsStream::Tcp(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
            TcpOrTlsStream::Tls(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for TcpOrTlsStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        match &mut *self {
            TcpOrTlsStream::Tcp(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
            TcpOrTlsStream::Tls(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match &mut *self {
            TcpOrTlsStream::Tcp(stream) => std::pin::Pin::new(stream).poll_flush(cx),
            TcpOrTlsStream::Tls(stream) => std::pin::Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match &mut *self {
            TcpOrTlsStream::Tcp(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
            TcpOrTlsStream::Tls(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
        }
    }
}
