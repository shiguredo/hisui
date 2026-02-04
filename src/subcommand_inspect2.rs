use std::{path::PathBuf, time::Duration};

use orfail::OrFail;

use crate::{
    media::{MediaSample, MediaStreamId},
    metadata::{ContainerFormat, SourceId},
    reader::{AudioReader, VideoReader},
    types::CodecName,
    video::{VideoFormat, VideoFrame},
    video_h264::H264AnnexBNalUnits,
};

const AUDIO_ENCODED_STREAM_ID: MediaStreamId = MediaStreamId::new(0);
const VIDEO_ENCODED_STREAM_ID: MediaStreamId = MediaStreamId::new(1);

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let _openh264: Option<PathBuf> = noargs::opt("openh264")
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

    let format = ContainerFormat::from_path(&input_file_path).or_fail()?;
    let dummy_source_id = SourceId::new("inspect");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .or_fail()?;
    let _guard = runtime.enter();
    let manager = crate::processor_async::ProcessorManager::new();
    let manager_handler = manager.start();

    // OutputPrinter を spawn
    let output_printer = OutputPrinter::new(input_file_path.clone(), format);
    runtime.spawn(output_printer.start(manager_handler.clone()));

    let reader = AudioReader::new(
        AUDIO_ENCODED_STREAM_ID,
        dummy_source_id.clone(),
        format,
        Duration::ZERO,
        vec![input_file_path.clone()],
    )
    .or_fail()?;
    runtime.spawn(reader.start(manager_handler.clone()));

    let reader = VideoReader::new(
        VIDEO_ENCODED_STREAM_ID,
        dummy_source_id.clone(),
        format,
        Duration::ZERO,
        vec![input_file_path.clone()],
    )
    .or_fail()?;
    runtime.spawn(reader.start(manager_handler.clone()));

    runtime.block_on(manager_handler.wait_finish());

    Ok(())
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
                        Err(_) => return None,
                    }
                }

                Some(VideoCodecSpecificInfo::H264 { nalus })
            }
            VideoFormat::H264 => {
                let mut nalus = Vec::new();
                let mut data = &sample.data[..];

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
    active_streams: std::collections::HashSet<MediaStreamId>,
}

impl OutputPrinter {
    fn new(path: PathBuf, format: ContainerFormat) -> Self {
        Self {
            path,
            format,
            audio_codec: None,
            video_codec: None,
            audio_samples: Vec::new(),
            video_samples: Vec::new(),
            active_streams: [AUDIO_ENCODED_STREAM_ID, VIDEO_ENCODED_STREAM_ID]
                .iter()
                .copied()
                .collect(),
        }
    }

    pub async fn start(
        mut self,
        handle: crate::processor_async::ProcessorManagerHandle,
    ) -> orfail::Result<()> {
        let id = crate::processor_async::ProcessorId::new("output_printer");
        let processor_handle = handle.register_processor(id.clone()).await.or_fail()?;

        let audio_track_id =
            crate::processor_async::TrackId::new(AUDIO_ENCODED_STREAM_ID.get().to_string());
        let mut audio_track = processor_handle.subscribe_track(audio_track_id).await;

        let video_track_id =
            crate::processor_async::TrackId::new(VIDEO_ENCODED_STREAM_ID.get().to_string());
        let mut video_track = processor_handle.subscribe_track(video_track_id).await;

        let mut audio_finished = false;
        let mut video_finished = false;

        loop {
            if audio_finished && video_finished {
                break;
            }

            tokio::select! {
                sample = async {
                    if !audio_finished {
                         audio_track.recv_media().await
                    } else {
                        std::future::pending::<Option<MediaSample>>().await
                    }
                } => {
                    if sample.is_none() {
                        audio_finished = true;
                    }
                    self.handle_audio_sample(sample)?;
                }
                sample = async {
                    if !video_finished {
                        video_track.recv_media().await
                    } else {
                        std::future::pending::<Option<MediaSample>>().await
                    }
                } => {
                    if sample.is_none() {
                        video_finished = true;
                    }
                    self.handle_video_sample(sample)?;
                }
            }
        }

        crate::json::pretty_print(&self).or_fail()?;
        Ok(())
    }

    fn handle_audio_sample(&mut self, sample: Option<MediaSample>) -> orfail::Result<()> {
        match sample {
            Some(media_sample) => {
                let audio_data = media_sample.expect_audio_data().or_fail()?;
                if self.audio_codec.is_none() {
                    self.audio_codec = audio_data.format.codec_name();
                }
                self.audio_samples.push(AudioSampleInfo {
                    timestamp: audio_data.timestamp,
                    duration: audio_data.duration,
                    data_size: audio_data.data.len(),
                });
            }
            None => {
                self.active_streams.remove(&AUDIO_ENCODED_STREAM_ID);
            }
        }
        Ok(())
    }

    fn handle_video_sample(&mut self, sample: Option<MediaSample>) -> orfail::Result<()> {
        match sample {
            Some(media_sample) => {
                let video_frame = media_sample.expect_video_frame().or_fail()?;
                if self.video_codec.is_none() {
                    self.video_codec = video_frame.format.codec_name();
                }
                self.video_samples.push(VideoSampleInfo {
                    timestamp: video_frame.timestamp,
                    duration: video_frame.duration,
                    data_size: video_frame.data.len(),
                    keyframe: video_frame.keyframe,
                    codec_specific_info: VideoCodecSpecificInfo::new(&video_frame),
                });
            }
            None => {
                self.active_streams.remove(&VIDEO_ENCODED_STREAM_ID);
            }
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
