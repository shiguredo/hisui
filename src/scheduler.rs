use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::mpsc;

use orfail::OrFail;

use crate::media::{MediaSample, MediaStreamId};
use crate::processor::{
    BoxedMediaProcessor, MediaProcessor, MediaProcessorInput, MediaProcessorOutput,
};
use crate::stats::ProcessorStats;

const CHANNEL_SIZE_LIMIT: usize = 5; // TODO:

type MediaSampleReceiver = mpsc::Receiver<MediaSample>;
type MediaSampleSyncSender = mpsc::SyncSender<MediaSample>;

#[derive(Debug)]
pub struct Task {
    processor: BoxedMediaProcessor,
    input_stream_rxs: HashMap<MediaStreamId, MediaSampleReceiver>,
    output_stream_txs: HashMap<MediaStreamId, Vec<MediaSampleSyncSender>>,
    awaiting_input_stream_id: Option<MediaStreamId>,
    output_sample: Option<(MediaStreamId, usize, MediaSample)>,
    stats: ProcessorStats,
    finished: bool,
}

impl Task {
    fn new<P>(processor: P) -> (Self, Vec<(MediaStreamId, MediaSampleSyncSender)>)
    where
        P: 'static + Send + MediaProcessor,
    {
        let mut input_stream_rxs = HashMap::new();
        let mut input_stream_txs = Vec::new();

        for input_stream_id in processor.spec().input_stream_ids {
            let (tx, rx) = mpsc::sync_channel(CHANNEL_SIZE_LIMIT);
            input_stream_rxs.insert(input_stream_id, rx);
            input_stream_txs.push((input_stream_id, tx));
        }

        let stats = processor.spec().stats;
        let task = Self {
            processor: BoxedMediaProcessor::new(processor),
            input_stream_rxs,
            output_stream_txs: HashMap::new(),
            awaiting_input_stream_id: None,
            output_sample: None,
            stats,
            finished: false,
        };
        (task, input_stream_txs)
    }

    fn process_input(&mut self) -> orfail::Result<bool> {
        let Some(stream_id) = self.awaiting_input_stream_id.take() else {
            return Ok(false);
        };
        let rx = self.input_stream_rxs.get(&stream_id).or_fail()?;
        match rx.try_recv() {
            Err(mpsc::TryRecvError::Disconnected) => {
                let input = MediaProcessorInput::eos(stream_id);
                self.processor.process_input(input).or_fail()?;
                Ok(true)
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.awaiting_input_stream_id = Some(stream_id);
                Ok(false)
            }
            Ok(sample) => {
                let input = MediaProcessorInput::sample(stream_id, sample);
                self.processor.process_input(input).or_fail()?;
                Ok(true)
            }
        }
    }

    fn process_output(&mut self) -> orfail::Result<bool> {
        if self.awaiting_input_stream_id.is_some() {
            return Ok(false);
        }

        if let Some((stream_id, mut i, sample)) = self.output_sample.take() {
            let txs = self.output_stream_txs.get_mut(&stream_id).or_fail()?;
            while i < txs.len() {
                match txs[i].try_send(sample.clone()) {
                    Ok(()) => {
                        i += 1;
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        txs.swap_remove(i);
                    }
                    Err(mpsc::TrySendError::Full(_)) => {
                        self.output_sample = Some((stream_id, i, sample));
                        return Ok(false);
                    }
                }
            }
            if txs.is_empty() {
                self.output_stream_txs.remove(&stream_id);
            }
        }

        match self.processor.process_output().or_fail()? {
            MediaProcessorOutput::Finished => {
                self.finished = true;
                Ok(false)
            }
            MediaProcessorOutput::Pending { awaiting_stream_id } => {
                self.awaiting_input_stream_id = Some(awaiting_stream_id);
                Ok(true)
            }
            MediaProcessorOutput::Processed { stream_id, sample } => {
                if self.output_stream_txs.is_empty() {
                    self.finished = true;
                    Ok(false)
                } else {
                    if self.output_stream_txs.contains_key(&stream_id) {
                        self.output_sample = Some((stream_id, 0, sample));
                    }
                    Ok(true)
                }
            }
        }
    }

