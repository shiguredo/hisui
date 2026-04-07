//! HLS ライブ出力の output エンジン。
//! Program 出力を HLS セグメント + M3U8 プレイリストとして出力するための processor 起動・停止を行う。
//! ABR (Adaptive Bitrate) 対応として複数 variant の並行処理を管理する。

use std::time::Duration;

use super::ObswsCoordinator;
use super::ObswsProgramOutputContext;
use super::output::{
    OutputOperationOutcome, build_s3_client, terminate_and_wait, wait_or_terminate,
};
use super::output_dynamic::{OutputRun, OutputSettings};

impl ObswsCoordinator {
    pub(crate) async fn handle_start_hls(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        use crate::obsws::input_registry::{
            HlsDestination, ObswsHlsRun, ObswsHlsVariantRun, ObswsRecordTrackRun,
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
        let OutputSettings::Hls(hls_settings) = &output.settings else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Output is not an HLS output",
                ),
            );
        };
        let hls_settings = hls_settings.clone();

        if output.runtime.active {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "HLS is already active",
                ),
            );
        }
        let Some(ref destination) = hls_settings.destination else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing outputSettings.destination field",
                ),
            );
        };
        if hls_settings.variants.is_empty() {
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
            canvas_width: self.input_registry.canvas_width(),
            canvas_height: self.input_registry.canvas_height(),
            frame_rate: self.input_registry.frame_rate(),
        };
        let is_abr = hls_settings.variants.len() > 1;
        let variant_runs: Vec<ObswsHlsVariantRun> = hls_settings
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
                    "output:{output_name}:{variant_label}_hls_writer:{run_id}"
                ));
                let variant_path = if is_abr {
                    destination.variant_path(i)
                } else {
                    match destination {
                        HlsDestination::Filesystem { directory } => directory.clone(),
                        HlsDestination::S3 { prefix, .. } => prefix.clone(),
                    }
                };
                ObswsHlsVariantRun {
                    video,
                    audio,
                    scaler_processor_id,
                    scaled_track_id,
                    writer_processor_id,
                    variant_path,
                }
            })
            .collect();
        let run = ObswsHlsRun {
            destination: destination.clone(),
            variant_runs,
        };
        // ランタイム状態を active にする
        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = true;
            output.runtime.started_at = Some(std::time::Instant::now());
            output.runtime.run = Some(OutputRun::Hls(run.clone()));
        }
        // filesystem の場合のみ出力ディレクトリを作成する
        if let HlsDestination::Filesystem { directory } = destination
            && let Err(e) = std::fs::create_dir_all(directory)
        {
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
            let error_comment = format!("Failed to create HLS output directory: {e}");
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
        if let HlsDestination::S3 {
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
                    // prefix スコープの expiration ルールを設定する
                    let rule_id = format!("hisui-hls-{}", prefix.replace('/', "-"));
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
        match start_hls_processors(pipeline_handle, &program_output, &run, &hls_settings).await {
            Ok(master_playlist_task) => {
                if let Some(output) = self.outputs.get_mut(output_name) {
                    output.runtime.background_task = master_playlist_task;
                }
            }
            Err(e) => {
                if let Some(output) = self.outputs.get_mut(output_name) {
                    output.runtime.active = false;
                    output.runtime.started_at = None;
                    output.runtime.run = None;
                }
                let _ = stop_processors_staged_hls(pipeline_handle, &run).await;
                let error_comment = format!("Failed to start HLS: {}", e.display());
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

    pub(crate) async fn handle_stop_hls(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        // run を取得してランタイム状態をリセット
        let run = self.outputs.get_mut(output_name).and_then(|o| {
            if let Some(handle) = o.runtime.background_task.take() {
                handle.abort();
            }
            let run = o.runtime.run.take();
            o.runtime.active = false;
            o.runtime.started_at = None;
            match run {
                Some(OutputRun::Hls(r)) => Some(r),
                _ => None,
            }
        });
        let Some(run) = run else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "HLS is not active",
                ),
            );
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_hls(pipeline_handle, &run).await
        {
            tracing::warn!("failed to stop HLS processors: {}", e.display());
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }
}

/// HLS 用プロセッサを起動する
/// 戻り値は ABR マスタープレイリスト書き出しタスクの JoinHandle（ABR でない場合は None）。
/// 呼び出し元は JoinHandle を保持し、出力停止時に abort() すること。
async fn start_hls_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    program_output: &ObswsProgramOutputContext,
    run: &crate::obsws::input_registry::ObswsHlsRun,
    hls_settings: &crate::obsws::input_registry::ObswsHlsSettings,
) -> crate::Result<Option<tokio::task::JoinHandle<()>>> {
    // HLS 用にキーフレーム間隔を設定する。
    // segment_duration に合わせたフレーム数を計算し、エンコーダーに事前通知する。
    let fps = program_output.frame_rate.numerator.get() as f64
        / program_output.frame_rate.denumerator.get() as f64;
    let keyframe_interval_frames = (hls_settings.segment_duration * fps).ceil() as u32;
    let keyframe_interval_frames = keyframe_interval_frames.max(1);
    let encode_params = crate::encoder::encode_config_with_keyframe_interval(
        keyframe_interval_frames,
        program_output.frame_rate,
    );

    let is_abr = run.is_abr();

    // ABR の場合、各 variant writer が SampleEntry から codec string を確定したら
    // oneshot channel 経由で通知を受け取り、全 variant の値がそろってからマスタープレイリストを書き出す。
    let mut codec_string_receivers = Vec::new();

    // バリアントごとにスケーラー、エンコーダー、ライターを起動する
    for (i, (variant, variant_run)) in hls_settings
        .variants
        .iter()
        .zip(run.variant_runs.iter())
        .enumerate()
    {
        // filesystem かつ ABR の場合はバリアントのサブディレクトリを作成する
        if is_abr
            && let crate::obsws::input_registry::HlsDestination::Filesystem { .. } = run.destination
        {
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
            crate::types::CodecName::H264,
            std::num::NonZeroUsize::new(variant.video_bitrate_bps)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            program_output.frame_rate,
            Some(encode_params.clone()),
            Some(variant_run.video.encoder_processor_id.clone()),
        )
        .await?;

        // オーディオエンコーダー（HLS 仕様で AAC 必須）
        crate::encoder::create_audio_processor(
            pipeline_handle,
            program_output.audio_track_id.clone(),
            variant_run.audio.encoded_track_id.clone(),
            crate::types::CodecName::Aac,
            std::num::NonZeroUsize::new(variant.audio_bitrate_bps)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            Some(variant_run.audio.encoder_processor_id.clone()),
        )
        .await?;

        // HLS ライター
        let storage_config = match &run.destination {
            crate::obsws::input_registry::HlsDestination::Filesystem { .. } => {
                crate::hls::writer::HlsStorageConfig::Filesystem {
                    output_directory: std::path::PathBuf::from(&variant_run.variant_path),
                }
            }
            crate::obsws::input_registry::HlsDestination::S3 {
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
                crate::hls::writer::HlsStorageConfig::S3 {
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

        crate::hls::writer::create_processor(
            pipeline_handle,
            crate::hls::writer::HlsWriterConfig {
                storage: storage_config,
                input_audio_track_id: variant_run.audio.encoded_track_id.clone(),
                input_video_track_id: variant_run.video.encoded_track_id.clone(),
                segment_duration: hls_settings.segment_duration,
                max_retained_segments: hls_settings.max_retained_segments,
                segment_format: hls_settings.segment_format,
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
            "HLS variant processor started"
        );
    }

    // ABR の場合は各 variant writer が SampleEntry から codec string を確定するのを待ち、
    // 全 variant の codec string が一致することを検証してからマスタープレイリストを書き出す。
    if is_abr {
        let master_variants: Vec<crate::hls::writer::MasterPlaylistVariant> = hls_settings
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
                crate::hls::writer::MasterPlaylistVariant {
                    bandwidth: variant.video_bitrate_bps as u64 + variant.audio_bitrate_bps as u64,
                    width,
                    height,
                    playlist_uri: format!("variant_{i}/playlist.m3u8"),
                }
            })
            .collect();

        let destination = run.destination.clone();

        let handle = tokio::spawn(async move {
            // 全 variant の codec string を収集する
            let mut codec_strings = Vec::with_capacity(codec_string_receivers.len());
            for (i, rx) in codec_string_receivers.into_iter().enumerate() {
                match rx.await {
                    Ok(cs) => codec_strings.push(cs),
                    Err(_) => {
                        tracing::warn!(
                            variant = i,
                            "HLS variant writer dropped codec string sender before resolving codecs"
                        );
                        return;
                    }
                }
            }

            // 全 variant の codec string が一致することを検証する
            let Some(first) = codec_strings.first() else {
                return;
            };
            for (i, cs) in codec_strings.iter().enumerate().skip(1) {
                if cs.video != first.video || cs.audio != first.audio {
                    tracing::error!(
                        variant = i,
                        expected_video = %first.video,
                        expected_audio = %first.audio,
                        actual_video = %cs.video,
                        actual_audio = %cs.audio,
                        "HLS ABR variant codec string mismatch: \
                         all variants must produce identical codec strings"
                    );
                    return;
                }
            }

            let master_content =
                crate::hls::writer::build_master_playlist_content(&master_variants, first);
            match &destination {
                crate::obsws::input_registry::HlsDestination::Filesystem { directory } => {
                    if let Err(e) = crate::hls::writer::write_master_playlist(
                        &std::path::PathBuf::from(directory),
                        &master_variants,
                        first,
                    ) {
                        tracing::error!(error = ?e, "failed to write HLS master playlist");
                    }
                }
                crate::obsws::input_registry::HlsDestination::S3 {
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
                    let s3_client = match build_s3_client(
                        region,
                        access_key_id,
                        secret_access_key,
                        session_token.as_deref(),
                        endpoint.as_deref(),
                        *use_path_style,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(error = ?e, "failed to create S3 client for HLS master playlist");
                            return;
                        }
                    };
                    let key = if prefix.is_empty() {
                        "playlist.m3u8".to_owned()
                    } else {
                        format!("{prefix}/playlist.m3u8")
                    };
                    let request = match s3_client
                        .client()
                        .put_object()
                        .bucket(bucket)
                        .key(&key)
                        .body(master_content.into_bytes())
                        .content_type("application/vnd.apple.mpegurl")
                        .build_request()
                    {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::error!(error = ?e, "failed to build S3 PutObject request for HLS master playlist");
                            return;
                        }
                    };
                    match s3_client.execute(&request).await {
                        Ok(response) if !response.is_success() => {
                            tracing::error!(
                                status = response.status_code,
                                "S3 PutObject failed for HLS master playlist {key}"
                            );
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "failed to upload HLS master playlist to S3");
                        }
                        _ => {}
                    }
                }
            }
        });
        Ok(Some(handle))
    } else {
        Ok(None)
    }
}

