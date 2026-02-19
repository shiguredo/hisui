use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
};
use std::time::Duration;

const PROMETHEUS_METRIC_PREFIX: &str = "hisui_";

#[derive(Debug, Default, Clone)]
pub struct Stats {
    shared_entries: Arc<Mutex<BTreeMap<StatsKey, StatsValue>>>,
    // `Stats` を clone した後にどちらかで `set_default_label()` を呼ぶと、
    // `Arc` を差し替えるため、もう片方には影響しない。
    default_labels: Arc<StatsLabels>,
    // 同一 `Stats` インスタンス内での再取得時にロックを減らすためのキャッシュ。
    entry_cache: BTreeMap<StatsKey, StatsValue>,
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
        let entry = self.get_or_insert_entry(key, || StatsValue::Counter(StatsCounter::new()));
        match entry {
            StatsValue::Counter(counter) => counter,
            other => panic!(
                "metric type mismatch: expected=counter actual={}",
                other.kind_name()
            ),
        }
    }

    pub fn gauge(&mut self, name: &'static str) -> StatsGauge {
        let key = self.make_key(name);
        let entry = self.get_or_insert_entry(key, || StatsValue::Gauge(StatsGauge::new()));
        match entry {
            StatsValue::Gauge(gauge) => gauge,
            other => panic!(
                "metric type mismatch: expected=gauge actual={}",
                other.kind_name()
            ),
        }
    }

    pub fn gauge_f64(&mut self, name: &'static str) -> StatsGaugeF64 {
        let key = self.make_key(name);
        let entry = self.get_or_insert_entry(key, || StatsValue::GaugeF64(StatsGaugeF64::new()));
        match entry {
            StatsValue::GaugeF64(gauge) => gauge,
            other => panic!(
                "metric type mismatch: expected=gauge_f64 actual={}",
                other.kind_name()
            ),
        }
    }

    pub fn duration(&mut self, name: &'static str) -> StatsDuration {
        let key = self.make_key(name);
        let entry = self.get_or_insert_entry(key, || StatsValue::Duration(StatsDuration::new()));
        match entry {
            StatsValue::Duration(duration) => duration,
            other => panic!(
                "metric type mismatch: expected=duration actual={}",
                other.kind_name()
            ),
        }
    }

    pub fn string(&mut self, name: &'static str) -> StatsString {
        let key = self.make_key(name);
        let entry = self.get_or_insert_entry(key, || StatsValue::StringValue(StatsString::new()));
        match entry {
            StatsValue::StringValue(string_value) => string_value,
            other => panic!(
                "metric type mismatch: expected=string actual={}",
                other.kind_name()
            ),
        }
    }

    pub fn flag(&mut self, name: &'static str) -> StatsFlag {
        let key = self.make_key(name);
        let entry = self.get_or_insert_entry(key, || StatsValue::Flag(StatsFlag::new()));
        match entry {
            StatsValue::Flag(flag) => flag,
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
            if let StatsValue::StringValue(string_value) = &entry {
                labels.insert("value", string_value.get());
            }
            append_prometheus_labels(&mut text, &labels)?;
            text.push(' ');
            text.push_str(&entry.prometheus_value_string());
            text.push('\n');
        }

        Ok(text)
    }

    pub fn entries(&self) -> crate::Result<Vec<StatsEntry>> {
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
            .map(|(key, entry)| StatsEntry {
                metric_name: key.metric_name,
                labels: (*key.default_labels).clone(),
                value: entry,
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
        create: impl FnOnce() -> StatsValue,
    ) -> StatsValue {
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
pub enum StatsValue {
    Counter(StatsCounter),
    Gauge(StatsGauge),
    GaugeF64(StatsGaugeF64),
    Duration(StatsDuration),
    Flag(StatsFlag),
    StringValue(StatsString),
}

impl StatsValue {
    pub fn as_counter(&self) -> Option<u64> {
        match self {
            Self::Counter(counter) => Some(counter.get()),
            _ => None,
        }
    }

    pub fn as_gauge(&self) -> Option<i64> {
        match self {
            Self::Gauge(gauge) => Some(gauge.get()),
            _ => None,
        }
    }

    pub fn as_gauge_f64(&self) -> Option<f64> {
        match self {
            Self::GaugeF64(gauge) => Some(gauge.get()),
            _ => None,
        }
    }

    pub fn as_duration(&self) -> Option<Duration> {
        match self {
            Self::Duration(duration) => Some(duration.get()),
            _ => None,
        }
    }

    pub fn as_flag(&self) -> Option<bool> {
        match self {
            Self::Flag(flag) => Some(flag.get()),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<String> {
        match self {
            Self::StringValue(string_value) => Some(string_value.get()),
            _ => None,
        }
    }

    pub fn as_numeric_f64(&self) -> Option<f64> {
        match self {
            Self::Counter(counter) => Some(counter.get() as f64),
            Self::Gauge(gauge) => Some(gauge.get() as f64),
            Self::GaugeF64(gauge) => Some(gauge.get()),
            Self::Duration(duration) => Some(duration.get().as_secs_f64()),
            Self::Flag(flag) => Some(if flag.get() { 1.0 } else { 0.0 }),
            Self::StringValue(_) => None,
        }
    }

    pub fn as_bool_for_legacy(&self) -> bool {
        match self {
            Self::Flag(flag) => flag.get(),
            Self::Counter(counter) => counter.get() != 0,
            Self::Gauge(gauge) => gauge.get() != 0,
            Self::GaugeF64(gauge) => gauge.get() != 0.0,
            Self::Duration(duration) => duration.get() != Duration::ZERO,
            Self::StringValue(string_value) => !string_value.get().is_empty(),
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            Self::Counter(_) => "counter",
            Self::Gauge(_) => "gauge",
            Self::GaugeF64(_) => "gauge_f64",
            Self::Duration(_) => "duration",
            Self::Flag(_) => "flag",
            Self::StringValue(_) => "string",
        }
    }

    fn prometheus_type_name(&self) -> &'static str {
        match self {
            Self::Counter(_) => "counter",
            Self::Gauge(_) => "gauge",
            Self::GaugeF64(_) => "gauge",
            Self::Duration(_) => "gauge",
            Self::Flag(_) => "gauge",
            Self::StringValue(_) => "gauge",
        }
    }

    fn prometheus_value_string(&self) -> String {
        match self {
            Self::Counter(counter) => counter.get().to_string(),
            Self::Gauge(gauge) => gauge.get().to_string(),
            Self::GaugeF64(gauge) => gauge.get().to_string(),
            Self::Duration(duration) => duration.get().as_secs_f64().to_string(),
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

impl nojson::DisplayJson for StatsValue {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::Counter(counter) => f.value(counter.get()),
            Self::Gauge(gauge) => f.value(gauge.get()),
            Self::GaugeF64(gauge) => f.value(gauge.get()),
            Self::Duration(duration) => f.value(duration.get().as_secs_f64()),
            Self::Flag(flag) => f.value(flag.get()),
            Self::StringValue(string_value) => {
                let value = string_value.get();
                f.value(&value)
            }
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

#[derive(Debug, Default, Clone)]
pub struct StatsDuration {
    value: Arc<AtomicU64>,
}

impl StatsDuration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&self, value: Duration) {
        self.value
            .fetch_add(value.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn set(&self, value: Duration) {
        self.value
            .store(value.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn get(&self) -> Duration {
        Duration::from_micros(self.value.load(Ordering::Relaxed))
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

#[derive(Debug, Default, Clone)]
pub struct SharedAtomicFlag(Arc<AtomicBool>);

impl SharedAtomicFlag {
    pub fn set(&self, value: bool) {
        self.0.store(value, Ordering::SeqCst);
    }

    pub fn get(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Default, Clone)]
pub struct SharedAtomicCounter(Arc<AtomicU64>);

impl SharedAtomicCounter {
    pub fn increment(&self) {
        self.add(1);
    }

    pub fn add(&self, value: u64) {
        self.0.fetch_add(value, Ordering::SeqCst);
    }

    pub fn set(&self, value: u64) {
        self.0.store(value, Ordering::SeqCst);
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl nojson::DisplayJson for SharedAtomicCounter {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.get())
    }
}

#[derive(Debug, Default, Clone)]
pub struct SharedAtomicDuration(SharedAtomicCounter);

impl SharedAtomicDuration {
    pub fn new(value: Duration) -> Self {
        let s = Self::default();
        s.set(value);
        s
    }

    pub fn add(&self, duration: Duration) {
        self.0.add(duration.as_micros() as u64)
    }

    pub fn set(&self, duration: Duration) {
        self.0.set(duration.as_micros() as u64);
    }

    pub fn get(&self) -> Duration {
        Duration::from_micros(self.0.get())
    }
}

impl nojson::DisplayJson for SharedAtomicDuration {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.get().as_secs_f64())
    }
}

#[derive(Debug, Default, Clone)]
pub struct SharedOption<T>(Arc<Mutex<Option<T>>>);

impl<T> SharedOption<T> {
    pub fn new(v: Option<T>) -> Self {
        Self(Arc::new(Mutex::new(v)))
    }

    pub fn get(&self) -> Option<T>
    where
        T: Clone,
    {
        self.0.lock().expect("lock() failed unexpectedly").clone()
    }

    pub fn set(&self, v: T) {
        *self.0.lock().expect("lock() failed unexpectedly") = Some(v);
    }

    pub fn clear(&self) {
        *self.0.lock().expect("lock() failed unexpectedly") = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VideoResolution {
    pub width: usize,
    pub height: usize,
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

#[derive(Debug, Clone)]
pub struct StatsEntry {
    pub metric_name: &'static str,
    pub labels: StatsLabels,
    pub value: StatsValue,
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
    fn duration_basic_ops() {
        let duration = StatsDuration::new();
        assert_eq!(duration.get(), Duration::ZERO);
        duration.set(Duration::from_millis(750));
        assert_eq!(duration.get(), Duration::from_millis(750));
        duration.add(Duration::from_millis(250));
        assert_eq!(duration.get(), Duration::from_secs(1));
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

        let mut cloned = stats1.clone();
        cloned.set_default_label("node", "b");
        let counter_b = cloned.counter("requests");

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
    fn entries_include_all_metric_types() {
        let mut stats = Stats::new();
        stats.set_default_label("processor_id", "p0");
        stats.counter("processed_total").add(10);
        stats.gauge("queue_depth").set(-3);
        stats.gauge_f64("latency_seconds").set(0.25);
        stats.duration("uptime").set(Duration::from_millis(1250));
        stats.flag("error").set(true);
        stats.string("state").set("running");

        let entries = stats.entries().expect("entries must succeed");
        assert!(
            entries.iter().any(|e| {
                e.metric_name == "processed_total"
                    && e.labels.get("processor_id") == Some(&"p0".to_owned())
                    && e.value.as_counter() == Some(10)
            }),
            "counter entry is missing: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.metric_name == "queue_depth" && e.value.as_gauge() == Some(-3)),
            "gauge entry is missing: {entries:?}"
        );
        assert!(
            entries.iter().any(|e| {
                e.metric_name == "latency_seconds" && e.value.as_gauge_f64() == Some(0.25)
            }),
            "gauge_f64 entry is missing: {entries:?}"
        );
        assert!(
            entries.iter().any(|e| {
                e.metric_name == "uptime"
                    && e.value.as_duration() == Some(Duration::from_millis(1250))
            }),
            "duration entry is missing: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.metric_name == "error" && e.value.as_flag() == Some(true)),
            "flag entry is missing: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.metric_name == "state" && e.value.as_string() == Some("running".into())),
            "string entry is missing: {entries:?}"
        );
    }
}
