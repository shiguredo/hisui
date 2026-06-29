use std::time::Duration;

use shiguredo_srt::KeyLength;

use hisui::{
    TrackId,
    srt::inbound_endpoint::{
        SrtInboundEndpoint, SrtInboundEndpointBuildError, SrtInboundEndpointOptions,
    },
};

// Options をデフォルトで渡す最小入力が受理されることを確認する
#[test]
fn new_accepts_with_options_default() -> Result<(), SrtInboundEndpointBuildError> {
    SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        SrtInboundEndpointOptions::default(),
    )?;
    Ok(())
}

// Options 全フィールドを Some で埋めても new() は通る
// (key_length / tsbpd_delay_ms を Some にしても eager 検証は走らないことの退行検知)
#[test]
fn new_accepts_with_all_options_some() -> Result<(), SrtInboundEndpointBuildError> {
    SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
        SrtInboundEndpointOptions {
            stream_id: Some("test-stream".to_owned()),
            passphrase: Some("secret123456".to_owned()),
            key_length: Some(KeyLength::Aes128),
            tsbpd_delay_ms: Some(Duration::from_millis(120)),
        },
    )?;
    Ok(())
}

// track_id が音声側のみでも「少なくとも片方」の条件を満たして受理されることを確認する
#[test]
fn new_accepts_with_audio_only_track_id() -> Result<(), SrtInboundEndpointBuildError> {
    SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        Some(TrackId::new("audio")),
        None,
        SrtInboundEndpointOptions::default(),
    )?;
    Ok(())
}

// track_id が映像側のみでも「少なくとも片方」の条件を満たして受理されることを確認する
#[test]
fn new_accepts_with_video_only_track_id() -> Result<(), SrtInboundEndpointBuildError> {
    SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        None,
        Some(TrackId::new("video")),
        SrtInboundEndpointOptions::default(),
    )?;
    Ok(())
}

// 空の input_url が EmptyInputUrl で拒否されることを退行検知する
// (audio 側 None・video 側 Some の組合せで、track_id 引数順入れ違い時の誤検知も避ける)
#[test]
fn new_rejects_empty_input_url() {
    let Err(err) = SrtInboundEndpoint::new(
        "".to_owned(),
        None,
        Some(TrackId::new("video")),
        SrtInboundEndpointOptions::default(),
    ) else {
        panic!("空 input_url は拒否される");
    };
    assert!(matches!(err, SrtInboundEndpointBuildError::EmptyInputUrl));
}

// 指定された stream_id が空文字なら EmptyStreamId で拒否されることを退行検知する
#[test]
fn new_rejects_empty_stream_id() {
    let Err(err) = SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        Some(TrackId::new("audio")),
        None,
        SrtInboundEndpointOptions {
            stream_id: Some("".to_owned()),
            ..Default::default()
        },
    ) else {
        panic!("空 stream_id は拒否される");
    };
    assert!(matches!(err, SrtInboundEndpointBuildError::EmptyStreamId));
}

// 指定された passphrase が空文字なら EmptyPassphrase で拒否されることを退行検知する
#[test]
fn new_rejects_empty_passphrase() {
    let Err(err) = SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        Some(TrackId::new("audio")),
        None,
        SrtInboundEndpointOptions {
            passphrase: Some("".to_owned()),
            ..Default::default()
        },
    ) else {
        panic!("空 passphrase は拒否される");
    };
    assert!(matches!(err, SrtInboundEndpointBuildError::EmptyPassphrase));
}

// audio / video の両 track_id が None なら NoTrackId で拒否されることを退行検知する
#[test]
fn new_rejects_both_track_ids_none() {
    let Err(err) = SrtInboundEndpoint::new(
        "srt://127.0.0.1:9000".to_owned(),
        None,
        None,
        SrtInboundEndpointOptions::default(),
    ) else {
        panic!("両 track_id が None なら拒否される");
    };
    assert!(matches!(err, SrtInboundEndpointBuildError::NoTrackId));
}
