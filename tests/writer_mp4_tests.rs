use std::{path::PathBuf, sync::Arc, time::Duration};

use hisui::{
    audio::{AudioData, AudioFormat, SAMPLE_RATE},
    layout::{AggregatedSourceInfo, AssignedSource, Layout, Resolution},
    layout_region::{Grid, Region},
    media::{MediaSample, MediaStreamId},
    metadata::{SourceId, SourceInfo},
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput},
    types::{CodecName, EvenUsize, PixelPosition},
    video::{FrameRate, VideoFormat, VideoFrame},
    writer_mp4::{Mp4Writer, Mp4WriterOptions},
};
use orfail::OrFail;
use shiguredo_mp4::{
    BoxSize, BoxType,
    boxes::{SampleEntry, UnknownBox},
};

const AUDIO_STREAM_ID: MediaStreamId = MediaStreamId::new(0);
const VIDEO_STREAM_ID: MediaStreamId = MediaStreamId::new(1);

#[test]
fn write_audio_only_mp4() -> orfail::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new().or_fail()?;
    let source = source(0, secs(0), secs(60));
    let layout = layout(&[source.clone()], &[]);

    // ライターを作成する
    let mut writer = Mp4Writer::new(
        output_file_path.path(),
        &Mp4WriterOptions::from_layout(&layout),
        Some(AUDIO_STREAM_ID),
        None,
    )
    .or_fail()?;

    // 1 秒尺の音声データを供給する
    for i in 0..60 {
        let input = MediaProcessorInput {
            stream_id: AUDIO_STREAM_ID,
            sample: Some(MediaSample::Audio(Arc::new(audio_data(
                &source,
                i,
                secs(1),
            )))),
        };
        writer.process_input(input).or_fail()?;
    }

    // 音声入力の終了を通知
    let input = MediaProcessorInput {
        stream_id: AUDIO_STREAM_ID,
        sample: None,
    };
    writer.process_input(input).or_fail()?;

    // 最後まで書き込む
    while !matches!(
        writer.process_output().or_fail()?,
        MediaProcessorOutput::Finished
    ) {}

    // 統計値を確認する
    let stats = writer.stats();
    assert!(stats.actual_moov_box_size.get() > 0);
    assert!(stats.actual_moov_box_size.get() <= stats.reserved_moov_box_size.get());

    assert_eq!(stats.total_audio_chunk_count.get(), 1);
    assert_eq!(stats.total_audio_sample_count.get(), 60);
    assert_eq!(stats.total_audio_track_duration.get(), secs(60));

    assert_eq!(stats.total_video_chunk_count.get(), 0);
    assert_eq!(stats.total_video_sample_count.get(), 0);
    assert_eq!(stats.total_video_track_duration.get(), secs(0));

    Ok(())
}

#[test]
fn write_video_only_mp4() -> orfail::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new().or_fail()?;
    let source = source(0, secs(0), secs(60));
    let layout = layout(&[], &[source.clone()]);

    // ライターを作成する
    let mut writer = Mp4Writer::new(
        output_file_path.path(),
        &Mp4WriterOptions::from_layout(&layout),
        None,
        Some(VIDEO_STREAM_ID),
    )
    .or_fail()?;

    // 1 秒尺の映像フレームを供給する
    for i in 0..60 {
        let input = MediaProcessorInput {
            stream_id: VIDEO_STREAM_ID,
            sample: Some(MediaSample::Video(Arc::new(video_frame(
                &source,
                i,
                secs(1),
            )))),
        };
        writer.process_input(input).or_fail()?;
    }

    // 映像入力の終了を通知
    let input = MediaProcessorInput {
        stream_id: VIDEO_STREAM_ID,
        sample: None,
    };
    writer.process_input(input).or_fail()?;

    // 最後まで書き込む
    while !matches!(
        writer.process_output().or_fail()?,
        MediaProcessorOutput::Finished
    ) {}

    // 統計値を確認する
    let stats = writer.stats();
    assert!(stats.actual_moov_box_size.get() > 0);
    assert!(stats.actual_moov_box_size.get() <= stats.reserved_moov_box_size.get());

    assert_eq!(stats.total_audio_chunk_count.get(), 0);
    assert_eq!(stats.total_audio_sample_count.get(), 0);
    assert_eq!(stats.total_audio_track_duration.get(), secs(0));

    assert_eq!(stats.total_video_chunk_count.get(), 1);
    assert_eq!(stats.total_video_sample_count.get(), 60);
    assert_eq!(stats.total_video_track_duration.get(), secs(60));

    Ok(())
}

