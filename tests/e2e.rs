use std::time::Duration;

#[cfg(not(feature = "nvcodec"))]
use hisui::decoder::libvpx::LibvpxDecoder;
use hisui::{
    MediaPipeline, Message, ProcessorHandle, ProcessorId, ProcessorMetadata, TrackId,
    decoder::{VideoDecoder, VideoDecoderOptions, opus::OpusDecoder},
    sora::recording_mp4_reader::{Mp4AudioReader, Mp4VideoReader},
    types::{CodecName, EngineName},
    video::VideoFrame,
};

fn run_hisui_command(args: &[&str]) -> noargs::Result<std::process::Output> {
    let hisui_bin = env!("CARGO_BIN_EXE_hisui");
    let output = std::process::Command::new(hisui_bin)
        .args(["--verbose"])
        .args(args)
        .output()?;

    eprintln!("hisui args: --verbose {}", args.join(" "));
    eprintln!("hisui stdout:\n{}", String::from_utf8_lossy(&output.stdout));
    eprintln!("hisui stderr:\n{}", String::from_utf8_lossy(&output.stderr));

    if !output.status.success() {
        return Err("hisui command failed".into());
    }

    Ok(output)
}

#[test]
fn inspect_mp4_without_decode() -> noargs::Result<()> {
    let output = run_hisui_command(&["inspect", "testdata/archive-red-320x320-vp9.mp4"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = nojson::RawJson::parse(&stdout)
        .map_err(|e| format!("Failed to parse inspect output JSON: {e}"))?;

    let root = json.value();
    assert_eq!(
        root.to_member("format")?
            .required()?
            .to_unquoted_string_str()?,
        "mp4"
    );

    let mut video_sample_count = 0;
    let mut has_decoded_data_size = false;
    for sample in root.to_member("video_samples")?.required()?.to_array()? {
        video_sample_count += 1;
        if sample.to_member("decoded_data_size")?.optional().is_some() {
            has_decoded_data_size = true;
        }
    }

    assert!(video_sample_count > 0, "video sample must exist");
    assert!(
        !has_decoded_data_size,
        "decoded_data_size must not exist without --decode",
    );
    Ok(())
}

#[test]
fn inspect_fragmented_mp4_video_only() -> noargs::Result<()> {
    let plain_stdout = inspect_stdout("testdata/archive-red-320x320-h264.mp4")?;
    let fmp4_stdout = inspect_stdout("testdata/archive-red-320x320-h264-fragmented.mp4")?;

    assert_inspect_format_and_codec(&plain_stdout, "mp4", None, Some("H264"))?;
    assert_inspect_format_and_codec(&fmp4_stdout, "fmp4", None, Some("H264"))?;

    let plain = extract_inspect_comparable_samples(&plain_stdout)?;
    let fmp4 = extract_inspect_comparable_samples(&fmp4_stdout)?;

    let plain_video = plain
        .video
        .expect("testdata/archive-red-320x320-h264.mp4 に video_samples が存在すること");
    let fmp4_video = fmp4
        .video
        .expect("testdata/archive-red-320x320-h264-fragmented.mp4 に video_samples が存在すること");
    assert_eq!(
        fmp4_video, plain_video,
        "映像サンプルの data_size / keyframe / nalus が通常 MP4 と fMP4 で一致すること"
    );

    // testdata 再生成時に通常 MP4 と fMP4 が同時に同数で変化した場合の回帰を検出するため、
    // 絶対値もアンカーとして固定する。
    assert_eq!(fmp4_video.len(), 25, "映像サンプル数 25 (回帰検出アンカー)");

    Ok(())
}

#[test]
fn inspect_fragmented_mp4_audio_only() -> noargs::Result<()> {
    let plain_stdout = inspect_stdout("testdata/beep-aac-audio.mp4")?;
    let fmp4_stdout = inspect_stdout("testdata/beep-aac-audio-fragmented.mp4")?;

    assert_inspect_format_and_codec(&plain_stdout, "mp4", Some("AAC"), None)?;
    assert_inspect_format_and_codec(&fmp4_stdout, "fmp4", Some("AAC"), None)?;

    let plain = extract_inspect_comparable_samples(&plain_stdout)?;
    let fmp4 = extract_inspect_comparable_samples(&fmp4_stdout)?;

    let plain_audio = plain
        .audio
        .expect("testdata/beep-aac-audio.mp4 に audio_samples が存在すること");
    let fmp4_audio = fmp4
        .audio
        .expect("testdata/beep-aac-audio-fragmented.mp4 に audio_samples が存在すること");
    assert_eq!(
        fmp4_audio, plain_audio,
        "音声サンプルの data_size が通常 MP4 と fMP4 で一致すること"
    );

    assert_eq!(fmp4_audio.len(), 45, "音声サンプル数 45 (回帰検出アンカー)");

    Ok(())
}

#[test]
fn inspect_fragmented_mp4_audio_video() -> noargs::Result<()> {
    let plain_stdout = inspect_stdout("testdata/red-320x320-h264-aac.mp4")?;
    let fmp4_stdout = inspect_stdout("testdata/red-320x320-h264-aac-fragmented.mp4")?;

    assert_inspect_format_and_codec(&plain_stdout, "mp4", Some("AAC"), Some("H264"))?;
    assert_inspect_format_and_codec(&fmp4_stdout, "fmp4", Some("AAC"), Some("H264"))?;

    let plain = extract_inspect_comparable_samples(&plain_stdout)?;
    let fmp4 = extract_inspect_comparable_samples(&fmp4_stdout)?;

    let plain_audio = plain
        .audio
        .expect("testdata/red-320x320-h264-aac.mp4 に audio_samples が存在すること");
    let fmp4_audio = fmp4
        .audio
        .expect("testdata/red-320x320-h264-aac-fragmented.mp4 に audio_samples が存在すること");
    let plain_video = plain
        .video
        .expect("testdata/red-320x320-h264-aac.mp4 に video_samples が存在すること");
    let fmp4_video = fmp4
        .video
        .expect("testdata/red-320x320-h264-aac-fragmented.mp4 に video_samples が存在すること");

    assert_eq!(
        fmp4_audio, plain_audio,
        "音声サンプルの data_size が通常 MP4 と fMP4 で一致すること"
    );
    assert_eq!(
        fmp4_video, plain_video,
        "映像サンプルの data_size / keyframe / nalus が通常 MP4 と fMP4 で一致すること"
    );

    assert_eq!(fmp4_audio.len(), 45, "音声サンプル数 45 (回帰検出アンカー)");
    assert_eq!(fmp4_video.len(), 25, "映像サンプル数 25 (回帰検出アンカー)");

    Ok(())
}

#[test]
fn inspect_mp4_with_decode() -> noargs::Result<()> {
    let output = run_hisui_command(&[
        "inspect",
        "--decode",
        "testdata/archive-red-320x320-vp9.mp4",
    ])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = nojson::RawJson::parse(&stdout)
        .map_err(|e| format!("Failed to parse inspect output JSON: {e}"))?;

    let root = json.value();
    assert_eq!(
        root.to_member("format")?
            .required()?
            .to_unquoted_string_str()?,
        "mp4"
    );

    let mut video_sample_count = 0;
    let mut has_decoded_data_size = false;
    let mut has_resolution = false;
    for sample in root.to_member("video_samples")?.required()?.to_array()? {
        video_sample_count += 1;
        if sample.to_member("decoded_data_size")?.optional().is_some() {
            has_decoded_data_size = true;
        }
        let has_width = sample.to_member("width")?.optional().is_some();
        let has_height = sample.to_member("height")?.optional().is_some();
        if has_width && has_height {
            has_resolution = true;
        }
    }

    assert!(video_sample_count > 0, "video sample must exist");
    assert!(
        has_decoded_data_size,
        "decoded_data_size must exist with --decode",
    );
    assert!(has_resolution, "width and height must exist with --decode");
    Ok(())
}

#[test]
fn inspect_webm_without_decode() -> noargs::Result<()> {
    let output = run_hisui_command(&["inspect", "testdata/archive-black-silent.webm"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = nojson::RawJson::parse(&stdout)
        .map_err(|e| format!("Failed to parse inspect output JSON: {e}"))?;

    let root = json.value();
    assert_eq!(
        root.to_member("format")?
            .required()?
            .to_unquoted_string_str()?,
        "webm"
    );

    let mut video_sample_count = 0;
    let mut has_decoded_data_size = false;
    for sample in root.to_member("video_samples")?.required()?.to_array()? {
        video_sample_count += 1;
        if sample.to_member("decoded_data_size")?.optional().is_some() {
            has_decoded_data_size = true;
        }
    }

    assert!(video_sample_count > 0, "video sample must exist");
    assert!(
        !has_decoded_data_size,
        "decoded_data_size must not exist without --decode",
    );
    Ok(())
}

#[test]
fn inspect_webm_with_decode() -> noargs::Result<()> {
    let output = run_hisui_command(&["inspect", "--decode", "testdata/archive-black-silent.webm"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = nojson::RawJson::parse(&stdout)
        .map_err(|e| format!("Failed to parse inspect output JSON: {e}"))?;

    let root = json.value();
    assert_eq!(
        root.to_member("format")?
            .required()?
            .to_unquoted_string_str()?,
        "webm"
    );

    let mut video_sample_count = 0;
    let mut has_decoded_data_size = false;
    let mut has_resolution = false;
    for sample in root.to_member("video_samples")?.required()?.to_array()? {
        video_sample_count += 1;
        if sample.to_member("decoded_data_size")?.optional().is_some() {
            has_decoded_data_size = true;
        }
        let has_width = sample.to_member("width")?.optional().is_some();
        let has_height = sample.to_member("height")?.optional().is_some();
        if has_width && has_height {
            has_resolution = true;
        }
    }

    assert!(video_sample_count > 0, "video sample must exist");
    assert!(
        has_decoded_data_size,
        "decoded_data_size must exist with --decode",
    );
    assert!(has_resolution, "width and height must exist with --decode");
    Ok(())
}

/// ソースが空の場合
#[test]
fn empty_source() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new()?;

    // ビルド済みバイナリのパスを取得
    let hisui_bin = env!("CARGO_BIN_EXE_hisui");
    let output = std::process::Command::new(hisui_bin)
        .args([
            "compose",
            "--no-progress-bar",
            "--output-file",
            &out_file.path().display().to_string(),
            "testdata/e2e/empty_source/",
        ])
        .output()?;

    if !output.status.success() {
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Err("hisui command failed".into());
    }

    // 結果ファイルを確認（映像・音声トラックが存在しない）
    assert!(out_file.path().exists());
    assert_eq!(Mp4AudioReader::new(out_file.path())?.count(), 0);
    assert_eq!(Mp4VideoReader::new(out_file.path())?.count(), 0);

    Ok(())
}

// 共通のテスト関数
fn test_simple_single_source_common(
    test_data_dir: &str,
    expected_video_codec: CodecName,
    expected_video_engine: Option<EngineName>,
    expected_audio_codec: CodecName,
) -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new()?;
    let stats_file = tempfile::NamedTempFile::new()?;

    // ビルド済みバイナリのパスを取得
    let hisui_bin = env!("CARGO_BIN_EXE_hisui");
    let output = std::process::Command::new(hisui_bin)
        .args([
            "compose",
            "--no-progress-bar",
            "--layout-file",
            &format!("{test_data_dir}/layout.jsonc"),
            "--output-file",
            &out_file.path().display().to_string(),
            "--stats-file",
            &stats_file.path().display().to_string(),
            test_data_dir,
        ])
        .output()?;

    if !output.status.success() {
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Err("hisui command failed".into());
    }

    if let Some(expected_video_engine) = expected_video_engine {
        check_engine_in_stats(&stats_file, expected_video_engine)?;
    }

    if expected_audio_codec == CodecName::Aac {
        // 現状の Hisui は読み込み側での AAC には対応しておらず、AAC の場合はこれ以降の確認は行えないので、
        // ここで終了する
        return Ok(());
    }

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut audio_reader = Mp4AudioReader::new(out_file.path())?;
    let mut video_reader = Mp4VideoReader::new(out_file.path())?;

    // 後でデコードするために読み込み結果を覚えておく
    let audio_samples = audio_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;
    let video_samples = video_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;

    // 統計値を確認
    let audio_stats = audio_reader.stats();
    assert!(
        audio_stats.codec == Some(expected_audio_codec) || audio_stats.codec.is_none(),
        "unexpected audio codec: {:?}",
        audio_stats.codec
    );

    // 一秒分 + 一サンプル (25 ms)
    // => これは入力データのサンプル数と等しい
    assert_eq!(audio_stats.total_sample_count, 51);
    assert_eq!(
        audio_stats.total_track_duration,
        Duration::from_millis(1020)
    );

    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec, Some(expected_video_codec));
    assert_eq!(
        video_stats
            .resolutions
            .iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        [(320, 240)]
    );

    // 一秒分 (25 fps = 40 ms)
    assert_eq!(video_stats.total_sample_count, 25);
    assert_eq!(video_stats.total_track_duration, Duration::from_secs(1));

    // 音声をデコードをして中身を確認する
    let mut decoder = OpusDecoder::new()?;
    for data in audio_samples {
        let decoded = decoder.decode(&data)?;

        // 無音期間があるのは想定外
        assert!(!decoded.data.iter().all(|v| *v == 0));
    }

    // 映像をデコードをして中身を確認する
    let check_decoded_frame = |decoded: &VideoFrame| -> hisui::Result<()> {
        // 画像が赤一色かどうかの確認する
        let (y_plane, u_plane, v_plane) = decoded
            .as_yuv_planes()
            .ok_or_else(|| hisui::Error::new("value is missing"))?;
        y_plane
            .iter()
            .for_each(|x| assert!(matches!(x, 80..=83), "y={x}"));
        u_plane
            .iter()
            .for_each(|x| assert!(matches!(*x, 90 | 91), "u={x}"));
        v_plane
            .iter()
            .for_each(|x| assert!(matches!(x, 240 | 241), "v={x}"));
        Ok(())
    };

    let decoded_frames = decode_video_frames_with_pipeline(video_samples)?;
    for decoded in decoded_frames {
        check_decoded_frame(&decoded)?;
    }

    Ok(())
}

fn decode_video_frames_with_pipeline(
    video_samples: Vec<VideoFrame>,
) -> hisui::Result<Vec<VideoFrame>> {
    const INPUT_TRACK_ID: &str = "e2e_decoder_input";
    const OUTPUT_TRACK_ID: &str = "e2e_decoder_output";

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let pipeline = MediaPipeline::new(Default::default(), Default::default())?;
        let pipeline_handle = pipeline.handle();
        let mut pipeline_task = tokio::spawn(async move {
            pipeline.run().await;
        });

        let source_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("e2e_decoder_source"),
            ProcessorMetadata::new("e2e_decoder_source"),
        )
        .await?;
        let source_task = tokio::spawn(async move {
            run_video_source(source_handle, video_samples, TrackId::new(INPUT_TRACK_ID)).await
        });

        let decoder_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("e2e_video_decoder"),
            ProcessorMetadata::new("video_decoder"),
        )
        .await?;
        let decoder_task = tokio::spawn(async move {
            let decoder = VideoDecoder::new(VideoDecoderOptions::default(), decoder_handle.stats());
            decoder
                .run(
                    decoder_handle,
                    TrackId::new(INPUT_TRACK_ID),
                    TrackId::new(OUTPUT_TRACK_ID),
                )
                .await
        });

        let sink_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("e2e_decoder_sink"),
            ProcessorMetadata::new("e2e_decoder_sink"),
        )
        .await?;
        let sink_task = tokio::spawn(async move {
            collect_video_frames(sink_handle, TrackId::new(OUTPUT_TRACK_ID)).await
        });

        pipeline_handle
            .trigger_start()
            .await
            .map_err(|_| hisui::Error::new("failed to trigger start: pipeline has terminated"))?;

        match source_task.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(hisui::Error::new(format!("source task join failed: {e}"))),
        }
        match decoder_task.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(hisui::Error::new(format!("decoder task join failed: {e}"))),
        }
        let decoded_frames = match sink_task.await {
            Ok(Ok(frames)) => frames,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(hisui::Error::new(format!("sink task join failed: {e}"))),
        };

        drop(pipeline_handle);
        match tokio::time::timeout(Duration::from_secs(5), &mut pipeline_task).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(hisui::Error::new(format!(
                    "media pipeline task failed: {e}"
                )));
            }
            Err(_) => {
                pipeline_task.abort();
                let join_error = pipeline_task
                    .await
                    .expect_err("media pipeline task should be cancelled after timeout abort");
                assert!(
                    join_error.is_cancelled(),
                    "media pipeline task join failed after timeout abort: {join_error}"
                );
            }
        }

        Ok(decoded_frames)
    })
}

