use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use orfail::OrFail;

use crate::{
    audio::{
        AudioData, AudioDataReceiver, AudioDataSyncSender, AudioFormat, CHANNELS, SAMPLE_RATE,
    },
    channel::{self, ErrorFlag},
    layout::Layout,
    media::{MediaSample, MediaStreamId},
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec},
    stats::{AudioMixerStats, ProcessorStats, Seconds, SharedStats},
};

pub const MIXED_AUDIO_DATA_DURATION: Duration = Duration::from_millis(20);
const MIXED_AUDIO_DATA_SAMPLES: usize = 960;

#[derive(Debug)]
pub struct AudioMixerThread {
    input_rxs: Vec<AudioDataReceiver>,
    output_tx: AudioDataSyncSender,
    inner: AudioMixer,
}

impl AudioMixerThread {
    pub fn start(
        error_flag: ErrorFlag,
        layout: Layout,
        input_rxs: Vec<AudioDataReceiver>,
        shared_stats: SharedStats,
    ) -> AudioDataReceiver {
        let (tx, rx) = channel::sync_channel();
        let input_stream_ids = (0..input_rxs.len())
            .map(|i| MediaStreamId::new(i as u64))
            .collect();
        let output_stream_id = MediaStreamId::new(input_rxs.len() as u64);
        let inner = AudioMixer::new(layout, input_stream_ids, output_stream_id);
        let mut this = Self {
            input_rxs,
            output_tx: tx,
            inner,
        };
        std::thread::spawn(move || {
            log::debug!("audio mixer started");
            if let Err(e) = this.run().or_fail() {
                error_flag.set();
                this.inner.stats.error.set(true);
                log::error!("failed to mix audio sources: {e}");
            }
            log::debug!("audio mixer finished");

            shared_stats.with_lock(|stats| {
                stats.processors.push(this.inner.spec().stats);
            });
        });
        rx
    }

