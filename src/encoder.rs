use std::num::NonZeroUsize;

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

#[cfg(target_os = "macos")]
use crate::encoder_audio_toolbox::AudioToolboxEncoder;
#[cfg(feature = "fdk-aac")]
use crate::encoder_fdk_aac::FdkAacEncoder;
#[cfg(target_os = "macos")]
use crate::encoder_video_toolbox::VideoToolboxEncoder;
use crate::{
    audio::AudioData,
    channel::{self, ErrorFlag},
    encoder_libvpx::LibvpxEncoder,
    encoder_openh264::Openh264Encoder,
    encoder_opus::OpusEncoder,
    encoder_svt_av1::SvtAv1Encoder,
    layout::Layout,
    stats::{AudioEncoderStats, EncoderStats, Seconds, SharedStats, VideoEncoderStats},
    types::{CodecName, EngineName},
    video::VideoFrame,
};

#[derive(Debug)]
pub enum AudioEncoder {
    #[cfg(feature = "fdk-aac")]
    FdkAac(FdkAacEncoder),
    #[cfg(target_os = "macos")]
    AudioToolbox(AudioToolboxEncoder),
    Opus(OpusEncoder),
}

impl AudioEncoder {
    pub fn new_opus(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        OpusEncoder::new(bitrate).map(Self::Opus).or_fail()
    }

    #[cfg(feature = "fdk-aac")]
    pub fn new_fdk_aac(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        FdkAacEncoder::new(bitrate).map(Self::FdkAac).or_fail()
    }

    #[cfg(target_os = "macos")]
    pub fn new_audio_toolbox_aac(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        AudioToolboxEncoder::new(bitrate)
            .map(Self::AudioToolbox)
            .or_fail()
    }

    pub fn encode(&mut self, data: &AudioData) -> orfail::Result<Option<AudioData>> {
        match self {
            #[cfg(feature = "fdk-aac")]
            AudioEncoder::FdkAac(encoder) => encoder.encode(data).or_fail(),
            #[cfg(target_os = "macos")]
            AudioEncoder::AudioToolbox(encoder) => encoder.encode(data).or_fail(),
            AudioEncoder::Opus(encoder) => encoder.encode(data).map(Some).or_fail(),
        }
    }

    pub fn finish(&mut self) -> orfail::Result<Option<AudioData>> {
        match self {
            #[cfg(feature = "fdk-aac")]
            AudioEncoder::FdkAac(encoder) => encoder.finish().or_fail(),
            #[cfg(target_os = "macos")]
            AudioEncoder::AudioToolbox(encoder) => encoder.finish().or_fail(),
            AudioEncoder::Opus(_encoder) => Ok(None),
        }
    }

    fn name(&self) -> EngineName {
        match self {
            #[cfg(feature = "fdk-aac")]
            AudioEncoder::FdkAac(_) => EngineName::FdkAac,
            #[cfg(target_os = "macos")]
            AudioEncoder::AudioToolbox(_) => EngineName::AudioToolbox,
            AudioEncoder::Opus(_) => EngineName::Opus,
        }
    }