async fn register_processor(
    pipeline_handle: &hisui::MediaPipelineHandle,
    processor_id: ProcessorId,
    metadata: ProcessorMetadata,
) -> hisui::Result<ProcessorHandle> {
    pipeline_handle
        .register_processor(processor_id.clone(), metadata)
        .await
        .map_err(|e| match e {
            hisui::RegisterProcessorError::PipelineTerminated => {
                hisui::Error::new("failed to register processor: pipeline has terminated")
            }
            hisui::RegisterProcessorError::DuplicateProcessorId => hisui::Error::new(format!(
                "processor ID already exists: {}",
                processor_id.get()
            )),
        })
}

async fn run_video_source(
    handle: ProcessorHandle,
    frames: Vec<VideoFrame>,
    track_id: TrackId,
) -> hisui::Result<()> {
    let mut tx = handle.publish_track(track_id).await?;
    handle.notify_ready();
    handle.wait_subscribers_ready().await?;
    for frame in frames {
        if !tx.send_video(frame) {
            break;
        }
    }
    tx.send_eos();
    Ok(())
}

async fn collect_video_frames(
    handle: ProcessorHandle,
    track_id: TrackId,
) -> hisui::Result<Vec<VideoFrame>> {
    let mut rx = handle.subscribe_track(track_id);
    handle.notify_ready();
    let mut frames = Vec::new();
    loop {
        match rx.recv().await {
            Message::Media(sample) => {
                let frame = sample.expect_video()?;
                frames.push((*frame).clone());
            }
            Message::Eos => break,
            Message::Syn(_) => {}
        }
    }
    Ok(frames)
}

