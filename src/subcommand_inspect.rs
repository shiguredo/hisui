use std::{collections::HashSet, path::PathBuf, time::Duration};

use crate::{
    Error, Result,
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    file_reader_mp4::{Mp4FileReader, Mp4FileReaderOptions},
    file_reader_webm::{WebmFileReader, WebmFileReaderOptions},
    metadata::ContainerFormat,
    types::CodecName,
    video::{VideoFormat, VideoFrame},
    video_h264::H264AnnexBNalUnits,
};
use shiguredo_openh264::Openh264Library;

const AUDIO_ENCODED_TRACK_ID: &str = "audio_encoded";
const VIDEO_ENCODED_TRACK_ID: &str = "video_encoded";
const AUDIO_DECODED_TRACK_ID: &str = "audio_decoded";
const VIDEO_DECODED_TRACK_ID: &str = "video_decoded";

const AUDIO_DECODER_INPUT_STREAM_ID: crate::media::MediaStreamId =
    crate::media::MediaStreamId::new(0);
const AUDIO_DECODER_OUTPUT_STREAM_ID: crate::media::MediaStreamId =
    crate::media::MediaStreamId::new(1);
const VIDEO_DECODER_INPUT_STREAM_ID: crate::media::MediaStreamId =
    crate::media::MediaStreamId::new(2);
const VIDEO_DECODER_OUTPUT_STREAM_ID: crate::media::MediaStreamId =
    crate::media::MediaStreamId::new(3);

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

    run_internal(input_file_path, decode, openh264)?;
    Ok(())
}

fn run_internal(input_file_path: PathBuf, decode: bool, openh264: Option<PathBuf>) -> Result<()> {
    let format =
        ContainerFormat::from_path(&input_file_path).map_err(|e| Error::new(e.to_string()))?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|e| Error::new(e.to_string()))?;

    runtime.block_on(async move {
        let pipeline = crate::MediaPipeline::new()?;
        let pipeline_handle = pipeline.handle();

        let output_printer = OutputPrinter::new(input_file_path.clone(), format, decode);
        pipeline_handle
            .spawn_processor(crate::ProcessorId::new("output_printer"), |handle| {
                let output_printer = output_printer;
                async move { output_printer.run(handle).await }
            })
            .await
            .map_err(|e| Error::new(e.to_string()))?;

        if decode {
            let openh264_lib = openh264
                .clone()
                .map(Openh264Library::load)
                .transpose()
                .map_err(|e| Error::new(e.to_string()))?;
            let audio_decoder = AudioDecoder::new(
                AUDIO_DECODER_INPUT_STREAM_ID,
                AUDIO_DECODER_OUTPUT_STREAM_ID,
            )
            .map_err(|e| Error::new(e.to_string()))?;
            pipeline_handle
                .spawn_processor(crate::ProcessorId::new("audio_decoder"), |handle| {
                    audio_decoder.run(
                        handle,
                        crate::TrackId::new(AUDIO_ENCODED_TRACK_ID),
                        crate::TrackId::new(AUDIO_DECODED_TRACK_ID),
                    )
                })
                .await
                .map_err(|e| Error::new(e.to_string()))?;

            let video_decoder = VideoDecoder::new(
                VIDEO_DECODER_INPUT_STREAM_ID,
                VIDEO_DECODER_OUTPUT_STREAM_ID,
                VideoDecoderOptions {
                    openh264_lib,
                    decode_params: Default::default(),
                    engines: None,
                },
            );
            pipeline_handle
                .spawn_processor(crate::ProcessorId::new("video_decoder"), |handle| {
                    video_decoder.run(
                        handle,
                        crate::TrackId::new(VIDEO_ENCODED_TRACK_ID),
                        crate::TrackId::new(VIDEO_DECODED_TRACK_ID),
                    )
                })
                .await
                .map_err(|e| Error::new(e.to_string()))?;
        }

        match format {
            ContainerFormat::Mp4 => {
                let reader = Mp4FileReader::new(
                    input_file_path,
                    Mp4FileReaderOptions {
                        realtime: false,
                        loop_playback: false,
                        audio_track_id: Some(crate::TrackId::new(AUDIO_ENCODED_TRACK_ID)),
                        video_track_id: Some(crate::TrackId::new(VIDEO_ENCODED_TRACK_ID)),
                    },
                )?;
                pipeline_handle
                    .spawn_processor(crate::ProcessorId::new("mp4_file_reader"), |handle| {
                        let reader = reader;
                        async move { reader.run(handle).await }
                    })
                    .await
                    .map_err(|e| Error::new(e.to_string()))?;
            }
            ContainerFormat::Webm => {
                let reader = WebmFileReader::new(
                    input_file_path,
                    WebmFileReaderOptions {
                        realtime: false,
                        loop_playback: false,
                        audio_track_id: Some(crate::TrackId::new(AUDIO_ENCODED_TRACK_ID)),
                        video_track_id: Some(crate::TrackId::new(VIDEO_ENCODED_TRACK_ID)),
                    },
                );
                pipeline_handle
                    .spawn_processor(crate::ProcessorId::new("webm_file_reader"), |handle| {
                        let reader = reader;
                        async move { reader.run(handle).await }
                    })
                    .await
                    .map_err(|e| Error::new(e.to_string()))?;
            }
        }

        drop(pipeline_handle);
        pipeline.run().await;
        Ok(())
    })
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
        self.width = Some(decoded.width);
        self.height = Some(decoded.height);
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
                        Err(_) => return None,
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
                        return None;
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
    active_streams: HashSet<crate::TrackId>,
    audio_encoded_track_id: crate::TrackId,
    video_encoded_track_id: crate::TrackId,
    audio_decoded_track_id: crate::TrackId,
    video_decoded_track_id: crate::TrackId,
}

