use hisui::{
    MediaPipeline, Message, ProcessorHandle, ProcessorId, ProcessorMetadata, TrackId,
    audio::AudioFrame,
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    video::VideoFrame,
};
use shiguredo_openh264::Openh264Library;

const VIDEO_INPUT_TRACK_ID: &str = "decoder_test_video_input";
const VIDEO_OUTPUT_TRACK_ID: &str = "decoder_test_video_output";
const AUDIO_INPUT_TRACK_ID: &str = "decoder_test_audio_input";
const AUDIO_OUTPUT_TRACK_ID: &str = "decoder_test_audio_output";

#[test]
fn h264_multi_resolutions() -> hisui::Result<()> {
    let reader0 = Mp4VideoReader::new("testdata/archive-blue-640x480-h264.mp4")?;
    let reader1 = Mp4VideoReader::new("testdata/archive-red-320x320-h264.mp4")?;
    multi_resolutions_test(reader0, reader1)?;
    Ok(())
}

#[test]
#[cfg(target_os = "macos")]
fn h265_multi_resolutions() -> hisui::Result<()> {
    let reader0 = Mp4VideoReader::new("testdata/archive-blue-640x480-h265.mp4")?;
    let reader1 = Mp4VideoReader::new("testdata/archive-red-320x320-h265.mp4")?;
    multi_resolutions_test(reader0, reader1)?;
    Ok(())
}

#[test]
fn vp9_multi_resolutions() -> hisui::Result<()> {
    let reader0 = Mp4VideoReader::new("testdata/archive-blue-640x480-vp9.mp4")?;
    let reader1 = Mp4VideoReader::new("testdata/archive-red-320x320-vp9.mp4")?;
    multi_resolutions_test(reader0, reader1)?;
    Ok(())
}

#[test]
fn av1_multi_resolutions() -> hisui::Result<()> {
    let reader0 = Mp4VideoReader::new("testdata/archive-blue-640x480-av1.mp4")?;
    let reader1 = Mp4VideoReader::new("testdata/archive-red-320x320-av1.mp4")?;
    multi_resolutions_test(reader0, reader1)?;
    Ok(())
}

fn multi_resolutions_test<I>(reader0: I, reader1: I) -> hisui::Result<()>
where
    I: Iterator<Item = hisui::Result<VideoFrame>>,
{
    let openh264_lib = if let Ok(path) = std::env::var("OPENH264_PATH") {
        Some(Openh264Library::load(path)?)
    } else {
        eprintln!("no available OpenH264 decoder");
        return Ok(());
    };
    let options = VideoDecoderOptions {
        openh264_lib,
        decode_params: Default::default(),
        engines: None,
    };

    // デコードする
    let mut output_frames = Vec::new();
    let mut blue_count = 0;
    let mut red_count = 0;
    let mut input_frames = Vec::new();

    for input_frame in reader0 {
        input_frames.push(input_frame?);
        blue_count += 1;
    }

    // このタイミングで解像度などが切り替わる
    for input_frame in reader1 {
        input_frames.push(input_frame?);
        red_count += 1;
    }
    output_frames.extend(decode_video_frames_with_pipeline(input_frames, options)?);

    // デコード結果を確認する
    for output_frame in output_frames {
        if blue_count > 0 {
            blue_count -= 1;
            let size = output_frame.size().expect("infallible");
            assert_eq!(size.width, 640);
            assert_eq!(size.height, 480);

            // 単色青色かどうかのチェック
            let (y_plane, u_plane, v_plane) = output_frame
                .as_yuv_planes()
                .ok_or_else(|| hisui::Error::new("value is missing"))?;
            y_plane.iter().for_each(|&y| assert_eq!(y, 41));
            u_plane.iter().for_each(|&y| assert_eq!(y, 240));
            v_plane.iter().for_each(|&y| assert_eq!(y, 110));
        } else {
            red_count -= 1;
            let size = output_frame.size().expect("infallible");
            assert_eq!(size.width, 320);
            assert_eq!(size.height, 320);

            // 単色赤色かどうかのチェック
            let (y_plane, u_plane, v_plane) = output_frame
                .as_yuv_planes()
                .ok_or_else(|| hisui::Error::new("value is missing"))?;
            y_plane.iter().for_each(|&y| assert_eq!(y, 81));
            u_plane.iter().for_each(|&u| assert_eq!(u, 90));
            v_plane.iter().for_each(|&v| assert_eq!(v, 240));
        }
    }
    assert_eq!(blue_count, 0);
    assert_eq!(red_count, 0);

    Ok(())
}
#[test]
fn aac_decode() -> hisui::Result<()> {
    if !cfg!(target_os = "macos") && std::env::var("HISUI_FDK_AAC_PATH").is_err() {
        eprintln!("skipping: AAC test requires macOS or HISUI_FDK_AAC_PATH");
        return Ok(());
    }
    let reader = Mp4AudioReader::new("testdata/beep-aac-audio.mp4")?;
    let mut input_samples = Vec::new();
    for input_data in reader {
        input_samples.push(input_data?);
    }
    let decoded_count = decode_audio_count_with_pipeline(input_samples)?;
    assert!(decoded_count > 0, "Should decode at least one audio frame");
    Ok(())
}

