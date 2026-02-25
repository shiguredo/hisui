"""hisui server processor 系 RPC の e2e テスト"""

import json
import shutil
import socket
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any

import pytest

from hisui_server import HisuiServer, reserve_ephemeral_port
from processor_metrics import ProcessorMetrics

_DEBUG_METRIC_NAMES = (
    "hisui_total_input_video_frame_count",
    "hisui_last_input_video_timestamp",
    "hisui_is_listening",
    "hisui_video_codec",
    "hisui_total_input_audio_data_count",
)


def _collect_processor_debug_metrics(
    metrics: list[dict[str, Any]],
    *,
    processor_id: str,
    processor_type: str,
) -> dict[str, list[dict[str, Any]]]:
    target_labels = {
        "processor_id": processor_id,
        "processor_type": processor_type,
    }
    result: dict[str, list[dict[str, Any]]] = {}

    for family in metrics:
        name = family.get("name")
        if name not in _DEBUG_METRIC_NAMES:
            continue
        samples = family.get("metrics")
        if not isinstance(samples, list):
            continue

        matched_samples: list[dict[str, Any]] = []
        for sample in samples:
            if not isinstance(sample, dict):
                continue
            labels = sample.get("labels")
            if not isinstance(labels, dict):
                continue
            if all(labels.get(k) == v for k, v in target_labels.items()):
                matched_samples.append(
                    {
                        "labels": labels,
                        "value": sample.get("value"),
                    }
                )

        if matched_samples:
            result[name] = matched_samples

    return result


def _run_ffmpeg_rtmp_publish(
    input_path: Path,
    publish_url: str,
    *,
    with_audio: bool,
) -> None:
    ffmpeg_path = shutil.which("ffmpeg")
    if ffmpeg_path is None:
        pytest.skip("ffmpeg is required for RTMP inbound endpoint test")

    deadline = time.time() + 10.0
    last_error = "(no attempt)"
    while time.time() < deadline:
        cmd = [
            ffmpeg_path,
            "-hide_banner",
            "-re",
            "-loglevel",
            "error",
            "-nostdin",
            "-i",
            str(input_path),
        ]
        if with_audio:
            cmd.extend(["-c", "copy"])
        else:
            cmd.extend(["-an", "-c:v", "copy"])
        cmd.extend(["-f", "flv", publish_url])

        publish_started_at = time.time()
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
        )
        elapsed = time.time() - publish_started_at
        if result.returncode == 0:
            return
        last_error = (
            f"returncode={result.returncode}, elapsed={elapsed:.3f}, stderr={result.stderr}"
        )
        time.sleep(0.2)

    raise AssertionError(
        f"ffmpeg failed: {last_error}"
    )


def _run_ffmpeg_srt_publish(
    input_path: Path,
    publish_url: str,
    *,
    with_audio: bool,
) -> None:
    ffmpeg_path = shutil.which("ffmpeg")
    if ffmpeg_path is None:
        pytest.skip("ffmpeg is required for SRT inbound endpoint test")

    deadline = time.time() + 10.0
    while time.time() < deadline:
        cmd = [
            ffmpeg_path,
            "-hide_banner",
            "-re",
            "-loglevel",
            "error",
            "-nostdin",
            "-i",
            str(input_path),
        ]
        if with_audio:
            cmd.extend(["-c", "copy"])
        else:
            cmd.extend(["-an", "-c:v", "copy"])
        cmd.extend(["-f", "mpegts", publish_url])

        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            return
        time.sleep(0.2)

    raise AssertionError(
        f"ffmpeg failed: returncode={result.returncode}, stderr={result.stderr}"
    )


def _create_opus_mp4(input_path: Path, output_path: Path) -> None:
    ffmpeg_path = shutil.which("ffmpeg")
    if ffmpeg_path is None:
        pytest.skip("ffmpeg is required for Opus mp4 test input generation")

    cmd = [
        ffmpeg_path,
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-i",
        str(input_path),
        "-vn",
        "-c:a",
        "libopus",
        "-f",
        "mp4",
        str(output_path),
    ]
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        pytest.skip(
            "ffmpeg with Opus-in-mp4 support is required: "
            f"returncode={result.returncode}, stderr={result.stderr}"
        )


def _run_ffmpeg_srt_publish_once(
    input_path: Path,
    publish_url: str,
    *,
    with_audio: bool,
) -> subprocess.CompletedProcess[str]:
    ffmpeg_path = shutil.which("ffmpeg")
    if ffmpeg_path is None:
        pytest.skip("ffmpeg is required for SRT inbound endpoint test")

    cmd = [
        ffmpeg_path,
        "-hide_banner",
        "-re",
        "-loglevel",
        "error",
        "-nostdin",
        "-i",
        str(input_path),
    ]
    if with_audio:
        cmd.extend(["-c", "copy"])
    else:
        cmd.extend(["-an", "-c:v", "copy"])
    cmd.extend(["-f", "mpegts", publish_url])

    return subprocess.run(
        cmd,
        capture_output=True,
        text=True,
    )


