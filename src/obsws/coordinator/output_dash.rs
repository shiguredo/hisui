//! MPEG-DASH ライブ出力の output エンジン。
//! Program 出力を DASH セグメント + MPD マニフェストとして出力するための processor 起動・停止を行う。
//! ABR (Adaptive Bitrate) 対応として複数 variant の並行処理を管理する。

use std::time::Duration;

use super::ObswsCoordinator;
use super::ObswsProgramOutputContext;
use super::output::{
    OutputOperationOutcome, build_s3_client, terminate_and_wait, wait_or_terminate,
};
use super::output_registry::{OutputRun, OutputSettings};

impl ObswsCoordinator {
    pub(crate) async fn handle_start_mpeg_dash(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::state::{
            DashDestination, ObswsDashRun, ObswsDashVariantRun, ObswsRecordTrackRun,
        };

        let Some(output) = self.outputs.get(output_name) else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                ),
            );
        };
        let OutputSettings::MpegDash(dash_settings) = &output.settings else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Output is not a MPEG-DASH output",
                ),
            );
        };
        let dash_settings = dash_settings.clone();

        if output.runtime.active {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "MPEG-DASH is already active",
                ),
            );
        }
        let Some(ref destination) = dash_settings.destination else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing outputSettings.destination field",
                ),
            );
        };
        if dash_settings.variants.is_empty() {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "variants must not be empty",
                ),
            );
        }
        let run_id = self.next_output_run_id;
        self.next_output_run_id = self.next_output_run_id.wrapping_add(1);
        let program_output = ObswsProgramOutputContext {
            video_track_id: self.program_output.video_track_id.clone(),
            audio_track_id: self.program_output.audio_track_id.clone(),
            canvas_width: self.state.canvas_width(),
            canvas_height: self.state.canvas_height(),
            frame_rate: self.state.frame_rate(),
        };
        let is_abr = dash_settings.variants.len() > 1;
        let variant_runs: Vec<ObswsDashVariantRun> = dash_settings
            .variants
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                let variant_label = format!("v{i}");
                let video = ObswsRecordTrackRun::new(
                    output_name,
                    run_id,
                    &format!("{variant_label}_video"),
                    &program_output.video_track_id,
                );
                let audio = ObswsRecordTrackRun::new(
                    output_name,
                    run_id,
                    &format!("{variant_label}_audio"),
                    &program_output.audio_track_id,
                );
                // variant ごとの fps 調整が必要になった場合は、この後段に映像整形を追加する。
                let needs_scaler = variant.width.zip(variant.height).is_some_and(|(w, h)| {
                    w != program_output.canvas_width || h != program_output.canvas_height
                });
                let scaler_processor_id = if needs_scaler {
                    Some(crate::ProcessorId::new(format!(
                        "output:{output_name}:{variant_label}_scaler:{run_id}"
                    )))
                } else {
                    None
                };
                let scaled_track_id = if needs_scaler {
                    Some(crate::TrackId::new(format!(
                        "output:{output_name}:{variant_label}_scaled_video:{run_id}"
                    )))
                } else {
                    None
                };
                let writer_processor_id = crate::ProcessorId::new(format!(
                    "output:{output_name}:{variant_label}_dash_writer:{run_id}"
                ));
                let variant_path = if is_abr {
                    destination.variant_path(i)
                } else {
                    match destination {
                        DashDestination::Filesystem { directory } => directory.clone(),
                        DashDestination::S3 { prefix, .. } => prefix.clone(),
                    }
                };
                ObswsDashVariantRun {
                    video,
                    audio,
                    scaler_processor_id,
                    scaled_track_id,
                    writer_processor_id,
                    variant_path,
                }
            })
            .collect();
        let run = ObswsDashRun {
            destination: destination.clone(),
            variant_runs,
        };
        // ランタイム状態を active にする
        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = true;
            output.runtime.started_at = Some(std::time::Instant::now());
            output.runtime.run = Some(OutputRun::MpegDash(run.clone()));
        }
        // filesystem の場合のみ出力ディレクトリを作成する
        if let DashDestination::Filesystem { directory } = destination
            && let Err(e) = std::fs::create_dir_all(directory)
        {
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
            let error_comment = format!("Failed to create MPEG-DASH output directory: {e}");
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        // S3 + lifetimeDays 指定時はバケットに lifecycle ルールを設定する
        if let DashDestination::S3 {
            bucket,
            prefix,
            region,
            endpoint,
            use_path_style,
            access_key_id,
            secret_access_key,
            session_token,
            lifetime_days: Some(days),
        } = destination
        {
            let s3_client = build_s3_client(
                region,
                access_key_id,
                secret_access_key,
                session_token.as_deref(),
                endpoint.as_deref(),
                *use_path_style,
            );
            match s3_client {
                Ok(client) => {
                    let rule_id = format!("hisui-dash-{}", prefix.replace('/', "-"));
                    let rule = shiguredo_s3::types::LifecycleRule {
                        id: Some(rule_id),
                        status: shiguredo_s3::types::ExpirationStatus::Enabled,
                        filter: Some(shiguredo_s3::types::LifecycleRuleFilter {
                            prefix: Some(prefix.clone()),
                            tag: None,
                            object_size_greater_than: None,
                            object_size_less_than: None,
                            and: None,
                        }),
                        expiration: Some(shiguredo_s3::types::LifecycleExpiration {
                            days: Some(*days as i32),
                            date: None,
                            expired_object_delete_marker: None,
                        }),
                        transitions: None,
                        noncurrent_version_transitions: None,
                        noncurrent_version_expiration: None,
                        abort_incomplete_multipart_upload: None,
                    };
                    let request = client
                        .client()
                        .put_bucket_lifecycle_configuration()
                        .bucket(bucket)
                        .rule(rule)
                        .build_request();
                    match request {
                        Ok(req) => match client.execute(&req).await {
                            Ok(response) if !response.is_success() => {
                                tracing::warn!(
                                    "PutBucketLifecycleConfiguration failed: status={}",
                                    response.status_code
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "failed to set S3 lifecycle configuration: {}",
                                    e.display()
                                );
                            }
                            _ => {}
                        },
                        Err(e) => {
                            tracing::warn!(
                                "failed to build PutBucketLifecycleConfiguration request: {e}"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "failed to build S3 client for lifecycle configuration: {}",
                        e.display()
                    );
                }
            }
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        match start_dash_processors(pipeline_handle, &program_output, &run, &dash_settings).await {
            Ok(combined_mpd_task) => {
                if let Some(output) = self.outputs.get_mut(output_name) {
                    output.runtime.background_task = combined_mpd_task;
                }
            }
            Err(e) => {
                if let Some(output) = self.outputs.get_mut(output_name) {
                    output.runtime.active = false;
                    output.runtime.started_at = None;
                    output.runtime.run = None;
                }
                let _ = stop_processors_staged_dash(pipeline_handle, &run).await;
                let error_comment = format!("Failed to start MPEG-DASH: {}", e.display());
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        &error_comment,
                    ),
                );
            }
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_output_response(request_id),
            None,
        )
    }

    pub(crate) async fn handle_stop_mpeg_dash(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        let run = self.outputs.get_mut(output_name).and_then(|o| {
            if let Some(handle) = o.runtime.background_task.take() {
                handle.abort();
            }
            let run = o.runtime.run.take();
            o.runtime.active = false;
            o.runtime.started_at = None;
            match run {
                Some(OutputRun::MpegDash(r)) => Some(r),
                _ => None,
            }
        });
        let Some(run) = run else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "MPEG-DASH is not active",
                ),
            );
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_dash(pipeline_handle, &run).await
        {
            tracing::warn!("failed to stop MPEG-DASH processors: {}", e.display());
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }
}

