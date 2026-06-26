use hisui::{
    TrackId,
    rtsp::subscriber::{RtspSubscriber, RtspSubscriberBuildError},
};

#[test]
fn new_accepts_with_both_track_ids() {
    // 正常系: audio + video の両 track_id を指定
    let subscriber = RtspSubscriber::new(
        "rtsp://127.0.0.1:554/stream".to_owned(),
        Some(TrackId::new("audio")),
        Some(TrackId::new("video")),
    )
    .expect("正常系のコンストラクタは成功する");
    let _ = subscriber;
}

#[test]
fn new_accepts_with_audio_only_track_id() {
    // 正常系: audio_track_id のみ指定でも片方ありで成功する
    let subscriber = RtspSubscriber::new(
        "rtsp://127.0.0.1:554/stream".to_owned(),
        Some(TrackId::new("audio")),
        None,
    )
    .expect("audio のみでもコンストラクタは成功する");
    let _ = subscriber;
}

#[test]
fn new_accepts_with_video_only_track_id() {
    // 正常系: video_track_id のみ指定でも片方ありで成功する
    let subscriber = RtspSubscriber::new(
        "rtsp://127.0.0.1:554/stream".to_owned(),
        None,
        Some(TrackId::new("video")),
    )
    .expect("video のみでもコンストラクタは成功する");
    let _ = subscriber;
}

#[test]
fn new_rejects_empty_input_url() {
    // 空の input_url は EmptyInputUrl で弾かれること
    let err = RtspSubscriber::new("".to_owned(), Some(TrackId::new("audio")), None)
        .expect_err("空 input_url は弾く");
    assert!(matches!(err, RtspSubscriberBuildError::EmptyInputUrl));
}

#[test]
fn new_rejects_both_track_ids_none() {
    // 両 track_id が None なら NoTrackId で弾かれること
    let err = RtspSubscriber::new("rtsp://127.0.0.1:554/stream".to_owned(), None, None)
        .expect_err("両 track_id が None なら弾く");
    assert!(matches!(err, RtspSubscriberBuildError::NoTrackId));
}