def _wait_for_video_frame_count(
    server: HisuiServer,
    processor_id: str,
    expected_count: int,
    *,
    processor_type: str,
    allow_greater: bool = False,
    timeout: float = 10.0,
) -> int:
    deadline = time.time() + timeout
    last_debug_metrics: dict[str, list[dict[str, Any]]] = {}
    while time.time() < deadline:
        metrics_json = server.metrics_json()
        last_debug_metrics = _collect_processor_debug_metrics(
            metrics_json,
            processor_id=processor_id,
            processor_type=processor_type,
        )
        metrics = ProcessorMetrics(
            metrics_json,
            processor_id=processor_id,
            processor_type=processor_type,
        )
        try:
            frame_count = int(metrics.value("hisui_total_input_video_frame_count"))
        except (AssertionError, ValueError):
            time.sleep(0.1)
            continue
        if frame_count == expected_count:
            return frame_count
        if frame_count > expected_count:
            if allow_greater:
                return frame_count
            debug_json = json.dumps(last_debug_metrics, ensure_ascii=False, sort_keys=True)
            print(
                f"DEBUG rtmp/srt processor metrics: processor_id={processor_id}, processor_type={processor_type}, metrics={debug_json}"
            )
            raise AssertionError(
                f"{processor_type} video frame count exceeded expected value: expected={expected_count}, actual={frame_count}, debug_metrics={debug_json}"
            )
        time.sleep(0.1)
    debug_json = json.dumps(last_debug_metrics, ensure_ascii=False, sort_keys=True)
    print(
        f"DEBUG rtmp/srt processor metrics: processor_id={processor_id}, processor_type={processor_type}, metrics={debug_json}"
    )
    raise AssertionError(
        f"{processor_type} did not reach expected video frame count in time: expected={expected_count}, debug_metrics={debug_json}"
    )


def _wait_for_processor_listening(
    server: HisuiServer,
    *,
    processor_id: str,
    processor_type: str,
    timeout: float = 10.0,
) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            listening = ProcessorMetrics(
                server.metrics_json(),
                processor_id=processor_id,
                processor_type=processor_type,
            ).value("hisui_is_listening")
            if listening == "1":
                return
        except AssertionError:
            pass
        time.sleep(0.1)
    raise AssertionError(
        f"processor did not become listening in time: processor_id={processor_id}, processor_type={processor_type}"
    )


def _last_video_timestamp_seconds(
    server: HisuiServer,
    *,
    processor_id: str,
    processor_type: str,
) -> float:
    return float(
        ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type=processor_type,
        ).value("hisui_last_input_video_timestamp")
    )


