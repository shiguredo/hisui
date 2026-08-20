use std::{path::Path, time::Duration};

use hisui::{
    Error, MediaPipeline, ProcessorHandle, ProcessorId, ProcessorMetadata, TrackId,
    audio::{AudioFormat, AudioFrame, Channels, SampleRate},
    mp4::writer::{Mp4Writer, Mp4WriterOptions},
    sample_entry::SharedSampleEntry,
    types::EvenUsize,
    video::{FrameRate, VideoFormat, VideoFrame},
};
use shiguredo_mp4::{
    BoxSize, BoxType,
    boxes::{SampleEntry, UnknownBox},
};

const AUDIO_TRACK_ID: &str = "writer_test_audio";
const VIDEO_TRACK_ID: &str = "writer_test_video";

#[test]
fn write_audio_only_mp4() -> hisui::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new()?;
    let source = source(secs(0));
    let audio_samples = (0..60)
        .map(|i| audio_data(&source, i, secs(1)))
        .collect::<Vec<_>>();
    let entries = run_writer_with_pipeline(
        output_file_path.path(),
        Some(mp4_writer_options()),
        Some(audio_samples),
        None,
    )?;

    // 統計値を確認する
    let actual_moov = writer_gauge(&entries, "actual_moov_box_size")?;
    let reserved_moov = writer_gauge(&entries, "reserved_moov_box_size")?;
    assert!(actual_moov > 0);
    assert!(actual_moov <= reserved_moov);

    assert_eq!(writer_gauge(&entries, "total_audio_chunk_count")?, 1);
    assert_eq!(writer_counter(&entries, "total_audio_sample_count")?, 60);
    assert_eq!(
        writer_duration(&entries, "total_audio_track_seconds")?,
        secs(60)
    );

    assert_eq!(writer_gauge(&entries, "total_video_chunk_count")?, 0);
    assert_eq!(writer_counter(&entries, "total_video_sample_count")?, 0);
    assert_eq!(
        writer_duration(&entries, "total_video_track_seconds")?,
        secs(0)
    );

    Ok(())
}

#[test]
fn write_video_only_mp4() -> hisui::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new()?;
    let source = source(secs(0));
    let video_frames = (0..60)
        .map(|i| video_frame(&source, i, secs(1)))
        .collect::<Vec<_>>();
    let entries = run_writer_with_pipeline(
        output_file_path.path(),
        Some(mp4_writer_options()),
        None,
        Some(video_frames),
    )?;

    // 統計値を確認する
    let actual_moov = writer_gauge(&entries, "actual_moov_box_size")?;
    let reserved_moov = writer_gauge(&entries, "reserved_moov_box_size")?;
    assert!(actual_moov > 0);
    assert!(actual_moov <= reserved_moov);

    assert_eq!(writer_gauge(&entries, "total_audio_chunk_count")?, 0);
    assert_eq!(writer_counter(&entries, "total_audio_sample_count")?, 0);
    assert_eq!(
        writer_duration(&entries, "total_audio_track_seconds")?,
        secs(0)
    );

    assert_eq!(writer_gauge(&entries, "total_video_chunk_count")?, 1);
    assert_eq!(writer_counter(&entries, "total_video_sample_count")?, 60);
    assert_eq!(
        writer_duration(&entries, "total_video_track_seconds")?,
        secs(60)
    );

    Ok(())
}

#[test]
fn write_video_and_audio_mp4() -> hisui::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new()?;
    let audio_source = source(secs(0));
    let video_source = source(secs(0));

    let audio_samples = (0..60)
        .map(|i| audio_data(&audio_source, i, secs(1)))
        .collect::<Vec<_>>();
    let video_frames = (0..60)
        .map(|i| video_frame(&video_source, i, secs(1)))
        .collect::<Vec<_>>();
    let entries = run_writer_with_pipeline(
        output_file_path.path(),
        Some(mp4_writer_options()),
        Some(audio_samples),
        Some(video_frames),
    )?;

    // 統計値を確認する
    let actual_moov = writer_gauge(&entries, "actual_moov_box_size")?;
    let reserved_moov = writer_gauge(&entries, "reserved_moov_box_size")?;
    assert!(actual_moov > 0);
    assert!(actual_moov <= reserved_moov);

    // 映像・音声混在時には 10 秒毎にチャンクが切り替わる
    assert_eq!(writer_gauge(&entries, "total_audio_chunk_count")?, 6);
    assert_eq!(writer_counter(&entries, "total_audio_sample_count")?, 60);
    assert_eq!(
        writer_duration(&entries, "total_audio_track_seconds")?,
        secs(60)
    );

    // 映像・音声混在時には 10 秒毎にチャンクが切り替わる
    assert_eq!(writer_gauge(&entries, "total_video_chunk_count")?, 6);
    assert_eq!(writer_counter(&entries, "total_video_sample_count")?, 60);
    assert_eq!(
        writer_duration(&entries, "total_video_track_seconds")?,
        secs(60)
    );

    Ok(())
}

