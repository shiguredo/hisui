"""hisui server リバースプロキシの e2e テスト"""

import httpx


def test_proxy_root(hisui_proxy_server: int):
    """/ への GET が upstream からプロキシされる"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_proxy_server}/")
    assert response.status_code == 200
    assert response.text == "Hello, World!"


def test_proxy_sub_path(hisui_proxy_server: int):
    """/sub/path への GET が upstream からプロキシされる"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_proxy_server}/sub/path")
    assert response.status_code == 200
    assert response.text == "Sub Path"


def test_proxy_json(hisui_proxy_server: int):
    """/json への GET で Content-Type が正しくプロキシされる"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_proxy_server}/json")
    assert response.status_code == 200
    assert response.json() == {"message": "hello"}
    assert "application/json" in response.headers["content-type"]


def test_proxy_ok_endpoint_not_proxied(hisui_proxy_server: int):
    """/.ok はプロキシされずローカルで 204 を返す"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_proxy_server}/.ok")
    assert response.status_code == 204


def test_proxy_post_returns_404(hisui_proxy_server: int):
    """POST リクエストはプロキシされず 404 を返す"""
    with httpx.Client() as client:
        response = client.post(f"http://127.0.0.1:{hisui_proxy_server}/")
    assert response.status_code == 404


def test_proxy_unknown_upstream_path(hisui_proxy_server: int):
    """upstream に存在しないパスへの GET で upstream の 404 がプロキシされる"""
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{hisui_proxy_server}/nonexistent")
    assert response.status_code == 404