def _wait_for_tcp_listen(port: int, timeout: float = 10.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        # 接続プローブは listen モードの ffmpeg に副作用を与えるため、
        # bind 可否で待受開始（ポート占有）を確認する。
        probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            probe.bind(("127.0.0.1", port))
        except OSError:
            probe.close()
            return
        probe.close()
        time.sleep(0.05)
    raise AssertionError(f"RTMP listener did not start within timeout: port={port}")


def _start_ffmpeg_rtmp_receive(
    receive_url: str,
    output_path: Path,
    *,
    with_audio: bool,
    max_video_frames: int | None,
    listen: bool = False,
    timeout_seconds: int | None = None,
    startup_timeout: float = 10.0,
) -> subprocess.Popen[str]:
    ffmpeg_path = shutil.which("ffmpeg")
    if ffmpeg_path is None:
        pytest.skip("ffmpeg is required for RTMP outbound endpoint test")

    cmd = [
        ffmpeg_path,
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
    ]
    if listen:
        cmd.extend(["-listen", "1"])
    if timeout_seconds is not None:
        cmd.extend(["-timeout", str(timeout_seconds)])
    cmd.extend([
        "-i",
        receive_url,
    ])
    if max_video_frames is not None:
        cmd.extend(["-frames:v", str(max_video_frames)])
    if not with_audio:
        cmd.append("-an")
    cmd.extend([
        "-c",
        "copy",
        "-f",
        "mp4",
        str(output_path),
    ])

    if listen:
        return subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

    deadline = time.time() + startup_timeout
    last_stderr = ""
    while time.time() < deadline:
        process = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        time.sleep(0.2)
        if process.poll() is None:
            return process
        stdout, stderr = process.communicate(timeout=5)
        last_stderr = f"stdout={stdout}, stderr={stderr}"
        time.sleep(0.1)

    raise AssertionError(
        f"failed to start ffmpeg receiver within timeout: url={receive_url}, details={last_stderr}"
    )


def _wait_process_exit(process: subprocess.Popen[str], timeout: float) -> tuple[str, str]:
    try:
        return_code = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired as e:
        process.kill()
        stdout, stderr = process.communicate(timeout=5)
        raise AssertionError(
            f"process timed out: timeout={timeout}, stdout={stdout}, stderr={stderr}"
        ) from e

    stdout, stderr = process.communicate(timeout=5)
    assert return_code == 0, (
        f"process exited with non-zero code: returncode={return_code}, stdout={stdout}, stderr={stderr}"
    )
    return stdout, stderr


def _inspect_mp4(binary_path: Path, path: Path) -> dict[str, Any]:
    result = subprocess.run(
        [str(binary_path), "inspect", str(path)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, (
        f"hisui inspect failed: returncode={result.returncode}, stderr={result.stderr}"
    )
    output = json.loads(result.stdout)
    assert isinstance(output, dict), "inspect output must be a JSON object"
    return output


def test_create_mp4_video_reader_and_compare_stats(binary_path: Path):
    """createMp4VideoReader で生成した processor の統計値を確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-av1.mp4"
    )
    processor_id = "e2e-mp4-video-reader"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createMp4VideoReader",
            {
                "path": str(input_path),
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        assert (
            server.wait_processor_terminated(
                processor_id,
                timeout=10.0,
            )
            == processor_id
        )

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="mp4_video_reader",
        )

        assert metrics.value("hisui_total_sample_count") == "25"
        assert float(metrics.value("hisui_total_track_seconds")) == pytest.approx(1.0)
        assert metrics.value("hisui_codec", value="AV1") == "1"


def test_create_mp4_audio_reader_and_compare_stats(binary_path: Path):
    """createMp4AudioReader で生成した processor の統計値を確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "red-320x320-h264-aac.mp4"
    )
    processor_id = "e2e-mp4-audio-reader"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createMp4AudioReader",
            {
                "path": str(input_path),
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        assert (
            server.wait_processor_terminated(
                processor_id,
                timeout=10.0,
            )
            == processor_id
        )

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="mp4_audio_reader",
        )
        assert int(metrics.value("hisui_total_sample_count")) > 0
        assert float(metrics.value("hisui_total_track_seconds")) > 0.0
        assert metrics.value("hisui_codec", value="AAC") == "1"


def test_create_video_decoder_from_mp4_video_reader_and_compare_stats(binary_path: Path):
    """Mp4VideoReader -> VideoDecoder の統計値を確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-av1.mp4"
    )
    reader_processor_id = "e2e-mp4-video-reader-for-decoder"
    decoder_processor_id = "e2e-video-decoder"
    decoded_video_track_id = "e2e-decoded-video-track"

    with HisuiServer(binary_path, manual_start_trigger=True) as server:
        create_reader_response = server.rpc_call(
            "createMp4VideoReader",
            {
                "path": str(input_path),
                "processorId": reader_processor_id,
            },
        )
        assert create_reader_response["result"]["processorId"] == reader_processor_id

        create_decoder_response = server.rpc_call(
            "createVideoDecoder",
            {
                "inputTrackId": reader_processor_id,
                "outputTrackId": decoded_video_track_id,
                "processorId": decoder_processor_id,
            },
        )
        assert create_decoder_response["result"]["processorId"] == decoder_processor_id

        start_response = server.trigger_start()
        assert start_response["result"]["started"] is True

        assert (
            server.wait_processor_terminated(
                reader_processor_id,
                timeout=10.0,
            )
            == reader_processor_id
        )
        assert (
            server.wait_processor_terminated(
                decoder_processor_id,
                timeout=10.0,
            )
            == decoder_processor_id
        )

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=decoder_processor_id,
            processor_type="video_decoder",
        )
        assert metrics.value("hisui_total_input_video_frame_count") == "25"
        assert metrics.value("hisui_total_output_video_frame_count") == "25"
        assert metrics.value("hisui_codec", value="AV1") == "1"


def test_create_audio_decoder_from_mp4_audio_reader_and_compare_stats(binary_path: Path):
    """Mp4AudioReader -> AudioDecoder の統計値を確認する"""
    src_input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "red-320x320-h264-aac.mp4"
    )
    reader_processor_id = "e2e-mp4-audio-reader-for-decoder"
    decoder_processor_id = "e2e-audio-decoder"
    decoded_audio_track_id = "e2e-decoded-audio-track"

    with tempfile.TemporaryDirectory() as tmp_dir:
        input_path = Path(tmp_dir) / "audio-opus.mp4"
        _create_opus_mp4(src_input_path, input_path)

        with HisuiServer(binary_path, manual_start_trigger=True) as server:
            create_reader_response = server.rpc_call(
                "createMp4AudioReader",
                {
                    "path": str(input_path),
                    "processorId": reader_processor_id,
                },
            )
            assert create_reader_response["result"]["processorId"] == reader_processor_id

            create_decoder_response = server.rpc_call(
                "createAudioDecoder",
                {
                    "inputTrackId": reader_processor_id,
                    "outputTrackId": decoded_audio_track_id,
                    "processorId": decoder_processor_id,
                },
            )
            assert create_decoder_response["result"]["processorId"] == decoder_processor_id

            start_response = server.trigger_start()
            assert start_response["result"]["started"] is True

            assert (
                server.wait_processor_terminated(
                    reader_processor_id,
                    timeout=10.0,
                )
                == reader_processor_id
            )
            assert (
                server.wait_processor_terminated(
                    decoder_processor_id,
                    timeout=10.0,
                )
                == decoder_processor_id
            )

            metrics = ProcessorMetrics(
                server.metrics_json(),
                processor_id=decoder_processor_id,
                processor_type="audio_decoder",
            )
            reader_metrics = ProcessorMetrics(
                server.metrics_json(),
                processor_id=reader_processor_id,
                processor_type="mp4_audio_reader",
            )
            assert reader_metrics.value("hisui_codec", value="OPUS") == "1"
            assert metrics.value("hisui_total_audio_data_count") == reader_metrics.value(
                "hisui_total_sample_count"
            )
            assert metrics.value("hisui_codec", value="OPUS") == "1"
            assert metrics.value("hisui_engine", value="opus") == "1"


def test_create_rtmp_inbound_endpoint_and_compare_stats(binary_path: Path):
    """createRtmpInboundEndpoint で受信した映像の統計値を確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-h264.mp4"
    )
    processor_id = "e2e-rtmp-inbound-endpoint"
    output_video_track_id = "e2e-rtmp-video-track"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_url = f"rtmp://127.0.0.1:{port}/live"
    publish_url = f"{input_url}/stream-main"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createRtmpInboundEndpoint",
            {
                "inputUrl": input_url,
                "streamName": "stream-main",
                "outputVideoTrackId": output_video_track_id,
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        _wait_for_processor_listening(
            server,
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )
        _run_ffmpeg_rtmp_publish(
            input_path,
            publish_url,
            with_audio=False,
        )
        frame_count = _wait_for_video_frame_count(
            server,
            processor_id,
            expected_count=24,
            processor_type="rtmp_inbound_endpoint",
            allow_greater=True,
        )

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )
        assert metrics.value("hisui_video_codec", value="H264") == "1"
        # CI 環境では ffmpeg -> RTMP 送信の終端タイミング差で 1 フレーム欠けることがある。
        assert 24 <= frame_count <= 25


def test_create_rtmp_inbound_endpoint_with_audio_video_and_compare_stats(
    binary_path: Path,
):
    """createRtmpInboundEndpoint で受信した映像 + 音声の統計値を確認する"""
    av_input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "red-320x320-h264-aac.mp4"
    )
    processor_id = "e2e-rtmp-inbound-endpoint-av"
    output_video_track_id = "e2e-rtmp-video-track-av"
    output_audio_track_id = "e2e-rtmp-audio-track-av"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_url = f"rtmp://127.0.0.1:{port}/live"
    publish_url = f"{input_url}/stream-main"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createRtmpInboundEndpoint",
            {
                "inputUrl": input_url,
                "streamName": "stream-main",
                "outputVideoTrackId": output_video_track_id,
                "outputAudioTrackId": output_audio_track_id,
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        _wait_for_processor_listening(
            server,
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )
        _run_ffmpeg_rtmp_publish(
            av_input_path,
            publish_url,
            with_audio=True,
        )
        video_count = _wait_for_video_frame_count(
            server,
            processor_id,
            expected_count=25,
            processor_type="rtmp_inbound_endpoint",
        )

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )
        audio_count = int(metrics.value("hisui_total_input_audio_data_count"))
        assert metrics.value("hisui_video_codec", value="H264") == "1"
        assert metrics.value("hisui_audio_codec", value="AAC") == "1"
        assert video_count == 25
        # ffmpeg の終了待機後でも、RTMP / FLV の終端処理タイミング差で
        # 音声カウントが 43 - 45 付近で揺れることがあるため、下限チェックにしている。
        assert audio_count >= 43


