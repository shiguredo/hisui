use std::collections::VecDeque;

use shiguredo_openh264::Openh264Library;

use crate::audio::AudioFormat;
#[cfg(feature = "libvpx")]
use crate::decoder_libvpx::LibvpxDecoder;
#[cfg(feature = "nvcodec")]
use crate::decoder_nvcodec::NvcodecDecoder;
#[cfg(target_os = "macos")]
use crate::decoder_video_toolbox::VideoToolboxDecoder;
use crate::{
    Error, Message, ProcessorHandle, Result, TrackId,
    audio::AudioData,
    decoder_dav1d::Dav1dDecoder,
    decoder_openh264::Openh264Decoder,
    decoder_opus::OpusDecoder,
    layout_decode_params::LayoutDecodeParams,
    media::MediaSample,
    types::{CodecName, EngineName},
    video::VideoFrame,
};

#[derive(Debug)]
pub struct AudioDecoder {
    total_audio_data_count_metric: crate::stats::StatsCounter,
    source_id_metric: crate::stats::StatsString,
    decoded: VecDeque<AudioData>,
    eos: bool,
    inner: Option<AudioDecoderInner>,
}

enum DecoderRunOutput {
    Processed(MediaSample),
    Pending,
    Finished,
}

impl AudioDecoder {
    pub fn new(mut compose_stats: crate::stats::Stats) -> crate::Result<Self> {
        compose_stats
            .string("engine")
            .set(EngineName::Opus.as_str());
        compose_stats.string("codec").set(CodecName::Opus.as_str());
        let total_audio_data_count_metric = compose_stats.counter("total_audio_data_count");
        let source_id_metric = compose_stats.string("source_id");
        compose_stats.flag("error").set(false);
        Ok(Self {
            total_audio_data_count_metric,
            source_id_metric,
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

            let finished = drain_audio_decoder_output(&mut self, &mut output_tx)?;

            if finished {
                output_tx.send_eos();
                break;
            }

            if is_eos {
                return Err(Error::new("audio decoder still pending after EOS"));
            }
        }

        Ok(())
    }

    fn handle_input_message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Media(sample) => self.handle_input_sample(Some(sample)),
            Message::Eos => self.handle_input_sample(None),
            Message::Syn(_) => Ok(()),
        }
    }

    fn handle_input_sample(&mut self, sample: Option<MediaSample>) -> Result<()> {
        let Some(sample) = sample else {
            self.eos = true;
            return Ok(());
        };
        let data = sample.expect_audio_data()?;

        // 遅延初期化
        if self.inner.is_none() {
            self.inner = Some(AudioDecoderInner::new(&data)?);
        }

        let inner = self.inner.as_mut().expect("infallible");
        let decoded = inner.decode(&data)?;
        self.total_audio_data_count_metric.inc();
        if let Some(id) = &data.source_id {
            self.source_id_metric.set(id.get());
        }

        self.decoded.push_back(decoded);
        Ok(())
    }

    fn poll_output(&mut self) -> Result<DecoderRunOutput> {
        if let Some(data) = self.decoded.pop_front() {
            Ok(DecoderRunOutput::Processed(MediaSample::audio_data(data)))
        } else if self.eos {
            if let Some(inner) = self.inner.as_mut()
                && let Some(remaining_data) = inner.finish()?
            {
                self.total_audio_data_count_metric.inc();
                self.decoded.push_back(remaining_data);
                let sample = self.decoded.pop_front().ok_or_else(|| {
                    crate::Error::new("decoded audio queue is unexpectedly empty")
                })?;
                return Ok(DecoderRunOutput::Processed(MediaSample::audio_data(sample)));
            }
            Ok(DecoderRunOutput::Finished)
        } else {
            Ok(DecoderRunOutput::Pending)
        }
    }

    pub fn get_engines(codec: CodecName) -> Vec<EngineName> {
        match codec {
            CodecName::Aac => {
                // cfg によって mut が必要だったり不要だったりするので警告は抑制する
                #[allow(unused_mut)]
                let mut engines = Vec::new();

                #[cfg(feature = "fdk-aac")]
                {
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
    AudioToolbox(crate::decoder_audio_toolbox::AudioToolboxDecoder),
    #[cfg(feature = "fdk-aac")]
    FdkAac(crate::decoder_fdk_aac::FdkAacDecoder),
}

impl AudioDecoderInner {
    fn new(data: &AudioData) -> crate::Result<Self> {
        match data.format {
            AudioFormat::Opus => OpusDecoder::new().map(Self::Opus),
            AudioFormat::Aac => {
                #[cfg(feature = "fdk-aac")]
                {
                    crate::decoder_fdk_aac::FdkAacDecoder::new().map(Self::FdkAac)
                }
                #[cfg(all(not(feature = "fdk-aac"), target_os = "macos"))]
                {
                    crate::decoder_audio_toolbox::AudioToolboxDecoder::new().map(Self::AudioToolbox)
                }
                #[cfg(all(not(feature = "fdk-aac"), not(target_os = "macos")))]
                {
                    Err(crate::Error::new(
                        "AAC decoding is only available on macOS or with fdk-aac feature enabled",
                    ))
                }
            }
            _ => Err(crate::Error::new(format!(
                "Unsupported audio format: {:?}",
                data.format
            ))),
        }
    }

    fn decode(&mut self, data: &AudioData) -> crate::Result<AudioData> {
        match self {
            Self::Opus(decoder) => decoder.decode(data),
            #[cfg(target_os = "macos")]
            Self::AudioToolbox(decoder) => decoder.decode(data),
            #[cfg(feature = "fdk-aac")]
            Self::FdkAac(decoder) => decoder.decode(data),
        }
    }

    fn finish(&mut self) -> crate::Result<Option<AudioData>> {
        match self {
            Self::Opus(_decoder) => Ok(None),
            #[cfg(target_os = "macos")]
            Self::AudioToolbox(decoder) => decoder.finish(),
            #[cfg(feature = "fdk-aac")]
            Self::FdkAac(_decoder) => Ok(None),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct VideoDecoderOptions {
    pub openh264_lib: Option<Openh264Library>,
    pub decode_params: LayoutDecodeParams,
    pub engines: Option<Vec<EngineName>>,
}

#[derive(Debug)]
pub struct VideoDecoder {
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    total_output_video_frame_count_metric: crate::stats::StatsCounter,
    source_id_metric: crate::stats::StatsString,
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
        let source_id_metric = compose_stats.string("source_id");
        compose_stats.flag("error").set(false);
        Self {
            engine_metric,
            codec_metric,
            total_input_video_frame_count_metric,
            total_output_video_frame_count_metric,
            source_id_metric,
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

            let finished = drain_video_decoder_output(&mut self, &mut output_tx)?;

            if finished {
                output_tx.send_eos();
                break;
            }

            if is_eos {
                return Err(Error::new("video decoder still pending after EOS"));
            }
        }

        Ok(())
    }

    fn handle_input_message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Media(sample) => self.handle_input_sample(Some(sample)),
            Message::Eos => self.handle_input_sample(None),
            Message::Syn(_) => Ok(()),
        }
    }

    fn handle_input_sample(&mut self, sample: Option<MediaSample>) -> Result<()> {
        if let Some(sample) = sample {
            let frame = sample.expect_video_frame()?;

            self.total_input_video_frame_count_metric.inc();
            if let Some(id) = &frame.source_id {
                self.source_id_metric.set(id.get());
            }

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

    fn poll_output(&mut self) -> Result<DecoderRunOutput> {
        if let Some(frame) = self.decoded.pop_front() {
            Ok(DecoderRunOutput::Processed(MediaSample::video_frame(frame)))
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
                #[cfg(feature = "libvpx")]
                {
                    engines.push(EngineName::Libvpx);
                }
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

fn drain_audio_decoder_output(
    decoder: &mut AudioDecoder,
    output_tx: &mut crate::MessageSender,
) -> Result<bool> {
    loop {
        match decoder.poll_output()? {
            DecoderRunOutput::Processed(sample) => {
                if !output_tx.send_media(sample) {
                    return Ok(true);
                }
            }
            DecoderRunOutput::Pending => {
                return Ok(false);
            }
            DecoderRunOutput::Finished => {
                return Ok(true);
            }
        }
    }
}

fn drain_video_decoder_output(
    decoder: &mut VideoDecoder,
    output_tx: &mut crate::MessageSender,
) -> Result<bool> {
    loop {
        match decoder.poll_output()? {
            DecoderRunOutput::Processed(sample) => {
                if !output_tx.send_media(sample) {
                    return Ok(true);
                }
            }
            DecoderRunOutput::Pending => {
                return Ok(false);
            }
            DecoderRunOutput::Finished => {
                return Ok(true);
            }
        }
    }
}

#[derive(Debug)]
enum VideoDecoderInner {
    Initial {
        options: VideoDecoderOptions,
    },
    #[cfg(feature = "libvpx")]
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
            #[cfg(feature = "libvpx")]
            (Some(EngineName::Libvpx), CodecName::Vp8) => {
                *self = LibvpxDecoder::new_vp8().map(Self::Libvpx)?;
            }
            #[cfg(feature = "libvpx")]
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
            #[cfg(feature = "libvpx")]
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
            #[cfg(feature = "libvpx")]
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
            #[cfg(feature = "libvpx")]
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
