use std::collections::VecDeque;

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

#[cfg(feature = "nvcodec")]
use crate::decoder_nvcodec::NvcodecDecoder;
#[cfg(target_os = "macos")]
use crate::decoder_video_toolbox::VideoToolboxDecoder;
use crate::{
    audio::AudioData,
    decoder_dav1d::Dav1dDecoder,
    decoder_libvpx::LibvpxDecoder,
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
    video::{VideoFormat, VideoFrame},
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
                engines.push(EngineName::Libvpx);
            }
            CodecName::H264 => {
                if is_openh264_available {
                    engines.push(EngineName::Openh264);
                }
                #[cfg(feature = "nvcodec")]
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
    Libvpx(LibvpxDecoder),
    Openh264(Openh264Decoder),
    #[cfg_attr(feature = "nvcodec", expect(dead_code))]
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

    fn decode(&mut self, frame: &VideoFrame, stats: &mut VideoDecoderStats) -> orfail::Result<()> {
        match self {
            Self::Initial { options } => match frame.format {
                #[cfg(feature = "nvcodec")]
                VideoFormat::H264 | VideoFormat::H264AnnexB if options.openh264_lib.is_none() => {
                    *self = NvcodecDecoder::new_h264(&options.decode_params)
                        .or_fail()
                        .map(Self::Nvcodec)?;
                    stats.engine.set(EngineName::Nvcodec);
                    stats.codec.set(CodecName::H264);
                    self.decode(frame, stats).or_fail()
                }
                #[cfg(target_os = "macos")]
                VideoFormat::H264 | VideoFormat::H264AnnexB if options.openh264_lib.is_none() => {
                    *self = VideoToolboxDecoder::new_h264(frame)
                        .or_fail()
                        .map(Box::new)
                        .map(Self::VideoToolbox)?;
                    stats.engine.set(EngineName::VideoToolbox);
                    stats.codec.set(CodecName::H264);
                    self.decode(frame, stats).or_fail()
                }
                VideoFormat::H264 | VideoFormat::H264AnnexB => {
                    let lib = options
                        .openh264_lib
                        .take()
                        .or_fail_with(|()| "no available H.264 decoder".to_owned())?;
                    *self = Openh264Decoder::new(lib.clone())
                        .or_fail()
                        .map(Self::Openh264)?;
                    stats.engine.set(EngineName::Openh264);
                    stats.codec.set(CodecName::H264);
                    self.decode(frame, stats).or_fail()
                }
                #[cfg(feature = "nvcodec")]
                VideoFormat::H265 => {
                    *self = NvcodecDecoder::new_h265(&options.decode_params)
                        .or_fail()
                        .map(Self::Nvcodec)?;
                    stats.engine.set(EngineName::Nvcodec);
                    stats.codec.set(CodecName::H265);
                    self.decode(frame, stats).or_fail()
                }
                #[cfg(all(not(feature = "nvcodec"), target_os = "macos"))]
                VideoFormat::H265 => {
                    *self = VideoToolboxDecoder::new_h265(frame)
                        .or_fail()
                        .map(Box::new)
                        .map(Self::VideoToolbox)?;
                    stats.engine.set(EngineName::VideoToolbox);
                    stats.codec.set(CodecName::H265);
                    self.decode(frame, stats).or_fail()
                }
                #[cfg(all(not(target_os = "macos"), not(feature = "nvcodec")))]
                VideoFormat::H265 => Err(orfail::Failure::new("no available H.265 decoder")),
                VideoFormat::Vp8 => {
                    *self = LibvpxDecoder::new_vp8().or_fail().map(Self::Libvpx)?;
                    stats.engine.set(EngineName::Libvpx);
                    stats.codec.set(CodecName::Vp8);
                    self.decode(frame, stats).or_fail()
                }
                VideoFormat::Vp9 => {
                    *self = LibvpxDecoder::new_vp9().or_fail().map(Self::Libvpx)?;
                    stats.engine.set(EngineName::Libvpx);
                    stats.codec.set(CodecName::Vp9);
                    self.decode(frame, stats).or_fail()
                }
                #[cfg(feature = "nvcodec")]
                VideoFormat::Av1 => {
                    *self = NvcodecDecoder::new_av1(&options.decode_params)
                        .or_fail()
                        .map(Self::Nvcodec)?;
                    stats.engine.set(EngineName::Nvcodec);
                    stats.codec.set(CodecName::Av1);
                    self.decode(frame, stats).or_fail()
                }
                #[cfg(not(feature = "nvcodec"))]
                VideoFormat::Av1 => {
                    *self = Dav1dDecoder::new().or_fail().map(Self::Dav1d)?;
                    stats.engine.set(EngineName::Dav1d);
                    stats.codec.set(CodecName::Av1);
                    self.decode(frame, stats).or_fail()
                }
                VideoFormat::I420 => {
                    // デコーダーに非圧縮が渡されるのは想定外
                    Err(orfail::Failure::new(format!(
                        "unexpected video format: {:?}",
                        frame.format
                    )))
                }
            },
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
