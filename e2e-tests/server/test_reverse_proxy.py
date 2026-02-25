"""hisui server リバースプロキシの e2e テスト"""

import socket
import struct
import time

import httpx


def test_proxy_root(hisui_proxy_server: int):
    """/ への GET が upstream からプロキシされる"""
    port = hisui_proxy_server
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/")
    assert response.status_code == 200
    assert response.text == "Hello, World!"


def test_proxy_sub_path(hisui_proxy_server: int):
    """/sub/path への GET が upstream からプロキシされる"""
    port = hisui_proxy_server
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/sub/path")
    assert response.status_code == 200
    assert response.text == "Sub Path"


def test_proxy_json(hisui_proxy_server: int):
    """/json への GET で Content-Type が正しくプロキシされる"""
    port = hisui_proxy_server
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/json")
    assert response.status_code == 200
    assert response.json() == {"message": "hello"}
    assert "application/json" in response.headers["content-type"]


def test_proxy_ok_endpoint_not_proxied(hisui_proxy_server: int):
    """/.ok はプロキシされずローカルで 204 を返す"""
    port = hisui_proxy_server
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/.ok")
    assert response.status_code == 204


def test_proxy_post_returns_404(hisui_proxy_server: int):
    """POST リクエストはプロキシされず 404 を返す"""
    port = hisui_proxy_server
    with httpx.Client() as client:
        response = client.post(f"http://127.0.0.1:{port}/")
    assert response.status_code == 404


def test_proxy_unknown_upstream_path(hisui_proxy_server: int):
    """upstream に存在しないパスへの GET で upstream の 404 がプロキシされる"""
    port = hisui_proxy_server
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/nonexistent")
    assert response.status_code == 404


def test_proxy_client_disconnect_does_not_crash_server(hisui_proxy_server: int):
    """クライアントが切断してもサーバーが継続稼働する"""
    port = hisui_proxy_server

    # ソケットで /slow にリクエストを送信し、RST で即座に切断する
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.connect(("127.0.0.1", port))
    sock.sendall(b"GET /slow HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
    # upstream が処理を開始するのを待ってから RST で切断する
    time.sleep(0.1)
    # SO_LINGER(1, 0) で FIN ではなく RST を送信して即座にリセットする
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    sock.close()

    # 切断検出処理の完了を待つ
    time.sleep(4)

    # サーバーがクラッシュしていないことを確認する
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/.ok")
    assert response.status_code == 204
