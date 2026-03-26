use crate::obsws::input_registry::ObswsRtmpInboundSettings;
use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan, ObswsSourceRequest,
};
use crate::{ProcessorId, TrackId};

pub(super) fn build_record_source_plan(
    settings: &ObswsRtmpInboundSettings,
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
        "obsws:{kind}:{run_id}:source:{source_key}:rtmp_inbound"
    ));
    let encoded_video_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:encoded_video"
    ));
    let encoded_audio_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:encoded_audio"
    ));
    let raw_video_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:raw_video"
    ));
    let raw_audio_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:raw_audio"
    ));
    let video_decoder_processor_id = ProcessorId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:video_decoder"
    ));
    let audio_decoder_processor_id = ProcessorId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_key}:audio_decoder"
    ));

    let endpoint = crate::rtmp::inbound_endpoint::RtmpInboundEndpoint {
        input_url: input_url.to_owned(),
        stream_name: settings.stream_name.clone(),
        output_audio_track_id: Some(encoded_audio_track_id.clone()),
        output_video_track_id: Some(encoded_video_track_id.clone()),
        options: Default::default(),
    };

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![
            source_processor_id.clone(),
            video_decoder_processor_id.clone(),
            audio_decoder_processor_id.clone(),
        ],
        source_video_track_id: Some(raw_video_track_id.clone()),
        source_audio_track_id: Some(raw_audio_track_id.clone()),
        requests: vec![
            ObswsSourceRequest::CreateRtmpInboundEndpoint {
                endpoint,
                processor_id: Some(source_processor_id),
            },
            ObswsSourceRequest::CreateVideoDecoder {
                input_track_id: encoded_video_track_id,
                output_track_id: raw_video_track_id,
                processor_id: Some(video_decoder_processor_id),
            },
            ObswsSourceRequest::CreateAudioDecoder {
                input_track_id: encoded_audio_track_id,
                output_track_id: raw_audio_track_id,
                processor_id: Some(audio_decoder_processor_id),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_record_source_plan_generates_three_requests() {
        let plan = build_record_source_plan(
            &ObswsRtmpInboundSettings {
                input_url: Some("rtmp://127.0.0.1:1935".to_owned()),
                stream_name: Some("live".to_owned()),
            },
            ObswsOutputKind::Record,
            1,
            "0",
        )
        .expect("rtmp_inbound source plan must succeed");

        assert_eq!(plan.source_processor_ids.len(), 3);
        assert_eq!(
            plan.source_processor_ids[0].get(),
            "obsws:record:1:source:0:rtmp_inbound"
        );
        assert_eq!(
            plan.source_processor_ids[1].get(),
            "obsws:record:1:source:0:video_decoder"
        );
        assert_eq!(
            plan.source_processor_ids[2].get(),
            "obsws:record:1:source:0:audio_decoder"
        );

        assert_eq!(plan.requests.len(), 3);

        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_video")
        );
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_audio")
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
            ObswsOutputKind::Record,
            2,
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
    fn build_record_source_plan_requires_input_url() {
        let result = build_record_source_plan(
            &ObswsRtmpInboundSettings {
                input_url: None,
                stream_name: None,
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
