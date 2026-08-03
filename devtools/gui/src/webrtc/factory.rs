//! PeerConnectionFactory の生成。
//!
//! 受信専用クライアントのため、音声出力用の AudioDeviceModule は設定しない。
//! 映像デコーダ・音声デコーダは builtin を使用する。

use std::sync::Arc;

use shiguredo_webrtc::{
    AudioDecoderFactory, AudioEncoderFactory, PeerConnectionFactory,
    PeerConnectionFactoryDependencies, RtcEventLogFactory, Thread, VideoDecoderFactory,
    VideoEncoderFactory,
};

/// WebRTC ファクトリとバックグラウンドスレッドの束。
pub struct WebRtcFactoryBundle {
    factory: Arc<PeerConnectionFactory>,
    _network: Thread,
    _worker: Thread,
    _signaling: Thread,
}

impl WebRtcFactoryBundle {
    pub fn new() -> crate::Result<Self> {
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
        deps.set_audio_encoder_factory(&AudioEncoderFactory::builtin());
        deps.set_audio_decoder_factory(&AudioDecoderFactory::builtin());
        deps.set_video_encoder_factory(VideoEncoderFactory::builtin());
        deps.set_video_decoder_factory(VideoDecoderFactory::builtin());
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

    pub fn factory(&self) -> Arc<PeerConnectionFactory> {
        self.factory.clone()
    }
}
