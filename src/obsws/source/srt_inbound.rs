use crate::obsws::input_registry::ObswsSrtInboundSettings;
use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan, ObswsSourceRequest,
};
use crate::{ProcessorId, TrackId};

pub(super) fn build_record_source_plan(
    settings: &ObswsSrtInboundSettings,
    output_kind: ObswsOutputKind,
    run_id: u64,
    source_key: &str,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(input_url) = settings.input_url.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::MissingRequiredField(
            "inputUrl",
        ));
    };

    let kind = output_kind.as_str();
    let source_processor_id = ProcessorId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:srt_inbound"
    ));
    let raw_video_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:raw_video"
    ));
    let raw_audio_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:raw_audio"
    ));

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
            ObswsOutputKind::Record,
            1,
            "0",
        )
        .expect("srt_inbound source plan must succeed");

        assert_eq!(plan.source_processor_ids.len(), 1);
        assert_eq!(
            plan.source_processor_ids[0].get(),
            "obsws:record:1:source:0:srt_inbound"
        );

        assert_eq!(plan.requests.len(), 1);

        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_video")
        );
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_audio")
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
            ObswsOutputKind::Record,
            2,
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
    fn build_record_source_plan_requires_input_url() {
        let result = build_record_source_plan(
            &ObswsSrtInboundSettings {
                input_url: None,
                stream_id: None,
                passphrase: None,
            },
            ObswsOutputKind::Record,
            1,
            "0",
        );
        assert!(matches!(
            result,
            Err(BuildObswsRecordSourcePlanError::MissingRequiredField(
                "inputUrl"
            ))
        ));
    }
}
