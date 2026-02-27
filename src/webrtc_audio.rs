use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use shiguredo_webrtc::AudioTransportRef;

use crate::audio::{Channels, SampleRate};
use crate::audio_converter::AudioConverterBuilder;

const AUDIO_BYTES_PER_SAMPLE: usize = 2;
const AUDIO_CHANNELS: usize = 2;
const AUDIO_TIMESTAMP_DRIFT_WARN_THRESHOLD: Duration = Duration::from_millis(20);
const AUDIO_SAMPLES_PER_CHANNEL_PER_CHUNK: usize = 480; // 48 kHz の 10 ms
const AUDIO_SAMPLES_PER_CHUNK: usize = AUDIO_SAMPLES_PER_CHANNEL_PER_CHUNK * AUDIO_CHANNELS;

#[derive(Debug, Default)]
struct AudioTimingState {
    expected_next_timestamp: Option<Duration>,
}

fn check_and_update_timestamp_continuity(
    state: &mut AudioTimingState,
    timestamp: Duration,
    samples_per_channel: usize,
) -> Option<(Duration, Duration)> {
    let expected_timestamp = state.expected_next_timestamp;
    let frame_duration = SampleRate::HZ_48000.duration_from_samples(samples_per_channel as u64);
    state.expected_next_timestamp = Some(timestamp.saturating_add(frame_duration));
    expected_timestamp.map(|expected| (expected, timestamp.abs_diff(expected)))
}

#[derive(Clone)]
pub(crate) struct WebRtcAudioTransportSink {
    transport: Arc<Mutex<Option<AudioTransportRef>>>,
    timing: Arc<Mutex<AudioTimingState>>,
    converter: Arc<Mutex<crate::audio_converter::AudioConverter>>,
    pending_samples: Arc<Mutex<Vec<i16>>>,
}

