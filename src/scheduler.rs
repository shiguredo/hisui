use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::mpsc;

use orfail::OrFail;

use crate::media::{MediaSample, MediaStreamId};
use crate::processor::{BoxedMediaProcessor, MediaProcessor};

const CHANNEL_SIZE_LIMIT: usize = 5; // TODO:

#[derive(Debug)]
pub struct Task {
    processor: BoxedMediaProcessor,
    input_stream_rxs: HashMap<MediaStreamId, mpsc::Receiver<MediaSample>>,
    input_stream_txs: HashMap<MediaStreamId, mpsc::SyncSender<MediaSample>>,
    output_stream_txs: HashMap<MediaStreamId, mpsc::SyncSender<MediaSample>>,
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
            output_stream_txs: HashMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct Scheduler {
    tasks: Vec<Task>,
    thread_count: NonZeroUsize,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            thread_count: NonZeroUsize::MIN,
        }
    }

    pub fn register<P>(&mut self, processor: P)
    where
        P: 'static + Send + MediaProcessor,
    {
        self.tasks.push(Task::new(processor));
    }

    pub fn run(mut self) -> orfail::Result<()> {
        self.update_output_stream_txs().or_fail()?;
        Ok(())
    }

    fn update_output_stream_txs(&mut self) -> orfail::Result<()> {
        Ok(())
    }
}
