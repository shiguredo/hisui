use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::mpsc;

use orfail::OrFail;

use crate::media::{MediaSample, MediaStreamId};
use crate::processor::{BoxedMediaProcessor, MediaProcessor};

const CHANNEL_SIZE_LIMIT: usize = 5; // TODO:

type MediaSampleReceiver = mpsc::Receiver<MediaSample>;
type MediaSampleSyncSender = mpsc::SyncSender<MediaSample>;

#[derive(Debug)]
pub struct Task {
    processor: BoxedMediaProcessor,
    input_stream_rxs: HashMap<MediaStreamId, MediaSampleReceiver>,
    output_stream_txs: HashMap<MediaStreamId, MediaSampleSyncSender>,
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

        let task = Self {
            processor: BoxedMediaProcessor::new(processor),
            input_stream_rxs,
            output_stream_txs: HashMap::new(),
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

    fn run(self) -> orfail::Result<()> {
        Ok(())
    }
}