/// stats_file を確認して、デコーダーとエンコーダーの engine が期待通りかをチェックする
fn check_engine_in_stats(
    stats_file: &tempfile::NamedTempFile,
    expected_engine: EngineName,
) -> noargs::Result<()> {
    // stats_file を読み込んでパース
    let stats_json = std::fs::read_to_string(stats_file.path())
        .map_err(|e| format!("Failed to read stats file: {e}"))?;
    let stats = nojson::RawJson::parse(&stats_json)
        .map_err(|e| format!("Failed to parse stats JSON: {e}"))?;

    // processors 配列を取得
    let processors = stats
        .value()
        .to_member("processors")?
        .required()?
        .to_array()?;

    // デコーダーとエンコーダーの engine をチェック
    let mut found_decoder = false;
    let mut found_encoder = false;

    for processor in processors {
        let processor_type = processor
            .to_member("type")?
            .required()?
            .to_unquoted_string_str()?;

        match processor_type.as_ref() {
            "video_decoder" => {
                if let Some(engine_value) = processor.to_member("engine")?.optional()
                    && let Ok(engine_str) = engine_value.to_unquoted_string_str()
                {
                    assert_eq!(
                        engine_str.as_ref(),
                        expected_engine.as_str(),
                        "video decoder engine mismatch"
                    );
                    found_decoder = true;
                }
            }
            "video_encoder" => {
                if let Some(engine_value) = processor.to_member("engine")?.optional() {
                    let engine_str = engine_value
                        .to_unquoted_string_str()
                        .map_err(|e| format!("engine is not a string: {e}"))?;
                    assert_eq!(
                        engine_str.as_ref(),
                        expected_engine.as_str(),
                        "video encoder engine mismatch"
                    );
                    found_encoder = true;
                }
            }
            _ => {}
        }
    }

    // デコーダーとエンコーダーが両方とも見つかったことを確認
    assert!(found_decoder, "video decoder not found in stats");
    assert!(found_encoder, "video encoder not found in stats");

    Ok(())
}

fn required_string_member(
    value: nojson::RawJsonValue<'_, '_>,
    key: &str,
) -> noargs::Result<String> {
    value
        .to_member(key)?
        .required()?
        .try_into()
        .map_err(|e| format!("member {key} must be string: {e}").into())
}

fn required_usize_member(value: nojson::RawJsonValue<'_, '_>, key: &str) -> noargs::Result<usize> {
    value
        .to_member(key)?
        .required()?
        .try_into()
        .map_err(|e| format!("member {key} must be integer: {e}").into())
}

fn required_u64_member(value: nojson::RawJsonValue<'_, '_>, key: &str) -> noargs::Result<u64> {
    value
        .to_member(key)?
        .required()?
        .try_into()
        .map_err(|e| format!("member {key} must be integer: {e}").into())
}

fn required_f64_member(value: nojson::RawJsonValue<'_, '_>, key: &str) -> noargs::Result<f64> {
    value
        .to_member(key)?
        .required()?
        .try_into()
        .map_err(|e| format!("member {key} must be number: {e}").into())
}

fn required_bool_member(value: nojson::RawJsonValue<'_, '_>, key: &str) -> noargs::Result<bool> {
    value
        .to_member(key)?
        .required()?
        .try_into()
        .map_err(|e| format!("member {key} must be boolean: {e}").into())
}

fn optional_string_member(
    value: nojson::RawJsonValue<'_, '_>,
    key: &str,
) -> noargs::Result<Option<String>> {
    value
        .to_member(key)?
        .try_into()
        .map_err(|e| format!("member {key} must be optional string: {e}").into())
}

fn optional_u64_member(
    value: nojson::RawJsonValue<'_, '_>,
    key: &str,
) -> noargs::Result<Option<u64>> {
    value
        .to_member(key)?
        .try_into()
        .map_err(|e| format!("member {key} must be optional integer: {e}").into())
}

fn optional_f64_member(
    value: nojson::RawJsonValue<'_, '_>,
    key: &str,
) -> noargs::Result<Option<f64>> {
    value
        .to_member(key)?
        .try_into()
        .map_err(|e| format!("member {key} must be optional number: {e}").into())
}

/// inspect を実行して標準出力を返す
fn inspect_stdout(path: &str) -> noargs::Result<String> {
    let output = run_hisui_command(&["inspect", path])?;
    String::from_utf8(output.stdout)
        .map_err(|e| format!("inspect stdout is not valid UTF-8: {e}").into())
}

/// inspect 出力の `format` と `audio_codec` / `video_codec` が期待どおりか確認する。
/// 各 codec は `None` を渡せば「キー自体が出力されないこと」を assert する。これにより
/// 「映像のみ」「音声のみ」テストで想定外のトラックが混入した場合も検出できる。
fn assert_inspect_format_and_codec(
    stdout: &str,
    expected_format: &str,
    expected_audio_codec: Option<&str>,
    expected_video_codec: Option<&str>,
) -> noargs::Result<()> {
    let json = nojson::RawJson::parse(stdout)
        .map_err(|e| format!("inspect 出力の JSON パースに失敗: {e}"))?;
    let root = json.value();
    assert_eq!(
        root.to_member("format")?
            .required()?
            .to_unquoted_string_str()?,
        expected_format,
        "format が期待値と一致すること"
    );
    match expected_audio_codec {
        Some(expected) => assert_eq!(
            root.to_member("audio_codec")?
                .required()?
                .to_unquoted_string_str()?,
            expected,
            "audio_codec が期待値と一致すること"
        ),
        None => assert!(
            root.to_member("audio_codec")?.optional().is_none(),
            "音声トラックが無いはずなのに audio_codec キーが含まれている"
        ),
    }
    match expected_video_codec {
        Some(expected) => assert_eq!(
            root.to_member("video_codec")?
                .required()?
                .to_unquoted_string_str()?,
            expected,
            "video_codec が期待値と一致すること"
        ),
        None => assert!(
            root.to_member("video_codec")?.optional().is_none(),
            "映像トラックが無いはずなのに video_codec キーが含まれている"
        ),
    }
    Ok(())
}

