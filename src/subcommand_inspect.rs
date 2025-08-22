use std::{path::PathBuf, time::Duration};

use crate::{
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    media::MediaStreamId,
    metadata::{ContainerFormat, SourceId},
    reader::{AudioReader, VideoReader},
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    scheduler::Scheduler,
    types::CodecName,
    video::{VideoFormat, VideoFrame},
    video_h264::H264AnnexBNalUnits,
};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let decode: bool = noargs::flag("decode")
        .doc("指定された場合にはデコードまで行います")
        .take(&mut args)
        .is_present();
    let openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパス")
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let input_file_path: PathBuf = noargs::arg("INPUT_FILE")
        .example("/path/to/archive.mp4")
        .doc("情報取得対象の録画ファイル(.mp4|.webm)")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    let format = match input_file_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .as_ref()
    {
        "mp4" => ContainerFormat::Mp4,
        "webm" => ContainerFormat::Webm,
        ext => {
            return Err(
                orfail::Failure::new(format!("unsupported container format: {ext}")).into(),
            );
        }
    };

    let mut scheduler = Scheduler::new();

    let dummy_source_id = SourceId::new("inspect"); // 使われないのでなんでもいい

    let audio_encoded_stream_id = MediaStreamId::new(0);
    let reader = match format {
        ContainerFormat::Mp4 => {
            let reader =
                Mp4AudioReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?;
            AudioReader::new_mp4(audio_encoded_stream_id, reader)
        }
        ContainerFormat::Webm => {
            let reader =
                WebmAudioReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?;
            AudioReader::new_webm(audio_encoded_stream_id, reader)
        }
    };
    scheduler.register(reader).or_fail()?;

    let video_encoded_stream_id = MediaStreamId::new(1);
    let reader = match format {
        ContainerFormat::Mp4 => {
            let reader =
                Mp4VideoReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?;
            VideoReader::new_mp4(video_encoded_stream_id, reader)
        }
        ContainerFormat::Webm => {
            let reader =
                WebmVideoReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?;
            VideoReader::new_webm(video_encoded_stream_id, reader)
        }
    };
    scheduler.register(reader).or_fail()?;

    let audio_decoded_stream_id = MediaStreamId::new(2);
    let video_decoded_stream_id = MediaStreamId::new(3);

    let audio_stream_id = MediaStreamId::new(0);
    let video_stream_id = MediaStreamId::new(1);
    let (audio_reader, video_reader): (Box<dyn Iterator<Item = _>>, Box<dyn Iterator<Item = _>>) =
        match format {
            ContainerFormat::Webm => {
                let audio = Box::new(
                    WebmAudioReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?,
                );
                let video = Box::new(
                    WebmVideoReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?,
                );
                (audio, video)
            }
            ContainerFormat::Mp4 => {
                let audio = Box::new(
                    Mp4AudioReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?,
                );
                let video = Box::new(
                    Mp4VideoReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?,
                );
                (audio, video)
            }
        };

    let decoded_audio_stream_id = MediaStreamId::new(2);
    let mut audio_codec = None;
    let mut audio_samples = Vec::new();
    let mut audio_decoder = None;
    for sample in audio_reader {
        let sample = sample.or_fail()?;
        if audio_codec.is_none() {
            audio_codec = sample.format.codec_name();
            if decode {
                audio_decoder = Some(
                    AudioDecoder::new_opus(audio_stream_id, decoded_audio_stream_id).or_fail()?,
                );
            }
        }

        let mut info = AudioSampleInfo {
            timestamp: sample.timestamp,
            duration: sample.duration,
            data_size: sample.data.len(),
            decoded_data_size: None,
        };

        if let Some(decoder) = &mut audio_decoder {
            let decoded = decoder.decode(sample).or_fail()?;
            info.decoded_data_size = Some(decoded.data.len());
        }

        audio_samples.push(info);
    }

    let decoded_video_stream_id = MediaStreamId::new(3);
    let mut video_codec = None;
    let mut video_samples = Vec::new();
    let mut video_decoder = None;
    let mut decoded_count = 0;
    for sample in video_reader {
        let sample = sample.or_fail()?;
        if video_codec.is_none() {
            video_codec = sample.format.codec_name();
            if decode {
                let options = VideoDecoderOptions {
                    openh264_lib: openh264
                        .clone()
                        .map(Openh264Library::load)
                        .transpose()
                        .or_fail()?,
                };
                video_decoder = Some(VideoDecoder::new(
                    video_stream_id,
                    decoded_video_stream_id,
                    options,
                ));
            }
        }

        video_samples.push(VideoSampleInfo {
            timestamp: sample.timestamp,
            duration: sample.duration,
            data_size: sample.data.len(),
            keyframe: sample.keyframe,
            codec_specific_info: VideoCodecSpecificInfo::new(&sample),
            decoded_data_size: None,
            width: None,
            height: None,
        });

        if let Some(decoder) = &mut video_decoder {
            decoder.decode(sample).or_fail()?;
            while let Some(decoded) = decoder.next_decoded_frame() {
                video_samples[decoded_count].update(&decoded);
                decoded_count += 1;
            }
        }
    }
    if let Some(decoder) = &mut video_decoder {
        decoder.finish().or_fail()?;
        while let Some(decoded) = decoder.next_decoded_frame() {
            video_samples[decoded_count].update(&decoded);
            decoded_count += 1;
        }
    }

    // 入力ファイルから取得した情報を出力する
    crate::json::pretty_print(FileInfo {
        path: input_file_path.to_path_buf(),
        format,
        audio_codec,
        audio_samples,
        video_codec,
        video_samples,
    })
    .or_fail()?;

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
            f.member("format", self.format)?;
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
    decoded_data_size: Option<usize>,
}

impl nojson::DisplayJson for AudioSampleInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.set_indent_size(0);
        f.object(|f| {
            f.member("timestamp_us", self.timestamp.as_micros())?;
            f.member("duration_us", self.duration.as_micros())?;
            f.member("data_size", self.data_size)?;
            if let Some(v) = self.decoded_data_size {
                f.member("decoded_data_size", v)?;
            }
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
    decoded_data_size: Option<usize>,
    width: Option<usize>,
    height: Option<usize>,
}

impl VideoSampleInfo {
    fn update(&mut self, decoded: &VideoFrame) {
        self.decoded_data_size = Some(decoded.data.len());
        self.width = Some(decoded.width.get());
        self.height = Some(decoded.height.get());
    }
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
            if let Some(v) = self.decoded_data_size {
                f.member("decoded_data_size", v)?;
            }
            if let Some(v) = self.width {
                f.member("width", v)?;
            }
            if let Some(v) = self.height {
                f.member("height", v)?;
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
