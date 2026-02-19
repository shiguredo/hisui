use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyWorkerThreadStats {
    pub total_processing_seconds: f64,
    pub total_waiting_seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
enum LegacyJsonValue {
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    String(String),
}

impl nojson::DisplayJson for LegacyJsonValue {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::Bool(v) => f.value(*v),
            Self::Unsigned(v) => f.value(*v),
            Self::Signed(v) => f.value(*v),
            Self::Float(v) => f.value(*v),
            Self::String(v) => f.value(v),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct LegacyProcessorStats {
    processor_type: String,
    error: bool,
    values: BTreeMap<String, LegacyJsonValue>,
}

impl LegacyProcessorStats {
    fn new() -> Self {
        Self {
            processor_type: "unknown".to_owned(),
            error: false,
            values: BTreeMap::new(),
        }
    }
}

impl nojson::DisplayJson for LegacyProcessorStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", &self.processor_type)?;
            for (name, value) in &self.values {
                f.member(name, value)?;
            }
            f.member("error", self.error)?;
            Ok(())
        })
    }
}

impl nojson::DisplayJson for LegacyWorkerThreadStats {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("total_processing_seconds", self.total_processing_seconds)?;
            f.member("total_waiting_seconds", self.total_waiting_seconds)?;
            Ok(())
        })
    }
}

struct LegacyStatsJson {
    elapsed_seconds: f64,
    error: bool,
    processors: Vec<LegacyProcessorStats>,
    worker_threads: Vec<LegacyWorkerThreadStats>,
}

impl nojson::DisplayJson for LegacyStatsJson {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("elapsed_seconds", self.elapsed_seconds)?;
            f.member("error", self.error)?;
            f.member("processors", &self.processors)?;
            f.member("worker_threads", &self.worker_threads)?;
            Ok(())
        })
    }
}

pub fn to_legacy_stats_json(
    stats: &crate::stats2::Stats,
    elapsed_seconds: f64,
    worker_threads: Vec<LegacyWorkerThreadStats>,
) -> crate::Result<nojson::RawJsonOwned> {
    let mut processors = BTreeMap::<String, LegacyProcessorStats>::new();
    for entry in stats.snapshot_entries()? {
        let Some(processor_id) = entry.labels.get("processor_id") else {
            continue;
        };
        let processor = processors
            .entry(processor_id.clone())
            .or_insert_with(LegacyProcessorStats::new);
        if let Some(processor_type) = entry.labels.get("processor_type") {
            processor.processor_type = processor_type.clone();
        }

        // 互換 JSON は processor 単位のフラットな値を前提にしているため、
        // processor_id / processor_type 以外のラベルを持つ指標は一旦除外する。
        if entry
            .labels
            .iter()
            .any(|(key, _)| !matches!(key, "processor_id" | "processor_type"))
        {
            continue;
        }

        if entry.metric_name == "error" {
            processor.error = snapshot_value_as_bool(&entry.value);
            continue;
        }
        processor.values.insert(
            entry.metric_name.to_owned(),
            snapshot_value_to_legacy_value(entry.value),
        );
    }

    let processors = processors.into_values().collect::<Vec<_>>();
    let stats = LegacyStatsJson {
        elapsed_seconds,
        error: processors.iter().any(|p| p.error),
        processors,
        worker_threads,
    };
    let json = nojson::json(|f| f.value(&stats));
    Ok(nojson::RawJsonOwned::parse(json.to_string()).expect("infallible"))
}

fn snapshot_value_to_legacy_value(value: crate::stats2::StatsSnapshotValue) -> LegacyJsonValue {
    match value {
        crate::stats2::StatsSnapshotValue::Counter(v) => LegacyJsonValue::Unsigned(v),
        crate::stats2::StatsSnapshotValue::Gauge(v) => LegacyJsonValue::Signed(v),
        crate::stats2::StatsSnapshotValue::GaugeF64(v) => LegacyJsonValue::Float(v),
        crate::stats2::StatsSnapshotValue::Flag(v) => LegacyJsonValue::Bool(v),
        crate::stats2::StatsSnapshotValue::String(v) => LegacyJsonValue::String(v),
    }
}

fn snapshot_value_as_bool(value: &crate::stats2::StatsSnapshotValue) -> bool {
    match value {
        crate::stats2::StatsSnapshotValue::Flag(v) => *v,
        crate::stats2::StatsSnapshotValue::Counter(v) => *v != 0,
        crate::stats2::StatsSnapshotValue::Gauge(v) => *v != 0,
        crate::stats2::StatsSnapshotValue::GaugeF64(v) => *v != 0.0,
        crate::stats2::StatsSnapshotValue::String(v) => !v.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_legacy_stats_json_excludes_worker_thread_processors_and_groups_by_processor() {
        let mut stats = crate::stats2::Stats::new();
        stats.set_default_label("processor_id", "reader0");
        stats.set_default_label("processor_type", "mp4_reader");
        stats.counter("total_input_video_sample_count").set(5);
        stats.flag("error").set(false);
        stats.gauge_f64("total_processing_seconds").set(1.5);

        stats.set_default_label("processor_id", "decoder0");
        stats.set_default_label("processor_type", "video_decoder");
        stats.counter("total_output_video_frame_count").set(4);
        stats.flag("error").set(true);

        let json = to_legacy_stats_json(
            &stats,
            3.0,
            vec![LegacyWorkerThreadStats {
                total_processing_seconds: 2.0,
                total_waiting_seconds: 1.0,
            }],
        )
        .expect("to_legacy_stats_json must succeed");

        let text = json.to_string();
        assert!(text.contains("\"elapsed_seconds\":3"));
        assert!(text.contains("\"error\":true"));
        assert!(text.contains("\"type\":\"mp4_reader\""));
        assert!(text.contains("\"type\":\"video_decoder\""));
        assert!(text.contains("\"total_input_video_sample_count\":5"));
        assert!(text.contains("\"total_output_video_frame_count\":4"));
        assert!(text.contains("\"total_processing_seconds\":1.5"));
        assert!(text.contains("\"worker_threads\":[{\"total_processing_seconds\":2"));
        assert!(!text.contains("\"processors\":[0"));
    }

    #[test]
    fn to_legacy_stats_json_skips_metrics_with_extra_labels() {
        let mut stats = crate::stats2::Stats::new();
        stats.set_default_label("processor_id", "mixer0");
        stats.set_default_label("processor_type", "video_mixer");
        stats.set_default_label("track_id", "video-main");
        stats.counter("frames_total").set(10);
        stats.flag("error").set(false);

        let json = to_legacy_stats_json(&stats, 0.0, Vec::new())
            .expect("to_legacy_stats_json must succeed");
        let text = json.to_string();
        assert!(!text.contains("\"frames_total\":10"));
        assert!(text.contains("\"type\":\"video_mixer\""));
        assert!(text.contains("\"error\":false"));
    }
}