/// 通常 MP4 と fMP4 で実値が一致するはずの inspect 出力フィールドだけを抽出した
/// 比較用構造体。`audio` / `video` の `None` は対応するトラックが inspect 出力に
/// 存在しないことを表す（`Some(Vec::new())` は「キーは存在するが要素 0」）。
///
/// 比較から除外する項目:
/// - `timestamp_us` / `duration_us`: 「音声+映像」ペアの映像トラックで testdata 生成差によりずれるため
/// - 集計値の `video_duration_us` / `audio_duration_us`: 同上
///
/// 映像サンプルの `nalus` は H.264 出力時のみ inspect が出力する。よって本構造体は
/// H.264 testdata 専用であり、H.265 / VP9 / AV1 の testdata では
/// `extract_inspect_comparable_samples` がパースエラーになる。
#[derive(Debug, PartialEq, Eq)]
struct InspectComparableSamples {
    audio: Option<Vec<InspectComparableAudioSample>>,
    video: Option<Vec<InspectComparableVideoSample>>,
}

#[derive(Debug, PartialEq, Eq)]
struct InspectComparableAudioSample {
    data_size: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct InspectComparableVideoSample {
    data_size: u64,
    keyframe: bool,
    nalus: Vec<InspectComparableNalu>,
}

/// `nalu_type` は JSON 出力上のキー名 `type`（Rust の予約語回避でリネーム）。
/// `nri` は H.264 の `nal_ref_idc`。
#[derive(Debug, PartialEq, Eq)]
struct InspectComparableNalu {
    nalu_type: u64,
    nri: u64,
}

/// inspect の出力 JSON 文字列から `InspectComparableSamples` を抽出する。
/// 詳細は `InspectComparableSamples` の doc を参照。
fn extract_inspect_comparable_samples(stdout: &str) -> noargs::Result<InspectComparableSamples> {
    let json = nojson::RawJson::parse(stdout)
        .map_err(|e| format!("inspect 出力の JSON パースに失敗: {e}"))?;
    let root = json.value();

    let audio = if let Some(audio_samples) = root.to_member("audio_samples")?.optional() {
        let mut out = Vec::new();
        for sample in audio_samples.to_array()? {
            out.push(InspectComparableAudioSample {
                data_size: required_u64_member(sample, "data_size")?,
            });
        }
        Some(out)
    } else {
        None
    };

    let video = if let Some(video_samples) = root.to_member("video_samples")?.optional() {
        let mut out = Vec::new();
        for sample in video_samples.to_array()? {
            let mut nalus = Vec::new();
            for nalu in sample.to_member("nalus")?.required()?.to_array()? {
                nalus.push(InspectComparableNalu {
                    nalu_type: required_u64_member(nalu, "type")?,
                    nri: required_u64_member(nalu, "nri")?,
                });
            }
            out.push(InspectComparableVideoSample {
                data_size: required_u64_member(sample, "data_size")?,
                keyframe: required_bool_member(sample, "keyframe")?,
                nalus,
            });
        }
        Some(out)
    } else {
        None
    };

    Ok(InspectComparableSamples { audio, video })
}

#[test]
fn compose_stdout_summary_has_required_fields() -> noargs::Result<()> {
    let out_file = tempfile::NamedTempFile::new()?;
    let stats_file = tempfile::NamedTempFile::new()?;

    let output = run_hisui_command(&[
        "compose",
        "--no-progress-bar",
        "--layout-file",
        "testdata/e2e/simple_single_source_vp9/layout.jsonc",
        "--output-file",
        &out_file.path().display().to_string(),
        "--stats-file",
        &stats_file.path().display().to_string(),
        "testdata/e2e/simple_single_source_vp9/",
    ])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = nojson::RawJson::parse(&stdout)
        .map_err(|e| format!("Failed to parse compose output JSON: {e}"))?;
    let root = json.value();

    let _ = required_string_member(root, "input_root_dir")?;
    let _ = required_string_member(root, "output_file_path")?;
    let _ = required_string_member(root, "output_audio_codec")?;
    let _ = required_string_member(root, "output_video_codec")?;
    let width = required_usize_member(root, "output_video_width")?;
    let height = required_usize_member(root, "output_video_height")?;
    let elapsed = required_f64_member(root, "elapsed_seconds")?;

    assert!(width > 0, "output_video_width must be greater than 0");
    assert!(height > 0, "output_video_height must be greater than 0");
    assert!(elapsed >= 0.0, "elapsed_seconds must be non-negative");

    let _ = optional_string_member(root, "layout_file_path")?;
    let _ = optional_string_member(root, "stats_file_path")?;
    let _ = optional_string_member(root, "output_audio_encode_engine")?;
    let _ = optional_string_member(root, "output_video_encode_engine")?;
    let _ = optional_f64_member(root, "output_audio_duration_seconds")?;
    let _ = optional_f64_member(root, "output_video_duration_seconds")?;
    let _ = optional_u64_member(root, "output_audio_bitrate")?;
    let _ = optional_u64_member(root, "output_video_bitrate")?;
    let _ = optional_f64_member(root, "total_audio_decoder_processing_seconds")?;
    let _ = optional_f64_member(root, "total_video_decoder_processing_seconds")?;
    let _ = optional_f64_member(root, "total_audio_encoder_processing_seconds")?;
    let _ = optional_f64_member(root, "total_video_encoder_processing_seconds")?;
    let _ = optional_f64_member(root, "total_audio_mixer_processing_seconds")?;
    let _ = optional_f64_member(root, "total_video_mixer_processing_seconds")?;

    Ok(())
}

#[test]
fn compose_stats_file_has_required_top_level_and_processor_entries() -> noargs::Result<()> {
    let out_file = tempfile::NamedTempFile::new()?;
    let stats_file = tempfile::NamedTempFile::new()?;

    let _ = run_hisui_command(&[
        "compose",
        "--no-progress-bar",
        "--layout-file",
        "testdata/e2e/simple_single_source_vp9/layout.jsonc",
        "--output-file",
        &out_file.path().display().to_string(),
        "--stats-file",
        &stats_file.path().display().to_string(),
        "testdata/e2e/simple_single_source_vp9/",
    ])?;

    let stats_json = std::fs::read_to_string(stats_file.path())
        .map_err(|e| format!("Failed to read stats file: {e}"))?;
    let stats = nojson::RawJson::parse(&stats_json)
        .map_err(|e| format!("Failed to parse stats JSON: {e}"))?;
    let root = stats.value();

    let elapsed = required_f64_member(root, "elapsed_seconds")?;
    assert!(elapsed >= 0.0, "elapsed_seconds must be non-negative");
    let _ = required_bool_member(root, "error")?;
    let processors = root.to_member("processors")?.required()?.to_array()?;

    let mut found_mp4_writer = false;
    let mut found_video_encoder = false;
    let mut found_audio_encoder = false;

    for processor in processors {
        let processor_type = processor
            .to_member("type")?
            .required()?
            .to_unquoted_string_str()?;
        match processor_type.as_ref() {
            "mp4_writer" => found_mp4_writer = true,
            "video_encoder" => found_video_encoder = true,
            "audio_encoder" => found_audio_encoder = true,
            _ => {}
        }
    }

    assert!(found_mp4_writer, "mp4_writer processor not found in stats");
    assert!(
        found_video_encoder,
        "video_encoder processor not found in stats",
    );
    assert!(
        found_audio_encoder,
        "audio_encoder processor not found in stats",
    );

    Ok(())
}

#[test]
fn compose_empty_source_summary_omits_media_specific_fields() -> noargs::Result<()> {
    let out_file = tempfile::NamedTempFile::new()?;

    let output = run_hisui_command(&[
        "compose",
        "--no-progress-bar",
        "--output-file",
        &out_file.path().display().to_string(),
        "testdata/e2e/empty_source/",
    ])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = nojson::RawJson::parse(&stdout)
        .map_err(|e| format!("Failed to parse compose output JSON: {e}"))?;
    let root = json.value();

    let _ = required_string_member(root, "input_root_dir")?;
    let _ = required_string_member(root, "output_file_path")?;
    let width = required_usize_member(root, "output_video_width")?;
    let height = required_usize_member(root, "output_video_height")?;
    let elapsed = required_f64_member(root, "elapsed_seconds")?;
    assert!(width > 0, "output_video_width must be greater than 0");
    assert!(height > 0, "output_video_height must be greater than 0");
    assert!(elapsed >= 0.0, "elapsed_seconds must be non-negative");

    assert!(
        root.to_member("output_audio_codec")?.optional().is_none(),
        "output_audio_codec must not exist for empty source",
    );
    assert!(
        root.to_member("output_video_codec")?.optional().is_none(),
        "output_video_codec must not exist for empty source",
    );
    assert!(
        root.to_member("output_audio_duration_seconds")?
            .optional()
            .is_none(),
        "output_audio_duration_seconds must not exist for empty source",
    );
    assert!(
        root.to_member("output_video_duration_seconds")?
            .optional()
            .is_none(),
        "output_video_duration_seconds must not exist for empty source",
    );

    Ok(())
}

#[test]
fn vmaf_stdout_summary_has_required_fields() -> noargs::Result<()> {
    let reference_yuv = tempfile::NamedTempFile::new()?;
    let distorted_yuv = tempfile::NamedTempFile::new()?;

    let output = run_hisui_command(&[
        "vmaf",
        "--layout-file",
        "testdata/e2e/simple_single_source_vp9/layout.jsonc",
        "--frame-count",
        "10",
        "--reference-yuv-file",
        &reference_yuv.path().display().to_string(),
        "--distorted-yuv-file",
        &distorted_yuv.path().display().to_string(),
        "testdata/e2e/simple_single_source_vp9/",
    ])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = nojson::RawJson::parse(&stdout)
        .map_err(|e| format!("Failed to parse vmaf output JSON: {e}"))?;
    let root = json.value();

    let width = required_usize_member(root, "width")?;
    let height = required_usize_member(root, "height")?;
    let encoded_frame_count = required_usize_member(root, "encoded_frame_count")?;
    assert!(width > 0, "width must be greater than 0");
    assert!(height > 0, "height must be greater than 0");
    assert_eq!(
        encoded_frame_count, 10,
        "encoded_frame_count must match --frame-count"
    );

    // VMAF スコアは 0..=100 の範囲に収まり、min <= harmonic_mean <= mean <= max の順序を満たす
    let min = required_f64_member(root, "vmaf_min")?;
    let max = required_f64_member(root, "vmaf_max")?;
    let mean = required_f64_member(root, "vmaf_mean")?;
    let harmonic_mean = required_f64_member(root, "vmaf_harmonic_mean")?;
    for (name, score) in [
        ("vmaf_min", min),
        ("vmaf_max", max),
        ("vmaf_mean", mean),
        ("vmaf_harmonic_mean", harmonic_mean),
    ] {
        assert!(
            (0.0..=100.0).contains(&score),
            "{name} must be within 0..=100, but got {score}"
        );
    }
    assert!(min <= max, "vmaf_min must be <= vmaf_max");
    assert!(
        min <= harmonic_mean && harmonic_mean <= mean && mean <= max,
        "expected min <= harmonic_mean <= mean <= max, but got min={min} harmonic_mean={harmonic_mean} mean={mean} max={max}"
    );

    Ok(())
}

/// tune コマンドが 1 試行を実行し、試行履歴を永続化することの最小確認。
///
/// tune は内部で vmaf サブコマンドを呼ぶ (current_exe() で自分自身を spawn する)。
/// ここが通れば「tune -> vmaf -> 合成 -> エンコード -> VMAF 評価 -> 履歴永続化」の配線が
/// 一通り生きていることになる (vmaf コマンドの変更などによるデグレ検知が目的)。
#[test]
fn tune_runs_one_trial_and_records_history() -> noargs::Result<()> {
    let work_dir = tempfile::tempdir()?;

    // 値が null のメンバーだけが探索対象になる。ここでは cpu_used 1 個だけを可変にした
    // 最小レイアウトを使う。ソース glob はフィクスチャに合わせて "*.json"。
    let layout_path = work_dir.path().join("tune-layout.jsonc");
    std::fs::write(
        &layout_path,
        r#"{
  "video_layout": {
    "main": {
      "cell_width": 320,
      "cell_height": 240,
      "max_columns": 3,
      "video_sources": ["*.json"]
    }
  },
  "video_codec": "VP9",
  "video_encode_engines": ["libvpx"],
  "video_decode_engines": ["libvpx"],
  "libvpx_vp9_encode_params": { "cpu_used": null }
}"#,
    )?;

