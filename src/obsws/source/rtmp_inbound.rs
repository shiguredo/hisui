use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan, ObswsSourceRpcRequest,
};
use crate::obsws_input_registry::ObswsRtmpInboundSettings;
use crate::{ProcessorId, TrackId};

pub(super) fn build_record_source_plan(
    settings: &ObswsRtmpInboundSettings,
    output_kind: ObswsOutputKind,
    run_id: u64,
    source_index: usize,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let Some(input_url) = settings.input_url.as_deref() else {
        return Err(BuildObswsRecordSourcePlanError::MissingRequiredField(
            "inputUrl",
        ));
    };

    let kind = output_kind.as_str();
    let source_processor_id = ProcessorId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_index}:rtmp_inbound"
    ));
    let encoded_video_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_index}:encoded_video"
    ));
    let encoded_audio_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_index}:encoded_audio"
    ));
    let raw_video_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_index}:raw_video"
    ));
    let raw_audio_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_index}:raw_audio"
    ));
    let video_decoder_processor_id = ProcessorId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_index}:video_decoder"
    ));
    let audio_decoder_processor_id = ProcessorId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_index}:audio_decoder"
    ));

    // createRtmpInboundEndpoint リクエスト
    let endpoint_request_text = nojson::object(|f| {
        f.member("jsonrpc", "2.0")?;
        f.member("id", 1)?;
        f.member("method", "createRtmpInboundEndpoint")?;
        f.member(
            "params",
            nojson::object(|f| {
                f.member("inputUrl", input_url)?;
                if let Some(stream_name) = &settings.stream_name {
                    f.member("streamName", stream_name)?;
                }
                f.member("outputVideoTrackId", &encoded_video_track_id)?;
                f.member("outputAudioTrackId", &encoded_audio_track_id)?;
                f.member("processorId", &source_processor_id)
            }),
        )
    })
    .to_string();

    // createVideoDecoder リクエスト
    let video_decoder_request_text = nojson::object(|f| {
        f.member("jsonrpc", "2.0")?;
        f.member("id", 1)?;
        f.member("method", "createVideoDecoder")?;
        f.member(
            "params",
            nojson::object(|f| {
                f.member("inputTrackId", &encoded_video_track_id)?;
                f.member("outputTrackId", &raw_video_track_id)?;
                f.member("processorId", &video_decoder_processor_id)
            }),
        )
    })
    .to_string();

    // createAudioDecoder リクエスト
    let audio_decoder_request_text = nojson::object(|f| {
        f.member("jsonrpc", "2.0")?;
        f.member("id", 1)?;
        f.member("method", "createAudioDecoder")?;
        f.member(
            "params",
            nojson::object(|f| {
                f.member("inputTrackId", &encoded_audio_track_id)?;
                f.member("outputTrackId", &raw_audio_track_id)?;
                f.member("processorId", &audio_decoder_processor_id)
            }),
        )
    })
    .to_string();

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![
            source_processor_id,
            video_decoder_processor_id,
            audio_decoder_processor_id,
        ],
        source_video_track_id: Some(raw_video_track_id),
        source_audio_track_id: Some(raw_audio_track_id),
        requests: vec![
            ObswsSourceRpcRequest {
                method: "createRtmpInboundEndpoint",
                request_text: endpoint_request_text,
            },
            ObswsSourceRpcRequest {
                method: "createVideoDecoder",
                request_text: video_decoder_request_text,
            },
            ObswsSourceRpcRequest {
                method: "createAudioDecoder",
                request_text: audio_decoder_request_text,
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_record_source_plan_generates_three_requests() -> Result<(), Box<dyn std::error::Error>>
    {
        let plan = build_record_source_plan(
            &ObswsRtmpInboundSettings {
                input_url: Some("rtmp://127.0.0.1:1935".to_owned()),
                stream_name: Some("live".to_owned()),
            },
            ObswsOutputKind::Record,
            1,
            0,
        )
        .expect("rtmp_inbound source plan must succeed");

        // source_processor_ids に endpoint + video_decoder + audio_decoder の 3 つが含まれることを検証する
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
        assert_eq!(plan.requests[0].method, "createRtmpInboundEndpoint");
        assert_eq!(plan.requests[1].method, "createVideoDecoder");
        assert_eq!(plan.requests[2].method, "createAudioDecoder");

        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_video")
        );
        assert_eq!(
            plan.source_audio_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_audio")
        );

        // createRtmpInboundEndpoint のパラメータを検証する
        let json = nojson::RawJson::parse(&plan.requests[0].request_text)?;
        let params = json.value().to_member("params")?.required()?;
        let input_url: String = params.to_member("inputUrl")?.required()?.try_into()?;
        assert_eq!(input_url, "rtmp://127.0.0.1:1935");
        let stream_name: Option<String> = params.to_member("streamName")?.try_into()?;
        assert_eq!(stream_name.as_deref(), Some("live"));
        let output_video_track_id: String = params
            .to_member("outputVideoTrackId")?
            .required()?
            .try_into()?;
        assert_eq!(
            output_video_track_id,
            "obsws:record:1:source:0:encoded_video"
        );
        let output_audio_track_id: String = params
            .to_member("outputAudioTrackId")?
            .required()?
            .try_into()?;
        assert_eq!(
            output_audio_track_id,
            "obsws:record:1:source:0:encoded_audio"
        );

        // createVideoDecoder のパラメータを検証する
        let json = nojson::RawJson::parse(&plan.requests[1].request_text)?;
        let params = json.value().to_member("params")?.required()?;
        let input_track_id: String = params.to_member("inputTrackId")?.required()?.try_into()?;
        assert_eq!(input_track_id, "obsws:record:1:source:0:encoded_video");
        let output_track_id: String = params.to_member("outputTrackId")?.required()?.try_into()?;
        assert_eq!(output_track_id, "obsws:record:1:source:0:raw_video");

        // createAudioDecoder のパラメータを検証する
        let json = nojson::RawJson::parse(&plan.requests[2].request_text)?;
        let params = json.value().to_member("params")?.required()?;
        let input_track_id: String = params.to_member("inputTrackId")?.required()?.try_into()?;
        assert_eq!(input_track_id, "obsws:record:1:source:0:encoded_audio");
        let output_track_id: String = params.to_member("outputTrackId")?.required()?.try_into()?;
        assert_eq!(output_track_id, "obsws:record:1:source:0:raw_audio");

        Ok(())
    }

    #[test]
    fn build_record_source_plan_without_stream_name() -> Result<(), Box<dyn std::error::Error>> {
        let plan = build_record_source_plan(
            &ObswsRtmpInboundSettings {
                input_url: Some("rtmp://127.0.0.1:1935".to_owned()),
                stream_name: None,
            },
            ObswsOutputKind::Record,
            2,
            1,
        )
        .expect("rtmp_inbound source plan without stream_name must succeed");

        let json = nojson::RawJson::parse(&plan.requests[0].request_text)?;
        let params = json.value().to_member("params")?.required()?;
        let stream_name: Option<String> = params.to_member("streamName")?.try_into()?;
        assert_eq!(stream_name, None);
        Ok(())
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
            0,
        );
        assert!(matches!(
            result,
            Err(BuildObswsRecordSourcePlanError::MissingRequiredField(
                "inputUrl"
            ))
        ));
    }
}
