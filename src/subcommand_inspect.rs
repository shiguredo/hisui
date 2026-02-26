use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    time::Duration,
};

use crate::{
    Error, Result,
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    file_reader_mp4::{Mp4FileReader, Mp4FileReaderOptions},
    file_reader_webm::{WebmFileReader, WebmFileReaderOptions},
    types::{CodecName, ContainerFormat},
    video::{VideoFormat, VideoFrame},
    video_h264::H264AnnexBNalUnits,
};
use shiguredo_openh264::Openh264Library;

const AUDIO_ENCODED_TRACK_ID: &str = "audio_encoded";
const VIDEO_ENCODED_TRACK_ID: &str = "video_encoded";
const AUDIO_DECODED_TRACK_ID: &str = "audio_decoded";
const VIDEO_DECODED_TRACK_ID: &str = "video_decoded";

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

    run_internal(input_file_path, decode, openh264).map_err(noargs::Error::from)?;
    Ok(())
}

fn run_internal(input_file_path: PathBuf, decode: bool, openh264: Option<PathBuf>) -> Result<()> {
    let format = ContainerFormat::from_path(&input_file_path)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|e| Error::new(e.to_string()))?;

    let pipeline = crate::MediaPipeline::new()?;
    let pipeline_handle = pipeline.handle();
    runtime.spawn(async move {
        if let Err(e) =
            setup_pipeline(pipeline_handle, input_file_path, format, decode, openh264).await
        {
            tracing::error!("pipeline setup failed: {e:?}");
        }
    });

    runtime.block_on(pipeline.run());
    Ok(())
}

async fn setup_pipeline(
    pipeline_handle: crate::MediaPipelineHandle,
    input_file_path: PathBuf,
    format: ContainerFormat,
    decode: bool,
    openh264: Option<PathBuf>,
) -> Result<()> {
    let output_printer = OutputPrinter::new(input_file_path.clone(), format, decode);

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
                .spawn_processor(
                    crate::ProcessorId::new("mp4_file_reader"),
                    crate::ProcessorMetadata::new("mp4_file_reader"),
                    |handle| reader.run(handle),
                )
                .await?;
        }
        ContainerFormat::Webm => {
            let reader = WebmFileReader::new(
                input_file_path,
                WebmFileReaderOptions {
                    audio_track_id: Some(crate::TrackId::new(AUDIO_ENCODED_TRACK_ID)),
                    video_track_id: Some(crate::TrackId::new(VIDEO_ENCODED_TRACK_ID)),
                },
            );

            pipeline_handle
                .spawn_processor(
                    crate::ProcessorId::new("webm_file_reader"),
                    crate::ProcessorMetadata::new("webm_file_reader"),
                    |handle| reader.run(handle),
                )
                .await?;
        }
    }

    if decode {
        let openh264_lib = openh264
            .clone()
            .map(Openh264Library::load)
            .transpose()
            .map_err(|e| Error::new(e.to_string()))?;

        let audio_decoder = AudioDecoder::new(crate::stats::Stats::new())?;

        pipeline_handle
            .spawn_processor(
                crate::ProcessorId::new("audio_decoder"),
                crate::ProcessorMetadata::new("audio_decoder"),
                |handle| {
                    audio_decoder.run(
                        handle,
                        crate::TrackId::new(AUDIO_ENCODED_TRACK_ID),
                        crate::TrackId::new(AUDIO_DECODED_TRACK_ID),
                    )
                },
            )
            .await?;

        let video_decoder = VideoDecoder::new(
            VideoDecoderOptions {
                openh264_lib,
                decode_params: Default::default(),
                engines: None,
            },
            crate::stats::Stats::new(),
        );
        pipeline_handle
            .spawn_processor(
                crate::ProcessorId::new("video_decoder"),
                crate::ProcessorMetadata::new("video_decoder"),
                |handle| {
                    video_decoder.run(
                        handle,
                        crate::TrackId::new(VIDEO_ENCODED_TRACK_ID),
                        crate::TrackId::new(VIDEO_DECODED_TRACK_ID),
                    )
                },
            )
            .await?;
    }

    pipeline_handle
        .spawn_processor(
            crate::ProcessorId::new("output_printer"),
            crate::ProcessorMetadata::new("inspect_output_printer"),
            |handle| output_printer.run(handle),
        )
        .await?;

    pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;

    Ok(())
}

#[derive(Debug)]
struct AudioSampleInfo {
    timestamp: Duration,
    duration: Option<Duration>,
    data_size: usize,
    decoded_data_size: Option<usize>,
}