def test_create_rtmp_inbound_endpoint_reconnect_keeps_live_timestamp_progress(
    binary_path: Path,
):
    """createRtmpInboundEndpoint で再接続後も timestamp が進み続けることを確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-h264.mp4"
    )
    processor_id = "e2e-rtmp-inbound-endpoint-reconnect"
    output_video_track_id = "e2e-rtmp-video-track-reconnect"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_url = f"rtmp://127.0.0.1:{port}/live"
    publish_url = f"{input_url}/stream-main"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createRtmpInboundEndpoint",
            {
                "inputUrl": input_url,
                "streamName": "stream-main",
                "outputVideoTrackId": output_video_track_id,
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        _wait_for_processor_listening(
            server,
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )
        _run_ffmpeg_rtmp_publish(
            input_path,
            publish_url,
            with_audio=False,
        )
        first_count = _wait_for_video_frame_count(
            server,
            processor_id,
            expected_count=24,
            processor_type="rtmp_inbound_endpoint",
            allow_greater=True,
        )
        # CI 環境では ffmpeg -> RTMP 送信の終端タイミング差で 1 フレーム欠けることがある。
        assert 24 <= first_count <= 25
        first_last_ts = _last_video_timestamp_seconds(
            server,
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )

        time.sleep(1.0)
        _run_ffmpeg_rtmp_publish(
            input_path,
            publish_url,
            with_audio=False,
        )
        second_count = _wait_for_video_frame_count(
            server,
            processor_id,
            expected_count=first_count + 23,
            processor_type="rtmp_inbound_endpoint",
            allow_greater=True,
        )
        # 2 回の配信でそれぞれ最大 1 フレーム欠ける可能性を許容する。
        assert second_count >= first_count + 23
        second_last_ts = _last_video_timestamp_seconds(
            server,
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )

        assert second_last_ts > first_last_ts + 0.5


def test_create_srt_inbound_endpoint_and_compare_stats(binary_path: Path):
    """createSrtInboundEndpoint で受信した映像の統計値を確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-h264.mp4"
    )
    processor_id = "e2e-srt-inbound-endpoint"
    output_video_track_id = "e2e-srt-video-track"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_url = f"srt://127.0.0.1:{port}?mode=listener"
    publish_url = f"srt://127.0.0.1:{port}?mode=caller"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createSrtInboundEndpoint",
            {
                "inputUrl": input_url,
                "outputVideoTrackId": output_video_track_id,
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        _wait_for_processor_listening(
            server,
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )
        _run_ffmpeg_srt_publish(
            input_path,
            publish_url,
            with_audio=False,
        )
        frame_count = _wait_for_video_frame_count(
            server,
            processor_id,
            expected_count=25,
            processor_type="srt_inbound_endpoint",
        )

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )
        assert metrics.value("hisui_video_codec", value="H264") == "1"
        assert frame_count == 25


