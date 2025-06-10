use std::path::{Path, PathBuf};

use crate::{
    metadata::{ContainerFormat, SourceId},
    reader::{AudioReader, VideoReader},
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    types::CodecName,
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

    let (audio_reader, video_reader) = match format {
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

    let mut audio_codec = None;
    for sample in audio_reader {
        let sample = sample.or_fail()?;
        if audio_codec.is_none() {
            audio_codec = sample.format.codec_name();
        }
    }

    let mut video_codec = None;
    for sample in video_reader {
        let sample = sample.or_fail()?;
        if video_codec.is_none() {
            video_codec = sample.format.codec_name();
        }
    }

    // 入力ファイルから取得した情報を出力する
    let info = FileInfo {
        path: input_file_path.as_ref().to_path_buf(),
        format,
        audio_codec,
        video_codec,
    };
    println!(
        "{}",
        nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.value(&info)
        })
    );

    Ok(())
}

#[derive(Debug)]
struct FileInfo {
    path: PathBuf,
    format: ContainerFormat,
    audio_codec: Option<CodecName>,
    video_codec: Option<CodecName>,
}

impl nojson::DisplayJson for FileInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("path", &self.path)?;
            f.member("format", &self.format)?;
            if let Some(c) = self.audio_codec {
                f.member("audio_codec", c)?;
            }
            if let Some(c) = self.video_codec {
                f.member("video_codec", c)?;
            }
            Ok(())
        })
    }
}
