"""hisui server processor 系 RPC の e2e テスト"""

from pathlib import Path
from typing import Any

import pytest

from hisui_server import HisuiServer


def _find_metric_sample(
    metrics: list[dict[str, Any]],
    metric_name: str,
    required_labels: dict[str, str],
) -> dict[str, Any] | None:
    for family in metrics:
        if family.get("name") != metric_name:
            continue
        samples = family.get("metrics")
        if not isinstance(samples, list):
            continue
        for sample in samples:
            if not isinstance(sample, dict):
                continue
            labels = sample.get("labels")
            if not isinstance(labels, dict):
                continue
            if all(labels.get(k) == v for k, v in required_labels.items()):
                return sample
    return None


def test_create_mp4_video_reader_and_compare_stats(binary_path: Path):
    """createMp4VideoReader で生成した processor の統計値を確認する"""
    input_path = Path("../testdata/archive-red-320x320-av1.mp4").resolve()
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
        assert wait_response["result"]["terminated"] is True

        metrics = server.metrics_json()
        base_labels = {
            "processor_id": processor_id,
            "processor_type": "mp4_video_reader",
        }

        total_sample_count = _find_metric_sample(
            metrics,
            "hisui_total_sample_count",
            base_labels,
        )
        assert total_sample_count is not None
        assert total_sample_count.get("value") == "25"

        total_track_seconds = _find_metric_sample(
            metrics,
            "hisui_total_track_seconds",
            base_labels,
        )
        assert total_track_seconds is not None
        assert float(str(total_track_seconds.get("value"))) == pytest.approx(1.0)

        codec_sample = _find_metric_sample(
            metrics,
            "hisui_codec",
            {
                **base_labels,
                "value": "AV1",
            },
        )
        assert codec_sample is not None
        assert codec_sample.get("value") == "1"
