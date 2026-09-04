use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    time::Duration,
};

use shiguredo_mp4::boxes::SampleEntry;
use shiguredo_mp4::demux::Mp4FileKind;

use crate::{
    Error, Result,
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    mp4::file_kind::detect_mp4_file_kind,
    mp4::sample_reader::{Mp4SampleReader, Mp4SampleReaderOptions},
    types::{CodecName, ContainerFormat},
    video::h264::{H264AnnexBNalUnits, NALU_HEADER_LENGTH},
    video::{VideoFormat, VideoFrame},
};
use shiguredo_openh264::Openh264Library;

const AUDIO_ENCODED_TRACK_ID: &str = "audio_encoded";
const VIDEO_ENCODED_TRACK_ID: &str = "video_encoded";
const AUDIO_DECODED_TRACK_ID: &str = "audio_decoded";
const VIDEO_DECODED_TRACK_ID: &str = "video_decoded";

pub fn try_run(args: &mut noargs::RawArgs, stats: crate::stats::Stats) -> noargs::Result<bool> {
    if !noargs::cmd("inspect")
        .doc("MP4 ファイルの情報を取得します")
        .take(args)
        .is_present()
    {
        return Ok(false);
    }
    run(args, stats)?;
    Ok(true)
}

fn run(args: &mut noargs::RawArgs, stats: crate::stats::Stats) -> noargs::Result<()> {
    let decode: bool = noargs::flag("decode")
        .doc("指定された場合にはデコードまで行います")
        .take(args)
        .is_present();
    let openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパス")
        .take(args)
        .present_and_then(|a| a.value().parse())?;
    #[cfg(feature = "fdk-aac")]
    let fdk_aac: Option<PathBuf> = noargs::opt("fdk-aac")
        .ty("PATH")
        .env("HISUI_FDK_AAC_PATH")
        .doc("FDK-AAC の共有ライブラリのパス")
        .take(args)
        .present_and_then(|a| a.value().parse())?;
    let input_file_path: PathBuf = noargs::arg("INPUT_FILE")
        .example("/path/to/archive.mp4")
        .doc("情報取得対象の MP4 ファイル (.mp4)")
        .take(args)
        .then(|a| a.value().parse())?;

    if args.metadata().help_mode {
        return Ok(());
    }

    run_internal(
        input_file_path,
        decode,
        openh264,
        #[cfg(feature = "fdk-aac")]
        fdk_aac,
        stats,
    )
    .map_err(noargs::Error::from)?;
    Ok(())
}

/// 入力ファイルのコンテナー形式を判定する
///
/// 拡張子で `Mp4` を判定したうえで、ファイル実体 (ftyp / moov) を見て
/// fragmented MP4 なら `Fmp4` に補正する。
/// 破損ファイル等で判定に失敗した場合はエラーを伝播し、`Mp4` へはフォールバックしない
/// (後段の reader 初期化でも同じ判定で失敗するため情報は失われない)。
fn detect_container_format(path: &Path) -> Result<ContainerFormat> {
    let format = ContainerFormat::from_path(path)?;
    if format == ContainerFormat::Mp4 && detect_mp4_file_kind(path)? == Mp4FileKind::FragmentedMp4 {
        return Ok(ContainerFormat::Fmp4);
    }
    Ok(format)
}

fn run_internal(
    input_file_path: PathBuf,
    decode: bool,
    openh264: Option<PathBuf>,
    #[cfg(feature = "fdk-aac")] fdk_aac: Option<PathBuf>,
    stats: crate::stats::Stats,
) -> Result<()> {
    let format = detect_container_format(&input_file_path)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|e| Error::new(e.to_string()))?;

    let pipeline = crate::MediaPipeline::new(Default::default(), stats)?;
    let pipeline_handle = pipeline.handle();
    runtime.spawn(async move {
        if let Err(e) = setup_pipeline(
            pipeline_handle,
            input_file_path,
            format,
            decode,
            openh264,
            #[cfg(feature = "fdk-aac")]
            fdk_aac,
        )
        .await
        {
            tracing::error!("pipeline setup failed: {e:?}");
        }
    });

    let processor_failed = runtime.block_on(pipeline.run());

    // いずれかの processor が異常終了していた場合は、非ゼロ終了コードになるようエラーを返す
    if processor_failed {
        return Err(Error::new(
            "inspect failed: one or more processors terminated abnormally",
        ));
    }
    Ok(())
}

