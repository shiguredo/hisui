use hisui::{
    TrackId,
    rtmp::outbound_endpoint::{
        RtmpOutboundEndpoint, RtmpOutboundEndpointBuildError, RtmpOutboundEndpointOptions,
    },
};

#[test]
fn new_accepts_with_stream_name_some() {
    // 正常系: stream_name 指定あり + audio + video の両 track_id を指定
    let endpoint = RtmpOutboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        RtmpOutboundEndpointOptions::default(),
    )
    .expect("正常系のコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_accepts_with_stream_name_none() {
    // 正常系: stream_name 未指定でも track_id が片方以上あれば成功する
    let endpoint = RtmpOutboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        None,
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        RtmpOutboundEndpointOptions::default(),
    )
    .expect("stream_name 未指定でもコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_accepts_with_audio_only_track_id() {
    // 正常系: audio_track_id のみ指定でも片方ありで成功する
    let endpoint = RtmpOutboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpOutboundEndpointOptions::default(),
    )
    .expect("audio のみでもコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_accepts_with_video_only_track_id() {
    // 正常系: video_track_id のみ指定でも片方ありで成功する
    let endpoint = RtmpOutboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        None,
        Some(TrackId::new("video")),
        RtmpOutboundEndpointOptions::default(),
    )
    .expect("video のみでもコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_rejects_empty_output_url() {
    // 空の output_url は EmptyOutputUrl で弾かれること
    let err = RtmpOutboundEndpoint::new(
        "".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpOutboundEndpointOptions::default(),
    )
    .expect_err("空 output_url は弾く");
    assert!(matches!(
        err,
        RtmpOutboundEndpointBuildError::EmptyOutputUrl
    ));
}

#[test]
fn new_rejects_empty_stream_name() {
    // 指定された stream_name が空文字なら EmptyStreamName で弾かれること
    let err = RtmpOutboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpOutboundEndpointOptions::default(),
    )
    .expect_err("空 stream_name は弾く");
    assert!(matches!(
        err,
        RtmpOutboundEndpointBuildError::EmptyStreamName
    ));
}

#[test]
fn new_rejects_both_track_ids_none() {
    // 両 track_id が None なら NoTrackId で弾かれること
    let err = RtmpOutboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        None,
        None,
        RtmpOutboundEndpointOptions::default(),
    )
    .expect_err("両 track_id が None なら弾く");
    assert!(matches!(err, RtmpOutboundEndpointBuildError::NoTrackId));
}
