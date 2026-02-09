"""hisui e2e テスト用 pytest fixtures"""

import signal
import socket
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Generator

import httpx
import pytest


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


def _wait_for_server(port: int, timeout: float = 10.0) -> bool:
    """サーバーの /.ok エンドポイントが 204 を返すまでリトライ"""
    start = time.time()
    while time.time() - start < timeout:
        try:
            with httpx.Client() as client:
                response = client.get(
                    f"http://127.0.0.1:{port}/.ok",
                    timeout=1.0,
                )
                if response.status_code == 204:
                    return True
        except httpx.ConnectError:
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