#[test]
fn write_video_and_audio_mp4() -> orfail::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new().or_fail()?;
    let audio_source = source(0, secs(0), secs(60));
    let video_source = source(1, secs(0), secs(60));
    let layout = layout(&[audio_source.clone()], &[video_source.clone()]);

    // ライターを作成する
    let mut writer = Mp4Writer::new(
        output_file_path.path(),
        &Mp4WriterOptions::from_layout(&layout),
        Some(AUDIO_STREAM_ID),
        Some(VIDEO_STREAM_ID),
    )
    .or_fail()?;

    // 1 秒尺の音声データ・映像フレームを供給する
    for i in 0..60 {
        let audio_input = MediaProcessorInput {
            stream_id: AUDIO_STREAM_ID,
            sample: Some(MediaSample::Audio(Arc::new(audio_data(
                &audio_source,
                i,
                secs(1),
            )))),
        };
        writer.process_input(audio_input).or_fail()?;

        let video_input = MediaProcessorInput {
            stream_id: VIDEO_STREAM_ID,
            sample: Some(MediaSample::Video(Arc::new(video_frame(
                &video_source,
                i,
                secs(1),
            )))),
        };
        writer.process_input(video_input).or_fail()?;
    }

    // 入力の終了を通知
    let audio_end_input = MediaProcessorInput {
        stream_id: AUDIO_STREAM_ID,
        sample: None,
    };
    writer.process_input(audio_end_input).or_fail()?;

    let video_end_input = MediaProcessorInput {
        stream_id: VIDEO_STREAM_ID,
        sample: None,
    };
    writer.process_input(video_end_input).or_fail()?;

    // 最後まで書き込む
    while !matches!(
        writer.process_output().or_fail()?,
        MediaProcessorOutput::Finished
    ) {}

    // 統計値を確認する
    let stats = writer.stats();
    assert!(stats.actual_moov_box_size.get() > 0);
    assert!(stats.actual_moov_box_size.get() <= stats.reserved_moov_box_size.get());

    assert_eq!(stats.total_audio_chunk_count.get(), 6); // 映像・音声混在時には 10 秒毎にチャンクが切り替わる
    assert_eq!(stats.total_audio_sample_count.get(), 60);
    assert_eq!(stats.total_audio_track_duration.get(), secs(60));

    assert_eq!(stats.total_video_chunk_count.get(), 6); // 映像・音声混在時には 10 秒毎にチャンクが切り替わる
    assert_eq!(stats.total_video_sample_count.get(), 60);
    assert_eq!(stats.total_video_track_duration.get(), secs(60));

    Ok(())
}

#[test]
fn no_video_and_audio_mp4() -> orfail::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new().or_fail()?;
    let layout = layout(&[], &[]);

    // ライターを作成する
    let mut writer = Mp4Writer::new(
        output_file_path.path(),
        &Mp4WriterOptions::from_layout(&layout),
        None,
        None,
    )
    .or_fail()?;

    // 最後まで書き込む
    while !matches!(
        writer.process_output().or_fail()?,
        MediaProcessorOutput::Finished
    ) {}

    // 統計値を確認する
    let stats = writer.stats();
    assert!(stats.actual_moov_box_size.get() > 0);
    assert!(stats.actual_moov_box_size.get() <= stats.reserved_moov_box_size.get());

    assert_eq!(stats.total_audio_chunk_count.get(), 0);
    assert_eq!(stats.total_audio_sample_count.get(), 0);
    assert_eq!(stats.total_audio_track_duration.get(), secs(0));

    assert_eq!(stats.total_video_chunk_count.get(), 0);
    assert_eq!(stats.total_video_sample_count.get(), 0);
    assert_eq!(stats.total_video_track_duration.get(), secs(0));

    Ok(())
}

