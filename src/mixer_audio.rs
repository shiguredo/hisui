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
    metadata::SourceId,
    stats::{AudioMixerStats, MixerStats, Seconds, SharedStats},
};

pub const MIXED_AUDIO_DATA_DURATION: Duration = Duration::from_millis(20);
const MIXED_AUDIO_DATA_SAMPLES: usize = 960;

#[derive(Debug)]
pub struct AudioMixerThread {
    layout: Layout,
    input_rxs: Vec<AudioDataReceiver>,
    output_tx: AudioDataSyncSender,
    input_sample_queues: HashMap<SourceId, VecDeque<(i16, i16)>>,
    stats: AudioMixerStats,
}

impl AudioMixerThread {
    pub fn start(
        error_flag: ErrorFlag,
        layout: Layout,
        input_rxs: Vec<AudioDataReceiver>,
        shared_stats: SharedStats,
    ) -> AudioDataReceiver {
        let (tx, rx) = channel::sync_channel();
        let mut this = Self {
            layout,
            input_rxs,
            output_tx: tx,
            input_sample_queues: HashMap::new(),
            stats: AudioMixerStats::default(),
        };
        std::thread::spawn(move || {
            log::debug!("audio mixer started");
            if let Err(e) = this.run().or_fail() {
                error_flag.set();
                this.stats.error.set(true);
                log::error!("failed to mix audio sources: {e}");
            }
            log::debug!("audio mixer finished");

            shared_stats.with_lock(|stats| {
                stats.mixers.push(MixerStats::Audio(this.stats));
            });
        });
        rx
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

    fn run(&mut self) -> orfail::Result<()> {
        while let Some(data) = self.next_data().or_fail()? {
            if !self.output_tx.send(data) {
                // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                log::info!("receiver of mixed audio stream has been closed");
                break;
            }
        }
        Ok(())
    }

    fn fill_input_queue(
        &mut self,
        now: Duration,
        input_rx: &mut channel::Receiver<AudioData>,
    ) -> orfail::Result<()> {
        while let Some(data) = input_rx.peek() {
            let source_id = data.source_id.as_ref().or_fail()?;

            if !self.input_sample_queues.contains_key(source_id) && now < data.timestamp {
                // まだ再生時刻に達していない
                break;
            } else if !self.input_sample_queues.contains_key(source_id) {
                // 再生時刻に達した
                //
                // 以後は、データのタイムスタンプにギャップがあったとしても
                // 連続しているものとして扱う
                // (Chrome を含む多くのブラウザがこの挙動なのと、
                // ギャップ部分のハンドリングは Sora 側の責務であるため。
                // 下手に Hisui 側でハンドリングしてしまうと、ギャップが
                // 極端に大きいためにあえて Sora がそのまま放置した区間を
                // 埋めようとしてディスクやメモリを食いつぶしてしまう恐れがある)
                self.input_sample_queues
                    .insert(source_id.clone(), VecDeque::new());
            }

            // サンプルキューに要素を追加する
            //
            // 想定外の入力が来ていないかを念のためにチェックする
            // (format と stereo については stereo_samples() の中でチェックしている)
            (data.sample_rate == SAMPLE_RATE).or_fail()?;

            let queue = self.input_sample_queues.get_mut(source_id).or_fail()?;
            queue.extend(data.stereo_samples().or_fail()?);

            // 処理した要素を取りだす
            let _ = input_rx.recv();
            self.stats.total_input_audio_data_count.increment();

            if queue.len() >= MIXED_AUDIO_DATA_SAMPLES {
                // 次の合成処理に必要な分のサンプルは溜った
                break;
            }
        }
        Ok(())
    }

    fn next_data(&mut self) -> orfail::Result<Option<AudioData>> {
        let mut now = self.next_input_timestamp();
        while self.layout.is_in_trim_span(now) {
            self.stats
                .total_trimmed_sample_count
                .add(MIXED_AUDIO_DATA_SAMPLES as u64);
            now = self.next_input_timestamp();
        }

        for mut input_rx in std::mem::take(&mut self.input_rxs) {
            self.fill_input_queue(now, &mut input_rx).or_fail()?;
            if input_rx.peek().is_some() {
                self.input_rxs.push(input_rx);
            }
        }

        if self.is_eos() {
            // 全部のソースが終端に達した
            return Ok(None);
        }

        let (result, elapsed) = Seconds::elapsed(|| self.mix_next_audio_data().or_fail().map(Some));
        self.stats.total_processing_seconds.add(elapsed);
        result
    }

    fn is_eos(&self) -> bool {
        self.input_rxs.is_empty()
            && self
                .input_sample_queues
                .values()
                .all(|queue| queue.is_empty())
    }

    fn mix_next_audio_data(&mut self) -> orfail::Result<AudioData> {
        let timestamp = self.next_output_timestamp();

        let bytes_per_sample = CHANNELS as usize * 2; // i16 で表現するので *2
        let mut mixed_samples = Vec::with_capacity(MIXED_AUDIO_DATA_SAMPLES * bytes_per_sample);

        let mut filled = true; // 無音補完されたかどうか
        for _ in 0..MIXED_AUDIO_DATA_SAMPLES {
            let mut acc_left = 0;
            let mut acc_right = 0;
            for queue in self.input_sample_queues.values_mut() {
                let Some((left, right)) = queue.pop_front() else {
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

        self.stats.total_output_audio_data_count.increment();
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
