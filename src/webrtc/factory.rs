use std::sync::Arc;

use shiguredo_webrtc::{
    AudioDecoderFactory, AudioDeviceModule, AudioDeviceModuleAudioLayer, AudioEncoderFactory,
    AudioProcessingBuilder, Environment, PeerConnectionFactory, PeerConnectionFactoryDependencies,
    RtcEventLogFactory, Thread, VideoDecoderFactory, VideoEncoderFactory,
};

pub(crate) struct WebRtcFactoryBundle {
    factory: Arc<PeerConnectionFactory>,
    _network: Thread,
    _worker: Thread,
    _signaling: Thread,
}

impl WebRtcFactoryBundle {
    pub(crate) fn new() -> crate::Result<Self> {
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

        let adm = AudioDeviceModule::new(&env, AudioDeviceModuleAudioLayer::Dummy)
            .map_err(|e| crate::Error::new(format!("failed to create AudioDeviceModule: {e}")))?;
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

        Ok(Self {
            factory: Arc::new(factory),
            _network: network,
            _worker: worker,
            _signaling: signaling,
        })
    }

    pub(crate) fn factory(&self) -> Arc<PeerConnectionFactory> {
        self.factory.clone()
    }
}