def test_create_srt_inbound_endpoint_with_audio_video_and_compare_stats(
    binary_path: Path,
):
    """createSrtInboundEndpoint で受信した映像 + 音声の統計値を確認する"""
    av_input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "red-320x320-h264-aac.mp4"
    )
    processor_id = "e2e-srt-inbound-endpoint-av"
    output_video_track_id = "e2e-srt-video-track-av"
    output_audio_track_id = "e2e-srt-audio-track-av"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_url = f"srt://127.0.0.1:{port}?mode=listener"
    publish_url = f"srt://127.0.0.1:{port}?mode=caller"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createSrtInboundEndpoint",
            {
                "inputUrl": input_url,
                "outputVideoTrackId": output_video_track_id,
                "outputAudioTrackId": output_audio_track_id,
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        _wait_for_processor_listening(
            server,
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )
        _run_ffmpeg_srt_publish(
            av_input_path,
            publish_url,
            with_audio=True,
        )
        video_count = _wait_for_video_frame_count(
            server,
            processor_id,
            expected_count=25,
            processor_type="srt_inbound_endpoint",
        )

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )
        audio_count = int(metrics.value("hisui_total_input_audio_data_count"))
        assert metrics.value("hisui_video_codec", value="H264") == "1"
        assert metrics.value("hisui_audio_codec", value="AAC") == "1"
        assert video_count == 25
        # ffmpeg の終了待機後でも、SRT / MPEG-TS の終端処理タイミング差で
        # 音声カウントが大きく揺れることがあるため、下限チェックにしている。
        assert audio_count >= 20


def test_create_srt_inbound_endpoint_reconnect_keeps_live_timestamp_progress(
    binary_path: Path,
):
    """createSrtInboundEndpoint で再接続後も timestamp が進み続けることを確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-h264.mp4"
    )
    processor_id = "e2e-srt-inbound-endpoint-reconnect"
    output_video_track_id = "e2e-srt-video-track-reconnect"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_url = f"srt://127.0.0.1:{port}"
    publish_url = f"srt://127.0.0.1:{port}?mode=caller"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createSrtInboundEndpoint",
            {
                "inputUrl": input_url,
                "outputVideoTrackId": output_video_track_id,
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        _wait_for_processor_listening(
            server,
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )
        _run_ffmpeg_srt_publish(
            input_path,
            publish_url,
            with_audio=False,
        )
        assert (
            _wait_for_video_frame_count(
                server,
                processor_id,
                expected_count=25,
                processor_type="srt_inbound_endpoint",
            )
            == 25
        )
        first_last_ts = _last_video_timestamp_seconds(
            server,
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )

        time.sleep(1.0)
        _run_ffmpeg_srt_publish(
            input_path,
            publish_url,
            with_audio=False,
        )
        assert (
            _wait_for_video_frame_count(
                server,
                processor_id,
                expected_count=50,
                processor_type="srt_inbound_endpoint",
            )
            == 50
        )
        second_last_ts = _last_video_timestamp_seconds(
            server,
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )

        assert second_last_ts > first_last_ts + 0.5


def test_create_srt_inbound_endpoint_with_stream_id_and_compare_stats(binary_path: Path):
    """createSrtInboundEndpoint で streamId を照合して受信した映像の統計値を確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-h264.mp4"
    )
    processor_id = "e2e-srt-inbound-endpoint-stream-id"
    output_video_track_id = "e2e-srt-video-track-stream-id"
    stream_id = "e2e-stream-main"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_url = f"srt://127.0.0.1:{port}"
    publish_url = f"srt://127.0.0.1:{port}?mode=caller&streamid={stream_id}"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createSrtInboundEndpoint",
            {
                "inputUrl": input_url,
                "outputVideoTrackId": output_video_track_id,
                "streamId": stream_id,
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        _wait_for_processor_listening(
            server,
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )
        _run_ffmpeg_srt_publish(
            input_path,
            publish_url,
            with_audio=False,
        )
        frame_count = _wait_for_video_frame_count(
            server,
            processor_id,
            expected_count=25,
            processor_type="srt_inbound_endpoint",
        )

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )
        assert metrics.value("hisui_video_codec", value="H264") == "1"
        assert frame_count == 25


