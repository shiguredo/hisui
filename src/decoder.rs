use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

#[cfg(target_os = "macos")]
use crate::decoder_video_toolbox::VideoToolboxDecoder;
use crate::{
    audio::AudioData,
    decoder_dav1d::Dav1dDecoder,
    decoder_libvpx::LibvpxDecoder,
    decoder_openh264::Openh264Decoder,
    decoder_opus::OpusDecoder,
    stats::VideoDecoderStats,
    types::{CodecEngines, CodecName, EngineName},
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub enum AudioDecoder {
    Opus(OpusDecoder),
}

impl AudioDecoder {
    pub fn new_opus() -> orfail::Result<Self> {
        OpusDecoder::new().or_fail().map(Self::Opus)
    }

    pub fn decode(&mut self, data: &AudioData) -> orfail::Result<AudioData> {
        match self {
            AudioDecoder::Opus(decoder) => decoder.decode(data).or_fail(),
        }
    }

    pub fn get_engines(codec: CodecName) -> Vec<EngineName> {
        match codec {
            CodecName::Aac => vec![],
            CodecName::Opus => vec![EngineName::Opus],
            _ => unreachable!(),
        }
    }

    // TODO: remove
    pub fn update_codec_engines(engines: &mut CodecEngines) {
        engines.insert_decoder(CodecName::Opus, EngineName::Opus);
    }
}

#[derive(Debug, Default, Clone)]
pub struct VideoDecoderOptions {
    pub openh264_lib: Option<Openh264Library>,
}

#[derive(Debug)]
pub enum VideoDecoder {
    Initial {
        options: VideoDecoderOptions,
    },
    Libvpx(LibvpxDecoder),
    Openh264(Openh264Decoder),
    Dav1d(Dav1dDecoder),
    #[cfg(target_os = "macos")]
    VideoToolbox(VideoToolboxDecoder),
}

impl VideoDecoder {
    pub fn new(options: VideoDecoderOptions) -> Self {
        // [NOTE] 最初の映像フレームが来た時点で実際のデコーダーに切り替わる
        Self::Initial { options }
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
                #[cfg(target_os = "macos")]
                {
                    engines.push(EngineName::VideoToolbox);
                }
            }
            CodecName::H265 => {
                #[cfg(target_os = "macos")]
                {
                    engines.push(EngineName::VideoToolbox);
                }
            }
            CodecName::Av1 => {
                engines.push(EngineName::Dav1d);
            }
            _ => unreachable!(),
        }
        engines
    }

    // TODO: delete
    pub fn update_codec_engines(engines: &mut CodecEngines, options: VideoDecoderOptions) {
        engines.insert_decoder(CodecName::Vp8, EngineName::Libvpx);
        engines.insert_decoder(CodecName::Vp9, EngineName::Libvpx);
        engines.insert_decoder(CodecName::Av1, EngineName::Dav1d);

        if options.openh264_lib.is_some() {
            engines.insert_decoder(CodecName::H264, EngineName::Openh264);
        }

        #[cfg(target_os = "macos")]
        {
            engines.insert_decoder(CodecName::H264, EngineName::VideoToolbox);
            engines.insert_decoder(CodecName::H265, EngineName::VideoToolbox);
        }
    }

    pub fn decode(
        &mut self,
        frame: VideoFrame,
        stats: &mut VideoDecoderStats,
    ) -> orfail::Result<()> {
        match self {
            VideoDecoder::Initial { options } => match frame.format {
                #[cfg(target_os = "macos")]
                VideoFormat::H264 | VideoFormat::H264AnnexB if options.openh264_lib.is_none() => {
                    *self = VideoToolboxDecoder::new_h264(&frame)
                        .or_fail()
                        .map(Self::VideoToolbox)?;
                    stats.engine = Some(EngineName::VideoToolbox);
                    stats.codec = Some(CodecName::H264);
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
                    stats.engine = Some(EngineName::Openh264);
                    stats.codec = Some(CodecName::H264);
                    self.decode(frame, stats).or_fail()
                }
                #[cfg(target_os = "macos")]
                VideoFormat::H265 => {
                    *self = VideoToolboxDecoder::new_h265(&frame)
                        .or_fail()
                        .map(Self::VideoToolbox)?;
                    stats.engine = Some(EngineName::VideoToolbox);
                    stats.codec = Some(CodecName::H265);
                    self.decode(frame, stats).or_fail()
                }
                #[cfg(not(target_os = "macos"))]
                VideoFormat::H265 => Err(orfail::Failure::new("no available H.265 decoder")),
                VideoFormat::Vp8 => {
                    *self = LibvpxDecoder::new_vp8().or_fail().map(Self::Libvpx)?;
                    stats.engine = Some(EngineName::Libvpx);
                    stats.codec = Some(CodecName::Vp8);
                    self.decode(frame, stats).or_fail()
                }
                VideoFormat::Vp9 => {
                    *self = LibvpxDecoder::new_vp9().or_fail().map(Self::Libvpx)?;
                    stats.engine = Some(EngineName::Libvpx);
                    stats.codec = Some(CodecName::Vp9);
                    self.decode(frame, stats).or_fail()
                }
                VideoFormat::Av1 => {
                    *self = Dav1dDecoder::new().or_fail().map(Self::Dav1d)?;
                    stats.engine = Some(EngineName::Dav1d);
                    stats.codec = Some(CodecName::Av1);
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
            VideoDecoder::Libvpx(decoder) => decoder.decode(frame).or_fail(),
            VideoDecoder::Openh264(decoder) => decoder.decode(frame).or_fail(),
            VideoDecoder::Dav1d(decoder) => decoder.decode(frame).or_fail(),
            #[cfg(target_os = "macos")]
            VideoDecoder::VideoToolbox(decoder) => decoder.decode(frame).or_fail(),
        }
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        match self {
            VideoDecoder::Initial { .. } => {}
            VideoDecoder::Libvpx(decoder) => decoder.finish().or_fail()?,
            VideoDecoder::Openh264(decoder) => decoder.finish().or_fail()?,
            VideoDecoder::Dav1d(decoder) => decoder.finish().or_fail()?,
            #[cfg(target_os = "macos")]
            VideoDecoder::VideoToolbox(_decoder) => {}
        }
        Ok(())
    }

    pub fn next_decoded_frame(&mut self) -> Option<VideoFrame> {
        match self {
            VideoDecoder::Initial { .. } => None,
            VideoDecoder::Libvpx(decoder) => decoder.next_decoded_frame(),
            VideoDecoder::Openh264(decoder) => decoder.next_decoded_frame(),
            VideoDecoder::Dav1d(decoder) => decoder.next_decoded_frame(),
            #[cfg(target_os = "macos")]
            VideoDecoder::VideoToolbox(decoder) => decoder.next_decoded_frame(),
        }
    }
}
