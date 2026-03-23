#[cfg(target_os = "macos")]
pub mod audio_toolbox;
#[cfg(feature = "fdk-aac")]
pub mod fdk_aac;
pub mod libvpx;
#[cfg(feature = "nvcodec")]
pub mod nvcodec;
pub mod openh264;
pub mod opus;
pub mod svt_av1;
#[cfg(target_os = "macos")]
pub mod video_toolbox;

use std::collections::VecDeque;
use std::num::NonZeroUsize;

use shiguredo_mp4::boxes::SampleEntry;
use shiguredo_openh264::Openh264Library;

#[cfg(target_os = "macos")]
use self::audio_toolbox::AudioToolboxEncoder;
#[cfg(feature = "fdk-aac")]
use self::fdk_aac::FdkAacEncoder;
use self::libvpx::LibvpxEncoder;
#[cfg(feature = "nvcodec")]
use self::nvcodec::NvcodecEncoder;
use self::openh264::Openh264Encoder;
use self::opus::OpusEncoder;
use self::svt_av1::SvtAv1Encoder;
#[cfg(target_os = "macos")]
use self::video_toolbox::VideoToolboxEncoder;
use crate::{
    Error, Message, ProcessorHandle, Result, TrackId,
    audio::converter::{AudioConverter, AudioConverterBuilder},
    audio::{AudioFormat, AudioFrame, Channels, SampleRate},
    media::MediaFrame,
    types::{CodecName, EngineName, EvenUsize},
    video::{FrameRate, RawVideoFrame, VideoFrame},
};

#[derive(Debug)]
pub struct AudioEncoder {
    total_audio_data_count_metric: crate::stats::StatsCounter,
    _error_flag: crate::stats::StatsFlag,
    encoded: VecDeque<AudioFrame>,
    eos: bool,
    converter: AudioConverter,
    inner: AudioEncoderInner,
}

enum EncoderRunOutput {
    Processed(MediaFrame),
    Pending,
    Finished,
}

impl AudioEncoder {
    pub fn new(
        codec: CodecName,
        bitrate: NonZeroUsize,
        #[cfg(feature = "fdk-aac")] fdk_aac_lib: Option<shiguredo_fdk_aac::FdkAacLibrary>,
        compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        match codec {
            CodecName::Aac => {
                #[cfg(feature = "fdk-aac")]
                if let Some(lib) = fdk_aac_lib {
                    return AudioEncoder::new_fdk_aac(lib, bitrate, compose_stats);
                }

                #[cfg(target_os = "macos")]
                return AudioEncoder::new_audio_toolbox_aac(bitrate, compose_stats);

                #[cfg(not(target_os = "macos"))]
                return Err(crate::Error::new(
                    "AAC encoding requires FDK-AAC library. \
                     Please specify the library path using --fdk-aac command line argument or \
                     HISUI_FDK_AAC_PATH environment variable.",
                ));
            }
            CodecName::Opus => AudioEncoder::new_opus(bitrate, compose_stats),
            _ => unreachable!(),
        }
    }

    fn new_opus(
        bitrate: NonZeroUsize,
        mut compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        compose_stats
            .string("engine")
            .set(EngineName::Opus.as_str());
        compose_stats.string("codec").set(CodecName::Opus.as_str());
        let total_audio_data_count_metric = compose_stats.counter("total_audio_data_count");
        let error_flag = compose_stats.flag("error");
        error_flag.set(false);
        Ok(Self {
            total_audio_data_count_metric,
            _error_flag: error_flag,
            encoded: VecDeque::new(),
            eos: false,
            converter: default_audio_converter(),
            inner: AudioEncoderInner::new_opus(bitrate)?,
        })
    }

    #[cfg(feature = "fdk-aac")]
    fn new_fdk_aac(
        lib: shiguredo_fdk_aac::FdkAacLibrary,
        bitrate: NonZeroUsize,
        mut compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        compose_stats
            .string("engine")
            .set(EngineName::FdkAac.as_str());
        compose_stats.string("codec").set(CodecName::Aac.as_str());
        let total_audio_data_count_metric = compose_stats.counter("total_audio_data_count");
        let error_flag = compose_stats.flag("error");
        error_flag.set(false);
        Ok(Self {
            total_audio_data_count_metric,
            _error_flag: error_flag,
            encoded: VecDeque::new(),
            eos: false,
            converter: default_audio_converter(),
            inner: AudioEncoderInner::new_fdk_aac(lib, bitrate)?,
        })
    }