    // cpu_used を速い側に限定して 1 試行を短時間で終わらせる。
    let search_space_path = work_dir.path().join("search-space.jsonc");
    std::fs::write(
        &search_space_path,
        r#"{ "libvpx_vp9_encode_params.cpu_used": { "min": 7, "max": 9 } }"#,
    )?;

    let tune_working_dir = work_dir.path().join("tune");
    run_hisui_command(&[
        "tune",
        "--layout-file",
        &layout_path.display().to_string(),
        "--search-space-file",
        &search_space_path.display().to_string(),
        "--trial-count",
        "1",
        "--frame-count",
        "5",
        "--tune-working-dir",
        &tune_working_dir.display().to_string(),
        "testdata/e2e/simple_single_source_vp9/",
    ])?;

    // 試行履歴 (JSON Lines) に成功レコードが 1 件書かれていること。
    let jsonl_path = tune_working_dir.join("hisui-tune.jsonl");
    let content = std::fs::read_to_string(&jsonl_path)
        .map_err(|e| format!("試行履歴 {} を読めること: {e}", jsonl_path.display()))?;
    let lines: Vec<&str> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 1, "1 試行ぶんの履歴が書かれること");
    assert!(
        lines[0].contains(r#""state":"complete""#),
        "成功 (complete) レコードであること: {}",
        lines[0]
    );
    assert!(
        lines[0].contains("vmaf_mean"),
        "vmaf_mean が記録されていること: {}",
        lines[0]
    );

    // 多重起動防止のロックは正常終了で解放され、残らないこと。
    assert!(
        !tune_working_dir.join("hisui-tune.lock").exists(),
        "ロックファイルが正常終了で残らないこと"
    );

    Ok(())
}

/// 単一のソースをそのまま変換する場合
/// - 入力:
///   - 映像:
///     - VP9
///     - 30 fps
///     - 320x240
///     - 赤一色
///   - 音声:
///     - OPUS
///     - ホワイトノイズ
/// - 出力:
///   - VP9, OPUS, 25 fps, 320x240
#[test]
fn simple_single_source_vp9() -> noargs::Result<()> {
    test_simple_single_source_common(
        "testdata/e2e/simple_single_source_vp9/",
        CodecName::Vp9,
        Some(EngineName::Libvpx),
        CodecName::Opus,
    )
}

