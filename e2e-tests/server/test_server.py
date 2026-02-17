"""hisui server サブコマンドの e2e テスト"""

import json
import signal
import socket
import ssl
import subprocess
import tempfile
import time
from pathlib import Path

import httpx


def test_ok_endpoint(hisui_server: int):
    """/.ok エンドポイントが 204 No Content を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_server}/.ok")
    assert response.status_code == 204


def test_rpc_endpoint(hisui_server: int):
    """/rpc への GET は 405 Method Not Allowed を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_server}/rpc")
    assert response.status_code == 405
    assert response.headers.get("allow") == "POST"


def test_rpc_post_endpoint(hisui_server: int):
    """/rpc への POST で JSON-RPC が実行される"""
    request_json = {"jsonrpc": "2.0", "id": 1, "method": "listProcessors"}
    with httpx.Client() as client:
        response = client.post(f"http://127.0.0.1:{hisui_server}/rpc", json=request_json)
    assert response.status_code == 200
    assert "application/json" in response.headers.get("content-type", "")
    assert response.json() == {"jsonrpc": "2.0", "id": 1, "result": []}


def test_rpc_post_notification_returns_204(hisui_server: int):
    """/rpc 通知（id なし）は 204 No Content を返す"""
    request_json = {"jsonrpc": "2.0", "method": "listProcessors"}
    with httpx.Client() as client:
        response = client.post(f"http://127.0.0.1:{hisui_server}/rpc", json=request_json)
    assert response.status_code == 204
    assert response.content == b""


def test_bootstrap_endpoint(hisui_server: int):
    """/bootstrap への GET は 405 Method Not Allowed を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_server}/bootstrap")
    assert response.status_code == 405


def test_unknown_endpoint(hisui_server: int):
    """未知のパスが 404 Not Found を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_server}/unknown")
    assert response.status_code == 404


def test_https_ok_endpoint(hisui_https_server: tuple[int, Path]):
    """HTTPS /.ok エンドポイントに証明書検証付きで接続し 204 を確認する"""
    port, cert_path = hisui_https_server
    ssl_ctx = ssl.create_default_context(cafile=str(cert_path))
    with httpx.Client(verify=ssl_ctx) as client:
        response = client.get(f"https://127.0.0.1:{port}/.ok")
    assert response.status_code == 204


def test_https_ok_endpoint_no_verify(hisui_https_server: tuple[int, Path]):
    """HTTPS /.ok エンドポイントに verify=False で接続し 204 を確認する"""
    port, _cert_path = hisui_https_server
    with httpx.Client(verify=False) as client:
        response = client.get(f"https://127.0.0.1:{port}/.ok")
    assert response.status_code == 204


def test_startup_rpc_file_is_executed(binary_path: Path):
    """--startup-rpc-file で指定した通知配列が起動時に実行される"""
    port, sock = _reserve_ephemeral_port()
    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)
        startup_rpc_file = tmp_path / "startup-rpcs.json"
        startup_rpc_file.write_text(
            json.dumps([{"jsonrpc": "2.0", "method": "listProcessors"}])
        )

        log_file = tmp_path / "hisui-server.log"
        log_handle = open(log_file, "w")
        sock.close()

        process = subprocess.Popen(
            [
                str(binary_path),
                "--verbose",
                "--experimental",
                "server",
                "--http-port",
                str(port),
                "--startup-rpc-file",
                str(startup_rpc_file),
            ],
            stdout=log_handle,
            stderr=subprocess.STDOUT,
        )

        try:
            assert _wait_for_server(port), log_file.read_text()
            with httpx.Client() as client:
                response = client.get(f"http://127.0.0.1:{port}/.ok")
            assert response.status_code == 204
        finally:
            _terminate_process(process)
            log_handle.close()


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
                response = client.get(f"http://127.0.0.1:{port}/.ok", timeout=1.0)
                if response.status_code == 204:
                    return True
        except (httpx.ConnectError, httpx.RemoteProtocolError):
            time.sleep(0.1)
    return False


def _wait_for_process_exit(process: subprocess.Popen[bytes], timeout: float = 10.0) -> bool:
    """プロセス終了を待つ"""
    start = time.time()
    while time.time() - start < timeout:
        if process.poll() is not None:
            return True
        time.sleep(0.1)
    return False


def _terminate_process(process: subprocess.Popen[bytes]) -> None:
    """プロセスを安全に終了する"""
    if process.poll() is not None:
        return
    try:
        process.send_signal(signal.SIGTERM)
    except OSError:
        return
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            pass
