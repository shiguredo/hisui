use crate::obsws::input_registry::ObswsSceneInputEntry;
use crate::obsws::source::{self, ObswsOutputKind, ObswsRecordSourcePlan};
use crate::types::PositiveFiniteF64;
use crate::{ProcessorId, TrackId};

pub struct ObswsComposedOutputPlan {
    pub source_plans: Vec<ObswsRecordSourcePlan>,
    pub source_processor_ids: Vec<ProcessorId>,
    pub video_track_id: TrackId,
    pub audio_track_id: TrackId,
    pub audio_mixer_processor_id: ProcessorId,
    pub video_mixer_processor_id: ProcessorId,
    pub video_mixer_input_tracks: Vec<ObswsVideoMixerInputTrack>,
    pub canvas_width: crate::types::EvenUsize,
    pub canvas_height: crate::types::EvenUsize,
    pub frame_rate: crate::video::FrameRate,
}

#[derive(Debug)]
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
}

impl BuildObswsComposedOutputPlanError {
    pub fn message(&self) -> String {
        match self {
            Self::Source(error) => error.message(),
        }
    }
}

/// 偶数に丸める（映像フレームサイズの要件）
fn round_to_even(value: f64) -> u32 {
    let v = value.round() as u32;
    if v.is_multiple_of(2) { v } else { v + 1 }
}

/// OBS 互換の出力プランを構築する。
/// OBS と同様に、ソースの有無に関わらず常に映像（黒画面）と音声（無音）の両トラックを含める。
pub fn build_composed_output_plan(
    scene_inputs: &[ObswsSceneInputEntry],
    output_kind: ObswsOutputKind,
    run_id: u64,
    canvas_width: crate::types::EvenUsize,
    canvas_height: crate::types::EvenUsize,
    frame_rate: crate::video::FrameRate,
) -> Result<ObswsComposedOutputPlan, BuildObswsComposedOutputPlanError> {
    let mut source_plans = Vec::with_capacity(scene_inputs.len());
    let mut active_scene_inputs = Vec::with_capacity(scene_inputs.len());
    for scene_input in scene_inputs.iter() {
        if !source::is_source_startable(&scene_input.input.input.settings) {
            continue;
        }
        let source_plan = source::build_record_source_plan(
            &scene_input.input,
            output_kind,
            run_id,
            &scene_input.input.input_uuid,
            frame_rate,
        )
        .map_err(BuildObswsComposedOutputPlanError::Source)?;
        source_plans.push(source_plan);
        active_scene_inputs.push(scene_input);
    }

    // 常にオーディオミキサーを使用する。
    // 音声ソースがない場合でも無音を出力する。
    let audio_track_id = TrackId::new(format!(
        "obsws:{}:{run_id}:mixed_audio",
        output_kind.as_str()
    ));
    let audio_mixer_processor_id = ProcessorId::new(format!(
        "obsws:{}:{run_id}:audio_mixer",
        output_kind.as_str()
    ));

    // 常に映像ミキサーを使用する。
    // 映像ソースがない場合でも黒画面を出力する。
    let video_track_id = TrackId::new(format!(
        "obsws:{}:{run_id}:mixed_video",
        output_kind.as_str()
    ));
    let video_mixer_processor_id = ProcessorId::new(format!(
        "obsws:{}:{run_id}:video_mixer",
        output_kind.as_str()
    ));

    // source_plans と active_scene_inputs は同じ順序・同じ長さ
    let video_mixer_input_tracks = source_plans
        .iter()
        .zip(active_scene_inputs.iter())
        .filter_map(|(plan, scene_input)| {
            let video_track_id = plan.source_video_track_id.as_ref()?;
            let transform = &scene_input.transform;
            // width/height が 0（ソースサイズ未確定）の場合、bounds をフォールバックとして使用する。
            //
            // sora_source や webrtc_source のような外部フレーム供給型のソースでは、
            // フレーム到着前に source_width/height が確定しない。mixer は width/height が
            // None の場合にフレームの元サイズをそのまま使って描画するが、それでは bounds
            // によるスケーリング指定が反映されない。
            //
            // 暫定対応として、width/height が 0 かつ boundsType が指定されている場合に
            // bounds_width/bounds_height をそのまま描画サイズとして使う。
            //
            // 将来的な改善: 初回フレーム到着時に source_width/source_height を動的に
            // 更新し、OBS 互換の boundsType に基づくスケーリング計算
            // （アスペクト比保持等）を正確に行う。
            let has_bounds = transform.bounds_type != "OBS_BOUNDS_NONE";
            let effective_width = if transform.width > 0.0 {
                transform.width
            } else if has_bounds && transform.bounds_width > 0.0 {
                transform.bounds_width
            } else {
                0.0
            };
            let effective_height = if transform.height > 0.0 {
                transform.height
            } else if has_bounds && transform.bounds_height > 0.0 {
                transform.bounds_height
            } else {
                0.0
            };
            let width = if effective_width > 0.0 {
                Some(round_to_even(effective_width))
            } else {
                None
            };
            let height = if effective_height > 0.0 {
                Some(round_to_even(effective_height))
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

    Ok(ObswsComposedOutputPlan {
        source_processor_ids: source_plans
            .iter()
            .flat_map(|plan| plan.source_processor_ids.iter().cloned())
            .collect(),
        source_plans,
        video_track_id,
        audio_track_id,
        audio_mixer_processor_id,
        video_mixer_processor_id,
        video_mixer_input_tracks,
        canvas_width,
        canvas_height,
        frame_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsws::input_registry::{
        ObswsInput, ObswsInputEntry, ObswsInputSettings, ObswsSceneItemTransform,
    };

    fn parse_owned_json(text: &str) -> nojson::RawJsonOwned {
        nojson::RawJsonOwned::parse(text).expect("test json must be valid")
    }

    #[test]
    fn build_composed_output_plan_skips_dormant_inputs() {
        let dormant_input = ObswsSceneInputEntry {
            input: ObswsInputEntry::new_for_test(
                "input-1",
                "dormant-image",
                ObswsInput {
                    settings: ObswsInputSettings::from_kind_and_settings(
                        "image_source",
                        parse_owned_json("{}").value(),
                    )
                    .expect("image_source settings must parse"),
                    input_muted: false,
                    input_volume_mul: crate::types::NonNegFiniteF64::ONE,
                },
            ),
            scene_item_index: 0,
            transform: ObswsSceneItemTransform::default(),
        };
        let active_input = ObswsSceneInputEntry {
            input: ObswsInputEntry::new_for_test(
                "input-2",
                "color",
                ObswsInput {
                    settings: ObswsInputSettings::ColorSource(
                        crate::obsws::input_registry::ObswsColorSourceSettings {
                            color: Some("#FF0000".to_owned()),
                        },
                    ),
                    input_muted: false,
                    input_volume_mul: crate::types::NonNegFiniteF64::ONE,
                },
            ),
            scene_item_index: 1,
            transform: ObswsSceneItemTransform::default(),
        };

        let plan = build_composed_output_plan(
            &[dormant_input, active_input],
            ObswsOutputKind::Program,
            0,
            crate::types::EvenUsize::new(1280).expect("valid width"),
            crate::types::EvenUsize::new(720).expect("valid height"),
            crate::video::FrameRate::FPS_30,
        )
        .expect("output plan must build");

        assert_eq!(plan.source_plans.len(), 1);
        assert_eq!(plan.source_processor_ids.len(), 1);
    }
}