fn decode_video_frames_with_pipeline(
    input_frames: Vec<VideoFrame>,
    options: VideoDecoderOptions,
) -> hisui::Result<Vec<VideoFrame>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let pipeline = MediaPipeline::new()?;
        let pipeline_handle = pipeline.handle();
        let mut pipeline_task = tokio::spawn(pipeline.run());

        let source_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_video_source"),
            ProcessorMetadata::new("decoder_test_video_source"),
        )
        .await?;
        let source_task = tokio::spawn(async move {
            run_video_source(
                source_handle,
                input_frames,
                TrackId::new(VIDEO_INPUT_TRACK_ID),
            )
            .await
        });

        let decoder_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_video_decoder"),
            ProcessorMetadata::new("video_decoder"),
        )
        .await?;
        let decoder_task = tokio::spawn(async move {
            let decoder = VideoDecoder::new(options, decoder_handle.stats());
            decoder
                .run(
                    decoder_handle,
                    TrackId::new(VIDEO_INPUT_TRACK_ID),
                    TrackId::new(VIDEO_OUTPUT_TRACK_ID),
                )
                .await
        });

        let sink_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_video_sink"),
            ProcessorMetadata::new("decoder_test_video_sink"),
        )
        .await?;
        let sink_task = tokio::spawn(async move {
            collect_video_frames(sink_handle, TrackId::new(VIDEO_OUTPUT_TRACK_ID)).await
        });

        pipeline_handle
            .trigger_start()
            .await
            .map_err(|_| hisui::Error::new("failed to trigger start: pipeline has terminated"))?;

        let output_frames = await_video_pipeline_tasks(
            source_task,
            decoder_task,
            sink_task,
            pipeline_handle,
            &mut pipeline_task,
        )
        .await?;
        Ok(output_frames)
    })
}

