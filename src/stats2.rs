use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, atomic::AtomicU64};

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub entries: Arc<Mutex<BTreeMap<String, StatsEntry>>>,
    pub local_common_labels: Labels,
    pub local_entries: BTreeMap<String, StatsEntry>, // cache?
}

impl Stats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_local_common_label(&mut self, name: &str, value: &str) {
        self.local_common_labels.insert(name, value);
    }
}

#[derive(Debug, Clone)]
pub enum StatsEntry {
    Counter(StatsCounter),
}

#[derive(Debug, Clone)]
pub struct StatsCounter {
    pub labels: Labels,
    pub value: Arc<AtomicU64>,
}

#[derive(Debug, Default, Clone)]
pub struct Labels(BTreeMap<String, String>);

impl Labels {
    fn insert(&mut self, name: &str, value: &str) {
        self.0.insert(name.to_owned(), value.to_owned());
    }
}