def test_create_srt_inbound_endpoint_rejects_mismatched_stream_id(binary_path: Path):
    """createSrtInboundEndpoint で streamId が不一致の場合に接続を拒否する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-h264.mp4"
    )
    processor_id = "e2e-srt-inbound-endpoint-stream-id-mismatch"
    output_video_track_id = "e2e-srt-video-track-stream-id-mismatch"
    expected_stream_id = "e2e-stream-expected"
    actual_stream_id = "e2e-stream-actual"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_url = f"srt://127.0.0.1:{port}"
    publish_url = f"srt://127.0.0.1:{port}?mode=caller&streamid={actual_stream_id}"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createSrtInboundEndpoint",
            {
                "inputUrl": input_url,
                "outputVideoTrackId": output_video_track_id,
                "streamId": expected_stream_id,
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        _wait_for_processor_listening(
            server,
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )
        _run_ffmpeg_srt_publish_once(
            input_path,
            publish_url,
            with_audio=False,
        )

        with pytest.raises(
            AssertionError,
            match="did not reach expected video frame count in time",
        ):
            _wait_for_video_frame_count(
                server,
                processor_id,
                expected_count=25,
                processor_type="srt_inbound_endpoint",
                timeout=3.0,
            )


def test_create_srt_inbound_endpoint_rejects_missing_stream_id(binary_path: Path):
    """createSrtInboundEndpoint で streamId が未設定の peer 接続を拒否する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-h264.mp4"
    )
    processor_id = "e2e-srt-inbound-endpoint-stream-id-missing"
    output_video_track_id = "e2e-srt-video-track-stream-id-missing"
    expected_stream_id = "e2e-stream-required"
    port, sock = reserve_ephemeral_port()
    sock.close()
    input_url = f"srt://127.0.0.1:{port}"
    publish_url = f"srt://127.0.0.1:{port}?mode=caller"

    with HisuiServer(binary_path) as server:
        create_response = server.rpc_call(
            "createSrtInboundEndpoint",
            {
                "inputUrl": input_url,
                "outputVideoTrackId": output_video_track_id,
                "streamId": expected_stream_id,
                "processorId": processor_id,
            },
        )
        assert create_response["result"]["processorId"] == processor_id

        _wait_for_processor_listening(
            server,
            processor_id=processor_id,
            processor_type="srt_inbound_endpoint",
        )
        result = _run_ffmpeg_srt_publish_once(
            input_path,
            publish_url,
            with_audio=False,
        )
        assert result.returncode != 0

        with pytest.raises(
            AssertionError,
            match="did not reach expected video frame count in time",
        ):
            _wait_for_video_frame_count(
                server,
                processor_id,
                expected_count=25,
                processor_type="srt_inbound_endpoint",
                timeout=3.0,
            )