    #[cfg(target_os = "macos")]
    fn new_audio_toolbox_aac(
        bitrate: NonZeroUsize,
        mut compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        compose_stats
            .string("engine")
            .set(EngineName::AudioToolbox.as_str());
        compose_stats.string("codec").set(CodecName::Aac.as_str());
        let total_audio_data_count_metric = compose_stats.counter("total_audio_data_count");
        let error_flag = compose_stats.flag("error");
        error_flag.set(false);
        Ok(Self {
            total_audio_data_count_metric,
            _error_flag: error_flag,
            encoded: VecDeque::new(),
            eos: false,
            converter: default_audio_converter(),
            inner: AudioEncoderInner::new_audio_toolbox_aac(bitrate)?,
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

    pub fn get_engines(codec: CodecName, is_fdk_aac_available: bool) -> Vec<EngineName> {
        let mut engines = Vec::new();
        match codec {
            CodecName::Aac => {
                if is_fdk_aac_available {
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

            let finished = drain_audio_encoder_output(&mut self, &mut output_tx)?;
            if finished {
                output_tx.send_eos();
                break;
            }

            if is_eos {
                return Err(Error::new("audio encoder still pending after EOS"));
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

    fn handle_input_sample(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        let frames = if let Some(sample) = sample {
            let frame = sample.expect_audio()?;
            let converted = self.converter.convert(&frame)?;
            self.inner.encode(&converted)?
        } else {
            self.eos = true;
            self.inner.finish()?
        };

        for encoded in frames {
            self.total_audio_data_count_metric.inc();
            self.encoded.push_back(encoded);
        }
        Ok(())
    }

    fn poll_output(&mut self) -> Result<EncoderRunOutput> {
        if let Some(frame) = self.encoded.pop_front() {
            Ok(EncoderRunOutput::Processed(MediaFrame::audio(frame)))
        } else if self.eos {
            Ok(EncoderRunOutput::Finished)
        } else {
            Ok(EncoderRunOutput::Pending)
        }
    }
}

fn default_audio_converter() -> AudioConverter {
    AudioConverterBuilder::new()
        .format(AudioFormat::I16Be)
        .channels(Channels::STEREO)
        .sample_rate(SampleRate::HZ_48000)
        .build()
}

fn drain_audio_encoder_output(
    encoder: &mut AudioEncoder,
    output_tx: &mut crate::MessageSender,
) -> Result<bool> {
    loop {
        match encoder.poll_output()? {
            EncoderRunOutput::Processed(sample) => {
                if !output_tx.send_media(sample) {
                    return Ok(true);
                }
            }
            EncoderRunOutput::Pending => {
                return Ok(false);
            }
            EncoderRunOutput::Finished => {
                return Ok(true);
            }
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
    fn new_opus(bitrate: NonZeroUsize) -> crate::Result<Self> {
        OpusEncoder::new(bitrate).map(Self::Opus)
    }

    #[cfg(feature = "fdk-aac")]
    fn new_fdk_aac(
        lib: shiguredo_fdk_aac::FdkAacLibrary,
        bitrate: NonZeroUsize,
    ) -> crate::Result<Self> {
        FdkAacEncoder::new(lib, bitrate).map(Self::FdkAac)
    }

    #[cfg(target_os = "macos")]
    fn new_audio_toolbox_aac(bitrate: NonZeroUsize) -> crate::Result<Self> {
        AudioToolboxEncoder::new(bitrate).map(Self::AudioToolbox)
    }

    fn encode(&mut self, frame: &AudioFrame) -> crate::Result<Vec<AudioFrame>> {
        match self {
            #[cfg(feature = "fdk-aac")]
            Self::FdkAac(encoder) => encoder.encode(frame),
            #[cfg(target_os = "macos")]
            Self::AudioToolbox(encoder) => encoder.encode(frame).map(|f| f.into_iter().collect()),
            Self::Opus(encoder) => encoder.encode(frame).map(|f| vec![f]),
        }
    }

    fn finish(&mut self) -> crate::Result<Vec<AudioFrame>> {
        match self {
            #[cfg(feature = "fdk-aac")]
            Self::FdkAac(encoder) => encoder.finish(),
            #[cfg(target_os = "macos")]
            Self::AudioToolbox(encoder) => encoder.finish().map(|f| f.into_iter().collect()),
            Self::Opus(_encoder) => Ok(vec![]),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EncodeConfig {
    pub libvpx_vp8: shiguredo_libvpx::EncoderConfig,
    pub libvpx_vp9: shiguredo_libvpx::EncoderConfig,
    pub openh264: shiguredo_openh264::EncoderConfig,
    pub svt_av1: shiguredo_svt_av1::EncoderConfig,
    #[cfg(target_os = "macos")]
    pub video_toolbox_h264: shiguredo_video_toolbox::EncoderConfig,
    #[cfg(target_os = "macos")]
    pub video_toolbox_h265: shiguredo_video_toolbox::EncoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_h264: shiguredo_nvcodec::EncoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_h265: shiguredo_nvcodec::EncoderConfig,
    #[cfg(feature = "nvcodec")]
    pub nvcodec_av1: shiguredo_nvcodec::EncoderConfig,
}

#[derive(Debug, Clone)]
pub struct VideoEncoderOptions {
    pub codec: CodecName,
    pub engines: Option<Vec<EngineName>>,
    pub bitrate: usize,
    pub width: EvenUsize,
    pub height: EvenUsize,
    pub frame_rate: FrameRate,
    pub encode_params: EncodeConfig,
}

impl VideoEncoderOptions {
    // width / height の最初の値は実際には使われず、後で実際のフレームの解像度で更新されるので、
    // その（使われない）初期値の設定を行いやすくするための定数を定義しておく
    pub const DUMMY_WIDTH: EvenUsize = EvenUsize::ZERO;
    pub const DUMMY_HEIGHT: EvenUsize = EvenUsize::ZERO;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoderRpcMessage {
    RequestKeyframe,
}

#[derive(Debug)]
pub struct VideoEncoder {
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    total_output_video_frame_count_metric: crate::stats::StatsCounter,
    total_output_video_keyframe_count_metric: crate::stats::StatsCounter,
    total_video_keyframe_request_count_metric: crate::stats::StatsCounter,
    _error_flag: crate::stats::StatsFlag,
    encoded: VecDeque<VideoFrame>,
    eos: bool,
    keyframe_request_pending: bool,
    last_video_sample_entry: Option<SampleEntry>,
    // 最初のフレームを受信するまで、内部エンコーダは初期化されない
    inner: Option<VideoEncoderInner>,
    options: VideoEncoderOptions,
    openh264_lib: Option<Openh264Library>,
}

impl VideoEncoder {
    pub fn new(
        options: &VideoEncoderOptions,
        openh264_lib: Option<Openh264Library>,
        mut compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        let engine_metric = compose_stats.string("engine");
        let codec_metric = compose_stats.string("codec");
        let total_input_video_frame_count_metric =
            compose_stats.counter("total_input_video_frame_count");
        let total_output_video_frame_count_metric =
            compose_stats.counter("total_output_video_frame_count");
        let total_output_video_keyframe_count_metric =
            compose_stats.counter("total_output_video_keyframe_count");
        let total_video_keyframe_request_count_metric =
            compose_stats.counter("total_video_keyframe_request_count");
        let error_flag = compose_stats.flag("error");
        error_flag.set(false);
        Ok(Self {
            engine_metric,
            codec_metric,
            total_input_video_frame_count_metric,
            total_output_video_frame_count_metric,
            total_output_video_keyframe_count_metric,
            total_video_keyframe_request_count_metric,
            _error_flag: error_flag,
            encoded: VecDeque::new(),
            eos: false,
            keyframe_request_pending: false,
            last_video_sample_entry: None,
            inner: None,
            options: options.clone(),
            openh264_lib,
        })
    }

    /// 最初のフレームの解像度を使用して、内部エンコーダを初期化する
    fn initialize_inner(&mut self, width: usize, height: usize) -> crate::Result<()> {
        // 既に初期化されている場合はスキップ
        if self.inner.is_some() {
            return Ok(());
        }

        // 解像度を含めたオプションを作成
        //
        // [NOTE] ここでは偶数解像度を期待する（奇数になる場合は前段でリサイズなどをする必要がある）
        self.options.width = EvenUsize::new(width)
            .ok_or_else(|| crate::Error::new(format!("frame width must be even, got {width}")))?;
        self.options.height = EvenUsize::new(height)
            .ok_or_else(|| crate::Error::new(format!("frame height must be even, got {height}")))?;

        // エンコーダーのインスタンスを作成
        let inner = self.create_inner()?;

        // エンジン名とコーデックを設定
        self.engine_metric.set(inner.name().as_str());
        self.codec_metric.set(inner.codec().as_str());

        self.inner = Some(inner);
        Ok(())
    }

    /// エンコーダーのインスタンスを生成する
    fn create_inner(&self) -> crate::Result<VideoEncoderInner> {
        let options = &self.options;
        let candidate_engines = options
            .engines
            .clone()
            .unwrap_or_else(|| EngineName::default_video_encoders(self.openh264_lib.is_some()));
        let engine = candidate_engines
            .iter()
            .find(|engine| engine.is_available_video_encode_codec(options.codec))
            .copied();

        match (engine, options.codec) {
            (Some(EngineName::Libvpx), CodecName::Vp8) => VideoEncoderInner::new_vp8(options),
            (Some(EngineName::Libvpx), CodecName::Vp9) => VideoEncoderInner::new_vp9(options),
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H264) => {
                VideoEncoderInner::new_nvcodec_h264(options)
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H265) => {
                VideoEncoderInner::new_nvcodec_h265(options)
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Av1) => {
                VideoEncoderInner::new_nvcodec_av1(options)
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H264) => {
                VideoEncoderInner::new_video_toolbox_h264(options)
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H265) => {
                VideoEncoderInner::new_video_toolbox_h265(options)
            }
            (Some(EngineName::Openh264), CodecName::H264) => {
                let lib = self.openh264_lib.clone().ok_or_else(|| {
                    crate::Error::new(
                        concat!(
                        "OpenH264 library is required for H.264 encoding. ",
                        "Please specify the library path using --openh264 command line argument or ",
                        "HISUI_OPENH264_PATH environment variable."
                    )
                        .to_owned(),
                    )
                })?;
                VideoEncoderInner::new_openh264(lib, options)
            }
            (Some(EngineName::SvtAv1), CodecName::Av1) => VideoEncoderInner::new_svt_av1(options),
            _ => Err(crate::Error::new(format!(
                "no available encoder for {} codec (candidate encoders: {})",
                options.codec.as_str(),
                candidate_engines
                    .iter()
                    .map(|engine| engine.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    pub fn name(&self) -> Option<EngineName> {
        self.inner.as_ref().map(|inner| inner.name())
    }

    pub fn codec(&self) -> Option<CodecName> {
        self.inner.as_ref().map(|inner| inner.codec())
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
                engines.push(EngineName::SvtAv1);
            }
            _ => unreachable!(),
        }
        engines
    }

    pub async fn run(
        mut self,
        handle: ProcessorHandle,
        input_track_id: TrackId,
        output_track_id: TrackId,
    ) -> Result<()> {
        let mut input_rx = handle.subscribe_track(input_track_id);
        let mut output_tx = handle.publish_track(output_track_id).await?;
        let (rpc_tx, mut rpc_rx) = tokio::sync::mpsc::unbounded_channel();
        handle
            .register_rpc_sender(rpc_tx)
            .await
            .map_err(|e| Error::new(format!("failed to register video encoder RPC sender: {e}")))?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;
        let mut rpc_rx_enabled = true;

        loop {
            tokio::select! {
                message = input_rx.recv() => {
                    let is_eos = matches!(message, Message::Eos);
                    self.handle_input_message(message)?;

                    let finished = drain_video_encoder_output(&mut self, &mut output_tx)?;
                    if finished {
                        output_tx.send_eos();
                        break;
                    }

                    if is_eos {
                        return Err(Error::new("video encoder still pending after EOS"));
                    }
                }
                rpc_message = recv_video_encoder_rpc_message_or_pending(
                    rpc_rx_enabled.then_some(&mut rpc_rx)
                ) => {
                    let Some(rpc_message) = rpc_message else {
                        rpc_rx_enabled = false;
                        continue;
                    };
                    self.handle_rpc_message(rpc_message);
                }
            }
        }

        Ok(())
    }

    fn handle_rpc_message(&mut self, message: VideoEncoderRpcMessage) {
        match message {
            VideoEncoderRpcMessage::RequestKeyframe => {
                self.total_video_keyframe_request_count_metric.inc();
                // 複数の keyframe 要求は 1 件に集約して扱う。
                // RPC 受信時点ではフラグのみ更新し、実際の keyframe 要求適用は
                // 次の入力フレーム処理時に行う。低フレームレート入力などでは遅延し得るが、
                // 現状は入力フローと同一タイミングでの適用を意図した設計とする。
                self.keyframe_request_pending = true;
            }
        }
    }

    fn handle_input_message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Media(sample) => self.handle_input_sample(Some(sample)),
            Message::Eos => self.handle_input_sample(None),
            Message::Syn(_) => Ok(()),
        }
    }

    fn handle_input_sample(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        if let Some(sample) = sample {
            let frame = sample.expect_video()?;
            let frame = RawVideoFrame::from_video_frame(frame)?;
            let size = frame.size();

            // 最初のフレームで、解像度を使って初期化する
            if self.inner.is_none() {
                self.initialize_inner(size.width, size.height)?;
            }
            if self.keyframe_request_pending {
                self.apply_pending_keyframe_request()?;
            }

            self.total_input_video_frame_count_metric.inc();
            self.inner.as_mut().expect("infallible").encode(frame)?;
        } else {
            self.eos = true;
            if let Some(inner) = &mut self.inner {
                inner.finish()?;
            }
        }

        self.drain_encoded_frames();
        Ok(())
    }

    fn apply_pending_keyframe_request(&mut self) -> Result<()> {
        debug_assert!(
            self.inner.is_some(),
            "apply_pending_keyframe_request must be called after initialize_inner"
        );
        let request_supported = match self.inner.as_mut() {
            Some(inner) => inner.request_keyframe(),
            None => false,
        };
        if !request_supported {
            self.drain_encoded_frames();
            let recreated = self.create_inner()?;
            self.engine_metric.set(recreated.name().as_str());
            self.codec_metric.set(recreated.codec().as_str());
            self.inner = Some(recreated);
        }
        self.keyframe_request_pending = false;
        Ok(())
    }

    fn drain_encoded_frames(&mut self) {
        let Some(mut inner) = self.inner.take() else {
            return;
        };
        while let Some(encoded) = inner.next_encoded_frame() {
            self.push_encoded_frame_with_metrics(encoded);
        }
        self.inner = Some(inner);
    }

    fn push_encoded_frame_with_metrics(&mut self, mut encoded: VideoFrame) {
        self.total_output_video_frame_count_metric.inc();
        if let Some(sample_entry) = encoded.sample_entry.as_ref() {
            self.last_video_sample_entry = Some(sample_entry.clone());
        }
        if encoded.keyframe {
            self.total_output_video_keyframe_count_metric.inc();
            // keyframe は単体でデコード可能であるべきなので、sample entry を常に補完する。
            if encoded.sample_entry.is_none()
                && let Some(sample_entry) = self.last_video_sample_entry.as_ref()
            {
                encoded.sample_entry = Some(sample_entry.clone());
            }
        }
        self.encoded.push_back(encoded);
    }

    fn poll_output(&mut self) -> Result<EncoderRunOutput> {
        if let Some(frame) = self.encoded.pop_front() {
            Ok(EncoderRunOutput::Processed(MediaFrame::video(frame)))
        } else if self.eos {
            Ok(EncoderRunOutput::Finished)
        } else {
            Ok(EncoderRunOutput::Pending)
        }
    }
}

fn drain_video_encoder_output(
    encoder: &mut VideoEncoder,
    output_tx: &mut crate::MessageSender,
) -> Result<bool> {
    loop {
        match encoder.poll_output()? {
            EncoderRunOutput::Processed(sample) => {
                if !output_tx.send_media(sample) {
                    return Ok(true);
                }
            }
            EncoderRunOutput::Pending => {
                return Ok(false);
            }
            EncoderRunOutput::Finished => {
                return Ok(true);
            }
        }
    }
}

async fn recv_video_encoder_rpc_message_or_pending(
    rpc_rx: Option<&mut tokio::sync::mpsc::UnboundedReceiver<VideoEncoderRpcMessage>>,
) -> Option<VideoEncoderRpcMessage> {
    if let Some(rpc_rx) = rpc_rx {
        rpc_rx.recv().await
    } else {
        std::future::pending().await
    }
}

#[derive(Debug)]
enum VideoEncoderInner {
    Libvpx(LibvpxEncoder),
    Openh264(Openh264Encoder),
    SvtAv1(SvtAv1Encoder),
    #[cfg(target_os = "macos")]
    VideoToolbox(VideoToolboxEncoder),
    #[cfg(feature = "nvcodec")]
    Nvcodec(Box<NvcodecEncoder>), // Box は clippy::large_enum_variant 対策
}

impl VideoEncoderInner {
    fn new_vp8(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let encoder = LibvpxEncoder::new_vp8(options)?;
        Ok(Self::Libvpx(encoder))
    }

    fn new_vp9(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let encoder = LibvpxEncoder::new_vp9(options)?;
        Ok(Self::Libvpx(encoder))
    }

    fn new_openh264(lib: Openh264Library, options: &VideoEncoderOptions) -> crate::Result<Self> {
        let encoder = Openh264Encoder::new(lib, options)?;
        Ok(Self::Openh264(encoder))
    }

    fn new_svt_av1(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let encoder = SvtAv1Encoder::new(options)?;
        Ok(Self::SvtAv1(encoder))
    }

    #[cfg(target_os = "macos")]
    fn new_video_toolbox_h264(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let encoder = VideoToolboxEncoder::new_h264(options)?;
        Ok(Self::VideoToolbox(encoder))
    }

    #[cfg(target_os = "macos")]
    fn new_video_toolbox_h265(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let encoder = VideoToolboxEncoder::new_h265(options)?;
        Ok(Self::VideoToolbox(encoder))
    }

    #[cfg(feature = "nvcodec")]
    fn new_nvcodec_h265(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let encoder = NvcodecEncoder::new_h265(options)?;
        Ok(Self::Nvcodec(Box::new(encoder)))
    }

    #[cfg(feature = "nvcodec")]
    fn new_nvcodec_h264(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let encoder = NvcodecEncoder::new_h264(options)?;
        Ok(Self::Nvcodec(Box::new(encoder)))
    }

    #[cfg(feature = "nvcodec")]
    fn new_nvcodec_av1(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let encoder = NvcodecEncoder::new_av1(options)?;
        Ok(Self::Nvcodec(Box::new(encoder)))
    }

    fn encode(&mut self, frame: RawVideoFrame) -> crate::Result<()> {
        match self {
            Self::Libvpx(encoder) => encoder.encode(frame),
            Self::Openh264(encoder) => encoder.encode(frame),
            Self::SvtAv1(encoder) => encoder.encode(frame),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.encode(frame),
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(encoder) => encoder.encode(frame),
        }
    }

    fn finish(&mut self) -> crate::Result<()> {
        match self {
            Self::Libvpx(encoder) => encoder.finish(),
            Self::Openh264(encoder) => encoder.finish(),
            Self::SvtAv1(encoder) => encoder.finish(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.finish(),
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(encoder) => encoder.finish(),
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

    fn request_keyframe(&mut self) -> bool {
        match self {
            Self::Libvpx(encoder) => {
                encoder.request_keyframe();
                true
            }
            Self::Openh264(encoder) => {
                encoder.request_keyframe();
                true
            }
            Self::SvtAv1(_) => false,
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(_) => false,
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(encoder) => {
                encoder.request_keyframe();
                true
            }
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
            Self::Nvcodec(encoder) => encoder.codec(),
        }
    }
}