impl OutputPrinter {
    fn new(path: PathBuf, format: ContainerFormat, decode: bool) -> Self {
        let audio_encoded_track_id = crate::TrackId::new(AUDIO_ENCODED_TRACK_ID);
        let video_encoded_track_id = crate::TrackId::new(VIDEO_ENCODED_TRACK_ID);
        let audio_decoded_track_id = crate::TrackId::new(AUDIO_DECODED_TRACK_ID);
        let video_decoded_track_id = crate::TrackId::new(VIDEO_DECODED_TRACK_ID);

        let mut active_streams: HashSet<crate::TrackId> = [
            audio_encoded_track_id.clone(),
            video_encoded_track_id.clone(),
        ]
        .into_iter()
        .collect();
        if decode {
            active_streams.insert(audio_decoded_track_id.clone());
            active_streams.insert(video_decoded_track_id.clone());
        }

        Self {
            path,
            format,
            audio_codec: None,
            video_codec: None,
            audio_samples: Vec::new(),
            video_samples: Vec::new(),
            active_streams,
            audio_encoded_track_id,
            video_encoded_track_id,
            audio_decoded_track_id,
            video_decoded_track_id,
        }
    }

    async fn run(mut self, handle: crate::ProcessorHandle) -> Result<()> {
        let audio_encoded_track_id = self.audio_encoded_track_id.clone();
        let mut audio_encoded_track = handle.subscribe_track(audio_encoded_track_id.clone());

        let video_encoded_track_id = self.video_encoded_track_id.clone();
        let mut video_encoded_track = handle.subscribe_track(video_encoded_track_id.clone());

        let audio_decoded_track_id = self.audio_decoded_track_id.clone();
        let mut audio_decoded_track = handle.subscribe_track(audio_decoded_track_id.clone());

        let video_decoded_track_id = self.video_decoded_track_id.clone();
        let mut video_decoded_track = handle.subscribe_track(video_decoded_track_id.clone());

        while !self.active_streams.is_empty() {
            tokio::select! {
                message = audio_encoded_track.recv(),
                          if self.active_streams.contains(&audio_encoded_track_id) => {
                    self.handle_audio_encoded_sample(message)?;
                }
                message = video_encoded_track.recv(),
                          if self.active_streams.contains(&video_encoded_track_id) => {
                    self.handle_video_encoded_sample(message)?;
                }
                message = audio_decoded_track.recv(),
                          if self.active_streams.contains(&audio_decoded_track_id) => {
                    self.handle_audio_decoded_sample(message)?;
                }
                message = video_decoded_track.recv(),
                          if self.active_streams.contains(&video_decoded_track_id) => {
                    self.handle_video_decoded_sample(message)?;
                }
            }
        }

        crate::json::pretty_print(&self).map_err(|e| Error::new(e.to_string()))?;
        Ok(())
    }