impl nojson::DisplayJson for AudioSampleInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.set_indent_size(0);
        f.object(|f| {
            f.member("timestamp_us", self.timestamp.as_micros())?;
            f.member("duration_us", self.duration.map(|v| v.as_micros() as u64))?;
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
    duration: Option<Duration>,
    data_size: usize,
    keyframe: bool,
    codec_specific_info: Option<VideoCodecSpecificInfo>,
    decoded_data_size: Option<usize>,
    width: Option<usize>,
    height: Option<usize>,
}

impl VideoSampleInfo {}

impl nojson::DisplayJson for VideoSampleInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.set_indent_size(0);
        f.object(|f| {
            f.member("timestamp_us", self.timestamp.as_micros())?;
            f.member("duration_us", self.duration.map(|v| v.as_micros() as u64))?;
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
    pending_audio_decoded_data_sizes: VecDeque<usize>,
    pending_video_decoded_infos: VecDeque<DecodedVideoInfo>,
    active_streams: HashSet<crate::TrackId>,
    audio_encoded_track_id: crate::TrackId,
    video_encoded_track_id: crate::TrackId,
    audio_decoded_track_id: crate::TrackId,
    video_decoded_track_id: crate::TrackId,
}

#[derive(Debug)]
struct DecodedVideoInfo {
    decoded_data_size: usize,
    width: usize,
    height: usize,
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
            pending_audio_decoded_data_sizes: VecDeque::new(),
            pending_video_decoded_infos: VecDeque::new(),
            active_streams,
            audio_encoded_track_id,
            video_encoded_track_id,
            audio_decoded_track_id,
            video_decoded_track_id,
        }
    }

    fn estimate_duration(prev_timestamp: Duration, next_timestamp: Duration) -> Option<Duration> {
        if next_timestamp > prev_timestamp {
            Some(next_timestamp.saturating_sub(prev_timestamp))
        } else {
            None
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

        handle.notify_ready();

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

        crate::json::pretty_print(&self)?;
        Ok(())
    }

    fn handle_audio_encoded_sample(&mut self, message: crate::Message) -> Result<()> {
        match message {
            crate::Message::Media(media_sample) => {
                let audio_data = match media_sample {
                    crate::MediaFrame::Audio(sample) => sample,
                    crate::MediaFrame::Video(_) => {
                        return Err(Error::new(
                            "expected an audio sample, but got a video sample",
                        ));
                    }
                };
                if self.audio_codec.is_none() {
                    self.audio_codec = audio_data.format.codec_name();
                }
                if let Some(prev) = self.audio_samples.last_mut() {
                    let duration = Self::estimate_duration(prev.timestamp, audio_data.timestamp);
                    prev.duration = duration;
                }
                self.audio_samples.push(AudioSampleInfo {
                    timestamp: audio_data.timestamp,
                    duration: None,
                    data_size: audio_data.data.len(),
                    decoded_data_size: None,
                });
                self.try_apply_pending_audio_decoded_data_sizes();
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
                    crate::MediaFrame::Video(sample) => sample,
                    crate::MediaFrame::Audio(_) => {
                        return Err(Error::new(
                            "expected a video sample, but got an audio sample",
                        ));
                    }
                };
                if self.video_codec.is_none() {
                    self.video_codec = video_frame.format.codec_name();
                }
                if let Some(prev) = self.video_samples.last_mut() {
                    let duration = Self::estimate_duration(prev.timestamp, video_frame.timestamp);
                    prev.duration = duration;
                }
                self.video_samples.push(VideoSampleInfo {
                    timestamp: video_frame.timestamp,
                    duration: None,
                    data_size: video_frame.data.len(),
                    keyframe: video_frame.keyframe,
                    codec_specific_info: VideoCodecSpecificInfo::new(&video_frame),
                    decoded_data_size: None,
                    width: None,
                    height: None,
                });
                self.try_apply_pending_video_decoded_infos();
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
                    crate::MediaFrame::Audio(sample) => sample,
                    crate::MediaFrame::Video(_) => {
                        return Err(Error::new(
                            "expected an audio sample, but got a video sample",
                        ));
                    }
                };
                self.pending_audio_decoded_data_sizes
                    .push_back(audio_data.data.len());
                self.try_apply_pending_audio_decoded_data_sizes();
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
                    crate::MediaFrame::Video(sample) => sample,
                    crate::MediaFrame::Audio(_) => {
                        return Err(Error::new(
                            "expected a video sample, but got an audio sample",
                        ));
                    }
                };
                self.pending_video_decoded_infos
                    .push_back(DecodedVideoInfo {
                        decoded_data_size: video_frame.data.len(),
                        width: video_frame.width,
                        height: video_frame.height,
                    });
                self.try_apply_pending_video_decoded_infos();
            }
            crate::Message::Eos => {
                self.active_streams.remove(&self.video_decoded_track_id);
            }
            crate::Message::Syn(_) => {}
        }
        Ok(())
    }

    fn try_apply_pending_audio_decoded_data_sizes(&mut self) {
        while let Some(decoded_data_size) = self.pending_audio_decoded_data_sizes.pop_front() {
            let Some(info) = self
                .audio_samples
                .iter_mut()
                .find(|s| s.decoded_data_size.is_none())
            else {
                self.pending_audio_decoded_data_sizes
                    .push_front(decoded_data_size);
                break;
            };
            info.decoded_data_size = Some(decoded_data_size);
        }
    }

    fn try_apply_pending_video_decoded_infos(&mut self) {
        while let Some(decoded_info) = self.pending_video_decoded_infos.pop_front() {
            let Some(info) = self
                .video_samples
                .iter_mut()
                .find(|s| s.decoded_data_size.is_none())
            else {
                self.pending_video_decoded_infos.push_front(decoded_info);
                break;
            };

            info.decoded_data_size = Some(decoded_info.decoded_data_size);
            info.width = Some(decoded_info.width);
            info.height = Some(decoded_info.height);
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
                // 末尾サンプルの duration は次サンプルとの差分で算出できないため None のままにする。
                // そのため合計 duration は filter_map で None を除外して集計する
                // （最後のサンプル分は含まれない）。
                f.member(
                    "audio_duration_us",
                    self.audio_samples
                        .iter()
                        .filter_map(|s| s.duration)
                        .sum::<Duration>()
                        .as_micros(),
                )?;
                f.member("audio_sample_count", self.audio_samples.len())?;
                f.member("audio_samples", &self.audio_samples)?;
            }
            if let Some(c) = self.video_codec {
                f.member("video_codec", c)?;
                // 末尾サンプルの duration は次サンプルとの差分で算出できないため None のままにする。
                // そのため合計 duration は filter_map で None を除外して集計する
                // （最後のサンプル分は含まれない）。
                f.member(
                    "video_duration_us",
                    self.video_samples
                        .iter()
                        .filter_map(|s| s.duration)
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
