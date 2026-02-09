"""hisui server サブコマンドの e2e テスト"""

import ssl
from pathlib import Path

import httpx


def test_ok_endpoint(hisui_server: int):
    """/.ok エンドポイントが 204 No Content を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_server}/.ok")
    assert response.status_code == 204


def test_rpc_endpoint(hisui_server: int):
    """/rpc エンドポイントが 204 No Content を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_server}/rpc")
    assert response.status_code == 204


def test_bootstrap_endpoint(hisui_server: int):
    """/bootstrap エンドポイントが 204 No Content を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_server}/bootstrap")
    assert response.status_code == 204


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
