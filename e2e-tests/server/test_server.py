"""hisui server サブコマンドの e2e テスト"""

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
