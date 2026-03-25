pub mod arg_utils;
pub mod audio;
pub mod decoder;
pub mod encoder;
pub mod endpoint_http_bootstrap;
pub mod endpoint_http_metrics;
pub mod error;
pub mod future;
pub mod json;
pub mod logger;
pub mod media;
pub mod media_pipeline;
pub mod mixer;
pub mod mp4;
mod obsws;
pub mod optuna;
pub mod progress;
pub mod srt;

pub mod rtmp;
pub mod rtsp;
pub mod sora;
pub mod stats;
pub mod subcommand_inspect;
pub mod subcommand_list_codecs;
pub mod subcommand_obsws;

pub mod tcp;
pub mod timestamp;
pub mod types;
pub mod video;
pub mod webm;
pub mod webrtc;
pub mod yuv;

pub use audio::AudioFrame;
pub use error::Error;
pub use media::MediaFrame;
pub use media_pipeline::{
    Ack, MediaPipeline, MediaPipelineConfig, MediaPipelineHandle, Message, MessageReceiver,
    MessageSender, PipelineTerminated, ProcessorHandle, ProcessorId, ProcessorMetadata,
    PublishTrackError, RegisterProcessorError, Syn, TrackId,
};
pub use video::VideoFrame;

pub use obsws::auth as obsws_auth;
pub use obsws::coordinator as obsws_coordinator;
pub use obsws::input_registry as obsws_input_registry;
pub use obsws::message as obsws_message;
pub use obsws::protocol as obsws_protocol;
pub use obsws::response as obsws_response_builder;
pub use obsws::server as obsws_server;
pub use obsws::session as obsws_session;

pub type Result<T> = std::result::Result<T, Error>;
pub type Failure = Error;