/// HLS 用プロセッサを段階的に停止する。
/// Program 出力は共有なので、variant 後段の processor のみを停止する。
async fn stop_processors_staged_hls(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &crate::obsws::input_registry::ObswsHlsRun,
) -> crate::Result<()> {
    // NOTE:
    // ライブ用途では StopOutput / ToggleOutput への応答遅延を避けることを優先し、
    // ここでは writer に finalize / cleanup を先行させる。
    // この経路は上流 encoder / scaler の完全 drain を保証しないため、
    // 停止直前の数フレームが最終セグメントに含まれない可能性がある。
    //
    // TODO:
    // 末尾欠損まで解消するには、writer を先に閉じるのではなく、
    // 上流から EOS 相当を伝播させる明示的な finish 経路が必要になる。
    // terminate_processor() は abort ベースで停止するだけなので、
    // encoder / scaler の残フレーム排出には使えない。
    let writer_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .map(|vr| vr.writer_processor_id.clone())
        .collect();
    for writer_id in &writer_ids {
        finish_hls_writer_rpc(pipeline_handle, writer_id).await;
    }
    wait_or_terminate(pipeline_handle, &writer_ids, Duration::from_secs(5)).await?;

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

    let scaler_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .filter_map(|vr| vr.scaler_processor_id.clone())
        .collect();
    if !scaler_ids.is_empty() {
        terminate_and_wait(pipeline_handle, &scaler_ids).await?;
    }

    // ABR の場合はマスタープレイリストとバリアントディレクトリを削除する
    if run.is_abr() {
        match &run.destination {
            crate::obsws::input_registry::HlsDestination::Filesystem { directory } => {
                let master_playlist_path =
                    std::path::PathBuf::from(directory).join("playlist.m3u8");
                if let Err(e) = std::fs::remove_file(&master_playlist_path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(
                        "failed to remove master playlist {}: {e}",
                        master_playlist_path.display()
                    );
                }
                // バリアントのサブディレクトリも削除する（ライターが中身を削除済みなので空のはず）
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
            crate::obsws::input_registry::HlsDestination::S3 {
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
                // マスタープレイリストを DeleteObject で削除する
                // バリアント「ディレクトリ」の削除は不要（S3 にディレクトリ概念なし）
                if let Ok(s3_client) = build_s3_client(
                    region,
                    access_key_id,
                    secret_access_key,
                    session_token.as_deref(),
                    endpoint.as_deref(),
                    *use_path_style,
                ) {
                    let key = if prefix.is_empty() {
                        "playlist.m3u8".to_owned()
                    } else {
                        format!("{prefix}/playlist.m3u8")
                    };
                    match s3_client
                        .client()
                        .delete_object()
                        .bucket(bucket)
                        .key(&key)
                        .build_request()
                    {
                        Ok(request) => match s3_client.execute(&request).await {
                            Ok(response) if !response.is_success() => {
                                tracing::warn!(
                                    "S3 DeleteObject failed for master playlist {key}: status={}",
                                    response.status_code
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "failed to delete S3 master playlist {key}: {}",
                                    e.display()
                                );
                            }
                            _ => {}
                        },
                        Err(e) => {
                            tracing::warn!(
                                "failed to build DeleteObject for master playlist {key}: {e}"
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// HLS writer に Finish RPC を送り、finalize / cleanup を促す。
/// これは writer 側の入力購読を閉じるためのもので、上流の完全 drain は保証しない。
/// 失敗時は terminate にフォールバックする。
async fn finish_hls_writer_rpc(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: &crate::ProcessorId,
) {
    const RETRY_TIMEOUT: Duration = Duration::from_millis(500);
    const RETRY_INTERVAL: Duration = Duration::from_millis(10);
    let deadline = tokio::time::Instant::now() + RETRY_TIMEOUT;

    loop {
        match pipeline_handle
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::hls::writer::HlsWriterRpcMessage,
            >>(processor_id)
            .await
        {
            Ok(sender) => {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                let _ = sender.send(crate::hls::writer::HlsWriterRpcMessage::Finish { reply_tx });
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
