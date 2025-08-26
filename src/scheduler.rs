use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use orfail::OrFail;

use crate::media::{MediaSample, MediaStreamId};
use crate::processor::{
    BoxedMediaProcessor, MediaProcessor, MediaProcessorInput, MediaProcessorOutput,
};
use crate::stats::{ProcessorStats, Seconds, SharedAtomicFlag, Stats, WorkerThreadStats};

type MediaSampleReceiver = mpsc::Receiver<MediaSample>;
type MediaSampleSyncSender = mpsc::SyncSender<MediaSample>;

// 各プロセッサが `MediaSample` をやりとりするチャネルのサイズ上限。
// 上限なしだと、プロデューサーのペースがコンシューマーよりも早い場合に、
// メモリ消費量が増え続けてしまうので、それを防止するための制限。
//
// 値の細かい調整は不要な想定だが、いちおう、隠し設定として環境変数経由で変更可能にしておく。
fn sync_channel_size() -> usize {
    let size = std::env::var("HISUI_SYNC_CHANNEL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    log::debug!("SYNC_CHANNEL_SIZE={size}");
    size
}

// プロセッサーが入力ないし出力送信待ちでやることがない場合のスリープ時間。
//
// 値の細かい調整は不要な想定だが、いちおう、隠し設定として環境変数経由で変更可能にしておく。
fn idle_thread_sleep_duration() -> Duration {
    let ms = std::env::var("HISUI_IDLE_THREAD_SLEEP_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    log::debug!("IDLE_THREAD_SLEEP_MS={ms}");
    Duration::from_millis(ms)
}

#[derive(Debug)]
pub struct Task {
    processor: BoxedMediaProcessor,
    input_stream_rxs: HashMap<MediaStreamId, MediaSampleReceiver>,
    output_stream_txs: HashMap<MediaStreamId, Vec<MediaSampleSyncSender>>,
    awaiting_input_stream_ids: Vec<MediaStreamId>,
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

        let channel_size = sync_channel_size();
        for input_stream_id in processor.spec().input_stream_ids {
            let (tx, rx) = mpsc::sync_channel(channel_size);
            input_stream_rxs.insert(input_stream_id, rx);
            input_stream_txs.push((input_stream_id, tx));
        }

        let stats = processor.spec().stats;
        let task = Self {
            processor: BoxedMediaProcessor::new(processor),
            input_stream_rxs,
            output_stream_txs: HashMap::new(),
            awaiting_input_stream_ids: Vec::new(),
            output_sample: None,
            stats,
            finished: false,
        };
        (task, input_stream_txs)
    }

    fn process_input(&mut self) -> orfail::Result<bool> {
        let mut input = None;
        for &stream_id in &self.awaiting_input_stream_ids {
            let rx = self.input_stream_rxs.get(&stream_id).or_fail()?;
            match rx.try_recv() {
                Err(mpsc::TryRecvError::Disconnected) => {
                    input = Some(MediaProcessorInput::eos(stream_id));
                    self.input_stream_rxs.remove(&stream_id);
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Ok(sample) => {
                    input = Some(MediaProcessorInput::sample(stream_id, sample));
                    break;
                }
            }
        }
        if let Some(input) = input {
            self.processor.process_input(input).or_fail()?;
            self.awaiting_input_stream_ids.clear();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn process_output(&mut self) -> orfail::Result<bool> {
        if !self.awaiting_input_stream_ids.is_empty() {
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
                if let Some(id) = awaiting_stream_id {
                    self.awaiting_input_stream_ids.push(id);
                } else {
                    self.awaiting_input_stream_ids
                        .extend(self.input_stream_rxs.keys().copied());
                }
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

    fn run_until_block(&mut self) -> orfail::Result<bool> {
        let mut did_something = false;
        while self.process_input().or_fail()? || self.process_output().or_fail()? {
            did_something = true;
        }
        Ok(did_something)
    }
}

#[derive(Debug)]
pub struct Scheduler {
    tasks: Vec<Task>,
    thread_count: NonZeroUsize,
    stream_txs: HashMap<MediaStreamId, Vec<MediaSampleSyncSender>>,
    stats: Stats,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            thread_count: NonZeroUsize::MIN,
            stream_txs: HashMap::new(),
            stats: Stats::default(),
        }
    }

    pub fn register<P>(&mut self, processor: P) -> orfail::Result<()>
    where
        P: 'static + Send + MediaProcessor,
    {
        self.stats.processors.push(processor.spec().stats);

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
            let mut worker_thread_stats = WorkerThreadStats::default();
            let mut thread_tasks = Vec::new();
            for (j, task) in tasks.iter_mut().enumerate() {
                if j % self.thread_count.get() != i {
                    continue;
                }
                thread_tasks.push(task.take().or_fail()?);
                worker_thread_stats.processors.push(j);
            }
            let runner = TaskRunner::new(
                thread_tasks,
                worker_thread_stats.clone(),
                self.stats.error.clone(),
            );
            let handle = std::thread::spawn(|| runner.run());
            handles.push(handle);
            self.stats.worker_threads.push(worker_thread_stats);
        }

        Ok(SchedulerHandle {
            handles,
            stats: self.stats,
        })
    }

    pub fn run(self) -> orfail::Result<Stats> {
        let start = Instant::now();
        let mut handle = self.spawn().or_fail()?;
        for handle in handle.handles {
            if let Err(e) = handle.join() {
                std::panic::resume_unwind(e);
            }
        }
        handle.stats.elapsed_seconds = Seconds::new(start.elapsed());
        Ok(handle.stats)
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

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct SchedulerHandle {
    handles: Vec<std::thread::JoinHandle<()>>,
    stats: Stats,
}

#[derive(Debug)]
pub struct TaskRunner {
    tasks: Vec<Task>,
    sleep_duration: Duration,
    stats: WorkerThreadStats,
    error_flag: SharedAtomicFlag,
}

impl TaskRunner {
    fn new(tasks: Vec<Task>, stats: WorkerThreadStats, error_flag: SharedAtomicFlag) -> Self {
        let sleep_duration = idle_thread_sleep_duration();
        Self {
            tasks,
            sleep_duration,
            stats,
            error_flag,
        }
    }

    fn run(mut self) {
        while !self.tasks.is_empty() && !self.error_flag.get() {
            self.run_one();
        }
    }

    fn run_one(&mut self) {
        let mut i = 0;
        let mut did_something = false;
        while i < self.tasks.len() {
            let start = Instant::now();
            let result = self.tasks[i].run_until_block().or_fail();
            let elapsed = start.elapsed();
            self.tasks[i]
                .stats
                .total_processing_seconds()
                .add(Seconds::new(elapsed));
            self.stats
                .total_processing_seconds
                .add(Seconds::new(elapsed));

            match result {
                Err(e) => {
                    log::error!("{e}");
                    self.error_flag.set(true);
                    self.tasks[i].stats.set_error();
                    self.tasks.swap_remove(i);
                }
                Ok(task_did_something) if self.tasks[i].finished => {
                    self.tasks.swap_remove(i);
                    did_something |= task_did_something;
                }
                Ok(task_did_something) => {
                    i += 1;
                    did_something |= task_did_something;
                }
            }
        }

        if !did_something {
            std::thread::sleep(self.sleep_duration);
            self.stats
                .total_waiting_seconds
                .add(Seconds::new(self.sleep_duration));
        }
    }
}