#[test]
fn no_video_and_audio_mp4() -> hisui::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new()?;
    let entries = run_writer_with_pipeline(
        output_file_path.path(),
        Some(mp4_writer_options()),
        None,
        None,
    )?;

    // 統計値を確認する
    let actual_moov = writer_gauge(&entries, "actual_moov_box_size")?;
    let reserved_moov = writer_gauge(&entries, "reserved_moov_box_size")?;
    assert!(actual_moov > 0);
    assert!(actual_moov <= reserved_moov);

    assert_eq!(writer_gauge(&entries, "total_audio_chunk_count")?, 0);
    assert_eq!(writer_counter(&entries, "total_audio_sample_count")?, 0);
    assert_eq!(
        writer_duration(&entries, "total_audio_track_seconds")?,
        secs(0)
    );

    assert_eq!(writer_gauge(&entries, "total_video_chunk_count")?, 0);
    assert_eq!(writer_counter(&entries, "total_video_sample_count")?, 0);
    assert_eq!(
        writer_duration(&entries, "total_video_track_seconds")?,
        secs(0)
    );

    Ok(())
}

fn run_writer_with_pipeline(
    output_path: &Path,
    options: Option<Mp4WriterOptions>,
    audio_samples: Option<Vec<AudioFrame>>,
    video_frames: Option<Vec<VideoFrame>>,
) -> hisui::Result<Vec<hisui::stats::StatsEntry>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let pipeline = MediaPipeline::new(Default::default(), Default::default())?;
        let pipeline_handle = pipeline.handle();
        let mut pipeline_task = tokio::spawn(async move {
            pipeline.run().await;
        });

        let has_audio = audio_samples.is_some();
        let has_video = video_frames.is_some();
        let mut processor_tasks = Vec::new();

        match (audio_samples, video_frames) {
            (Some(audio_samples), Some(video_frames)) => {
                let av_source_handle = register_processor(
                    &pipeline_handle,
                    ProcessorId::new("writer_test_av_source"),
                    ProcessorMetadata::new("writer_test_av_source"),
                )
                .await?;
                let task = tokio::spawn(async move {
                    run_audio_video_source(av_source_handle, audio_samples, video_frames).await
                });
                processor_tasks.push(task);
            }
            (Some(audio_samples), None) => {
                let audio_source_handle = register_processor(
                    &pipeline_handle,
                    ProcessorId::new("writer_test_audio_source"),
                    ProcessorMetadata::new("writer_test_audio_source"),
                )
                .await?;
                let task = tokio::spawn(async move {
                    run_audio_source(audio_source_handle, audio_samples).await
                });
                processor_tasks.push(task);
            }
            (None, Some(video_frames)) => {
                let video_source_handle = register_processor(
                    &pipeline_handle,
                    ProcessorId::new("writer_test_video_source"),
                    ProcessorMetadata::new("writer_test_video_source"),
                )
                .await?;
                let task = tokio::spawn(async move {
                    run_video_source(video_source_handle, video_frames).await
                });
                processor_tasks.push(task);
            }
            (None, None) => {}
        }

        let writer_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("writer_test_mp4_writer"),
            ProcessorMetadata::new("mp4_writer"),
        )
        .await?;
        let output_path = output_path.to_path_buf();
        let writer_task = tokio::spawn(async move {
            let input_audio_track_id = has_audio.then_some(TrackId::new(AUDIO_TRACK_ID));
            let input_video_track_id = has_video.then_some(TrackId::new(VIDEO_TRACK_ID));
            let writer = Mp4Writer::new(
                &output_path,
                options,
                input_audio_track_id.clone(),
                input_video_track_id.clone(),
                writer_handle.stats(),
            )?;
            writer
                .run(writer_handle, input_audio_track_id, input_video_track_id)
                .await
        });
        processor_tasks.push(writer_task);

        pipeline_handle
            .trigger_start()
            .await
            .map_err(|_| Error::new("failed to trigger start: pipeline has terminated"))?;

        for task in processor_tasks {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(Error::new(format!("processor task join failed: {e}"))),
            }
        }

        let entries = pipeline_handle.stats().entries()?;
        drop(pipeline_handle);
        match tokio::time::timeout(Duration::from_secs(5), &mut pipeline_task).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(Error::new(format!("media pipeline task failed: {e}"))),
            Err(_) => {
                pipeline_task.abort();
                let _ = pipeline_task.await;
            }
        }
        Ok(entries)
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
                Error::new("failed to register processor: pipeline has terminated")
            }
            hisui::RegisterProcessorError::DuplicateProcessorId => Error::new(format!(
                "processor ID already exists: {}",
                processor_id.get()
            )),
        })
}

