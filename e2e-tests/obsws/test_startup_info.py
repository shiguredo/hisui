"""obsws server の --emit-startup-info が実バインド情報を JSON Lines で出力することを確認する e2e テスト

既存の ObswsServer ヘルパー (helpers.py) は固定ポート + _wait_until_listening 前提で、
--port 0 + startup_info readline の検証には使えないため、本ファイルは独立の subprocess.Popen
で書く。本格的なヘルパー化は issue 0035 で行う。
"""

import json
import select
import socket
import subprocess
from pathlib import Path

import pytest

from hisui_server import build_hisui_command


def test_obsws_emit_startup_info_returns_actual_port(binary_path: Path):
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
        # 10 秒のタイムアウトをかけて無限ハングを防ぐ。
        assert proc.stdout is not None
        ready, _, _ = select.select([proc.stdout], [], [], 10.0)
        if not ready:
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

        assert body["type"] == "startup_info", body
        assert body["server"]["scheme"] == "http", body
        assert body["server"]["host"] == "127.0.0.1", body
        actual_port = body["server"]["port"]
        assert actual_port > 0, body
        assert body["server"]["url"] == f"http://127.0.0.1:{actual_port}", body
        assert "ui" not in body, body
        assert isinstance(body["pid"], int) and body["pid"] > 0, body

        with socket.create_connection(("127.0.0.1", actual_port), timeout=2.0):
            pass
    finally:
        proc.terminate()
        try:
            proc.communicate(timeout=5.0)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.communicate()


def test_obsws_emit_startup_info_disabled_no_stdout_output(binary_path: Path):
    """--emit-startup-info を付けない場合に stdout への出力が無いことを確認する（後方互換）"""
    cmd, cwd = build_hisui_command(
        binary_path,
        "server",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
    )

    proc = subprocess.Popen(
        cmd,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    try:
        assert proc.stdout is not None
        # 起動完了を待つために 10 秒の余裕を取り、その間 stdout に出力が無いことを確認する。
        # --emit-startup-info を付けない場合は startup_info も --emit-exit-metrics も走らない想定。
        ready, _, _ = select.select([proc.stdout], [], [], 10.0)
        if ready:
            line = proc.stdout.readline()
            pytest.fail(f"--emit-startup-info 未指定時に stdout に出力があった: {line!r}")
    finally:
        proc.terminate()
        try:
            proc.communicate(timeout=5.0)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.communicate()


def test_obsws_emit_startup_info_with_ui_no_open(binary_path: Path):
    """--ui --no-open 指定時に ui フィールドが出力され ui.url が実 addr ベースで組まれることを確認する"""
    cmd, cwd = build_hisui_command(
        binary_path,
        "server",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        "--ui",
        "--no-open",
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
        assert proc.stdout is not None
        ready, _, _ = select.select([proc.stdout], [], [], 10.0)
        if not ready:
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

        assert body["type"] == "startup_info", body
        actual_port = body["server"]["port"]
        assert actual_port > 0, body
        assert "ui" in body, body
        assert body["ui"]["url"] == f"http://127.0.0.1:{actual_port}/", body
    finally:
        proc.terminate()
        try:
            proc.communicate(timeout=5.0)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.communicate()


def test_obsws_emit_startup_info_wildcard_host(binary_path: Path):
    """--host 0.0.0.0 のワイルドカード bind 時、server.host / server.url が actual_addr ベースで組まれることを確認する"""
    # 127.0.0.1 単独テストだと引数値と actual_addr.ip() が同じ値になるため、actual_addr 経由で
    # 組まれていることが分からない。ワイルドカード 0.0.0.0 を使うと、引数値がそのまま actual_addr に
    # 反映される（kernel は 0.0.0.0 を解決しない）ので、JSON 出力が actual_addr ベースか確認できる。
    cmd, cwd = build_hisui_command(
        binary_path,
        "server",
        "--host",
        "0.0.0.0",
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
        assert proc.stdout is not None
        ready, _, _ = select.select([proc.stdout], [], [], 10.0)
        if not ready:
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

        assert body["type"] == "startup_info", body
        actual_port = body["server"]["port"]
        assert actual_port > 0, body
        assert body["server"]["host"] == "0.0.0.0", body
        assert body["server"]["url"] == f"http://0.0.0.0:{actual_port}", body
    finally:
        proc.terminate()
        try:
            proc.communicate(timeout=5.0)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.communicate()
