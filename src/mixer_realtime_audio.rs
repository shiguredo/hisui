use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use crate::{
    Error, MediaFrame, Message, ProcessorHandle, TrackId,
    audio::converter::AudioConverterBuilder,
    audio::{AudioFormat, AudioFrame, Channels, SampleRate},
    sample_based_timestamp_aligner::{DEFAULT_REBASE_THRESHOLD, SampleBasedTimestampAligner},
};

const DEFAULT_FRAME_DURATION: Duration = Duration::from_millis(20);
const DEFAULT_TIMESTAMP_REBASE_THRESHOLD: Duration = DEFAULT_REBASE_THRESHOLD;
const DEFAULT_TERMINATE_ON_INPUT_EOS: bool = false;

#[derive(Debug, Clone)]
pub struct AudioRealtimeMixer {
    /// 出力音声のサンプルレート
    pub sample_rate: SampleRate,
    /// 出力音声のチャンネル数
    pub channels: Channels,
    /// ミキサーが 1 回の出力で生成するフレーム長
    pub frame_duration: Duration,
    /// 入力タイムスタンプとサンプル数ベース推定値の乖離がこの閾値を超えたときに、基準タイムスタンプを再設定する
    pub timestamp_rebase_threshold: Duration,
    /// すべての入力が EOS かつ内部キューが空になった時点で出力を EOS で終了する
    pub terminate_on_input_eos: bool,
    /// 合成対象の入力トラック一覧
    pub input_tracks: Vec<AudioRealtimeInputTrack>,
    /// 合成後の音声を書き出す出力トラック ID
    pub output_track_id: TrackId,
}

impl nojson::DisplayJson for AudioRealtimeMixer {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("sampleRate", self.sample_rate.get())?;
            f.member("channels", self.channels.get())?;
            f.member("frameDurationMs", self.frame_duration.as_millis())?;
            f.member(
                "timestampRebaseThresholdMs",
                self.timestamp_rebase_threshold.as_millis(),
            )?;
            f.member("terminateOnInputEos", self.terminate_on_input_eos)?;
            f.member("inputTracks", &self.input_tracks)?;
            f.member("outputTrackId", &self.output_track_id)
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for AudioRealtimeMixer {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let sample_rate_hz: Option<u32> = value.to_member("sampleRate")?.try_into()?;
        let channels_value: Option<u8> = value.to_member("channels")?.try_into()?;
        let frame_duration_ms: Option<u64> = value.to_member("frameDurationMs")?.try_into()?;
        let timestamp_rebase_threshold_ms: Option<u64> =
            value.to_member("timestampRebaseThresholdMs")?.try_into()?;
        let terminate_on_input_eos: Option<bool> =
            value.to_member("terminateOnInputEos")?.try_into()?;
        let input_tracks: Vec<AudioRealtimeInputTrack> =
            value.to_member("inputTracks")?.required()?.try_into()?;
        let output_track_id = value.to_member("outputTrackId")?.required()?.try_into()?;

        let mut seen_track_ids = HashSet::new();
        for track in &input_tracks {
            if !seen_track_ids.insert(track.track_id.clone()) {
                let error_value = value.to_member("inputTracks")?.required()?;
                return Err(
                    error_value.invalid(format!("duplicate input track ID: {}", track.track_id))
                );
            }
        }

        let sample_rate =
            SampleRate::from_u32(sample_rate_hz.unwrap_or(SampleRate::HZ_48000.get()))
                .map_err(|e| value.invalid(format!("invalid sampleRate: {}", e.display())))?;
        let channels = Channels::from_u8(channels_value.unwrap_or(Channels::STEREO.get()))
            .map_err(|e| value.invalid(format!("invalid channels: {}", e.display())))?;

        let frame_duration = Duration::from_millis(
            frame_duration_ms.unwrap_or(DEFAULT_FRAME_DURATION.as_millis() as u64),
        );
        if frame_duration.is_zero() {
            return Err(value.invalid("frameDurationMs must be greater than 0"));
        }

        let timestamp_rebase_threshold = Duration::from_millis(
            timestamp_rebase_threshold_ms
                .unwrap_or(DEFAULT_TIMESTAMP_REBASE_THRESHOLD.as_millis() as u64),
        );
        if timestamp_rebase_threshold.is_zero() {
            return Err(value.invalid("timestampRebaseThresholdMs must be greater than 0"));
        }

        validate_frame_duration(frame_duration, sample_rate)
            .map_err(|e| value.invalid(e.display()))?;

        Ok(Self {
            sample_rate,
            channels,
            frame_duration,
            timestamp_rebase_threshold,
            terminate_on_input_eos: terminate_on_input_eos
                .unwrap_or(DEFAULT_TERMINATE_ON_INPUT_EOS),
            input_tracks,
            output_track_id,
        })
    }
}

impl AudioRealtimeMixer {
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let AudioRealtimeMixer {
            sample_rate,
            channels,
            frame_duration,
            timestamp_rebase_threshold,
            terminate_on_input_eos,
            input_tracks,
            output_track_id,
        } = self;
        let config = AudioRealtimeMixerConfig::new(
            sample_rate,
            channels,
            frame_duration,
            timestamp_rebase_threshold,
            terminate_on_input_eos,
        )?;

