use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use shiguredo_webrtc::AudioTransportRef;

const AUDIO_BYTES_PER_SAMPLE: usize = 2;
const AUDIO_CHANNELS: usize = 2;
const AUDIO_TIMESTAMP_DRIFT_WARN_THRESHOLD: Duration = Duration::from_millis(20);

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
    let frame_duration =
        Duration::from_secs(samples_per_channel as u64) / u32::from(crate::audio::SAMPLE_RATE);
    state.expected_next_timestamp = Some(timestamp.saturating_add(frame_duration));
    expected_timestamp.map(|expected| (expected, timestamp.abs_diff(expected)))
}

#[derive(Clone)]
pub(crate) struct WebRtcAudioTransportSink {
    transport: Arc<Mutex<Option<AudioTransportRef>>>,
    timing: Arc<Mutex<AudioTimingState>>,
}

impl WebRtcAudioTransportSink {
    pub(crate) fn new() -> Self {
        Self {
            transport: Arc::new(Mutex::new(None)),
            timing: Arc::new(Mutex::new(AudioTimingState::default())),
        }
    }

    pub(crate) fn set_transport(&self, transport: Option<AudioTransportRef>) {
        if let Ok(mut guard) = self.transport.lock() {
            *guard = transport;
        }
        if let Ok(mut timing) = self.timing.lock() {
            *timing = AudioTimingState::default();
        }
    }

    pub(crate) fn push_i16be_stereo_48khz(&self, audio: &crate::AudioData) -> crate::Result<()> {
        if audio.format != crate::audio::AudioFormat::I16Be {
            return Err(crate::Error::new(format!(
                "unsupported audio format: expected I16Be, got {}",
                audio.format
            )));
        }
        if !audio.stereo {
            return Err(crate::Error::new(
                "unsupported audio channel layout: expected stereo",
            ));
        }
        if audio.sample_rate != crate::audio::SAMPLE_RATE {
            return Err(crate::Error::new(format!(
                "unsupported audio sample rate: expected {}, got {}",
                crate::audio::SAMPLE_RATE,
                audio.sample_rate
            )));
        }
        if !audio.data.len().is_multiple_of(AUDIO_BYTES_PER_SAMPLE) {
            return Err(crate::Error::new("invalid I16Be audio data length"));
        }

        let sample_count_total = audio.data.len() / AUDIO_BYTES_PER_SAMPLE;
        if !sample_count_total.is_multiple_of(AUDIO_CHANNELS) {
            return Err(crate::Error::new("invalid stereo audio sample count"));
        }
        let samples_per_channel = sample_count_total / AUDIO_CHANNELS;

        if let Ok(mut timing) = self.timing.lock()
            && let Some((expected_timestamp, drift)) = check_and_update_timestamp_continuity(
                &mut timing,
                audio.timestamp,
                samples_per_channel,
            )
            && drift > AUDIO_TIMESTAMP_DRIFT_WARN_THRESHOLD
        {
            tracing::warn!(
                expected_timestamp_us = expected_timestamp.as_micros(),
                actual_timestamp_us = audio.timestamp.as_micros(),
                drift_us = drift.as_micros(),
                samples_per_channel,
                "audio timestamp drift detected while pushing frame to WebRTC AudioTransport",
            );
        }

        let mut native_endian = Vec::with_capacity(audio.data.len());
        for chunk in audio.data.chunks_exact(AUDIO_BYTES_PER_SAMPLE) {
            let sample = i16::from_be_bytes([chunk[0], chunk[1]]);
            native_endian.extend_from_slice(&sample.to_ne_bytes());
        }

        let transport = self
            .transport
            .lock()
            .ok()
            .and_then(|guard| *guard)
            .ok_or_else(|| crate::Error::new("audio transport is not ready"))?;

        let mut new_mic_level = 0u32;
        let estimated_capture_time_ns = i64::try_from(audio.timestamp.as_nanos()).ok();
        tracing::trace!(
            timestamp_us = audio.timestamp.as_micros(),
            samples_per_channel,
            sample_rate = audio.sample_rate,
            "pushing audio frame to WebRTC AudioTransport",
        );

        // AudioTransport はネイティブ実装が期待する生バッファを参照するため、
        // ここでは引数の整合を確認したうえで FFI 呼び出しを行う。
        let result = unsafe {
            transport.recorded_data_is_available(
                native_endian.as_ptr(),
                samples_per_channel,
                AUDIO_BYTES_PER_SAMPLE,
                AUDIO_CHANNELS,
                u32::from(audio.sample_rate),
                0,
                0,
                0,
                false,
                &mut new_mic_level,
                estimated_capture_time_ns,
            )
        };
        if result != 0 {
            return Err(crate::Error::new(format!(
                "recorded_data_is_available failed: {result}"
            )));
        }
        Ok(())
    }
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
        let second_timestamp = Duration::from_secs(960) / u32::from(crate::audio::SAMPLE_RATE);
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
            Duration::from_secs(960) / u32::from(crate::audio::SAMPLE_RATE)
        );
        assert_eq!(drift, Duration::from_millis(30));
        assert!(drift > AUDIO_TIMESTAMP_DRIFT_WARN_THRESHOLD);
    }
}
