use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use crate::media::{MediaSample, MediaStreamId};
use crate::processor::{
    BoxedMediaProcessor, MediaProcessor, MediaProcessorInput, MediaProcessorOutput,
    MediaProcessorWorkloadHint,
};

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
        // NOTE: ここが小さいと mp4_writer の処理方法的にブロックすることがあるので大きめにしている。
        //       近い将来にそもそも今の scheduler.rs はなくなって tokio ベースに切り替わる予定なので、
        //       この暫定修正で問題ない。
        .unwrap_or(500);
    tracing::debug!("SYNC_CHANNEL_SIZE={size}");
    size
}

#[derive(Debug)]
pub struct Task {
    thread_number: usize,
    processor: BoxedMediaProcessor,
    input_stream_rxs: HashMap<MediaStreamId, MediaSampleReceiver>,
    output_stream_txs: HashMap<MediaStreamId, Vec<MediaSampleSyncSender>>,
    awaiting_input_stream_ids: Vec<MediaStreamId>,
    output_sample: Option<(MediaStreamId, usize, MediaSample)>,
    workload_hint: MediaProcessorWorkloadHint,
    finished: bool,
}

impl Task {
    fn new<P>(processor: P) -> (Self, Vec<(MediaStreamId, MediaSampleSyncSender)>)
    where
        P: 'static + Send + MediaProcessor,
    {
        let mut input_stream_rxs = HashMap::new();
        let mut input_stream_txs = Vec::new();

        let spec = processor.spec();
        let channel_size = sync_channel_size();
        for input_stream_id in spec.input_stream_ids {
            let (tx, rx) = mpsc::sync_channel(channel_size);
            input_stream_rxs.insert(input_stream_id, rx);
            input_stream_txs.push((input_stream_id, tx));
        }

        let task = Self {
            thread_number: 0, // 複数スレッドを使う場合には、後で再割り当てされる
            processor: BoxedMediaProcessor::new(processor),
            input_stream_rxs,
            output_stream_txs: HashMap::new(),
            awaiting_input_stream_ids: Vec::new(),
            output_sample: None,
            workload_hint: spec.workload_hint,
            finished: false,
        };
        (task, input_stream_txs)
    }

