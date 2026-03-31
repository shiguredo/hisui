use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use shiguredo_webrtc::{AudioDeviceModuleHandler, AudioTransportRef};

pub struct BootstrapAudioDeviceModuleState {
    transport: Mutex<Option<AudioTransportRef>>,
    playing: AtomicBool,
}

impl Default for BootstrapAudioDeviceModuleState {
    fn default() -> Self {
        Self::new()
    }
}

impl BootstrapAudioDeviceModuleState {
    pub fn new() -> Self {
        Self {
            transport: Mutex::new(None),
            playing: AtomicBool::new(false),
        }
    }

    pub fn render_10ms_audio(&self) {
        let guard = self.transport.lock().unwrap();
        let Some(transport) = guard.as_ref() else {
            return;
        };

        let bits_per_sample = 16;
        let sample_rate = 48_000;
        let number_of_channels = 2;
        let number_of_frames = sample_rate as usize / 100;
        let mut audio_data =
            vec![0_u8; number_of_frames * number_of_channels * (bits_per_sample as usize / 8)];
        let mut elapsed_time_ms = 0_i64;
        let mut ntp_time_ms = 0_i64;

        unsafe {
            transport.pull_render_data(
                bits_per_sample,
                sample_rate,
                number_of_channels,
                number_of_frames,
                audio_data.as_mut_ptr(),
                &mut elapsed_time_ms,
                &mut ntp_time_ms,
            );
        }
    }

    pub fn shutdown(&self) {
        self.playing.store(false, Ordering::Relaxed);
        let mut guard = self.transport.lock().unwrap();
        *guard = None;
    }
}

pub struct BootstrapAudioDeviceModuleHandler {
    state: Arc<BootstrapAudioDeviceModuleState>,
}

impl BootstrapAudioDeviceModuleHandler {
    pub fn new(state: Arc<BootstrapAudioDeviceModuleState>) -> Self {
        Self { state }
    }
}

impl AudioDeviceModuleHandler for BootstrapAudioDeviceModuleHandler {
    fn register_audio_callback(&self, audio_transport: Option<AudioTransportRef>) -> i32 {
        let mut guard = self.state.transport.lock().unwrap();
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

    fn playout_is_available(&self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn init_playout(&self) -> i32 {
        0
    }

    fn playout_is_initialized(&self) -> bool {
        true
    }

    fn start_playout(&self) -> i32 {
        self.state.playing.store(true, Ordering::Relaxed);
        0
    }

    fn stop_playout(&self) -> i32 {
        self.state.playing.store(false, Ordering::Relaxed);
        0
    }

    fn playing(&self) -> bool {
        self.state.playing.load(Ordering::Relaxed)
    }

    fn stereo_playout_is_available(&self, available: &mut bool) -> i32 {
        *available = true;
        0
    }

    fn set_stereo_playout(&self, _enable: bool) -> i32 {
        0
    }

    fn stereo_playout(&self, enabled: &mut bool) -> i32 {
        *enabled = true;
        0
    }
}
