"""hisui e2e テスト用 pytest fixtures"""

import datetime
import signal
import socket
import ssl
import subprocess
import tempfile
import time
from ipaddress import IPv4Address
from pathlib import Path
from typing import Generator

import httpx
import pytest
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID


def _find_binary() -> Path:
    """hisui バイナリを探す"""
    paths = [
        Path("target/release/hisui"),
        Path("target/debug/hisui"),
        Path("../target/release/hisui"),
        Path("../target/debug/hisui"),
    ]
    # 存在するバイナリのうち最も新しいものを返す
    candidates = [(p.resolve(), p.stat().st_mtime) for p in paths if p.exists()]
    if candidates:
        candidates.sort(key=lambda x: x[1], reverse=True)
        return candidates[0][0]
    raise FileNotFoundError(
        "hisui binary not found. Run 'cargo build' first."
    )


def _reserve_ephemeral_port() -> tuple[int, socket.socket]:
    """空きポートを確保して、予約ソケットとともに返す"""
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(("127.0.0.1", 0))
    port = int(sock.getsockname()[1])
    return port, sock


def _wait_for_server(
    port: int,
    timeout: float = 10.0,
    *,
    scheme: str = "http",
    verify: ssl.SSLContext | bool = True,
) -> bool:
    """サーバーの /.ok エンドポイントが 204 を返すまでリトライ"""
    start = time.time()
    while time.time() - start < timeout:
        try:
            with httpx.Client(verify=verify) as client:
                response = client.get(
                    f"{scheme}://127.0.0.1:{port}/.ok",
                    timeout=1.0,
                )
                if response.status_code == 204:
                    return True
        except (httpx.ConnectError, httpx.RemoteProtocolError):
            time.sleep(0.1)
    return False


@pytest.fixture(scope="session")
def binary_path() -> Path:
    """hisui バイナリのパス"""
    return _find_binary()


@pytest.fixture(scope="module")
def hisui_server(binary_path: Path) -> Generator[int, None, None]:
    """hisui server を起動して HTTP ポート番号を yield する"""
    port, sock = _reserve_ephemeral_port()

    tmp_dir = tempfile.TemporaryDirectory()
    tmp_path = Path(tmp_dir.name)
    log_file = tmp_path / "hisui-server.log"
    log_handle = open(log_file, "w")

    # バイナリ起動直前に予約ソケットを解放する
    sock.close()

    process = subprocess.Popen(
        [str(binary_path), "--verbose", "server", "--http-port", str(port)],
        stdout=log_handle,
        stderr=subprocess.STDOUT,
    )

    if not _wait_for_server(port):
        process.kill()
        log_handle.close()
        log_content = log_file.read_text() if log_file.exists() else "(no log)"
        tmp_dir.cleanup()
        raise RuntimeError(
            f"hisui server failed to start on port {port}.\nlog: {log_content}"
        )

    yield port

    # teardown: SIGTERM → wait → kill
    try:
        process.send_signal(signal.SIGTERM)
    except OSError:
        pass

    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            pass

    log_handle.close()
    tmp_dir.cleanup()


@pytest.fixture(scope="session")
def tls_certificate() -> Generator[tuple[Path, Path], None, None]:
    """ECDSA 自己署名証明書を生成し (cert_path, key_path) を yield する"""
    # ECDSA 鍵ペアを生成する
    private_key = ec.generate_private_key(ec.SECP256R1())

    subject = issuer = x509.Name([
        x509.NameAttribute(NameOID.COMMON_NAME, "hisui-e2e-test"),
    ])

    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(private_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(datetime.datetime.now(datetime.timezone.utc))
        .not_valid_after(
            datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(hours=1)
        )
        .add_extension(
            x509.SubjectAlternativeName([x509.IPAddress(IPv4Address("127.0.0.1"))]),
            critical=False,
        )
        .sign(private_key, hashes.SHA256())
    )

    tmp_dir = tempfile.TemporaryDirectory()
    tmp_path = Path(tmp_dir.name)

    cert_path = tmp_path / "cert.pem"
    key_path = tmp_path / "key.pem"

    cert_path.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
    key_path.write_bytes(
        private_key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        )
    )

    yield cert_path, key_path

    tmp_dir.cleanup()


@pytest.fixture(scope="module")
def hisui_https_server(
    binary_path: Path,
    tls_certificate: tuple[Path, Path],
) -> Generator[tuple[int, Path], None, None]:
    """hisui server を HTTPS で起動して (port, cert_path) を yield する"""
    cert_path, key_path = tls_certificate
    port, sock = _reserve_ephemeral_port()

    tmp_dir = tempfile.TemporaryDirectory()
    tmp_path = Path(tmp_dir.name)
    log_file = tmp_path / "hisui-https-server.log"
    log_handle = open(log_file, "w")

    # バイナリ起動直前に予約ソケットを解放する
    sock.close()

    process = subprocess.Popen(
        [
            str(binary_path),
            "--verbose",
            "server",
            "--http-port",
            str(port),
            "--https-cert-path",
            str(cert_path),
            "--https-key-path",
            str(key_path),
        ],
        stdout=log_handle,
        stderr=subprocess.STDOUT,
    )

    ssl_ctx = ssl.create_default_context(cafile=str(cert_path))
    if not _wait_for_server(port, scheme="https", verify=ssl_ctx):
        process.kill()
        log_handle.close()
        log_content = log_file.read_text() if log_file.exists() else "(no log)"
        tmp_dir.cleanup()
        raise RuntimeError(
            f"hisui HTTPS server failed to start on port {port}.\nlog: {log_content}"
        )

    yield port, cert_path

    # teardown: SIGTERM → wait → kill
    try:
        process.send_signal(signal.SIGTERM)
    except OSError:
        pass

    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            pass

    log_handle.close()
    tmp_dir.cleanup()
