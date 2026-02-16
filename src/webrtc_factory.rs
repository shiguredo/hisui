use std::sync::Arc;

use shiguredo_webrtc::{
    AudioDecoderFactory, AudioDeviceModule, AudioDeviceModuleAudioLayer,
    AudioDeviceModuleCallbacks, AudioEncoderFactory, AudioProcessingBuilder, Environment,
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
            let sink = sink.clone();
            let callbacks = AudioDeviceModuleCallbacks {
                init: Some(Box::new(|| 0)),
                terminate: Some(Box::new(|| 0)),
                initialized: Some(Box::new(|| true)),
                recording_devices: Some(Box::new(|| 1)),
                recording_device_name: Some(Box::new(|_| {
                    Some(("hisui-whip".to_owned(), "hisui-whip".to_owned()))
                })),
                set_recording_device: Some(Box::new(|_| 0)),
                recording_is_available: Some(Box::new(|available| {
                    *available = true;
                    0
                })),
                init_recording: Some(Box::new(|| 0)),
                recording_is_initialized: Some(Box::new(|| true)),
                start_recording: Some(Box::new(|| 0)),
                stop_recording: Some(Box::new(|| 0)),
                recording: Some(Box::new(|| true)),
                stereo_recording_is_available: Some(Box::new(|available| {
                    *available = true;
                    0
                })),
                set_stereo_recording: Some(Box::new(|_| 0)),
                stereo_recording: Some(Box::new(|enabled| {
                    *enabled = true;
                    0
                })),
                register_audio_callback: Some(Box::new(move |transport| {
                    sink.set_transport(transport);
                    0
                })),
                ..Default::default()
            };
            AudioDeviceModule::new_with_callbacks(callbacks)
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
