use crate::obsws_input_registry::{ObswsInputEntry, ObswsInputSettings};
use crate::{PipelineOperationError, ProcessorId, TrackId};

mod image;
mod mp4;
mod rtmp_inbound;
mod rtsp_subscriber;
mod srt_inbound;
mod video_capture_device;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObswsOutputKind {
    Stream,
    Record,
    RtmpOutbound,
}

impl ObswsOutputKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Record => "record",
            Self::RtmpOutbound => "rtmp_outbound",
        }
    }
}

/// obsws ソースプランで使用する型付きリクエスト
pub enum ObswsSourceRequest {
    CreateMp4FileSource {
        source: crate::Mp4FileSource,
        processor_id: Option<ProcessorId>,
    },
    CreatePngFileSource {
        source: crate::PngFileSource,
        processor_id: Option<ProcessorId>,
    },
    CreateVideoDeviceSource {
        source: crate::VideoDeviceSource,
        processor_id: Option<ProcessorId>,
    },
    CreateRtmpInboundEndpoint {
        endpoint: crate::inbound_endpoint_rtmp::RtmpInboundEndpoint,
        processor_id: Option<ProcessorId>,
    },
    CreateSrtInboundEndpoint {
        endpoint: crate::inbound_endpoint_srt::SrtInboundEndpoint,
        processor_id: Option<ProcessorId>,
    },
    CreateRtspSubscriber {
        subscriber: crate::subscriber_rtsp::RtspSubscriber,
        processor_id: Option<ProcessorId>,
    },
    CreateVideoDecoder {
        input_track_id: TrackId,
        output_track_id: TrackId,
        processor_id: Option<ProcessorId>,
    },
    CreateAudioDecoder {
        input_track_id: TrackId,
        output_track_id: TrackId,
        processor_id: Option<ProcessorId>,
    },
}

impl ObswsSourceRequest {
    pub async fn execute(
        self,
        handle: &crate::MediaPipelineHandle,
    ) -> Result<ProcessorId, PipelineOperationError> {
        match self {
            Self::CreateMp4FileSource {
                source,
                processor_id,
            } => handle.create_mp4_file_source(source, processor_id).await,
            Self::CreatePngFileSource {
                source,
                processor_id,
            } => handle.create_png_file_source(source, processor_id).await,
            Self::CreateVideoDeviceSource {
                source,
                processor_id,
            } => {
                handle
                    .create_video_device_source(source, processor_id)
                    .await
            }
            Self::CreateRtmpInboundEndpoint {
                endpoint,
                processor_id,
            } => {
                handle
                    .create_rtmp_inbound_endpoint(endpoint, processor_id)
                    .await
            }
            Self::CreateSrtInboundEndpoint {
                endpoint,
                processor_id,
            } => {
                handle
                    .create_srt_inbound_endpoint(endpoint, processor_id)
                    .await
            }
            Self::CreateRtspSubscriber {
                subscriber,
                processor_id,
            } => {
                handle
                    .create_rtsp_subscriber(subscriber, processor_id)
                    .await
            }
            Self::CreateVideoDecoder {
                input_track_id,
                output_track_id,
                processor_id,
            } => {
                handle
                    .create_video_decoder(input_track_id, output_track_id, processor_id)
                    .await
            }
            Self::CreateAudioDecoder {
                input_track_id,
                output_track_id,
                processor_id,
            } => {
                handle
                    .create_audio_decoder(input_track_id, output_track_id, processor_id)
                    .await
            }
        }
    }
}

pub struct ObswsRecordSourcePlan {
    pub source_processor_ids: Vec<ProcessorId>,
    pub source_video_track_id: Option<TrackId>,
    pub source_audio_track_id: Option<TrackId>,
    pub requests: Vec<ObswsSourceRequest>,
}

#[derive(Debug)]
pub enum BuildObswsRecordSourcePlanError {
    MissingRequiredField(&'static str),
    InvalidInput(String),
}

impl BuildObswsRecordSourcePlanError {
    pub fn message(&self) -> String {
        match self {
            Self::MissingRequiredField(field_name) => {
                format!("inputSettings.{field_name} is required")
            }
            Self::InvalidInput(message) => message.clone(),
        }
    }
}

pub fn build_record_source_plan(
    input: &ObswsInputEntry,
    output_kind: ObswsOutputKind,
    run_id: u64,
    source_index: usize,
    frame_rate: crate::video::FrameRate,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    match &input.input.settings {
        ObswsInputSettings::ImageSource(settings) => {
            image::build_record_source_plan(settings, output_kind, run_id, source_index, frame_rate)
        }
        ObswsInputSettings::Mp4FileSource(settings) => {
            mp4::build_record_source_plan(settings, output_kind, run_id, source_index)
        }
        ObswsInputSettings::VideoCaptureDevice(settings) => {
            video_capture_device::build_record_source_plan(
                settings,
                output_kind,
                run_id,
                source_index,
            )
        }
        ObswsInputSettings::RtmpInbound(settings) => {
            rtmp_inbound::build_record_source_plan(settings, output_kind, run_id, source_index)
        }
        ObswsInputSettings::SrtInbound(settings) => {
            srt_inbound::build_record_source_plan(settings, output_kind, run_id, source_index)
        }
        ObswsInputSettings::RtspSubscriber(settings) => {
            rtsp_subscriber::build_record_source_plan(settings, output_kind, run_id, source_index)
        }
    }
}
