use std::collections::{BinaryHeap, HashMap};
use std::time::{Duration, Instant};

use orfail::OrFail;

use crate::audio::AudioData;
use crate::media::{MediaSample, MediaStreamId};
use crate::stats::ProcessorStats;
use crate::video::VideoFrame;

pub trait MediaProcessor {
    fn spec(&self) -> MediaProcessorSpec;

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()>;
    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput>;

    fn set_error(&self) {
        self.spec().stats.set_error();
    }
}

pub struct BoxedMediaProcessor(Box<dyn 'static + Send + MediaProcessor>);

impl BoxedMediaProcessor {
    pub fn new<P: 'static + Send + MediaProcessor>(processor: P) -> Self {
        Self(Box::new(processor))
    }
}

impl std::fmt::Debug for BoxedMediaProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxedMediaProcessor")
            .finish_non_exhaustive()
    }
}

impl MediaProcessor for BoxedMediaProcessor {
    fn spec(&self) -> MediaProcessorSpec {
        self.0.spec()
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        self.0.process_input(input)
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        self.0.process_output()
    }
}

#[derive(Debug, Clone)]
pub enum MediaProcessorWorkloadHint {
    /// I/O集約的なプロセッサ
    ///
    /// できるだけCPU集約的なプロセッサ群とは別のスレッドにまとめて配置される
    IoIntensive,

    /// CPU集約的なプロセッサ
    CpuIntensive {
        /// プロセッサの処理の重さの目安
        /// 各スレッドが担当するコストの総量ができるだけ均等になるように配置される
        cost: std::num::NonZeroUsize,
    },
}

#[derive(Debug, Clone)]
pub struct MediaProcessorSpec {
    pub input_stream_ids: Vec<MediaStreamId>,
    pub output_stream_ids: Vec<MediaStreamId>,
    pub stats: ProcessorStats,
}

#[derive(Debug)]
pub struct MediaProcessorInput {
    pub stream_id: MediaStreamId,
    pub sample: Option<MediaSample>, // None なら EOS を表す
}

impl MediaProcessorInput {
    pub fn eos(stream_id: MediaStreamId) -> Self {
        Self {
            stream_id,
            sample: None,
        }
    }

    pub fn sample(stream_id: MediaStreamId, sample: MediaSample) -> Self {
        Self {
            stream_id,
            sample: Some(sample),
        }
    }

    pub fn audio_data(stream_id: MediaStreamId, data: AudioData) -> Self {
        Self {
            stream_id,
            sample: Some(MediaSample::audio_data(data)),
        }
    }

    pub fn video_frame(stream_id: MediaStreamId, frame: VideoFrame) -> Self {
        Self {
            stream_id,
            sample: Some(MediaSample::video_frame(frame)),
        }
    }
}

#[derive(Debug)]
pub enum MediaProcessorOutput {
    Processed {
        stream_id: MediaStreamId,
        sample: MediaSample,
    },
    Pending {
        // 入力を待機しているストリームの ID
        //
        // None の場合は任意のストリームを待機していることを示す
        //
        // [NOTE]
        // `Mp4Writer` のように複数の入力をとるプロセッサーが
        // 複数いた場合にデッドロックが発生する可能性がある
        // （たとえば、片方が音声ストリームを、もう片方が映像ストリームを優先して処理するような場合）
        //
        // ただし、通常はそういったプロセッサーは、
        // 一番最後のフェーズにひとつだけ存在することが多いはずなので
        // これが実際に問題となることはほぼないはず
        //
        // もし発生した場合には `SyncSender` のバッファサイズを増やすか、
        // 問題となっているプロセッサーの実装を見直す必要がある
        awaiting_stream_id: Option<MediaStreamId>,
    },
    Finished,
}

impl MediaProcessorOutput {
    pub fn expect_processed(self) -> Option<(MediaStreamId, MediaSample)> {
        if let Self::Processed { stream_id, sample } = self {
            Some((stream_id, sample))
        } else {
            None
        }
    }

    pub fn pending(awaiting_stream_id: MediaStreamId) -> Self {
        Self::Pending {
            awaiting_stream_id: Some(awaiting_stream_id),
        }
    }