/// simple_single_source_vp9 とほぼ同様だけど nvcodec は VP9 エンコードをサポートしていないので、
/// 出力では H.264 を使っている
#[test]
#[cfg(feature = "nvcodec")]
fn simple_single_source_vp9_nvcodec() -> noargs::Result<()> {
    test_simple_single_source_common(
        "testdata/e2e/simple_single_source_vp9_nvcodec/",
        CodecName::H264,
        Some(EngineName::Nvcodec),
        CodecName::Opus,
    )
}

/// simple_single_source_vp9 とほぼ同様だけどエンコードに AAC を指定している
#[test]
fn simple_single_source_aac_encode() -> noargs::Result<()> {
    if !cfg!(target_os = "macos")
        && (!cfg!(feature = "fdk-aac") || std::env::var("HISUI_FDK_AAC_PATH").is_err())
    {
        eprintln!("skipping: AAC test requires macOS or fdk-aac feature with HISUI_FDK_AAC_PATH");
        return Ok(());
    }
    test_simple_single_source_common(
        "testdata/e2e/simple_single_source_aac_encode/",
        CodecName::Av1,
        None,
        CodecName::Aac,
    )
}

/// 単一のソースをそのまま変換する場合 (H.265版)
/// - 入力:
///   - 映像:
///     - H.265
///     - 30 fps
///     - 320x240
///     - 赤一色
///   - 音声:
///     - OPUS
///     - ホワイトノイズ
/// - 出力:
///   - VP9, OPUS, 25 fps, 320x240
#[test]
#[cfg(any(feature = "nvcodec", target_os = "macos"))]
fn simple_single_source_h265() -> noargs::Result<()> {
    test_simple_single_source_common(
        "testdata/e2e/simple_single_source_h265/",
        CodecName::H265,
        None,
        CodecName::Opus,
    )
}

/// 単一のソースをそのまま変換する場合 (H.264 版)
/// - 入力:
///   - 映像:
///     - H.264
///     - 30 fps
///     - 320x240
///     - 赤一色
///   - 音声:
///     - OPUS
///     - ホワイトノイズ
/// - 出力:
///   - VP9, OPUS, 25 fps, 320x240
#[test]
#[cfg(any(feature = "nvcodec", target_os = "macos"))]
fn simple_single_source_h264() -> noargs::Result<()> {
    test_simple_single_source_common(
        "testdata/e2e/simple_single_source_h264/",
        CodecName::H264,
        None,
        CodecName::Opus,
    )
}

/// 単一のソースをそのまま変換する場合 (AV1 版)
/// - 入力:
///   - 映像:
///     - AV1
///     - 30 fps
///     - 320x240
///     - 赤一色
///   - 音声:
///     - OPUS
///     - ホワイトノイズ
/// - 出力:
///   - VP9, OPUS, 25 fps, 320x240
#[test]
fn simple_single_source_av1() -> noargs::Result<()> {
    test_simple_single_source_common(
        "testdata/e2e/simple_single_source_av1/",
        CodecName::Av1,
        None,
        CodecName::Opus,
    )
}

/// 単一のソースをそのまま変換する場合（奇数解像度版）
/// - 入力:
///   - 映像:
///     - VP9
///     - 30 fps
///     - 319x239
///     - 赤一色
///   - 音声:
///     - OPUS
///     - ホワイトノイズ
/// - 出力:
///   - VP9, OPUS, 25 fps, 319x239
// nvcodec feature が有効な場合、デコーダ選択で NVDEC が優先されるが、
// テスト用の小さい解像度の映像（16x16 等）や奇数解像度は NVDEC の制約で処理できないためスキップする
#[test]
#[cfg(not(feature = "nvcodec"))]
fn odd_resolution_single_source() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new()?;

    // ビルド済みバイナリのパスを取得
    let hisui_bin = env!("CARGO_BIN_EXE_hisui");
    let output = std::process::Command::new(hisui_bin)
        .args([
            "compose",
            "--no-progress-bar",
            "--output-file",
            &out_file.path().display().to_string(),
            "testdata/e2e/odd_resolution_single_source/",
        ])
        .output()?;

    if !output.status.success() {
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Err("hisui command failed".into());
    }

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut audio_reader = Mp4AudioReader::new(out_file.path())?;
    let mut video_reader = Mp4VideoReader::new(out_file.path())?;

    // 後でデコードするために読み込み結果を覚えておく
    let audio_samples = audio_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;
    let video_samples = video_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;

    // 統計値を確認
    let audio_stats = audio_reader.stats();
    assert!(
        audio_stats.codec == Some(CodecName::Opus) || audio_stats.codec.is_none(),
        "unexpected audio codec: {:?}",
        audio_stats.codec
    );

    // 一秒分 + 一サンプル (25 ms)
    // => これは入力データのサンプル数と等しい
    assert_eq!(audio_stats.total_sample_count, 51);
    assert_eq!(
        audio_stats.total_track_duration,
        Duration::from_millis(1020)
    );

    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec, Some(CodecName::Vp9));
    assert_eq!(
        video_stats
            .resolutions
            .iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        // 合成後は偶数解像度になる
        //（下と右に枠線が入る）
        [(320, 240)]
    );

    // 一秒分 (25 fps = 40 ms)
    assert_eq!(video_stats.total_sample_count, 25);
    assert_eq!(video_stats.total_track_duration, Duration::from_secs(1));

    // 音声をデコードをして中身を確認する
    let mut decoder = OpusDecoder::new()?;
    for data in audio_samples {
        let decoded = decoder.decode(&data)?;

        // 無音期間があるのは想定外
        assert!(!decoded.data.iter().all(|v| *v == 0));
    }

    // 映像をデコードをして中身を確認する
    // (デコーダーは出力フレームを sink 経由で内部 channel へ流すため、
    //  外側から rx で受け取る形に変わった点に注意)
    let check_decoded_frames = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        hisui::Result<hisui::VideoFrame>,
    >|
     -> hisui::Result<()> {
        while let Ok(result) = rx.try_recv() {
            let decoded = result?;
            // 画像が赤一色かどうかの確認する（ただし、右と下の枠線は黒色になる）
            let (y_plane, u_plane, v_plane) = decoded
                .as_yuv_planes()
                .ok_or_else(|| hisui::Error::new("value is missing"))?;

            y_plane.iter().enumerate().for_each(|(i, &x)| {
                let col = i % 320;
                let row = i / 320;
                if col >= 318 || row >= 238 {
                    assert!(matches!(x, 0..=8), "Expected black Y value, got y={x}",);
                } else {
                    assert!(matches!(x, 75..=84), "Expected red Y value, got y={x}",);
                }
            });

            u_plane.iter().enumerate().for_each(|(i, &x)| {
                let col = (i % 160) * 2;
                let row = (i / 160) * 2;
                if col >= 318 || row >= 238 {
                    assert!(matches!(x, 122..=131), "Expected black U value, got u={x}");
                } else {
                    assert!(matches!(x, 86..=95), "Expected red U value, got u={x}");
                }
            });

            v_plane.iter().enumerate().for_each(|(i, &x)| {
                let col = (i % 160) * 2;
                let row = (i / 160) * 2;
                if col >= 318 || row >= 238 {
                    assert!(matches!(x, 122..=131), "Expected black V value, got v={x}");
                } else {
                    assert!(matches!(x, 235..=244), "Expected red V value, got v={x}");
                }
            });
        }
        Ok(())
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut stats = hisui::stats::Stats::new();
    let test_metric = stats.counter("test_total_output");
    let sink = hisui::decoder::OutputSink::new(tx, test_metric);
    let mut decoder = LibvpxDecoder::new_vp9(sink)?;
    for frame in video_samples {
        decoder.decode(&frame)?;
        check_decoded_frames(&mut rx)?;
    }
    decoder.finish()?;
    check_decoded_frames(&mut rx)?;

    Ok(())
}

