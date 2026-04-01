use crate::TrackId;

use super::{BuildObswsRecordSourcePlanError, ObswsOutputKind, ObswsRecordSourcePlan};

/// sora_source 用のソースプランを構築する。
///
/// sora_source は Sora RecvOnly 接続のリモートトラックからフレームを受け取るため、
/// 自律的な source processor は生成しない。video_track_id と audio_track_id を確保して、
/// 実際のフレーム publish は coordinator 側の AttachSoraSourceTrack で行う。
pub fn build_record_source_plan(
    output_kind: ObswsOutputKind,
    source_key: &str,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let video_track_id = TrackId::new(format!(
        "sora_source:{}:{}:video",
        output_kind.as_str(),
        source_key
    ));
    let audio_track_id = TrackId::new(format!(
        "sora_source:{}:{}:audio",
        output_kind.as_str(),
        source_key
    ));

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![],
        source_video_track_id: Some(video_track_id),
        source_audio_track_id: Some(audio_track_id),
        requests: vec![],
    })
}
