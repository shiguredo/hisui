use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
};

const PROMETHEUS_METRIC_PREFIX: &str = "hisui_";

#[derive(Debug, Default, Clone)]
pub struct Stats {
    shared_entries: Arc<Mutex<BTreeMap<StatsKey, StatsEntry>>>,
    // `Stats` を clone した後にどちらかで `set_default_label()` を呼ぶと、
    // `Arc` を差し替えるため、もう片方には影響しない。
    default_labels: Arc<StatsLabels>,
    // 同一 `Stats` インスタンス内での再取得時にロックを減らすためのキャッシュ。
    entry_cache: BTreeMap<StatsKey, StatsEntry>,
}

impl Stats {
    pub fn new() -> Self {
        Self::default()
    }

    // [NOTE]
    // default label は「これ以降に取得するメトリクスのキー」にだけ反映される。
    // すでに取得済みのメトリクス（counter()/gauge() 等の戻り値）は、以前のラベル集合のまま。
    pub fn set_default_label(&mut self, name: &'static str, value: &str) {
        let mut labels = (*self.default_labels).clone();
        labels.insert(name, value.to_owned());
        self.default_labels = Arc::new(labels);
    }

    pub fn counter(&mut self, name: &'static str) -> StatsCounter {
        let key = self.make_key(name);
        let entry = self.get_or_insert_entry(key, || StatsEntry::Counter(StatsCounter::new()));
        match entry {
            StatsEntry::Counter(counter) => counter,
            other => panic!(
                "metric type mismatch: expected=counter actual={}",
                other.kind_name()
            ),
        }
    }

    pub fn gauge(&mut self, name: &'static str) -> StatsGauge {
        let key = self.make_key(name);
        let entry = self.get_or_insert_entry(key, || StatsEntry::Gauge(StatsGauge::new()));
        match entry {
            StatsEntry::Gauge(gauge) => gauge,
            other => panic!(
                "metric type mismatch: expected=gauge actual={}",
                other.kind_name()
            ),
        }
    }

    pub fn gauge_f64(&mut self, name: &'static str) -> StatsGaugeF64 {
        let key = self.make_key(name);
        let entry = self.get_or_insert_entry(key, || StatsEntry::GaugeF64(StatsGaugeF64::new()));
        match entry {
            StatsEntry::GaugeF64(gauge) => gauge,
            other => panic!(
                "metric type mismatch: expected=gauge_f64 actual={}",
                other.kind_name()
            ),
        }
    }

    pub fn string(&mut self, name: &'static str) -> StatsString {
        let key = self.make_key(name);
        let entry = self.get_or_insert_entry(key, || StatsEntry::StringValue(StatsString::new()));
        match entry {
            StatsEntry::StringValue(string_value) => string_value,
            other => panic!(
                "metric type mismatch: expected=string actual={}",
                other.kind_name()
            ),
        }
    }

    pub fn flag(&mut self, name: &'static str) -> StatsFlag {
        let key = self.make_key(name);
        let entry = self.get_or_insert_entry(key, || StatsEntry::Flag(StatsFlag::new()));
        match entry {
            StatsEntry::Flag(flag) => flag,
            other => panic!(
                "metric type mismatch: expected=flag actual={}",
                other.kind_name()
            ),
        }
    }

    pub fn to_prometheus_text(&self) -> crate::Result<String> {
        let entries = {
            let shared_entries = self
                .shared_entries
                .lock()
                .map_err(|_| crate::Error::new("stats lock poisoned: shared_entries"))?;
            shared_entries
                .iter()
                .map(|(key, entry)| (key.clone(), entry.clone()))
                .collect::<Vec<_>>()
        };

        let mut text = String::new();
        let mut previous_metric_name: Option<String> = None;
        for (key, entry) in entries {
            validate_prometheus_metric_name(key.metric_name)?;
            let metric_name = format!("{PROMETHEUS_METRIC_PREFIX}{}", key.metric_name);
            // [NOTE]
            // 同じ metric_name に対して label が違うエントリの型不一致は、ここでは検出しない。
            // 先に出力した型で `# TYPE` を 1 回だけ出す挙動。
            if previous_metric_name.as_deref() != Some(metric_name.as_str()) {
                text.push_str("# TYPE ");
                text.push_str(&metric_name);
                text.push(' ');
                text.push_str(entry.prometheus_type_name());
                text.push('\n');
                previous_metric_name = Some(metric_name.clone());
            }

            text.push_str(&metric_name);
            let mut labels = (*key.default_labels).clone();
            if let StatsEntry::StringValue(string_value) = &entry {
                labels.insert("value", string_value.get());
            }
            append_prometheus_labels(&mut text, &labels)?;
            text.push(' ');
            text.push_str(&entry.prometheus_value_string());
            text.push('\n');
        }

        Ok(text)
    }

