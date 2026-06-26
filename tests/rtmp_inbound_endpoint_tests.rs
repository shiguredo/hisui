use hisui::{
    TrackId,
    rtmp::inbound_endpoint::{
        RtmpInboundEndpoint, RtmpInboundEndpointBuildError, RtmpInboundEndpointOptions,
    },
};

#[test]
fn new_accepts_with_stream_name_some() {
    // 正常系: stream_name 指定あり + audio + video の両 track_id を指定
    let endpoint = RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        RtmpInboundEndpointOptions::default(),
    )
    .expect("正常系のコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_accepts_with_stream_name_none() {
    // 正常系: stream_name 未指定でも track_id が片方以上あれば成功する
    let endpoint = RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        None,
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        RtmpInboundEndpointOptions::default(),
    )
    .expect("stream_name 未指定でもコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_accepts_with_audio_only_track_id() {
    // 正常系: audio_track_id のみ指定でも片方ありで成功する
    let endpoint = RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpInboundEndpointOptions::default(),
    )
    .expect("audio のみでもコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_accepts_with_video_only_track_id() {
    // 正常系: video_track_id のみ指定でも片方ありで成功する
    let endpoint = RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        None,
        Some(TrackId::new("video")),
        RtmpInboundEndpointOptions::default(),
    )
    .expect("video のみでもコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_rejects_empty_input_url() {
    // 空の input_url は EmptyInputUrl で弾かれること
    let err = RtmpInboundEndpoint::new(
        "".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpInboundEndpointOptions::default(),
    )
    .err()
    .expect("空 input_url は弾く");
    assert!(matches!(err, RtmpInboundEndpointBuildError::EmptyInputUrl));
}

#[test]
fn new_rejects_empty_stream_name() {
    // 指定された stream_name が空文字なら EmptyStreamName で弾かれること
    let err = RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpInboundEndpointOptions::default(),
    )
    .err()
    .expect("空 stream_name は弾く");
    assert!(matches!(
        err,
        RtmpInboundEndpointBuildError::EmptyStreamName
    ));
}

#[test]
fn new_rejects_both_track_ids_none() {
    // 両 track_id が None なら NoTrackId で弾かれること
    let err = RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        None,
        None,
        RtmpInboundEndpointOptions::default(),
    )
    .err()
    .expect("両 track_id が None なら弾く");
    assert!(matches!(err, RtmpInboundEndpointBuildError::NoTrackId));
}
