use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, AtomicU64},
};

#[derive(Debug, Default, Clone)]
pub struct Stats {
    entries: Arc<Mutex<BTreeMap<StatsKey, StatsEntry>>>,
    local_common_labels: Arc<Labels>,
    local_entries: BTreeMap<StatsKey, StatsEntry>, // cache?
}

impl Stats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_local_common_label(&mut self, name: &'static str, value: &str) {
        let mut labels = (*self.local_common_labels).clone();
        labels.0.insert(name, value.to_owned());
        self.local_common_labels = Arc::new(labels);
    }

    pub fn counter(&mut self, name: &'static str) -> StatsCounter {
        loop {
            let key = StatsKey {
                name,
                labels: self.local_common_labels.clone(),
            };

            if let Some(StatsEntry::Counter(counter)) = self.local_entries.get(&key) {
                // ローカルにキャッシュが存在する
                return counter.clone();
            }

            // NOTE: キーが同じでも、種類が違うとエントリが上書きされてしまうけど、それは利用側の自己責任

            // キャッシュが存在しない場合には、必要に応じてグローバルで追加した上で、キャッシュにも登録する
            let mut entries = self.entries.lock().expect("lock() failed unexpectedly");
            let entry = entries
                .entry(key.clone())
                .or_insert_with(|| StatsEntry::Counter(StatsCounter::default()));
            self.local_entries.insert(key, entry.clone());
        }
    }

    pub fn guage(&mut self, name: &'static str) -> StatsGuage {
        loop {
            let key = StatsKey {
                name,
                labels: self.local_common_labels.clone(),
            };

            if let Some(StatsEntry::Guage(guage)) = self.local_entries.get(&key) {
                // ローカルにキャッシュが存在する
                return guage.clone();
            }

            // NOTE: キーが同じでも、種類が違うとエントリが上書きされてしまうけど、それは利用側の自己責任

            // キャッシュが存在しない場合には、必要に応じてグローバルで追加した上で、キャッシュにも登録する
            let mut entries = self.entries.lock().expect("lock() failed unexpectedly");
            let entry = entries
                .entry(key.clone())
                .or_insert_with(|| StatsEntry::Guage(StatsGuage::default()));
            self.local_entries.insert(key, entry.clone());
        }
    }

    pub fn string(&mut self, name: &'static str, label_key: &'static str) -> StatsString {
        loop {
            let key = StatsKey {
                name,
                labels: self.local_common_labels.clone(),
            };

            if let Some(StatsEntry::String(string)) = self.local_entries.get(&key) {
                // ローカルにキャッシュが存在する
                return string.clone();
            }

            // NOTE: キーが同じでも、種類が違うとエントリが上書きされてしまうけど、それは利用側の自己責任

            // キャッシュが存在しない場合には、必要に応じてグローバルで追加した上で、キャッシュにも登録する
            let mut entries = self.entries.lock().expect("lock() failed unexpectedly");
            let entry = entries
                .entry(key.clone())
                .or_insert_with(|| StatsEntry::String(StatsString::new(label_key)));
            self.local_entries.insert(key, entry.clone());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct StatsKey {
    name: &'static str,
    labels: Arc<Labels>,
}

#[derive(Debug, Clone)]
pub enum StatsEntry {
    Counter(StatsCounter),
    Guage(StatsGuage),
    String(StatsString),
}

#[derive(Debug, Default, Clone)]
pub struct StatsCounter {
    value: Arc<AtomicU64>,
}

#[derive(Debug, Default, Clone)]
pub struct StatsGuage {
    value: Arc<AtomicI64>,
}

#[derive(Debug, Clone)]
pub struct StatsString {
    value: Arc<Mutex<String>>,
    label_key: &'static str,
}

impl StatsString {
    pub fn new(label_key: &'static str) -> Self {
        Self {
            value: Arc::new(Mutex::new(String::new())),
            label_key,
        }
    }

    pub fn set(&self, value: impl Into<String>) {
        let mut v = self.value.lock().expect("lock() failed unexpectedly");
        *v = value.into();
    }

    pub fn get(&self) -> String {
        self.value
            .lock()
            .expect("lock() failed unexpectedly")
            .clone()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Labels(BTreeMap<&'static str, String>);
