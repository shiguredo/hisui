"""hisui server processor 系 RPC の e2e テスト"""

import shutil
import subprocess
import time
from pathlib import Path

import pytest

from hisui_server import HisuiServer, reserve_ephemeral_port
from processor_metrics import ProcessorMetrics


def _run_ffmpeg_rtmp_publish(input_path: Path, publish_url: str) -> None:
    ffmpeg_path = shutil.which("ffmpeg")
    if ffmpeg_path is None:
        pytest.skip("ffmpeg is required for RTMP inbound endpoint test")

    deadline = time.time() + 10.0
    while time.time() < deadline:
        result = subprocess.run(
            [
                ffmpeg_path,
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-i",
                str(input_path),
                "-an",
                "-c:v",
                "copy",
                "-f",
                "flv",
                publish_url,
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            return
        time.sleep(0.2)

    raise AssertionError(
        f"ffmpeg failed: returncode={result.returncode}, stderr={result.stderr}"
    )


def _run_ffmpeg_rtmp_publish_audio_video(input_path: Path, publish_url: str) -> None:
    ffmpeg_path = shutil.which("ffmpeg")
    if ffmpeg_path is None:
        pytest.skip("ffmpeg is required for RTMP inbound endpoint test")

    deadline = time.time() + 10.0
    while time.time() < deadline:
        result = subprocess.run(
            [
                ffmpeg_path,
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-i",
                str(input_path),
                "-c",
                "copy",
                "-f",
                "flv",
                publish_url,
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            return
        time.sleep(0.2)

    raise AssertionError(
        f"ffmpeg failed: returncode={result.returncode}, stderr={result.stderr}"
    )


def _wait_for_video_frame_count(server: HisuiServer, processor_id: str) -> int:
    deadline = time.time() + 10.0
    while time.time() < deadline:
        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )
        try:
            frame_count = int(metrics.value("hisui_total_input_video_frame_count"))
        except (AssertionError, ValueError):
            time.sleep(0.1)
            continue
        if frame_count >= 1:
            return frame_count
        time.sleep(0.1)
    raise AssertionError("RTMP inbound endpoint did not receive video frames in time")


def _wait_for_audio_and_video_data_count(server: HisuiServer, processor_id: str) -> tuple[int, int]:
    deadline = time.time() + 10.0
    while time.time() < deadline:
        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )
        try:
            video_count = int(metrics.value("hisui_total_input_video_frame_count"))
            audio_count = int(metrics.value("hisui_total_input_audio_data_count"))
        except (AssertionError, ValueError):
            time.sleep(0.1)
            continue
        if video_count >= 1 and audio_count >= 1:
            return video_count, audio_count
        time.sleep(0.1)
    raise AssertionError("RTMP inbound endpoint did not receive audio/video data in time")


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

        wait_response = server.rpc_call(
            "waitProcessorTerminated",
            {
                "processorId": processor_id,
            },
            timeout=10.0,
        )
        assert wait_response["result"]["processorId"] == processor_id

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

        _run_ffmpeg_rtmp_publish(input_path, publish_url)
        frame_count = _wait_for_video_frame_count(server, processor_id)

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )
        assert metrics.value("hisui_video_codec", value="H264") == "1"
        assert frame_count >= 1


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

        _run_ffmpeg_rtmp_publish_audio_video(av_input_path, publish_url)
        video_count, audio_count = _wait_for_audio_and_video_data_count(
            server,
            processor_id,
        )

        metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="rtmp_inbound_endpoint",
        )
        assert metrics.value("hisui_video_codec", value="H264") == "1"
        assert metrics.value("hisui_audio_codec", value="AAC") == "1"
        assert video_count >= 1
        assert audio_count >= 1
