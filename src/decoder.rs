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
use tokio::sync::mpsc;

use self::dav1d::Dav1dDecoder;
use self::libvpx::LibvpxDecoder;
#[cfg(feature = "nvcodec")]
use self::nvcodec::NvcodecDecoder;
use self::openh264::Openh264Decoder;
use self::opus::OpusDecoder;
#[cfg(target_os = "macos")]
use self::video_toolbox::VideoToolboxDecoder;
use crate::{
    Error, Message, MessageReceiver, ProcessorHandle, Result, TrackId, TrackPublisher,
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

/// 内部 channel の容量
///
/// `inner.decode()` 1 回が送出しうる最大フレーム数 + 余裕で N=8 とする
/// (例: Openh264 は keyframe 時の finish() 経由で最大 2 フレーム送出するため、N >= 4 が必須)。
const INTERNAL_CHANNEL_CAPACITY: usize = 8;

#[derive(Debug)]
pub struct VideoDecoder {
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    total_output_video_frame_count_metric: crate::stats::StatsCounter,
    inner: VideoDecoderInner,
    // 内部 inner → run() 出力 task の橋渡し用 Sender
    // (decode 完了フレームを上位の `mpsc::Receiver` 側に流す)
    tx: mpsc::Sender<crate::Result<VideoFrame>>,
}

impl VideoDecoder {
    /// VideoDecoder と、対の Receiver (内部 channel の受信側) を生成する。
    ///
    /// 戻り値の Receiver は呼出側で保持し、 `run()` 起動時に引数で戻す。
    pub fn new(
        options: VideoDecoderOptions,
        mut compose_stats: crate::stats::Stats,
    ) -> (Self, mpsc::Receiver<crate::Result<VideoFrame>>) {
        let engine_metric = compose_stats.string("engine");
        let codec_metric = compose_stats.string("codec");
        let total_input_video_frame_count_metric =
            compose_stats.counter("total_input_video_frame_count");
        let total_output_video_frame_count_metric =
            compose_stats.counter("total_output_video_frame_count");
        compose_stats.flag("error").set(false);
        let (tx, rx) = mpsc::channel(INTERNAL_CHANNEL_CAPACITY);
        (
            Self {
                engine_metric,
                codec_metric,
                total_input_video_frame_count_metric,
                total_output_video_frame_count_metric,
                inner: VideoDecoderInner::new(options),
                tx,
            },
            rx,
        )
    }

    /// 入力 / 出力を 2 sub-task に分離して実行する。
    ///
    /// - run_input task: input_rx 受信 → inner.decode() → 内部 channel (tx) に push
    /// - run_output task: 内部 channel (decoded_rx) 受信 → output_tx.send_media()
    /// - 出力 task が下流 close を検知したら oneshot で入力 task に shutdown 伝達
    /// - 入力 task が EOS を受けたら inner.finish() で残フレームを流して tx drop、
    ///   出力 task は channel close を検知して send_eos して終了
    pub async fn run(
        self,
        handle: ProcessorHandle,
        input_track_id: TrackId,
        output_track_id: TrackId,
        decoded_rx: mpsc::Receiver<crate::Result<VideoFrame>>,
    ) -> Result<()> {
        let input_rx = handle.subscribe_track(input_track_id);
        let output_tx = handle.publish_track(output_track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        let Self {
            engine_metric,
            codec_metric,
            total_input_video_frame_count_metric,
            total_output_video_frame_count_metric,
            inner,
            tx,
        } = self;

        // 出力 task → 入力 task への shutdown 伝達 (下流 close 検知時)
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let input_handle = tokio::spawn(run_video_decoder_input_task(
            inner,
            tx,
            input_rx,
            total_input_video_frame_count_metric,
            engine_metric,
            codec_metric,
            shutdown_rx,
        ));
        let output_handle = tokio::spawn(run_video_decoder_output_task(
            output_tx,
            decoded_rx,
            total_output_video_frame_count_metric,
            shutdown_tx,
        ));

        let (input_result, output_result) = tokio::join!(input_handle, output_handle);
        input_result
            .map_err(|e| Error::new(format!("video decoder input task panicked: {e}")))??;
        output_result
            .map_err(|e| Error::new(format!("video decoder output task panicked: {e}")))??;
        Ok(())
    }

    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> {
        let mut engines = Vec::new();
        match codec {
            CodecName::Vp8 => {
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_library_available() {
                    engines.push(EngineName::Nvcodec);
                }
                engines.push(EngineName::Libvpx);
            }
            CodecName::Vp9 => {
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_library_available() {
                    engines.push(EngineName::Nvcodec);
                }
                #[cfg(target_os = "macos")]
                if EngineName::VideoToolbox.is_available_video_decode_codec(codec) {
                    engines.push(EngineName::VideoToolbox);
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
                if EngineName::VideoToolbox.is_available_video_decode_codec(codec) {
                    engines.push(EngineName::VideoToolbox);
                }
            }
            CodecName::H265 => {
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_library_available() {
                    engines.push(EngineName::Nvcodec);
                }
                #[cfg(target_os = "macos")]
                if EngineName::VideoToolbox.is_available_video_decode_codec(codec) {
                    engines.push(EngineName::VideoToolbox);
                }
            }
            CodecName::Av1 => {
                #[cfg(feature = "nvcodec")]
                if shiguredo_nvcodec::is_cuda_library_available() {
                    engines.push(EngineName::Nvcodec);
                }
                #[cfg(target_os = "macos")]
                if EngineName::VideoToolbox.is_available_video_decode_codec(codec) {
                    engines.push(EngineName::VideoToolbox);
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

/// VideoDecoder の入力 task ループ
///
/// 入力 track からメッセージを受信して inner.decode() を呼ぶ。EOS 受信時は
/// inner.finish() で残フレームを内部 channel に流してから tx を drop して終了する。
/// 出力 task から shutdown 通知 (下流 close 検知時) を受けたら即座に抜ける。
async fn run_video_decoder_input_task(
    mut inner: VideoDecoderInner,
    tx: mpsc::Sender<crate::Result<VideoFrame>>,
    mut input_rx: MessageReceiver,
    total_input_metric: crate::stats::StatsCounter,
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<()> {
    loop {
        tokio::select! {
            // 出力 task からの shutdown 通知で抜ける
            _ = &mut shutdown_rx => return Ok(()),
            message = input_rx.recv() => {
                match message {
                    Message::Media(sample) => {
                        let frame = sample.expect_video()?;
                        total_input_metric.inc();
                        inner
                            .decode(&frame, &codec_metric, &engine_metric, &tx)
                            .await?;
                    }
                    Message::Eos => {
                        // EOS 順序保証: inner.finish() で残フレームを内部 channel に全送出してから
                        // tx を drop して出力 task に channel close を通知する。
                        inner.finish().await?;
                        drop(tx);
                        return Ok(());
                    }
                    Message::Syn(_) => {}
                }
            }
        }
    }
}

/// VideoDecoder の出力 task ループ
///
/// 内部 channel から `Result<VideoFrame>` を受信し、Ok なら下流に流す。
/// Err 受信時 (inner エラー) は closed/0054 整合の fail-fast で送 EOS + Err 返却。
/// send_media が false (下流 close) の場合は入力 task に shutdown 通知してから fail-fast。
/// 内部 channel close (入力 task の EOS シーケンス完了) を検知したら send_eos して終了。
async fn run_video_decoder_output_task(
    mut output_tx: TrackPublisher,
    mut decoded_rx: mpsc::Receiver<crate::Result<VideoFrame>>,
    total_output_metric: crate::stats::StatsCounter,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
) -> Result<()> {
    let mut shutdown_tx = Some(shutdown_tx);
    while let Some(result) = decoded_rx.recv().await {
        match result {
            Ok(frame) => {
                total_output_metric.inc();
                if !output_tx.send_media(MediaFrame::video(frame)) {
                    // 下流 close 検知 → 入力 task に shutdown 通知して fail-fast
                    if let Some(tx) = shutdown_tx.take() {
                        let _ = tx.send(());
                    }
                    return Err(Error::new("pipeline closed before video decoder finished"));
                }
            }
            Err(e) => {
                // inner エラー → fail-fast (closed/0054 整合)
                output_tx.send_eos();
                return Err(e);
            }
        }
    }
    // 内部 channel close (= 入力 task の EOS シーケンス完了) → 通常終了
    output_tx.send_eos();
    Ok(())
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
        tx: mpsc::Sender<crate::Result<VideoFrame>>,
    ) -> crate::Result<()> {
        let codec = frame.format.codec_name().ok_or_else(|| {
            crate::Error::new(format!("unexpected video format: {:?}", frame.format))
        })?;
        codec_metric.set(codec.as_str());

        let candidate_engines = options
            .engines
            .unwrap_or_else(|| VideoDecoder::get_engines(codec, options.openh264_lib.is_some()));

        // TODO: デコーダー初期化が失敗したときに次の候補エンジンにフォールバックする仕組みを入れる
        //       （例: HWA が解像度が小さすぎるケースで失敗する場合など）
        let engine = candidate_engines
            .iter()
            .find(|engine| {
                if !engine.is_available_video_decode_codec(codec) {
                    return false;
                }
                // VideoToolbox の VP9/AV1 デコーダーは CMVideoFormatDescriptionCreate に
                // width/height が必須なので、frame.size が無い入力では選択しない
                #[cfg(target_os = "macos")]
                if **engine == EngineName::VideoToolbox
                    && matches!(codec, CodecName::Vp9 | CodecName::Av1)
                    && frame.size.is_none()
                {
                    return false;
                }
                true
            })
            .copied();
        if let Some(engine) = engine {
            engine_metric.set(engine.as_str());
        }

        match (engine, codec) {
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H264) => {
                *self = NvcodecDecoder::new_h264(&options.decode_params, tx).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H265) => {
                *self = NvcodecDecoder::new_h265(&options.decode_params, tx).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Vp8) => {
                *self = NvcodecDecoder::new_vp8(&options.decode_params, tx).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Vp9) => {
                *self = NvcodecDecoder::new_vp9(&options.decode_params, tx).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Av1) => {
                *self = NvcodecDecoder::new_av1(&options.decode_params, tx).map(Self::Nvcodec)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H264) => {
                *self = VideoToolboxDecoder::new_h264(frame, tx)
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H265) => {
                *self = VideoToolboxDecoder::new_h265(frame, tx)
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::Vp9) => {
                *self = VideoToolboxDecoder::new_vp9(frame, tx)
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::Av1) => {
                *self = VideoToolboxDecoder::new_av1(frame, tx)
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            (Some(EngineName::Openh264), CodecName::H264) => {
                let lib = options.openh264_lib.ok_or_else(|| {
                    crate::Error::new("OpenH264 library is required for H.264 decoding")
                })?;
                *self = Openh264Decoder::new(lib.clone(), tx).map(Self::Openh264)?;
            }
            (Some(EngineName::Libvpx), CodecName::Vp8) => {
                *self = LibvpxDecoder::new_vp8(tx).map(Self::Libvpx)?;
            }
            (Some(EngineName::Libvpx), CodecName::Vp9) => {
                *self = LibvpxDecoder::new_vp9(tx).map(Self::Libvpx)?;
            }
            (Some(EngineName::Dav1d), CodecName::Av1) => {
                *self = Dav1dDecoder::new(tx).map(Self::Dav1d)?;
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

    /// async fn 統一シグネチャ
    ///
    /// Initial 状態の場合は最初に `initialize_decoder` で実 decoder を生成し、
    /// `tx.clone()` を内包させてから dispatch する。
    async fn decode(
        &mut self,
        frame: &VideoFrame,
        codec_metric: &crate::stats::StatsString,
        engine_metric: &crate::stats::StatsString,
        tx: &mpsc::Sender<crate::Result<VideoFrame>>,
    ) -> crate::Result<()> {
        if let Self::Initial { options } = self {
            let options = options.clone();
            self.initialize_decoder(frame, codec_metric, engine_metric, options, tx.clone())?;
        }
        match self {
            Self::Initial { .. } => {
                unreachable!("decoder must have been initialized above")
            }
            Self::Libvpx(decoder) => decoder.decode(frame).await,
            Self::Openh264(decoder) => decoder.decode(frame).await,
            Self::Dav1d(decoder) => decoder.decode(frame).await,
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(decoder) => decoder.decode(frame).await,
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(decoder) => decoder.decode(frame).await,
        }
    }

    async fn finish(&mut self) -> crate::Result<()> {
        match self {
            Self::Initial { .. } => {}
            Self::Libvpx(decoder) => decoder.finish().await?,
            Self::Openh264(decoder) => decoder.finish().await?,
            Self::Dav1d(decoder) => decoder.finish().await?,
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(decoder) => decoder.finish().await?,
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(decoder) => decoder.finish().await?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::video::{VideoFormat, VideoFrame, VideoFrameSize};

    /// VideoToolbox と SW デコーダーのみを候補にしたオプションを作る
    ///
    /// Nvcodec を候補から除外することで、CUDA 環境の有無に関わらず
    /// VideoToolbox のスキップ判定をテストできるようにする。
    fn options_without_nvcodec(engines: Vec<EngineName>) -> VideoDecoderOptions {
        VideoDecoderOptions {
            engines: Some(engines),
            ..Default::default()
        }
    }

    /// size: None の VP9 フレームで VideoToolbox がスキップされ Libvpx が選ばれることを確認する
    #[tokio::test]
    async fn vp9_without_size_skips_video_toolbox() {
        let frame = VideoFrame {
            data: vec![],
            format: VideoFormat::Vp9,
            keyframe: true,
            size: None,
            timestamp: Duration::ZERO,
            sample_entry: None,
        };

        let engines = vec![EngineName::VideoToolbox, EngineName::Libvpx];
        let stats = crate::stats::Stats::new();
        // Sender 化に伴い `new` は (decoder, rx) のタプルを返す
        let (mut decoder, _rx) = VideoDecoder::new(options_without_nvcodec(engines), stats);
        // 空データなので実際の decode は失敗するが、Initial → 実 decoder への遷移
        // (initialize_decoder) は成功するため、その結果の inner variant を検証する
        let (tx_test, _rx_test) = mpsc::channel(8);
        let _ = decoder
            .inner
            .decode(
                &frame,
                &decoder.codec_metric,
                &decoder.engine_metric,
                &tx_test,
            )
            .await;

        assert!(
            matches!(decoder.inner, VideoDecoderInner::Libvpx(_)),
            "expected Libvpx decoder, got {:?}",
            std::mem::discriminant(&decoder.inner)
        );
    }

    /// size: None の AV1 フレームで VideoToolbox がスキップされ Dav1d が選ばれることを確認する
    #[tokio::test]
    async fn av1_without_size_skips_video_toolbox() {
        let frame = VideoFrame {
            data: vec![],
            format: VideoFormat::Av1,
            keyframe: true,
            size: None,
            timestamp: Duration::ZERO,
            sample_entry: None,
        };

        let engines = vec![EngineName::VideoToolbox, EngineName::Dav1d];
        let stats = crate::stats::Stats::new();
        let (mut decoder, _rx) = VideoDecoder::new(options_without_nvcodec(engines), stats);
        let (tx_test, _rx_test) = mpsc::channel(8);
        let _ = decoder
            .inner
            .decode(
                &frame,
                &decoder.codec_metric,
                &decoder.engine_metric,
                &tx_test,
            )
            .await;

        assert!(
            matches!(decoder.inner, VideoDecoderInner::Dav1d(_)),
            "expected Dav1d decoder, got {:?}",
            std::mem::discriminant(&decoder.inner)
        );
    }

    /// size ありの VP9 フレームでは macOS 対応環境なら VideoToolbox、非対応なら Libvpx が選ばれることを確認する
    #[tokio::test]
    async fn vp9_with_size_selects_available_engine() {
        let frame = VideoFrame {
            data: vec![],
            format: VideoFormat::Vp9,
            keyframe: true,
            size: Some(VideoFrameSize {
                width: 1920,
                height: 1080,
            }),
            timestamp: Duration::ZERO,
            sample_entry: None,
        };

        let engines = vec![EngineName::VideoToolbox, EngineName::Libvpx];
        let stats = crate::stats::Stats::new();
        let (mut decoder, _rx) = VideoDecoder::new(options_without_nvcodec(engines), stats);
        let (tx_test, _rx_test) = mpsc::channel(8);
        let _ = decoder
            .inner
            .decode(
                &frame,
                &decoder.codec_metric,
                &decoder.engine_metric,
                &tx_test,
            )
            .await;

        // size ありなら VideoToolbox がスキップされずにエンジン選択が行われることを確認する。
        // どちらが選ばれるかは実行環境の VP9 ハードウェアデコード対応状況に依存するため、
        // ここでは「いずれかの有効なエンジンが選択されること」のみを検証する。
        #[cfg(target_os = "macos")]
        let is_valid = matches!(decoder.inner, VideoDecoderInner::Libvpx(_))
            || matches!(decoder.inner, VideoDecoderInner::VideoToolbox(_));
        #[cfg(not(target_os = "macos"))]
        let is_valid = matches!(decoder.inner, VideoDecoderInner::Libvpx(_));
        assert!(
            is_valid,
            "expected Libvpx or VideoToolbox decoder, got {:?}",
            std::mem::discriminant(&decoder.inner)
        );
    }
}
