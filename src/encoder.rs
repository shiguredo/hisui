use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::Arc;

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

#[cfg(target_os = "macos")]
use crate::encoder_audio_toolbox::AudioToolboxEncoder;
#[cfg(feature = "fdk-aac")]
use crate::encoder_fdk_aac::FdkAacEncoder;
#[cfg(feature = "nvcodec")]
use crate::encoder_nvcodec::NvcodecEncoder;
#[cfg(target_os = "macos")]
use crate::encoder_video_toolbox::VideoToolboxEncoder;
use crate::{
    audio::AudioData,
    encoder_libvpx::LibvpxEncoder,
    encoder_openh264::Openh264Encoder,
    encoder_opus::OpusEncoder,
    encoder_svt_av1::SvtAv1Encoder,
    layout::Layout,
    layout_encode_params::LayoutEncodeParams,
    media::{MediaSample, MediaStreamId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    stats::{AudioEncoderStats, ProcessorStats, VideoEncoderStats},
    types::{CodecName, EngineName, EvenUsize},
    video::{FrameRate, VideoFrame},
};

#[derive(Debug)]
pub struct AudioEncoder {
    input_stream_id: MediaStreamId,
    output_stream_id: MediaStreamId,
    stats: AudioEncoderStats,
    encoded: VecDeque<AudioData>,
    eos: bool,
    inner: AudioEncoderInner,
}

impl AudioEncoder {
    pub fn new(
        codec: CodecName,
        bitrate: NonZeroUsize,
        input_stream_id: MediaStreamId,
        output_stream_id: MediaStreamId,
    ) -> orfail::Result<Self> {
        match codec {
            #[cfg(feature = "fdk-aac")]
            CodecName::Aac => {
                AudioEncoder::new_fdk_aac(input_stream_id, output_stream_id, bitrate).or_fail()
            }
            #[cfg(all(not(feature = "fdk-aac"), target_os = "macos"))]
            CodecName::Aac => {
                AudioEncoder::new_audio_toolbox_aac(input_stream_id, output_stream_id, bitrate)
                    .or_fail()
            }
            #[cfg(all(not(feature = "fdk-aac"), not(target_os = "macos")))]
            CodecName::Aac => Err(orfail::Failure::new("AAC output is not supported")),
            CodecName::Opus => {
                AudioEncoder::new_opus(input_stream_id, output_stream_id, bitrate).or_fail()
            }
            _ => unreachable!(),
        }
    }

    fn new_opus(
        input_stream_id: MediaStreamId,
        output_stream_id: MediaStreamId,
        bitrate: NonZeroUsize,
    ) -> orfail::Result<Self> {
        let stats = AudioEncoderStats::new(EngineName::Opus, CodecName::Opus);
        Ok(Self {
            input_stream_id,
            output_stream_id,
            stats,
            encoded: VecDeque::new(),
            eos: false,
            inner: AudioEncoderInner::new_opus(bitrate).or_fail()?,
        })
    }

    #[cfg(feature = "fdk-aac")]
    fn new_fdk_aac(
        input_stream_id: MediaStreamId,
        output_stream_id: MediaStreamId,
        bitrate: NonZeroUsize,
    ) -> orfail::Result<Self> {
        let stats = AudioEncoderStats::new(EngineName::FdkAac, CodecName::Aac);
        Ok(Self {
            input_stream_id,
            output_stream_id,
            stats,
            encoded: VecDeque::new(),
            eos: false,
            inner: AudioEncoderInner::new_fdk_aac(bitrate).or_fail()?,
        })
    }

    #[cfg(target_os = "macos")]
    fn new_audio_toolbox_aac(
        input_stream_id: MediaStreamId,
        output_stream_id: MediaStreamId,
        bitrate: NonZeroUsize,
    ) -> orfail::Result<Self> {
        let stats = AudioEncoderStats::new(EngineName::AudioToolbox, CodecName::Aac);
        Ok(Self {
            input_stream_id,
            output_stream_id,
            stats,
            encoded: VecDeque::new(),
            eos: false,
            inner: AudioEncoderInner::new_audio_toolbox_aac(bitrate).or_fail()?,
        })
    }

    pub fn name(&self) -> EngineName {
        match &self.inner {
            #[cfg(feature = "fdk-aac")]
            AudioEncoderInner::FdkAac(_) => EngineName::FdkAac,
            #[cfg(target_os = "macos")]
            AudioEncoderInner::AudioToolbox(_) => EngineName::AudioToolbox,
            AudioEncoderInner::Opus(_) => EngineName::Opus,
        }
    }

    pub fn codec(&self) -> CodecName {
        match &self.inner {
            #[cfg(feature = "fdk-aac")]
            AudioEncoderInner::FdkAac(_) => CodecName::Aac,
            #[cfg(target_os = "macos")]
            AudioEncoderInner::AudioToolbox(_) => CodecName::Aac,
            AudioEncoderInner::Opus(_) => CodecName::Opus,
        }
    }

    pub fn get_engines(codec: CodecName) -> Vec<EngineName> {
        let mut engines = Vec::new();
        match codec {
            CodecName::Aac => {
                #[cfg(feature = "fdk-aac")]
                {
                    engines.push(EngineName::FdkAac);
                }
                #[cfg(target_os = "macos")]
                {
                    engines.push(EngineName::AudioToolbox);
                }
            }
            CodecName::Opus => engines.push(EngineName::Opus),
            _ => unreachable!(),
        }
        engines
    }
}

impl MediaProcessor for AudioEncoder {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: vec![self.input_stream_id],
            output_stream_ids: vec![self.output_stream_id],
            stats: ProcessorStats::AudioEncoder(self.stats.clone()),
            workload_hint: MediaProcessorWorkloadHint::AUDIO_ENCODER,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        let encoded = if let Some(sample) = input.sample {
            let data = sample.expect_audio_data().or_fail()?;
            self.inner.encode(&data).or_fail()?
        } else {
            self.eos = true;
            self.inner.finish().or_fail()?
        };

        if let Some(encoded) = encoded {
            self.stats.total_audio_data_count.add(1);
            self.encoded.push_back(encoded);
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        if let Some(data) = self.encoded.pop_front() {
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
enum AudioEncoderInner {
    #[cfg(feature = "fdk-aac")]
    FdkAac(FdkAacEncoder),
    #[cfg(target_os = "macos")]
    AudioToolbox(AudioToolboxEncoder),
    Opus(OpusEncoder),
}

impl AudioEncoderInner {
    fn new_opus(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        OpusEncoder::new(bitrate).map(Self::Opus).or_fail()
    }

    #[cfg(feature = "fdk-aac")]
    fn new_fdk_aac(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        FdkAacEncoder::new(bitrate).map(Self::FdkAac).or_fail()
    }

    #[cfg(target_os = "macos")]
    fn new_audio_toolbox_aac(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        AudioToolboxEncoder::new(bitrate)
            .map(Self::AudioToolbox)
            .or_fail()
    }

    fn encode(&mut self, data: &AudioData) -> orfail::Result<Option<AudioData>> {
        match self {
            #[cfg(feature = "fdk-aac")]
            Self::FdkAac(encoder) => encoder.encode(data).or_fail(),
            #[cfg(target_os = "macos")]
            Self::AudioToolbox(encoder) => encoder.encode(data).or_fail(),
            Self::Opus(encoder) => encoder.encode(data).map(Some).or_fail(),
        }
    }

    fn finish(&mut self) -> orfail::Result<Option<AudioData>> {
        match self {
            #[cfg(feature = "fdk-aac")]
            Self::FdkAac(encoder) => encoder.finish().or_fail(),
            #[cfg(target_os = "macos")]
            Self::AudioToolbox(encoder) => encoder.finish().or_fail(),
            Self::Opus(_encoder) => Ok(None),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VideoEncoderOptions {
    pub codec: CodecName,
    pub bitrate: usize,
    pub width: EvenUsize,
    pub height: EvenUsize,
    pub frame_rate: FrameRate,
    pub encode_params: LayoutEncodeParams,
}

impl VideoEncoderOptions {
    pub fn from_layout(layout: &Layout) -> Self {
        Self {
            codec: layout.video_codec,
            bitrate: layout.video_bitrate_bps(),
            width: layout.resolution.width(),
            height: layout.resolution.height(),
            frame_rate: layout.frame_rate,
            encode_params: layout.encode_params.clone(),
        }
    }
}

#[derive(Debug)]
pub struct VideoEncoder {
    input_stream_id: MediaStreamId,
    output_stream_id: MediaStreamId,
    stats: VideoEncoderStats,
    encoded: VecDeque<VideoFrame>,
    eos: bool,
    inner: VideoEncoderInner,
}

impl VideoEncoder {
    pub fn new(
        options: &VideoEncoderOptions,
        input_stream_id: MediaStreamId,
        output_stream_id: MediaStreamId,
        openh264_lib: Option<Openh264Library>,
    ) -> orfail::Result<Self> {
        let inner = match options.codec {
            CodecName::Vp8 => VideoEncoderInner::new_vp8(options).or_fail()?,
            CodecName::Vp9 => VideoEncoderInner::new_vp9(options).or_fail()?,
            #[cfg(feature = "nvcodec")]
            CodecName::H264 if openh264_lib.is_none() => {
                VideoEncoderInner::new_nvcodec_h264(options).or_fail()?
            }
            #[cfg(target_os = "macos")]
            CodecName::H264 if openh264_lib.is_none() => {
                VideoEncoderInner::new_video_toolbox_h264(options).or_fail()?
            }
            CodecName::H264 => {
                let lib = openh264_lib.or_fail_with(|()| {
                    concat!(
                        "OpenH264 library is required for H.264 encoding. ",
                        "Please specify the library path using --openh264 command line argument or ",
                        "HISUI_OPENH264_PATH environment variable.").to_owned()
                })?;
                VideoEncoderInner::new_openh264(lib, options).or_fail()?
            }
            #[cfg(feature = "nvcodec")]
            CodecName::H265 => VideoEncoderInner::new_nvcodec_h265(options).or_fail()?,
            #[cfg(target_os = "macos")]
            CodecName::H265 => VideoEncoderInner::new_video_toolbox_h265(options).or_fail()?,
            #[cfg(all(not(target_os = "macos"), not(feature = "nvcodec")))]
            CodecName::H265 => return Err(orfail::Failure::new("no available H.265 encoder")),
            #[cfg(feature = "nvcodec")]
            CodecName::Av1 => VideoEncoderInner::new_nvcodec_av1(options).or_fail()?,
            CodecName::Av1 => VideoEncoderInner::new_svt_av1(options).or_fail()?,
            _ => unreachable!(),
        };

        let stats = VideoEncoderStats::new(inner.name(), inner.codec());

        Ok(Self {
            input_stream_id,
            output_stream_id,
            stats,
            encoded: VecDeque::new(),
            eos: false,
            inner,
        })
    }

    pub fn name(&self) -> EngineName {
        self.inner.name()
    }

    pub fn codec(&self) -> CodecName {
        self.inner.codec()
    }

    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> {
        let mut engines = Vec::new();
        match codec {
            CodecName::Vp8 | CodecName::Vp9 => {
                engines.push(EngineName::Libvpx);
            }
            CodecName::H264 => {
                if is_openh264_available {
                    engines.push(EngineName::Openh264);
                }
                #[cfg(target_os = "nvcodec")]
                {
                    engines.push(EngineName::Nvcodec);
                }
                #[cfg(target_os = "macos")]
                {
                    engines.push(EngineName::VideoToolbox);
                }
            }
            CodecName::H265 => {
                #[cfg(feature = "nvcodec")]
                {
                    engines.push(EngineName::Nvcodec);
                }
                #[cfg(target_os = "macos")]
                {
                    engines.push(EngineName::VideoToolbox);
                }
            }
            CodecName::Av1 => {
                #[cfg(feature = "nvcodec")]
                {
                    engines.push(EngineName::Nvcodec);
                }
                engines.push(EngineName::SvtAv1);
            }
            _ => unreachable!(),
        }
        engines
    }

    pub fn encoder_stats(&self) -> &VideoEncoderStats {
        &self.stats
    }
}

impl MediaProcessor for VideoEncoder {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: vec![self.input_stream_id],
            output_stream_ids: vec![self.output_stream_id],
            stats: ProcessorStats::VideoEncoder(self.stats.clone()),
            workload_hint: MediaProcessorWorkloadHint::VIDEO_ENCODER,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        if let Some(sample) = input.sample {
            let frame = sample.expect_video_frame().or_fail()?;
            self.stats.total_input_video_frame_count.add(1);
            self.inner.encode(frame).or_fail()?;
        } else {
            self.eos = true;
            self.inner.finish().or_fail()?;
        }

        while let Some(encoded) = self.inner.next_encoded_frame() {
            self.stats.total_output_video_frame_count.add(1);
            self.encoded.push_back(encoded);
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        if let Some(frame) = self.encoded.pop_front() {
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
enum VideoEncoderInner {
    Libvpx(LibvpxEncoder),
    Openh264(Openh264Encoder),
    #[cfg_attr(feature = "nvcodec", expect(dead_code))]
    SvtAv1(SvtAv1Encoder),
    #[cfg(target_os = "macos")]
    VideoToolbox(VideoToolboxEncoder),
    #[cfg(feature = "nvcodec")]
    Nvcodec(Box<NvcodecEncoder>), // Box は clippy::large_enum_variant 対策
}

impl VideoEncoderInner {
    fn new_vp8(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let encoder = LibvpxEncoder::new_vp8(options).or_fail()?;
        Ok(Self::Libvpx(encoder))
    }

    fn new_vp9(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let encoder = LibvpxEncoder::new_vp9(options).or_fail()?;
        Ok(Self::Libvpx(encoder))
    }

    fn new_openh264(lib: Openh264Library, options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let encoder = Openh264Encoder::new(lib, options).or_fail()?;
        Ok(Self::Openh264(encoder))
    }

    fn new_svt_av1(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let encoder = SvtAv1Encoder::new(options).or_fail()?;
        Ok(Self::SvtAv1(encoder))
    }

    #[cfg(target_os = "macos")]
    fn new_video_toolbox_h264(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let encoder = VideoToolboxEncoder::new_h264(options).or_fail()?;
        Ok(Self::VideoToolbox(encoder))
    }

    #[cfg(target_os = "macos")]
    fn new_video_toolbox_h265(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let encoder = VideoToolboxEncoder::new_h265(options).or_fail()?;
        Ok(Self::VideoToolbox(encoder))
    }

    #[cfg(feature = "nvcodec")]
    fn new_nvcodec_h265(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let encoder =
            NvcodecEncoder::new_h265(options.width.get(), options.height.get()).or_fail()?;
        Ok(Self::Nvcodec(Box::new(encoder)))
    }

    #[cfg(feature = "nvcodec")]
    fn new_nvcodec_h264(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let encoder =
            NvcodecEncoder::new_h264(options.width.get(), options.height.get()).or_fail()?;
        Ok(Self::Nvcodec(Box::new(encoder)))
    }

    #[cfg(feature = "nvcodec")]
    fn new_nvcodec_av1(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let encoder =
            NvcodecEncoder::new_av1(options.width.get(), options.height.get()).or_fail()?;
        Ok(Self::Nvcodec(Box::new(encoder)))
    }

    fn encode(&mut self, frame: Arc<VideoFrame>) -> orfail::Result<()> {
        match self {
            Self::Libvpx(encoder) => encoder.encode(frame).or_fail(),
            Self::Openh264(encoder) => encoder.encode(frame).or_fail(),
            Self::SvtAv1(encoder) => encoder.encode(frame).or_fail(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.encode(frame).or_fail(),
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(encoder) => encoder.encode(&frame).or_fail(),
        }
    }

    fn finish(&mut self) -> orfail::Result<()> {
        match self {
            Self::Libvpx(encoder) => encoder.finish().or_fail(),
            Self::Openh264(encoder) => encoder.finish().or_fail(),
            Self::SvtAv1(encoder) => encoder.finish().or_fail(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.finish().or_fail(),
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(encoder) => encoder.finish().or_fail(),
        }
    }

    fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        match self {
            Self::Libvpx(encoder) => encoder.next_encoded_frame(),
            Self::Openh264(encoder) => encoder.next_encoded_frame(),
            Self::SvtAv1(encoder) => encoder.next_encoded_frame(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.next_encoded_frame(),
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(encoder) => encoder.next_encoded_frame(),
        }
    }

    fn name(&self) -> EngineName {
        match self {
            Self::Libvpx(_) => EngineName::Libvpx,
            Self::Openh264(_) => EngineName::Openh264,
            Self::SvtAv1(_) => EngineName::SvtAv1,
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(_) => EngineName::VideoToolbox,
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(_) => EngineName::Nvcodec,
        }
    }

    fn codec(&self) -> CodecName {
        match self {
            Self::Libvpx(encoder) => encoder.codec(),
            Self::Openh264(_) => CodecName::H264,
            Self::SvtAv1(_) => CodecName::Av1,
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.codec(),
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(_) => CodecName::H265,
        }
    }
}
