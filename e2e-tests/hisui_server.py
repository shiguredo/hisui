"""hisui server e2e テスト補助クラス"""

import signal
import socket
import ssl
import subprocess
import tempfile
import time
from pathlib import Path

import httpx


class HisuiServer:
    """hisui server プロセスを管理するテスト補助クラス"""

    def __init__(
        self,
        binary_path: Path,
        *,
        https_cert_path: Path | None = None,
        https_key_path: Path | None = None,
        ui_remote_url: str | None = None,
        startup_rpc_file: Path | None = None,
        verbose: bool = True,
    ):
        self.binary_path = binary_path
        self.https_cert_path = https_cert_path
        self.https_key_path = https_key_path
        self.ui_remote_url = ui_remote_url
        self.startup_rpc_file = startup_rpc_file
        self.verbose = verbose

        self.port: int | None = None
        self.log_file: Path | None = None

        self._process: subprocess.Popen[bytes] | None = None
        self._log_handle = None
        self._tmp_dir: tempfile.TemporaryDirectory[str] | None = None

    def __enter__(self):
        return self.start()

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.stop()

    @property
    def is_https(self) -> bool:
        return self.https_cert_path is not None and self.https_key_path is not None

    @property
    def scheme(self) -> str:
        return "https" if self.is_https else "http"

    @property
    def base_url(self) -> str:
        if self.port is None:
            raise RuntimeError("server is not started")
        return f"{self.scheme}://127.0.0.1:{self.port}"

    def start(self):
        if self._process is not None:
            raise RuntimeError("server is already started")

        if (self.https_cert_path is None) != (self.https_key_path is None):
            raise ValueError("https_cert_path and https_key_path must be provided together")

        port, sock = reserve_ephemeral_port()
        self.port = port

        self._tmp_dir = tempfile.TemporaryDirectory()
        tmp_path = Path(self._tmp_dir.name)
        self.log_file = tmp_path / "hisui-server.log"
        self._log_handle = open(self.log_file, "w")

        cmd = [str(self.binary_path)]
        if self.verbose:
            cmd.append("--verbose")
        cmd.extend(["--experimental", "server", "--http-port", str(port)])

        if self.https_cert_path and self.https_key_path:
            cmd.extend([
                "--https-cert-path",
                str(self.https_cert_path),
                "--https-key-path",
                str(self.https_key_path),
            ])

        if self.ui_remote_url:
            cmd.extend(["--ui-remote-url", self.ui_remote_url])

        if self.startup_rpc_file:
            cmd.extend(["--startup-rpc-file", str(self.startup_rpc_file)])

        # バイナリ起動直前に予約ソケットを解放する
        sock.close()

        self._process = subprocess.Popen(
            cmd,
            stdout=self._log_handle,
            stderr=subprocess.STDOUT,
        )

        verify: ssl.SSLContext | bool = True
        if self.is_https and self.https_cert_path is not None:
            verify = ssl.create_default_context(cafile=str(self.https_cert_path))

        if not wait_for_server(port, scheme=self.scheme, verify=verify):
            self._terminate_process()
            log_content = self._read_log_or_default()
            self._cleanup_temp_resources()
            raise RuntimeError(
                f"hisui server failed to start on port {port}.\nlog: {log_content}"
            )

        return self

    def stop(self) -> None:
        self._terminate_process()
        self._cleanup_temp_resources()

    def request(self, method: str, path: str, **kwargs) -> httpx.Response:
        url = f"{self.base_url}{path}"
        with httpx.Client() as client:
            return client.request(method, url, **kwargs)

    def ok(self) -> httpx.Response:
        return self.request("GET", "/.ok")

    def rpc(self, payload: dict[str, object]) -> httpx.Response:
        return self.request("POST", "/rpc", json=payload)

    def metrics(self, fmt: str = "text") -> httpx.Response:
        if fmt == "json":
            return self.request("GET", "/metrics?format=json")
        if fmt == "text":
            return self.request("GET", "/metrics")
        raise ValueError("unsupported format")

    def _terminate_process(self) -> None:
        process = self._process
        if process is None:
            return

        if process.poll() is None:
            try:
                process.send_signal(signal.SIGTERM)
            except OSError:
                pass

            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    pass

        self._process = None

    def _cleanup_temp_resources(self) -> None:
        if self._log_handle is not None:
            self._log_handle.close()
            self._log_handle = None

        if self._tmp_dir is not None:
            self._tmp_dir.cleanup()
            self._tmp_dir = None

    def _read_log_or_default(self) -> str:
        if self.log_file is None or not self.log_file.exists():
            return "(no log)"
        return self.log_file.read_text()


def reserve_ephemeral_port() -> tuple[int, socket.socket]:
    """空きポートを確保して、予約ソケットとともに返す"""
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(("127.0.0.1", 0))
    port = int(sock.getsockname()[1])
    return port, sock


def wait_for_server(
    port: int,
    timeout: float = 10.0,
    *,
    scheme: str = "http",
    verify: ssl.SSLContext | bool = True,
) -> bool:
    """サーバーの /.ok エンドポイントが 204 を返すまでリトライ"""
    start = time.time()
    while time.time() - start < timeout:
        try:
            with httpx.Client(verify=verify) as client:
                response = client.get(f"{scheme}://127.0.0.1:{port}/.ok", timeout=1.0)
                if response.status_code == 204:
                    return True
        except (httpx.ConnectError, httpx.RemoteProtocolError):
            time.sleep(0.1)
    return False
