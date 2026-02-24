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


def _metric_value(
    metrics: list[dict[str, Any]],
    metric_name: str,
    required_labels: dict[str, str],
) -> str:
    sample = _find_metric_sample(metrics, metric_name, required_labels)
    assert sample is not None, (
        f"metric sample not found: metric_name={metric_name}, labels={required_labels}"
    )
    value = sample.get("value")
    assert isinstance(value, str), (
        f"metric value must be string: metric_name={metric_name}, labels={required_labels}"
    )
    return value


class ProcessorMetrics:
    """processor 固有のラベルを保持してメトリクス参照を簡略化する"""

    def __init__(
        self,
        metrics: list[dict[str, Any]],
        *,
        processor_id: str,
        processor_type: str,
    ):
        self._metrics = metrics
        self._base_labels = {
            "processor_id": processor_id,
            "processor_type": processor_type,
        }

    def value(self, metric_name: str, **labels: str) -> str:
        return _metric_value(
            metric_name=metric_name,
            metrics=self._metrics,
            required_labels={**self._base_labels, **labels},
        )


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

        processor_metrics = ProcessorMetrics(
            server.metrics_json(),
            processor_id=processor_id,
            processor_type="mp4_video_reader",
        )

        assert processor_metrics.value("hisui_total_sample_count") == "25"
        assert float(processor_metrics.value("hisui_total_track_seconds")) == pytest.approx(
            1.0
        )
        assert processor_metrics.value("hisui_codec", value="AV1") == "1"