/// 複数のソースをレイアウト指定なしで変換する場合
// nvcodec feature が有効な場合、デコーダ選択で NVDEC が優先されるが、
// テスト用の小さい解像度の映像（16x16 等）は NVDEC の制約で処理できないためスキップする
#[test]
#[cfg(not(feature = "nvcodec"))]
fn simple_multi_sources() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new()?;

    // ビルド済みバイナリのパスを取得
    let hisui_bin = env!("CARGO_BIN_EXE_hisui");
    let output = std::process::Command::new(hisui_bin)
        .args([
            "compose",
            "--no-progress-bar",
            "--output-file",
            &out_file.path().display().to_string(),
            "testdata/e2e/simple_multi_sources/",
        ])
        .output()?;

    if !output.status.success() {
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Err("hisui command failed".into());
    }

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut audio_reader = Mp4AudioReader::new(out_file.path())?;
    let mut video_reader = Mp4VideoReader::new(out_file.path())?;

    // [NOTE]
    // レイアウトファイル未指定だと映像の解像度が大きめになって
    // テスト内でデコード結果を確認するのが少し面倒なので、このテストでは省略している
    // （統計値を取得するためにイテレーターを最後まで実行する必要はある）
    let _audio_samples = audio_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;
    let _video_samples = video_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;

    // 統計値を確認
    let audio_stats = audio_reader.stats();
    assert!(
        audio_stats.codec == Some(CodecName::Opus) || audio_stats.codec.is_none(),
        "unexpected audio codec: {:?}",
        audio_stats.codec
    );

    // 一秒分 + 一サンプル (25 ms)
    // => これは入力データのサンプル数と等しい
    assert_eq!(audio_stats.total_sample_count, 51);
    assert_eq!(
        audio_stats.total_track_duration,
        Duration::from_millis(1020)
    );

    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec, Some(CodecName::Vp9));

    // レイアウトファイル未指定の場合には、一つのセルの解像度は 320x240 で、
    // 今回はソースが三つなのでグリッドは 3x1 となり、
    // 以下の解像度になる
    assert_eq!(
        video_stats
            .resolutions
            .iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        // NOTE: +4 は枠線用
        [(320 * 3 + 4, 240)]
    );

    // 一秒分 (25 fps = 40 ms)
    assert_eq!(video_stats.total_sample_count, 25);
    assert_eq!(video_stats.total_track_duration, Duration::from_secs(1));

    Ok(())
}

/// 分割録画の変換テスト
/// - 同一接続から時系列で分割された複数のソースファイル（R -> G -> B）を一つにまとめる
/// - 各ソースファイルは16x16の解像度
/// - レイアウトファイルで縦に並べて配置
// nvcodec feature が有効な場合、デコーダ選択で NVDEC が優先されるが、
// テスト用の小さい解像度の映像（16x16 等）は NVDEC の制約で処理できないためスキップする
#[test]
#[cfg(not(feature = "nvcodec"))]
fn simple_split_archive() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new()?;

    // ビルド済みバイナリのパスを取得
    let hisui_bin = env!("CARGO_BIN_EXE_hisui");
    let output = std::process::Command::new(hisui_bin)
        .args([
            "compose",
            "--no-progress-bar",
            "--layout-file",
            "testdata/e2e/simple_split_archive/layout.jsonc",
            "--output-file",
            &out_file.path().display().to_string(),
            "testdata/e2e/simple_split_archive/",
        ])
        .output()?;

    if !output.status.success() {
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Err("hisui command failed".into());
    }

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut audio_reader = Mp4AudioReader::new(out_file.path())?;
    let mut video_reader = Mp4VideoReader::new(out_file.path())?;

    // 後でデコードするために読み込み結果を覚えておく
    let audio_samples = audio_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;
    let video_samples = video_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;

    // 統計値を確認
    let audio_stats = audio_reader.stats();
    assert!(
        audio_stats.codec == Some(CodecName::Opus) || audio_stats.codec.is_none(),
        "unexpected audio codec: {:?}",
        audio_stats.codec
    );

    // 分割ファイルが3つ（各1秒）なので合計3秒分 + 3サンプル (25 ms * 3)
    assert_eq!(audio_stats.total_sample_count, 153); // 51 * 3
    assert_eq!(
        audio_stats.total_track_duration,
        Duration::from_millis(3060) // 1020 * 3
    );

    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec, Some(CodecName::Vp9));
    assert_eq!(
        video_stats
            .resolutions
            .iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        [(16, 16)] // 単一ソース（分割された部分）なので16x16
    );

    // 3秒分 (25 fps = 40 ms * 75フレーム)
    assert_eq!(video_stats.total_sample_count, 75); // 25 * 3
    assert_eq!(video_stats.total_track_duration, Duration::from_secs(3));

    // 音声をデコードをして中身を確認する
    let mut decoder = OpusDecoder::new()?;
    for data in audio_samples {
        let decoded = decoder.decode(&data)?;

        // 無音期間があるのは想定外
        assert!(!decoded.data.iter().all(|v| *v == 0));
    }

    // 映像をデコードをして中身を確認する
    // 時系列順に R -> G -> B の色変化を確認
    // (デコーダーは出力フレームを sink 経由で内部 channel へ流すため、
    //  外側から rx で受け取る形に変わった点に注意)
    let check_decoded_frames =
        |rx: &mut tokio::sync::mpsc::UnboundedReceiver<hisui::Result<hisui::VideoFrame>>,
         frame_index: &mut usize|
         -> hisui::Result<()> {
            while let Ok(result) = rx.try_recv() {
                let decoded = result?;
                // Y成分だけを確認して色の変化を検証
                let (y_plane, _u_plane, v_plane) = decoded
                    .as_yuv_planes()
                    .ok_or_else(|| hisui::Error::new("value is missing"))?;

                // フレーム番号に基づいて期待される色を判定
                // 0-24: 赤, 25-49: 緑, 50-74: 青
                //
                // なお赤と緑は同じような Y 値でエンコードされているので、 Vの値も考慮している

                if *frame_index < 25 {
                    // 赤色の期間
                    (y_plane.iter().zip(v_plane.iter())).for_each(|(&y, &v)| {
                        assert!(
                            matches!(y, 80..=82) && matches!(v, 240),
                            "Expected red Y / V value, got y={y} / v={v} at frame {}",
                            *frame_index
                        );
                    });
                } else if *frame_index < 50 {
                    // 緑色の期間
                    (y_plane.iter().zip(v_plane.iter())).for_each(|(&y, &v)| {
                        assert!(
                            matches!(y, 80..=82) && matches!(v, 81),
                            "Expected green Y / V value, got y={y} / v={v} at frame {}",
                            *frame_index
                        );
                    });
                } else if *frame_index < 75 {
                    // 青色の期間
                    y_plane.iter().for_each(|&y| {
                        assert!(
                            matches!(y, 40..=42),
                            "Expected blue Y value, got y={y} at frame {}",
                            *frame_index
                        );
                    });
                }
                *frame_index += 1;
            }
            Ok(())
        };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut stats = hisui::stats::Stats::new();
    let test_metric = stats.counter("test_total_output");
    let sink = hisui::decoder::OutputSink::new(tx, test_metric);
    let mut decoder = LibvpxDecoder::new_vp9(sink)?;
    let mut frame_index = 0;
    for frame in video_samples {
        decoder.decode(&frame)?;
        check_decoded_frames(&mut rx, &mut frame_index)?;
    }
    decoder.finish()?;
    check_decoded_frames(&mut rx, &mut frame_index)?;

    // 全フレームが処理されたことを確認
    assert_eq!(frame_index, 75);

    Ok(())
}

