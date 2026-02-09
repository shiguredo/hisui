use std::path::PathBuf;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls_platform_verifier::ConfigVerifierExt;

#[derive(Debug)]
pub enum TcpOrTlsStream {
    Tcp(tokio::net::TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>),
}

impl TcpOrTlsStream {
    pub async fn connect(host: &str, port: u16, tls: bool) -> std::io::Result<Self> {
        if tls {
            Self::connect_tls(host, port).await
        } else {
            Self::connect_tcp(host, port).await
        }
    }

    async fn connect_tcp(host: &str, port: u16) -> std::io::Result<Self> {
        let stream = tokio::net::TcpStream::connect(format!("{host}:{port}")).await?;
        Ok(TcpOrTlsStream::Tcp(stream))
    }

    async fn connect_tls(host: &str, port: u16) -> std::io::Result<Self> {
        let config = rustls::ClientConfig::with_platform_verifier()
            .map_err(|e| std::io::Error::other(format!("Failed to create TLS config: {e}")))?;

        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let tcp_stream = tokio::net::TcpStream::connect(format!("{host}:{port}")).await?;

        let server_name_ref =
            rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Invalid server name: {e}"),
                )
            })?;

        let tls_stream = connector
            .connect(server_name_ref, tcp_stream)
            .await
            .map_err(|e| std::io::Error::other(format!("TLS handshake failed: {e}")))?;

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

#[derive(Debug)]
pub enum ServerTcpOrTlsStream {
    Tcp(tokio::net::TcpStream),
    Tls(Box<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>),
}

impl ServerTcpOrTlsStream {
    pub async fn accept_with_tls(
        stream: tokio::net::TcpStream,
        tls_acceptor: Option<&Arc<tokio_rustls::TlsAcceptor>>,
    ) -> std::io::Result<Self> {
        match tls_acceptor {
            Some(acceptor) => {
                let tls_stream = acceptor.accept(stream).await?;
                Ok(ServerTcpOrTlsStream::Tls(Box::new(tls_stream)))
            }
            None => Ok(ServerTcpOrTlsStream::Tcp(stream)),
        }
    }
}

impl tokio::io::AsyncRead for ServerTcpOrTlsStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            ServerTcpOrTlsStream::Tcp(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
            ServerTcpOrTlsStream::Tls(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for ServerTcpOrTlsStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        match &mut *self {
            ServerTcpOrTlsStream::Tcp(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
            ServerTcpOrTlsStream::Tls(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match &mut *self {
            ServerTcpOrTlsStream::Tcp(stream) => std::pin::Pin::new(stream).poll_flush(cx),
            ServerTcpOrTlsStream::Tls(stream) => std::pin::Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match &mut *self {
            ServerTcpOrTlsStream::Tcp(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
            ServerTcpOrTlsStream::Tls(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
        }
    }
}

/// サーバー側 TLSAcceptor を作成する
pub async fn create_server_tls_acceptor(
    cert_path: &PathBuf,
    key_path: &PathBuf,
) -> std::io::Result<Arc<tokio_rustls::TlsAcceptor>> {
    tracing::debug!("Loading TLS certificates from {}", cert_path.display());

    let certs = rustls::pki_types::CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| std::io::Error::other(format!("Failed to open certificate file: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| std::io::Error::other(format!("Failed to parse certificate file: {e}")))?;

    if certs.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "No certificates found in cert file",
        ));
    }

    tracing::debug!("Loaded {} certificate(s)", certs.len());

    tracing::debug!("Loading private key from {}", key_path.display());
    let key = rustls::pki_types::PrivateKeyDer::from_pem_file(key_path)
        .map_err(|e| std::io::Error::other(format!("Failed to load private key: {e}")))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| std::io::Error::other(format!("Failed to create server config: {e}")))?;

    Ok(Arc::new(tokio_rustls::TlsAcceptor::from(Arc::new(config))))
}
