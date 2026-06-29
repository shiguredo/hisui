use hisui::{
    TrackId,
    rtsp::subscriber::{RtspSubscriber, RtspSubscriberBuildError},
};

// audio + video の両 track_id を指定する一般的な利用パターンが受理されることを確認する
#[test]
fn new_accepts_with_both_track_ids() -> Result<(), RtspSubscriberBuildError> {
    RtspSubscriber::new(
        "rtsp://127.0.0.1:554/stream".to_owned(),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
    )?;
    Ok(())
}

// track_id が音声側のみでも「少なくとも片方」の条件を満たして受理されることを確認する
#[test]
fn new_accepts_with_audio_only_track_id() -> Result<(), RtspSubscriberBuildError> {
    RtspSubscriber::new(
        "rtsp://127.0.0.1:554/stream".to_owned(),
        Some(TrackId::new("audio")),
        None,
    )?;
    Ok(())
}

// track_id が映像側のみでも「少なくとも片方」の条件を満たして受理されることを確認する
#[test]
fn new_accepts_with_video_only_track_id() -> Result<(), RtspSubscriberBuildError> {
    RtspSubscriber::new(
        "rtsp://127.0.0.1:554/stream".to_owned(),
        None,
        Some(TrackId::new("video")),
    )?;
    Ok(())
}

// 空の input_url が EmptyInputUrl で拒否されることを退行検知する
// (audio 側 None・video 側 Some の組合せで、track_id 引数順入れ違い時の誤検知も避ける)
#[test]
fn new_rejects_empty_input_url() {
    let Err(err) = RtspSubscriber::new("".to_owned(), None, Some(TrackId::new("video"))) else {
        panic!("空 input_url は拒否される");
    };
    assert!(matches!(err, RtspSubscriberBuildError::EmptyInputUrl));
}

// audio / video の両 track_id が None なら NoTrackId で拒否されることを退行検知する
#[test]
fn new_rejects_both_track_ids_none() {
    let Err(err) = RtspSubscriber::new("rtsp://127.0.0.1:554/stream".to_owned(), None, None) else {
        panic!("両 track_id が None なら拒否される");
    };
    assert!(matches!(err, RtspSubscriberBuildError::NoTrackId));
}
