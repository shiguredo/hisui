"""hisui server processor 系 RPC の e2e テスト"""

from pathlib import Path

import pytest

from hisui_server import HisuiServer
from processor_metrics import ProcessorMetrics


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
