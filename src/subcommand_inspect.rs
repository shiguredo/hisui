use std::{path::PathBuf, time::Duration};

use crate::{
    channel::ErrorFlag,
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    media::MediaStreamId,
    metadata::{ContainerFormat, SourceId},
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec},
    reader::{AudioReader, VideoReader},
    reader_mp4::{Mp4AudioReader, Mp4VideoReader},
    reader_webm::{WebmAudioReader, WebmVideoReader},
    scheduler::Scheduler,
    stats::ProcessorStats,
    types::CodecName,
    video::{VideoFormat, VideoFrame},
    video_h264::H264AnnexBNalUnits,
};

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

const AUDIO_ENCODED_STREAM_ID: MediaStreamId = MediaStreamId::new(0);
const VIDEO_ENCODED_STREAM_ID: MediaStreamId = MediaStreamId::new(1);
const AUDIO_DECODED_STREAM_ID: MediaStreamId = MediaStreamId::new(2);
const VIDEO_DECODED_STREAM_ID: MediaStreamId = MediaStreamId::new(3);

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

    let error_flag = ErrorFlag::new();
    let mut scheduler = Scheduler::new(error_flag);
    let dummy_source_id = SourceId::new("inspect"); // 使われないのでなんでもいい

    let reader = match format {
        ContainerFormat::Mp4 => {
            let reader =
                Mp4AudioReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?;
            AudioReader::new_mp4(AUDIO_ENCODED_STREAM_ID, reader)
        }
        ContainerFormat::Webm => {
            let reader =
                WebmAudioReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?;
            AudioReader::new_webm(AUDIO_ENCODED_STREAM_ID, reader)
        }
    };
    scheduler.register(reader).or_fail()?;

    let reader = match format {
        ContainerFormat::Mp4 => {
            let reader =
                Mp4VideoReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?;
            VideoReader::new_mp4(VIDEO_ENCODED_STREAM_ID, reader)
        }
        ContainerFormat::Webm => {
            let reader =
                WebmVideoReader::new(dummy_source_id.clone(), &input_file_path).or_fail()?;
            VideoReader::new_webm(VIDEO_ENCODED_STREAM_ID, reader)
        }
    };
    scheduler.register(reader).or_fail()?;

    if decode {
        let decoder =
            AudioDecoder::new_opus(AUDIO_ENCODED_STREAM_ID, AUDIO_DECODED_STREAM_ID).or_fail()?;
        scheduler.register(decoder).or_fail()?;
    }

    if decode {
        let options = VideoDecoderOptions {
            openh264_lib: openh264
                .clone()
                .map(Openh264Library::load)
                .transpose()
                .or_fail()?,
        };
        let decoder = VideoDecoder::new(VIDEO_ENCODED_STREAM_ID, VIDEO_DECODED_STREAM_ID, options);
        scheduler.register(decoder).or_fail()?;
    }

    scheduler
        .register(OutputPrinter::new(input_file_path.clone(), format, decode))
        .or_fail()?;
    scheduler.run().or_fail()?;

    Ok(())
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

#[derive(Debug)]
pub struct OutputPrinter {
    path: PathBuf,
    format: ContainerFormat,
    audio_codec: Option<CodecName>,
    video_codec: Option<CodecName>,
    audio_samples: Vec<AudioSampleInfo>,
    video_samples: Vec<VideoSampleInfo>,
    input_stream_ids: Vec<MediaStreamId>,
    next_input_stream_index: usize,
}

impl OutputPrinter {
    fn new(path: PathBuf, format: ContainerFormat, decode: bool) -> Self {
        Self {
            path,
            format,
            audio_codec: None,
            video_codec: None,
            audio_samples: Vec::new(),
            video_samples: Vec::new(),
            input_stream_ids: if decode {
                vec![
                    AUDIO_ENCODED_STREAM_ID,
                    VIDEO_ENCODED_STREAM_ID,
                    AUDIO_DECODED_STREAM_ID,
                    VIDEO_DECODED_STREAM_ID,
                ]
            } else {
                vec![AUDIO_ENCODED_STREAM_ID, VIDEO_ENCODED_STREAM_ID]
            },
            next_input_stream_index: 0,
        }
    }
}

impl MediaProcessor for OutputPrinter {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self.input_stream_ids.clone(),
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::other("output-printer"),
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        let Some(sample) = input.sample else {
            self.input_stream_ids.retain(|id| *id != input.stream_id);
            self.next_input_stream_index = 0;
            return Ok(());
        };
        match input.stream_id {
            AUDIO_ENCODED_STREAM_ID => {
                let sample = sample.expect_audio_data().or_fail()?;
                if self.audio_codec.is_none() {
                    self.audio_codec = sample.format.codec_name();
                }
                self.audio_samples.push(AudioSampleInfo {
                    timestamp: sample.timestamp,
                    duration: sample.duration,
                    data_size: sample.data.len(),
                    decoded_data_size: None,
                });
            }
            AUDIO_DECODED_STREAM_ID => {
                let sample = sample.expect_audio_data().or_fail()?;
                let info = self
                    .audio_samples
                    .iter_mut()
                    .rfind(|s| s.decoded_data_size.is_none())
                    .or_fail()?;
                info.decoded_data_size = Some(sample.data.len());
            }
            VIDEO_ENCODED_STREAM_ID => {
                let sample = sample.expect_video_frame().or_fail()?;
                if self.video_codec.is_none() {
                    self.video_codec = sample.format.codec_name();
                }
                self.video_samples.push(VideoSampleInfo {
                    timestamp: sample.timestamp,
                    duration: sample.duration,
                    data_size: sample.data.len(),
                    keyframe: sample.keyframe,
                    codec_specific_info: VideoCodecSpecificInfo::new(&sample),
                    decoded_data_size: None,
                    width: None,
                    height: None,
                });
            }
            VIDEO_DECODED_STREAM_ID => {
                let sample = sample.expect_video_frame().or_fail()?;
                let info = self
                    .video_samples
                    .iter_mut()
                    .rfind(|s| s.decoded_data_size.is_none())
                    .or_fail()?;
                info.update(&sample);
            }
            _ => return Err(orfail::Failure::new("BUG: unexpected stream ID")),
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        if self.input_stream_ids.is_empty() {
            crate::json::pretty_print(self).or_fail()?;
            Ok(MediaProcessorOutput::Finished)
        } else {
            let awaiting_stream_id = self.input_stream_ids[self.next_input_stream_index];
            self.next_input_stream_index =
                (self.next_input_stream_index + 1) % self.input_stream_ids.len();
            Ok(MediaProcessorOutput::Pending { awaiting_stream_id })
        }
    }
}

impl nojson::DisplayJson for OutputPrinter {
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
