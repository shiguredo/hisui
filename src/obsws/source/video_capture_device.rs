use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan, ObswsSourceRpcRequest,
};
use crate::obsws_input_registry::ObswsVideoCaptureDeviceSettings;
use crate::{ProcessorId, TrackId};

pub(super) fn build_record_source_plan(
    settings: &ObswsVideoCaptureDeviceSettings,
    output_kind: ObswsOutputKind,
    run_id: u64,
    source_index: usize,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let kind = output_kind.as_str();
    let source_processor_id = ProcessorId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_index}:video_device_source"
    ));
    let raw_video_track_id = TrackId::new(format!(
        "obsws:{kind}:{run_id}:source:{source_index}:raw_video"
    ));

    // createVideoDeviceSource リクエスト
    let request_text = nojson::object(|f| {
        f.member("jsonrpc", "2.0")?;
        f.member("id", 1)?;
        f.member("method", "createVideoDeviceSource")?;
        f.member(
            "params",
            nojson::object(|f| {
                f.member("outputVideoTrackId", &raw_video_track_id)?;
                if let Some(device_id) = &settings.device_id {
                    f.member("deviceId", device_id)?;
                }
                f.member("processorId", &source_processor_id)
            }),
        )
    })
    .to_string();

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id],
        source_video_track_id: Some(raw_video_track_id),
        // video_capture_device は映像のみ出力する
        source_audio_track_id: None,
        requests: vec![ObswsSourceRpcRequest {
            method: "createVideoDeviceSource",
            request_text,
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_record_source_plan_with_device_id() -> Result<(), Box<dyn std::error::Error>> {
        let plan = build_record_source_plan(
            &ObswsVideoCaptureDeviceSettings {
                device_id: Some("camera0".to_owned()),
            },
            ObswsOutputKind::Record,
            1,
            0,
        )
        .expect("video_capture_device source plan must succeed");

        assert_eq!(plan.source_processor_ids.len(), 1);
        assert_eq!(
            plan.source_processor_ids[0].get(),
            "obsws:record:1:source:0:video_device_source"
        );

        assert_eq!(plan.requests.len(), 1);
        assert_eq!(plan.requests[0].method, "createVideoDeviceSource");

        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_video")
        );
        assert!(plan.source_audio_track_id.is_none());

        // createVideoDeviceSource のパラメータを検証する
        let json = nojson::RawJson::parse(&plan.requests[0].request_text)?;
        let params = json.value().to_member("params")?.required()?;
        let output_video_track_id: String = params
            .to_member("outputVideoTrackId")?
            .required()?
            .try_into()?;
        assert_eq!(output_video_track_id, "obsws:record:1:source:0:raw_video");
        let device_id: Option<String> = params.to_member("deviceId")?.try_into()?;
        assert_eq!(device_id.as_deref(), Some("camera0"));

        Ok(())
    }

    #[test]
    fn build_record_source_plan_without_device_id() -> Result<(), Box<dyn std::error::Error>> {
        let plan = build_record_source_plan(
            &ObswsVideoCaptureDeviceSettings { device_id: None },
            ObswsOutputKind::Record,
            2,
            1,
        )
        .expect("video_capture_device source plan without device_id must succeed");

        let json = nojson::RawJson::parse(&plan.requests[0].request_text)?;
        let params = json.value().to_member("params")?.required()?;
        let device_id: Option<String> = params.to_member("deviceId")?.try_into()?;
        assert_eq!(device_id, None);
        Ok(())
    }
}
