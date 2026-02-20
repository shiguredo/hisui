use hisui::{
    metadata::SourceId,
    reader_webm::{WebmAudioReader, WebmVideoReader},
};

#[test]
fn webm_audio_reader_test() -> hisui::Result<()> {
    let reader =
        WebmAudioReader::new(SourceId::new("dummy"), "testdata/archive-black-silent.webm")?;
    for audio_data in reader {
        audio_data?;
    }
    Ok(())
}

#[test]
fn webm_video_reader_test() -> hisui::Result<()> {
    let reader =
        WebmVideoReader::new(SourceId::new("dummy"), "testdata/archive-black-silent.webm")?;
    for video_frame in reader {
        video_frame?;
    }
    Ok(())
}
