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
    # --no-open でブラウザ自動起動を切っても、UI 自体は有効なので ui フィールドは出力される
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
        # --ui --no-open でも ui フィールドが出力される
        assert "ui" in body, body
        # ui.url は scheme + actual_addr + "/" を組み立てた完成形 URL
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
        # ワイルドカード host がそのまま server.host に入る
        assert body["server"]["host"] == "0.0.0.0", body
        # server.url も actual_addr ベースで組み立てられる
        # （ワイルドカード bind の場合は接続用 URL として機能しないが、host / port の整合性を確認する）
        assert body["server"]["url"] == f"http://0.0.0.0:{actual_port}", body
    finally:
        proc.terminate()
        try:
            proc.communicate(timeout=5.0)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.communicate()