fn layout(audio_sources: &[SourceInfo], video_sources: &[SourceInfo]) -> Layout {
    Layout {
        audio_source_ids: audio_sources.iter().map(|s| s.id.clone()).collect(),
        video_regions: if video_sources.is_empty() {
            Vec::new()
        } else {
            vec![region(video_sources)]
        },
        sources: audio_sources
            .iter()
            .chain(video_sources.iter())
            .map(|s| {
                (
                    s.id.clone(),
                    AggregatedSourceInfo {
                        id: s.id.clone(),
                        start_timestamp: s.start_timestamp,
                        stop_timestamp: s.stop_timestamp,
                        audio: true,
                        video: true,
                        format: Default::default(),
                        media_paths: Default::default(),
                    },
                )
            })
            .collect(),
        frame_rate: FrameRate::FPS_1,

        // 以下のフィールドはテストで使われないので、適当な値を設定しておく
        trim_spans: Default::default(),
        base_path: PathBuf::from(""),
        resolution: Resolution::new(16, 16).expect("infallible"),
        audio_codec: CodecName::Opus,
        video_codec: CodecName::Vp8,
        audio_bitrate: None,
        video_bitrate: None,
        encode_params: Default::default(),
    }
}

fn region(video_sources: &[SourceInfo]) -> Region {
    Region {
        grid: Grid {
            assigned_sources: video_sources
                .iter()
                .map(|source| {
                    (
                        source.id.clone(),
                        AssignedSource {
                            cell_index: 0,
                            priority: 0,
                        },
                    )
                })
                .collect(),
            rows: 0,
            columns: 0,
            cell_width: EvenUsize::truncating_new(4),
            cell_height: EvenUsize::truncating_new(4),
        },
        source_ids: video_sources.iter().map(|s| s.id.clone()).collect(),
        width: EvenUsize::truncating_new(16),
        height: EvenUsize::truncating_new(16),
        position: PixelPosition::default(),
        top_border_pixels: EvenUsize::default(),
        left_border_pixels: EvenUsize::default(),
        inner_border_pixels: EvenUsize::truncating_new(2),
        z_pos: 0,
        background_color: [0, 0, 0],
    }
}

fn secs(timestamp: u64) -> Duration {
    Duration::from_secs(timestamp)
}

fn source(id: usize, start_timestamp: Duration, stop_timestamp: Duration) -> SourceInfo {
    SourceInfo {
        id: SourceId::new(&id.to_string()),
        start_timestamp,
        stop_timestamp,

        // 以下はダミー値
        audio: true,
        video: true,
        format: Default::default(),
    }
}

fn audio_data(source: &SourceInfo, i: usize, duration: Duration) -> AudioData {
    AudioData {
        source_id: Some(source.id.clone()),
        data: vec![0], // 中身はなんでもいい
        format: AudioFormat::I16Be,
        stereo: true,
        sample_rate: SAMPLE_RATE,
        timestamp: source.start_timestamp + duration * i as u32,
        duration,
        sample_entry: if i == 0 {
            // 中身はなんでもいい
            Some(SampleEntry::Unknown(UnknownBox {
                box_type: BoxType::Normal(*b"dumy"),
                box_size: BoxSize::U32(8),
                payload: Vec::new(),
            }))
        } else {
            None
        },
    }
}

fn video_frame(source: &SourceInfo, i: usize, duration: Duration) -> VideoFrame {
    VideoFrame {
        source_id: Some(source.id.clone()),
        data: vec![0], // 中身はなんでもいい
        format: VideoFormat::I420,
        keyframe: i % 2 == 0,
        width: EvenUsize::MIN_CELL_SIZE.get(),
        height: EvenUsize::MIN_CELL_SIZE.get(),
        timestamp: source.start_timestamp + duration * i as u32,
        duration,
        sample_entry: if i == 0 {
            // 中身はなんでもいい
            Some(SampleEntry::Unknown(UnknownBox {
                box_type: BoxType::Normal(*b"dumy"),
                box_size: BoxSize::U32(8),
                payload: Vec::new(),
            }))
        } else {
            None
        },
    }
}