    fn run(&mut self) -> orfail::Result<()> {
        loop {
            match self.inner.process_output().or_fail()? {
                MediaProcessorOutput::Finished => break,
                MediaProcessorOutput::Processed { sample, .. } => {
                    let data = sample.expect_audio_data().or_fail()?;

                    // 今の実装ではここの参照カウントは常に 1 となるので必ず成功する
                    // [NOTE] 将来的には `run()` 自体がなくなるので、このコードも暫定的なもの
                    let data = std::sync::Arc::into_inner(data).or_fail()?;

                    if !self.output_tx.send(data) {
                        // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                        log::info!("receiver of mixed audio stream has been closed");
                        break;
                    }
                }
                MediaProcessorOutput::Pending { awaiting_stream_id } => {
                    let i = awaiting_stream_id.get() as usize;
                    let input = if let Some(data) = self.input_rxs[i].recv() {
                        MediaProcessorInput::audio_data(awaiting_stream_id, data)
                    } else {
                        MediaProcessorInput::eos(awaiting_stream_id)
                    };
                    self.inner.process_input(input).or_fail()?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct InputStream {
    eos: bool,
    sample_queue: VecDeque<(i16, i16)>,
    start_timestamp: Option<Duration>,
}

#[derive(Debug)]
pub struct AudioMixer {
    layout: Layout,
    input_streams: HashMap<MediaStreamId, InputStream>,
    output_stream_id: MediaStreamId,
    stats: AudioMixerStats,
}

impl AudioMixer {
    pub fn new(
        layout: Layout,
        input_stream_ids: Vec<MediaStreamId>,
        output_stream_id: MediaStreamId,
    ) -> Self {
        Self {
            layout,
            input_streams: input_stream_ids
                .into_iter()
                .map(|id| (id, InputStream::default()))
                .collect(),
            output_stream_id,
            stats: AudioMixerStats::default(),
        }
    }

    fn next_input_timestamp(&self) -> Duration {
        Duration::from_secs(
            self.stats.total_output_sample_count.get()
                + self.stats.total_trimmed_sample_count.get(),
        ) / SAMPLE_RATE as u32
    }

    fn next_output_timestamp(&self) -> Duration {
        Duration::from_secs(self.stats.total_output_sample_count.get()) / SAMPLE_RATE as u32
    }

    fn mix_next_audio_data(&mut self, now: Duration) -> orfail::Result<AudioData> {
        let timestamp = self.next_output_timestamp();

        let bytes_per_sample = CHANNELS as usize * 2; // i16 で表現するので *2
        let mut mixed_samples = Vec::with_capacity(MIXED_AUDIO_DATA_SAMPLES * bytes_per_sample);

        let mut filled = true; // 無音補完されたかどうか
        for _ in 0..MIXED_AUDIO_DATA_SAMPLES {
            let mut acc_left = 0;
            let mut acc_right = 0;
            for stream in self.input_streams.values_mut() {
                if stream.start_timestamp.is_none_or(|t| t < now) {
                    // 開始時刻に達していない
                    continue;
                }
                let Some((left, right)) = stream.sample_queue.pop_front() else {
                    continue;
                };
                acc_left += left as i32;
                acc_right += right as i32;
                filled = false;
            }

            let left = acc_left.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            let right = acc_right.clamp(i16::MIN as i32, i16::MAX as i32) as i16;

            mixed_samples.extend_from_slice(&left.to_be_bytes());
            mixed_samples.extend_from_slice(&right.to_be_bytes());
        }

        self.stats.total_output_audio_data_count.add(1);
        self.stats
            .total_output_audio_data_seconds
            .add(Seconds::new(MIXED_AUDIO_DATA_DURATION));
        self.stats
            .total_output_sample_count
            .add(MIXED_AUDIO_DATA_SAMPLES as u64);
        if filled {
            self.stats
                .total_output_filled_sample_count
                .add(MIXED_AUDIO_DATA_SAMPLES as u64);
        }

        Ok(AudioData {
            // 以下は固定値
            source_id: None, // 合成後は常に None になる
            format: AudioFormat::I16Be,
            stereo: true, // Hisui では音声は常にステレオとして扱う
            sample_rate: SAMPLE_RATE,
            duration: MIXED_AUDIO_DATA_DURATION,
            sample_entry: None, // 生データにはサンプルエントリーはない

            // 以下は合成結果に応じた値
            data: mixed_samples,
            timestamp,
        })
    }
}

impl MediaProcessor for AudioMixer {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self.input_streams.keys().copied().collect(),
            output_stream_ids: vec![self.output_stream_id],
            stats: ProcessorStats::AudioMixer(self.stats.clone()),
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        let input_stream = self
            .input_streams
            .iter_mut()
            .find_map(|(&id, v)| (id == input.stream_id).then_some(v))
            .or_fail()?;
        if let Some(sample) = input.sample {
            let data = sample.expect_audio_data().or_fail()?;

            if input_stream.start_timestamp.is_none() {
                // 合成開始時刻の判断用に最初のタイムスタンプを覚えておく
                //
                // なお開始時刻に達した後は、データのタイムスタンプにギャップがあったとしても
                // 連続しているものとして扱う。
                //
                // これは Chrome を含む多くのブラウザがこの挙動なのと、
                // ギャップ部分のハンドリングは Sora 側の責務であるため。
                // 下手に Hisui 側でハンドリングしてしまうと、ギャップが
                // 極端に大きいためにあえて Sora がそのまま放置した区間を
                // 埋めようとしてディスクやメモリを食いつぶしてしまう恐れがある。
                input_stream.start_timestamp = Some(data.timestamp);
            }

            // サンプルキューに要素を追加する
            //
            // 想定外の入力が来ていないかを念のためにチェックする
            // (format と stereo については stereo_samples() の中でチェックしている)
            (data.sample_rate == SAMPLE_RATE).or_fail()?;
            input_stream
                .sample_queue
                .extend(data.stereo_samples().or_fail()?);
        } else {
            input_stream.eos = true;
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        let mut now = self.next_input_timestamp();
        while self.layout.is_in_trim_span(now) {
            self.stats
                .total_trimmed_sample_count
                .add(MIXED_AUDIO_DATA_SAMPLES as u64);
            now = self.next_input_timestamp();
        }

        // 入力が不足しているソースがないかをチェックする
        for (input_stream_id, input_stream) in &self.input_streams {
            if input_stream.eos {
                // これ以上新しい入力は来ないので待たない
                continue;
            }
            if input_stream.start_timestamp.is_some_and(|t| t < now) {
                // 開始時刻に達していない
                continue;
            }
            if input_stream.start_timestamp.is_none()
                || input_stream.sample_queue.len() < MIXED_AUDIO_DATA_SAMPLES
            {
                // 次の合成に必要なサンプル数が足りないので待つ
                return Ok(MediaProcessorOutput::Pending {
                    awaiting_stream_id: *input_stream_id,
                });
            }
        }

        // EOS 判定
        let eos = self
            .input_streams
            .values()
            .all(|s| s.eos && s.sample_queue.is_empty());
        if eos {
            return Ok(MediaProcessorOutput::Finished);
        }

        // 合成
        let (result, elapsed) = Seconds::elapsed(|| self.mix_next_audio_data(now).or_fail());

        // TODO: プロセッサ実行スレッドの導入タイミングで、時間計測はそっちに移動する
        self.stats.total_processing_seconds.add(elapsed);

        let mixed_data = result?;
        Ok(MediaProcessorOutput::Processed {
            stream_id: self.output_stream_id,
            sample: MediaSample::audio_data(mixed_data),
        })
    }
}