    fn process_input(&mut self) -> crate::Result<bool> {
        let mut input = None;
        for &stream_id in &self.awaiting_input_stream_ids {
            let rx = self
                .input_stream_rxs
                .get(&stream_id)
                .ok_or_else(|| crate::Error::new("value is missing"))?;
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
            self.processor.process_input(input)?;
            self.awaiting_input_stream_ids.clear();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn process_output(&mut self) -> crate::Result<bool> {
        if !self.awaiting_input_stream_ids.is_empty() {
            return Ok(false);
        }

        if let Some((stream_id, mut i, sample)) = self.output_sample.take() {
            let txs = self
                .output_stream_txs
                .get_mut(&stream_id)
                .ok_or_else(|| crate::Error::new("value is missing"))?;
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

        match self.processor.process_output()? {
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

    fn run_until_block(&mut self) -> crate::Result<bool> {
        let mut did_something = false;
        while self.process_input()? || self.process_output()? {
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
    error: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct SchedulerResult {
    pub elapsed_duration: Duration,
    pub error: bool,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::with_thread_count(NonZeroUsize::MIN)
    }

    pub fn with_thread_count(thread_count: NonZeroUsize) -> Self {
        Self {
            tasks: Vec::new(),
            thread_count,
            stream_txs: HashMap::new(),
            error: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn register<P>(&mut self, processor: P) -> crate::Result<()>
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

    fn spawn(mut self) -> crate::Result<SchedulerHandle> {
        self.update_output_stream_txs()?;

        // コストが高い順にソートする
        // なお、現時点では、I/O タスクは「コストが最低の CPU タスク」として扱っている
        // （将来的に I/O タスクと特別扱いした方がいいようなユースケースが出てきたら、その時に扱いを変更する）
        self.tasks.sort_by_key(|t| match t.workload_hint {
            MediaProcessorWorkloadHint::IoIntensive => NonZeroUsize::MIN,
            MediaProcessorWorkloadHint::CpuIntensive { cost } => cost,
        });
        self.tasks.reverse();

        // コストができるだけ均等になるように、タスクをスレッドに割り当てる
        let mut thread_costs = vec![0; self.thread_count.get()];
        for task in &mut self.tasks {
            let cost = match task.workload_hint {
                MediaProcessorWorkloadHint::IoIntensive => NonZeroUsize::MIN,
                MediaProcessorWorkloadHint::CpuIntensive { cost } => cost,
            };

            // スレッド数は多くても高々数十なので、シンプルな線形探索を行う
            let i = thread_costs
                .iter()
                .enumerate()
                .min_by_key(|(_, cost)| *cost)
                .ok_or_else(|| crate::Error::new("value is missing"))? // 累積コストが一番低いスレッドを選ぶ
                .0;
            thread_costs[i] += cost.get();
            task.thread_number = i;
        }

        let mut handles = Vec::new();
        for i in 0..self.thread_count.get() {
            let mut thread_tasks = Vec::new();

            let mut j = 0;
            while j < self.tasks.len() {
                if self.tasks[j].thread_number == i {
                    let task = self.tasks.swap_remove(j);
                    thread_tasks.push(task);
                } else {
                    j += 1
                };
            }
            if thread_tasks.is_empty() {
                continue;
            };
            let runner = TaskRunner::new(thread_tasks, self.error.clone());
            let handle = std::thread::spawn(|| runner.run());
            handles.push(handle);
        }

        Ok(SchedulerHandle {
            handles,
            error: self.error,
        })
    }

    pub fn run(self) -> crate::Result<SchedulerResult> {
        let start = Instant::now();
        let handle = self.spawn()?;
        for handle in handle.handles {
            if let Err(e) = handle.join() {
                std::panic::resume_unwind(e);
            }
        }
        Ok(SchedulerResult {
            elapsed_duration: start.elapsed(),
            error: handle.error.load(Ordering::Relaxed),
        })
    }

    pub fn run_timeout(self, timeout: Duration) -> crate::Result<(bool, SchedulerResult)> {
        // 完了待ちのビジーループを避けるためのスリープの時間
        // 適当に長めの時間ならなんでもいい
        const SLEEP_DURATION: Duration = Duration::from_millis(100);

        let start = Instant::now();
        let mut handle = self.spawn()?;
        let mut timeout_expired = false;
        while !handle.handles.is_empty() {
            if !timeout_expired && timeout < start.elapsed() {
                // エラーフラグを立てて、ワーカースレッドを終了処理に移行させる
                handle.error.store(true, Ordering::Relaxed);
                timeout_expired = true;
                tracing::debug!(
                    "Timeout expired after {} seconds, signaling worker threads to terminate",
                    timeout.as_secs_f32()
                );
            }

            let mut i = 0;
            let mut did_something = false;
            while i < handle.handles.len() {
                if !handle.handles[i].is_finished() {
                    i += 1;
                    continue;
                }

                let handle = handle.handles.swap_remove(i);
                if let Err(e) = handle.join() {
                    std::panic::resume_unwind(e);
                }
                did_something = true;
            }

            if !did_something {
                std::thread::sleep(SLEEP_DURATION);
            }
        }

        Ok((
            timeout_expired,
            SchedulerResult {
                elapsed_duration: start.elapsed(),
                error: handle.error.load(Ordering::Relaxed),
            },
        ))
    }

    fn update_output_stream_txs(&mut self) -> crate::Result<()> {
        for task in &mut self.tasks {
            for id in task.processor.spec().output_stream_ids {
                if let Some(tx) = self.stream_txs.get(&id).cloned() {
                    task.output_stream_txs.insert(id, tx);
                } else {
                    // このストリームを入力に取るプロセッサがいない場合にはここにくる（正常系）
                }
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
struct SchedulerHandle {
    handles: Vec<std::thread::JoinHandle<()>>,
    error: Arc<AtomicBool>,
}

#[derive(Debug)]
struct TaskRunner {
    tasks: Vec<Task>,
    error_flag: Arc<AtomicBool>,
    next_sleep_duration: Option<Duration>,
}

impl TaskRunner {
    fn new(tasks: Vec<Task>, error_flag: Arc<AtomicBool>) -> Self {
        Self {
            tasks,
            error_flag,
            next_sleep_duration: None,
        }
    }

    fn run(mut self) {
        while !self.tasks.is_empty() && !self.error_flag.load(Ordering::Relaxed) {
            self.run_one();
        }
    }

    fn run_one(&mut self) {
        let mut i = 0;
        let mut did_something = false;
        while i < self.tasks.len() {
            let result = self.tasks[i].run_until_block();

            match result {
                Err(e) => {
                    tracing::error!("{e}");
                    self.error_flag.store(true, Ordering::Relaxed);
                    self.tasks[i].processor.set_error();
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

        if did_something {
            self.next_sleep_duration = None;
        } else if let Some(duration) = self.next_sleep_duration {
            // 指数的バックオフを使ってスリープする
            //
            // 最大値は適当に大きめの値であればなんでもいい
            const MAX_SLEEP_DURATION: Duration = Duration::from_millis(50);

            std::thread::sleep(duration);
            self.next_sleep_duration = Some((duration * 2).min(MAX_SLEEP_DURATION));
        } else {
            self.next_sleep_duration = Some(Duration::from_millis(1));
        }
    }
}