def test_create_rtmp_outbound_endpoint_with_mp4_video_reader_and_inspect_output(
    binary_path: Path,
):
    """createRtmpOutboundEndpoint で配信した映像を受信し inspect で確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-h264.mp4"
    )
    reader_processor_id = "e2e-mp4-video-reader-for-rtmp-outbound"
    outbound_processor_id = "e2e-rtmp-outbound-endpoint"
    port, sock = reserve_ephemeral_port()
    sock.close()
    output_url = f"rtmp://127.0.0.1:{port}/live"
    receive_url = f"{output_url}/stream-main"

    with tempfile.TemporaryDirectory() as tmp_dir:
        output_path = Path(tmp_dir) / "received.mp4"

        with HisuiServer(binary_path, manual_start_trigger=True) as server:
            create_reader_response = server.rpc_call(
                "createMp4VideoReader",
                {
                    "path": str(input_path),
                    "processorId": reader_processor_id,
                },
            )
            assert create_reader_response["result"]["processorId"] == reader_processor_id

            create_outbound_response = server.rpc_call(
                "createRtmpOutboundEndpoint",
                {
                    "outputUrl": output_url,
                    "streamName": "stream-main",
                    "inputVideoTrackId": reader_processor_id,
                    "processorId": outbound_processor_id,
                },
            )
            assert create_outbound_response["result"]["processorId"] == outbound_processor_id

            _wait_for_tcp_listen(port)
            ffmpeg_process = _start_ffmpeg_rtmp_receive(
                receive_url,
                output_path,
                with_audio=False,
                max_video_frames=25,
            )
            try:
                start_response = server.trigger_start()
                assert start_response["result"]["started"] is True

                assert (
                    server.wait_processor_terminated(
                        reader_processor_id,
                        timeout=10.0,
                    )
                    == reader_processor_id
                )

                _wait_process_exit(ffmpeg_process, timeout=20.0)
            finally:
                if ffmpeg_process.poll() is None:
                    ffmpeg_process.kill()
                    ffmpeg_process.communicate(timeout=5)

        assert output_path.exists(), "RTMP received output file must exist"
        assert output_path.stat().st_size > 0, "RTMP received output file must not be empty"

        inspect_output = _inspect_mp4(binary_path, output_path)
        assert inspect_output["format"] == "mp4"
        assert inspect_output["video_codec"] == "H264"
        assert inspect_output["video_sample_count"] == 25
        assert len(inspect_output["video_samples"]) == 25


def test_create_rtmp_outbound_endpoint_with_mp4_audio_video_readers_and_inspect_output(
    binary_path: Path,
):
    """createRtmpOutboundEndpoint で配信した映像 + 音声を受信し inspect で確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "red-320x320-h264-aac.mp4"
    )
    video_reader_processor_id = "e2e-mp4-video-reader-for-rtmp-outbound-av"
    audio_reader_processor_id = "e2e-mp4-audio-reader-for-rtmp-outbound-av"
    outbound_processor_id = "e2e-rtmp-outbound-endpoint-av"
    port, sock = reserve_ephemeral_port()
    sock.close()
    output_url = f"rtmp://127.0.0.1:{port}/live"
    receive_url = f"{output_url}/stream-main"

    with tempfile.TemporaryDirectory() as tmp_dir:
        output_path = Path(tmp_dir) / "received-av.mp4"

        with HisuiServer(binary_path, manual_start_trigger=True) as server:
            create_video_reader_response = server.rpc_call(
                "createMp4VideoReader",
                {
                    "path": str(input_path),
                    "processorId": video_reader_processor_id,
                },
            )
            assert (
                create_video_reader_response["result"]["processorId"]
                == video_reader_processor_id
            )

            create_audio_reader_response = server.rpc_call(
                "createMp4AudioReader",
                {
                    "path": str(input_path),
                    "processorId": audio_reader_processor_id,
                },
            )
            assert (
                create_audio_reader_response["result"]["processorId"]
                == audio_reader_processor_id
            )

            create_outbound_response = server.rpc_call(
                "createRtmpOutboundEndpoint",
                {
                    "outputUrl": output_url,
                    "streamName": "stream-main",
                    "inputVideoTrackId": video_reader_processor_id,
                    "inputAudioTrackId": audio_reader_processor_id,
                    "processorId": outbound_processor_id,
                },
            )
            assert create_outbound_response["result"]["processorId"] == outbound_processor_id

            _wait_for_tcp_listen(port)
            ffmpeg_process = _start_ffmpeg_rtmp_receive(
                receive_url,
                output_path,
                with_audio=True,
                max_video_frames=None,
            )
            try:
                start_response = server.trigger_start()
                assert start_response["result"]["started"] is True

                assert (
                    server.wait_processor_terminated(
                        video_reader_processor_id,
                        timeout=10.0,
                    )
                    == video_reader_processor_id
                )

                assert (
                    server.wait_processor_terminated(
                        audio_reader_processor_id,
                        timeout=10.0,
                    )
                    == audio_reader_processor_id
                )

                assert (
                    server.wait_processor_terminated(
                        outbound_processor_id,
                        timeout=10.0,
                    )
                    == outbound_processor_id
                )

                _wait_process_exit(ffmpeg_process, timeout=20.0)
            finally:
                if ffmpeg_process.poll() is None:
                    ffmpeg_process.kill()
                    ffmpeg_process.communicate(timeout=5)

        assert output_path.exists(), "RTMP received output file must exist"
        assert output_path.stat().st_size > 0, "RTMP received output file must not be empty"

        inspect_output = _inspect_mp4(binary_path, output_path)
        assert inspect_output["format"] == "mp4"
        assert inspect_output["video_codec"] == "H264"
        assert inspect_output["audio_codec"] == "AAC"
        assert inspect_output["video_sample_count"] == 25
        assert inspect_output["audio_sample_count"] == 45


