use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsRecordSourcePlan, ObswsSourceRpcRequest,
};
use crate::obsws_input_registry::ObswsImageSourceSettings;

pub(super) fn build_record_source_plan(
    settings: &ObswsImageSourceSettings,
    run_id: u64,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(path) = settings.file.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::MissingRequiredField(
            "file",
        ));
    };

    let source_processor_id = format!("obsws:record:{run_id}:png_source");
    let source_video_track_id = format!("obsws:record:{run_id}:raw_video");
    let request_text = nojson::object(|f| {
        f.member("jsonrpc", "2.0")?;
        f.member("id", 1)?;
        f.member("method", "createPngFileSource")?;
        f.member(
            "params",
            nojson::object(|f| {
                f.member("path", path)?;
                f.member("frameRate", 30)?;
                f.member("outputVideoTrackId", &source_video_track_id)?;
                f.member("processorId", &source_processor_id)
            }),
        )
    })
    .to_string();

    Ok(ObswsRecordSourcePlan {
        source_processor_id,
        source_video_track_id: Some(source_video_track_id),
        source_audio_track_id: None,
        requests: vec![ObswsSourceRpcRequest {
            method: "createPngFileSource",
            request_text,
        }],
    })
}
