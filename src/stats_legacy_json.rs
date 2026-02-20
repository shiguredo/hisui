use std::collections::BTreeMap;

#[derive(Debug, Clone)]
struct LegacyProcessorStats {
    processor_type: String,
    error: bool,
    values: BTreeMap<String, crate::stats::StatsValue>,
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

struct LegacyStatsJson {
    elapsed_seconds: f64,
    error: bool,
    processors: Vec<LegacyProcessorStats>,
    // TODO: compose の tokio 化後は、トップレベルに `tokio_metrics`
    // （例: `num_workers`, `num_alive_tasks`, `global_queue_depth`）
    // を追加できるようにする。
}

impl nojson::DisplayJson for LegacyStatsJson {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("elapsed_seconds", self.elapsed_seconds)?;
            f.member("error", self.error)?;
            f.member("processors", &self.processors)?;
            Ok(())
        })
    }
}

pub fn to_legacy_stats_json(
    stats: &crate::stats::Stats,
    elapsed_seconds: f64,
) -> crate::Result<nojson::RawJsonOwned> {
    let mut processors = BTreeMap::<String, LegacyProcessorStats>::new();
    for entry in stats.entries()? {
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
            processor.error = entry.value.as_bool_for_legacy();
            continue;
        }
        processor
            .values
            .insert(entry.metric_name.to_owned(), entry.value);
    }

    let processors = processors.into_values().collect::<Vec<_>>();
    let stats = LegacyStatsJson {
        elapsed_seconds,
        error: processors.iter().any(|p| p.error),
        processors,
    };
    let json = nojson::json(|f| f.value(&stats));
    Ok(nojson::RawJsonOwned::parse(json.to_string()).expect("infallible"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_legacy_stats_json_excludes_worker_thread_processors_and_groups_by_processor() {
        let mut stats = crate::stats::Stats::new();
        stats.set_default_label("processor_id", "reader0");
        stats.set_default_label("processor_type", "mp4_reader");
        stats.counter("total_input_video_sample_count").add(5);
        stats.flag("error").set(false);

        stats.set_default_label("processor_id", "decoder0");
        stats.set_default_label("processor_type", "video_decoder");
        stats.counter("total_output_video_frame_count").add(4);
        stats.flag("error").set(true);

        let json = to_legacy_stats_json(&stats, 3.0).expect("to_legacy_stats_json must succeed");

        let text = json.to_string();
        assert!(text.contains("\"elapsed_seconds\":3"));
        assert!(text.contains("\"error\":true"));
        assert!(text.contains("\"type\":\"mp4_reader\""));
        assert!(text.contains("\"type\":\"video_decoder\""));
        assert!(text.contains("\"total_input_video_sample_count\":5"));
        assert!(text.contains("\"total_output_video_frame_count\":4"));
        assert!(!text.contains("\"processors\":[0"));
    }

    #[test]
    fn to_legacy_stats_json_skips_metrics_with_extra_labels() {
        let mut stats = crate::stats::Stats::new();
        stats.set_default_label("processor_id", "mixer0");
        stats.set_default_label("processor_type", "video_mixer");
        stats.set_default_label("track_id", "video-main");
        stats.counter("frames_total").add(10);
        stats.flag("error").set(false);

        let json = to_legacy_stats_json(&stats, 0.0).expect("to_legacy_stats_json must succeed");
        let text = json.to_string();
        assert!(!text.contains("\"frames_total\":10"));
        assert!(text.contains("\"type\":\"video_mixer\""));
        assert!(text.contains("\"error\":false"));
    }
}
