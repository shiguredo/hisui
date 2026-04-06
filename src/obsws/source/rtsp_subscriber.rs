use crate::obsws::input_registry::ObswsRtspSubscriberSettings;
use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan, ObswsSourceRequest,
};
use crate::{ProcessorId, TrackId};

/// source processor を起動できる設定が揃っているかを返す
pub(super) fn is_source_startable(settings: &ObswsRtspSubscriberSettings) -> bool {
    settings.input_url.is_some()
}

pub(super) fn build_record_source_plan(
    settings: &ObswsRtspSubscriberSettings,
    output_kind: ObswsOutputKind,
    run_id: u64,
    source_key: &str,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let input_url = settings
        .input_url
        .as_deref()
        .expect("is_source_startable() で inputUrl の存在は確認済み");

    let kind = output_kind.as_str();
    let source_processor_id = ProcessorId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:rtsp_subscriber"
    ));
    let raw_video_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:raw_video"
    ));
    let raw_audio_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:raw_audio"
    ));

    let subscriber = crate::rtsp::subscriber::RtspSubscriber {
        input_url: input_url.to_owned(),
        output_video_track_id: Some(raw_video_track_id.clone()),
        output_audio_track_id: Some(raw_audio_track_id.clone()),
    };

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
            ObswsOutputKind::Program,
            1,
            "0",
        )
        .expect("rtsp_subscriber source plan must succeed");

        assert_eq!(plan.source_processor_ids.len(), 1);
        assert_eq!(
            plan.source_processor_ids[0].get(),
            "obsws:program:1:source:0:rtsp_subscriber"
        );

        assert_eq!(plan.requests.len(), 1);

        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("obsws:program:1:source:0:raw_video")
        );
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("obsws:program:1:source:0:raw_audio")
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
}
