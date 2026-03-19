"""hisui server サブコマンドの e2e テスト"""

import httpx
import pytest

from hisui_server import HisuiServer


def test_ok_endpoint(hisui_server: HisuiServer):
    """/.ok エンドポイントが 204 No Content を返す"""
    response = hisui_server.ok()
    assert response.status_code == 204


def test_bootstrap_endpoint(hisui_server: HisuiServer):
    """/bootstrap への GET は 405 Method Not Allowed を返す"""
    response = hisui_server.request("GET", "/bootstrap")
    assert response.status_code == 405


def test_metrics_endpoint(hisui_server: HisuiServer):
    """/metrics は Prometheus text を返す"""
    response = hisui_server.metrics(fmt="text")
    assert response.status_code == 200
    assert "text/plain" in response.headers.get("content-type", "")
    assert "hisui_tokio_num_workers" in response.text


def test_metrics_json_endpoint(hisui_server: HisuiServer):
    """/metrics?format=json は JSON を返す"""
    response = hisui_server.metrics(fmt="json")
    assert response.status_code == 200
    assert "application/json" in response.headers.get("content-type", "")
    assert isinstance(response.json(), list)


def test_unknown_endpoint(hisui_server: HisuiServer):
    """未知のパスが 404 Not Found を返す"""
    response = hisui_server.request("GET", "/unknown")
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
