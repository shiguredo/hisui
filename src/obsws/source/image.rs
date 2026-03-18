use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan, ObswsSourceRpcRequest,
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
    let request_text = nojson::object(|f| {
        f.member("jsonrpc", "2.0")?;
        f.member("id", 1)?;
        f.member("method", "createPngFileSource")?;
        f.member(
            "params",
            nojson::object(|f| {
                f.member("path", path)?;
                f.member("frameRate", frame_rate)?;
                f.member("outputVideoTrackId", &source_video_track_id)?;
                f.member("processorId", &source_processor_id)
            }),
        )
    })
    .to_string();

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id],
        source_video_track_id: Some(source_video_track_id),
        source_audio_track_id: None,
        requests: vec![ObswsSourceRpcRequest {
            method: "createPngFileSource",
            request_text,
        }],
    })
}
