use crate::obsws::source::{
    BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan, ObswsSourceRequest,
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

    let source = crate::VideoDeviceSource {
        output_video_track_id: raw_video_track_id.clone(),
        device_id: settings.device_id.clone(),
        width: None,
        height: None,
        fps: None,
    };

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![source_processor_id.clone()],
        source_video_track_id: Some(raw_video_track_id),
        source_audio_track_id: None,
        requests: vec![ObswsSourceRequest::CreateVideoDeviceSource {
            source,
            processor_id: Some(source_processor_id),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_record_source_plan_with_device_id() {
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

        assert_eq!(
            plan.source_video_track_id.as_ref().map(|t| t.get()),
            Some("obsws:record:1:source:0:raw_video")
        );
        assert!(plan.source_audio_track_id.is_none());

        // CreateVideoDeviceSource のパラメータを検証する
        match &plan.requests[0] {
            ObswsSourceRequest::CreateVideoDeviceSource {
                source,
                processor_id,
            } => {
                assert_eq!(
                    source.output_video_track_id.get(),
                    "obsws:record:1:source:0:raw_video"
                );
                assert_eq!(source.device_id.as_deref(), Some("camera0"));
                assert_eq!(
                    processor_id.as_ref().map(|p| p.get()),
                    Some("obsws:record:1:source:0:video_device_source")
                );
            }
            _ => panic!("expected CreateVideoDeviceSource"),
        }
    }

    #[test]
    fn build_record_source_plan_without_device_id() {
        let plan = build_record_source_plan(
            &ObswsVideoCaptureDeviceSettings { device_id: None },
            ObswsOutputKind::Record,
            2,
            1,
        )
        .expect("video_capture_device source plan without device_id must succeed");

        match &plan.requests[0] {
            ObswsSourceRequest::CreateVideoDeviceSource { source, .. } => {
                assert_eq!(source.device_id, None);
            }
            _ => panic!("expected CreateVideoDeviceSource"),
        }
    }
}