        let mut output_tx = handle.publish_track(output_track_id).await?;
        let mut states = HashMap::with_capacity(input_tracks.len());
        let mut input_track_ids = Vec::with_capacity(input_tracks.len());
        let mut input_receivers = HashMap::with_capacity(input_tracks.len());

        let (track_event_tx, track_event_rx) = tokio::sync::mpsc::unbounded_channel();
        for input_track in input_tracks {
            states.insert(
                input_track.track_id.clone(),
                InputTrackState::new(
                    config.sample_rate,
                    config.channels,
                    config.timestamp_rebase_threshold,
                ),
            );
            input_track_ids.push(input_track.track_id.clone());
            let input_rx = handle.subscribe_track(input_track.track_id.clone());
            let receiver =
                spawn_input_receiver(input_track.track_id, input_rx, track_event_tx.clone());
            input_receivers.insert(receiver.track_id.clone(), receiver);
        }

        let (rpc_tx, rpc_rx) = tokio::sync::mpsc::unbounded_channel();
        handle
            .register_rpc_sender(rpc_tx)
            .await
            .map_err(|e| Error::new(format!("failed to register audio mixer RPC sender: {e}")))?;

        let mut stats_root = handle.stats();
        let stats = AudioRealtimeMixerStats::new(&mut stats_root);
        stats.set_current_input_track_count(input_track_ids.len());

        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        AudioRealtimeMixerRunner {
            config,
            processor_handle: handle,
            output_tx: &mut output_tx,
            states,
            input_track_ids,
            input_receivers,
            track_event_tx,
            track_event_rx: Some(track_event_rx),
            rpc_rx: Some(rpc_rx),
            next_output_timestamp: None,
            mixer_start: tokio::time::Instant::now(),
            finishing: false,
            stats,
        }
        .run()
        .await
    }
}

#[derive(Debug, Clone)]
pub struct AudioRealtimeInputTrack {
    pub track_id: TrackId,
}

impl nojson::DisplayJson for AudioRealtimeInputTrack {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| f.member("trackId", &self.track_id))
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for AudioRealtimeInputTrack {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let track_id: TrackId = value.to_member("trackId")?.required()?.try_into()?;
        Ok(Self { track_id })
    }
}

#[derive(Debug)]
pub enum AudioRealtimeMixerRpcMessage {
    UpdateInputs {
        input_tracks: Vec<AudioRealtimeInputTrack>,
        reply_tx: tokio::sync::oneshot::Sender<crate::Result<AudioRealtimeMixerUpdateInputsResult>>,
    },
    Finish {
        reply_tx: tokio::sync::oneshot::Sender<()>,
    },
}

#[derive(Debug, Clone)]
pub struct AudioRealtimeMixerUpdateInputsResult {
    pub previous_input_tracks: Vec<AudioRealtimeInputTrack>,
}

#[derive(Debug, Clone, Copy)]
struct AudioRealtimeMixerConfig {
    sample_rate: SampleRate,
    channels: Channels,
    frame_duration: Duration,
    frame_samples_per_channel: usize,
    frame_samples_total: usize,
    timestamp_rebase_threshold: Duration,
    terminate_on_input_eos: bool,
}

impl AudioRealtimeMixerConfig {
    fn new(
        sample_rate: SampleRate,
        channels: Channels,
        frame_duration: Duration,
        timestamp_rebase_threshold: Duration,
        terminate_on_input_eos: bool,
    ) -> crate::Result<Self> {
        let frame_samples_per_channel: usize =
            samples_from_duration_exact(frame_duration, sample_rate)?
                .try_into()
                .map_err(|_| crate::Error::new("frame duration is too large"))?;
        if frame_samples_per_channel == 0 {
            return Err(crate::Error::new(
                "frame duration must represent at least one sample",
            ));
        }
        let frame_samples_total =
            frame_samples_per_channel.saturating_mul(usize::from(channels.get()));
        Ok(Self {
            sample_rate,
            channels,
            frame_duration,
            frame_samples_per_channel,
            frame_samples_total,
            timestamp_rebase_threshold,
            terminate_on_input_eos,
        })
    }
}

#[derive(Debug)]
struct AudioRealtimeMixerStats {
    total_input_audio_frame_count: crate::stats::StatsCounter,
    total_output_audio_frame_count: crate::stats::StatsCounter,
    total_output_audio_duration: crate::stats::StatsDuration,
    total_output_sample_count: crate::stats::StatsCounter,
    current_input_track_count: crate::stats::StatsGauge,
    total_gap_filled_sample_count: crate::stats::StatsCounter,
    total_late_dropped_sample_count: crate::stats::StatsCounter,
    total_timestamp_rebase_count: crate::stats::StatsCounter,
}

