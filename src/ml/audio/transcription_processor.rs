//! MediaPipeline 上の Whisper 文字起こし processor。

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::audio::{AudioFormat, AudioFrame, Channels, SampleRate, resample::resample_to_mono};
use crate::media::MediaFrame;
use crate::ml::audio::config::VadConfig;
use crate::ml::audio::silero_vad::SileroVadModel;
use crate::ml::audio::transcription_service::{
    TranscriptRequest, TranscriptResult, TranscriptionService,
};
use crate::ml::audio::vad::{SpeechSegment, VadGate};
use crate::text::TextFrame;
use crate::{Message, ProcessorHandle, TrackId};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const MAX_TRANSCRIPT_SAMPLES: usize = 30 * TARGET_SAMPLE_RATE as usize;
const MIN_TRANSCRIPT_SAMPLES: usize = 160;

struct QueuedChunk {
    start_sample: u64,
    end_sample: u64,
    pcm: Vec<f32>,
}

struct PendingTranscript {
    start: Duration,
    end: Duration,
    result_task: JoinHandle<crate::Result<TranscriptResult>>,
}

/// 1 audio track を 1 text track に変換する processor。
pub struct TranscriptionProcessor {
    service: Arc<TranscriptionService>,
    silero: Arc<SileroVadModel>,
    language: String,
    input_track_id: TrackId,
    output_track_id: TrackId,
}

impl TranscriptionProcessor {
    pub fn new(
        service: Arc<TranscriptionService>,
        silero: Arc<SileroVadModel>,
        language: String,
        input_track_id: TrackId,
        output_track_id: TrackId,
    ) -> Self {
        Self {
            service,
            silero,
            language,
            input_track_id,
            output_track_id,
        }
    }

    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let mut input_rx = handle.subscribe_track(self.input_track_id);
        let mut output_tx = handle.publish_track(self.output_track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        let mut state = ProcessorState::new(self.service, self.silero, self.language);
        let mut eos_received = false;

        loop {
            if eos_received {
                while let Some(pending) = state.pending.pop_front() {
                    state.publish_completed(pending, &mut output_tx).await?;
                }
                output_tx.send_eos();
                return Ok(());
            }

            if state.pending.is_empty() {
                let message = input_rx.recv().await;
                eos_received = state.handle_message(message, &mut output_tx).await?;
                continue;
            }

            let pending_result = &mut state
                .pending
                .front_mut()
                .expect("pending queue must not be empty")
                .result_task;
            tokio::select! {
                message = input_rx.recv() => {
                    eos_received = state.handle_message(message, &mut output_tx).await?;
                }
                result = pending_result => {
                    let mut pending = state.pending.pop_front().expect("pending queue head exists");
                    pending.result_task = tokio::spawn(async move {
                        result.map_err(crate::Error::from)?
                    });
                    state.publish_completed(pending, &mut output_tx).await?;
                }
            }
        }
    }
}

struct ProcessorState {
    service: Arc<TranscriptionService>,
    vad: VadGate,
    language: String,
    input_sample_rate: Option<SampleRate>,
    input_channels: Option<Channels>,
    base_offset: Option<Duration>,
    input_buffer: Vec<f32>,
    retained_pcm16k: Vec<f32>,
    retained_start_sample: u64,
    pending: VecDeque<PendingTranscript>,
}

impl ProcessorState {
    fn new(
        service: Arc<TranscriptionService>,
        silero: Arc<SileroVadModel>,
        language: String,
    ) -> Self {
        Self {
            service,
            vad: VadGate::new(silero.new_instance(), VadConfig::default()),
            language,
            input_sample_rate: None,
            input_channels: None,
            base_offset: None,
            input_buffer: Vec::new(),
            retained_pcm16k: Vec::new(),
            retained_start_sample: 0,
            pending: VecDeque::new(),
        }
    }

    async fn handle_message(
        &mut self,
        message: Message,
        output_tx: &mut crate::TrackPublisher,
    ) -> crate::Result<bool> {
        match message {
            Message::Media(frame) => {
                let frame = frame.expect_audio()?;
                self.handle_audio_frame(&frame).await?;
                Ok(false)
            }
            Message::Eos => {
                self.flush_end_of_stream().await?;
                while let Some(pending) = self.pending.pop_front() {
                    self.publish_completed(pending, output_tx).await?;
                }
                Ok(true)
            }
            Message::Syn(_) => Ok(false),
        }
    }

