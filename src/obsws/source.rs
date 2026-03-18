use crate::obsws_input_registry::{ObswsInputEntry, ObswsInputSettings};
use crate::{ProcessorId, TrackId};

mod image;
mod mp4;
mod rtmp_inbound;
mod srt_inbound;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObswsOutputKind {
    Stream,
    Record,
}

impl ObswsOutputKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Record => "record",
        }
    }
}

pub struct ObswsSourceRpcRequest {
    pub method: &'static str,
    pub request_text: String,
}

pub struct ObswsRecordSourcePlan {
    pub source_processor_ids: Vec<ProcessorId>,
    pub source_video_track_id: Option<TrackId>,
    pub source_audio_track_id: Option<TrackId>,
    pub requests: Vec<ObswsSourceRpcRequest>,
}

#[derive(Debug)]
pub enum BuildObswsRecordSourcePlanError {
    UnsupportedInputKind(&'static str),
    MissingRequiredField(&'static str),
    InvalidInput(String),
}

impl BuildObswsRecordSourcePlanError {
    pub fn message(&self) -> String {
        match self {
            Self::UnsupportedInputKind(kind) => {
                format!("Input kind is not supported for StartRecord: {kind}")
            }
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
        ObswsInputSettings::VideoCaptureDevice(_) => Err(
            BuildObswsRecordSourcePlanError::UnsupportedInputKind("video_capture_device"),
        ),
        ObswsInputSettings::RtmpInbound(settings) => {
            rtmp_inbound::build_record_source_plan(settings, output_kind, run_id, source_index)
        }
        ObswsInputSettings::SrtInbound(settings) => {
            srt_inbound::build_record_source_plan(settings, output_kind, run_id, source_index)
        }
    }
}