impl AudioRealtimeMixerStats {
    fn new(stats: &mut crate::stats::Stats) -> Self {
        let total_input_audio_frame_count = stats.counter("total_input_audio_frame_count");
        let total_output_audio_frame_count = stats.counter("total_output_audio_frame_count");
        let total_output_audio_duration = stats.duration("total_output_audio_duration_seconds");
        let total_output_sample_count = stats.counter("total_output_sample_count");
        let current_input_track_count = stats.gauge("current_input_track_count");
        let total_gap_filled_sample_count = stats.counter("total_gap_filled_sample_count");
        let total_late_dropped_sample_count = stats.counter("total_late_dropped_sample_count");
        let total_timestamp_rebase_count = stats.counter("total_timestamp_rebase_count");
        stats.flag("error").set(false);
        Self {
            total_input_audio_frame_count,
            total_output_audio_frame_count,
            total_output_audio_duration,
            total_output_sample_count,
            current_input_track_count,
            total_gap_filled_sample_count,
            total_late_dropped_sample_count,
            total_timestamp_rebase_count,
        }
    }

    fn add_input_audio_frame_count(&self) {
        self.total_input_audio_frame_count.inc();
    }

    fn add_output_audio_frame_count(&self) {
        self.total_output_audio_frame_count.inc();
    }

    fn add_output_audio_duration(&self, duration: Duration) {
        self.total_output_audio_duration.add(duration);
    }

    fn add_output_sample_count(&self, value: u64) {
        self.total_output_sample_count.add(value);
    }

    fn set_current_input_track_count(&self, value: usize) {
        self.current_input_track_count.set(value as i64);
    }

    fn add_gap_filled_sample_count(&self, value: u64) {
        self.total_gap_filled_sample_count.add(value);
    }

    fn add_late_dropped_sample_count(&self, value: u64) {
        self.total_late_dropped_sample_count.add(value);
    }

    fn add_timestamp_rebase_count(&self) {
        self.total_timestamp_rebase_count.inc();
    }
}

#[derive(Debug)]
struct InputTrackState {
    converter: crate::audio::converter::AudioConverter,
    aligner: SampleBasedTimestampAligner,
    timing_initialized: bool,
    total_input_samples_per_channel: u64,
    queue_head_timestamp: Option<Duration>,
    sample_queue: VecDeque<i16>,
    eos: bool,
}

impl InputTrackState {
    fn new(sample_rate: SampleRate, channels: Channels, rebase_threshold: Duration) -> Self {
        Self {
            converter: AudioConverterBuilder::new()
                .format(AudioFormat::I16Be)
                .sample_rate(sample_rate)
                .channels(channels)
                .build(),
            aligner: SampleBasedTimestampAligner::new(sample_rate, rebase_threshold),
            timing_initialized: false,
            total_input_samples_per_channel: 0,
            queue_head_timestamp: None,
            sample_queue: VecDeque::new(),
            eos: false,
        }
    }

    fn handle_audio_frame(
        &mut self,
        frame: Arc<AudioFrame>,
        config: AudioRealtimeMixerConfig,
        stats: &AudioRealtimeMixerStats,
    ) -> crate::Result<()> {
        let frame = self.converter.convert(&frame)?;
        validate_input_audio_frame(&frame, config.sample_rate, config.channels)?;
        let mut interleaved_samples = audio_frame_to_i16_samples(&frame)?;

        let channels = usize::from(config.channels.get());
        let samples_per_channel = interleaved_samples.len() / channels;

        if self.timing_initialized {
            let predicted_timestamp = self
                .aligner
                .estimate_timestamp_from_output_samples(self.total_input_samples_per_channel);
            if predicted_timestamp.abs_diff(frame.timestamp) > config.timestamp_rebase_threshold {
                stats.add_timestamp_rebase_count();
            }
        }

        self.aligner
            .align_input_timestamp(frame.timestamp, self.total_input_samples_per_channel);
        self.timing_initialized = true;
        let aligned_timestamp = self
            .aligner
            .estimate_timestamp_from_output_samples(self.total_input_samples_per_channel);

        if self.queue_head_timestamp.is_none() {
            self.queue_head_timestamp = Some(aligned_timestamp);
        }

        let queue_head_timestamp = self
            .queue_head_timestamp
            .ok_or_else(|| Error::new("queue head timestamp is not initialized"))?;
        let queued_samples_per_channel = self.sample_queue.len() / channels;
        let queue_tail_timestamp = queue_head_timestamp.saturating_add(
            config
                .sample_rate
                .duration_from_samples(queued_samples_per_channel as u64),
        );

        if aligned_timestamp > queue_tail_timestamp {
            let gap_duration = aligned_timestamp.saturating_sub(queue_tail_timestamp);
            let gap_samples = samples_from_duration_rounded(gap_duration, config.sample_rate);
            if gap_samples > 0 {
                self.sample_queue
                    .extend(std::iter::repeat_n(0, gap_samples as usize * channels));
                stats.add_gap_filled_sample_count(gap_samples);
            }
        } else if aligned_timestamp < queue_tail_timestamp {
            let late_duration = queue_tail_timestamp.saturating_sub(aligned_timestamp);
            let mut late_samples = samples_from_duration_rounded(late_duration, config.sample_rate);
            late_samples = late_samples.min(samples_per_channel as u64);
            if late_samples > 0 {
                let drop_samples = late_samples as usize * channels;
                interleaved_samples.drain(..drop_samples);
                stats.add_late_dropped_sample_count(late_samples);
            }
        }

        self.sample_queue.extend(interleaved_samples);
        self.total_input_samples_per_channel = self
            .total_input_samples_per_channel
            .saturating_add(samples_per_channel as u64);

        stats.add_input_audio_frame_count();
        Ok(())
    }

