use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, AtomicU64},
};

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub entries: Arc<Mutex<BTreeMap<StatsKey, StatsEntry>>>,
    pub local_common_labels: Arc<Labels>,
    pub local_entries: BTreeMap<StatsKey, StatsEntry>, // cache?
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
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatsKey {
    pub name: &'static str,
    pub labels: Arc<Labels>,
}

#[derive(Debug, Clone)]
pub enum StatsEntry {
    Counter(StatsCounter),
    Guage(StatsGuage),
}

#[derive(Debug, Default, Clone)]
pub struct StatsCounter {
    pub value: Arc<AtomicU64>,
}

#[derive(Debug, Default, Clone)]
pub struct StatsGuage {
    pub value: Arc<AtomicI64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Labels(BTreeMap<&'static str, String>);
