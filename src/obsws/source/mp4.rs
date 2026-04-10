use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsRecordSourcePlan, ObswsSourceRequest,
};
use crate::obsws::state::ObswsMp4FileSourceSettings;
use crate::{ProcessorId, TrackId};

/// source processor を起動できる設定が揃っているかを返す
pub(super) fn is_source_startable(settings: &ObswsMp4FileSourceSettings) -> bool {
    settings.path.is_some()
}

pub(super) fn build_record_source_plan(
    settings: &ObswsMp4FileSourceSettings,
    source_key: &str,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(path) = settings.path.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::InvalidInput(
            "inputSettings.path is required".to_owned(),
        ));
    };

    let source_processor_id = ProcessorId::new(format!("input:mp4_source:{source_key}"));
    let availability = crate::mp4::reader::probe_mp4_track_availability(path)
        .map_err(|e| BuildObswsRecordSourcePlanError::InvalidInput(e.display()))?;
    let source_video_track_id = availability
        .has_video
        .then(|| TrackId::new(format!("input:raw_video:{source_key}")));
    let source_audio_track_id = availability
        .has_audio
        .then(|| TrackId::new(format!("input:raw_audio:{source_key}")));

    let source = super::file_mp4::Mp4FileSource {
        path: std::path::PathBuf::from(path),
        loop_playback: settings.loop_playback,
        audio_track_id: source_audio_track_id.clone(),
        video_track_id: source_video_track_id.clone(),
    };

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id,
        source_audio_track_id,
        requests: vec![ObswsSourceRequest::CreateMp4FileSource {
            source,
            processor_id: Some(source_processor_id),
            event_ctx: None,
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
            "0",
        )
        .expect("audio-only mp4 source plan must succeed");
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("input:raw_audio:0")
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
            "0",
        )
        .expect("video-only mp4 source plan must succeed");
        assert_eq!(plan.source_audio_track_id, None);
        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("input:raw_video:0")
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