    fn drain_samples_for_tick(
        &mut self,
        now: Duration,
        config: AudioRealtimeMixerConfig,
        stats: &AudioRealtimeMixerStats,
    ) -> Vec<i16> {
        let mut mixed = vec![0; config.frame_samples_total];
        let Some(mut queue_head_timestamp) = self.queue_head_timestamp else {
            return mixed;
        };
        if now < queue_head_timestamp {
            return mixed;
        }

        let channels = usize::from(config.channels.get());

        if queue_head_timestamp < now {
            let lag_duration = now.saturating_sub(queue_head_timestamp);
            let drop_samples = samples_from_duration_rounded(lag_duration, config.sample_rate)
                .min((self.sample_queue.len() / channels) as u64);
            if drop_samples > 0 {
                let drop_total_samples = drop_samples as usize * channels;
                self.sample_queue.drain(..drop_total_samples);
                stats.add_late_dropped_sample_count(drop_samples);
                queue_head_timestamp = queue_head_timestamp
                    .saturating_add(config.sample_rate.duration_from_samples(drop_samples));
            }
        }

        let take_samples_per_channel = config
            .frame_samples_per_channel
            .min(self.sample_queue.len() / channels);
        let take_total = take_samples_per_channel * channels;
        for (slot, sample) in mixed.iter_mut().zip(self.sample_queue.drain(..take_total)) {
            *slot = sample;
        }

        queue_head_timestamp = queue_head_timestamp.saturating_add(config.frame_duration);
        self.queue_head_timestamp = Some(queue_head_timestamp);
        mixed
    }

    fn is_empty(&self) -> bool {
        self.sample_queue.is_empty()
    }
}

#[derive(Debug)]
struct AudioRealtimeMixerRunner<'a> {
    config: AudioRealtimeMixerConfig,
    processor_handle: ProcessorHandle,
    output_tx: &'a mut crate::MessageSender,
    states: HashMap<TrackId, InputTrackState>,
    input_track_ids: Vec<TrackId>,
    input_receivers: HashMap<TrackId, InputReceiverHandle>,
    track_event_tx: tokio::sync::mpsc::UnboundedSender<TrackEvent>,
    track_event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<TrackEvent>>,
    rpc_rx: Option<tokio::sync::mpsc::UnboundedReceiver<AudioRealtimeMixerRpcMessage>>,
    next_output_timestamp: Option<Duration>,
    mixer_start: tokio::time::Instant,
    finishing: bool,
    stats: AudioRealtimeMixerStats,
}

