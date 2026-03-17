"""obsws の HTTP エンドポイントに関する e2e テスト"""

import asyncio
from pathlib import Path

from helpers import (
    ObswsServer,
    _http_get,
)
from hisui_server import reserve_ephemeral_port


def test_obsws_http_ok_endpoint(binary_path: Path):
    """obsws が HTTP /.ok エンドポイントを公開することを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    http_port, http_sock = reserve_ephemeral_port()
    http_sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=ws_port,
        http_port=http_port,
        use_env=False,
    ) as server:
        status, _, _ = asyncio.run(
            _http_get(f"http://{server.http_host}:{server.http_port}/.ok")
        )
        assert status == 204


def test_obsws_http_metrics_endpoint(binary_path: Path):
    """obsws が HTTP /metrics エンドポイントを公開することを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    http_port, http_sock = reserve_ephemeral_port()
    http_sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=ws_port,
        http_port=http_port,
        use_env=False,
    ) as server:
        status, body, headers = asyncio.run(
            _http_get(f"http://{server.http_host}:{server.http_port}/metrics")
        )
        assert status == 200
        assert headers.get("Content-Type") == "text/plain; version=0.0.4; charset=utf-8"
        assert "# TYPE hisui_tokio_num_workers gauge" in body


def test_obsws_http_metrics_json_endpoint(binary_path: Path):
    """obsws が HTTP /metrics?format=json を返すことを確認する"""
    host = "127.0.0.1"
    ws_port, ws_sock = reserve_ephemeral_port()
    ws_sock.close()
    http_port, http_sock = reserve_ephemeral_port()
    http_sock.close()

    with ObswsServer(
        binary_path,
        host=host,
        port=ws_port,
        http_port=http_port,
        use_env=False,
    ) as server:
        status, body, headers = asyncio.run(
            _http_get(
                f"http://{server.http_host}:{server.http_port}/metrics?format=json"
            )
        )
        assert status == 200
        assert headers.get("Content-Type") == "application/json; charset=utf-8"
        assert '"name":"hisui_tokio_num_workers"' in body
