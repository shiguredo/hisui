use crate::obsws::source::{self, ObswsOutputKind, ObswsRecordSourcePlan};
use crate::obsws_input_registry::ObswsSceneInputEntry;
use crate::types::PositiveFiniteF64;
use crate::{ProcessorId, TrackId};

pub struct ObswsComposedOutputPlan {
    pub source_plans: Vec<ObswsRecordSourcePlan>,
    pub source_processor_ids: Vec<ProcessorId>,
    pub source_video_track_id: Option<TrackId>,
    pub source_audio_track_id: Option<TrackId>,
    pub audio_mixer_processor_id: Option<ProcessorId>,
    pub video_mixer_processor_id: Option<ProcessorId>,
    pub video_mixer_input_tracks: Vec<ObswsVideoMixerInputTrack>,
    pub canvas_width: crate::types::EvenUsize,
    pub canvas_height: crate::types::EvenUsize,
    pub frame_rate: crate::video::FrameRate,
}

pub struct ObswsVideoMixerInputTrack {
    pub track_id: TrackId,
    pub x: i64,
    pub y: i64,
    pub z: i64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub scale_x: Option<PositiveFiniteF64>,
    pub scale_y: Option<PositiveFiniteF64>,
    pub crop_top: u32,
    pub crop_bottom: u32,
    pub crop_left: u32,
    pub crop_right: u32,
}

#[derive(Debug)]
pub enum BuildObswsComposedOutputPlanError {
    Source(source::BuildObswsRecordSourcePlanError),
    NoEnabledInputs,
    NoOutputTracks,
}

impl BuildObswsComposedOutputPlanError {
    pub fn message(&self, request_type: &str) -> String {
        match self {
            Self::Source(error) => error.message(),
            Self::NoEnabledInputs => {
                "At least one enabled input is required in the current program scene".to_owned()
            }
            Self::NoOutputTracks => {
                format!("At least one audio or video track is required for {request_type}")
            }
        }
    }
}

/// 偶数に丸める（映像フレームサイズの要件）
fn round_to_even(value: f64) -> u32 {
    let v = value.round() as u32;
    if v.is_multiple_of(2) { v } else { v + 1 }
}

pub fn build_composed_output_plan(
    scene_inputs: &[ObswsSceneInputEntry],
    output_kind: ObswsOutputKind,
    run_id: u64,
    canvas_width: crate::types::EvenUsize,
    canvas_height: crate::types::EvenUsize,
    frame_rate: crate::video::FrameRate,
) -> Result<ObswsComposedOutputPlan, BuildObswsComposedOutputPlanError> {
    if scene_inputs.is_empty() {
        return Err(BuildObswsComposedOutputPlanError::NoEnabledInputs);
    }

    let mut source_plans = Vec::with_capacity(scene_inputs.len());
    for (source_index, scene_input) in scene_inputs.iter().enumerate() {
        let source_plan = source::build_record_source_plan(
            &scene_input.input,
            output_kind,
            run_id,
            source_index,
            frame_rate,
        )
        .map_err(BuildObswsComposedOutputPlanError::Source)?;
        source_plans.push(source_plan);
    }

    let audio_track_ids = source_plans
        .iter()
        .filter_map(|plan| plan.source_audio_track_id.clone())
        .collect::<Vec<_>>();
    let video_track_ids = source_plans
        .iter()
        .filter_map(|plan| plan.source_video_track_id.clone())
        .collect::<Vec<_>>();

    if audio_track_ids.is_empty() && video_track_ids.is_empty() {
        return Err(BuildObswsComposedOutputPlanError::NoOutputTracks);
    }

    // 複数音声入力の場合はオーディオミキサーを使用する
    let source_audio_track_id = if audio_track_ids.len() > 1 {
        Some(TrackId::new(format!(
            "obsws:{}:{run_id}:mixed_audio",
            output_kind.as_str()
        )))
    } else {
        audio_track_ids.first().cloned()
    };
    let audio_mixer_processor_id = (audio_track_ids.len() > 1).then(|| {
        ProcessorId::new(format!(
            "obsws:{}:{run_id}:audio_mixer",
            output_kind.as_str()
        ))
    });

    // 映像入力がある場合は映像ミキサーを使用する
    // 単一映像入力でも sceneItemTransform（position / scale / crop）を適用するためミキサーを経由する
    let (source_video_track_id, video_mixer_processor_id, video_mixer_input_tracks) =
        if !video_track_ids.is_empty() {
            let mixed_video_track_id = TrackId::new(format!(
                "obsws:{}:{run_id}:mixed_video",
                output_kind.as_str()
            ));
            let mixer_processor_id = ProcessorId::new(format!(
                "obsws:{}:{run_id}:video_mixer",
                output_kind.as_str()
            ));

            // source_plans と scene_inputs は同じ順序・同じ長さ
            let input_tracks = source_plans
                .iter()
                .zip(scene_inputs.iter())
                .filter_map(|(plan, scene_input)| {
                    let video_track_id = plan.source_video_track_id.as_ref()?;
                    let transform = &scene_input.transform;
                    let width = if transform.width > 0.0 {
                        Some(round_to_even(transform.width))
                    } else {
                        None
                    };
                    let height = if transform.height > 0.0 {
                        Some(round_to_even(transform.height))
                    } else {
                        None
                    };
                    // scale_x / scale_y は 1.0 以外の場合のみミキサーに渡す
                    let scale_x = if transform.scale_x != PositiveFiniteF64::ONE {
                        Some(transform.scale_x)
                    } else {
                        None
                    };
                    let scale_y = if transform.scale_y != PositiveFiniteF64::ONE {
                        Some(transform.scale_y)
                    } else {
                        None
                    };
                    Some(ObswsVideoMixerInputTrack {
                        track_id: video_track_id.clone(),
                        x: transform.position_x as i64,
                        y: transform.position_y as i64,
                        z: scene_input.scene_item_index as i64,
                        width,
                        height,
                        scale_x,
                        scale_y,
                        crop_top: transform.crop_top.max(0) as u32,
                        crop_bottom: transform.crop_bottom.max(0) as u32,
                        crop_left: transform.crop_left.max(0) as u32,
                        crop_right: transform.crop_right.max(0) as u32,
                    })
                })
                .collect();

            (
                Some(mixed_video_track_id),
                Some(mixer_processor_id),
                input_tracks,
            )
        } else {
            (None, None, Vec::new())
        };

    Ok(ObswsComposedOutputPlan {
        source_processor_ids: source_plans
            .iter()
            .map(|plan| plan.source_processor_id.clone())
            .collect(),
        source_plans,
        source_video_track_id,
        source_audio_track_id,
        audio_mixer_processor_id,
        video_mixer_processor_id,
        video_mixer_input_tracks,
        canvas_width,
        canvas_height,
        frame_rate,
    })
}
