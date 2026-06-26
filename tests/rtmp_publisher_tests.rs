use std::num::NonZeroUsize;

use hisui::{
    TrackId,
    rtmp::publisher::{RtmpPublisher, RtmpPublisherBuildError, RtmpPublisherOptions},
};

#[test]
fn new_accepts_with_stream_name_some() {
    // 正常系: stream_name 指定あり + audio + video の両 track_id を指定
    let publisher = RtmpPublisher::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        RtmpPublisherOptions::default(),
    )
    .expect("正常系のコンストラクタは成功する");
    let _ = publisher;
}

#[test]
fn new_accepts_with_stream_name_none() {
    // 正常系: stream_name 未指定でも track_id が片方以上あれば成功する
    let publisher = RtmpPublisher::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        None,
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        RtmpPublisherOptions::default(),
    )
    .expect("stream_name 未指定でもコンストラクタは成功する");
    let _ = publisher;
}

#[test]
fn new_accepts_with_audio_only_track_id() {
    // 正常系: audio_track_id のみ指定でも片方ありで成功する
    let publisher = RtmpPublisher::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpPublisherOptions::default(),
    )
    .expect("audio のみでもコンストラクタは成功する");
    let _ = publisher;
}

#[test]
fn new_accepts_with_video_only_track_id() {
    // 正常系: video_track_id のみ指定でも片方ありで成功する
    let publisher = RtmpPublisher::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        None,
        Some(TrackId::new("video")),
        RtmpPublisherOptions::default(),
    )
    .expect("video のみでもコンストラクタは成功する");
    let _ = publisher;
}

#[test]
fn new_rejects_empty_output_url() {
    // 空の output_url は EmptyOutputUrl で弾かれること
    let err = RtmpPublisher::new(
        "".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpPublisherOptions::default(),
    )
    .expect_err("空 output_url は弾く");
    assert!(matches!(err, RtmpPublisherBuildError::EmptyOutputUrl));
}

#[test]
fn new_rejects_empty_stream_name() {
    // 指定された stream_name が空文字なら EmptyStreamName で弾かれること
    let err = RtmpPublisher::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpPublisherOptions::default(),
    )
    .expect_err("空 stream_name は弾く");
    assert!(matches!(err, RtmpPublisherBuildError::EmptyStreamName));
}

#[test]
fn new_rejects_both_track_ids_none() {
    // 両 track_id が None なら NoTrackId で弾かれること
    let err = RtmpPublisher::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        None,
        None,
        RtmpPublisherOptions::default(),
    )
    .expect_err("両 track_id が None なら弾く");
    assert!(matches!(err, RtmpPublisherBuildError::NoTrackId));
}

#[test]
fn options_default_max_buffered_frame_count_is_1000() {
    // デフォルト値が 1000 のままであること (NonZeroUsize 化後の退行検知)
    assert_eq!(
        RtmpPublisherOptions::default().max_buffered_frame_count,
        NonZeroUsize::new(1000).expect("non-zero constant"),
    );
}