    fn handle_audio_encoded_sample(&mut self, message: crate::Message) -> Result<()> {
        match message {
            crate::Message::Media(media_sample) => {
                let audio_data = match media_sample {
                    crate::MediaSample::Audio(sample) => sample,
                    crate::MediaSample::Video(_) => {
                        return Err(Error::new(
                            "expected an audio sample, but got a video sample",
                        ));
                    }
                };
                if self.audio_codec.is_none() {
                    self.audio_codec = audio_data.format.codec_name();
                }
                self.audio_samples.push(AudioSampleInfo {
                    timestamp: audio_data.timestamp,
                    duration: audio_data.duration,
                    data_size: audio_data.data.len(),
                    decoded_data_size: None,
                });
            }
            crate::Message::Eos => {
                self.active_streams.remove(&self.audio_encoded_track_id);
            }
            crate::Message::Syn(_) => {}
        }
        Ok(())
    }

    fn handle_video_encoded_sample(&mut self, message: crate::Message) -> Result<()> {
        match message {
            crate::Message::Media(media_sample) => {
                let video_frame = match media_sample {
                    crate::MediaSample::Video(sample) => sample,
                    crate::MediaSample::Audio(_) => {
                        return Err(Error::new(
                            "expected a video sample, but got an audio sample",
                        ));
                    }
                };
                if self.video_codec.is_none() {
                    self.video_codec = video_frame.format.codec_name();
                }
                self.video_samples.push(VideoSampleInfo {
                    timestamp: video_frame.timestamp,
                    duration: video_frame.duration,
                    data_size: video_frame.data.len(),
                    keyframe: video_frame.keyframe,
                    codec_specific_info: VideoCodecSpecificInfo::new(&video_frame),
                    decoded_data_size: None,
                    width: None,
                    height: None,
                });
            }
            crate::Message::Eos => {
                self.active_streams.remove(&self.video_encoded_track_id);
            }
            crate::Message::Syn(_) => {}
        }
        Ok(())
    }

    fn handle_audio_decoded_sample(&mut self, message: crate::Message) -> Result<()> {
        match message {
            crate::Message::Media(media_sample) => {
                let audio_data = match media_sample {
                    crate::MediaSample::Audio(sample) => sample,
                    crate::MediaSample::Video(_) => {
                        return Err(Error::new(
                            "expected an audio sample, but got a video sample",
                        ));
                    }
                };
                let info = self
                    .audio_samples
                    .iter_mut()
                    .rfind(|s| s.decoded_data_size.is_none())
                    .ok_or_else(|| Error::new("no undecoded audio sample found"))?;
                info.decoded_data_size = Some(audio_data.data.len());
            }
            crate::Message::Eos => {
                self.active_streams.remove(&self.audio_decoded_track_id);
            }
            crate::Message::Syn(_) => {}
        }
        Ok(())
    }

    fn handle_video_decoded_sample(&mut self, message: crate::Message) -> Result<()> {
        match message {
            crate::Message::Media(media_sample) => {
                let video_frame = match media_sample {
                    crate::MediaSample::Video(sample) => sample,
                    crate::MediaSample::Audio(_) => {
                        return Err(Error::new(
                            "expected a video sample, but got an audio sample",
                        ));
                    }
                };
                let info = self
                    .video_samples
                    .iter_mut()
                    .rfind(|s| s.decoded_data_size.is_none())
                    .ok_or_else(|| Error::new("no undecoded video sample found"))?;
                info.update(&video_frame);
            }
            crate::Message::Eos => {
                self.active_streams.remove(&self.video_decoded_track_id);
            }
            crate::Message::Syn(_) => {}
        }
        Ok(())
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