impl WebRtcAudioTransportSink {
    pub(crate) fn new() -> Self {
        Self {
            transport: Arc::new(Mutex::new(None)),
            timing: Arc::new(Mutex::new(AudioTimingState::default())),
            converter: Arc::new(Mutex::new(
                AudioConverterBuilder::new()
                    .format(crate::audio::AudioFormat::I16Be)
                    .channels(Channels::STEREO)
                    .sample_rate(SampleRate::HZ_48000)
                    .build(),
            )),
            pending_samples: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn set_transport(&self, transport: Option<AudioTransportRef>) {
        if let Ok(mut guard) = self.transport.lock() {
            *guard = transport;
        }
        if let Ok(mut timing) = self.timing.lock() {
            *timing = AudioTimingState::default();
        }
        if let Ok(mut converter) = self.converter.lock() {
            converter.reset();
        }
        if let Ok(mut pending_samples) = self.pending_samples.lock() {
            pending_samples.clear();
        }
    }

    pub(crate) fn push_i16be_stereo_48khz(&self, frame: &crate::AudioFrame) -> crate::Result<()> {
        let frame = {
            let mut converter = self
                .converter
                .lock()
                .map_err(|_| crate::Error::new("audio converter lock poisoned"))?;
            converter.convert(frame)?
        };

        if frame.format != crate::audio::AudioFormat::I16Be {
            return Err(crate::Error::new(format!(
                "unsupported audio format: expected I16Be, got {}",
                frame.format
            )));
        }
        if !frame.is_stereo() {
            return Err(crate::Error::new(
                "unsupported audio channel layout: expected stereo",
            ));
        }
        if frame.sample_rate != SampleRate::HZ_48000 {
            return Err(crate::Error::new(format!(
                "unsupported audio sample rate: expected {}, got {}",
                SampleRate::HZ_48000.get(),
                frame.sample_rate.get()
            )));
        }
        if !frame.data.len().is_multiple_of(AUDIO_BYTES_PER_SAMPLE) {
            return Err(crate::Error::new("invalid I16Be audio data length"));
        }

        let sample_count_total = frame.data.len() / AUDIO_BYTES_PER_SAMPLE;
        if !sample_count_total.is_multiple_of(AUDIO_CHANNELS) {
            return Err(crate::Error::new("invalid stereo audio sample count"));
        }
        let samples_per_channel = sample_count_total / AUDIO_CHANNELS;

        if let Ok(mut timing) = self.timing.lock()
            && let Some((expected_timestamp, drift)) = check_and_update_timestamp_continuity(
                &mut timing,
                frame.timestamp,
                samples_per_channel,
            )
            && drift > AUDIO_TIMESTAMP_DRIFT_WARN_THRESHOLD
        {
            tracing::warn!(
                expected_timestamp_us = expected_timestamp.as_micros(),
                actual_timestamp_us = frame.timestamp.as_micros(),
                drift_us = drift.as_micros(),
                samples_per_channel,
                "audio timestamp drift detected while pushing frame to WebRTC AudioTransport",
            );
        }

        let transport = self
            .transport
            .lock()
            .ok()
            .and_then(|guard| *guard)
            .ok_or_else(|| {
                if let Ok(mut pending_samples) = self.pending_samples.lock() {
                    // 未接続中の古い音声は後送しない。
                    pending_samples.clear();
                }
                crate::Error::new("audio transport is not ready")
            })?;

        let mut new_mic_level = 0u32;
        let mut pending_samples = self
            .pending_samples
            .lock()
            .map_err(|_| crate::Error::new("audio pending buffer lock poisoned"))?;
        append_i16be_samples(&mut pending_samples, &frame.data);

        let sent_chunks = drain_ready_chunks(&mut pending_samples, |chunk| {
            let result = unsafe {
                transport.recorded_data_is_available(
                    chunk.as_ptr().cast::<u8>(),
                    AUDIO_SAMPLES_PER_CHANNEL_PER_CHUNK,
                    AUDIO_BYTES_PER_SAMPLE,
                    AUDIO_CHANNELS,
                    frame.sample_rate.get(),
                    0,
                    0,
                    0,
                    false,
                    &mut new_mic_level,
                    // Rust 側と libwebrtc 側の時刻基準の差異を避けるため、capture 時刻は渡さない。
                    None,
                )
            };
            if result != 0 {
                return Err(crate::Error::new(format!(
                    "recorded_data_is_available failed: {result}"
                )));
            }
            Ok(())
        })?;

        tracing::trace!(
            timestamp_us = frame.timestamp.as_micros(),
            samples_per_channel,
            sample_rate = frame.sample_rate.get(),
            sent_chunks,
            pending_samples_per_channel = pending_samples.len() / AUDIO_CHANNELS,
            "pushed audio frame chunks to WebRTC AudioTransport",
        );
        Ok(())
    }
}

fn append_i16be_samples(pending_samples: &mut Vec<i16>, data: &[u8]) {
    pending_samples.extend(
        data.chunks_exact(AUDIO_BYTES_PER_SAMPLE)
            .map(|chunk| i16::from_be_bytes([chunk[0], chunk[1]])),
    );
}

fn drain_ready_chunks(
    pending_samples: &mut Vec<i16>,
    mut on_chunk: impl FnMut(&[i16]) -> crate::Result<()>,
) -> crate::Result<usize> {
    let mut sent_chunks = 0usize;
    while pending_samples.len() >= AUDIO_SAMPLES_PER_CHUNK {
        on_chunk(&pending_samples[..AUDIO_SAMPLES_PER_CHUNK])?;
        pending_samples.drain(..AUDIO_SAMPLES_PER_CHUNK);
        sent_chunks += 1;
    }
    Ok(sent_chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuity_check_handles_variable_frame_sizes() {
        let mut state = AudioTimingState::default();

        assert_eq!(
            check_and_update_timestamp_continuity(&mut state, Duration::ZERO, 960),
            None
        );
        let second_timestamp = SampleRate::HZ_48000.duration_from_samples(960);
        let result = check_and_update_timestamp_continuity(&mut state, second_timestamp, 1024);
        assert_eq!(result, Some((second_timestamp, Duration::ZERO)));
    }

    #[test]
    fn continuity_check_reports_large_drift() {
        let mut state = AudioTimingState::default();

        let _ = check_and_update_timestamp_continuity(&mut state, Duration::ZERO, 960);
        let result =
            check_and_update_timestamp_continuity(&mut state, Duration::from_millis(50), 960);
        let Some((expected_timestamp, drift)) = result else {
            panic!("expected drift result");
        };

        assert_eq!(
            expected_timestamp,
            SampleRate::HZ_48000.duration_from_samples(960)
        );
        assert_eq!(drift, Duration::from_millis(30));
        assert!(drift > AUDIO_TIMESTAMP_DRIFT_WARN_THRESHOLD);
    }

    #[test]
    fn drain_ready_chunks_splits_samples_by_10ms() {
        let mut pending = vec![0; AUDIO_SAMPLES_PER_CHUNK * 2 + AUDIO_CHANNELS * 100];
        let mut chunks = Vec::new();

        let sent = drain_ready_chunks(&mut pending, |chunk| {
            chunks.push(chunk.len());
            Ok(())
        })
        .expect("drain_ready_chunks must succeed");

        assert_eq!(sent, 2);
        assert_eq!(
            chunks,
            vec![AUDIO_SAMPLES_PER_CHUNK, AUDIO_SAMPLES_PER_CHUNK]
        );
        assert_eq!(pending.len(), AUDIO_CHANNELS * 100);
    }

    #[test]
    fn drain_ready_chunks_keeps_partial_samples() {
        let mut pending = vec![0; AUDIO_SAMPLES_PER_CHUNK - 1];
        let sent =
            drain_ready_chunks(&mut pending, |_chunk| Ok(())).expect("drain_ready_chunks failed");

        assert_eq!(sent, 0);
        assert_eq!(pending.len(), AUDIO_SAMPLES_PER_CHUNK - 1);
    }
}
