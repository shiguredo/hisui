"""hisui e2e テスト用 pytest fixtures"""

import asyncio
import datetime
import tempfile
from ipaddress import IPv4Address
from pathlib import Path
from typing import Generator

import pytest
from aiohttp import web
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

from hisui_server import HisuiServer, reserve_ephemeral_port


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
    raise FileNotFoundError("hisui binary not found. Run 'cargo build' first.")


@pytest.fixture(scope="session")
def binary_path() -> Path:
    """hisui バイナリのパス"""
    return _find_binary()


@pytest.fixture(scope="module")
def hisui_server(binary_path: Path) -> Generator[HisuiServer, None, None]:
    """hisui server を HTTP で起動して HisuiServer を yield する"""
    with HisuiServer(binary_path) as server:
        yield server


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
) -> Generator[HisuiServer, None, None]:
    """hisui server を HTTPS で起動して HisuiServer を yield する"""
    cert_path, key_path = tls_certificate
    with HisuiServer(
        binary_path,
        https_cert_path=cert_path,
        https_key_path=key_path,
    ) as server:
        yield server


async def _handle_root(request: web.Request) -> web.Response:
    return web.Response(text="Hello, World!", content_type="text/plain")


async def _handle_sub_path(request: web.Request) -> web.Response:
    return web.Response(text="Sub Path", content_type="text/plain")


async def _handle_json(request: web.Request) -> web.Response:
    return web.json_response({"message": "hello"})


async def _handle_slow(request: web.Request) -> web.Response:
    """レスポンスを遅延させ、大きなレスポンスを返す（499 テスト用）"""
    await asyncio.sleep(3)
    # 大きなレスポンスを返して write_all 途中でのエラー検出を確実にする
    body = "x" * 1024 * 1024
    return web.Response(text=body, content_type="text/plain")


def _create_upstream_app() -> web.Application:
    """upstream テスト用 aiohttp アプリケーションを作成する"""
    app = web.Application()
    app.router.add_get("/", _handle_root)
    app.router.add_get("/sub/path", _handle_sub_path)
    app.router.add_get("/json", _handle_json)
    app.router.add_get("/slow", _handle_slow)
    return app


@pytest.fixture(scope="module")
def upstream_server() -> Generator[int, None, None]:
    """upstream テスト用 aiohttp サーバーを起動してポート番号を yield する"""
    port, sock = reserve_ephemeral_port()
    sock.close()

    loop = asyncio.new_event_loop()
    app = _create_upstream_app()
    runner = web.AppRunner(app)
    loop.run_until_complete(runner.setup())
    site = web.TCPSite(runner, "127.0.0.1", port)
    loop.run_until_complete(site.start())

    # イベントループをバックグラウンドスレッドで実行する
    import threading

    thread = threading.Thread(target=loop.run_forever, daemon=True)
    thread.start()

    yield port

    loop.call_soon_threadsafe(loop.stop)
    thread.join(timeout=5)
    loop.run_until_complete(runner.cleanup())
    loop.close()


@pytest.fixture(scope="module")
def hisui_proxy_server(
    binary_path: Path,
    upstream_server: int,
) -> Generator[int, None, None]:
    """--ui-remote-url 付きで hisui server を起動して port を yield する"""
    with HisuiServer(
        binary_path,
        ui_remote_url=f"http://127.0.0.1:{upstream_server}",
    ) as server:
        assert server.port is not None
        yield server.port
