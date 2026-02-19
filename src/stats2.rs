use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, atomic::AtomicU64};

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

    pub fn set_local_common_label(&mut self, name: &str, value: &str) {
        let mut labels = (*self.local_common_labels).clone();
        labels.0.insert(name.to_owned(), value.to_owned());
        self.local_common_labels = Arc::new(labels);
    }

    pub fn counter(&mut self, name: &'static str) -> &StatsEntry {
        let key = StatsKey {
            name,
            labels: self.local_common_labels.clone(),
        };
        if let Some(entry) = self.local_entries.get_mut(&key) {
            entry
        } else {
            let entries = self.entries.lock().expect("lock() failed unexpectedly");
            todo!()
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
}

#[derive(Debug, Clone)]
pub struct StatsCounter {
    pub value: Arc<AtomicU64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Labels(BTreeMap<String, String>);
