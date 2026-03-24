"""obsws の HTTP エンドポイントに関する e2e テスト"""

import asyncio
import datetime
import socket
import socketserver
import ssl
import struct
import threading
import time
from http.server import BaseHTTPRequestHandler
from ipaddress import IPv4Address
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID

from helpers import ObswsServer, _http_get, _http_request
from hisui_server import reserve_ephemeral_port


class _UpstreamHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _send_body(self, status: int, body: bytes, content_type: str):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/":
            self._send_body(200, b"Hello, World!", "text/plain; charset=utf-8")
            return
        if self.path == "/sub/path":
            self._send_body(200, b"Sub Path", "text/plain; charset=utf-8")
            return
        if self.path == "/json":
            self._send_body(200, b'{"message":"hello"}', "application/json")
            return
        if self.path == "/slow":
            time.sleep(1.0)
            self._send_body(200, b"slow", "text/plain; charset=utf-8")
            return

        self._send_body(404, b"Not Found", "text/plain; charset=utf-8")

    def log_message(self, format, *args):
        pass


class _ThreadingTcpServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True


class _UpstreamServer:
    def __init__(self):
        self.port, sock = reserve_ephemeral_port()
        sock.close()
        self._server = _ThreadingTcpServer(("127.0.0.1", self.port), _UpstreamHandler)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    def __enter__(self):
        self._thread.start()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=3.0)


def _create_self_signed_cert(tmp_path: Path) -> tuple[Path, Path]:
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    subject = issuer = x509.Name(
        [
            x509.NameAttribute(NameOID.COUNTRY_NAME, "JP"),
            x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Shiguredo"),
            x509.NameAttribute(NameOID.COMMON_NAME, "127.0.0.1"),
        ]
    )
    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(days=1))
        .not_valid_after(datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(days=30))
        .add_extension(
            x509.SubjectAlternativeName(
                [
                    x509.DNSName("localhost"),
                    x509.IPAddress(IPv4Address("127.0.0.1")),
                ]
            ),
            critical=False,
        )
        .sign(key, hashes.SHA256())
    )

    cert_path = tmp_path / "obsws-test-cert.pem"
    key_path = tmp_path / "obsws-test-key.pem"
    cert_path.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
    key_path.write_bytes(
        key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.TraditionalOpenSSL,
            serialization.NoEncryption(),
        )
    )
    return cert_path, key_path


def _client_ssl_context(cert_path: Path) -> ssl.SSLContext:
    return ssl.create_default_context(cafile=str(cert_path))


def test_obsws_http_ok_endpoint(binary_path: Path):
    """obsws が HTTP /.ok エンドポイントを公開することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False) as server:
        status, _, _ = asyncio.run(_http_get(f"http://{server.host}:{server.port}/.ok"))
        assert status == 204


def test_obsws_http_metrics_endpoint(binary_path: Path):
    """obsws が HTTP /metrics エンドポイントを公開することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False) as server:
        status, body, headers = asyncio.run(
            _http_get(f"http://{server.host}:{server.port}/metrics")
        )
        assert status == 200
        assert headers.get("Content-Type") == "text/plain; version=0.0.4; charset=utf-8"
        assert "# TYPE hisui_tokio_num_workers gauge" in body


def test_obsws_http_metrics_json_endpoint(binary_path: Path):
    """obsws が HTTP /metrics?format=json を返すことを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False) as server:
        status, body, headers = asyncio.run(
            _http_get(f"http://{server.host}:{server.port}/metrics?format=json")
        )
        assert status == 200
        assert headers.get("Content-Type") == "application/json; charset=utf-8"
        assert '"name":"hisui_tokio_num_workers"' in body


def test_obsws_http_bootstrap_get_returns_405(binary_path: Path):
    """obsws が GET /bootstrap を拒否することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with ObswsServer(binary_path, host=host, port=port, use_env=False) as server:
        status, _, _ = asyncio.run(
            _http_request("GET", f"http://{server.host}:{server.port}/bootstrap")
        )
        assert status == 405


def test_obsws_proxy_root(binary_path: Path):
    """obsws が root への GET を upstream にリバースプロキシすることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with _UpstreamServer() as upstream:
        with ObswsServer(
            binary_path,
            host=host,
            port=port,
            ui_remote_url=f"http://127.0.0.1:{upstream.port}",
            use_env=False,
        ) as server:
            status, body, _ = asyncio.run(_http_get(f"http://{server.host}:{server.port}/"))
            assert status == 200
            assert body == "Hello, World!"


def test_obsws_proxy_sub_path(binary_path: Path):
    """obsws がサブパスへの GET を upstream にリバースプロキシすることを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with _UpstreamServer() as upstream:
        with ObswsServer(
            binary_path,
            host=host,
            port=port,
            ui_remote_url=f"http://127.0.0.1:{upstream.port}",
            use_env=False,
        ) as server:
            status, body, _ = asyncio.run(
                _http_get(f"http://{server.host}:{server.port}/sub/path")
            )
            assert status == 200
            assert body == "Sub Path"


