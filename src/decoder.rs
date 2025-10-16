use std::collections::VecDeque;

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

#[cfg(feature = "libvpx")]
use crate::decoder_libvpx::LibvpxDecoder;
#[cfg(feature = "nvcodec")]
use crate::decoder_nvcodec::NvcodecDecoder;
#[cfg(target_os = "macos")]
use crate::decoder_video_toolbox::VideoToolboxDecoder;
use crate::{
    audio::AudioData,
    decoder_dav1d::Dav1dDecoder,
    decoder_openh264::Openh264Decoder,
    decoder_opus::OpusDecoder,
    layout_decode_params::LayoutDecodeParams,
    media::{MediaSample, MediaStreamId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    stats::{AudioDecoderStats, ProcessorStats, VideoDecoderStats, VideoResolution},
    types::{CodecName, EngineName},
    video::VideoFrame,
};

#[derive(Debug)]
pub struct AudioDecoder {
    input_stream_id: MediaStreamId,
    output_stream_id: MediaStreamId,
    stats: AudioDecoderStats,
    decoded: VecDeque<AudioData>,
    eos: bool,
    inner: AudioDecoderInner,
}

impl AudioDecoder {
    pub fn new_opus(
        input_stream_id: MediaStreamId,
        output_stream_id: MediaStreamId,
    ) -> orfail::Result<Self> {
        let stats = AudioDecoderStats {
            engine: Some(EngineName::Opus),
            codec: Some(CodecName::Opus),
            ..Default::default()
        };
        Ok(Self {
            input_stream_id,
            output_stream_id,
            stats,
            decoded: VecDeque::new(),
            eos: false,
            inner: AudioDecoderInner::new_opus().or_fail()?,
        })
    }

    pub fn get_engines(codec: CodecName) -> Vec<EngineName> {
        match codec {
            CodecName::Aac => vec![],
            CodecName::Opus => vec![EngineName::Opus],
            _ => unreachable!(),
        }
    }
}

impl MediaProcessor for AudioDecoder {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: vec![self.input_stream_id],
            output_stream_ids: vec![self.output_stream_id],
            stats: ProcessorStats::AudioDecoder(self.stats.clone()),
            workload_hint: MediaProcessorWorkloadHint::AUDIO_DECODER,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        let Some(sample) = input.sample else {
            self.eos = true;
            return Ok(());
        };
        let data = sample.expect_audio_data().or_fail()?;

        let decoded = self.inner.decode(&data).or_fail()?;
        self.stats.total_audio_data_count.add(1);
        if let Some(id) = &data.source_id {
            self.stats.source_id.set_once(|| id.clone());
        }

        self.decoded.push_back(decoded);
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        if let Some(data) = self.decoded.pop_front() {
            Ok(MediaProcessorOutput::Processed {
                stream_id: self.output_stream_id,
                sample: MediaSample::audio_data(data),
            })
        } else if self.eos {
            Ok(MediaProcessorOutput::Finished)
        } else {
            Ok(MediaProcessorOutput::Pending {
                awaiting_stream_id: Some(self.input_stream_id),
            })
        }
    }
}

#[derive(Debug)]
enum AudioDecoderInner {
    Opus(OpusDecoder),
}

impl AudioDecoderInner {
    fn new_opus() -> orfail::Result<Self> {
        OpusDecoder::new().or_fail().map(Self::Opus)
    }

