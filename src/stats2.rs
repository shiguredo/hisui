use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, atomic::AtomicU64};

#[derive(Debug)]
pub struct Stats {
    pub entries: BTreeMap<String, StatsEntry>,
}

impl Stats {}

#[derive(Debug, Clone)]
pub struct LocalStats {
    pub global: Arc<Mutex<Stats>>,
    pub local_common_labels: Labels,
    pub local_entires: BTreeMap<String, StatsEntry>,
}

impl LocalStats {}

#[derive(Debug, Clone)]
pub enum StatsEntry {
    Counter(StatsCounter),
}

#[derive(Debug, Clone)]
pub struct StatsCounter {
    pub labels: Labels,
    pub value: Arc<AtomicU64>,
}

#[derive(Debug, Clone)]
pub struct Labels(pub Arc<Mutex<BTreeMap<String, String>>>);
