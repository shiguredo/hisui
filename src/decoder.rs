#[cfg(target_os = "macos")]
pub mod audio_toolbox;
pub mod dav1d;
#[cfg(feature = "fdk-aac")]
pub mod fdk_aac;
pub mod libvpx;
#[cfg(feature = "nvcodec")]
pub mod nvcodec;
pub mod openh264;
pub mod opus;
#[cfg(target_os = "macos")]
pub mod video_toolbox;

use std::collections::VecDeque;

use shiguredo_openh264::Openh264Library;

use self::dav1d::Dav1dDecoder;
use self::libvpx::LibvpxDecoder;
#[cfg(feature = "nvcodec")]
use self::nvcodec::NvcodecDecoder;
use self::openh264::Openh264Decoder;
use self::opus::OpusDecoder;
#[cfg(target_os = "macos")]
use self::video_toolbox::VideoToolboxDecoder;
use crate::{
    Error, Message, ProcessorHandle, Result, TrackId,
    audio::{AudioFormat, AudioFrame},
    media::MediaFrame,
    types::{CodecName, EngineName},
    video::VideoFrame,
};

#[derive(Debug)]
pub struct AudioDecoder {
    #[cfg(feature = "fdk-aac")]
    fdk_aac_lib: Option<shiguredo_fdk_aac::FdkAacLibrary>,
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_audio_data_count_metric: crate::stats::StatsCounter,
    decoded: VecDeque<AudioFrame>,
    eos: bool,
    inner: Option<AudioDecoderInner>,
}

pub enum DecoderRunOutput {
    Processed(MediaFrame),
    Pending,
    Finished,
}

/// `drain_*_decoder_output()` の結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainResult {
    /// デコーダーの出力バッファが空になった（継続可能）
    Pending,
    /// 送信先が閉じた（pipeline が終了した）
    PipelineClosed,
    /// デコーダーの EOS flush が完了した
    Finished,
}

impl AudioDecoder {
    pub fn new(
        #[cfg(feature = "fdk-aac")] fdk_aac_lib: Option<shiguredo_fdk_aac::FdkAacLibrary>,
        mut compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        let engine_metric = compose_stats.string("engine");
        let codec_metric = compose_stats.string("codec");
        let total_audio_data_count_metric = compose_stats.counter("total_audio_data_count");
        compose_stats.flag("error").set(false);
        Ok(Self {
            #[cfg(feature = "fdk-aac")]
            fdk_aac_lib,
            engine_metric,
            codec_metric,
            total_audio_data_count_metric,
            decoded: VecDeque::new(),
            eos: false,
            inner: None,
        })
    }