impl AudioRealtimeMixerRunner<'_> {
    async fn run(&mut self) -> crate::Result<()> {
        let mut ticker = tokio::time::interval(self.config.frame_duration);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if !self.handle_output_tick()? {
                        break;
                    }
                }
                event = recv_or_pending(&mut self.track_event_rx) => {
                    let event = event.ok_or_else(|| {
                        Error::new("BUG: audio mixer track event channel closed unexpectedly")
                    })?;
                    self.handle_track_event(event)?;
                }
                rpc_message = recv_or_pending(&mut self.rpc_rx) => {
                    self.handle_rpc_message(rpc_message)?;
                }
            }
        }

        Ok(())
    }

    fn handle_track_event(&mut self, event: TrackEvent) -> crate::Result<()> {
        match event {
            TrackEvent::Audio { track_id, frame } => {
                let Some(state) = self.states.get_mut(&track_id) else {
                    return Ok(());
                };
                state.handle_audio_frame(frame, self.config, &self.stats)?;
            }
            TrackEvent::Eos { track_id } => {
                if let Some(state) = self.states.get_mut(&track_id) {
                    state.eos = true;
                }
            }
            TrackEvent::Syn(_syn) => {}
            TrackEvent::Error { track_id, message } => {
                if self.states.contains_key(&track_id) {
                    return Err(Error::new(format!(
                        "audio mixer input track {track_id} error: {message}"
                    )));
                }
            }
        }

        Ok(())
    }

    fn handle_rpc_message(
        &mut self,
        rpc_message: Option<AudioRealtimeMixerRpcMessage>,
    ) -> crate::Result<()> {
        let Some(rpc_message) = rpc_message else {
            self.rpc_rx = None;
            return Ok(());
        };

        match rpc_message {
            AudioRealtimeMixerRpcMessage::UpdateInputs {
                input_tracks,
                reply_tx,
            } => {
                let result = self.update_inputs(input_tracks);
                let _ = reply_tx.send(result);
            }
            AudioRealtimeMixerRpcMessage::Finish { reply_tx } => {
                self.finishing = true;
                let _ = reply_tx.send(());
            }
        }

        Ok(())
    }

    fn update_inputs(
        &mut self,
        input_tracks: Vec<AudioRealtimeInputTrack>,
    ) -> crate::Result<AudioRealtimeMixerUpdateInputsResult> {
        validate_unique_input_tracks(&input_tracks)?;

        let previous_input_tracks = self
            .input_track_ids
            .iter()
            .cloned()
            .map(|track_id| AudioRealtimeInputTrack { track_id })
            .collect();

        let requested_track_ids: HashSet<TrackId> = input_tracks
            .iter()
            .map(|input_track| input_track.track_id.clone())
            .collect();
        let removed_track_ids = self
            .input_track_ids
            .iter()
            .filter(|track_id| !requested_track_ids.contains(*track_id))
            .cloned()
            .collect::<Vec<_>>();
        for track_id in removed_track_ids {
            self.states.remove(&track_id);
            if let Some(receiver) = self.input_receivers.remove(&track_id) {
                receiver.shutdown();
            }
        }

        for input_track in &input_tracks {
            if self.states.contains_key(&input_track.track_id) {
                continue;
            }
            self.states.insert(
                input_track.track_id.clone(),
                InputTrackState::new(
                    self.config.sample_rate,
                    self.config.channels,
                    self.config.timestamp_rebase_threshold,
                ),
            );
            let input_rx = self
                .processor_handle
                .subscribe_track(input_track.track_id.clone());
            let receiver = spawn_input_receiver(
                input_track.track_id.clone(),
                input_rx,
                self.track_event_tx.clone(),
            );
            self.input_receivers
                .insert(receiver.track_id.clone(), receiver);
        }

        self.input_track_ids = input_tracks
            .into_iter()
            .map(|input_track| input_track.track_id)
            .collect();
        self.stats
            .set_current_input_track_count(self.input_track_ids.len());

        Ok(AudioRealtimeMixerUpdateInputsResult {
            previous_input_tracks,
        })
    }

    fn handle_output_tick(&mut self) -> crate::Result<bool> {
        if self.finishing || self.should_finish() {
            self.output_tx.send_eos();
            return Ok(false);
        }

        if self.next_output_timestamp.is_none() {
            self.next_output_timestamp = self.maybe_initialize_output_timestamp();
        }
        let Some(timestamp) = self.next_output_timestamp else {
            return Ok(true);
        };

        let frame = self.mix_next_audio_frame(timestamp);
        if !self.output_tx.send_audio(frame) {
            return Ok(false);
        }

        self.next_output_timestamp = Some(timestamp.saturating_add(self.config.frame_duration));
        Ok(true)
    }

    fn maybe_initialize_output_timestamp(&self) -> Option<Duration> {
        let from_inputs = self
            .states
            .values()
            .filter_map(|state| state.queue_head_timestamp)
            .min();
        // 入力フレームがない場合は mixer_start からの経過時間をタイムスタンプとして使用する
        Some(from_inputs.unwrap_or_else(|| self.mixer_start.elapsed()))
    }

    fn mix_next_audio_frame(&mut self, timestamp: Duration) -> AudioFrame {
        let mut accum = vec![0i32; self.config.frame_samples_total];
        for state in self.states.values_mut() {
            let samples = state.drain_samples_for_tick(timestamp, self.config, &self.stats);
            for (acc, sample) in accum.iter_mut().zip(samples.into_iter()) {
                *acc = acc.saturating_add(i32::from(sample));
            }
        }

        let data = accum
            .into_iter()
            .flat_map(|sample| {
                let clamped = sample.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
                clamped.to_be_bytes()
            })
            .collect::<Vec<_>>();

        self.stats.add_output_audio_frame_count();
        self.stats
            .add_output_audio_duration(self.config.frame_duration);
        self.stats
            .add_output_sample_count(self.config.frame_samples_per_channel as u64);

        AudioFrame {
            data,
            format: AudioFormat::I16Be,
            channels: self.config.channels,
            sample_rate: self.config.sample_rate,
            timestamp,
            sample_entry: None,
        }
    }

    fn should_finish(&self) -> bool {
        should_finish_with(self.config.terminate_on_input_eos, &self.states)
    }
}

/// `should_finish` のロジックを切り出した自由関数。テスト容易性のために分離。
fn should_finish_with(
    terminate_on_input_eos: bool,
    states: &HashMap<TrackId, InputTrackState>,
) -> bool {
    if !terminate_on_input_eos {
        return false;
    }
    // 入力トラックが 0 件の場合は無音を送出し続ける。
    // 後から updateAudioMixerInputs で入力が追加される可能性がある。
    if states.is_empty() {
        return false;
    }
    states.values().all(|state| state.eos && state.is_empty())
}

#[derive(Debug)]
enum TrackEvent {
    Audio {
        track_id: TrackId,
        frame: Arc<AudioFrame>,
    },
    Eos {
        track_id: TrackId,
    },
    Syn(crate::Syn),
    Error {
        track_id: TrackId,
        message: String,
    },
}

