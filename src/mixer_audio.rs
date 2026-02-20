use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use orfail::OrFail;

use crate::{
    Error, Message, ProcessorHandle, Result, TrackId,
    audio::{AudioData, AudioFormat, CHANNELS, SAMPLE_RATE},
    layout::TrimSpans,
    media::{MediaSample, MediaStreamId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
};

pub const MIXED_AUDIO_DATA_DURATION: Duration = Duration::from_millis(20);
const MIXED_AUDIO_DATA_SAMPLES: usize = 960;

#[derive(Debug, Default)]
struct InputStream {
    eos: bool,
    sample_queue: VecDeque<(i16, i16)>,
    start_timestamp: Option<Duration>,
}

#[derive(Debug)]
pub struct AudioMixer {
    trim_spans: TrimSpans,
    input_stream_ids: Vec<MediaStreamId>,
    input_streams: HashMap<MediaStreamId, InputStream>,
    output_stream_id: MediaStreamId,
    stats: AudioMixerStats,
}

#[derive(Debug)]
pub struct AudioMixerStats {
    total_input_audio_data_count: crate::stats::StatsCounter,
    total_output_audio_data_count: crate::stats::StatsCounter,
    total_output_audio_data_duration: crate::stats::StatsDuration,
    total_output_sample_count: crate::stats::StatsCounter,
    total_output_filled_sample_count: crate::stats::StatsCounter,
    total_trimmed_sample_count: crate::stats::StatsCounter,
    error: crate::stats::StatsFlag,
}

impl AudioMixerStats {
    fn new(stats: &mut crate::stats::Stats) -> Self {
        let total_input_audio_data_count = stats.counter("total_input_audio_data_count");
        let total_output_audio_data_count = stats.counter("total_output_audio_data_count");
        let total_output_audio_data_duration = stats.duration("total_output_audio_data_seconds");
        let total_output_sample_count = stats.counter("total_output_sample_count");
        let total_output_filled_sample_count = stats.counter("total_output_filled_sample_count");
        let total_trimmed_sample_count = stats.counter("total_trimmed_sample_count");
        let error = stats.flag("error");
        error.set(false);
        Self {
            total_input_audio_data_count,
            total_output_audio_data_count,
            total_output_audio_data_duration,
            total_output_sample_count,
            total_output_filled_sample_count,
            total_trimmed_sample_count,
            error,
        }
    }

    fn add_input_audio_data_count(&self) {
        self.total_input_audio_data_count.inc();
    }

    fn add_output_audio_data_count(&self) {
        self.total_output_audio_data_count.inc();
    }

    fn add_output_audio_data_duration(&self, duration: Duration) {
        self.total_output_audio_data_duration.add(duration);
    }

    fn add_output_sample_count(&self, value: u64) {
        self.total_output_sample_count.add(value);
    }

    fn add_output_filled_sample_count(&self, value: u64) {
        self.total_output_filled_sample_count.add(value);
    }

    fn add_trimmed_sample_count(&self, value: u64) {
        self.total_trimmed_sample_count.add(value);
    }

    fn set_error(&self) {
        self.error.set(true);
    }

    pub fn total_input_audio_data_count(&self) -> u64 {
        self.total_input_audio_data_count.get()
    }

    pub fn total_output_audio_data_count(&self) -> u64 {
        self.total_output_audio_data_count.get()
    }

    pub fn total_output_audio_data_duration(&self) -> Duration {
        self.total_output_audio_data_duration.get()
    }

    pub fn total_output_sample_count(&self) -> u64 {
        self.total_output_sample_count.get()
    }

    pub fn total_output_filled_sample_count(&self) -> u64 {
        self.total_output_filled_sample_count.get()
    }

    pub fn total_trimmed_sample_count(&self) -> u64 {
        self.total_trimmed_sample_count.get()
    }
}

impl AudioMixer {
    pub fn new(
        trim_spans: TrimSpans,
        input_stream_ids: Vec<MediaStreamId>,
        output_stream_id: MediaStreamId,
        mut stats: crate::stats::Stats,
    ) -> Self {
        let input_streams = input_stream_ids
            .iter()
            .copied()
            .map(|id| (id, InputStream::default()))
            .collect();
        let stats = AudioMixerStats::new(&mut stats);
        Self {
            trim_spans,
            input_stream_ids,
            input_streams,
            output_stream_id,
            stats,
        }
    }

    pub fn stats(&self) -> &AudioMixerStats {
        &self.stats
    }

    pub async fn run(
        mut self,
        handle: ProcessorHandle,
        input_track_ids: Vec<TrackId>,
        output_track_id: TrackId,
    ) -> Result<()> {
        if input_track_ids.len() != self.input_stream_ids.len() {
            return Err(Error::new(format!(
                "input track count mismatch: expected {}, got {}",
                self.input_stream_ids.len(),
                input_track_ids.len()
            )));
        }

        let mut input_tracks = self
            .input_stream_ids
            .iter()
            .copied()
            .zip(input_track_ids.into_iter())
            .map(|(stream_id, track_id)| InputTrack {
                stream_id,
                track_id: track_id.clone(),
                rx: handle.subscribe_track(track_id),
            })
            .collect::<Vec<_>>();
        let mut output_tx = handle.publish_track(output_track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        loop {
            match self
                .process_output()
                .map_err(|e| Error::new(e.to_string()))?
            {
                MediaProcessorOutput::Processed { sample, .. } => {
                    if !output_tx.send_media(sample) {
                        break;
                    }
                }
                MediaProcessorOutput::Pending { awaiting_stream_id } => {
                    let Some(stream_id) = awaiting_stream_id else {
                        return Err(Error::new("audio mixer returned pending without stream id"));
                    };
                    let input_track = input_tracks
                        .iter_mut()
                        .find(|track| track.stream_id == stream_id)
                        .ok_or_else(|| {
                            Error::new(format!(
                                "audio mixer is waiting for unknown stream id: {}",
                                stream_id.get()
                            ))
                        })?;
                    let track_id = input_track.track_id.clone();
                    let message = input_track.rx.recv().await;
                    self.handle_input_message(stream_id, &track_id, message)?;
                }
                MediaProcessorOutput::Finished => {
                    output_tx.send_eos();
                    break;
                }
            }
        }

        Ok(())
    }

    fn next_input_timestamp(&self) -> Duration {
        Duration::from_secs(
            self.stats.total_output_sample_count() + self.stats.total_trimmed_sample_count(),
        ) / SAMPLE_RATE as u32
    }

    fn next_output_timestamp(&self) -> Duration {
        Duration::from_secs(self.stats.total_output_sample_count()) / SAMPLE_RATE as u32
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
                if stream.start_timestamp.is_none_or(|t| now < t) {
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

        self.stats.add_output_audio_data_count();
        self.stats
            .add_output_audio_data_duration(MIXED_AUDIO_DATA_DURATION);
        self.stats
            .add_output_sample_count(MIXED_AUDIO_DATA_SAMPLES as u64);
        if filled {
            self.stats
                .add_output_filled_sample_count(MIXED_AUDIO_DATA_SAMPLES as u64);
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

    fn handle_input_message(
        &mut self,
        stream_id: MediaStreamId,
        track_id: &TrackId,
        message: Message,
    ) -> Result<()> {
        match message {
            Message::Media(MediaSample::Audio(sample)) => self
                .process_input(MediaProcessorInput::sample(
                    stream_id,
                    MediaSample::Audio(sample),
                ))
                .map_err(|e| Error::new(e.to_string())),
            Message::Media(MediaSample::Video(_)) => Err(Error::new(format!(
                "expected an audio sample on track {}, but got a video sample",
                track_id.get()
            ))),
            Message::Eos => self
                .process_input(MediaProcessorInput::eos(stream_id))
                .map_err(|e| Error::new(e.to_string())),
            Message::Syn(_) => Ok(()),
        }
    }
}

impl MediaProcessor for AudioMixer {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self.input_stream_ids.clone(),
            output_stream_ids: vec![self.output_stream_id],
            workload_hint: MediaProcessorWorkloadHint::AUDIO_MIXER,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        let input_stream = self.input_streams.get_mut(&input.stream_id).or_fail()?;
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

            self.stats.add_input_audio_data_count();
        } else {
            input_stream.eos = true;
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        let mut now = self.next_input_timestamp();
        while self.trim_spans.contains(now) {
            self.stats
                .add_trimmed_sample_count(MIXED_AUDIO_DATA_SAMPLES as u64);
            now = self.next_input_timestamp();
        }

        // 入力が不足しているソースがないかをチェックする
        for (input_stream_id, input_stream) in &self.input_streams {
            if input_stream.eos {
                // これ以上新しい入力は来ないので待たない
                continue;
            }
            if input_stream.sample_queue.len() < MIXED_AUDIO_DATA_SAMPLES {
                // 次の合成に必要なサンプル数が足りないので待つ
                return Ok(MediaProcessorOutput::Pending {
                    awaiting_stream_id: Some(*input_stream_id),
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
        let mixed_data = self.mix_next_audio_data(now).or_fail()?;

        Ok(MediaProcessorOutput::Processed {
            stream_id: self.output_stream_id,
            sample: MediaSample::audio_data(mixed_data),
        })
    }

    fn set_error(&self) {
        self.stats.set_error();
    }
}

#[derive(Debug)]
struct InputTrack {
    stream_id: MediaStreamId,
    track_id: TrackId,
    rx: crate::MessageReceiver,
}
