use std::path::PathBuf;

use hisui::{
    TrackId,
    rtmp::inbound_endpoint::{
        RtmpInboundEndpoint, RtmpInboundEndpointBuildError, RtmpInboundEndpointOptions,
    },
};

// stream_name と track_id を両方指定する一般的な利用パターンが受理されることを確認する
#[test]
fn new_accepts_with_full_args() -> Result<(), RtmpInboundEndpointBuildError> {
    RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        RtmpInboundEndpointOptions::default(),
    )?;
    Ok(())
}

// stream_name 未指定 (None) を許容する仕様を退行検知する
#[test]
fn new_accepts_without_stream_name() -> Result<(), RtmpInboundEndpointBuildError> {
    RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        None,
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        RtmpInboundEndpointOptions::default(),
    )?;
    Ok(())
}

// track_id が音声側のみでも「少なくとも片方」の条件を満たして受理されることを確認する
#[test]
fn new_accepts_with_audio_only_track_id() -> Result<(), RtmpInboundEndpointBuildError> {
    RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpInboundEndpointOptions::default(),
    )?;
    Ok(())
}

// track_id が映像側のみでも「少なくとも片方」の条件を満たして受理されることを確認する
#[test]
fn new_accepts_with_video_only_track_id() -> Result<(), RtmpInboundEndpointBuildError> {
    RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        None,
        Some(TrackId::new("video")),
        RtmpInboundEndpointOptions::default(),
    )?;
    Ok(())
}

// TLS 用 cert_path / key_path を Some で渡しても new() は通る (lazy validation 維持の退行検知)
#[test]
fn new_accepts_with_tls_paths() -> Result<(), RtmpInboundEndpointBuildError> {
    RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        RtmpInboundEndpointOptions {
            cert_path: Some(PathBuf::from("dummy-cert.pem")),
            key_path: Some(PathBuf::from("dummy-key.pem")),
        },
    )?;
    Ok(())
}

// 空の input_url が EmptyInputUrl で拒否されることを退行検知する
// (audio 側 None・video 側 Some の組合せで、track_id 引数順入れ違い時の誤検知も避ける)
#[test]
fn new_rejects_empty_input_url() {
    let Err(err) = RtmpInboundEndpoint::new(
        "".to_owned(),
        Some("live".to_owned()),
        None,
        Some(TrackId::new("video")),
        RtmpInboundEndpointOptions::default(),
    ) else {
        panic!("空 input_url は拒否される");
    };
    assert!(matches!(err, RtmpInboundEndpointBuildError::EmptyInputUrl));
}

// 指定された stream_name が空文字なら EmptyStreamName で拒否されることを退行検知する
#[test]
fn new_rejects_empty_stream_name() {
    let Err(err) = RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("".to_owned()),
        Some(TrackId::new("audio")),
        None,
        RtmpInboundEndpointOptions::default(),
    ) else {
        panic!("空 stream_name は拒否される");
    };
    assert!(matches!(
        err,
        RtmpInboundEndpointBuildError::EmptyStreamName
    ));
}

// audio / video の両 track_id が None なら NoTrackId で拒否されることを退行検知する
#[test]
fn new_rejects_both_track_ids_none() {
    let Err(err) = RtmpInboundEndpoint::new(
        "rtmp://127.0.0.1:1935".to_owned(),
        Some("live".to_owned()),
        None,
        None,
        RtmpInboundEndpointOptions::default(),
    ) else {
        panic!("両 track_id が None なら拒否される");
    };
    assert!(matches!(err, RtmpInboundEndpointBuildError::NoTrackId));
}
