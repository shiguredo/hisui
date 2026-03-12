use crate::obsws::source::{self, ObswsOutputKind, ObswsRecordSourcePlan};
use crate::obsws_input_registry::ObswsInputEntry;

pub struct ObswsComposedOutputPlan {
    pub source_plans: Vec<ObswsRecordSourcePlan>,
    pub source_processor_ids: Vec<String>,
    pub source_video_track_id: Option<String>,
    pub source_audio_track_id: Option<String>,
    pub audio_mixer_processor_id: Option<String>,
}

#[derive(Debug)]
pub enum BuildObswsComposedOutputPlanError {
    Source(source::BuildObswsRecordSourcePlanError),
    NoEnabledInputs,
    NoOutputTracks,
    MultipleVideoInputsUnsupported,
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
            Self::MultipleVideoInputsUnsupported => {
                format!("At most one video input is supported for {request_type}")
            }
        }
    }
}

pub fn build_composed_output_plan(
    scene_inputs: &[ObswsInputEntry],
    output_kind: ObswsOutputKind,
    run_id: u64,
) -> Result<ObswsComposedOutputPlan, BuildObswsComposedOutputPlanError> {
    if scene_inputs.is_empty() {
        return Err(BuildObswsComposedOutputPlanError::NoEnabledInputs);
    }

    let mut source_plans = Vec::with_capacity(scene_inputs.len());
    for (source_index, input) in scene_inputs.iter().enumerate() {
        let source_plan =
            source::build_record_source_plan(input, output_kind, run_id, source_index)
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
    if video_track_ids.len() > 1 {
        return Err(BuildObswsComposedOutputPlanError::MultipleVideoInputsUnsupported);
    }

    let source_audio_track_id = if audio_track_ids.len() > 1 {
        Some(format!(
            "obsws:{}:{run_id}:mixed_audio",
            output_kind.as_str()
        ))
    } else {
        audio_track_ids.first().cloned()
    };
    let audio_mixer_processor_id = (audio_track_ids.len() > 1)
        .then(|| format!("obsws:{}:{run_id}:audio_mixer", output_kind.as_str()));

    Ok(ObswsComposedOutputPlan {
        source_processor_ids: source_plans
            .iter()
            .map(|plan| plan.source_processor_id.clone())
            .collect(),
        source_plans,
        source_video_track_id: video_track_ids.first().cloned(),
        source_audio_track_id,
        audio_mixer_processor_id,
    })
}
