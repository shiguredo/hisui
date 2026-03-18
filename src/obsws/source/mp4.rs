use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan, ObswsSourceRpcRequest,
};
use crate::obsws_input_registry::ObswsMp4FileSourceSettings;
use crate::{ProcessorId, TrackId};

pub(super) fn build_record_source_plan(
    settings: &ObswsMp4FileSourceSettings,
    output_kind: ObswsOutputKind,
    run_id: u64,
    source_index: usize,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(path) = settings.path.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::MissingRequiredField(
            "path",
        ));
    };

    let source_processor_id = ProcessorId::new(format!(
        "obsws:{}:{run_id}:source:{source_index}:mp4_source",
        output_kind.as_str()
    ));
    let availability = crate::file_reader_mp4::probe_mp4_track_availability(path)
        .map_err(|e| BuildObswsRecordSourcePlanError::InvalidInput(e.display()))?;
    let source_video_track_id = availability.has_video.then(|| {
        TrackId::new(format!(
            "obsws:{}:{run_id}:source:{source_index}:raw_video",
            output_kind.as_str()
        ))
    });
    let source_audio_track_id = availability.has_audio.then(|| {
        TrackId::new(format!(
            "obsws:{}:{run_id}:source:{source_index}:raw_audio",
            output_kind.as_str()
        ))
    });
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
                if let Some(source_audio_track_id) = &source_audio_track_id {
                    f.member("audioTrackId", source_audio_track_id)?;
                }
                if let Some(source_video_track_id) = &source_video_track_id {
                    f.member("videoTrackId", source_video_track_id)?;
                }
                f.member("processorId", &source_processor_id)
            }),
        )
    })
    .to_string();

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id],
        source_video_track_id,
        source_audio_track_id,
        requests: vec![ObswsSourceRpcRequest {
            method: "createMp4FileSource",
            request_text,
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_record_source_plan_uses_audio_track_only_for_audio_only_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let plan = build_record_source_plan(
            &ObswsMp4FileSourceSettings {
                path: Some("testdata/beep-aac-audio.mp4".to_owned()),
                loop_playback: true,
            },
            ObswsOutputKind::Record,
            1,
            0,
        )
        .expect("audio-only mp4 source plan must succeed");
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_audio")
        );
        assert_eq!(plan.source_video_track_id, None);

        let json = nojson::RawJson::parse(&plan.requests[0].request_text)?;
        let params = json.value().to_member("params")?.required()?;
        let audio_track_id: Option<String> = params.to_member("audioTrackId")?.try_into()?;
        let video_track_id: Option<String> = params.to_member("videoTrackId")?.try_into()?;
        assert_eq!(
            audio_track_id.as_deref(),
            Some("obsws:record:1:source:0:raw_audio")
        );
        assert_eq!(video_track_id, None);
        Ok(())
    }

    #[test]
    fn build_record_source_plan_uses_video_track_only_for_video_only_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let plan = build_record_source_plan(
            &ObswsMp4FileSourceSettings {
                path: Some("testdata/archive-red-320x320-h264.mp4".to_owned()),
                loop_playback: false,
            },
            ObswsOutputKind::Record,
            2,
            0,
        )
        .expect("video-only mp4 source plan must succeed");
        assert_eq!(plan.source_audio_track_id, None);
        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:2:source:0:raw_video")
        );

        let json = nojson::RawJson::parse(&plan.requests[0].request_text)?;
        let params = json.value().to_member("params")?.required()?;
        let audio_track_id: Option<String> = params.to_member("audioTrackId")?.try_into()?;
        let video_track_id: Option<String> = params.to_member("videoTrackId")?.try_into()?;
        assert_eq!(audio_track_id, None);
        assert_eq!(
            video_track_id.as_deref(),
            Some("obsws:record:2:source:0:raw_video")
        );
        Ok(())
    }
}