    fn decode(&mut self, data: &AudioData) -> orfail::Result<AudioData> {
        match self {
            Self::Opus(decoder) => decoder.decode(data).or_fail(),
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
    input_stream_id: MediaStreamId,
    output_stream_id: MediaStreamId,
    stats: VideoDecoderStats,
    decoded: VecDeque<VideoFrame>,
    eos: bool,
    inner: VideoDecoderInner,
}

impl VideoDecoder {
    pub fn new(
        input_stream_id: MediaStreamId,
        output_stream_id: MediaStreamId,
        options: VideoDecoderOptions,
    ) -> Self {
        let stats = VideoDecoderStats::default();
        Self {
            input_stream_id,
            output_stream_id,
            stats,
            decoded: VecDeque::new(),
            eos: false,
            inner: VideoDecoderInner::new(options),
        }
    }

    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> {
        let mut engines = Vec::new();
        match codec {
            CodecName::Vp8 | CodecName::Vp9 => {
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_available() {
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
                if shiguredo_nvcodec::is_cuda_available() {
                    engines.push(EngineName::Nvcodec);
                }
                #[cfg(target_os = "macos")]
                {
                    engines.push(EngineName::VideoToolbox);
                }
            }
            CodecName::H265 => {
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_available() {
                    engines.push(EngineName::Nvcodec);
                }
                #[cfg(target_os = "macos")]
                {
                    engines.push(EngineName::VideoToolbox);
                }
            }
            CodecName::Av1 => {
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_available() {
                    engines.push(EngineName::Nvcodec);
                }
                engines.push(EngineName::Dav1d);
            }
            _ => unreachable!(),
        }
        engines
    }
}

impl MediaProcessor for VideoDecoder {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: vec![self.input_stream_id],
            output_stream_ids: vec![self.output_stream_id],
            stats: ProcessorStats::VideoDecoder(self.stats.clone()),
            workload_hint: MediaProcessorWorkloadHint::VIDEO_DECODER,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        if let Some(sample) = input.sample {
            let frame = sample.expect_video_frame().or_fail()?;

            self.stats.total_input_video_frame_count.add(1);
            if let Some(id) = &frame.source_id {
                self.stats.source_id.set_once(|| id.clone());
            }

            self.inner.decode(&frame, &mut self.stats).or_fail()?;
        } else {
            self.eos = true;
            self.inner.finish().or_fail()?;
        };

        while let Some(frame) = self.inner.next_decoded_frame() {
            self.stats.total_output_video_frame_count.add(1);
            self.stats.resolutions.insert(VideoResolution::new(&frame));
            self.decoded.push_back(frame);
        }

        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        if let Some(frame) = self.decoded.pop_front() {
            Ok(MediaProcessorOutput::Processed {
                stream_id: self.output_stream_id,
                sample: MediaSample::video_frame(frame),
            })
        } else if self.eos {
            Ok(MediaProcessorOutput::Finished)
        } else {
            Ok(MediaProcessorOutput::Pending {
                awaiting_stream_id: Some(self.input_stream_id),
            })
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
        stats: &mut VideoDecoderStats,
        options: VideoDecoderOptions,
    ) -> orfail::Result<()> {
        let codec = frame
            .format
            .codec_name()
            .or_fail_with(|()| format!("unexpected video format: {:?}", frame.format))?;
        stats.codec.set(codec);

        let candidate_engines = options
            .engines
            .unwrap_or_else(|| VideoDecoder::get_engines(codec, options.openh264_lib.is_some()));

        let engine = candidate_engines
            .iter()
            .find(|engine| engine.is_available_video_decode_codec(codec))
            .copied();
        if let Some(engine) = engine {
            stats.engine.set(engine);
        }

        match (engine, codec) {
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H264) => {
                *self = NvcodecDecoder::new_h264(&options.decode_params)
                    .or_fail()
                    .map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H265) => {
                *self = NvcodecDecoder::new_h265(&options.decode_params)
                    .or_fail()
                    .map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Vp8) => {
                *self = NvcodecDecoder::new_vp8(&options.decode_params)
                    .or_fail()
                    .map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Vp9) => {
                *self = NvcodecDecoder::new_vp9(&options.decode_params)
                    .or_fail()
                    .map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Av1) => {
                *self = NvcodecDecoder::new_av1(&options.decode_params)
                    .or_fail()
                    .map(Self::Nvcodec)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H264) => {
                *self = VideoToolboxDecoder::new_h264(frame)
                    .or_fail()
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H265) => {
                *self = VideoToolboxDecoder::new_h265(frame)
                    .or_fail()
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            (Some(EngineName::Openh264), CodecName::H264) => {
                let lib = options.openh264_lib.or_fail_with(|()| {
                    "OpenH264 library is required for H.264 decoding".to_owned()
                })?;
                *self = Openh264Decoder::new(lib.clone())
                    .or_fail()
                    .map(Self::Openh264)?;
            }
            #[cfg(feature = "libvpx")]
            (Some(EngineName::Libvpx), CodecName::Vp8) => {
                *self = LibvpxDecoder::new_vp8().or_fail().map(Self::Libvpx)?;
            }
            #[cfg(feature = "libvpx")]
            (Some(EngineName::Libvpx), CodecName::Vp9) => {
                *self = LibvpxDecoder::new_vp9().or_fail().map(Self::Libvpx)?;
            }
            (Some(EngineName::Dav1d), CodecName::Av1) => {
                *self = Dav1dDecoder::new().or_fail().map(Self::Dav1d)?;
            }
            _ => {
                return Err(orfail::Failure::new(format!(
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

    fn decode(&mut self, frame: &VideoFrame, stats: &mut VideoDecoderStats) -> orfail::Result<()> {
        match self {
            Self::Initial { options } => {
                let options = options.clone();
                self.initialize_decoder(frame, stats, options).or_fail()?;
                self.decode(frame, stats).or_fail()
            }
            #[cfg(feature = "libvpx")]
            Self::Libvpx(decoder) => decoder.decode(frame).or_fail(),
            Self::Openh264(decoder) => decoder.decode(frame).or_fail(),
            Self::Dav1d(decoder) => decoder.decode(frame).or_fail(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(decoder) => decoder.decode(frame).or_fail(),
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(decoder) => decoder.decode(frame).or_fail(),
        }
    }

    fn finish(&mut self) -> orfail::Result<()> {
        match self {
            Self::Initial { .. } => {}
            #[cfg(feature = "libvpx")]
            Self::Libvpx(decoder) => decoder.finish().or_fail()?,
            Self::Openh264(decoder) => decoder.finish().or_fail()?,
            Self::Dav1d(decoder) => decoder.finish().or_fail()?,
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(_decoder) => {}
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(decoder) => decoder.finish().or_fail()?,
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