    async fn handle_audio_frame(&mut self, frame: &AudioFrame) -> crate::Result<()> {
        self.ensure_audio_format(frame)?;
        self.base_offset.get_or_insert(frame.timestamp);
        self.remember_audio_layout(frame)?;
        let normalized = normalize_i16be(frame)?;
        self.input_buffer.extend(normalized);

        let samples_per_second =
            usize::try_from(
                self.input_sample_rate
                    .expect("sample rate initialized")
                    .get(),
            )? * usize::from(self.input_channels.expect("channels initialized").get());
        while self.input_buffer.len() >= samples_per_second {
            let chunk: Vec<f32> = self.input_buffer.drain(..samples_per_second).collect();
            let resampled = resample_to_mono(
                &chunk,
                self.input_sample_rate.expect("sample rate initialized"),
                SampleRate::from_u32(TARGET_SAMPLE_RATE)?,
                self.input_channels.expect("channels initialized"),
            )?;
            self.handle_resampled_pcm(&resampled).await?;
        }
        Ok(())
    }

    async fn flush_end_of_stream(&mut self) -> crate::Result<()> {
        if !self.input_buffer.is_empty() {
            let resampled = resample_to_mono(
                &self.input_buffer,
                self.input_sample_rate.ok_or_else(|| {
                    crate::Error::new("EOS reached before any audio frame was received")
                })?,
                SampleRate::from_u32(TARGET_SAMPLE_RATE)?,
                self.input_channels.ok_or_else(|| {
                    crate::Error::new("EOS reached before any audio frame was received")
                })?,
            )?;
            self.input_buffer.clear();
            self.handle_resampled_pcm(&resampled).await?;
        }

        for segment in self.vad.flush()? {
            self.queue_segment(segment).await?;
        }
        Ok(())
    }

    async fn handle_resampled_pcm(&mut self, pcm16k: &[f32]) -> crate::Result<()> {
        if pcm16k.is_empty() {
            return Ok(());
        }
        self.retained_pcm16k.extend_from_slice(pcm16k);
        for segment in self.vad.feed(pcm16k)? {
            self.queue_segment(segment).await?;
        }
        self.drop_consumed_pcm();
        Ok(())
    }

    async fn queue_segment(&mut self, segment: SpeechSegment) -> crate::Result<()> {
        let relative_start = usize::try_from(segment.start_sample - self.retained_start_sample)?;
        let relative_end = usize::try_from(segment.end_sample - self.retained_start_sample)?;
        let segment_pcm = &self.retained_pcm16k[relative_start..relative_end];
        for chunk in split_segment_pcm(segment.start_sample, segment_pcm) {
            self.submit_chunk(chunk).await?;
        }
        Ok(())
    }

    async fn submit_chunk(&mut self, chunk: QueuedChunk) -> crate::Result<()> {
        let request = TranscriptRequest {
            pcm: chunk.pcm,
            language: self.language.clone(),
        };
        let start = self.segment_time(chunk.start_sample)?;
        let end = self.segment_time(chunk.end_sample)?;
        let rx = self.service.submit(request).await;
        let task = tokio::spawn(async move {
            rx.await.map_err(|e| {
                crate::Error::new(format!("transcription result channel closed: {e}"))
            })?
        });
        self.pending.push_back(PendingTranscript {
            start,
            end,
            result_task: task,
        });
        Ok(())
    }

    async fn publish_completed(
        &mut self,
        pending: PendingTranscript,
        output_tx: &mut crate::TrackPublisher,
    ) -> crate::Result<()> {
        let result = pending.result_task.await??;
        if result.text.is_empty() {
            return Ok(());
        }

        let frame = TextFrame {
            start: pending.start,
            end: pending.end,
            text: result.text,
            language: result.language,
            no_speech_prob: Some(result.no_speech_prob),
            avg_logprob: Some(result.avg_logprob),
        };
        if !output_tx.send_media(MediaFrame::new_text(frame)) {
            return Err(crate::Error::new(
                "failed to publish text frame because pipeline is closed",
            ));
        }
        Ok(())
    }

    fn drop_consumed_pcm(&mut self) {
        let min_required = self.vad.min_required_sample();
        if min_required <= self.retained_start_sample {
            return;
        }
        let drop_len = usize::try_from(min_required - self.retained_start_sample)
            .expect("sample difference must fit into usize");
        if drop_len >= self.retained_pcm16k.len() {
            self.retained_pcm16k.clear();
        } else {
            self.retained_pcm16k.drain(..drop_len);
        }
        self.retained_start_sample = min_required;
    }

    fn ensure_audio_format(&self, frame: &AudioFrame) -> crate::Result<()> {
        if frame.format != AudioFormat::I16Be {
            return Err(crate::Error::new(format!(
                "transcription processor expects I16Be input, got {}",
                frame.format
            )));
        }
        Ok(())
    }

    fn remember_audio_layout(&mut self, frame: &AudioFrame) -> crate::Result<()> {
        match self.input_sample_rate {
            None => self.input_sample_rate = Some(frame.sample_rate),
            Some(sample_rate) if sample_rate != frame.sample_rate => {
                return Err(crate::Error::new(format!(
                    "input sample rate changed during transcription: {} -> {}",
                    sample_rate.get(),
                    frame.sample_rate.get()
                )));
            }
            Some(_) => {}
        }
        match self.input_channels {
            None => self.input_channels = Some(frame.channels),
            Some(channels) if channels != frame.channels => {
                return Err(crate::Error::new(format!(
                    "input channel count changed during transcription: {} -> {}",
                    channels.get(),
                    frame.channels.get()
                )));
            }
            Some(_) => {}
        }
        Ok(())
    }

