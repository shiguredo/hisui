use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use hisui::{
    audio::{AudioData, AudioDataReceiver, AudioDataSyncSender, AudioFormat, SAMPLE_RATE},
    channel,
    layout::{AggregatedSourceInfo, AssignedSource, Grid, Layout, Region, Resolution},
    metadata::{SourceId, SourceInfo},
    types::{CodecName, EvenUsize, PixelPosition},
    video::{FrameRate, VideoFormat, VideoFrame, VideoFrameReceiver, VideoFrameSyncSender},
    writer_mp4::Mp4Writer,
};
use orfail::OrFail;
use shiguredo_mp4::{
    BoxSize, BoxType,
    boxes::{SampleEntry, UnknownBox},
};

#[test]
fn write_audio_only_mp4() -> orfail::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new().or_fail()?;
    let source = source(0, secs(0), secs(60));
    let layout = layout(&[source.clone()], &[]);
    let ((audio_tx, audio_rx), (_video_tx, video_rx)) = channels();

    // ライターを作成する
    let mut writer =
        Mp4Writer::new(output_file_path.path(), &layout, audio_rx, video_rx).or_fail()?;

    // 1 秒尺の音声データを供給する
    for i in 0..60 {
        let _ = audio_tx.send(audio_data(&source, i, secs(1)));
    }
    std::mem::drop(audio_tx);

    // 最後まで書き込む
    while writer.poll().or_fail()?.is_some() {}

    // 統計値を確認する
    let stats = writer.stats();
    assert!(stats.actual_moov_box_size > 0);
    assert!(stats.actual_moov_box_size <= stats.reserved_moov_box_size);

    assert_eq!(stats.total_audio_chunk_count, 1);
    assert_eq!(stats.total_audio_sample_count, 60);
    assert_eq!(stats.total_audio_track_seconds.get(), secs(60));

    assert_eq!(stats.total_video_chunk_count, 0);
    assert_eq!(stats.total_video_sample_count, 0);
    assert_eq!(stats.total_video_track_seconds.get(), secs(0));

    Ok(())
}

#[test]
fn write_video_only_mp4() -> orfail::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new().or_fail()?;
    let source = source(0, secs(0), secs(60));
    let layout = layout(&[], &[source.clone()]);
    let ((_audio_tx, audio_rx), (video_tx, video_rx)) = channels();

    // ライターを作成する
    let mut writer =
        Mp4Writer::new(output_file_path.path(), &layout, audio_rx, video_rx).or_fail()?;

    // 1 秒尺の映像フレームを供給する
    for i in 0..60 {
        let _ = video_tx.send(video_frame(&source, i, secs(1)));
    }
    std::mem::drop(video_tx);

    // 最後まで書き込む
    while writer.poll().or_fail()?.is_some() {}

    // 統計値を確認する
    let stats = writer.stats();
    assert!(stats.actual_moov_box_size > 0);
    assert!(stats.actual_moov_box_size <= stats.reserved_moov_box_size);

    assert_eq!(stats.total_audio_chunk_count, 0);
    assert_eq!(stats.total_audio_sample_count, 0);
    assert_eq!(stats.total_audio_track_seconds.get(), secs(0));

    assert_eq!(stats.total_video_chunk_count, 1);
    assert_eq!(stats.total_video_sample_count, 60);
    assert_eq!(stats.total_video_track_seconds.get(), secs(60));

    Ok(())
}

#[test]
fn write_video_and_audio_mp4() -> orfail::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new().or_fail()?;
    let audio_source = source(0, secs(0), secs(60));
    let video_source = source(1, secs(0), secs(60));
    let layout = layout(&[audio_source.clone()], &[video_source.clone()]);
    let ((audio_tx, audio_rx), (video_tx, video_rx)) = channels();

    // ライターを作成する
    let mut writer =
        Mp4Writer::new(output_file_path.path(), &layout, audio_rx, video_rx).or_fail()?;

    // 1 秒尺の音声データ・映像フレームを供給する
    for i in 0..60 {
        let _ = audio_tx.send(audio_data(&audio_source, i, secs(1)));
        let _ = video_tx.send(video_frame(&video_source, i, secs(1)));
    }
    std::mem::drop(audio_tx);
    std::mem::drop(video_tx);

    // 最後まで書き込む
    while writer.poll().or_fail()?.is_some() {}

    // 統計値を確認する
    let stats = writer.stats();
    assert!(stats.actual_moov_box_size > 0);
    assert!(stats.actual_moov_box_size <= stats.reserved_moov_box_size);

    assert_eq!(stats.total_audio_chunk_count, 6); // 映像・音声混在時には 10 秒毎にチャンクが切り替わる
    assert_eq!(stats.total_audio_sample_count, 60);
    assert_eq!(stats.total_audio_track_seconds.get(), secs(60));

    assert_eq!(stats.total_video_chunk_count, 6); // 映像・音声混在時には 10 秒毎にチャンクが切り替わる
    assert_eq!(stats.total_video_sample_count, 60);
    assert_eq!(stats.total_video_track_seconds.get(), secs(60));

    Ok(())
}

#[test]
fn no_video_and_audio_mp4() -> orfail::Result<()> {
    let output_file_path = tempfile::NamedTempFile::new().or_fail()?;
    let layout = layout(&[], &[]);
    let ((_audio_tx, audio_rx), (_video_tx, video_rx)) = channels();

    // ライターを作成する
    let mut writer =
        Mp4Writer::new(output_file_path.path(), &layout, audio_rx, video_rx).or_fail()?;

    // 最後まで書き込む
    while writer.poll().or_fail()?.is_some() {}

    // 統計値を確認する
    let stats = writer.stats();
    assert!(stats.actual_moov_box_size > 0);
    assert!(stats.actual_moov_box_size <= stats.reserved_moov_box_size);

    assert_eq!(stats.total_audio_chunk_count, 0);
    assert_eq!(stats.total_audio_sample_count, 0);
    assert_eq!(stats.total_audio_track_seconds.get(), secs(0));

    assert_eq!(stats.total_video_chunk_count, 0);
    assert_eq!(stats.total_video_sample_count, 0);
    assert_eq!(stats.total_video_track_seconds.get(), secs(0));

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
        fps: FrameRate::FPS_1,

        // 以下のフィールドはテストで使われないので、適当な値を設定しておく
        trim_spans: BTreeMap::new(),
        base_path: PathBuf::from(""),
        resolution: Resolution::new(16, 16).expect("infallible"),
        bitrate_kbps: 0,
        audio_codec: CodecName::Opus,
        video_codec: CodecName::Vp8,
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
        z_pos: 0,
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

fn channels() -> (
    (AudioDataSyncSender, AudioDataReceiver),
    (VideoFrameSyncSender, VideoFrameReceiver),
) {
    (
        channel::sync_channel_with_bound(1000),
        channel::sync_channel_with_bound(1000),
    )
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
        width: EvenUsize::MIN_CELL_SIZE,
        height: EvenUsize::MIN_CELL_SIZE,
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
