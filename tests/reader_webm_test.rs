use hisui::reader_webm::{WebmAudioReader, WebmVideoReader};

#[test]
fn webm_audio_reader_test() -> hisui::Result<()> {
    let mut reader = WebmAudioReader::new("testdata/archive-black-silent.webm")?;
    let mut last_timestamp = None;
    for audio_data in reader.by_ref() {
        let audio_data = audio_data?;
        last_timestamp = Some(audio_data.timestamp);
    }
    if let Some(last_timestamp) = last_timestamp {
        assert_eq!(reader.stats().total_track_duration, last_timestamp);
    }
    Ok(())
}

#[test]
fn webm_video_reader_test() -> hisui::Result<()> {
    let mut reader = WebmVideoReader::new("testdata/archive-black-silent.webm")?;
    let mut last_timestamp = None;
    for video_frame in reader.by_ref() {
        let video_frame = video_frame?;
        last_timestamp = Some(video_frame.timestamp);
    }
    if let Some(last_timestamp) = last_timestamp {
        assert_eq!(reader.stats().total_track_duration, last_timestamp);
    }
    Ok(())
}