async fn setup_pipeline(
    pipeline_handle: crate::MediaPipelineHandle,
    input_file_path: PathBuf,
    format: ContainerFormat,
    decode: bool,
    openh264: Option<PathBuf>,
    #[cfg(feature = "fdk-aac")] fdk_aac: Option<PathBuf>,
) -> Result<()> {
    let output_printer = OutputPrinter::new(input_file_path.clone(), format, decode);

    match format {
        // fMP4 も通常 MP4 と同じ reader で扱える
        ContainerFormat::Mp4 | ContainerFormat::Fmp4 => {
            let reader = Mp4SampleReader::new(
                input_file_path,
                Mp4SampleReaderOptions {
                    audio_track_id: Some(crate::TrackId::new(AUDIO_ENCODED_TRACK_ID)),
                    video_track_id: Some(crate::TrackId::new(VIDEO_ENCODED_TRACK_ID)),
                },
            );

            pipeline_handle
                .spawn_processor(
                    crate::ProcessorId::new("mp4_file_reader"),
                    crate::ProcessorMetadata::new("mp4_file_reader"),
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
        #[cfg(feature = "fdk-aac")]
        let fdk_aac_lib = fdk_aac
            .map(shiguredo_fdk_aac::FdkAacLibrary::load)
            .transpose()?;

        let audio_decoder = AudioDecoder::new(
            #[cfg(feature = "fdk-aac")]
            fdk_aac_lib,
            crate::stats::Stats::new(),
        )?;

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
                // AVCC 形式では NAL 長フィールドのバイト数は対応トラックの avcC の
                // lengthSizeMinusOne (0〜3 = 1〜4 バイト) で決まる。
                // サンプルエントリーから取得し、取得できない場合は 4 バイト固定にフォールバックする。
                let length_size = sample
                    .sample_entry
                    .as_ref()
                    .and_then(|entry| match entry.get() {
                        SampleEntry::Avc1(avc1) => {
                            Some(avc1.avcc_box.length_size_minus_one.get() as usize + 1)
                        }
                        _ => None,
                    })
                    .unwrap_or(NALU_HEADER_LENGTH);

                let mut nalus = Vec::new();
                let mut data = &sample.data[..];

                while data.len() > length_size {
                    let length = read_nal_length(data, length_size)?;
                    data = &data[length_size..];

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

/// AVCC 形式の先頭 `length_size` バイトを big-endian の NAL 長として読み取る
///
/// `length_size` は 1〜4 を想定する (avcC の `lengthSizeMinusOne + 1`)。
/// 長フィールドが `length_size` バイトに満たない場合は `None` を返す。
fn read_nal_length(data: &[u8], length_size: usize) -> Option<usize> {
    if data.len() < length_size {
        return None;
    }
    let mut length = 0usize;
    for &byte in &data[..length_size] {
        length = (length << 8) | byte as usize;
    }
    Some(length)
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
    /// デコード出力側の timestamp (エンコード済みサンプルとの対応付けに使う)
    timestamp: Duration,
    decoded_data_size: usize,
    width: Option<usize>,
    height: Option<usize>,
}

/// デコード出力を同じ timestamp のエンコード済み映像サンプルへ載せる
///
/// 対応するエンコード済みサンプルがまだ無いデコード出力は `pending` に残す。
/// 見つからない timestamp のサンプルへは繰り下げない (FIFO ではない)。
fn apply_decoded_video_infos_by_timestamp(
    video_samples: &mut [VideoSampleInfo],
    pending: &mut VecDeque<DecodedVideoInfo>,
) {
    let mut remaining = VecDeque::new();
    while let Some(decoded_info) = pending.pop_front() {
        if let Some(info) = video_samples
            .iter_mut()
            .find(|s| s.timestamp == decoded_info.timestamp)
        {
            info.decoded_data_size = Some(decoded_info.decoded_data_size);
            info.width = decoded_info.width;
            info.height = decoded_info.height;
        } else {
            // エンコード済みサンプルがまだ来ていないので後で再試行する
            remaining.push_back(decoded_info);
        }
    }
    *pending = remaining;
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
                let audio_data = media_sample.expect_audio()?;
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
                let video_frame = media_sample.expect_video()?;
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
                let audio_data = media_sample.expect_audio()?;
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
                let video_frame = media_sample.expect_video()?;
                self.pending_video_decoded_infos
                    .push_back(DecodedVideoInfo {
                        timestamp: video_frame.timestamp,
                        decoded_data_size: video_frame.data.len(),
                        width: video_frame.size().map(|size| size.width),
                        height: video_frame.size().map(|size| size.height),
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
        apply_decoded_video_infos_by_timestamp(
            &mut self.video_samples,
            &mut self.pending_video_decoded_infos,
        );
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

#[cfg(test)]
mod tests {
    use shiguredo_mp4::{
        Uint,
        boxes::{Avc1Box, AvccBox, SampleEntry},
    };

    use super::*;
    use crate::sample_entry::SharedSampleEntry;
    use crate::video::sample_entry_visual_fields;

    /// `VideoCodecSpecificInfo::new` を検証するための AVCC 形式 H.264 `VideoFrame` を構築する
    ///
    /// `length_size` は NAL 長フィールドのバイト数 (avcC の `lengthSizeMinusOne + 1` に相当)。
    /// `sample_entry` が `Some` の場合は、`length_size - 1` を `lengthSizeMinusOne` に持つ
    /// `SampleEntry::Avc1` を設定し、`None` の場合は sample_entry を空にする。
    fn build_h264_avcc_frame(
        data: &[u8],
        length_size: usize,
        with_sample_entry: bool,
    ) -> VideoFrame {
        let sample_entry = with_sample_entry.then(|| {
            SharedSampleEntry::new(SampleEntry::Avc1(Avc1Box {
                visual: sample_entry_visual_fields(320, 240),
                avcc_box: AvccBox {
                    avc_profile_indication: 0x42,
                    profile_compatibility: 0,
                    avc_level_indication: 0x1e,
                    chroma_format: None,
                    bit_depth_luma_minus8: None,
                    bit_depth_chroma_minus8: None,
                    length_size_minus_one: Uint::new((length_size - 1) as u8),
                    sps_ext_list: Vec::new(),
                    sps_list: Vec::new(),
                    pps_list: Vec::new(),
                },
                unknown_boxes: Vec::new(),
            }))
        });
        VideoFrame {
            data: data.to_vec(),
            format: VideoFormat::H264,
            keyframe: true,
            size: None,
            timestamp: Duration::ZERO,
            sample_entry,
        }
    }

    /// `length_size` バイトの big-endian NAL 長プレフィックス + NAL データを組み立てる
    fn encode_nal(length_size: usize, nalu: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let len = nalu.len();
        for shift in (0..length_size).rev() {
            out.push((len >> (shift * 8)) as u8);
        }
        out.extend_from_slice(nalu);
        out
    }

    #[test]
    fn read_nal_length_reads_variable_length() {
        // 1〜4 バイトの長フィールドを big-endian で正しく読めること
        assert_eq!(read_nal_length(&[0x01], 1), Some(1));
        assert_eq!(read_nal_length(&[0x01, 0x02], 2), Some(0x0102));
        assert_eq!(read_nal_length(&[0x01, 0x02, 0x03], 3), Some(0x010203));
        assert_eq!(
            read_nal_length(&[0x01, 0x02, 0x03, 0x04], 4),
            Some(0x01020304)
        );
    }

    #[test]
    fn read_nal_length_returns_none_when_too_short() {
        // 長フィールドが `length_size` バイトに満たない場合は None を返すこと
        assert_eq!(read_nal_length(&[], 1), None);
        assert_eq!(read_nal_length(&[0x01], 2), None);
    }

    #[test]
    fn video_codec_specific_info_h264_respects_length_size() {
        // length_size 1〜4 のそれぞれで、AVCC の NAL 長フィールドを正しく読めること
        //
        // 各 length_size で「長フィールドの複数バイトにまたがる長さ」の NAL を 1 つ含め、
        // encode_nal による往復 (encode → parse → type 抽出) を検証する。
        // length_size=1 では 255 (1 バイト長の最大表現可能値) を境界として検証する。
        for length_size in 1..=4 {
            let long_nalu_len = match length_size {
                1 => 255,
                2 => 300,
                3 => 70000,
                4 => 200000,
                _ => unreachable!(),
            };
            // 先頭バイトを SPS (0x67) にすることで、長さが正しく読めた場合に type 7 として検出される
            let long_nalu = vec![0x67u8; long_nalu_len];

            // SPS (0x67) / PPS (0x68) / IDR (0x65) / 長い SPS の 4 NAL を length_size で連結する
            let mut data = Vec::new();
            data.extend_from_slice(&encode_nal(length_size, &[0x67, 0x42]));
            data.extend_from_slice(&encode_nal(length_size, &[0x68, 0xce]));
            data.extend_from_slice(&encode_nal(length_size, &[0x65, 0x88]));
            data.extend_from_slice(&encode_nal(length_size, &long_nalu));

            let frame = build_h264_avcc_frame(&data, length_size, true);
            let info = VideoCodecSpecificInfo::new(&frame)
                .expect("AVCC 形式の H.264 フレームから NAL 情報を取得できること");

            match info {
                VideoCodecSpecificInfo::H264 { nalus } => {
                    assert_eq!(nalus.len(), 4, "4 個の NAL が検出されること");
                    assert_eq!(nalus[0].ty, 7, "先頭 NAL は SPS (type 7) であること");
                    assert_eq!(nalus[1].ty, 8, "2 番目の NAL は PPS (type 8) であること");
                    assert_eq!(nalus[2].ty, 5, "3 番目の NAL は IDR (type 5) であること");
                    assert_eq!(
                        nalus[3].ty, 7,
                        "長い NAL が正しく読めて SPS (type 7) として検出されること"
                    );
                }
            }
        }
    }

    #[test]
    fn video_codec_specific_info_h264_returns_none_on_invalid_length() {
        // 長さプレフィックスが残データを超える不正入力では None が返ること
        // 先頭の NAL (長さ 2) を処理した後、次の長フィールド 0xff (実データ 1 バイトより超過) を読んで None になる
        let mut data = encode_nal(4, &[0x67, 0x42]);
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0xff, 0x00]);
        let frame = build_h264_avcc_frame(&data, 4, true);
        assert!(
            VideoCodecSpecificInfo::new(&frame).is_none(),
            "長さ超過の NAL で None が返ること"
        );

        // 長さ 0 の NAL では None が返ること
        // 先頭の NAL (長さ 2) を処理した後、次の長フィールド 0 (長さ 0) を読んで None になる
        let mut data = encode_nal(4, &[0x67, 0x42]);
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00]);
        let frame = build_h264_avcc_frame(&data, 4, true);
        assert!(
            VideoCodecSpecificInfo::new(&frame).is_none(),
            "長さ 0 の NAL で None が返ること"
        );
    }

    #[test]
    fn video_codec_specific_info_h264_falls_back_to_4bytes_without_sample_entry() {
        // sample_entry が None の場合、4 バイト固定で NAL 長フィールドを読むこと
        let data = encode_nal(4, &[0x67, 0x42]);
        let frame = build_h264_avcc_frame(&data, 4, false);
        let info = VideoCodecSpecificInfo::new(&frame)
            .expect("sample_entry なしでも 4 バイト固定で NAL 情報を取得できること");

        match info {
            VideoCodecSpecificInfo::H264 { nalus } => {
                assert_eq!(nalus.len(), 1, "1 個の NAL が検出されること");
                assert_eq!(nalus[0].ty, 7, "NAL は SPS (type 7) であること");
            }
        }
    }

    #[test]
    fn detect_regular_mp4_as_mp4() {
        assert_eq!(
            detect_container_format(Path::new("testdata/red-320x320-h264-aac.mp4"))
                .expect("通常 MP4 の判定に成功すること"),
            ContainerFormat::Mp4
        );
    }

    #[test]
    fn detect_fragmented_mp4_as_fmp4() {
        assert_eq!(
            detect_container_format(Path::new("testdata/red-320x320-h264-aac-fragmented.mp4"))
                .expect("fMP4 の判定に成功すること"),
            ContainerFormat::Fmp4
        );
    }

    #[test]
    fn detect_propagates_error_for_corrupted_mp4() {
        use std::io::Write;

        // .mp4 拡張子だが中身が不正なファイルは detect_mp4_file_kind がエラーになり、
        // それが伝播すること (Mp4 へフォールバックしないこと) を検証する
        let mut file = tempfile::Builder::new()
            .suffix(".mp4")
            .tempfile()
            .expect("一時ファイルを作成できること");
        file.write_all(b"this is definitely not a valid mp4 file")
            .expect("一時ファイルに書き込めること");
        let result = detect_container_format(file.path());
        assert!(result.is_err(), "破損 MP4 は判定エラーが伝播すること");
    }

    /// テスト用のエンコード済み映像サンプルを作る
    fn video_sample(timestamp_us: u64, data_size: usize) -> VideoSampleInfo {
        VideoSampleInfo {
            timestamp: Duration::from_micros(timestamp_us),
            duration: None,
            data_size,
            keyframe: false,
            codec_specific_info: None,
            decoded_data_size: None,
            width: None,
            height: None,
        }
    }

    /// テスト用のデコード出力情報を作る
    fn decoded_video(
        timestamp_us: u64,
        decoded_data_size: usize,
        width: usize,
        height: usize,
    ) -> DecodedVideoInfo {
        DecodedVideoInfo {
            timestamp: Duration::from_micros(timestamp_us),
            decoded_data_size,
            width: Some(width),
            height: Some(height),
        }
    }

    #[test]
    fn apply_decoded_video_infos_matches_by_timestamp() {
        // エンコード列とデコード列を timestamp で対応付けること
        let mut samples = vec![
            video_sample(0, 100),
            video_sample(40_000, 110),
            video_sample(80_000, 120),
        ];
        let mut pending = VecDeque::from([
            decoded_video(0, 1000, 320, 240),
            decoded_video(40_000, 2000, 320, 240),
            decoded_video(80_000, 3000, 640, 480),
        ]);

        apply_decoded_video_infos_by_timestamp(&mut samples, &mut pending);

        assert!(
            pending.is_empty(),
            "全て対応付けられて pending が空になること"
        );
        assert_eq!(samples[0].decoded_data_size, Some(1000));
        assert_eq!(samples[0].width, Some(320));
        assert_eq!(samples[0].height, Some(240));
        assert_eq!(samples[1].decoded_data_size, Some(2000));
        assert_eq!(samples[2].decoded_data_size, Some(3000));
        assert_eq!(samples[2].width, Some(640));
        assert_eq!(samples[2].height, Some(480));
    }

    #[test]
    fn apply_decoded_video_infos_does_not_shift_on_missing_decode() {
        // 途中 1 件のデコード出力が欠けても、欠けていない timestamp へ誤って載せないこと
        // FIFO なら先頭未設定へ繰り下げて S1 に 3000 が乗るが、timestamp 対応では乗らない
        let mut samples = vec![
            video_sample(0, 100),
            video_sample(40_000, 110),
            video_sample(80_000, 120),
        ];
        let mut pending = VecDeque::from([
            decoded_video(0, 1000, 320, 240),
            // timestamp 40_000 の出力は欠落
            decoded_video(80_000, 3000, 640, 480),
        ]);

        apply_decoded_video_infos_by_timestamp(&mut samples, &mut pending);

        assert!(
            pending.is_empty(),
            "存在する timestamp は全て対応付けられること"
        );
        assert_eq!(
            samples[0].decoded_data_size,
            Some(1000),
            "先頭サンプルは自分のデコード結果を持つこと"
        );
        assert_eq!(
            samples[1].decoded_data_size, None,
            "欠落した timestamp のサンプルは未設定のまま残ること"
        );
        assert_eq!(samples[1].width, None);
        assert_eq!(samples[1].height, None);
        assert_eq!(
            samples[2].decoded_data_size,
            Some(3000),
            "後続サンプルは自分の timestamp のデコード結果を持つこと"
        );
        assert_eq!(samples[2].width, Some(640));
        assert_eq!(samples[2].height, Some(480));
    }

    #[test]
    fn apply_decoded_video_infos_keeps_pending_until_encoded_arrives() {
        // デコード出力がエンコード済みサンプルより先に来た場合、対応するサンプルが来るまで pending に残ること
        let mut samples = vec![video_sample(0, 100)];
        let mut pending = VecDeque::from([
            decoded_video(0, 1000, 320, 240),
            decoded_video(40_000, 2000, 320, 240),
        ]);

        apply_decoded_video_infos_by_timestamp(&mut samples, &mut pending);

        assert_eq!(samples[0].decoded_data_size, Some(1000));
        assert_eq!(
            pending.len(),
            1,
            "未到着のエンコード済みサンプル向けデコード出力が残ること"
        );
        assert_eq!(pending[0].timestamp, Duration::from_micros(40_000));

        // 後からエンコード済みサンプルが来たら対応付けられること
        samples.push(video_sample(40_000, 110));
        apply_decoded_video_infos_by_timestamp(&mut samples, &mut pending);

        assert!(pending.is_empty());
        assert_eq!(samples[1].decoded_data_size, Some(2000));
        assert_eq!(samples[1].width, Some(320));
        assert_eq!(samples[1].height, Some(240));
    }
}
