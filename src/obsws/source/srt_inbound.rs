use crate::obsws::input_registry::ObswsSrtInboundSettings;
use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsRecordSourcePlan, ObswsSourceRequest,
};
use crate::{ProcessorId, TrackId};

/// source processor を起動できる設定が揃っているかを返す
pub(super) fn is_source_startable(settings: &ObswsSrtInboundSettings) -> bool {
    settings.input_url.is_some()
}

pub(super) fn build_record_source_plan(
    settings: &ObswsSrtInboundSettings,
    source_key: &str,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(input_url) = settings.input_url.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::InvalidInput(
            "inputSettings.inputUrl is required".to_owned(),
        ));
    };

    let source_processor_id = ProcessorId::new(format!("input:srt_inbound:{source_key}"));
    let raw_video_track_id = TrackId::new(format!("input:raw_video:{source_key}"));
    let raw_audio_track_id = TrackId::new(format!("input:raw_audio:{source_key}"));

    let endpoint = crate::srt::inbound_endpoint::SrtInboundEndpoint {
        input_url: input_url.to_owned(),
        output_audio_track_id: Some(raw_audio_track_id.clone()),
        output_video_track_id: Some(raw_video_track_id.clone()),
        stream_id: settings.stream_id.clone(),
        passphrase: settings.passphrase.clone(),
        key_length: None,
        tsbpd_delay_ms: None,
    };

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id: Some(raw_video_track_id),
        source_audio_track_id: Some(raw_audio_track_id),
        requests: vec![ObswsSourceRequest::CreateSrtInboundEndpoint {
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
            &ObswsSrtInboundSettings {
                input_url: Some("srt://127.0.0.1:9000".to_owned()),
                stream_id: Some("test-stream".to_owned()),
                passphrase: Some("secret123456".to_owned()),
            },
            "0",
        )
        .expect("srt_inbound source plan must succeed");

        assert_eq!(plan.source_processor_ids.len(), 1);
        assert_eq!(plan.source_processor_ids[0].get(), "input:srt_inbound:0");

        assert_eq!(plan.requests.len(), 1);

        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("input:raw_video:0")
        );
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("input:raw_audio:0")
        );

        // CreateSrtInboundEndpoint のパラメータを検証する
        match &plan.requests[0] {
            ObswsSourceRequest::CreateSrtInboundEndpoint { endpoint, .. } => {
                assert_eq!(endpoint.input_url, "srt://127.0.0.1:9000");
                assert_eq!(endpoint.stream_id.as_deref(), Some("test-stream"));
                assert_eq!(endpoint.passphrase.as_deref(), Some("secret123456"));
            }
            _ => panic!("expected CreateSrtInboundEndpoint"),
        }
    }

    #[test]
    fn build_record_source_plan_without_optional_params() {
        let plan = build_record_source_plan(
            &ObswsSrtInboundSettings {
                input_url: Some("srt://127.0.0.1:9000".to_owned()),
                stream_id: None,
                passphrase: None,
            },
            "1",
        )
        .expect("srt_inbound source plan without optional params must succeed");

        match &plan.requests[0] {
            ObswsSourceRequest::CreateSrtInboundEndpoint { endpoint, .. } => {
                assert_eq!(endpoint.stream_id, None);
                assert_eq!(endpoint.passphrase, None);
            }
            _ => panic!("expected CreateSrtInboundEndpoint"),
        }
    }

    #[test]
    fn is_source_startable_requires_input_url() {
        assert!(!is_source_startable(&ObswsSrtInboundSettings {
            input_url: None,
            stream_id: None,
            passphrase: None,
        }));
        assert!(is_source_startable(&ObswsSrtInboundSettings {
            input_url: Some("srt://127.0.0.1:9000".to_owned()),
            stream_id: None,
            passphrase: None,
        }));
    }
}
