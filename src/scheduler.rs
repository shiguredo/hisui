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
    output_stream_txs: HashMap<MediaStreamId, MediaSampleSyncSender>,
    awaiting_input_stream_id: Option<MediaStreamId>,
    awaiting_output_sample: Option<(MediaStreamId, MediaSample)>,
    stats: ProcessorStats,
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
            awaiting_output_sample: None,
            stats,
        };
        (task, input_stream_txs)
    }
}

#[derive(Debug)]
pub struct Scheduler {
    tasks: Vec<Task>,
    thread_count: NonZeroUsize,
    stream_txs: HashMap<MediaStreamId, MediaSampleSyncSender>,
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
            self.stream_txs
                .insert(id, tx)
                .is_none()
                .or_fail_with(|()| format!("BUG: conflicting processor ID: {id:?}"))?;
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
            let handle = std::thread::spawn(|| runner.run().or_fail());
            handles.push(handle);
        }

        Ok(SchedulerHandle { handles })
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
    handles: Vec<std::thread::JoinHandle<orfail::Result<()>>>,
}

#[derive(Debug)]
pub struct TaskRunner {
    tasks: Vec<Task>,
}

impl TaskRunner {
    fn new(tasks: Vec<Task>) -> Self {
        Self { tasks }
    }

    fn run(mut self) -> orfail::Result<()> {
        while !self.tasks.is_empty() {
            self.run_one().or_fail()?;
        }
        Ok(())
    }

    fn run_one(&mut self) -> orfail::Result<()> {
        let mut i = 0;
        while i < self.tasks.len() {
            // TODO: 時間計測
            match self.run_task(i).or_fail() {
                Err(e) => {
                    log::error!("{e}");
                    self.tasks[i].stats.set_error();
                    self.tasks.swap_remove(i);
                }
                Ok(true) => {
                    self.tasks.swap_remove(i);
                }
                Ok(false) => {
                    i += 1;
                }
            }
        }

        if self.is_awaiting() {
            let duration = std::time::Duration::from_millis(10); // TODO
            std::thread::sleep(duration);
        }
        Ok(())
    }

    fn is_awaiting(&self) -> bool {
        self.tasks
            .iter()
            .all(|t| t.awaiting_input_stream_id.is_some() || t.awaiting_output_sample.is_some())
    }

    fn run_task(&mut self, i: usize) -> orfail::Result<bool> {
        let task = &mut self.tasks[i];
        loop {
            // TODO: handle awaiting_output_sample
            if let Some(stream_id) = task.awaiting_input_stream_id.take() {
                let rx = task.input_stream_rxs.get(&stream_id).or_fail()?;
                match rx.try_recv() {
                    Err(mpsc::TryRecvError::Disconnected) => {
                        let input = MediaProcessorInput::eos(stream_id);
                        task.processor.process_input(input).or_fail()?;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        task.awaiting_input_stream_id = Some(stream_id);
                        break;
                    }
                    Ok(sample) => {
                        let input = MediaProcessorInput::sample(stream_id, sample);
                        task.processor.process_input(input).or_fail()?;
                    }
                }
            } else {
                match task.processor.process_output().or_fail()? {
                    MediaProcessorOutput::Finished => return Ok(true),
                    MediaProcessorOutput::Pending { awaiting_stream_id } => {
                        task.awaiting_input_stream_id = Some(awaiting_stream_id);
                    }
                    MediaProcessorOutput::Processed { stream_id, sample } => {
                        let tx = task.output_stream_txs.get(&stream_id).or_fail()?;
                        match tx.try_send(sample) {
                            Ok(()) => {}
                            Err(mpsc::TrySendError::Full(sample)) => {
                                task.awaiting_output_sample = Some((stream_id, sample));
                                break;
                            }
                            Err(mpsc::TrySendError::Disconnected(_)) => {
                                todo!();
                            }
                        }
                    }
                }
            }
        }
        Ok(false)
    }
}
