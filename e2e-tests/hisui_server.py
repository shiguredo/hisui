"""hisui server e2e テスト補助クラス"""

import signal
import socket
import ssl
import subprocess
import time
from pathlib import Path
from typing import Any

import httpx
from processor_metrics import ProcessorMetrics


class HisuiServer:
    """hisui server プロセスを管理するテスト補助クラス"""

    def __init__(
        self,
        binary_path: Path,
        *,
        https_cert_path: Path | None = None,
        https_key_path: Path | None = None,
        ui_remote_url: str | None = None,
        verbose: bool = True,
    ):
        self.binary_path = binary_path
        self.https_cert_path = https_cert_path
        self.https_key_path = https_key_path
        self.ui_remote_url = ui_remote_url
        self.verbose = verbose

        self.port: int | None = None

        self._process: subprocess.Popen[None] | None = None
        self._verify: ssl.SSLContext | bool = True

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

        # バイナリ起動直前に予約ソケットを解放する
        sock.close()

        self._process = subprocess.Popen(cmd)

        self._verify = True
        if self.is_https:
            cert_path = self.https_cert_path
            if cert_path is None:
                raise RuntimeError("Internal error: https_cert_path is missing")
            self._verify = ssl.create_default_context(cafile=str(cert_path))

        if not wait_for_server(port, scheme=self.scheme, verify=self._verify):
            self._terminate_process()
            raise RuntimeError(
                f"hisui server failed to start on port {port}"
            )

        return self

    def stop(self) -> None:
        self._terminate_process()
        self._verify = True

    def request(self, method: str, path: str, **kwargs) -> httpx.Response:
        if "verify" in kwargs:
            raise ValueError("verify override is not supported")
        url = f"{self.base_url}{path}"
        with httpx.Client(verify=self._verify) as client:
            return client.request(method, url, **kwargs)

    def ok(self) -> httpx.Response:
        return self.request("GET", "/.ok")

    def metrics(self, fmt: str = "text") -> httpx.Response:
        if fmt == "json":
            return self.request("GET", "/metrics?format=json")
        if fmt == "text":
            return self.request("GET", "/metrics")
        raise ValueError("unsupported format")

    def metrics_json(self) -> list[dict[str, Any]]:
        response = self.metrics(fmt="json")
        if response.status_code != 200:
            raise RuntimeError(
                f"unexpected metrics HTTP status: {response.status_code}, body={response.text}"
            )
        data = response.json()
        if not isinstance(data, list):
            raise RuntimeError("unexpected metrics JSON format")
        return data

    def wait_processor_metric_int(
        self,
        *,
        processor_id: str,
        processor_type: str,
        metric_name: str,
        expected_value: int,
        timeout: float = 10.0,
        interval: float = 0.1,
    ) -> None:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                value = int(
                    ProcessorMetrics(
                        self.metrics_json(),
                        processor_id=processor_id,
                        processor_type=processor_type,
                    ).value(metric_name)
                )
                if value == expected_value:
                    return
            except (AssertionError, ValueError):
                pass
            time.sleep(interval)
        raise AssertionError(
            f"processor metric did not reach expected value: processor_id={processor_id}, processor_type={processor_type}, metric_name={metric_name}, expected_value={expected_value}"
        )

    def wait_processor_metric_int_stable(
        self,
        *,
        processor_id: str,
        processor_type: str,
        metric_name: str,
        expected_value: int,
        stable_duration: float = 1.0,
        timeout: float = 10.0,
        interval: float = 0.1,
    ) -> None:
        deadline = time.time() + timeout
        stable_since: float | None = None
        last_value: int | None = None

        while time.time() < deadline:
            try:
                last_value = int(
                    ProcessorMetrics(
                        self.metrics_json(),
                        processor_id=processor_id,
                        processor_type=processor_type,
                    ).value(metric_name)
                )
                if last_value == expected_value:
                    if stable_since is None:
                        stable_since = time.time()
                    if time.time() - stable_since >= stable_duration:
                        return
                else:
                    stable_since = None
            except (AssertionError, ValueError):
                stable_since = None
            time.sleep(interval)

        raise AssertionError(
            "processor metric did not stay at expected value long enough: "
            f"processor_id={processor_id}, "
            f"processor_type={processor_type}, "
            f"metric_name={metric_name}, "
            f"expected_value={expected_value}, "
            f"stable_duration={stable_duration}, "
            f"last_value={last_value}"
        )

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
