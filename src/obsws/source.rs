use crate::obsws_input_registry::{ObswsInputEntry, ObswsInputSettings};

mod image;
mod mp4;

pub struct ObswsSourceRpcRequest {
    pub method: &'static str,
    pub request_text: String,
}

pub struct ObswsRecordSourcePlan {
    pub source_processor_id: String,
    pub source_video_track_id: Option<String>,
    pub source_audio_track_id: Option<String>,
    pub requests: Vec<ObswsSourceRpcRequest>,
}

pub enum BuildObswsRecordSourcePlanError {
    UnsupportedInputKind(&'static str),
    MissingRequiredField(&'static str),
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
        }
    }
}

pub fn build_record_source_plan(
    input: &ObswsInputEntry,
    run_id: u64,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    match &input.input.settings {
        ObswsInputSettings::ImageSource(settings) => {
            image::build_record_source_plan(settings, run_id)
        }
        ObswsInputSettings::Mp4FileInput(settings) => {
            mp4::build_record_source_plan(settings, run_id)
        }
        ObswsInputSettings::VideoCaptureDevice(_) => Err(
            BuildObswsRecordSourcePlanError::UnsupportedInputKind("video_capture_device"),
        ),
    }
}
