use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsRecordSourcePlan, ObswsSourceRpcRequest,
};
use crate::obsws_input_registry::ObswsMp4FileInputSettings;

pub(super) fn build_record_source_plan(
    settings: &ObswsMp4FileInputSettings,
    run_id: u64,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(path) = settings.path.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::MissingRequiredField(
            "path",
        ));
    };

    let source_processor_id = format!("obsws:record:{run_id}:mp4_source");
    let source_video_track_id = format!("obsws:record:{run_id}:raw_video");
    let source_audio_track_id = format!("obsws:record:{run_id}:raw_audio");
    let request_text = nojson::object(|f| {
        f.member("jsonrpc", "2.0")?;
        f.member("id", 1)?;
        f.member("method", "createMp4FileSource")?;
        f.member(
            "params",
            nojson::object(|f| {
                f.member("path", path)?;
                f.member("realtime", true)?;
                f.member("loopPlayback", settings.loop_playback)?;
                f.member("audioTrackId", &source_audio_track_id)?;
                f.member("videoTrackId", &source_video_track_id)?;
                f.member("processorId", &source_processor_id)
            }),
        )
    })
    .to_string();

    Ok(ObswsRecordSourcePlan {
        source_processor_id,
        source_video_track_id: Some(source_video_track_id),
        source_audio_track_id: Some(source_audio_track_id),
        requests: vec![ObswsSourceRpcRequest {
            method: "createMp4FileSource",
            request_text,
        }],
    })
}