    pub fn snapshot_entries(&self) -> crate::Result<Vec<StatsSnapshotEntry>> {
        let entries = {
            let shared_entries = self
                .shared_entries
                .lock()
                .map_err(|_| crate::Error::new("stats lock poisoned: shared_entries"))?;
            shared_entries
                .iter()
                .map(|(key, entry)| (key.clone(), entry.clone()))
                .collect::<Vec<_>>()
        };

        Ok(entries
            .into_iter()
            .map(|(key, entry)| StatsSnapshotEntry {
                metric_name: key.metric_name,
                labels: (*key.default_labels).clone(),
                value: match entry {
                    StatsEntry::Counter(v) => StatsSnapshotValue::Counter(v.get()),
                    StatsEntry::Gauge(v) => StatsSnapshotValue::Gauge(v.get()),
                    StatsEntry::GaugeF64(v) => StatsSnapshotValue::GaugeF64(v.get()),
                    StatsEntry::Flag(v) => StatsSnapshotValue::Flag(v.get()),
                    StatsEntry::StringValue(v) => StatsSnapshotValue::String(v.get()),
                },
            })
            .collect())
    }

    fn make_key(&self, metric_name: &'static str) -> StatsKey {
        StatsKey {
            metric_name,
            default_labels: self.default_labels.clone(),
        }
    }

    fn get_or_insert_entry(
        &mut self,
        key: StatsKey,
        create: impl FnOnce() -> StatsEntry,
    ) -> StatsEntry {
        if let Some(entry) = self.entry_cache.get(&key) {
            return entry.clone();
        }
        let mut shared_entries = self
            .shared_entries
            .lock()
            .expect("lock() failed unexpectedly");
        let entry = shared_entries
            .entry(key.clone())
            .or_insert_with(create)
            .clone();
        self.entry_cache.insert(key, entry.clone());
        entry
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct StatsKey {
    metric_name: &'static str,
    default_labels: Arc<StatsLabels>,
}

#[derive(Debug, Clone)]
pub enum StatsEntry {
    Counter(StatsCounter),
    Gauge(StatsGauge),
    GaugeF64(StatsGaugeF64),
    Flag(StatsFlag),
    StringValue(StatsString),
}

impl StatsEntry {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Counter(_) => "counter",
            Self::Gauge(_) => "gauge",
            Self::GaugeF64(_) => "gauge_f64",
            Self::Flag(_) => "flag",
            Self::StringValue(_) => "string",
        }
    }

