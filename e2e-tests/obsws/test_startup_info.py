"""obsws server の --emit-startup-info が実バインド情報を JSON Lines で出力することを確認する e2e テスト"""

import json
import select
import socket
import subprocess
from pathlib import Path

import pytest

from hisui_server import build_hisui_command


def test_emit_startup_info_returns_actual_port(binary_path: Path):
    """--port 0 + --emit-startup-info でカーネル割り当て後の実ポートが stdout に 1 行 JSON で出ることを確認する"""
    cmd, cwd = build_hisui_command(
        binary_path,
        "server",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        "--emit-startup-info",
    )

    proc = subprocess.Popen(
        cmd,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    try:
        # bind 失敗等で startup_info 行が出ないまま hisui が exit するケースに備え、
        # readline には 10 秒のタイムアウトをかける。これが無いと CI で無限ハングする。
        # 10 秒は cargo run のビルド待ちを含めた余裕を見た値。
        assert proc.stdout is not None
        ready, _, _ = select.select([proc.stdout], [], [], 10.0)
        if not ready:
            # stderr が PIPE バッファを超えてブロックしないよう、wait ではなく
            # communicate で stdout/stderr を読み切りつつプロセスを回収する。
            proc.terminate()
            try:
                _, stderr_output = proc.communicate(timeout=2.0)
            except subprocess.TimeoutExpired:
                proc.kill()
                _, stderr_output = proc.communicate()
            pytest.fail(
                f"startup_info の出力が 10 秒以内に取得できなかった (stderr: {stderr_output!r})"
            )

        line = proc.stdout.readline()
        body = json.loads(line)

        # startup_info の各フィールドを検証する
        assert body["type"] == "startup_info", body
        assert body["server"]["scheme"] == "http", body
        assert body["server"]["host"] == "127.0.0.1", body
        actual_port = body["server"]["port"]
        # --port 0 のカーネル割り当て結果なので 0 にはならない
        assert actual_port > 0, body
        # server.url は scheme + actual_addr を組み立てた完成形 URL になっている
        assert body["server"]["url"] == f"http://127.0.0.1:{actual_port}", body
        # --ui 未指定時は ui フィールドごと省略される
        assert "ui" not in body, body
        # pid は正の整数
        assert isinstance(body["pid"], int) and body["pid"] > 0, body

        # 取得した実ポートに対して TCP 接続が成立することを確認する。
        # bind 直後で accept ループ未開始でも kernel の listen backlog により接続自体は成立する。
        with socket.create_connection(("127.0.0.1", actual_port), timeout=2.0):
            pass
    finally:
        # 子プロセス (hisui) は常駐するので必ず terminate して回収する。
        # stderr が PIPE バッファを超えてブロックしないよう communicate を使う。
        proc.terminate()
        try:
            proc.communicate(timeout=5.0)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.communicate()
