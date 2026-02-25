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
    while time.time() < deadline:
        cmd = [
            ffmpeg_path,
            "-hide_banner",
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


def _wait_for_video_frame_count(
    server: HisuiServer,
    processor_id: str,
    expected_count: int,
    *,
    processor_type: str = "rtmp_inbound_endpoint",
    timeout: float = 10.0,
) -> int:
    deadline = time.time() + timeout
    while time.time() < deadline:
        metrics = ProcessorMetrics(
            server.metrics_json(),
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
            raise AssertionError(
                f"{processor_type} video frame count exceeded expected value: expected={expected_count}, actual={frame_count}"
            )
        time.sleep(0.1)
    raise AssertionError(
        f"{processor_type} did not reach expected video frame count in time: expected={expected_count}"
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

    return subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
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


def _wait_for_server_log_contains(server: HisuiServer, pattern: str, timeout: float = 10.0) -> None:
    if server.log_file is None:
        raise AssertionError("server.log_file must exist")

    deadline = time.time() + timeout
    while time.time() < deadline:
        if server.log_file.exists() and pattern in server.log_file.read_text():
            return
        time.sleep(0.1)
    log_content = server.log_file.read_text() if server.log_file.exists() else "(no log)"
    raise AssertionError(f"pattern not found in server log: pattern={pattern}, log={log_content}")


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

        _run_ffmpeg_rtmp_publish(
            input_path,
            publish_url,
            with_audio=False,
        )
        frame_count = _wait_for_video_frame_count(
            server,
            processor_id,
            expected_count=25,
        )

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )
        assert metrics.value("hisui_video_codec", value="H264") == "1"
        assert frame_count == 25


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

        _run_ffmpeg_rtmp_publish(
            av_input_path,
            publish_url,
            with_audio=True,
        )
        video_count = _wait_for_video_frame_count(
            server,
            processor_id,
            expected_count=25,
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
        # 音声カウントが 43 - 45 付近で揺れることがあるため、下限チェックにしている。
        assert audio_count >= 43


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

        _run_ffmpeg_srt_publish(
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
        _wait_for_server_log_contains(server, "SRT peer stream id mismatch", timeout=3.0)


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

        _run_ffmpeg_srt_publish(
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
        _wait_for_server_log_contains(server, "SRT peer stream id mismatch", timeout=3.0)


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
                _wait_for_server_log_contains(server, "Client started playing stream")
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
                _wait_for_server_log_contains(server, "Client started playing stream")
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
                _wait_for_server_log_contains(server, "StateChanged(Publishing)")
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
                _wait_for_server_log_contains(server, "StateChanged(Publishing)")
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
