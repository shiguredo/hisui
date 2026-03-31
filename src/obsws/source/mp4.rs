use crate::obsws::input_registry::ObswsMp4FileSourceSettings;
use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan, ObswsSourceRequest,
};
use crate::{ProcessorId, TrackId};

pub(super) fn build_record_source_plan(
    settings: &ObswsMp4FileSourceSettings,
    output_kind: ObswsOutputKind,
    run_id: u64,
    source_key: &str,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(path) = settings.path.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::MissingRequiredField(
            "path",
        ));
    };

    let source_processor_id = ProcessorId::new(format!(
        "obsws:{}:{run_id}:source:{source_key}:mp4_source",
        output_kind.as_str()
    ));
    let availability = crate::mp4::reader::probe_mp4_track_availability(path)
        .map_err(|e| BuildObswsRecordSourcePlanError::InvalidInput(e.display()))?;
    let source_video_track_id = availability.has_video.then(|| {
        TrackId::new(format!(
            "obsws:{}:{run_id}:source:{source_key}:raw_video",
            output_kind.as_str()
        ))
    });
    let source_audio_track_id = availability.has_audio.then(|| {
        TrackId::new(format!(
            "obsws:{}:{run_id}:source:{source_key}:raw_audio",
            output_kind.as_str()
        ))
    });

    let source = super::file_mp4::Mp4FileSource {
        path: std::path::PathBuf::from(path),
        loop_playback: settings.loop_playback,
        audio_track_id: source_audio_track_id.clone(),
        video_track_id: source_video_track_id.clone(),
        enable_media_control: true,
    };

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id,
        source_audio_track_id,
        requests: vec![ObswsSourceRequest::CreateMp4FileSource {
            source,
            processor_id: Some(source_processor_id),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_record_source_plan_uses_audio_track_only_for_audio_only_file() {
        let plan = build_record_source_plan(
            &ObswsMp4FileSourceSettings {
                path: Some("testdata/beep-aac-audio.mp4".to_owned()),
                loop_playback: true,
            },
            ObswsOutputKind::Record,
            1,
            "0",
        )
        .expect("audio-only mp4 source plan must succeed");
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_audio")
        );
        assert_eq!(plan.source_video_track_id, None);

        // ObswsSourceRequest の中身を検証する
        assert_eq!(plan.requests.len(), 1);
        match &plan.requests[0] {
            ObswsSourceRequest::CreateMp4FileSource { source, .. } => {
                assert!(source.audio_track_id.is_some());
                assert!(source.video_track_id.is_none());
            }
            _ => panic!("expected CreateMp4FileSource"),
        }
    }

    #[test]
    fn build_record_source_plan_uses_video_track_only_for_video_only_file() {
        let plan = build_record_source_plan(
            &ObswsMp4FileSourceSettings {
                path: Some("testdata/archive-red-320x320-h264.mp4".to_owned()),
                loop_playback: false,
            },
            ObswsOutputKind::Record,
            2,
            "0",
        )
        .expect("video-only mp4 source plan must succeed");
        assert_eq!(plan.source_audio_track_id, None);
        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:2:source:0:raw_video")
        );

        assert_eq!(plan.requests.len(), 1);
        match &plan.requests[0] {
            ObswsSourceRequest::CreateMp4FileSource { source, .. } => {
                assert!(source.audio_track_id.is_none());
                assert!(source.video_track_id.is_some());
            }
            _ => panic!("expected CreateMp4FileSource"),
        }
    }
}