/// 複数のソースをレイアウト指定で、縦に並べて変換する場合
// nvcodec feature が有効な場合、デコーダ選択で NVDEC が優先されるが、
// テスト用の小さい解像度の映像（16x16 等）は NVDEC の制約で処理できないためスキップする
#[test]
#[cfg(not(feature = "nvcodec"))]
fn multi_sources_single_column() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new()?;

    // ビルド済みバイナリのパスを取得
    let hisui_bin = env!("CARGO_BIN_EXE_hisui");
    let output = std::process::Command::new(hisui_bin)
        .args([
            "compose",
            "--no-progress-bar",
            "--layout-file",
            "testdata/e2e/multi_sources_single_column/layout.json",
            "--output-file",
            &out_file.path().display().to_string(),
            "testdata/e2e/multi_sources_single_column/",
        ])
        .output()?;

    if !output.status.success() {
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Err("hisui command failed".into());
    }

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut audio_reader = Mp4AudioReader::new(out_file.path())?;
    let mut video_reader = Mp4VideoReader::new(out_file.path())?;

    // 後でデコードするために読み込み結果を覚えておく
    let audio_samples = audio_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;
    let video_samples = video_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;

    // 統計値を確認
    let audio_stats = audio_reader.stats();
    assert!(
        audio_stats.codec == Some(CodecName::Opus) || audio_stats.codec.is_none(),
        "unexpected audio codec: {:?}",
        audio_stats.codec
    );

    // 一秒分 + 一サンプル (25 ms)
    // => これは入力データのサンプル数と等しい
    assert_eq!(audio_stats.total_sample_count, 51);
    assert_eq!(
        audio_stats.total_track_duration,
        Duration::from_millis(1020)
    );

    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec, Some(CodecName::Vp9));
    assert_eq!(
        video_stats
            .resolutions
            .iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        [(16, 52)]
    );

    // 一秒分 (25 fps = 40 ms)
    assert_eq!(video_stats.total_sample_count, 25);
    assert_eq!(video_stats.total_track_duration, Duration::from_secs(1));

    // 音声をデコードをして中身を確認する
    let mut decoder = OpusDecoder::new()?;
    for data in audio_samples {
        let decoded = decoder.decode(&data)?;

        // 無音期間があるのは想定外
        assert!(!decoded.data.iter().all(|v| *v == 0));
    }

    // 映像をデコードをして中身を確認する
    // (デコーダーは出力フレームを sink 経由で内部 channel へ流すため、
    //  外側から rx で受け取る形に変わった点に注意)
    let check_decoded_frames = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        hisui::Result<hisui::VideoFrame>,
    >|
     -> hisui::Result<()> {
        while let Ok(result) = rx.try_recv() {
            let decoded = result?;
            // 完全なチェックは面倒なので Y 成分だけを確認する
            let (y_plane, _u_plane, _v_plane) = decoded
                .as_yuv_planes()
                .ok_or_else(|| hisui::Error::new("value is missing"))?;

            let width = 16;
            for (i, y) in y_plane.iter().copied().enumerate() {
                if i / width < 16 {
                    // 最初の 16 行は青
                    assert!(matches!(y, 40..=43), "y={y}");
                } else if i / width < 16 + 2 {
                    // 次の 2 行は黒色（枠線）
                    assert!(matches!(y, 0..=8), "y={y}");
                } else if i / width < 16 + 2 + 16 {
                    // 次の 16 行は緑
                    assert!(matches!(y, 182..=192), "y={y}");
                } else if i / width < 16 + 2 + 16 + 2 {
                    // 次の 2 行は黒色（枠線）
                    assert!(matches!(y, 0..=8), "y={y}");
                } else if i / width < 16 + 2 + 16 + 2 + 16 {
                    // 最後の 16 行は赤
                    assert!(matches!(y, 75..=86), "y={y}");
                } else {
                    unreachable!()
                }
            }
        }
        Ok(())
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut stats = hisui::stats::Stats::new();
    let test_metric = stats.counter("test_total_output");
    let sink = hisui::decoder::OutputSink::new(tx, test_metric);
    let mut decoder = LibvpxDecoder::new_vp9(sink)?;
    for frame in video_samples {
        decoder.decode(&frame)?;
        check_decoded_frames(&mut rx)?;
    }
    decoder.finish()?;
    check_decoded_frames(&mut rx)?;

    Ok(())
}

/// リージョンが二つあるレイアウトのテスト
/// - 全体の解像度は 16x34
/// - 一つ目のリージョンには縦並びの二つのセルがある（青と緑）
/// - 二つ目のリージョンは中央に一つのセルがある（赤） => 後ろに別のリージョンがあるので外枠がつく
/// - 音声ソースはなし
// nvcodec feature が有効な場合、デコーダ選択で NVDEC が優先されるが、
// テスト用の小さい解像度の映像（16x16 等）は NVDEC の制約で処理できないためスキップする
#[test]
#[cfg(not(feature = "nvcodec"))]
fn two_regions() -> noargs::Result<()> {
    // 変換を実行
    let out_file = tempfile::NamedTempFile::new()?;

    // ビルド済みバイナリのパスを取得
    let hisui_bin = env!("CARGO_BIN_EXE_hisui");
    let output = std::process::Command::new(hisui_bin)
        .args([
            "compose",
            "--no-progress-bar",
            "--layout-file",
            "testdata/e2e/two_regions/layout.json",
            "--output-file",
            &out_file.path().display().to_string(),
            "testdata/e2e/two_regions/",
        ])
        .output()?;

    if !output.status.success() {
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Err("hisui command failed".into());
    }

    // 変換結果ファイルを読み込む
    assert!(out_file.path().exists());
    let mut video_reader = Mp4VideoReader::new(out_file.path())?;

    // 音声はなし
    assert_eq!(Mp4AudioReader::new(out_file.path())?.count(), 0);

    // 後でデコードするために読み込み結果を覚えておく
    let video_samples = video_reader.by_ref().collect::<hisui::Result<Vec<_>>>()?;

    // 統計値を確認
    let video_stats = video_reader.stats();
    assert_eq!(video_stats.codec, Some(CodecName::Vp9));
    assert_eq!(
        video_stats
            .resolutions
            .iter()
            .map(|r| (r.width, r.height))
            .collect::<Vec<_>>(),
        [(16, 34)]
    );

    // 一秒分 (25 fps = 40 ms)
    assert_eq!(video_stats.total_sample_count, 25);
    assert_eq!(video_stats.total_track_duration, Duration::from_secs(1));

    // 映像をデコードをして中身を確認する
    // (デコーダーは出力フレームを sink 経由で内部 channel へ流すため、
    //  外側から rx で受け取る形に変わった点に注意)
    let check_decoded_frames = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        hisui::Result<hisui::VideoFrame>,
    >|
     -> hisui::Result<()> {
        while let Ok(result) = rx.try_recv() {
            let decoded = result?;
            // 完全なチェックは面倒なので Y 成分だけを確認する
            let (y_plane, _u_plane, _v_plane) = decoded
                .as_yuv_planes()
                .ok_or_else(|| hisui::Error::new("value is missing"))?;

            let width = 16;
            for (i, y) in y_plane.iter().copied().enumerate() {
                if i / width < 8 {
                    // 最初の 8 行は青
                    assert!(matches!(y, 40..=44), "y={y}");
                } else if i / width < 8 + 2 {
                    // 次の 2 行は黒色（枠線）
                    assert!(matches!(y, 0..=8), "y={y}");
                } else if i / width < 8 + 2 + 16 {
                    // 次の 16 行は赤
                    assert!(matches!(y, 75..=84), "y={y}");
                } else if i / width < 8 + 2 + 16 + 2 {
                    // 次の 2 行は黒色（枠線）
                    assert!(matches!(y, 0..=8), "y={y}");
                } else if i / width < 8 + 2 + 16 + 2 + 6 {
                    // 最後の 6 行は緑
                    assert!(matches!(y, 182..=192), "y={y}");
                } else {
                    unreachable!()
                }
            }
        }
        Ok(())
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut stats = hisui::stats::Stats::new();
    let test_metric = stats.counter("test_total_output");
    let sink = hisui::decoder::OutputSink::new(tx, test_metric);
    let mut decoder = LibvpxDecoder::new_vp9(sink)?;
    for frame in video_samples {
        decoder.decode(&frame)?;
        check_decoded_frames(&mut rx)?;
    }
    decoder.finish()?;
    check_decoded_frames(&mut rx)?;

    Ok(())
}