    fn segment_time(&self, sample: u64) -> crate::Result<Duration> {
        let base = self
            .base_offset
            .ok_or_else(|| crate::Error::new("base offset is not initialized"))?;
        Ok(base + duration_from_16k_samples(sample))
    }
}

fn normalize_i16be(frame: &AudioFrame) -> crate::Result<Vec<f32>> {
    let bytes_per_sample = 2usize;
    let channels = usize::from(frame.channels.get());
    if !frame.data.len().is_multiple_of(bytes_per_sample * channels) {
        return Err(crate::Error::new(format!(
            "I16Be payload length is not aligned to channels: len={}, channels={}",
            frame.data.len(),
            frame.channels.get()
        )));
    }
    Ok(frame
        .data
        .chunks_exact(2)
        .map(|chunk| i16::from_be_bytes([chunk[0], chunk[1]]) as f32 / 32768.0)
        .collect())
}

fn split_segment_pcm(start_sample: u64, pcm: &[f32]) -> Vec<QueuedChunk> {
    let mut chunks = Vec::new();
    let mut offset = 0usize;
    while offset < pcm.len() {
        let end = usize::min(offset + MAX_TRANSCRIPT_SAMPLES, pcm.len());
        let len = end - offset;
        if len < MIN_TRANSCRIPT_SAMPLES {
            break;
        }
        chunks.push(QueuedChunk {
            start_sample: start_sample + offset as u64,
            end_sample: start_sample + end as u64,
            pcm: pcm[offset..end].to_vec(),
        });
        offset = end;
    }
    chunks
}

fn duration_from_16k_samples(samples: u64) -> Duration {
    Duration::from_nanos(samples * 62_500)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PCM 全長が 30 秒超なら 30 秒単位に分割し、末尾 10 ms 未満は破棄する。
    #[test]
    fn split_segment_pcm_splits_by_30_seconds() {
        let pcm = vec![0.0; MAX_TRANSCRIPT_SAMPLES * 2 + 200];
        let chunks = split_segment_pcm(100, &pcm);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].start_sample, 100);
        assert_eq!(chunks[0].end_sample, 100 + MAX_TRANSCRIPT_SAMPLES as u64);
        assert_eq!(chunks[1].start_sample, 100 + MAX_TRANSCRIPT_SAMPLES as u64);
        assert_eq!(
            chunks[1].end_sample,
            100 + (MAX_TRANSCRIPT_SAMPLES * 2) as u64
        );
        assert_eq!(
            chunks[2].start_sample,
            100 + (MAX_TRANSCRIPT_SAMPLES * 2) as u64
        );
        assert_eq!(
            chunks[2].end_sample,
            100 + (MAX_TRANSCRIPT_SAMPLES * 2 + 200) as u64
        );
        assert_eq!(chunks[2].pcm.len(), 200);
    }

    /// 末尾が 10 ms 未満なら捨てる。
    #[test]
    fn split_segment_pcm_drops_short_tail() {
        let pcm = vec![0.0; MAX_TRANSCRIPT_SAMPLES + MIN_TRANSCRIPT_SAMPLES - 1];
        let chunks = split_segment_pcm(0, &pcm);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].pcm.len(), MAX_TRANSCRIPT_SAMPLES);
    }

    /// I16Be を [-1.0, 1.0) の f32 に正規化する。
    #[test]
    fn normalize_i16be_converts_big_endian_samples() {
        let frame = AudioFrame {
            data: vec![0x80, 0x00, 0x00, 0x00, 0x7f, 0xff, 0xff, 0xff],
            format: AudioFormat::I16Be,
            channels: Channels::STEREO,
            sample_rate: SampleRate::from_u32(48_000).expect("valid sample rate"),
            timestamp: Duration::ZERO,
            sample_entry: None,
        };
        let pcm = normalize_i16be(&frame).expect("normalize must succeed");
        assert_eq!(pcm.len(), 4);
        assert_eq!(pcm[0], -1.0);
        assert_eq!(pcm[1], 0.0);
        assert!((pcm[2] - (32767.0 / 32768.0)).abs() < f32::EPSILON);
        assert!((pcm[3] - (-1.0 / 32768.0)).abs() < f32::EPSILON);
    }

    /// 16 kHz サンプル番号から track 時刻へ丸め誤差なく写像できる。
    #[test]
    fn duration_from_16k_samples_maps_exactly() {
        assert_eq!(duration_from_16k_samples(16_000), Duration::from_secs(1));
        assert_eq!(duration_from_16k_samples(1), Duration::from_nanos(62_500));
    }
}
