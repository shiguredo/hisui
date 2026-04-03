use std::sync::{Arc, Mutex, atomic::AtomicBool};

use shiguredo_webrtc::{AudioDeviceModuleHandler, AudioTransportRef};

/// メディアパイプラインから受け取った PCM データを WebRTC の音声パイプラインに供給する共有状態
pub(crate) struct SharedAudioState {
    transport: Mutex<Option<AudioTransportRef>>,
    recording: AtomicBool,
}

impl SharedAudioState {
    pub(crate) fn new() -> Self {
        Self {
            transport: Mutex::new(None),
            recording: AtomicBool::new(false),
        }
    }

    /// メディアパイプラインからの AudioFrame を WebRTC に供給する
    pub(crate) fn push_audio_frame(&self, frame: &crate::AudioFrame) -> crate::Result<()> {
        if !self.recording.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }

        let guard = self
            .transport
            .lock()
            .expect("transport mutex is not poisoned");
        let Some(transport) = guard.as_ref() else {
            return Ok(());
        };

        // I16Be → i16 ネイティブエンディアン（リトルエンディアン）に変換する
        let samples: Vec<i16> = frame
            .data
            .chunks_exact(2)
            .map(|c| i16::from_be_bytes([c[0], c[1]]))
            .collect();

        let n_channels = frame.channels.get() as usize;
        // チャンネルあたりのサンプル数
        let n_samples = samples.len() / n_channels;
        // WebRTC の bytes_per_sample は 1 サンプルあたりのバイト数
        let n_bytes_per_sample = std::mem::size_of::<i16>();

        let mut new_mic_level: u32 = 0;
        unsafe {
            transport.recorded_data_is_available(
                samples.as_ptr() as *const u8,
                n_samples,
                n_bytes_per_sample,
                n_channels,
                frame.sample_rate.get(),
                0,     // total_delay_ms
                0,     // clock_drift
                0,     // current_mic_level
                false, // key_pressed
                &mut new_mic_level,
                None, // estimated_capture_time_ns
            );
        }
        Ok(())
    }
}

/// WebRTC の AudioDeviceModule にメディアパイプラインの音声を供給するハンドラ
pub(crate) struct HisuiAudioDeviceModuleHandler {
    state: Arc<SharedAudioState>,
}

impl HisuiAudioDeviceModuleHandler {
    pub(crate) fn new(state: Arc<SharedAudioState>) -> Self {
        Self { state }
    }
}

impl AudioDeviceModuleHandler for HisuiAudioDeviceModuleHandler {
    fn register_audio_callback(&self, audio_transport: Option<AudioTransportRef>) -> i32 {
        let mut guard = self
            .state
            .transport
            .lock()
            .expect("transport mutex is not poisoned");
        *guard = audio_transport;
        0
    }

    fn init(&self) -> i32 {
        0
    }

    fn terminate(&self) -> i32 {
        0
    }

    fn initialized(&self) -> bool {
        true
    }

    fn recording_is_available(&self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn init_recording(&self) -> i32 {
        0
    }

    fn recording_is_initialized(&self) -> bool {
        true
    }

    fn start_recording(&self) -> i32 {
        self.state
            .recording
            .store(true, std::sync::atomic::Ordering::Relaxed);
        0
    }

    fn stop_recording(&self) -> i32 {
        self.state
            .recording
            .store(false, std::sync::atomic::Ordering::Relaxed);
        0
    }

    fn recording(&self) -> bool {
        self.state
            .recording
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn stereo_recording_is_available(&self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn set_stereo_recording(&self, _enable: bool) -> i32 {
        0
    }

    fn stereo_recording(&self, enabled: &mut bool) -> i32 {
        *enabled = true;
        0
    }
}
