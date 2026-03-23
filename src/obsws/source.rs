use crate::obsws_input_registry::{ObswsInputEntry, ObswsInputSettings};
use crate::{PipelineOperationError, ProcessorId, ProcessorMetadata, TrackId};

pub mod file_mp4;
mod mp4;
pub mod png_file;
mod rtmp_inbound;
mod rtsp_subscriber;
mod srt_inbound;
pub mod video_device;

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
        source: self::file_mp4::Mp4FileSource,
        processor_id: Option<ProcessorId>,
    },
    CreatePngFileSource {
        source: self::png_file::PngFileSource,
        processor_id: Option<ProcessorId>,
    },
    CreateVideoDeviceSource {
        source: self::video_device::VideoDeviceSource,
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
            } => {
                let processor_id = processor_id
                    .unwrap_or_else(|| ProcessorId::new(source.path.display().to_string()));
                handle
                    .spawn_processor(
                        processor_id.clone(),
                        ProcessorMetadata::new("mp4_file_source"),
                        move |h| source.run(h),
                    )
                    .await
                    .map_err(|e| Self::map_register_error(e, &processor_id))?;
                Ok(processor_id)
            }
            Self::CreatePngFileSource {
                source,
                processor_id,
            } => {
                let processor_id = processor_id
                    .unwrap_or_else(|| ProcessorId::new(source.path.display().to_string()));
                handle
                    .spawn_processor(
                        processor_id.clone(),
                        ProcessorMetadata::new("png_file_source"),
                        move |h| source.run(h),
                    )
                    .await
                    .map_err(|e| Self::map_register_error(e, &processor_id))?;
                Ok(processor_id)
            }
            Self::CreateVideoDeviceSource {
                source,
                processor_id,
            } => {
                let processor_id = processor_id.unwrap_or_else(|| {
                    if let Some(device_id) = source.device_id.as_deref() {
                        ProcessorId::new(format!("videoDeviceSource:{device_id}"))
                    } else {
                        ProcessorId::new("videoDeviceSource:default")
                    }
                });
                handle
                    .spawn_processor(
                        processor_id.clone(),
                        ProcessorMetadata::new("video_device_source"),
                        move |h| source.run(h),
                    )
                    .await
                    .map_err(|e| Self::map_register_error(e, &processor_id))?;
                Ok(processor_id)
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

    fn map_register_error(
        e: crate::RegisterProcessorError,
        processor_id: &ProcessorId,
    ) -> PipelineOperationError {
        match e {
            crate::RegisterProcessorError::DuplicateProcessorId => {
                PipelineOperationError::DuplicateProcessorId(processor_id.clone())
            }
            crate::RegisterProcessorError::PipelineTerminated => {
                PipelineOperationError::PipelineTerminated
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
        ObswsInputSettings::ImageSource(settings) => png_file::build_record_source_plan(
            settings,
            output_kind,
            run_id,
            source_index,
            frame_rate,
        ),
        ObswsInputSettings::Mp4FileSource(settings) => {
            mp4::build_record_source_plan(settings, output_kind, run_id, source_index)
        }
        ObswsInputSettings::VideoCaptureDevice(settings) => {
            video_device::build_record_source_plan(settings, output_kind, run_id, source_index)
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