/// 戻り値は ABR 結合 MPD 書き出しタスクの JoinHandle（ABR でない場合は None）。
/// 呼び出し元は JoinHandle を保持し、出力停止時に abort() すること。
async fn start_dash_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    program_output: &ObswsProgramOutputContext,
    run: &crate::obsws::state::ObswsDashRun,
    dash_settings: &crate::obsws::state::ObswsDashSettings,
) -> crate::Result<Option<tokio::task::JoinHandle<()>>> {
    // MPEG-DASH 用にキーフレーム間隔を設定する
    let fps = program_output.frame_rate.numerator.get() as f64
        / program_output.frame_rate.denumerator.get() as f64;
    let keyframe_interval_frames = (dash_settings.segment_duration * fps).ceil() as u32;
    let keyframe_interval_frames = keyframe_interval_frames.max(1);
    let encode_params = crate::encoder::encode_config_with_keyframe_interval(
        keyframe_interval_frames,
        program_output.frame_rate,
    );

    let is_abr = run.is_abr();

    // ABR の場合、各 variant writer が SampleEntry から codec string を確定したら
    // oneshot channel 経由で通知を受け取り、全 variant の値がそろってから結合 MPD を書き出す。
    let mut codec_string_receivers = Vec::new();

    // バリアントごとにスケーラー、エンコーダー、ライターを起動する
    for (i, (variant, variant_run)) in dash_settings
        .variants
        .iter()
        .zip(run.variant_runs.iter())
        .enumerate()
    {
        // filesystem かつ ABR の場合はバリアントのサブディレクトリを作成する
        if is_abr && let crate::obsws::state::DashDestination::Filesystem { .. } = run.destination {
            std::fs::create_dir_all(&variant_run.variant_path).map_err(|e| {
                crate::Error::new(format!(
                    "failed to create variant directory {}: {e}",
                    variant_run.variant_path
                ))
            })?;
        }

        // 解像度変換が必要な場合はスケーラーを挿入する
        let video_encoder_input_track = if let (Some(scaler_id), Some(scaled_track_id)) = (
            &variant_run.scaler_processor_id,
            &variant_run.scaled_track_id,
        ) {
            let width = variant.width.expect("infallible: scaler requires width");
            let height = variant.height.expect("infallible: scaler requires height");
            crate::scaler::create_processor(
                pipeline_handle,
                crate::scaler::VideoScalerConfig {
                    input_track_id: program_output.video_track_id.clone(),
                    output_track_id: scaled_track_id.clone(),
                    width,
                    height,
                },
                Some(scaler_id.clone()),
            )
            .await?;
            scaled_track_id.clone()
        } else {
            variant_run.video.source_track_id.clone()
        };

        // ビデオエンコーダー
        crate::encoder::create_video_processor_with_params(
            pipeline_handle,
            video_encoder_input_track,
            variant_run.video.encoded_track_id.clone(),
            dash_settings.video_codec,
            std::num::NonZeroUsize::new(variant.video_bitrate_bps)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            program_output.frame_rate,
            Some(encode_params.clone()),
            Some(variant_run.video.encoder_processor_id.clone()),
        )
        .await?;

        // オーディオエンコーダー
        crate::encoder::create_audio_processor(
            pipeline_handle,
            program_output.audio_track_id.clone(),
            variant_run.audio.encoded_track_id.clone(),
            dash_settings.audio_codec,
            std::num::NonZeroUsize::new(variant.audio_bitrate_bps)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            Some(variant_run.audio.encoder_processor_id.clone()),
        )
        .await?;

        // DASH ライター
        let storage_config = match &run.destination {
            crate::obsws::state::DashDestination::Filesystem { .. } => {
                crate::dash::writer::DashStorageConfig::Filesystem {
                    output_directory: std::path::PathBuf::from(&variant_run.variant_path),
                }
            }
            crate::obsws::state::DashDestination::S3 {
                bucket,
                region,
                endpoint,
                use_path_style,
                access_key_id,
                secret_access_key,
                session_token,
                ..
            } => {
                let client = build_s3_client(
                    region,
                    access_key_id,
                    secret_access_key,
                    session_token.as_deref(),
                    endpoint.as_deref(),
                    *use_path_style,
                )?;
                crate::dash::writer::DashStorageConfig::S3 {
                    client,
                    bucket: bucket.clone(),
                    prefix: variant_run.variant_path.clone(),
                }
            }
        };
        // ABR の場合は codec string 通知用の channel を作成する
        let codec_string_sender = if is_abr {
            let (tx, rx) = tokio::sync::oneshot::channel();
            codec_string_receivers.push(rx);
            Some(tx)
        } else {
            None
        };

        crate::dash::writer::create_processor(
            pipeline_handle,
            crate::dash::writer::DashWriterConfig {
                storage: storage_config,
                input_audio_track_id: variant_run.audio.encoded_track_id.clone(),
                input_video_track_id: variant_run.video.encoded_track_id.clone(),
                segment_duration: dash_settings.segment_duration,
                max_retained_segments: dash_settings.max_retained_segments,
                skip_mpd: is_abr,
                codec_string_sender,
            },
            Some(variant_run.writer_processor_id.clone()),
        )
        .await?;

        tracing::info!(
            variant = i,
            video_bitrate = variant.video_bitrate_bps,
            audio_bitrate = variant.audio_bitrate_bps,
            directory = %variant_run.variant_path,
            "MPEG-DASH variant processor started"
        );
    }

    // ABR の場合は各 variant writer が SampleEntry から codec string を確定するのを待ち、
    // 全 variant の codec string が一致することを検証してから結合 MPD を書き出す。
    if is_abr {
        let mpd_variants: Vec<crate::dash::writer::CombinedMpdVariant> = dash_settings
            .variants
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                let width = variant
                    .width
                    .map(|w| w.get() as u32)
                    .unwrap_or(program_output.canvas_width.get() as u32);
                let height = variant
                    .height
                    .map(|h| h.get() as u32)
                    .unwrap_or(program_output.canvas_height.get() as u32);
                Ok(crate::dash::writer::CombinedMpdVariant {
                    bandwidth: variant.video_bitrate_bps as u64 + variant.audio_bitrate_bps as u64,
                    width,
                    height,
                    media_path: dash_variant_media_path(&run.destination, &run.variant_runs[i])?,
                    init_path: dash_variant_init_path(&run.destination, &run.variant_runs[i])?,
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        let root_storage_config = build_dash_root_storage_config(&run.destination)?;
        let segment_duration = dash_settings.segment_duration;
        let max_retained_segments = dash_settings.max_retained_segments;

        // 各 variant の codec string が確定するのを待ってから結合 MPD を書き出すタスクを起動する。
        // JoinHandle を呼び出し元に返し、出力停止時に abort() でキャンセルできるようにする。
        let handle = tokio::spawn(async move {
            // 全 variant の codec string を収集する
            let mut codec_strings = Vec::with_capacity(codec_string_receivers.len());
            for (i, rx) in codec_string_receivers.into_iter().enumerate() {
                match rx.await {
                    Ok(cs) => codec_strings.push(cs),
                    Err(_) => {
                        tracing::warn!(
                            variant = i,
                            "DASH variant writer dropped codec string sender before resolving codecs"
                        );
                        return;
                    }
                }
            }

            // 全 variant の codec string が一致することを検証する
            if let Some(first) = codec_strings.first() {
                for (i, cs) in codec_strings.iter().enumerate().skip(1) {
                    if cs.video != first.video || cs.audio != first.audio {
                        tracing::error!(
                            variant = i,
                            expected_video = %first.video,
                            expected_audio = %first.audio,
                            actual_video = %cs.video,
                            actual_audio = %cs.audio,
                            "DASH ABR variant codec string mismatch: \
                             all variants must produce identical codec strings"
                        );
                        return;
                    }
                }

                if let Err(e) = crate::dash::writer::write_combined_mpd(
                    root_storage_config,
                    &mpd_variants,
                    segment_duration,
                    max_retained_segments,
                    first,
                )
                .await
                {
                    tracing::error!(error = ?e, "failed to write combined DASH MPD");
                }
            }
        });
        Ok(Some(handle))
    } else {
        Ok(None)
    }
}

/// MPEG-DASH 用プロセッサを段階的に停止する。
/// Program 出力は共有なので、variant 後段の processor のみを停止する。
async fn stop_processors_staged_dash(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::state::ObswsDashRun,
) -> crate::Result<()> {
    // NOTE:
    // ライブ用途では StopOutput / ToggleOutput への応答遅延を避けることを優先し、
    // ここでは writer に finalize / cleanup を先行させる。
    // この経路は上流 encoder / scaler の完全 drain を保証しないため、
    // 停止直前の数フレームが最終 segment や MPD に反映されない可能性がある。
    //
    // TODO:
    // 末尾欠損まで解消するには、writer を先に閉じるのではなく、
    // 上流から EOS 相当を伝播させる明示的な finish 経路が必要になる。
    // terminate_processor() は abort ベースで停止するだけなので、
    // encoder / scaler の残フレーム排出には使えない。
    // 1. 各 writer に finalize / cleanup を要求し、停止を待つ。
    let writer_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .map(|vr| vr.writer_processor_id.clone())
        .collect();
    for writer_id in &writer_ids {
        finish_dash_writer_rpc(pipeline_handle, writer_id).await;
    }
    wait_or_terminate(pipeline_handle, &writer_ids, Duration::from_secs(5)).await?;

    // 2. 全バリアントのエンコーダーを停止する。
    let encoder_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .flat_map(|vr| {
            [
                vr.video.encoder_processor_id.clone(),
                vr.audio.encoder_processor_id.clone(),
            ]
        })
        .collect();
    terminate_and_wait(pipeline_handle, &encoder_ids).await?;

    // 3. 解像度変換があるバリアントのスケーラーを停止する。
    let scaler_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .filter_map(|vr| vr.scaler_processor_id.clone())
        .collect();
    if !scaler_ids.is_empty() {
        terminate_and_wait(pipeline_handle, &scaler_ids).await?;
    }

    // ABR の場合は結合 MPD とバリアントディレクトリを削除する
    if run.is_abr() {
        if let Ok(root_storage_config) = build_dash_root_storage_config(&run.destination) {
            crate::dash::writer::delete_combined_mpd(root_storage_config).await;
        }
        // filesystem の場合はバリアントのサブディレクトリも削除する（ライターが中身を削除済みなので空のはず）
        if let crate::obsws::state::DashDestination::Filesystem { .. } = &run.destination {
            for vr in &run.variant_runs {
                if let Err(e) = std::fs::remove_dir(&vr.variant_path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(
                        "failed to remove variant directory {}: {e}",
                        vr.variant_path
                    );
                }
            }
        }
    }

    Ok(())
}

/// DASH writer に Finish RPC を送り、finalize / cleanup を促す。
/// これは writer 側の入力購読を閉じるためのもので、上流の完全 drain は保証しない。
/// 失敗時は terminate にフォールバックする。
async fn finish_dash_writer_rpc(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: &crate::ProcessorId,
) {
    const RETRY_TIMEOUT: Duration = Duration::from_millis(500);
    const RETRY_INTERVAL: Duration = Duration::from_millis(10);
    let deadline = tokio::time::Instant::now() + RETRY_TIMEOUT;

    loop {
        match pipeline_handle
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::dash::writer::DashWriterRpcMessage,
            >>(processor_id)
            .await
        {
            Ok(sender) => {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                let _ = sender.send(crate::dash::writer::DashWriterRpcMessage::Finish { reply_tx });
                let _ = reply_rx.await;
                return;
            }
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(_) => {
                let _ = pipeline_handle.terminate_processor(processor_id.clone()).await;
                return;
            }
        }
    }
}

/// 結合 MPD に書く media path を生成する。
/// writer が実際に使う variant_path と同じ規則から相対パスを導出する。
fn dash_variant_media_path(
    destination: &crate::obsws::state::DashDestination,
    variant_run: &crate::obsws::state::ObswsDashVariantRun,
) -> crate::Result<String> {
    let base_path = dash_variant_relative_path(destination, &variant_run.variant_path)?;
    Ok(format!("{base_path}/segment-$Number%06d$.m4s"))
}

/// 結合 MPD に書く init segment path を生成する。
/// writer が実際に使う variant_path と同じ規則から相対パスを導出する。
fn dash_variant_init_path(
    destination: &crate::obsws::state::DashDestination,
    variant_run: &crate::obsws::state::ObswsDashVariantRun,
) -> crate::Result<String> {
    let base_path = dash_variant_relative_path(destination, &variant_run.variant_path)?;
    Ok(format!("{base_path}/init.mp4"))
}

/// variant_path から結合 MPD 用の相対パス部分を取り出す。
fn dash_variant_relative_path(
    destination: &crate::obsws::state::DashDestination,
    variant_path: &str,
) -> crate::Result<String> {
    match destination {
        crate::obsws::state::DashDestination::Filesystem { directory } => {
            let root = std::path::Path::new(directory);
            let path = std::path::Path::new(variant_path);
            let relative = path.strip_prefix(root).map_err(|_| {
                crate::Error::new(format!(
                    "variant path {variant_path} is not under DASH destination root {directory}"
                ))
            })?;
            Ok(relative.to_string_lossy().replace('\\', "/"))
        }
        crate::obsws::state::DashDestination::S3 { prefix, .. } => {
            if prefix.is_empty() {
                return Ok(variant_path.to_owned());
            }
            let Some(relative) = variant_path.strip_prefix(prefix) else {
                return Err(crate::Error::new(format!(
                    "variant path {variant_path} does not start with DASH destination prefix {prefix}"
                )));
            };
            Ok(relative.trim_start_matches('/').to_owned())
        }
    }
}

/// DASH destination からルートディレクトリ/prefix 用の DashStorageConfig を構築する。
/// 結合 MPD の書き出し・削除に使用する。
fn build_dash_root_storage_config(
    destination: &crate::obsws::state::DashDestination,
) -> crate::Result<crate::dash::writer::DashStorageConfig> {
    match destination {
        crate::obsws::state::DashDestination::Filesystem { directory } => {
            Ok(crate::dash::writer::DashStorageConfig::Filesystem {
                output_directory: std::path::PathBuf::from(directory),
            })
        }
        crate::obsws::state::DashDestination::S3 {
            bucket,
            prefix,
            region,
            endpoint,
            use_path_style,
            access_key_id,
            secret_access_key,
            session_token,
            ..
        } => {
            let client = build_s3_client(
                region,
                access_key_id,
                secret_access_key,
                session_token.as_deref(),
                endpoint.as_deref(),
                *use_path_style,
            )?;
            Ok(crate::dash::writer::DashStorageConfig::S3 {
                client,
                bucket: bucket.clone(),
                prefix: prefix.clone(),
            })
        }
    }
}
