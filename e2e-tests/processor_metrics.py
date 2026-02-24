"""processor 系メトリクス参照ヘルパー"""

from typing import Any


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
