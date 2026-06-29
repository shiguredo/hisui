use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsRecordSourcePlan, ObswsSourceRequest,
};
use crate::obsws::state::ObswsRtspSubscriberSettings;
use crate::{ProcessorId, TrackId};

/// source processor を起動できる設定が揃っているかを返す
pub(super) fn is_source_startable(settings: &ObswsRtspSubscriberSettings) -> bool {
    settings.input_url.is_some()
}

pub(super) fn build_record_source_plan(
    settings: &ObswsRtspSubscriberSettings,
    source_key: &str,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(input_url) = settings.input_url.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::InvalidInput(
            "inputSettings.input_url is required".to_owned(),
        ));
    };

    let source_processor_id = ProcessorId::new(format!("input:rtsp_subscriber:{source_key}"));
    let raw_video_track_id = TrackId::new(format!("input:raw_video:{source_key}"));
    let raw_audio_track_id = TrackId::new(format!("input:raw_audio:{source_key}"));

    let subscriber = crate::rtsp::subscriber::RtspSubscriber::new(
        input_url.to_owned(),
        Some(raw_audio_track_id.clone()),
        Some(raw_video_track_id.clone()),
    )
    .map_err(|e| {
        BuildObswsRecordSourcePlanError::InvalidInput(format!(
            "invalid rtsp_subscriber config: {e}"
        ))
    })?;

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id: Some(raw_video_track_id),
        source_audio_track_id: Some(raw_audio_track_id),
        requests: vec![ObswsSourceRequest::CreateRtspSubscriber {
            subscriber,
            processor_id: Some(source_processor_id),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_record_source_plan_generates_one_request() {
        let plan = build_record_source_plan(
            &ObswsRtspSubscriberSettings {
                input_url: Some("rtsp://127.0.0.1:554/stream".to_owned()),
            },
            "0",
        )
        .expect("rtsp_subscriber source plan must succeed");

        assert_eq!(plan.source_processor_ids.len(), 1);
        assert_eq!(
            plan.source_processor_ids[0].get(),
            "input:rtsp_subscriber:0"
        );

        assert_eq!(plan.requests.len(), 1);

        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("input:raw_video:0")
        );
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("input:raw_audio:0")
        );

        // CreateRtspSubscriber のパラメータを検証する
        match &plan.requests[0] {
            ObswsSourceRequest::CreateRtspSubscriber { subscriber, .. } => {
                assert_eq!(subscriber.input_url, "rtsp://127.0.0.1:554/stream");
            }
            _ => panic!("expected CreateRtspSubscriber"),
        }
    }

    #[test]
    fn is_source_startable_requires_input_url() {
        assert!(!is_source_startable(&ObswsRtspSubscriberSettings {
            input_url: None,
        }));
        assert!(is_source_startable(&ObswsRtspSubscriberSettings {
            input_url: Some("rtsp://127.0.0.1:554/stream".to_owned()),
        }));
    }

    // is_source_startable は空文字を弾かず、最終的な検証は new()? で行う責務分担を退行検知する
    #[test]
    fn is_source_startable_accepts_empty_input_url() {
        assert!(is_source_startable(&ObswsRtspSubscriberSettings {
            input_url: Some(String::new()),
        }));
    }

    // 空 input_url は build_record_source_plan の new()? で InvalidInput として弾かれることを退行検知する
    #[test]
    fn build_record_source_plan_rejects_empty_input_url() {
        let err = build_record_source_plan(
            &ObswsRtspSubscriberSettings {
                input_url: Some(String::new()),
            },
            "0",
        )
        .err()
        .expect("空 input_url は拒否される");
        let BuildObswsRecordSourcePlanError::InvalidInput(msg) = err;
        assert!(msg.contains("invalid rtsp_subscriber config"));
    }
}
