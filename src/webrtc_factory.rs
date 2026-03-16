use std::sync::Arc;

use shiguredo_webrtc::{
    AudioDecoderFactory, AudioDeviceModule, AudioDeviceModuleAudioLayer, AudioDeviceModuleHandler,
    AudioEncoderFactory, AudioProcessingBuilder, AudioTransportRef, Environment,
    PeerConnectionFactory, PeerConnectionFactoryDependencies, RtcEventLogFactory, Thread,
    VideoDecoderFactory, VideoEncoderFactory,
};

pub(crate) struct WebRtcFactoryBundle {
    factory: Arc<PeerConnectionFactory>,
    _network: Thread,
    _worker: Thread,
    _signaling: Thread,
}

impl WebRtcFactoryBundle {
    pub(crate) fn new() -> crate::Result<Self> {
        let (bundle, _sink) = Self::new_internal(None)?;
        Ok(bundle)
    }

    pub(crate) fn new_with_audio_transport_sink()
    -> crate::Result<(Self, crate::webrtc_audio::WebRtcAudioTransportSink)> {
        let sink = crate::webrtc_audio::WebRtcAudioTransportSink::new();
        let (bundle, sink) = Self::new_internal(Some(sink))?;
        let sink = sink.expect("BUG: audio transport sink must exist");
        Ok((bundle, sink))
    }

    fn new_internal(
        audio_sink: Option<crate::webrtc_audio::WebRtcAudioTransportSink>,
    ) -> crate::Result<(Self, Option<crate::webrtc_audio::WebRtcAudioTransportSink>)> {
        let env = Environment::new();
        let mut network = Thread::new_with_socket_server();
        let mut worker = Thread::new();
        let mut signaling = Thread::new();
        network.start();
        worker.start();
        signaling.start();

        let mut deps = PeerConnectionFactoryDependencies::new();
        deps.set_network_thread(&network);
        deps.set_worker_thread(&worker);
        deps.set_signaling_thread(&signaling);
        deps.set_event_log_factory(RtcEventLogFactory::new());

        let adm = if let Some(sink) = audio_sink.as_ref() {
            let handler = HisuiAudioDeviceModuleHandler { sink: sink.clone() };
            AudioDeviceModule::new_with_handler(Box::new(handler))
        } else {
            AudioDeviceModule::new(&env, AudioDeviceModuleAudioLayer::Dummy).map_err(|e| {
                crate::Error::new(format!("failed to create AudioDeviceModule: {e}"))
            })?
        };
        deps.set_audio_device_module(&adm);
        deps.set_audio_encoder_factory(&AudioEncoderFactory::builtin());
        deps.set_audio_decoder_factory(&AudioDecoderFactory::builtin());
        deps.set_video_encoder_factory(VideoEncoderFactory::builtin());
        deps.set_video_decoder_factory(VideoDecoderFactory::builtin());
        deps.set_audio_processing_builder(AudioProcessingBuilder::new_builtin());
        deps.enable_media();

        let factory = PeerConnectionFactory::create_modular(&mut deps).map_err(|e| {
            crate::Error::new(format!("failed to create PeerConnectionFactory: {e}"))
        })?;

        Ok((
            Self {
                factory: Arc::new(factory),
                _network: network,
                _worker: worker,
                _signaling: signaling,
            },
            audio_sink,
        ))
    }

    pub(crate) fn factory(&self) -> Arc<PeerConnectionFactory> {
        self.factory.clone()
    }
}

struct HisuiAudioDeviceModuleHandler {
    sink: crate::webrtc_audio::WebRtcAudioTransportSink,
}

impl AudioDeviceModuleHandler for HisuiAudioDeviceModuleHandler {
    fn init(&self) -> i32 {
        0
    }

    fn terminate(&self) -> i32 {
        0
    }

    fn initialized(&self) -> bool {
        true
    }

    fn recording_devices(&self) -> i16 {
        1
    }

    fn recording_device_name(&self, _index: u16) -> Option<(String, String)> {
        Some(("hisui-whip".to_owned(), "hisui-whip".to_owned()))
    }

    fn set_recording_device(&self, _index: u16) -> i32 {
        0
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
        0
    }

    fn stop_recording(&self) -> i32 {
        0
    }

    fn recording(&self) -> bool {
        true
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

    fn register_audio_callback(&self, audio_transport: Option<AudioTransportRef>) -> i32 {
        self.sink.set_transport(audio_transport);
        0
    }
}