#[derive(Debug)]
struct InputReceiverHandle {
    track_id: TrackId,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl InputReceiverHandle {
    fn shutdown(mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

fn spawn_input_receiver(
    track_id: TrackId,
    mut input_rx: crate::MessageReceiver,
    event_tx: tokio::sync::mpsc::UnboundedSender<TrackEvent>,
) -> InputReceiverHandle {
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let task_track_id = track_id.clone();
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = &mut shutdown_rx => {
                    break;
                }
                message = input_rx.recv() => {
                    match message {
                        Message::Media(MediaFrame::Audio(frame)) => TrackEvent::Audio {
                            track_id: task_track_id.clone(),
                            frame,
                        },
                        Message::Media(MediaFrame::Video(_)) => TrackEvent::Error {
                            track_id: task_track_id.clone(),
                            message: "expected audio sample, got video sample".to_owned(),
                        },
                        Message::Eos => {
                            let _ = event_tx.send(TrackEvent::Eos {
                                track_id: task_track_id.clone(),
                            });
                            break;
                        }
                        Message::Syn(syn) => TrackEvent::Syn(syn),
                    }
                }
            };

            if event_tx.send(event).is_err() {
                break;
            }
        }
    });

    InputReceiverHandle {
        track_id,
        shutdown_tx: Some(shutdown_tx),
    }
}

async fn recv_or_pending<T>(rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<T>>) -> Option<T> {
    if let Some(rx) = rx {
        rx.recv().await
    } else {
        std::future::pending().await
    }
}

fn validate_unique_input_tracks(input_tracks: &[AudioRealtimeInputTrack]) -> crate::Result<()> {
    let mut seen_track_ids = HashSet::new();
    for input_track in input_tracks {
        if !seen_track_ids.insert(input_track.track_id.clone()) {
            return Err(Error::new(format!(
                "duplicate input track ID: {}",
                input_track.track_id
            )));
        }
    }
    Ok(())
}

fn validate_frame_duration(frame_duration: Duration, sample_rate: SampleRate) -> crate::Result<()> {
    let _ = samples_from_duration_exact(frame_duration, sample_rate)?;
    Ok(())
}

fn samples_from_duration_exact(duration: Duration, sample_rate: SampleRate) -> crate::Result<u64> {
    let ns = duration.as_nanos();
    let rate = u128::from(sample_rate.get());
    let numerator = ns.saturating_mul(rate);
    let denominator = 1_000_000_000u128;
    if numerator % denominator != 0 {
        return Err(Error::new(
            "frame duration must align with sample rate without fractional samples",
        ));
    }
    let samples = numerator / denominator;
    if samples > u128::from(u64::MAX) {
        return Err(Error::new("frame duration is too large"));
    }
    Ok(samples as u64)
}

fn samples_from_duration_rounded(duration: Duration, sample_rate: SampleRate) -> u64 {
    let ns = duration.as_nanos();
    let rate = u128::from(sample_rate.get());
    let numerator = ns.saturating_mul(rate).saturating_add(500_000_000);
    let samples = numerator / 1_000_000_000u128;
    samples.min(u128::from(u64::MAX)) as u64
}

fn validate_input_audio_frame(
    frame: &AudioFrame,
    sample_rate: SampleRate,
    channels: Channels,
) -> crate::Result<()> {
    if frame.format != AudioFormat::I16Be {
        return Err(Error::new(format!(
            "unsupported input audio format: expected I16Be, got {}",
            frame.format
        )));
    }
    if frame.sample_rate != sample_rate {
        return Err(Error::new(format!(
            "unsupported input audio sample rate: expected {}, got {}",
            sample_rate.get(),
            frame.sample_rate.get()
        )));
    }
    if frame.channels != channels {
        return Err(Error::new(format!(
            "unsupported input audio channels: expected {}, got {}",
            channels.get(),
            frame.channels.get()
        )));
    }
    if !frame.data.len().is_multiple_of(2) {
        return Err(Error::new("invalid I16Be audio data length"));
    }
    let sample_count_total = frame.data.len() / 2;
    if !sample_count_total.is_multiple_of(usize::from(channels.get())) {
        return Err(Error::new("invalid interleaved audio sample count"));
    }
    Ok(())
}

