use std::sync::{Arc, Mutex};

use shiguredo_webrtc::AudioTransportRef;

const AUDIO_BYTES_PER_SAMPLE: usize = 2;
const AUDIO_CHANNELS: usize = 2;

#[derive(Clone)]
pub(crate) struct WebRtcAudioTransportSink {
    transport: Arc<Mutex<Option<AudioTransportRef>>>,
}

impl WebRtcAudioTransportSink {
    pub(crate) fn new() -> Self {
        Self {
            transport: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn set_transport(&self, transport: Option<AudioTransportRef>) {
        if let Ok(mut guard) = self.transport.lock() {
            *guard = transport;
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
