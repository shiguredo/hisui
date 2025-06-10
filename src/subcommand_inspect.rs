use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    metadata::{ContainerFormat, SourceId},
    reader::{AudioReader, VideoReader},
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    types::CodecName,
    video::{VideoFormat, VideoFrame},
    video_h264::H264AnnexBNalUnits,
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
    let mut audio_samples = Vec::new();
    for sample in audio_reader {
        let sample = sample.or_fail()?;
        if audio_codec.is_none() {
            audio_codec = sample.format.codec_name();
        }
        audio_samples.push(AudioSampleInfo {
            timestamp: sample.timestamp,
            duration: sample.duration,
            data_size: sample.data.len(),
        });
    }

    let mut video_codec = None;
    let mut video_samples = Vec::new();
    for sample in video_reader {
        let sample = sample.or_fail()?;
        if video_codec.is_none() {
            video_codec = sample.format.codec_name();
        }
        video_samples.push(VideoSampleInfo {
            timestamp: sample.timestamp,
            duration: sample.duration,
            data_size: sample.data.len(),
            keyframe: sample.keyframe,
            codec_specific_info: VideoCodecSpecificInfo::new(&sample),
        });
    }

    // 入力ファイルから取得した情報を出力する
    let info = FileInfo {
        path: input_file_path.as_ref().to_path_buf(),
        format,
        audio_codec,
        audio_samples,
        video_codec,
        video_samples,
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
    audio_samples: Vec<AudioSampleInfo>,
    video_codec: Option<CodecName>,
    video_samples: Vec<VideoSampleInfo>,
}

impl nojson::DisplayJson for FileInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("path", &self.path)?;
            f.member("format", &self.format)?;
            if let Some(c) = self.audio_codec {
                f.member("audio_codec", c)?;
                f.member(
                    "audio_duration_us",
                    self.audio_samples
                        .iter()
                        .map(|s| s.duration)
                        .sum::<Duration>()
                        .as_micros(),
                )?;
                f.member("audio_sample_count", self.audio_samples.len())?;
                f.member("audio_samples", &self.audio_samples)?;
            }
            if let Some(c) = self.video_codec {
                f.member("video_codec", c)?;
                f.member(
                    "video_duration_us",
                    self.video_samples
                        .iter()
                        .map(|s| s.duration)
                        .sum::<Duration>()
                        .as_micros(),
                )?;
                f.member("video_sample_count", self.video_samples.len())?;
                f.member(
                    "video_keyframe_sample_count",
                    self.video_samples.iter().filter(|s| s.keyframe).count(),
                )?;
                f.member("video_samples", &self.video_samples)?;
            }
            Ok(())
        })
    }
}

#[derive(Debug)]
struct AudioSampleInfo {
    timestamp: Duration,
    duration: Duration,
    data_size: usize,
}

impl nojson::DisplayJson for AudioSampleInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.set_indent_size(0);
        f.object(|f| {
            f.member("timestamp_us", self.timestamp.as_micros())?;
            f.member("duration_us", self.duration.as_micros())?;
            f.member("data_size", self.data_size)?;
            Ok(())
        })?;
        f.set_indent_size(2);
        Ok(())
    }
}

#[derive(Debug)]
struct VideoSampleInfo {
    timestamp: Duration,
    duration: Duration,
    data_size: usize,
    keyframe: bool,
    codec_specific_info: Option<VideoCodecSpecificInfo>,
}

impl nojson::DisplayJson for VideoSampleInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.set_indent_size(0);
        f.object(|f| {
            f.member("timestamp_us", self.timestamp.as_micros())?;
            f.member("duration_us", self.duration.as_micros())?;
            f.member("data_size", self.data_size)?;
            f.member("keyframe", self.keyframe)?;
            match &self.codec_specific_info {
                None => {}
                Some(VideoCodecSpecificInfo::H264 { nalus }) => {
                    f.member("nalus", nalus)?;
                }
            }
            Ok(())
        })?;
        f.set_indent_size(2);
        Ok(())
    }
}

#[derive(Debug)]
struct H264NalUnitInfo {
    ty: u8,
    nri: u8,
}

impl nojson::DisplayJson for H264NalUnitInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("type", self.ty)?;
            f.member("nri", self.nri)
        })
    }
}

#[derive(Debug)]
enum VideoCodecSpecificInfo {
    H264 { nalus: Vec<H264NalUnitInfo> },
}

impl VideoCodecSpecificInfo {
    fn new(sample: &VideoFrame) -> Option<Self> {
        match sample.format {
            VideoFormat::H264AnnexB => {
                let mut nalus = Vec::new();
                for nalu in H264AnnexBNalUnits::new(&sample.data) {
                    match nalu {
                        Ok(nalu) => {
                            let header_byte = nalu.data.first()?;
                            let nri = (header_byte >> 5) & 0b11;
                            nalus.push(H264NalUnitInfo { ty: nalu.ty, nri });
                        }
                        Err(_) => return None, // パースエラー
                    }
                }

                Some(VideoCodecSpecificInfo::H264 { nalus })
            }
            VideoFormat::H264 => {
                let mut nalus = Vec::new();
                let mut data = &sample.data[..];

                // NOTE: sora の場合は区切りバイトサイズは 4 に固定
                while data.len() > 4 {
                    let length = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
                    data = &data[4..];

                    if data.len() < length || length == 0 {
                        return None; // パースエラー
                    }

                    let header_byte = data[0];
                    let nalu_type = header_byte & 0b0001_1111;
                    let nri = (header_byte >> 5) & 0b11;

                    nalus.push(H264NalUnitInfo { ty: nalu_type, nri });

                    data = &data[length..];
                }

                Some(VideoCodecSpecificInfo::H264 { nalus })
            }
            _ => None,
        }
    }
}
