// --- Program 出力用の公開ヘルパー関数 ---

/// output_plan のビデオミキサー入力トラックを mixer::video::InputTrack に変換する
pub fn convert_video_mixer_input_tracks(
    output_plan: &crate::obsws::output_plan::ObswsComposedOutputPlan,
) -> Vec<crate::mixer::video::InputTrack> {
    output_plan
        .video_mixer_input_tracks
        .iter()
        .map(|t| crate::mixer::video::InputTrack {
            track_id: t.track_id.clone(),
            x: t.x as isize,
            y: t.y as isize,
            z: t.z as isize,
            width: t
                .width
                .and_then(|w| crate::types::EvenUsize::new(w as usize)),
            height: t
                .height
                .and_then(|h| crate::types::EvenUsize::new(h as usize)),
            scale_x: t.scale_x,
            scale_y: t.scale_y,
            crop_top: t.crop_top as usize,
            crop_bottom: t.crop_bottom as usize,
            crop_left: t.crop_left as usize,
            crop_right: t.crop_right as usize,
        })
        .collect()
}

/// output_plan のオーディオミキサー入力トラックを AudioRealtimeInputTrack に変換する
pub fn convert_audio_mixer_input_tracks(
    source_plans: &[crate::obsws::source::ObswsRecordSourcePlan],
) -> Vec<crate::mixer::audio::AudioRealtimeInputTrack> {
    source_plans
        .iter()
        .filter_map(|sp| {
            sp.source_audio_track_id.as_ref().map(|track_id| {
                crate::mixer::audio::AudioRealtimeInputTrack {
                    track_id: track_id.clone(),
                }
            })
        })
        .collect()
}

/// 映像・音声ミキサープロセッサを起動する（エンコーダは含まない）
pub async fn start_mixer_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_plan: &crate::obsws::output_plan::ObswsComposedOutputPlan,
) -> crate::Result<()> {
    // オーディオミキサーを起動する
    let audio_input_tracks = convert_audio_mixer_input_tracks(&output_plan.source_plans);
    let audio_mixer = crate::mixer::audio::AudioRealtimeMixer {
        sample_rate: crate::audio::SampleRate::HZ_48000,
        channels: crate::audio::Channels::STEREO,
        frame_duration: std::time::Duration::from_millis(20),
        timestamp_rebase_threshold: std::time::Duration::from_millis(100),
        terminate_on_input_eos: false,
        input_tracks: audio_input_tracks,
        output_track_id: output_plan.audio_track_id.clone(),
    };
    crate::mixer::audio::create_processor(
        pipeline_handle,
        audio_mixer,
        Some(output_plan.audio_mixer_processor_id.clone()),
    )
    .await?;

    // ビデオミキサーを起動する
    let video_input_tracks = convert_video_mixer_input_tracks(output_plan);
    let video_mixer = crate::mixer::video::VideoRealtimeMixer {
        canvas_width: output_plan.canvas_width,
        canvas_height: output_plan.canvas_height,
        frame_rate: output_plan.frame_rate,
        input_tracks: video_input_tracks,
        output_track_id: output_plan.video_track_id.clone(),
    };
    crate::mixer::video::create_processor(
        pipeline_handle,
        video_mixer,
        Some(output_plan.video_mixer_processor_id.clone()),
    )
    .await?;

    Ok(())
}

/// ソースプロセッサ群を起動する
pub async fn start_source_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    source_plans: &mut [crate::obsws::source::ObswsRecordSourcePlan],
) -> crate::Result<()> {
    for source_plan in source_plans {
        for request in source_plan.requests.drain(..) {
            request.execute(pipeline_handle).await?;
        }
    }
    Ok(())
}

/// ソースプロセッサ群を停止する
pub async fn stop_source_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_ids: &[crate::ProcessorId],
) -> crate::Result<()> {
    for processor_id in processor_ids {
        if let Err(e) = pipeline_handle
            .terminate_processor(processor_id.clone())
            .await
        {
            tracing::warn!("failed to terminate source processor {processor_id}: {e}",);
        }
    }
    Ok(())
}

/// Program ミキサーの入力トラックを更新する
pub async fn update_program_mixers(
    pipeline_handle: &crate::MediaPipelineHandle,
    output_plan: &crate::obsws::output_plan::ObswsComposedOutputPlan,
    video_mixer_processor_id: &crate::ProcessorId,
    audio_mixer_processor_id: &crate::ProcessorId,
) -> crate::Result<()> {
    // ビデオミキサーを更新する
    let video_input_tracks = convert_video_mixer_input_tracks(output_plan);
    let video_request = crate::mixer::video::VideoRealtimeMixerUpdateConfigRequest {
        canvas_width: output_plan.canvas_width,
        canvas_height: output_plan.canvas_height,
        frame_rate: output_plan.frame_rate,
        input_tracks: video_input_tracks,
    };
    crate::mixer::video::update_video_mixer(
        pipeline_handle,
        video_mixer_processor_id.clone(),
        video_request,
    )
    .await?;

    // オーディオミキサーを更新する
    let audio_input_tracks = convert_audio_mixer_input_tracks(&output_plan.source_plans);
    crate::mixer::audio::update_audio_mixer_inputs(
        pipeline_handle,
        audio_mixer_processor_id.clone(),
        audio_input_tracks,
    )
    .await?;

    Ok(())
}
