pub mod arg_utils;
pub mod audio;
pub mod composer;
pub mod decoder;
#[cfg(target_os = "macos")]
pub mod decoder_audio_toolbox;
pub mod decoder_dav1d;
#[cfg(feature = "fdk-aac")]
pub mod decoder_fdk_aac;
#[cfg(feature = "libvpx")]
pub mod decoder_libvpx;
#[cfg(feature = "nvcodec")]
pub mod decoder_nvcodec;
#[cfg(feature = "nvcodec")]
pub mod decoder_nvcodec_params;
pub mod decoder_openh264;
pub mod decoder_opus;
#[cfg(target_os = "macos")]
pub mod decoder_video_toolbox;
pub mod encoder;
#[cfg(target_os = "macos")]
pub mod encoder_audio_toolbox;
#[cfg(feature = "fdk-aac")]
pub mod encoder_fdk_aac;
#[cfg(feature = "libvpx")]
pub mod encoder_libvpx;
#[cfg(feature = "libvpx")]
pub mod encoder_libvpx_params;
#[cfg(feature = "nvcodec")]
pub mod encoder_nvcodec;
#[cfg(feature = "nvcodec")]
pub mod encoder_nvcodec_params;
pub mod encoder_openh264;
pub mod encoder_openh264_params;
pub mod encoder_opus;
pub mod encoder_svt_av1;
pub mod encoder_svt_av1_params;
#[cfg(target_os = "macos")]
pub mod encoder_video_toolbox;
#[cfg(target_os = "macos")]
pub mod encoder_video_toolbox_params;
pub mod endpoint_http_bootstrap;
pub mod endpoint_http_metrics;
pub mod endpoint_http_rpc;
pub mod error;
pub mod file_reader_mp4;
pub mod file_reader_webm;
pub mod future;
pub mod inbound_endpoint_rtmp;
pub mod json;
pub mod jsonrpc;
pub mod layout;
pub mod layout_decode_params;
pub mod layout_encode_params;
pub mod layout_region;
mod legacy_processor_stats;
pub mod logger;
pub mod media;
pub mod media_pipeline;
mod media_pipeline_rpc;
pub mod metadata;
pub mod mixer_audio;
pub mod mixer_realtime_video;
pub mod mixer_video;
pub mod optuna;
pub mod outbound_endpoint_rtmp;
pub mod processor;
pub mod progress;
pub mod publisher_rtmp;
pub mod publisher_whip;
pub mod reader;
pub mod reader_mp4;
pub mod reader_webm;
mod rpc_request_file;
pub mod rtmp;
pub mod scheduler;
pub mod source_file_mp4;
pub mod source_png_file;
pub mod source_video_device;
pub mod stats;
mod stats_legacy_json;
pub mod subcommand_compose;
pub mod subcommand_inspect;
pub mod subcommand_list_codecs;
pub mod subcommand_server;
pub mod subcommand_tune;
pub mod subcommand_vmaf;
pub mod subscriber_whep;
pub mod tcp;
pub mod types;
pub mod video;
pub mod video_av1;
pub mod video_canvas;
pub mod video_h264;
pub mod video_h265;
mod webrtc_audio;
mod webrtc_factory;
mod webrtc_http;
pub mod webrtc_p2p_session;
mod webrtc_sdp;
mod webrtc_video;
pub mod writer_mp4;
pub mod writer_yuv;

pub use audio::AudioData;
pub use error::Error;
pub use media::MediaSample;
pub use media_pipeline::{
    Ack, MediaPipeline, MediaPipelineHandle, Message, MessageReceiver, MessageSender,
    PipelineTerminated, ProcessorHandle, ProcessorId, ProcessorMetadata, PublishTrackError,
    RegisterProcessorError, Syn, TrackId,
};
pub use source_file_mp4::Mp4FileSource;
pub use source_png_file::PngFileSource;
pub use source_video_device::VideoDeviceSource;
pub use video::VideoFrame;

pub type Result<T> = std::result::Result<T, Error>;
