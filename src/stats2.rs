use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, AtomicU64, Ordering},
};

#[derive(Debug, Default, Clone)]
pub struct Stats {
    shared_entries: Arc<Mutex<BTreeMap<StatsKey, StatsEntry>>>,
    // `Stats` を clone した後にどちらかで `set_default_label()` を呼ぶと、
    // `Arc` を差し替えるため、もう片方には影響しない。
    default_labels: Arc<Labels>,
    // 同一 `Stats` インスタンス内での再取得時にロックを減らすためのキャッシュ。
    entry_cache: BTreeMap<StatsKey, StatsEntry>,
}

impl Stats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_default_label(&mut self, name: &'static str, value: &str) {
        let mut labels = (*self.default_labels).clone();
        labels.0.insert(name, value.to_owned());
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

    pub fn string(&mut self, name: &'static str, label_key: &'static str) -> StatsString {
        let key = self.make_key(name);
        let entry =
            self.get_or_insert_entry(key, || StatsEntry::StringValue(StatsString::new(label_key)));
        match entry {
            StatsEntry::StringValue(string_value) => {
                if string_value.label_key() != label_key {
                    panic!(
                        "metric label_key mismatch: expected={label_key} actual={}",
                        string_value.label_key()
                    );
                }
                string_value
            }
            other => panic!(
                "metric type mismatch: expected=string actual={}",
                other.kind_name()
            ),
        }
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
    default_labels: Arc<Labels>,
}

#[derive(Debug, Clone)]
pub enum StatsEntry {
    Counter(StatsCounter),
    Gauge(StatsGauge),
    StringValue(StatsString),
}

impl StatsEntry {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Counter(_) => "counter",
            Self::Gauge(_) => "gauge",
            Self::StringValue(_) => "string",
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

    pub fn set(&self, value: u64) {
        self.value.store(value, Ordering::Relaxed);
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
        self.add(1);
    }

    pub fn dec(&self) {
        self.sub(1);
    }

    pub fn add(&self, value: i64) {
        self.value.fetch_add(value, Ordering::Relaxed);
    }

    pub fn sub(&self, value: i64) {
        self.value.fetch_sub(value, Ordering::Relaxed);
    }

    pub fn set(&self, value: i64) {
        self.value.store(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone)]
pub struct StatsString {
    string_value: Arc<Mutex<String>>,
    value_label_key: &'static str,
}

impl StatsString {
    pub fn new(label_key: &'static str) -> Self {
        Self {
            string_value: Arc::new(Mutex::new(String::new())),
            value_label_key: label_key,
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

    pub fn label_key(&self) -> &'static str {
        self.value_label_key
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Labels(BTreeMap<&'static str, String>);

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
        counter.set(10);
        assert_eq!(counter.get(), 10);
    }

    #[test]
    fn gauge_basic_ops() {
        let gauge = StatsGauge::new();
        assert_eq!(gauge.get(), 0);
        gauge.inc();
        gauge.add(5);
        gauge.dec();
        gauge.sub(2);
        assert_eq!(gauge.get(), 3);
        gauge.set(-4);
        assert_eq!(gauge.get(), -4);
    }

    #[test]
    fn string_basic_ops() {
        let string_value = StatsString::new("state");
        assert_eq!(string_value.label_key(), "state");
        assert_eq!(string_value.get(), "");
        string_value.set("running");
        assert_eq!(string_value.get(), "running");
        string_value.clear();
        assert_eq!(string_value.get(), "");
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
}