def test_create_rtmp_publisher_with_mp4_video_reader_and_inspect_output(
    binary_path: Path,
):
    """createRtmpPublisher で配信した映像を受信し inspect で確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "archive-red-320x320-h264.mp4"
    )
    reader_processor_id = "e2e-mp4-video-reader-for-rtmp-publisher"
    publisher_processor_id = "e2e-rtmp-publisher"
    port, sock = reserve_ephemeral_port()
    sock.close()
    output_url = f"rtmp://127.0.0.1:{port}/live"
    receive_url = f"{output_url}/stream-main"

    with tempfile.TemporaryDirectory() as tmp_dir:
        output_path = Path(tmp_dir) / "publisher-received.mp4"

        with HisuiServer(binary_path, manual_start_trigger=True) as server:
            ffmpeg_process = _start_ffmpeg_rtmp_receive(
                receive_url,
                output_path,
                with_audio=False,
                max_video_frames=25,
                listen=True,
                timeout_seconds=20,
            )
            try:
                _wait_for_tcp_listen(port)

                create_reader_response = server.rpc_call(
                    "createMp4VideoReader",
                    {
                        "path": str(input_path),
                        "processorId": reader_processor_id,
                    },
                )
                assert create_reader_response["result"]["processorId"] == reader_processor_id

                create_publisher_response = server.rpc_call(
                    "createRtmpPublisher",
                    {
                        "outputUrl": output_url,
                        "streamName": "stream-main",
                        "inputVideoTrackId": reader_processor_id,
                        "processorId": publisher_processor_id,
                    },
                )
                assert (
                    create_publisher_response["result"]["processorId"]
                    == publisher_processor_id
                )
                start_response = server.trigger_start()
                assert start_response["result"]["started"] is True

                assert (
                    server.wait_processor_terminated(
                        reader_processor_id,
                        timeout=10.0,
                    )
                    == reader_processor_id
                )

                assert (
                    server.wait_processor_terminated(
                        publisher_processor_id,
                        timeout=10.0,
                    )
                    == publisher_processor_id
                )

                _wait_process_exit(ffmpeg_process, timeout=20.0)
            finally:
                if ffmpeg_process.poll() is None:
                    ffmpeg_process.kill()
                    ffmpeg_process.communicate(timeout=5)

        assert output_path.exists(), "RTMP received output file must exist"
        assert output_path.stat().st_size > 0, "RTMP received output file must not be empty"

        inspect_output = _inspect_mp4(binary_path, output_path)
        assert inspect_output["format"] == "mp4"
        assert inspect_output["video_codec"] == "H264"
        assert inspect_output["video_sample_count"] == 25


def test_create_rtmp_publisher_with_mp4_audio_video_readers_and_inspect_output(
    binary_path: Path,
):
    """createRtmpPublisher で配信した映像 + 音声を受信し inspect で確認する"""
    input_path = (
        Path(__file__).resolve().parents[2]
        / "testdata"
        / "red-320x320-h264-aac.mp4"
    )
    video_reader_processor_id = "e2e-mp4-video-reader-for-rtmp-publisher-av"
    audio_reader_processor_id = "e2e-mp4-audio-reader-for-rtmp-publisher-av"
    publisher_processor_id = "e2e-rtmp-publisher-av"
    port, sock = reserve_ephemeral_port()
    sock.close()
    output_url = f"rtmp://127.0.0.1:{port}/live"
    receive_url = f"{output_url}/stream-main"

    with tempfile.TemporaryDirectory() as tmp_dir:
        output_path = Path(tmp_dir) / "publisher-received-av.mp4"

        with HisuiServer(binary_path, manual_start_trigger=True) as server:
            ffmpeg_process = _start_ffmpeg_rtmp_receive(
                receive_url,
                output_path,
                with_audio=True,
                max_video_frames=None,
                listen=True,
                timeout_seconds=20,
            )
            try:
                _wait_for_tcp_listen(port)

                create_video_reader_response = server.rpc_call(
                    "createMp4VideoReader",
                    {
                        "path": str(input_path),
                        "processorId": video_reader_processor_id,
                    },
                )
                assert (
                    create_video_reader_response["result"]["processorId"]
                    == video_reader_processor_id
                )

                create_audio_reader_response = server.rpc_call(
                    "createMp4AudioReader",
                    {
                        "path": str(input_path),
                        "processorId": audio_reader_processor_id,
                    },
                )
                assert (
                    create_audio_reader_response["result"]["processorId"]
                    == audio_reader_processor_id
                )

                create_publisher_response = server.rpc_call(
                    "createRtmpPublisher",
                    {
                        "outputUrl": output_url,
                        "streamName": "stream-main",
                        "inputVideoTrackId": video_reader_processor_id,
                        "inputAudioTrackId": audio_reader_processor_id,
                        "processorId": publisher_processor_id,
                    },
                )
                assert (
                    create_publisher_response["result"]["processorId"]
                    == publisher_processor_id
                )
                start_response = server.trigger_start()
                assert start_response["result"]["started"] is True

                assert (
                    server.wait_processor_terminated(
                        video_reader_processor_id,
                        timeout=10.0,
                    )
                    == video_reader_processor_id
                )

                assert (
                    server.wait_processor_terminated(
                        audio_reader_processor_id,
                        timeout=10.0,
                    )
                    == audio_reader_processor_id
                )

                assert (
                    server.wait_processor_terminated(
                        publisher_processor_id,
                        timeout=10.0,
                    )
                    == publisher_processor_id
                )

                _wait_process_exit(ffmpeg_process, timeout=20.0)
            finally:
                if ffmpeg_process.poll() is None:
                    ffmpeg_process.kill()
                    ffmpeg_process.communicate(timeout=5)

        assert output_path.exists(), "RTMP received output file must exist"
        assert output_path.stat().st_size > 0, "RTMP received output file must not be empty"

        inspect_output = _inspect_mp4(binary_path, output_path)
        assert inspect_output["format"] == "mp4"
        assert inspect_output["video_codec"] == "H264"
        assert inspect_output["audio_codec"] == "AAC"
        assert inspect_output["video_sample_count"] == 25
        assert inspect_output["audio_sample_count"] == 45
