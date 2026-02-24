"""hisui server サブコマンドの e2e テスト"""

import json
import tempfile
from pathlib import Path

import httpx
import pytest

from hisui_server import HisuiServer


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


def test_metrics_endpoint(hisui_server: int):
    """/metrics は Prometheus text を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_server}/metrics")
    assert response.status_code == 200
    assert "text/plain" in response.headers.get("content-type", "")
    assert "hisui_tokio_num_workers" in response.text


def test_metrics_json_endpoint(hisui_server: int):
    """/metrics?format=json は JSON を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_server}/metrics?format=json")
    assert response.status_code == 200
    assert "application/json" in response.headers.get("content-type", "")
    assert isinstance(response.json(), list)


def test_unknown_endpoint(hisui_server: int):
    """未知のパスが 404 Not Found を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_server}/unknown")
    assert response.status_code == 404


def test_https_ok_endpoint(hisui_https_server: HisuiServer):
    """HTTPS でも HisuiServer.ok() が 204 を返す"""
    response = hisui_https_server.ok()
    assert response.status_code == 204


def test_https_metrics_json_endpoint(hisui_https_server: HisuiServer):
    """HTTPS でも HisuiServer.metrics() が JSON を返す"""
    response = hisui_https_server.metrics(fmt="json")
    assert response.status_code == 200
    assert "application/json" in response.headers.get("content-type", "")
    assert isinstance(response.json(), list)


def test_https_rpc_call_endpoint(hisui_https_server: HisuiServer):
    """HTTPS でも HisuiServer.rpc_call() で JSON-RPC が実行される"""
    response = hisui_https_server.rpc_call("listProcessors")
    assert response["result"] == []


def test_https_request_verify_override_is_rejected(hisui_https_server: HisuiServer):
    """request() への verify 上書き指定は明示的に拒否する"""
    with pytest.raises(ValueError, match="verify override is not supported"):
        hisui_https_server.request("GET", "/.ok", verify=False)


def test_https_direct_no_verify_endpoint(hisui_https_server: HisuiServer):
    """直接 httpx.Client(verify=False) を使うアクセスも 204 を返す"""
    assert hisui_https_server.port is not None
    with httpx.Client(verify=False) as client:
        response = client.get(f"https://127.0.0.1:{hisui_https_server.port}/.ok")
    assert response.status_code == 204


def test_startup_rpc_file_is_executed(binary_path: Path):
    """--startup-rpc-file で指定した通知配列が起動時に実行される"""
    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)
        startup_rpc_file = tmp_path / "startup-rpcs.json"
        startup_rpc_file.write_text(
            json.dumps([{"jsonrpc": "2.0", "method": "listProcessors"}])
        )

        with HisuiServer(binary_path, startup_rpc_file=startup_rpc_file) as server:
            response = server.ok()
            assert response.status_code == 204