    pub fn awaiting_any() -> Self {
        Self::Pending {
            awaiting_stream_id: None,
        }
    }

    pub fn audio_data(stream_id: MediaStreamId, data: AudioData) -> Self {
        Self::Processed {
            stream_id,
            sample: MediaSample::audio_data(data),
        }
    }

    pub fn video_frame(stream_id: MediaStreamId, frame: VideoFrame) -> Self {
        Self::Processed {
            stream_id,
            sample: MediaSample::video_frame(frame),
        }
    }
}

#[derive(Debug)]
struct PacerQueueItem(MediaStreamId, MediaSample);

impl PartialEq for PacerQueueItem {
    fn eq(&self, other: &Self) -> bool {
        self.1.timestamp() == other.1.timestamp()
    }
}

impl Eq for PacerQueueItem {}

impl PartialOrd for PacerQueueItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PacerQueueItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Note: BinaryHeap is a max-heap, so we reverse the comparison
        // to get earliest timestamps first
        other.1.timestamp().cmp(&self.1.timestamp())
    }
}

#[derive(Debug)]
pub struct RealtimePacer {
    stream_ids: HashMap<MediaStreamId, MediaStreamId>,
    stream_timestamps: HashMap<MediaStreamId, Duration>,
    queue: BinaryHeap<PacerQueueItem>,
    start_time: Option<Instant>,
}

impl RealtimePacer {
    pub fn new(
        input_stream_ids: Vec<MediaStreamId>,
        output_stream_ids: Vec<MediaStreamId>,
    ) -> orfail::Result<Self> {
        (input_stream_ids.len() == output_stream_ids.len()).or_fail()?;
        Ok(Self {
            stream_ids: input_stream_ids
                .iter()
                .copied()
                .zip(output_stream_ids)
                .collect(),
            stream_timestamps: input_stream_ids
                .iter()
                .copied()
                .zip(std::iter::repeat(Duration::ZERO))
                .collect(),
            queue: BinaryHeap::new(),
            start_time: None,
        })
    }

    fn elapsed(&mut self) -> Duration {
        if let Some(t) = self.start_time {
            t.elapsed()
        } else {
            let t = Instant::now();
            self.start_time = Some(t);
            t.elapsed()
        }
    }
}

impl MediaProcessor for RealtimePacer {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self.stream_ids.keys().copied().collect(),
            output_stream_ids: self.stream_ids.values().copied().collect(),
            stats: ProcessorStats::other("realtime_pacer"),
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        let output_stream_id = self.stream_ids.get(&input.stream_id).copied().or_fail()?;
        if let Some(sample) = input.sample {
            self.stream_timestamps
                .insert(input.stream_id, sample.timestamp());

            // TODO(atode): キューはストリーム毎に管理すべき
            self.queue.push(PacerQueueItem(output_stream_id, sample));
        } else {
            self.stream_ids.remove(&input.stream_id);
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        let Some(PacerQueueItem(stream_id, sample)) = self.queue.pop() else {
            if self.stream_ids.is_empty() {
                return Ok(MediaProcessorOutput::Finished);
            } else {
                return Ok(MediaProcessorOutput::awaiting_any());
            }
        };

        let now = self.elapsed();
        let Some(time_to_wait) = sample
            .timestamp()
            .checked_sub(now)
            .take_if(|d| !d.is_zero())
        else {
            return Ok(MediaProcessorOutput::Processed { stream_id, sample });
        };

        // TODO(atode): ハードコーディングをやめる
        if self.queue.len() < 10 {
            self.queue.push(PacerQueueItem(stream_id, sample));

            if let Some((input_stream_id, _)) =
                self.stream_timestamps.iter().min_by_key(|(_, t)| *t)
            {
                return Ok(MediaProcessorOutput::pending(*input_stream_id));
            } else {
                return Ok(MediaProcessorOutput::awaiting_any());
            }
        }

        // TODO(atode): sleep はやめる
        std::thread::sleep(time_to_wait);
        Ok(MediaProcessorOutput::Processed { stream_id, sample })
    }
}
