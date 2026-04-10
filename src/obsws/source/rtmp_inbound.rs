use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsRecordSourcePlan, ObswsSourceRequest,
};
use crate::obsws::state::ObswsRtmpInboundSettings;
use crate::{ProcessorId, TrackId};

/// source processor を起動できる設定が揃っているかを返す
pub(super) fn is_source_startable(settings: &ObswsRtmpInboundSettings) -> bool {
    settings.input_url.is_some()
}

pub(super) fn build_record_source_plan(
    settings: &ObswsRtmpInboundSettings,
    source_key: &str,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(input_url) = settings.input_url.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::InvalidInput(
            "inputSettings.inputUrl is required".to_owned(),
        ));
    };

    let source_processor_id = ProcessorId::new(format!("input:rtmp_inbound:{source_key}"));
    let raw_video_track_id = TrackId::new(format!("input:raw_video:{source_key}"));
    let raw_audio_track_id = TrackId::new(format!("input:raw_audio:{source_key}"));

    let endpoint = crate::rtmp::inbound_endpoint::RtmpInboundEndpoint {
        input_url: input_url.to_owned(),
        stream_name: settings.stream_name.clone(),
        output_audio_track_id: Some(raw_audio_track_id.clone()),
        output_video_track_id: Some(raw_video_track_id.clone()),
        options: Default::default(),
    };

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id: Some(raw_video_track_id),
        source_audio_track_id: Some(raw_audio_track_id),
        requests: vec![ObswsSourceRequest::CreateRtmpInboundEndpoint {
            endpoint,
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
            &ObswsRtmpInboundSettings {
                input_url: Some("rtmp://127.0.0.1:1935".to_owned()),
                stream_name: Some("live".to_owned()),
            },
            "0",
        )
        .expect("rtmp_inbound source plan must succeed");

        assert_eq!(plan.source_processor_ids.len(), 1);
        assert_eq!(plan.source_processor_ids[0].get(), "input:rtmp_inbound:0");

        assert_eq!(plan.requests.len(), 1);

        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("input:raw_video:0")
        );
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("input:raw_audio:0")
        );

        // CreateRtmpInboundEndpoint のパラメータを検証する
        match &plan.requests[0] {
            ObswsSourceRequest::CreateRtmpInboundEndpoint { endpoint, .. } => {
                assert_eq!(endpoint.input_url, "rtmp://127.0.0.1:1935");
                assert_eq!(endpoint.stream_name.as_deref(), Some("live"));
            }
            _ => panic!("expected CreateRtmpInboundEndpoint"),
        }
    }

    #[test]
    fn build_record_source_plan_without_stream_name() {
        let plan = build_record_source_plan(
            &ObswsRtmpInboundSettings {
                input_url: Some("rtmp://127.0.0.1:1935".to_owned()),
                stream_name: None,
            },
            "1",
        )
        .expect("rtmp_inbound source plan without stream_name must succeed");

        match &plan.requests[0] {
            ObswsSourceRequest::CreateRtmpInboundEndpoint { endpoint, .. } => {
                assert_eq!(endpoint.stream_name, None);
            }
            _ => panic!("expected CreateRtmpInboundEndpoint"),
        }
    }

    #[test]
    fn is_source_startable_requires_input_url() {
        assert!(!is_source_startable(&ObswsRtmpInboundSettings {
            input_url: None,
            stream_name: None,
        }));
        assert!(is_source_startable(&ObswsRtmpInboundSettings {
            input_url: Some("rtmp://127.0.0.1:1935/live".to_owned()),
            stream_name: None,
        }));
    }
}
