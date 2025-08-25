use hisui::{
    metadata::SourceId,
    reader_webm::{WebmAudioReader, WebmVideoReader},
};
use orfail::OrFail;

#[test]
fn webm_audio_reader_test() -> orfail::Result<()> {
    let reader = WebmAudioReader::new(
        SourceId::new("dummy"),
        "testdata/archive-black-silent.webm",
        Default::default(),
    )
    .or_fail()?;
    for audio_data in reader {
        audio_data.or_fail()?;
    }
    Ok(())
}

#[test]
fn webm_video_reader_test() -> orfail::Result<()> {
    let reader = WebmVideoReader::new(SourceId::new("dummy"), "testdata/archive-black-silent.webm")
        .or_fail()?;
    for video_frame in reader {
        video_frame.or_fail()?;
    }
    Ok(())
}