def test_obsws_proxy_json(binary_path: Path):
    """obsws が JSON レスポンスの Content-Type を維持することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with _UpstreamServer() as upstream:
        with ObswsServer(
            binary_path,
            host=host,
            port=port,
            ui_remote_url=f"http://127.0.0.1:{upstream.port}",
            use_env=False,
        ) as server:
            status, body, headers = asyncio.run(
                _http_get(f"http://{server.host}:{server.port}/json")
            )
            assert status == 200
            assert body == '{"message":"hello"}'
            assert "application/json" in headers["Content-Type"]


def test_obsws_proxy_ok_endpoint_not_proxied(binary_path: Path):
    """obsws が /.ok を upstream に流さずローカルで返すことを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with _UpstreamServer() as upstream:
        with ObswsServer(
            binary_path,
            host=host,
            port=port,
            ui_remote_url=f"http://127.0.0.1:{upstream.port}",
            use_env=False,
        ) as server:
            status, _, _ = asyncio.run(_http_get(f"http://{server.host}:{server.port}/.ok"))
            assert status == 204


def test_obsws_proxy_post_returns_404(binary_path: Path):
    """obsws が proxy 対象外の POST を 404 で返すことを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with _UpstreamServer() as upstream:
        with ObswsServer(
            binary_path,
            host=host,
            port=port,
            ui_remote_url=f"http://127.0.0.1:{upstream.port}",
            use_env=False,
        ) as server:
            status, _, _ = asyncio.run(
                _http_request("POST", f"http://{server.host}:{server.port}/")
            )
            assert status == 404


def test_obsws_proxy_unknown_upstream_path(binary_path: Path):
    """obsws が upstream の 404 をそのまま返すことを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with _UpstreamServer() as upstream:
        with ObswsServer(
            binary_path,
            host=host,
            port=port,
            ui_remote_url=f"http://127.0.0.1:{upstream.port}",
            use_env=False,
        ) as server:
            status, _, _ = asyncio.run(
                _http_get(f"http://{server.host}:{server.port}/nonexistent")
            )
            assert status == 404


def test_obsws_proxy_client_disconnect_does_not_crash_server(binary_path: Path):
    """obsws が proxy 中の client disconnect 後も継続稼働することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()

    with _UpstreamServer() as upstream:
        with ObswsServer(
            binary_path,
            host=host,
            port=port,
            ui_remote_url=f"http://127.0.0.1:{upstream.port}",
            use_env=False,
        ) as server:
            rst_sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            rst_sock.connect((server.host, server.port))
            rst_sock.sendall(b"GET /slow HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            time.sleep(0.1)
            rst_sock.setsockopt(
                socket.SOL_SOCKET,
                socket.SO_LINGER,
                struct.pack("ii", 1, 0),
            )
            rst_sock.close()

            time.sleep(1.2)
            status, _, _ = asyncio.run(_http_get(f"http://{server.host}:{server.port}/.ok"))
            assert status == 204


def test_obsws_https_ok_endpoint(binary_path: Path, tmp_path: Path):
    """obsws が HTTPS /.ok エンドポイントを公開することを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()
    cert_path, key_path = _create_self_signed_cert(tmp_path)
    ssl_context = _client_ssl_context(cert_path)

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        https_cert_path=cert_path,
        https_key_path=key_path,
        use_env=False,
    ) as server:
        status, _, _ = asyncio.run(
            _http_request(
                "GET",
                f"https://{server.host}:{server.port}/.ok",
                ssl_context=ssl_context,
            )
        )
        assert status == 204


def test_obsws_https_metrics_json_endpoint(binary_path: Path, tmp_path: Path):
    """obsws が HTTPS でも /metrics?format=json を返すことを確認する"""
    host = "127.0.0.1"
    port, sock = reserve_ephemeral_port()
    sock.close()
    cert_path, key_path = _create_self_signed_cert(tmp_path)
    ssl_context = _client_ssl_context(cert_path)

    with ObswsServer(
        binary_path,
        host=host,
        port=port,
        https_cert_path=cert_path,
        https_key_path=key_path,
        use_env=False,
    ) as server:
        status, body, headers = asyncio.run(
            _http_request(
                "GET",
                f"https://{server.host}:{server.port}/metrics?format=json",
                ssl_context=ssl_context,
            )
        )
        assert status == 200
        assert headers.get("Content-Type") == "application/json; charset=utf-8"
        assert '"name":"hisui_tokio_num_workers"' in body
