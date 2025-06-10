use std::path::Path;

use crate::{
    metadata::{ContainerFormat, SourceId},
    reader::{AudioReader, VideoReader},
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
};

use orfail::OrFail;

pub fn run<P: AsRef<Path>>(input_file_path: P) -> orfail::Result<()> {
    let format = match input_file_path
        .as_ref()
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .as_ref()
    {
        "mp4" => ContainerFormat::Mp4,
        "webm" => ContainerFormat::Webm,
        ext => {
            return Err(orfail::Failure::new(format!(
                "unsupported container format: {ext}"
            )))
        }
    };

    let dummy_source_id = SourceId::new("inspect"); // 使われないのでなんでもいい

    let (video_reader, audio_reader) = match format {
        ContainerFormat::Webm => {
            let audio = AudioReader::Webm(
                WebmAudioReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?,
            );
            let video = VideoReader::Webm(
                WebmVideoReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?,
            );
            (audio, video)
        }
        ContainerFormat::Mp4 => {
            let audio = AudioReader::Mp4(
                Mp4AudioReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?,
            );
            let video = VideoReader::Mp4(
                Mp4VideoReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?,
            );
            (audio, video)
        }
    };

    Ok(())
}