async fn run_audio_source(handle: ProcessorHandle, samples: Vec<AudioFrame>) -> hisui::Result<()> {
    let mut tx = handle.publish_track(TrackId::new(AUDIO_TRACK_ID)).await?;
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

async fn run_video_source(handle: ProcessorHandle, frames: Vec<VideoFrame>) -> hisui::Result<()> {
    let mut tx = handle.publish_track(TrackId::new(VIDEO_TRACK_ID)).await?;
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

async fn run_audio_video_source(
    handle: ProcessorHandle,
    audio_samples: Vec<AudioFrame>,
    video_frames: Vec<VideoFrame>,
) -> hisui::Result<()> {
    let mut audio_tx = handle.publish_track(TrackId::new(AUDIO_TRACK_ID)).await?;
    let mut video_tx = handle.publish_track(TrackId::new(VIDEO_TRACK_ID)).await?;
    handle.notify_ready();
    handle.wait_subscribers_ready().await?;

    let max_len = audio_samples.len().max(video_frames.len());
    for i in 0..max_len {
        if let Some(sample) = audio_samples.get(i)
            && !audio_tx.send_audio(sample.clone())
        {
            break;
        }
        if let Some(frame) = video_frames.get(i)
            && !video_tx.send_video(frame.clone())
        {
            break;
        }
    }

    audio_tx.send_eos();
    video_tx.send_eos();
    Ok(())
}

fn writer_metric<'a>(
    entries: &'a [hisui::stats::StatsEntry],
    metric_name: &str,
) -> Option<&'a hisui::stats::StatsValue> {
    entries.iter().find_map(|entry| {
        if entry.metric_name != metric_name {
            return None;
        }
        if entry.labels.get("processor_type").map(String::as_str) != Some("mp4_writer") {
            return None;
        }
        Some(&entry.value)
    })
}

fn writer_counter(entries: &[hisui::stats::StatsEntry], metric_name: &str) -> hisui::Result<u64> {
    writer_metric(entries, metric_name)
        .and_then(|v| v.as_counter())
        .ok_or_else(|| Error::new(format!("missing writer counter metric: {metric_name}")))
}

fn writer_gauge(entries: &[hisui::stats::StatsEntry], metric_name: &str) -> hisui::Result<u64> {
    writer_metric(entries, metric_name)
        .and_then(|v| v.as_gauge())
        .map(|v| v.max(0) as u64)
        .ok_or_else(|| Error::new(format!("missing writer gauge metric: {metric_name}")))
}

fn writer_duration(
    entries: &[hisui::stats::StatsEntry],
    metric_name: &str,
) -> hisui::Result<Duration> {
    writer_metric(entries, metric_name)
        .and_then(|v| v.as_duration())
        .ok_or_else(|| Error::new(format!("missing writer duration metric: {metric_name}")))
}

/// テスト用の `Mp4WriterOptions`
///
/// 以前はレイアウト (`Layout::mp4_writer_options`) から生成していたが、
/// 録画機能の削除に伴い直接生成する形へ置き換えた。
/// 値は旧レイアウトと同じく「60 秒・1 FPS」を表す。
fn mp4_writer_options() -> Mp4WriterOptions {
    Mp4WriterOptions {
        duration: secs(60),
        frame_rate: FrameRate::FPS_1,
    }
}

fn secs(timestamp: u64) -> Duration {
    Duration::from_secs(timestamp)
}

/// テスト用のソースタイミング情報
struct TestSource {
    start_timestamp: Duration,
}

fn source(start_timestamp: Duration) -> TestSource {
    TestSource { start_timestamp }
}

fn audio_data(source: &TestSource, i: usize, duration: Duration) -> AudioFrame {
    AudioFrame {
        data: vec![0], // 中身はなんでもいい
        format: AudioFormat::I16Be,
        channels: Channels::STEREO,
        sample_rate: SampleRate::HZ_48000,
        timestamp: source.start_timestamp + duration * i as u32,
        sample_entry: if i == 0 {
            // 中身はなんでもいい
            Some(SharedSampleEntry::new(SampleEntry::Unknown(UnknownBox {
                box_type: BoxType::Normal(*b"dumy"),
                box_size: BoxSize::U32(8),
                payload: Vec::new(),
            })))
        } else {
            None
        },
    }
}

fn video_frame(source: &TestSource, i: usize, duration: Duration) -> VideoFrame {
    VideoFrame {
        data: vec![0], // 中身はなんでもいい
        format: VideoFormat::I420,
        keyframe: i.is_multiple_of(2),
        size: Some(hisui::video::VideoFrameSize {
            width: EvenUsize::MIN_CELL_SIZE.get(),
            height: EvenUsize::MIN_CELL_SIZE.get(),
        }),
        timestamp: source.start_timestamp + duration * i as u32,
        sample_entry: if i == 0 {
            // 中身はなんでもいい
            Some(SharedSampleEntry::new(SampleEntry::Unknown(UnknownBox {
                box_type: BoxType::Normal(*b"dumy"),
                box_size: BoxSize::U32(8),
                payload: Vec::new(),
            })))
        } else {
            None
        },
    }
}