    pub async fn run(
        mut self,
        handle: ProcessorHandle,
        input_track_id: TrackId,
        output_track_id: TrackId,
    ) -> Result<()> {
        let mut input_rx = handle.subscribe_track(input_track_id);
        let mut output_tx = handle.publish_track(output_track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        loop {
            let message = input_rx.recv().await;
            let is_eos = matches!(message, Message::Eos);

            self.handle_input_message(message)?;

            match drain_audio_decoder_output(&mut self, &mut output_tx)? {
                DrainResult::PipelineClosed | DrainResult::Finished => {
                    output_tx.send_eos();
                    break;
                }
                DrainResult::Pending => {}
            }

            if is_eos {
                return Err(Error::new("audio decoder still pending after EOS"));
            }
        }

        Ok(())
    }

    pub fn handle_input_message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Media(sample) => self.handle_input_sample(Some(sample)),
            Message::Eos => self.handle_input_sample(None),
            Message::Syn(_) => Ok(()),
        }
    }

    pub fn handle_input_sample(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        let Some(sample) = sample else {
            self.eos = true;
            return Ok(());
        };
        let frame = sample.expect_audio()?;

        // 遅延初期化
        if self.inner.is_none() {
            let inner = AudioDecoderInner::new(
                &frame,
                #[cfg(feature = "fdk-aac")]
                self.fdk_aac_lib.take(),
            )?;
            self.engine_metric.set(inner.engine_name().as_str());
            self.codec_metric.set(inner.codec_name().as_str());
            self.inner = Some(inner);
        }

        let inner = self.inner.as_mut().expect("infallible");
        let decoded = inner.decode(&frame)?;
        self.total_audio_data_count_metric.inc();

        self.decoded.push_back(decoded);
        Ok(())
    }

    pub fn poll_output(&mut self) -> Result<DecoderRunOutput> {
        if let Some(frame) = self.decoded.pop_front() {
            Ok(DecoderRunOutput::Processed(MediaFrame::audio(frame)))
        } else if self.eos {
            if let Some(inner) = self.inner.as_mut()
                && let Some(remaining_frame) = inner.finish()?
            {
                self.total_audio_data_count_metric.inc();
                self.decoded.push_back(remaining_frame);
                let sample = self.decoded.pop_front().ok_or_else(|| {
                    crate::Error::new("decoded audio queue is unexpectedly empty")
                })?;
                return Ok(DecoderRunOutput::Processed(MediaFrame::audio(sample)));
            }
            Ok(DecoderRunOutput::Finished)
        } else {
            Ok(DecoderRunOutput::Pending)
        }
    }

    pub fn get_engines(codec: CodecName, is_fdk_aac_available: bool) -> Vec<EngineName> {
        match codec {
            CodecName::Aac => {
                let mut engines = Vec::new();

                if is_fdk_aac_available {
                    engines.push(EngineName::FdkAac);
                }
                #[cfg(target_os = "macos")]
                {
                    engines.push(EngineName::AudioToolbox);
                }

                engines
            }
            CodecName::Opus => vec![EngineName::Opus],
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
enum AudioDecoderInner {
    Opus(OpusDecoder),
    #[cfg(target_os = "macos")]
    AudioToolbox(self::audio_toolbox::AudioToolboxDecoder),
    #[cfg(feature = "fdk-aac")]
    FdkAac(self::fdk_aac::FdkAacDecoder),
}

impl AudioDecoderInner {
    fn new(
        frame: &AudioFrame,
        #[cfg(feature = "fdk-aac")] fdk_aac_lib: Option<shiguredo_fdk_aac::FdkAacLibrary>,
    ) -> crate::Result<Self> {
        match frame.format {
            AudioFormat::Opus => OpusDecoder::new().map(Self::Opus),
            AudioFormat::Aac => {
                #[cfg(feature = "fdk-aac")]
                if let Some(lib) = fdk_aac_lib {
                    return self::fdk_aac::FdkAacDecoder::new(lib).map(Self::FdkAac);
                }

                #[cfg(target_os = "macos")]
                return self::audio_toolbox::AudioToolboxDecoder::new().map(Self::AudioToolbox);

                #[cfg(not(target_os = "macos"))]
                return Err(crate::Error::new(
                    "AAC decoding is not supported without --fdk-aac option or macOS",
                ));
            }
            _ => Err(crate::Error::new(format!(
                "Unsupported audio format: {:?}",
                frame.format
            ))),
        }
    }

    fn decode(&mut self, frame: &AudioFrame) -> crate::Result<AudioFrame> {
        match self {
            Self::Opus(decoder) => decoder.decode(frame),
            #[cfg(target_os = "macos")]
            Self::AudioToolbox(decoder) => decoder.decode(frame),
            #[cfg(feature = "fdk-aac")]
            Self::FdkAac(decoder) => decoder.decode(frame),
        }
    }

    fn finish(&mut self) -> crate::Result<Option<AudioFrame>> {
        match self {
            Self::Opus(_decoder) => Ok(None),
            #[cfg(target_os = "macos")]
            Self::AudioToolbox(decoder) => decoder.finish(),
            #[cfg(feature = "fdk-aac")]
            Self::FdkAac(_decoder) => Ok(None),
        }
    }

    fn engine_name(&self) -> EngineName {
        match self {
            Self::Opus(_) => EngineName::Opus,
            #[cfg(target_os = "macos")]
            Self::AudioToolbox(_) => EngineName::AudioToolbox,
            #[cfg(feature = "fdk-aac")]
            Self::FdkAac(_) => EngineName::FdkAac,
        }
    }

    fn codec_name(&self) -> CodecName {
        match self {
            Self::Opus(_) => CodecName::Opus,
            #[cfg(target_os = "macos")]
            Self::AudioToolbox(_) => CodecName::Aac,
            #[cfg(feature = "fdk-aac")]
            Self::FdkAac(_) => CodecName::Aac,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodeConfig {
    #[cfg(feature = "nvcodec")]
    pub nvcodec_h264: shiguredo_nvcodec::DecoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_h265: shiguredo_nvcodec::DecoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_av1: shiguredo_nvcodec::DecoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_vp8: shiguredo_nvcodec::DecoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_vp9: shiguredo_nvcodec::DecoderConfig,
}

#[cfg_attr(
    not(feature = "nvcodec"),
    expect(
        clippy::derivable_impls,
        reason = "nvcodec feature 無効時は導出可能だが、有効時は shiguredo_nvcodec::DecoderConfig に Default がないため手動実装を共用している"
    )
)]
impl Default for DecodeConfig {
    fn default() -> Self {
        Self {
            #[cfg(feature = "nvcodec")]
            nvcodec_h264: default_nvcodec_decoder_config(shiguredo_nvcodec::DecoderCodec::H264),
            #[cfg(feature = "nvcodec")]
            nvcodec_h265: default_nvcodec_decoder_config(shiguredo_nvcodec::DecoderCodec::Hevc),
            #[cfg(feature = "nvcodec")]
            nvcodec_av1: default_nvcodec_decoder_config(shiguredo_nvcodec::DecoderCodec::Av1),
            #[cfg(feature = "nvcodec")]
            nvcodec_vp8: default_nvcodec_decoder_config(shiguredo_nvcodec::DecoderCodec::Vp8),
            #[cfg(feature = "nvcodec")]
            nvcodec_vp9: default_nvcodec_decoder_config(shiguredo_nvcodec::DecoderCodec::Vp9),
        }
    }
}

#[cfg(feature = "nvcodec")]
fn default_nvcodec_decoder_config(
    codec: shiguredo_nvcodec::DecoderCodec,
) -> shiguredo_nvcodec::DecoderConfig {
    shiguredo_nvcodec::DecoderConfig {
        codec,
        device_id: 0,
        max_num_decode_surfaces: 20,
        max_display_delay: 0,
        surface_format: shiguredo_nvcodec::SurfaceFormat::Nv12,
    }
}

#[derive(Debug, Default, Clone)]
pub struct VideoDecoderOptions {
    pub openh264_lib: Option<Openh264Library>,
    pub decode_params: DecodeConfig,
    pub engines: Option<Vec<EngineName>>,
}

#[derive(Debug)]
pub struct VideoDecoder {
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    total_output_video_frame_count_metric: crate::stats::StatsCounter,
    decoded: VecDeque<VideoFrame>,
    eos: bool,
    inner: VideoDecoderInner,
}

impl VideoDecoder {
    pub fn new(options: VideoDecoderOptions, mut compose_stats: crate::stats::Stats) -> Self {
        let engine_metric = compose_stats.string("engine");
        let codec_metric = compose_stats.string("codec");
        let total_input_video_frame_count_metric =
            compose_stats.counter("total_input_video_frame_count");
        let total_output_video_frame_count_metric =
            compose_stats.counter("total_output_video_frame_count");
        compose_stats.flag("error").set(false);
        Self {
            engine_metric,
            codec_metric,
            total_input_video_frame_count_metric,
            total_output_video_frame_count_metric,
            decoded: VecDeque::new(),
            eos: false,
            inner: VideoDecoderInner::new(options),
        }
    }

    pub async fn run(
        mut self,
        handle: ProcessorHandle,
        input_track_id: TrackId,
        output_track_id: TrackId,
    ) -> Result<()> {
        let mut input_rx = handle.subscribe_track(input_track_id);
        let mut output_tx = handle.publish_track(output_track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        loop {
            let message = input_rx.recv().await;
            let is_eos = matches!(message, Message::Eos);

            self.handle_input_message(message)?;

            match drain_video_decoder_output(&mut self, &mut output_tx)? {
                DrainResult::PipelineClosed | DrainResult::Finished => {
                    output_tx.send_eos();
                    break;
                }
                DrainResult::Pending => {}
            }

            if is_eos {
                return Err(Error::new("video decoder still pending after EOS"));
            }
        }

        Ok(())
    }

    pub fn handle_input_message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Media(sample) => self.handle_input_sample(Some(sample)),
            Message::Eos => self.handle_input_sample(None),
            Message::Syn(_) => Ok(()),
        }
    }

    pub fn handle_input_sample(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        if let Some(sample) = sample {
            let frame = sample.expect_video()?;

            self.total_input_video_frame_count_metric.inc();

            self.inner
                .decode(&frame, &self.codec_metric, &self.engine_metric)?;
        } else {
            self.eos = true;
            self.inner.finish()?;
        };

        while let Some(frame) = self.inner.next_decoded_frame() {
            self.total_output_video_frame_count_metric.inc();
            self.decoded.push_back(frame);
        }

        Ok(())
    }

    pub fn poll_output(&mut self) -> Result<DecoderRunOutput> {
        if let Some(frame) = self.decoded.pop_front() {
            Ok(DecoderRunOutput::Processed(MediaFrame::video(frame)))
        } else if self.eos {
            Ok(DecoderRunOutput::Finished)
        } else {
            Ok(DecoderRunOutput::Pending)
        }
    }

    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> {
        let mut engines = Vec::new();
        match codec {
            CodecName::Vp8 | CodecName::Vp9 => {
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_library_available() {
                    engines.push(EngineName::Nvcodec);
                }
                engines.push(EngineName::Libvpx);
            }
            CodecName::H264 => {
                if is_openh264_available {
                    engines.push(EngineName::Openh264);
                }
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_library_available() {
                    engines.push(EngineName::Nvcodec);
                }
                #[cfg(target_os = "macos")]
                {
                    engines.push(EngineName::VideoToolbox);
                }
            }
            CodecName::H265 => {
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_library_available() {
                    engines.push(EngineName::Nvcodec);
                }
                #[cfg(target_os = "macos")]
                {
                    engines.push(EngineName::VideoToolbox);
                }
            }
            CodecName::Av1 => {
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_library_available() {
                    engines.push(EngineName::Nvcodec);
                }
                engines.push(EngineName::Dav1d);
            }
            _ => unreachable!(),
        }
        engines
    }
}

pub fn drain_audio_decoder_output(
    decoder: &mut AudioDecoder,
    output_tx: &mut crate::TrackPublisher,
) -> Result<DrainResult> {
    loop {
        match decoder.poll_output()? {
            DecoderRunOutput::Processed(sample) => {
                if !output_tx.send_media(sample) {
                    return Ok(DrainResult::PipelineClosed);
                }
            }
            DecoderRunOutput::Pending => {
                return Ok(DrainResult::Pending);
            }
            DecoderRunOutput::Finished => {
                return Ok(DrainResult::Finished);
            }
        }
    }
}

pub fn drain_video_decoder_output(
    decoder: &mut VideoDecoder,
    output_tx: &mut crate::TrackPublisher,
) -> Result<DrainResult> {
    loop {
        match decoder.poll_output()? {
            DecoderRunOutput::Processed(sample) => {
                if !output_tx.send_media(sample) {
                    return Ok(DrainResult::PipelineClosed);
                }
            }
            DecoderRunOutput::Pending => {
                return Ok(DrainResult::Pending);
            }
            DecoderRunOutput::Finished => {
                return Ok(DrainResult::Finished);
            }
        }
    }
}

#[derive(Debug)]
enum VideoDecoderInner {
    Initial {
        options: VideoDecoderOptions,
    },
    Libvpx(LibvpxDecoder),
    Openh264(Openh264Decoder),
    Dav1d(Dav1dDecoder),
    #[cfg(target_os = "macos")]
    VideoToolbox(Box<VideoToolboxDecoder>), // Box は clippy::large_enum_variant 対策
    #[cfg(feature = "nvcodec")]
    Nvcodec(NvcodecDecoder),
}

impl VideoDecoderInner {
    fn new(options: VideoDecoderOptions) -> Self {
        // [NOTE] 最初の映像フレームが来た時点で実際のデコーダーに切り替わる
        Self::Initial { options }
    }

    fn initialize_decoder(
        &mut self,
        frame: &VideoFrame,
        codec_metric: &crate::stats::StatsString,
        engine_metric: &crate::stats::StatsString,
        options: VideoDecoderOptions,
    ) -> crate::Result<()> {
        let codec = frame.format.codec_name().ok_or_else(|| {
            crate::Error::new(format!("unexpected video format: {:?}", frame.format))
        })?;
        codec_metric.set(codec.as_str());

        let candidate_engines = options
            .engines
            .unwrap_or_else(|| VideoDecoder::get_engines(codec, options.openh264_lib.is_some()));

        let engine = candidate_engines
            .iter()
            .find(|engine| engine.is_available_video_decode_codec(codec))
            .copied();
        if let Some(engine) = engine {
            engine_metric.set(engine.as_str());
        }

        match (engine, codec) {
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H264) => {
                *self = NvcodecDecoder::new_h264(&options.decode_params).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H265) => {
                *self = NvcodecDecoder::new_h265(&options.decode_params).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Vp8) => {
                *self = NvcodecDecoder::new_vp8(&options.decode_params).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Vp9) => {
                *self = NvcodecDecoder::new_vp9(&options.decode_params).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Av1) => {
                *self = NvcodecDecoder::new_av1(&options.decode_params).map(Self::Nvcodec)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H264) => {
                *self = VideoToolboxDecoder::new_h264(frame)
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H265) => {
                *self = VideoToolboxDecoder::new_h265(frame)
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            (Some(EngineName::Openh264), CodecName::H264) => {
                let lib = options.openh264_lib.ok_or_else(|| {
                    crate::Error::new("OpenH264 library is required for H.264 decoding")
                })?;
                *self = Openh264Decoder::new(lib.clone()).map(Self::Openh264)?;
            }
            (Some(EngineName::Libvpx), CodecName::Vp8) => {
                *self = LibvpxDecoder::new_vp8().map(Self::Libvpx)?;
            }
            (Some(EngineName::Libvpx), CodecName::Vp9) => {
                *self = LibvpxDecoder::new_vp9().map(Self::Libvpx)?;
            }
            (Some(EngineName::Dav1d), CodecName::Av1) => {
                *self = Dav1dDecoder::new().map(Self::Dav1d)?;
            }
            _ => {
                return Err(crate::Error::new(format!(
                    "no available decoder for {} codec (candidate decoders: {})",
                    codec.as_str(),
                    candidate_engines
                        .iter()
                        .map(|engine| engine.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
        Ok(())
    }

    fn decode(
        &mut self,
        frame: &VideoFrame,
        codec_metric: &crate::stats::StatsString,
        engine_metric: &crate::stats::StatsString,
    ) -> crate::Result<()> {
        match self {
            Self::Initial { options } => {
                let options = options.clone();
                self.initialize_decoder(frame, codec_metric, engine_metric, options)?;
                self.decode(frame, codec_metric, engine_metric)
            }
            Self::Libvpx(decoder) => decoder.decode(frame),
            Self::Openh264(decoder) => decoder.decode(frame),
            Self::Dav1d(decoder) => decoder.decode(frame),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(decoder) => decoder.decode(frame),
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(decoder) => decoder.decode(frame),
        }
    }

    fn finish(&mut self) -> crate::Result<()> {
        match self {
            Self::Initial { .. } => {}
            Self::Libvpx(decoder) => decoder.finish()?,
            Self::Openh264(decoder) => decoder.finish()?,
            Self::Dav1d(decoder) => decoder.finish()?,
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(_decoder) => {}
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(decoder) => decoder.finish()?,
        }
        Ok(())
    }

    fn next_decoded_frame(&mut self) -> Option<VideoFrame> {
        match self {
            Self::Initial { .. } => None,
            Self::Libvpx(decoder) => decoder.next_decoded_frame(),
            Self::Openh264(decoder) => decoder.next_decoded_frame(),
            Self::Dav1d(decoder) => decoder.next_decoded_frame(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(decoder) => decoder.next_decoded_frame(),
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(decoder) => decoder.next_decoded_frame(),
        }
    }
}
