"""hisui server リバースプロキシの e2e テスト"""

import socket
import struct
import time
from pathlib import Path

import httpx


def test_proxy_root(hisui_proxy_server: tuple[int, Path]):
    """/ への GET が upstream からプロキシされる"""
    port, _log = hisui_proxy_server
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/")
    assert response.status_code == 200
    assert response.text == "Hello, World!"


def test_proxy_sub_path(hisui_proxy_server: tuple[int, Path]):
    """/sub/path への GET が upstream からプロキシされる"""
    port, _log = hisui_proxy_server
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/sub/path")
    assert response.status_code == 200
    assert response.text == "Sub Path"


def test_proxy_json(hisui_proxy_server: tuple[int, Path]):
    """/json への GET で Content-Type が正しくプロキシされる"""
    port, _log = hisui_proxy_server
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/json")
    assert response.status_code == 200
    assert response.json() == {"message": "hello"}
    assert "application/json" in response.headers["content-type"]


def test_proxy_ok_endpoint_not_proxied(hisui_proxy_server: tuple[int, Path]):
    """/.ok はプロキシされずローカルで 204 を返す"""
    port, _log = hisui_proxy_server
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/.ok")
    assert response.status_code == 204


def test_proxy_post_returns_404(hisui_proxy_server: tuple[int, Path]):
    """POST リクエストはプロキシされず 404 を返す"""
    port, _log = hisui_proxy_server
    with httpx.Client() as client:
        response = client.post(f"http://127.0.0.1:{port}/")
    assert response.status_code == 404


def test_proxy_unknown_upstream_path(hisui_proxy_server: tuple[int, Path]):
    """upstream に存在しないパスへの GET で upstream の 404 がプロキシされる"""
    port, _log = hisui_proxy_server
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/nonexistent")
    assert response.status_code == 404


def test_proxy_client_disconnect_logs_499(hisui_proxy_server: tuple[int, Path]):
    """クライアントが切断した場合に 499 がログに記録される"""
    port, log_file = hisui_proxy_server

    # ソケットで /slow にリクエストを送信し、RST で即座に切断する
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.connect(("127.0.0.1", port))
    sock.sendall(b"GET /slow HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
    # upstream が処理を開始するのを待ってから RST で切断する
    time.sleep(0.1)
    # SO_LINGER(1, 0) で FIN ではなく RST を送信して即座にリセットする
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    sock.close()

    # hisui がクライアント切断を検出してログに書き込むのを待つ
    time.sleep(4)

    log_content = log_file.read_text()
    assert "499 Client Closed Request" in log_content

    # サーバーがクラッシュしていないことを確認する
    with httpx.Client() as client:
        response = client.get(f"http://127.0.0.1:{port}/.ok")
    assert response.status_code == 204
