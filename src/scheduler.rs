use std::collections::HashMap;
use std::sync::mpsc;

use crate::media::{MediaSample, MediaStreamId};
use crate::processor::{BoxedMediaProcessor, MediaProcessor};

const CHANNEL_SIZE_LIMIT: usize = 5; // TODO:

#[derive(Debug)]
pub struct Task {
    processor: BoxedMediaProcessor,
    input_stream_rxs: HashMap<MediaStreamId, mpsc::Receiver<MediaSample>>,
    input_stream_txs: HashMap<MediaStreamId, mpsc::SyncSender<MediaSample>>,
}

impl Task {
    fn new<P>(processor: P) -> Self
    where
        P: 'static + Send + MediaProcessor,
    {
        let mut input_stream_rxs = HashMap::new();
        let mut input_stream_txs = HashMap::new();

        for input_stream_id in processor.spec().input_stream_ids {
            let (tx, rx) = mpsc::sync_channel(CHANNEL_SIZE_LIMIT);
            input_stream_rxs.insert(input_stream_id, rx);
            input_stream_txs.insert(input_stream_id, tx);
        }

        Self {
            processor: BoxedMediaProcessor::new(processor),
            input_stream_rxs,
            input_stream_txs,
        }
    }
}

#[derive(Debug)]
pub struct SchedulerBuilder {
    tasks: Vec<Task>,
}

impl SchedulerBuilder {
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    pub fn register<P>(&mut self, processor: P)
    where
        P: 'static + Send + MediaProcessor,
    {
        self.tasks.push(Task::new(processor));
    }
}

#[derive(Debug)]
pub struct Scheduler {}

impl Scheduler {}
