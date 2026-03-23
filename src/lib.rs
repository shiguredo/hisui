pub mod arg_utils;
pub mod audio;
pub mod audio_aac;
pub mod audio_converter;
pub mod decoder;
#[cfg(target_os = "macos")]
pub mod decoder_audio_toolbox;
pub mod decoder_dav1d;
#[cfg(feature = "fdk-aac")]
pub mod decoder_fdk_aac;
pub mod decoder_libvpx;
#[cfg(feature = "nvcodec")]
pub mod decoder_nvcodec;
pub mod decoder_openh264;
pub mod decoder_opus;
#[cfg(target_os = "macos")]
pub mod decoder_video_toolbox;
pub mod encoder;
#[cfg(target_os = "macos")]
pub mod encoder_audio_toolbox;
#[cfg(feature = "fdk-aac")]
pub mod encoder_fdk_aac;
pub mod encoder_libvpx;
#[cfg(feature = "nvcodec")]
pub mod encoder_nvcodec;
pub mod encoder_openh264;
pub mod encoder_opus;
pub mod encoder_svt_av1;
#[cfg(target_os = "macos")]
pub mod encoder_video_toolbox;
pub mod endpoint_http_bootstrap;
pub mod endpoint_http_metrics;
pub mod error;
pub mod file_reader_mp4;
pub mod file_reader_webm;
pub mod future;
pub mod inbound_endpoint_rtmp;
pub mod inbound_endpoint_srt;
pub mod json;
pub mod logger;
pub mod media;
pub mod media_pipeline;
mod media_pipeline_rpc;
pub mod mixer_realtime_audio;
pub mod mixer_realtime_video;
mod obsws;
pub mod optuna;
pub mod outbound_endpoint_rtmp;
pub mod progress;
pub mod publisher_rtmp;
pub mod reader_mp4;
pub mod reader_webm;
pub mod rtmp;
pub mod sample_based_timestamp_aligner;
pub mod sora;
pub mod source_file_mp4;
pub mod source_png_file;
pub mod source_video_device;
pub mod stats;
pub mod subcommand_inspect;
pub mod subcommand_list_codecs;
pub mod subcommand_obsws;
pub mod subcommand_server;
pub mod subscriber_rtsp;
pub mod tcp;
pub mod timestamp_mapper;
pub mod types;
pub mod video;
pub mod video_av1;
pub mod video_canvas;
pub mod video_h264;
pub mod video_h265;
mod webrtc_factory;
pub mod webrtc_p2p_session;
mod webrtc_sdp;
mod webrtc_video;
pub mod writer_mp4;
pub mod writer_yuv;

pub use audio::AudioFrame;
pub use error::Error;
pub use media::MediaFrame;
pub use media_pipeline::{
    Ack, MediaPipeline, MediaPipelineConfig, MediaPipelineHandle, Message, MessageReceiver,
    MessageSender, PipelineOperationError, PipelineTerminated, ProcessorHandle, ProcessorId,
    ProcessorMetadata, PublishTrackError, RegisterProcessorError, Syn, TrackId,
};
pub use source_file_mp4::Mp4FileSource;
pub use source_png_file::PngFileSource;
pub use source_video_device::VideoDeviceSource;
pub use video::VideoFrame;

pub use obsws::auth as obsws_auth;
pub use obsws::input_registry as obsws_input_registry;
pub use obsws::message as obsws_message;
pub use obsws::protocol as obsws_protocol;
pub use obsws::response as obsws_response_builder;
pub use obsws::server as obsws_server;
pub use obsws::session as obsws_session;

pub type Result<T> = std::result::Result<T, Error>;
pub type Failure = Error;