fn decode_audio_count_with_pipeline(input_samples: Vec<AudioFrame>) -> hisui::Result<usize> {
    // FDK-AAC ライブラリを環境変数から読み込む（macOS の場合は不要）
    #[cfg(feature = "fdk-aac")]
    let fdk_aac_lib = if let Ok(path) = std::env::var("HISUI_FDK_AAC_PATH") {
        Some(shiguredo_fdk_aac::FdkAacLibrary::load(path)?)
    } else {
        None
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let pipeline = MediaPipeline::new()?;
        let pipeline_handle = pipeline.handle();
        let mut pipeline_task = tokio::spawn(pipeline.run());

        let source_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_audio_source"),
            ProcessorMetadata::new("decoder_test_audio_source"),
        )
        .await?;
        let source_task = tokio::spawn(async move {
            run_audio_source(
                source_handle,
                input_samples,
                TrackId::new(AUDIO_INPUT_TRACK_ID),
            )
            .await
        });

        let decoder_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_audio_decoder"),
            ProcessorMetadata::new("audio_decoder"),
        )
        .await?;
        let decoder_task = tokio::spawn(async move {
            let decoder = AudioDecoder::new(
                #[cfg(feature = "fdk-aac")]
                fdk_aac_lib,
                decoder_handle.stats(),
            )?;
            decoder
                .run(
                    decoder_handle,
                    TrackId::new(AUDIO_INPUT_TRACK_ID),
                    TrackId::new(AUDIO_OUTPUT_TRACK_ID),
                )
                .await
        });

        let sink_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_audio_sink"),
            ProcessorMetadata::new("decoder_test_audio_sink"),
        )
        .await?;
        let sink_task = tokio::spawn(async move {
            collect_audio_count(sink_handle, TrackId::new(AUDIO_OUTPUT_TRACK_ID)).await
        });

        pipeline_handle
            .trigger_start()
            .await
            .map_err(|_| hisui::Error::new("failed to trigger start: pipeline has terminated"))?;

        let output_count = await_audio_pipeline_tasks(
            source_task,
            decoder_task,
            sink_task,
            pipeline_handle,
            &mut pipeline_task,
        )
        .await?;
        Ok(output_count)
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

async fn run_audio_source(
    handle: ProcessorHandle,
    samples: Vec<AudioFrame>,
    track_id: TrackId,
) -> hisui::Result<()> {
    let mut tx = handle.publish_track(track_id).await?;
    handle.notify_ready();
    handle.wait_subscribers_ready().await?;
    for sample in samples {
        if !tx.send_audio(sample) {
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

async fn collect_audio_count(handle: ProcessorHandle, track_id: TrackId) -> hisui::Result<usize> {
    let mut rx = handle.subscribe_track(track_id);
    handle.notify_ready();
    let mut count = 0usize;
    loop {
        match rx.recv().await {
            Message::Media(sample) => {
                let _data = sample.expect_audio()?;
                count += 1;
            }
            Message::Eos => break,
            Message::Syn(_) => {}
        }
    }
    Ok(count)
}

async fn await_video_pipeline_tasks(
    source_task: tokio::task::JoinHandle<hisui::Result<()>>,
    decoder_task: tokio::task::JoinHandle<hisui::Result<()>>,
    sink_task: tokio::task::JoinHandle<hisui::Result<Vec<VideoFrame>>>,
    pipeline_handle: hisui::MediaPipelineHandle,
    pipeline_task: &mut tokio::task::JoinHandle<()>,
) -> hisui::Result<Vec<VideoFrame>> {
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
    let output_frames = match sink_task.await {
        Ok(Ok(frames)) => frames,
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(hisui::Error::new(format!("sink task join failed: {e}"))),
    };

    drop(pipeline_handle);
    match tokio::time::timeout(std::time::Duration::from_secs(5), &mut *pipeline_task).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return Err(hisui::Error::new(format!(
                "media pipeline task failed: {e}"
            )));
        }
        Err(_) => {
            pipeline_task.abort();
            let _ = pipeline_task.await;
        }
    }

    Ok(output_frames)
}

async fn await_audio_pipeline_tasks(
    source_task: tokio::task::JoinHandle<hisui::Result<()>>,
    decoder_task: tokio::task::JoinHandle<hisui::Result<()>>,
    sink_task: tokio::task::JoinHandle<hisui::Result<usize>>,
    pipeline_handle: hisui::MediaPipelineHandle,
    pipeline_task: &mut tokio::task::JoinHandle<()>,
) -> hisui::Result<usize> {
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
    let output_count = match sink_task.await {
        Ok(Ok(count)) => count,
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(hisui::Error::new(format!("sink task join failed: {e}"))),
    };

    drop(pipeline_handle);
    match tokio::time::timeout(std::time::Duration::from_secs(5), &mut *pipeline_task).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return Err(hisui::Error::new(format!(
                "media pipeline task failed: {e}"
            )));
        }
        Err(_) => {
            pipeline_task.abort();
            let _ = pipeline_task.await;
        }
    }

    Ok(output_count)
}
