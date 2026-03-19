use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan, ObswsSourceRequest,
};
use crate::obsws_input_registry::ObswsImageSourceSettings;
use crate::{ProcessorId, TrackId};

pub(super) fn build_record_source_plan(
    settings: &ObswsImageSourceSettings,
    output_kind: ObswsOutputKind,
    run_id: u64,
    source_index: usize,
    frame_rate: crate::video::FrameRate,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(path) = settings.file.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::MissingRequiredField(
            "file",
        ));
    };

    let source_processor_id = ProcessorId::new(format!(
        "obsws:{}:{run_id}:source:{source_index}:png_source",
        output_kind.as_str()
    ));
    let source_video_track_id = TrackId::new(format!(
        "obsws:{}:{run_id}:source:{source_index}:raw_video",
        output_kind.as_str()
    ));

    let source = crate::PngFileSource {
        path: std::path::PathBuf::from(path),
        frame_rate,
        output_video_track_id: source_video_track_id.clone(),
    };

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id: Some(source_video_track_id),
        source_audio_track_id: None,
        requests: vec![ObswsSourceRequest::CreatePngFileSource {
            source,
            processor_id: Some(source_processor_id),
        }],
    })
}