fn audio_frame_to_i16_samples(frame: &AudioFrame) -> crate::Result<Vec<i16>> {
    if !frame.data.len().is_multiple_of(2) {
        return Err(Error::new("invalid I16Be audio data length"));
    }
    Ok(frame
        .data
        .chunks_exact(2)
        .map(|c| i16::from_be_bytes([c[0], c[1]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(
        timestamp: Duration,
        samples_per_channel: usize,
        sample_value: i16,
        sample_rate: SampleRate,
        channels: Channels,
    ) -> AudioFrame {
        let mut data = Vec::new();
        for _ in 0..samples_per_channel {
            if channels.is_stereo() {
                data.extend_from_slice(&sample_value.to_be_bytes());
                data.extend_from_slice(&sample_value.to_be_bytes());
            } else {
                data.extend_from_slice(&sample_value.to_be_bytes());
            }
        }
        AudioFrame {
            data,
            format: AudioFormat::I16Be,
            channels,
            sample_rate,
            timestamp,
            sample_entry: None,
        }
    }

    fn test_config() -> AudioRealtimeMixerConfig {
        AudioRealtimeMixerConfig::new(
            SampleRate::HZ_48000,
            Channels::STEREO,
            Duration::from_millis(20),
            Duration::from_millis(100),
            false,
        )
        .expect("valid config")
    }

    #[test]
    fn mixer_json_parse_defaults() {
        let mixer = crate::json::parse_str::<AudioRealtimeMixer>(
            r#"
{
  "inputTracks": [{"trackId": "audio-input"}],
  "outputTrackId": "audio-output"
}
        "#,
        )
        .expect("parse must succeed");

        assert_eq!(mixer.sample_rate, SampleRate::HZ_48000);
        assert_eq!(mixer.channels, Channels::STEREO);
        assert_eq!(mixer.frame_duration, Duration::from_millis(20));
        assert_eq!(mixer.timestamp_rebase_threshold, Duration::from_millis(100));
        assert!(!mixer.terminate_on_input_eos);
        assert_eq!(mixer.input_tracks.len(), 1);
    }

    #[test]
    fn mixer_json_parse_rejects_zero_frame_duration() {
        let result = crate::json::parse_str::<AudioRealtimeMixer>(
            r#"
{
  "frameDurationMs": 0,
  "inputTracks": [{"trackId": "audio-input"}],
  "outputTrackId": "audio-output"
}
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn mixer_json_parse_rejects_non_integral_samples_per_frame_duration() {
        let result = crate::json::parse_str::<AudioRealtimeMixer>(
            r#"
{
  "sampleRate": 44100,
  "frameDurationMs": 1,
  "inputTracks": [{"trackId": "audio-input"}],
  "outputTrackId": "audio-output"
}
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn mixer_json_parse_rejects_duplicate_track_ids() {
        let result = crate::json::parse_str::<AudioRealtimeMixer>(
            r#"
{
  "inputTracks": [{"trackId": "audio-input"}, {"trackId": "audio-input"}],
  "outputTrackId": "audio-output"
}
        "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn input_track_state_fills_gap_with_silence() {
        let config = test_config();
        let mut root_stats = crate::stats::Stats::new();
        let stats = AudioRealtimeMixerStats::new(&mut root_stats);
        let mut state = InputTrackState::new(
            config.sample_rate,
            config.channels,
            config.timestamp_rebase_threshold,
        );

        state
            .handle_audio_frame(
                Arc::new(make_frame(
                    Duration::from_millis(0),
                    config.frame_samples_per_channel,
                    100,
                    config.sample_rate,
                    config.channels,
                )),
                config,
                &stats,
            )
            .expect("first frame");
        state
            .handle_audio_frame(
                Arc::new(make_frame(
                    Duration::from_millis(220),
                    config.frame_samples_per_channel,
                    100,
                    config.sample_rate,
                    config.channels,
                )),
                config,
                &stats,
            )
            .expect("second frame");

        assert_eq!(stats.total_gap_filled_sample_count.get(), 9600);
    }

    #[test]
    fn input_track_state_drops_late_samples() {
        let config = test_config();
        let mut root_stats = crate::stats::Stats::new();
        let stats = AudioRealtimeMixerStats::new(&mut root_stats);
        let mut state = InputTrackState::new(
            config.sample_rate,
            config.channels,
            config.timestamp_rebase_threshold,
        );

        state
            .handle_audio_frame(
                Arc::new(make_frame(
                    Duration::from_millis(200),
                    config.frame_samples_per_channel,
                    100,
                    config.sample_rate,
                    config.channels,
                )),
                config,
                &stats,
            )
            .expect("first frame");
        state
            .handle_audio_frame(
                Arc::new(make_frame(
                    Duration::from_millis(0),
                    config.frame_samples_per_channel,
                    100,
                    config.sample_rate,
                    config.channels,
                )),
                config,
                &stats,
            )
            .expect("second frame");

        assert_eq!(stats.total_late_dropped_sample_count.get(), 960);
    }

    #[test]
    fn input_track_state_counts_timestamp_rebase() {
        let config = test_config();
        let mut root_stats = crate::stats::Stats::new();
        let stats = AudioRealtimeMixerStats::new(&mut root_stats);
        let mut state = InputTrackState::new(
            config.sample_rate,
            config.channels,
            config.timestamp_rebase_threshold,
        );

        state
            .handle_audio_frame(
                Arc::new(make_frame(
                    Duration::from_millis(0),
                    config.frame_samples_per_channel,
                    100,
                    config.sample_rate,
                    config.channels,
                )),
                config,
                &stats,
            )
            .expect("first frame");
        state
            .handle_audio_frame(
                Arc::new(make_frame(
                    Duration::from_millis(300),
                    config.frame_samples_per_channel,
                    100,
                    config.sample_rate,
                    config.channels,
                )),
                config,
                &stats,
            )
            .expect("second frame");

        assert_eq!(stats.total_timestamp_rebase_count.get(), 1);
    }

    fn make_eos_empty_state(config: AudioRealtimeMixerConfig) -> InputTrackState {
        let mut state = InputTrackState::new(
            config.sample_rate,
            config.channels,
            config.timestamp_rebase_threshold,
        );
        state.eos = true;
        state
    }

    fn make_active_state(config: AudioRealtimeMixerConfig) -> InputTrackState {
        InputTrackState::new(
            config.sample_rate,
            config.channels,
            config.timestamp_rebase_threshold,
        )
    }

    #[test]
    fn should_finish_with_empty_inputs_returns_false() {
        // 入力トラックが 0 件のとき、terminate_on_input_eos = true でも終了しない
        let states = HashMap::new();
        assert!(!should_finish_with(true, &states));
    }

    #[test]
    fn should_finish_with_terminate_disabled_returns_false() {
        // terminate_on_input_eos = false なら、EOS 済み入力があっても終了しない
        let config = test_config();
        let mut states = HashMap::new();
        states.insert(TrackId::new("t1"), make_eos_empty_state(config));
        assert!(!should_finish_with(false, &states));
    }

    #[test]
    fn should_finish_with_all_eos_returns_true() {
        // 全入力が EOS かつキューが空なら終了する
        let config = test_config();
        let mut states = HashMap::new();
        states.insert(TrackId::new("t1"), make_eos_empty_state(config));
        states.insert(TrackId::new("t2"), make_eos_empty_state(config));
        assert!(should_finish_with(true, &states));
    }

    #[test]
    fn should_finish_with_partial_eos_returns_false() {
        // 一部の入力のみ EOS の場合は終了しない
        let config = test_config();
        let mut states = HashMap::new();
        states.insert(TrackId::new("t1"), make_eos_empty_state(config));
        states.insert(TrackId::new("t2"), make_active_state(config));
        assert!(!should_finish_with(true, &states));
    }
}

pub async fn update_audio_mixer_inputs(
    handle: &crate::MediaPipelineHandle,
    processor_id: crate::ProcessorId,
    input_tracks: Vec<AudioRealtimeInputTrack>,
) -> Result<Vec<AudioRealtimeInputTrack>, crate::PipelineOperationError> {
    let sender = handle
        .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<AudioRealtimeMixerRpcMessage>>(
            &processor_id,
        )
        .await
        .map_err(|e| map_rpc_sender_error(e, &processor_id, "audio mixer input"))?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    sender
        .send(AudioRealtimeMixerRpcMessage::UpdateInputs {
            input_tracks,
            reply_tx,
        })
        .map_err(|_| {
            crate::PipelineOperationError::InternalError(
                "audio mixer RPC sender channel is closed".to_owned(),
            )
        })?;
    let result = reply_rx.await.map_err(|_| {
        crate::PipelineOperationError::InternalError(
            "audio mixer RPC response channel is closed".to_owned(),
        )
    })?;
    let result = result.map_err(|e| crate::PipelineOperationError::InvalidParams(e.display()))?;
    Ok(result.previous_input_tracks)
}

pub async fn finish_audio_mixer(
    handle: &crate::MediaPipelineHandle,
    processor_id: crate::ProcessorId,
) -> Result<(), crate::PipelineOperationError> {
    let sender = handle
        .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<AudioRealtimeMixerRpcMessage>>(
            &processor_id,
        )
        .await
        .map_err(|e| map_rpc_sender_error(e, &processor_id, "audio mixer"))?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    sender
        .send(AudioRealtimeMixerRpcMessage::Finish { reply_tx })
        .map_err(|_| {
            crate::PipelineOperationError::InternalError(
                "audio mixer RPC sender channel is closed".to_owned(),
            )
        })?;
    reply_rx.await.map_err(|_| {
        crate::PipelineOperationError::InternalError(
            "audio mixer RPC response channel is closed".to_owned(),
        )
    })?;
    Ok(())
}

fn map_rpc_sender_error(
    e: crate::media_pipeline::GetProcessorRpcSenderError,
    processor_id: &crate::ProcessorId,
    component: &str,
) -> crate::PipelineOperationError {
    match e {
        crate::media_pipeline::GetProcessorRpcSenderError::PipelineTerminated => {
            crate::PipelineOperationError::PipelineTerminated
        }
        crate::media_pipeline::GetProcessorRpcSenderError::ProcessorNotFound => {
            crate::PipelineOperationError::InvalidParams(format!(
                "processorId not found: {processor_id}"
            ))
        }
        crate::media_pipeline::GetProcessorRpcSenderError::SenderNotRegistered
        | crate::media_pipeline::GetProcessorRpcSenderError::TypeMismatch => {
            crate::PipelineOperationError::InvalidParams(format!(
                "processor does not support {component} updates: {processor_id}"
            ))
        }
    }
}