    fn codec(&self) -> CodecName {
        match self {
            #[cfg(feature = "fdk-aac")]
            AudioEncoder::FdkAac(_) => CodecName::Aac,
            #[cfg(target_os = "macos")]
            AudioEncoder::AudioToolbox(_) => CodecName::Aac,
            AudioEncoder::Opus(_) => CodecName::Opus,
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

#[derive(Debug)]
pub enum VideoEncoder {
    Libvpx(LibvpxEncoder),
    Openh264(Openh264Encoder),
    SvtAv1(SvtAv1Encoder),
    #[cfg(target_os = "macos")]
    VideoToolbox(VideoToolboxEncoder),
}

impl VideoEncoder {
    pub fn new(layout: &Layout, openh264_lib: Option<Openh264Library>) -> orfail::Result<Self> {
        match layout.video_codec {
            CodecName::Vp8 => VideoEncoder::new_vp8(layout).or_fail(),
            CodecName::Vp9 => VideoEncoder::new_vp9(layout).or_fail(),
            #[cfg(target_os = "macos")]
            CodecName::H264 if openh264_lib.is_none() => {
                VideoEncoder::new_video_toolbox_h264(layout).or_fail()
            }
            CodecName::H264 => {
                let lib = openh264_lib.or_fail()?;
                VideoEncoder::new_openh264(lib, layout).or_fail()
            }
            #[cfg(target_os = "macos")]
            CodecName::H265 => VideoEncoder::new_video_toolbox_h265(layout).or_fail(),
            #[cfg(not(target_os = "macos"))]
            CodecName::H265 => Err(orfail::Failure::new("no available H.265 encoder")),
            CodecName::Av1 => VideoEncoder::new_svt_av1(layout).or_fail(),
            _ => unreachable!(),
        }
    }

    fn new_vp8(layout: &Layout) -> orfail::Result<Self> {
        let encoder = LibvpxEncoder::new_vp8(layout).or_fail()?;
        Ok(Self::Libvpx(encoder))
    }

    fn new_vp9(layout: &Layout) -> orfail::Result<Self> {
        let encoder = LibvpxEncoder::new_vp9(layout).or_fail()?;
        Ok(Self::Libvpx(encoder))
    }

    fn new_openh264(lib: Openh264Library, layout: &Layout) -> orfail::Result<Self> {
        let encoder = Openh264Encoder::new(lib, layout).or_fail()?;
        Ok(Self::Openh264(encoder))
    }

    fn new_svt_av1(layout: &Layout) -> orfail::Result<Self> {
        let encoder = SvtAv1Encoder::new(layout).or_fail()?;
        Ok(Self::SvtAv1(encoder))
    }

    #[cfg(target_os = "macos")]
    fn new_video_toolbox_h264(layout: &Layout) -> orfail::Result<Self> {
        let encoder = VideoToolboxEncoder::new_h264(layout).or_fail()?;
        Ok(Self::VideoToolbox(encoder))
    }

    #[cfg(target_os = "macos")]
    fn new_video_toolbox_h265(layout: &Layout) -> orfail::Result<Self> {
        let encoder = VideoToolboxEncoder::new_h265(layout).or_fail()?;
        Ok(Self::VideoToolbox(encoder))
    }

    pub fn encode(&mut self, frame: VideoFrame) -> orfail::Result<()> {
        match self {
            Self::Libvpx(encoder) => encoder.encode(frame).or_fail(),
            Self::Openh264(encoder) => encoder.encode(frame).or_fail(),
            Self::SvtAv1(encoder) => encoder.encode(frame).or_fail(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.encode(frame).or_fail(),
        }
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        match self {
            Self::Libvpx(encoder) => encoder.finish().or_fail(),
            Self::Openh264(encoder) => encoder.finish().or_fail(),
            Self::SvtAv1(encoder) => encoder.finish().or_fail(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.finish().or_fail(),
        }
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        match self {
            Self::Libvpx(encoder) => encoder.next_encoded_frame(),
            Self::Openh264(encoder) => encoder.next_encoded_frame(),
            Self::SvtAv1(encoder) => encoder.next_encoded_frame(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.next_encoded_frame(),
        }
    }

    pub fn name(&self) -> EngineName {
        match self {
            Self::Libvpx(_) => EngineName::Libvpx,
            Self::Openh264(_) => EngineName::Openh264,
            Self::SvtAv1(_) => EngineName::SvtAv1,
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(_) => EngineName::VideoToolbox,
        }
    }

    fn codec(&self) -> CodecName {
        match self {
            Self::Libvpx(encoder) => encoder.codec(),
            Self::Openh264(_) => CodecName::H264,
            Self::SvtAv1(_) => CodecName::Av1,
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.codec(),
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
                engines.push(EngineName::SvtAv1);
            }
            _ => unreachable!(),
        }
        engines
    }
}

#[derive(Debug)]
pub struct AudioEncoderThread {
    input_rx: channel::Receiver<AudioData>,
    output_tx: channel::SyncSender<AudioData>,
    encoder: AudioEncoder,
    stats: AudioEncoderStats,
}

impl AudioEncoderThread {
    pub fn start(
        error_flag: ErrorFlag,
        input_rx: channel::Receiver<AudioData>,
        encoder: AudioEncoder,
        stats: SharedStats,
    ) -> channel::Receiver<AudioData> {
        let (tx, rx) = channel::sync_channel();
        std::thread::spawn(move || {
            let mut this = Self {
                input_rx,
                output_tx: tx,
                stats: AudioEncoderStats {
                    engine: Some(encoder.name()),
                    codec: Some(encoder.codec()),
                    ..Default::default()
                },
                encoder,
            };
            if let Err(e) = this.run().or_fail() {
                error_flag.set();
                this.stats.error = true;
                log::error!("failed to produce encoded audio stream: {e}");
            }

            stats.with_lock(|stats| {
                stats.encoders.push(EncoderStats::Audio(this.stats));
            });
        });
        rx
    }

    fn run(&mut self) -> orfail::Result<()> {
        while let Some(data) = self.input_rx.recv() {
            self.stats.total_audio_data_count += 1;

            let (encoded, elapsed) = Seconds::try_elapsed(|| self.encoder.encode(&data).or_fail())?;
            self.stats.total_processing_seconds += elapsed;
            let Some(encoded) = encoded else {
                continue;
            };

            if !self.output_tx.send(encoded) {
                // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                log::info!("receiver of encoded audio stream has been closed");
                break;
            }
        }

        if let Some(encoded) = self.encoder.finish().or_fail()?
            && !self.output_tx.send(encoded) {
                log::info!("receiver of encoded audio stream has been closed");
            }

        Ok(())
    }
}

#[derive(Debug)]
pub struct VideoEncoderThread {
    input_rx: channel::Receiver<VideoFrame>,
    output_tx: channel::SyncSender<VideoFrame>,
    encoder: VideoEncoder,
    stats: VideoEncoderStats,
}

impl VideoEncoderThread {
    pub fn start(
        error_flag: ErrorFlag,
        input_rx: channel::Receiver<VideoFrame>,
        encoder: VideoEncoder,
        stats: SharedStats,
    ) -> channel::Receiver<VideoFrame> {
        let (tx, rx) = channel::sync_channel();
        let mut this = Self {
            input_rx,
            output_tx: tx,
            stats: VideoEncoderStats {
                engine: Some(encoder.name()),
                codec: Some(encoder.codec()),
                ..Default::default()
            },
            encoder,
        };
        std::thread::spawn(move || {
            if let Err(e) = this.run().or_fail() {
                error_flag.set();
                this.stats.error = true;
                log::error!("failed to produce encoded video stream: {e}");
            }

            stats.with_lock(|stats| {
                stats.encoders.push(EncoderStats::Video(this.stats));
            });
        });
        rx
    }

    fn run(&mut self) -> orfail::Result<()> {
        while let Some(frame) = self.input_rx.recv() {
            self.stats.total_input_video_frame_count += 1;
            let ((), elapsed) = Seconds::try_elapsed(|| self.encoder.encode(frame).or_fail())?;
            self.stats.total_processing_seconds += elapsed;

            while let Some(encoded) = self.encoder.next_encoded_frame() {
                self.stats.total_output_video_frame_count += 1;
                if !self.output_tx.send(encoded) {
                    // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                    log::info!("receiver of encoded video stream has been closed");
                    return Ok(());
                }
            }
        }

        let ((), elapsed) = Seconds::try_elapsed(|| self.encoder.finish().or_fail())?;
        self.stats.total_processing_seconds += elapsed;

        while let Some(encoded) = self.encoder.next_encoded_frame() {
            self.stats.total_output_video_frame_count += 1;
            if !self.output_tx.send(encoded) {
                // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                log::info!("receiver of encoded video stream has been closed");
                break;
            }
        }

        Ok(())
    }
}