    fn prometheus_type_name(&self) -> &'static str {
        match self {
            Self::Counter(_) => "counter",
            Self::Gauge(_) => "gauge",
            Self::GaugeF64(_) => "gauge",
            Self::Flag(_) => "gauge",
            Self::StringValue(_) => "gauge",
        }
    }

    fn prometheus_value_string(&self) -> String {
        match self {
            Self::Counter(counter) => counter.get().to_string(),
            Self::Gauge(gauge) => gauge.get().to_string(),
            Self::GaugeF64(gauge) => gauge.get().to_string(),
            Self::Flag(flag) => {
                if flag.get() {
                    "1".to_owned()
                } else {
                    "0".to_owned()
                }
            }
            Self::StringValue(_) => "1".to_owned(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct StatsCounter {
    value: Arc<AtomicU64>,
}

impl StatsCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc(&self) {
        self.add(1);
    }

    pub fn add(&self, value: u64) {
        self.value.fetch_add(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Default, Clone)]
pub struct StatsGauge {
    value: Arc<AtomicI64>,
}

impl StatsGauge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec(&self) {
        self.value.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn set(&self, value: i64) {
        self.value.store(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Default, Clone)]
pub struct StatsGaugeF64 {
    value: Arc<AtomicU64>,
}

impl StatsGaugeF64 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, value: f64) {
        self.value.store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn get(&self) -> f64 {
        f64::from_bits(self.value.load(Ordering::Relaxed))
    }
}

#[derive(Debug, Clone)]
pub struct StatsString {
    string_value: Arc<Mutex<String>>,
}

impl Default for StatsString {
    fn default() -> Self {
        Self::new()
    }
}

impl StatsString {
    pub fn new() -> Self {
        Self {
            string_value: Arc::new(Mutex::new(String::new())),
        }
    }

    pub fn set(&self, value: impl Into<String>) {
        let mut v = self
            .string_value
            .lock()
            .expect("lock() failed unexpectedly");
        *v = value.into();
    }

    pub fn get(&self) -> String {
        self.string_value
            .lock()
            .expect("lock() failed unexpectedly")
            .clone()
    }

    pub fn clear(&self) {
        let mut v = self
            .string_value
            .lock()
            .expect("lock() failed unexpectedly");
        v.clear();
    }
}

#[derive(Debug, Default, Clone)]
pub struct StatsFlag {
    value: Arc<AtomicBool>,
}

impl StatsFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, value: bool) {
        self.value.store(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> bool {
        self.value.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatsLabels(BTreeMap<&'static str, String>);

impl StatsLabels {
    pub fn insert(&mut self, name: &'static str, value: impl Into<String>) {
        self.0.insert(name, value.into());
    }

    pub fn get(&self, name: &str) -> Option<&String> {
        self.0.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &String)> {
        self.0.iter().map(|(k, v)| (*k, v))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatsSnapshotEntry {
    pub metric_name: &'static str,
    pub labels: StatsLabels,
    pub value: StatsSnapshotValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StatsSnapshotValue {
    Counter(u64),
    Gauge(i64),
    GaugeF64(f64),
    Flag(bool),
    String(String),
}

fn escape_label_value(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

fn append_prometheus_labels(text: &mut String, labels: &StatsLabels) -> crate::Result<()> {
    if labels.is_empty() {
        return Ok(());
    }

    text.push('{');
    let mut first = true;
    for (name, value) in labels.iter() {
        validate_prometheus_label_name(name)?;
        if !first {
            text.push(',');
        }
        first = false;
        text.push_str(name);
        text.push_str("=\"");
        text.push_str(&escape_label_value(value));
        text.push('"');
    }
    text.push('}');
    Ok(())
}

fn validate_prometheus_metric_name(name: &str) -> crate::Result<()> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(crate::Error::new("invalid Prometheus metric name: empty"));
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == ':') {
        return Err(crate::Error::new(format!(
            "invalid Prometheus metric name: {name}"
        )));
    }
    if chars.any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == ':')) {
        return Err(crate::Error::new(format!(
            "invalid Prometheus metric name: {name}"
        )));
    }
    Ok(())
}

fn validate_prometheus_label_name(name: &str) -> crate::Result<()> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(crate::Error::new("invalid Prometheus label name: empty"));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(crate::Error::new(format!(
            "invalid Prometheus label name: {name}"
        )));
    }
    if chars.any(|c| !(c.is_ascii_alphanumeric() || c == '_')) {
        return Err(crate::Error::new(format!(
            "invalid Prometheus label name: {name}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_basic_ops() {
        let counter = StatsCounter::new();
        assert_eq!(counter.get(), 0);
        counter.inc();
        counter.add(4);
        assert_eq!(counter.get(), 5);
        counter.add(5);
        assert_eq!(counter.get(), 10);
    }

    #[test]
    fn gauge_basic_ops() {
        let gauge = StatsGauge::new();
        assert_eq!(gauge.get(), 0);
        gauge.inc();
        gauge.dec();
        assert_eq!(gauge.get(), 0);
        gauge.set(-4);
        assert_eq!(gauge.get(), -4);
    }

    #[test]
    fn gauge_f64_basic_ops() {
        let gauge = StatsGaugeF64::new();
        assert_eq!(gauge.get(), 0.0);
        gauge.set(3.25);
        assert_eq!(gauge.get(), 3.25);
    }

    #[test]
    fn string_basic_ops() {
        let string_value = StatsString::new();
        assert_eq!(string_value.get(), "");
        string_value.set("running");
        assert_eq!(string_value.get(), "running");
        string_value.clear();
        assert_eq!(string_value.get(), "");
    }

    #[test]
    fn stats_labels_basic_ops() {
        let mut labels = StatsLabels::default();
        assert!(labels.is_empty());
        labels.insert("processor_id", "p0");
        assert_eq!(labels.get("processor_id"), Some(&"p0".to_owned()));
        let collected = labels
            .iter()
            .map(|(name, value)| (name, value.clone()))
            .collect::<Vec<_>>();
        assert_eq!(collected, vec![("processor_id", "p0".to_owned())]);
        assert!(!labels.is_empty());
    }

    #[test]
    fn same_key_returns_shared_state() {
        let mut stats = Stats::new();
        let counter1 = stats.counter("processed_frames");
        let counter2 = stats.counter("processed_frames");
        counter1.add(7);
        assert_eq!(counter2.get(), 7);
    }

    #[test]
    fn different_default_labels_are_isolated() {
        let mut stats = Stats::new();
        stats.set_default_label("node", "a");
        let counter_a = stats.counter("requests");
        counter_a.inc();

        stats.set_default_label("node", "b");
        let counter_b = stats.counter("requests");
        counter_b.add(2);

        assert_eq!(counter_a.get(), 1);
        assert_eq!(counter_b.get(), 2);
    }

    #[test]
    fn set_default_label_overwrites_value() {
        let mut stats = Stats::new();
        stats.set_default_label("node", "a");
        let counter_a = stats.counter("requests");
        counter_a.inc();

        stats.set_default_label("node", "b");
        stats.set_default_label("node", "c");
        let counter_c = stats.counter("requests");
        counter_c.add(3);

        assert_eq!(counter_a.get(), 1);
        assert_eq!(counter_c.get(), 3);
    }

    #[test]
    fn clone_keeps_label_snapshot_semantics() {
        let mut stats1 = Stats::new();
        stats1.set_default_label("node", "a");
        let counter_a = stats1.counter("requests");

        let mut stats2 = stats1.clone();
        stats2.set_default_label("node", "b");
        let counter_b = stats2.counter("requests");

        counter_a.inc();
        counter_b.add(2);

        let counter_a_again = stats1.counter("requests");
        assert_eq!(counter_a_again.get(), 1);
        assert_eq!(counter_b.get(), 2);
    }

    #[test]
    fn type_mismatch_panics_with_clear_message() {
        let mut stats = Stats::new();
        let _ = stats.counter("requests");

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = stats.gauge("requests");
        }));
        assert!(panic.is_err());
        let message = panic_message(panic);
        assert!(
            message.contains("metric type mismatch: expected=gauge actual=counter"),
            "unexpected panic message: {message}"
        );
    }

    #[test]
    fn flag_basic_ops() {
        let flag = StatsFlag::new();
        assert!(!flag.get());
        flag.set(true);
        assert!(flag.get());
        flag.set(false);
        assert!(!flag.get());
    }

    #[test]
    fn prometheus_text_outputs_counter_gauge_and_flag() {
        let mut stats = Stats::new();
        stats.set_default_label("processor_id", "p0");
        let counter = stats.counter("processed_frames_total");
        let gauge = stats.gauge("queue_depth");
        let gauge_f64 = stats.gauge_f64("latency_seconds");
        let flag = stats.flag("error");
        counter.add(3);
        gauge.set(-1);
        gauge_f64.set(0.5);
        flag.set(true);

        let text = stats
            .to_prometheus_text()
            .expect("to_prometheus_text must succeed");
        assert!(text.contains("# TYPE hisui_processed_frames_total counter"));
        assert!(text.contains("# TYPE hisui_queue_depth gauge"));
        assert!(text.contains("# TYPE hisui_latency_seconds gauge"));
        assert!(text.contains("# TYPE hisui_error gauge"));
        assert!(text.contains("hisui_processed_frames_total{processor_id=\"p0\"} 3"));
        assert!(text.contains("hisui_queue_depth{processor_id=\"p0\"} -1"));
        assert!(text.contains("hisui_latency_seconds{processor_id=\"p0\"} 0.5"));
        assert!(text.contains("hisui_error{processor_id=\"p0\"} 1"));
    }

    #[test]
    fn prometheus_text_outputs_string_as_gauge_with_value_label() {
        let mut stats = Stats::new();
        stats.set_default_label("processor_id", "p0");
        let state = stats.string("state");
        state.set("running");
        let text = stats
            .to_prometheus_text()
            .expect("to_prometheus_text must succeed");
        assert!(text.contains("# TYPE hisui_state gauge"));
        assert!(text.contains("hisui_state{processor_id=\"p0\",value=\"running\"} 1"));
    }

    #[test]
    fn prometheus_text_escapes_label_value() {
        let mut stats = Stats::new();
        let state = stats.string("state");
        state.set("a\"b\\c\nd");
        let text = stats
            .to_prometheus_text()
            .expect("to_prometheus_text must succeed");
        assert!(text.contains("value=\"a\\\"b\\\\c\\nd\""));
    }

    #[test]
    fn prometheus_text_rejects_invalid_metric_name() {
        let mut stats = Stats::new();
        stats.counter("foo-bar").inc();
        let err = stats
            .to_prometheus_text()
            .expect_err("to_prometheus_text must fail");
        assert!(err.to_string().contains("invalid Prometheus metric name"));
    }

    #[test]
    fn prometheus_text_rejects_invalid_label_name() {
        let mut stats = Stats::new();
        stats.set_default_label("bad-label", "x");
        stats.counter("requests_total").inc();
        let err = stats
            .to_prometheus_text()
            .expect_err("to_prometheus_text must fail");
        assert!(err.to_string().contains("invalid Prometheus label name"));
    }

    fn panic_message(panic: std::result::Result<(), Box<dyn std::any::Any + Send>>) -> String {
        let panic = panic.expect_err("panic is expected");
        if let Some(message) = panic.downcast_ref::<String>() {
            return message.clone();
        }
        if let Some(message) = panic.downcast_ref::<&str>() {
            return message.to_string();
        }
        "<non-string panic>".to_string()
    }

    #[test]
    fn snapshot_entries_include_all_metric_types() {
        let mut stats = Stats::new();
        stats.set_default_label("processor_id", "p0");
        stats.counter("processed_total").add(10);
        stats.gauge("queue_depth").set(-3);
        stats.gauge_f64("latency_seconds").set(0.25);
        stats.flag("error").set(true);
        stats.string("state").set("running");

        let entries = stats
            .snapshot_entries()
            .expect("snapshot_entries must succeed");
        assert!(
            entries.iter().any(|e| {
                e.metric_name == "processed_total"
                    && e.labels.get("processor_id") == Some(&"p0".to_owned())
                    && e.value == StatsSnapshotValue::Counter(10)
            }),
            "counter entry is missing: {entries:?}"
        );
        assert!(
            entries.iter().any(|e| {
                e.metric_name == "queue_depth" && e.value == StatsSnapshotValue::Gauge(-3)
            }),
            "gauge entry is missing: {entries:?}"
        );
        assert!(
            entries.iter().any(|e| {
                e.metric_name == "latency_seconds" && e.value == StatsSnapshotValue::GaugeF64(0.25)
            }),
            "gauge_f64 entry is missing: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.metric_name == "error" && e.value == StatsSnapshotValue::Flag(true)),
            "flag entry is missing: {entries:?}"
        );
        assert!(
            entries.iter().any(|e| {
                e.metric_name == "state" && e.value == StatsSnapshotValue::String("running".into())
            }),
            "string entry is missing: {entries:?}"
        );
    }
}
