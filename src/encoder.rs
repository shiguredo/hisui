use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::Arc;

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
    media::{MediaSample, MediaStreamId},
    processor::{MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec},
    stats::{AudioEncoderStats, ProcessorStats, Seconds, SharedStats, VideoEncoderStats},
    types::{CodecName, EngineName},
    video::VideoFrame,
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
    pub fn new_opus(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        // TODO: スケジューリングスレッドの導入タイミングでちゃんとする
        let input_stream_id = MediaStreamId::new(0);
        let output_stream_id = MediaStreamId::new(1);

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
    pub fn new_fdk_aac(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        // TODO: スケジューリングスレッドの導入タイミングでちゃんとする
        let input_stream_id = MediaStreamId::new(0);
        let output_stream_id = MediaStreamId::new(1);

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
    pub fn new_audio_toolbox_aac(bitrate: NonZeroUsize) -> orfail::Result<Self> {
        // TODO: スケジューリングスレッドの導入タイミングでちゃんとする
        let input_stream_id = MediaStreamId::new(0);
        let output_stream_id = MediaStreamId::new(1);

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

    // TODO: スケジューリングスレッドの導入タイミングで削除する
    pub fn encode(&mut self, data: &AudioData) -> orfail::Result<Option<AudioData>> {
        let input = MediaProcessorInput {
            stream_id: self.input_stream_id,
            sample: Some(MediaSample::audio_data(data.clone())),
        };
        self.process_input(input).or_fail()?;
        let MediaProcessorOutput::Processed { sample, .. } = self.process_output().or_fail()?
        else {
            return Ok(None);
        };
        let encoded = sample.expect_audio_data().or_fail()?;
        Ok(Some(std::sync::Arc::into_inner(encoded).or_fail()?))
    }

    // TODO: スケジューリングスレッドの導入タイミングで削除する
    pub fn finish(&mut self) -> orfail::Result<Option<AudioData>> {
        let input = MediaProcessorInput {
            stream_id: self.input_stream_id,
            sample: None,
        };
        self.process_input(input).or_fail()?;
        let MediaProcessorOutput::Processed { sample, .. } = self.process_output().or_fail()?
        else {
            return Ok(None);
        };
        let encoded = sample.expect_audio_data().or_fail()?;
        Ok(Some(std::sync::Arc::into_inner(encoded).or_fail()?))
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
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        let (encoded, elapsed) = if let Some(sample) = input.sample {
            let data = sample.expect_audio_data().or_fail()?;
            Seconds::try_elapsed(|| self.inner.encode(&data).or_fail())
        } else {
            self.eos = true;
            Seconds::try_elapsed(|| self.inner.finish().or_fail())
        }?;

        // TODO: プロセッサ実行スレッドの導入タイミングで、時間計測はそっちに移動する
        self.stats.total_processing_seconds.add(elapsed);

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
                awaiting_stream_id: self.input_stream_id,
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
    pub fn new(layout: &Layout, openh264_lib: Option<Openh264Library>) -> orfail::Result<Self> {
        // TODO: スケジューリングスレッドの導入タイミングでちゃんとする
        let input_stream_id = MediaStreamId::new(0);
        let output_stream_id = MediaStreamId::new(1);

        let inner = match layout.video_codec {
            CodecName::Vp8 => VideoEncoderInner::new_vp8(layout).or_fail()?,
            CodecName::Vp9 => VideoEncoderInner::new_vp9(layout).or_fail()?,
            #[cfg(target_os = "macos")]
            CodecName::H264 if openh264_lib.is_none() => {
                VideoEncoderInner::new_video_toolbox_h264(layout).or_fail()?
            }
            CodecName::H264 => {
                let lib = openh264_lib.or_fail()?;
                VideoEncoderInner::new_openh264(lib, layout).or_fail()?
            }
            #[cfg(target_os = "macos")]
            CodecName::H265 => VideoEncoderInner::new_video_toolbox_h265(layout).or_fail()?,
            #[cfg(not(target_os = "macos"))]
            CodecName::H265 => return Err(orfail::Failure::new("no available H.265 encoder")),
            CodecName::Av1 => VideoEncoderInner::new_svt_av1(layout).or_fail()?,
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

    // TODO: スケジューリングスレッドの導入タイミングで削除する
    pub fn encode(&mut self, frame: VideoFrame) -> orfail::Result<()> {
        let input = MediaProcessorInput {
            stream_id: self.input_stream_id,
            sample: Some(MediaSample::video_frame(frame)),
        };
        self.process_input(input).or_fail()
    }

    // TODO: スケジューリングスレッドの導入タイミングで削除する
    pub fn finish(&mut self) -> orfail::Result<()> {
        let input = MediaProcessorInput {
            stream_id: self.input_stream_id,
            sample: None,
        };
        self.process_input(input).or_fail()
    }

    // TODO: スケジューリングスレッドの導入タイミングで削除する
    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        let Ok(MediaProcessorOutput::Processed { sample, .. }) = self.process_output() else {
            return None;
        };
        let encoded = sample.expect_video_frame().ok()?;
        std::sync::Arc::into_inner(encoded)
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

impl MediaProcessor for VideoEncoder {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: vec![self.input_stream_id],
            output_stream_ids: vec![self.output_stream_id],
            stats: ProcessorStats::VideoEncoder(self.stats.clone()),
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        let ((), elapsed) = if let Some(sample) = input.sample {
            let frame = sample.expect_video_frame().or_fail()?;
            self.stats.total_input_video_frame_count.add(1);
            Seconds::try_elapsed(|| self.inner.encode(frame).or_fail())
        } else {
            self.eos = true;
            Seconds::try_elapsed(|| self.inner.finish().or_fail())
        }?;

        // TODO: プロセッサ実行スレッドの導入タイミングで、時間計測はそっちに移動する
        self.stats.total_processing_seconds.add(elapsed);

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
                awaiting_stream_id: self.input_stream_id,
            })
        }
    }
}

#[derive(Debug)]
enum VideoEncoderInner {
    Libvpx(LibvpxEncoder),
    Openh264(Openh264Encoder),
    SvtAv1(SvtAv1Encoder),
    #[cfg(target_os = "macos")]
    VideoToolbox(VideoToolboxEncoder),
}

impl VideoEncoderInner {
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

    fn encode(&mut self, frame: Arc<VideoFrame>) -> orfail::Result<()> {
        match self {
            Self::Libvpx(encoder) => encoder.encode(frame).or_fail(),
            Self::Openh264(encoder) => encoder.encode(frame).or_fail(),
            Self::SvtAv1(encoder) => encoder.encode(frame).or_fail(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.encode(frame).or_fail(),
        }
    }

    fn finish(&mut self) -> orfail::Result<()> {
        match self {
            Self::Libvpx(encoder) => encoder.finish().or_fail(),
            Self::Openh264(encoder) => encoder.finish().or_fail(),
            Self::SvtAv1(encoder) => encoder.finish().or_fail(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.finish().or_fail(),
        }
    }

    fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        match self {
            Self::Libvpx(encoder) => encoder.next_encoded_frame(),
            Self::Openh264(encoder) => encoder.next_encoded_frame(),
            Self::SvtAv1(encoder) => encoder.next_encoded_frame(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.next_encoded_frame(),
        }
    }

    fn name(&self) -> EngineName {
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
                stats: AudioEncoderStats::new(encoder.name(), encoder.codec()),
                encoder,
            };
            if let Err(e) = this.run().or_fail() {
                error_flag.set();
                this.stats.error.set(true);
                log::error!("failed to produce encoded audio stream: {e}");
            }

            stats.with_lock(|stats| {
                stats
                    .processors
                    .push(ProcessorStats::AudioEncoder(this.stats));
            });
        });
        rx
    }

    fn run(&mut self) -> orfail::Result<()> {
        while let Some(data) = self.input_rx.recv() {
            self.stats.total_audio_data_count.add(1);

            let (encoded, elapsed) = Seconds::try_elapsed(|| self.encoder.encode(&data).or_fail())?;
            self.stats.total_processing_seconds.add(elapsed);
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
            && !self.output_tx.send(encoded)
        {
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
            stats: VideoEncoderStats::new(encoder.name(), encoder.codec()),
            encoder,
        };
        std::thread::spawn(move || {
            if let Err(e) = this.run().or_fail() {
                error_flag.set();
                this.stats.error.set(true);
                log::error!("failed to produce encoded video stream: {e}");
            }

            stats.with_lock(|stats| {
                stats
                    .processors
                    .push(ProcessorStats::VideoEncoder(this.stats));
            });
        });
        rx
    }

    fn run(&mut self) -> orfail::Result<()> {
        while let Some(frame) = self.input_rx.recv() {
            self.stats.total_input_video_frame_count.add(1);
            let ((), elapsed) = Seconds::try_elapsed(|| self.encoder.encode(frame).or_fail())?;
            self.stats.total_processing_seconds.add(elapsed);

            while let Some(encoded) = self.encoder.next_encoded_frame() {
                self.stats.total_output_video_frame_count.add(1);
                if !self.output_tx.send(encoded) {
                    // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                    log::info!("receiver of encoded video stream has been closed");
                    return Ok(());
                }
            }
        }

        let ((), elapsed) = Seconds::try_elapsed(|| self.encoder.finish().or_fail())?;
        self.stats.total_processing_seconds.add(elapsed);

        while let Some(encoded) = self.encoder.next_encoded_frame() {
            self.stats.total_output_video_frame_count.add(1);
            if !self.output_tx.send(encoded) {
                // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                log::info!("receiver of encoded video stream has been closed");
                break;
            }
        }

        Ok(())
    }
}