    fn run_until_block(&mut self) -> orfail::Result<()> {
        while self.process_input().or_fail()? || self.process_output().or_fail()? {}
        Ok(())
    }
}

#[derive(Debug)]
pub struct Scheduler {
    tasks: Vec<Task>,
    thread_count: NonZeroUsize,
    stream_txs: HashMap<MediaStreamId, Vec<MediaSampleSyncSender>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            thread_count: NonZeroUsize::MIN,
            stream_txs: HashMap::new(),
        }
    }

    pub fn register<P>(&mut self, processor: P) -> orfail::Result<()>
    where
        P: 'static + Send + MediaProcessor,
    {
        let (task, input_stream_txs) = Task::new(processor);
        self.tasks.push(task);

        for (id, tx) in input_stream_txs {
            self.stream_txs.entry(id).or_default().push(tx);
        }

        Ok(())
    }

    pub fn spawn(mut self) -> orfail::Result<SchedulerHandle> {
        self.update_output_stream_txs().or_fail()?;

        let mut tasks = self.tasks.into_iter().map(Some).collect::<Vec<_>>();

        // TODO(atode): スレッドへの割り当て方法は後で改善する
        // TODO(atode): スレッド単位の統計を追加する
        let mut handles = Vec::new();
        for i in 0..self.thread_count.get() {
            let mut thread_tasks = Vec::new();
            for (j, task) in tasks.iter_mut().enumerate() {
                if j % self.thread_count.get() != i {
                    continue;
                }
                thread_tasks.push(task.take().or_fail()?);
            }
            let runner = TaskRunner::new(thread_tasks);
            let handle = std::thread::spawn(|| runner.run());
            handles.push(handle);
        }

        Ok(SchedulerHandle { handles })
    }

    pub fn run(self) -> orfail::Result<()> {
        let handle = self.spawn().or_fail()?;
        for handle in handle.handles {
            if let Err(e) = handle.join() {
                std::panic::resume_unwind(e);
            }
        }
        Ok(())
    }

    fn update_output_stream_txs(&mut self) -> orfail::Result<()> {
        for task in &mut self.tasks {
            for id in task.processor.spec().output_stream_ids {
                let tx = self
                    .stream_txs
                    .get(&id)
                    .cloned()
                    .or_fail_with(|()| format!("BUG: missing output stream ID: {id:?}"))?;
                task.output_stream_txs.insert(id, tx);
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct SchedulerHandle {
    handles: Vec<std::thread::JoinHandle<()>>,
}

#[derive(Debug)]
pub struct TaskRunner {
    tasks: Vec<Task>,
}

impl TaskRunner {
    fn new(tasks: Vec<Task>) -> Self {
        Self { tasks }
    }

    fn run(mut self) {
        while !self.tasks.is_empty() {
            self.run_one();
        }
    }

    fn run_one(&mut self) {
        let mut i = 0;
        while i < self.tasks.len() {
            // TODO: 時間計測
            match self.tasks[i].run_until_block().or_fail() {
                Err(e) => {
                    log::error!("{e}");
                    self.tasks[i].stats.set_error();
                    self.tasks.swap_remove(i);
                }
                Ok(()) if self.tasks[i].finished => {
                    self.tasks.swap_remove(i);
                }
                Ok(()) => {
                    i += 1;
                }
            }
        }

        if self.is_awaiting() {
            let duration = std::time::Duration::from_millis(10); // TODO
            std::thread::sleep(duration);
        }
    }

    fn is_awaiting(&self) -> bool {
        self.tasks
            .iter()
            .all(|t| t.awaiting_input_stream_id.is_some() || t.output_sample.is_some())
    }
}
