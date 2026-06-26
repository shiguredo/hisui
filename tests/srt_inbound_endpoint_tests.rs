use hisui::{
    TrackId,
    srt::inbound_endpoint::{
        SrtInboundEndpoint, SrtInboundEndpointBuildError, SrtInboundEndpointOptions,
    },
};

#[test]
fn new_accepts_with_options_default() {
    // 正常系: Options をデフォルト + audio + video の両 track_id を指定
    let endpoint = SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        SrtInboundEndpointOptions::default(),
    )
    .expect("正常系のコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_accepts_with_options_filled() {
    // 正常系: Options に stream_id と passphrase を指定
    let endpoint = SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        SrtInboundEndpointOptions {
            stream_id: Some("test-stream".to_owned()),
            passphrase: Some("secret123456".to_owned()),
            key_length: None,
            tsbpd_delay_ms: None,
        },
    )
    .expect("Options に値を指定してもコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_accepts_with_audio_only_track_id() {
    // 正常系: audio_track_id のみ指定でも片方ありで成功する
    let endpoint = SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        Some(TrackId::new("audio")),
        None,
        SrtInboundEndpointOptions::default(),
    )
    .expect("audio のみでもコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_accepts_with_video_only_track_id() {
    // 正常系: video_track_id のみ指定でも片方ありで成功する
    let endpoint = SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        None,
        Some(TrackId::new("video")),
        SrtInboundEndpointOptions::default(),
    )
    .expect("video のみでもコンストラクタは成功する");
    let _ = endpoint;
}

#[test]
fn new_rejects_empty_input_url() {
    // 空の input_url は EmptyInputUrl で弾かれること
    let err = SrtInboundEndpoint::new(
        "".to_owned(),
        Some(TrackId::new("audio")),
        None,
        SrtInboundEndpointOptions::default(),
    )
    .err()
    .expect("空 input_url は弾く");
    assert!(matches!(err, SrtInboundEndpointBuildError::EmptyInputUrl));
}

#[test]
fn new_rejects_empty_stream_id() {
    // 指定された stream_id が空文字なら EmptyStreamId で弾かれること
    let err = SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        Some(TrackId::new("audio")),
        None,
        SrtInboundEndpointOptions {
            stream_id: Some("".to_owned()),
            passphrase: None,
            key_length: None,
            tsbpd_delay_ms: None,
        },
    )
    .err()
    .expect("空 stream_id は弾く");
    assert!(matches!(err, SrtInboundEndpointBuildError::EmptyStreamId));
}

#[test]
fn new_rejects_empty_passphrase() {
    // 指定された passphrase が空文字なら EmptyPassphrase で弾かれること
    let err = SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        Some(TrackId::new("audio")),
        None,
        SrtInboundEndpointOptions {
            stream_id: None,
            passphrase: Some("".to_owned()),
            key_length: None,
            tsbpd_delay_ms: None,
        },
    )
    .err()
    .expect("空 passphrase は弾く");
    assert!(matches!(err, SrtInboundEndpointBuildError::EmptyPassphrase));
}

#[test]
fn new_rejects_both_track_ids_none() {
    // 両 track_id が None なら NoTrackId で弾かれること
    let err = SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        None,
        None,
        SrtInboundEndpointOptions::default(),
    )
    .err()
    .expect("両 track_id が None なら弾く");
    assert!(matches!(err, SrtInboundEndpointBuildError::NoTrackId));
}
