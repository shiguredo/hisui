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

#[cfg(test)]
mod test_helpers;

use std::collections::VecDeque;
use std::num::NonZeroUsize;

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

pub enum EncoderRunOutput {
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
    output_tx: &mut crate::TrackPublisher,
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

/// 内部エンコーダーが出力フレーム / エラーを `AsyncVideoEncoder` 内の受信側 (`rx`) に流すための送信側の型エイリアス
pub type EncoderOutputSender = tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>;

/// `AsyncVideoEncoder` 内部で内部エンコーダーからの出力フレーム / エラーを受け取る受信側の型エイリアス
pub(crate) type EncoderOutputReceiver =
    tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>;

/// 内部エンコーダーが出力フレーム / エラーを `AsyncVideoEncoder` 内の受信側 (`rx`) に流すためのシンク。
///
/// 出力フレーム (`emit_ok`) 送信時に `total_output_metric` の増分と keyframe 判定を物理的に強制ペアリングする。
/// エラー (`emit_err`) 送信時はメトリクスを増分しない (出力フレーム数 / keyframe 数の意味論を汚さないため)。
///
/// `unreachable!()` 検出契約: シンクと `rx` は `AsyncVideoEncoder` 内で同居するため、
/// 送信失敗 (受信側 drop) は構造上到達不能な不変条件違反 = バグ。 通常運用では起こらない。
#[derive(Debug, Clone)]
pub struct OutputSink {
    tx: EncoderOutputSender,
    total_output_metric: crate::stats::StatsCounter,
    total_output_keyframe_metric: crate::stats::StatsCounter,
}

impl OutputSink {
    pub fn new(
        tx: EncoderOutputSender,
        total_output_metric: crate::stats::StatsCounter,
        total_output_keyframe_metric: crate::stats::StatsCounter,
    ) -> Self {
        Self {
            tx,
            total_output_metric,
            total_output_keyframe_metric,
        }
    }

    /// 出力フレームを 1 件送信し、 `total_output_metric` と (keyframe の場合) `total_output_keyframe_metric` を増分する。
    pub fn emit_ok(&self, frame: VideoFrame) {
        // keyframe フラグは send 前に取り出す。 VideoFrame は data: Vec<u8> を持ち Clone は
        // 圧縮ペイロード全体の deep copy になるため送信は move。
        let is_keyframe = frame.keyframe;
        if self.tx.send(Ok(frame)).is_err() {
            unreachable!("encoder output sink receiver dropped before sink (bug)");
        }
        // 送信成功後に増分することで「送信できなかったフレームをカウントする」嘘を物理的に防ぐ。
        self.total_output_metric.inc();
        if is_keyframe {
            self.total_output_keyframe_metric.inc();
        }
    }

    /// エラーを 1 件送信する (メトリクスは増分しない)。
    pub fn emit_err(&self, err: crate::Error) {
        if self.tx.send(Err(err)).is_err() {
            unreachable!("encoder output sink receiver dropped before sink (bug)");
        }
    }
}

/// 上流の video encoder にキーフレーム要求を送る。
///
/// encoder が見つからない場合は debug ログを出して正常終了する（ベストエフォート）。
/// encoder は後から追加される可能性があり、その時点でキーフレームが届く。
pub async fn request_upstream_video_keyframe(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: &crate::ProcessorId,
    trigger: &str,
) -> crate::Result<()> {
    let maybe_encoder_processor_id = pipeline_handle
        .find_upstream_video_encoder(processor_id)
        .await
        .map_err(|_| crate::Error::new("failed to find upstream video encoder"))?;
    let Some(encoder_processor_id) = maybe_encoder_processor_id else {
        tracing::debug!(
            "skip keyframe request: upstream video encoder not found (processor={}, trigger={})",
            processor_id,
            trigger,
        );
        return Ok(());
    };

    let rpc_sender = pipeline_handle
        .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<VideoEncoderRpcMessage>>(
            &encoder_processor_id,
        )
        .await
        .map_err(|e| {
            crate::Error::new(format!(
                "failed to get video encoder RPC sender ({encoder_processor_id}): {e}"
            ))
        })?;

    rpc_sender
        .send(VideoEncoderRpcMessage::RequestKeyframe)
        .map_err(|_| {
            crate::Error::new(format!(
                "failed to send keyframe request to video encoder: {encoder_processor_id}"
            ))
        })?;
    tracing::debug!(
        "requested keyframe: processor={}, encoder={}, trigger={}",
        processor_id,
        encoder_processor_id,
        trigger,
    );
    Ok(())
}

/// 内部チャンネルベースの映像エンコーダー
///
/// エンコーダー本体で、`VideoEncoder` (wrap) の `run` (processor 経路) から
/// `handle_input_sample_sync` / `poll_output_sync` / `handle_rpc_message_sync` 等の
/// `_sync` 付き内部 API 経由で同期駆動される。 wrap 側は同名の非 `_sync` API
/// (`handle_input_sample` / `poll_output`) を露出し、 内部で本 struct の `_sync` 版に
/// delegate する。 直接利用するときは `next_encoded_frame_async` で非同期に取得する。
///
/// **注意**: 非同期な内部エンコーダー (Nvcodec 等) 使用時、 `AsyncVideoEncoder` を
/// drop する前に必ずエンコード結果を drain し切ること。 drop 順は「`inner` を先に
/// drop → callback スレッドが `sink.emit_ok` した最後の 1 フレームが `rx` に届く
/// → その後 `rx` を drop」で成立するが、 未 drain の状態で drop すると
/// `total_output_video_frame_count` メトリクスが実際の出力数より少ない値のまま
/// 観測される (メトリクスは inner 内で inc されるが、 未回収の frame は下流には流れない)。
#[derive(Debug)]
pub struct AsyncVideoEncoder {
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    total_video_keyframe_request_count_metric: crate::stats::StatsCounter,
    _error_flag: crate::stats::StatsFlag,
    eos: bool,
    keyframe_request_pending: bool,

    // 以下 2 フィールドの宣言順は drop 順を意図的に制御している (Rust 言語仕様で drop 順 = 宣言順)。
    // `inner` を `rx` より先に drop することで、 非同期な内部エンコーダー
    // (Nvcodec 等の callback 完結型 inner) の worker drop 中にコールバックが
    // `sink.emit_ok` → `tx.send` した際に `rx` がまだ alive で send が成功する。
    // 逆順にすると `emit_ok` の `unreachable!()` が発火する。 なお `inner` が
    // `None` (未初期化 = 最初のフレームが未到達) のケースでは callback 経路が動いて
    // いないため、 この順序制約は自動的に満たされる。
    inner: Option<VideoEncoderInner>,
    rx: EncoderOutputReceiver,

    // 下記 `sink` は新規 inner 生成用テンプレートで、 実際に emit する sink は
    // `create_inner` で inner に clone して渡されるため、 上記 drop 順制約とは無関係
    // (drop 順の意味論を持つのは inner が保持する clone の方)。
    sink: OutputSink,
    options: VideoEncoderOptions,
    openh264_lib: Option<Openh264Library>,
}

impl AsyncVideoEncoder {
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
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = OutputSink::new(
            tx,
            total_output_video_frame_count_metric,
            total_output_video_keyframe_count_metric,
        );
        Ok(Self {
            engine_metric,
            codec_metric,
            total_input_video_frame_count_metric,
            total_video_keyframe_request_count_metric,
            _error_flag: error_flag,
            eos: false,
            keyframe_request_pending: false,
            inner: None,
            rx,
            sink,
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
        let sink = self.sink.clone();
        let candidate_engines = options
            .engines
            .clone()
            .unwrap_or_else(|| EngineName::default_video_encoders(self.openh264_lib.is_some()));
        let engine = candidate_engines
            .iter()
            .find(|engine| engine.is_available_video_encode_codec(options.codec))
            .copied();

        match (engine, options.codec) {
            (Some(EngineName::Libvpx), CodecName::Vp8) => VideoEncoderInner::new_vp8(options, sink),
            (Some(EngineName::Libvpx), CodecName::Vp9) => VideoEncoderInner::new_vp9(options, sink),
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H264) => {
                VideoEncoderInner::new_nvcodec_h264(options, sink)
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H265) => {
                VideoEncoderInner::new_nvcodec_h265(options, sink)
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Av1) => {
                VideoEncoderInner::new_nvcodec_av1(options, sink)
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H264) => {
                VideoEncoderInner::new_video_toolbox_h264(options, sink)
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H265) => {
                VideoEncoderInner::new_video_toolbox_h265(options, sink)
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
                VideoEncoderInner::new_openh264(lib, options, sink)
            }
            (Some(EngineName::SvtAv1), CodecName::Av1) => {
                VideoEncoderInner::new_svt_av1(options, sink)
            }
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

    /// wrap (`VideoEncoder`) の `run` 内 RPC 腕から delegate される同期 RPC ハンドラ。
    ///
    /// 現状扱う RPC は `RequestKeyframe` のみで、 受信時に
    /// `total_video_keyframe_request_count` メトリクスを inc し、
    /// `keyframe_request_pending` フラグを立てる (実際の keyframe 要求適用は次の
    /// `handle_input_sample_sync` 呼び出し時に inner へ伝播する)。
    pub(crate) fn handle_rpc_message_sync(&mut self, message: VideoEncoderRpcMessage) {
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

    /// wrap から呼ぶ同期入力 API
    pub(crate) fn handle_input_sample_sync(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        if let Some(sample) = sample {
            let frame = sample.expect_video()?;
            let frame = RawVideoFrame::from_video_frame(frame)?;
            let size = frame.size();

            // 最初のフレームで、解像度を使って初期化する
            if self.inner.is_none() {
                self.initialize_inner(size.width, size.height)?;
            }
            if self.keyframe_request_pending {
                if let Some(inner) = self.inner.as_mut() {
                    inner.request_keyframe();
                }
                self.keyframe_request_pending = false;
            }

            self.total_input_video_frame_count_metric.inc();
            self.inner.as_mut().expect("infallible").encode(frame)?;
        } else {
            self.eos = true;
            if let Some(inner) = &mut self.inner {
                inner.finish()?;
            }
        }
        Ok(())
    }

    /// wrap から呼ぶ同期 poll
    pub(crate) fn poll_output_sync(&mut self) -> Result<EncoderRunOutput> {
        match self.rx.try_recv() {
            Ok(Ok(frame)) => Ok(EncoderRunOutput::Processed(MediaFrame::video(frame))),
            Ok(Err(e)) => Err(e),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if self.eos {
                    Ok(EncoderRunOutput::Finished)
                } else {
                    Ok(EncoderRunOutput::Pending)
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                unreachable!(
                    "encoder output channel disconnected unexpectedly (sink dropped before rx)"
                )
            }
        }
    }

    /// エンコード済みフレームを非同期に取得する。
    ///
    /// - `Some(Ok(frame))`: 正常フレーム
    /// - `Some(Err(e))`: 内部エンコーダーからのエラー
    /// - `None`: 全ての送信側が drop された
    pub async fn next_encoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>> {
        self.rx.recv().await
    }
}

/// 同期 API を保つ VideoEncoder は `AsyncVideoEncoder` の wrap として動作する。
///
/// 既存の外部 API 挙動を維持しつつ、内部は Sender 経由のフレーム受け渡しに移行している。
/// 将来 `AsyncVideoEncoder` 直接利用への段階移行が完了した時点で本 wrap 型は削除される。
#[derive(Debug)]
pub struct VideoEncoder {
    inner_encoder: AsyncVideoEncoder,
}

impl VideoEncoder {
    pub fn new(
        options: &VideoEncoderOptions,
        openh264_lib: Option<Openh264Library>,
        compose_stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        Ok(Self {
            inner_encoder: AsyncVideoEncoder::new(options, openh264_lib, compose_stats)?,
        })
    }

    pub fn name(&self) -> Option<EngineName> {
        self.inner_encoder.name()
    }

    pub fn codec(&self) -> Option<CodecName> {
        self.inner_encoder.codec()
    }

    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> {
        AsyncVideoEncoder::get_engines(codec, is_openh264_available)
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
        self.inner_encoder.handle_rpc_message_sync(message);
    }

    fn handle_input_message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Media(sample) => self.handle_input_sample(Some(sample)),
            Message::Eos => self.handle_input_sample(None),
            Message::Syn(_) => Ok(()),
        }
    }

    pub fn handle_input_sample(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        self.inner_encoder.handle_input_sample_sync(sample)
    }

    pub fn poll_output(&mut self) -> Result<EncoderRunOutput> {
        self.inner_encoder.poll_output_sync()
    }
}

fn drain_video_encoder_output(
    encoder: &mut VideoEncoder,
    output_tx: &mut crate::TrackPublisher,
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
    Libvpx(Box<LibvpxEncoder>), // Box は clippy::large_enum_variant 対策
    Openh264(Openh264Encoder),
    SvtAv1(SvtAv1Encoder),
    #[cfg(target_os = "macos")]
    VideoToolbox(VideoToolboxEncoder),
    #[cfg(feature = "nvcodec")]
    Nvcodec(Box<NvcodecEncoder>), // Box は clippy::large_enum_variant 対策
}

impl VideoEncoderInner {
    fn new_vp8(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let encoder = LibvpxEncoder::new_vp8(options, sink)?;
        Ok(Self::Libvpx(Box::new(encoder)))
    }

    fn new_vp9(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let encoder = LibvpxEncoder::new_vp9(options, sink)?;
        Ok(Self::Libvpx(Box::new(encoder)))
    }

    fn new_openh264(
        lib: Openh264Library,
        options: &VideoEncoderOptions,
        sink: OutputSink,
    ) -> crate::Result<Self> {
        let encoder = Openh264Encoder::new(lib, options, sink)?;
        Ok(Self::Openh264(encoder))
    }

    fn new_svt_av1(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let encoder = SvtAv1Encoder::new(options, sink)?;
        Ok(Self::SvtAv1(encoder))
    }

    #[cfg(target_os = "macos")]
    fn new_video_toolbox_h264(
        options: &VideoEncoderOptions,
        sink: OutputSink,
    ) -> crate::Result<Self> {
        let encoder = VideoToolboxEncoder::new_h264(options, sink)?;
        Ok(Self::VideoToolbox(encoder))
    }

    #[cfg(target_os = "macos")]
    fn new_video_toolbox_h265(
        options: &VideoEncoderOptions,
        sink: OutputSink,
    ) -> crate::Result<Self> {
        let encoder = VideoToolboxEncoder::new_h265(options, sink)?;
        Ok(Self::VideoToolbox(encoder))
    }

    #[cfg(feature = "nvcodec")]
    fn new_nvcodec_h265(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let encoder = NvcodecEncoder::new_h265(options, sink)?;
        Ok(Self::Nvcodec(Box::new(encoder)))
    }

    #[cfg(feature = "nvcodec")]
    fn new_nvcodec_h264(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let encoder = NvcodecEncoder::new_h264(options, sink)?;
        Ok(Self::Nvcodec(Box::new(encoder)))
    }

    #[cfg(feature = "nvcodec")]
    fn new_nvcodec_av1(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let encoder = NvcodecEncoder::new_av1(options, sink)?;
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

    fn request_keyframe(&mut self) {
        match self {
            Self::Libvpx(encoder) => encoder.request_keyframe(),
            Self::Openh264(encoder) => encoder.request_keyframe(),
            Self::SvtAv1(encoder) => encoder.request_keyframe(),
            #[cfg(target_os = "macos")]
            Self::VideoToolbox(encoder) => encoder.request_keyframe(),
            #[cfg(feature = "nvcodec")]
            Self::Nvcodec(encoder) => encoder.request_keyframe(),
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

pub fn default_video_encode_config_for_rpc() -> EncodeConfig {
    // server RPC の既定 encode params は、compose 既定値と同じ値を利用する
    crate::sora::recording_layout_encode_params::LayoutEncodeParams::default().config
}

/// 指定したキーフレーム間隔（フレーム数）を全エンコーダーに設定した EncodeConfig を生成する。
/// HLS セグメント分割に必要なキーフレームを確実に得るために使用する。
pub fn encode_config_with_keyframe_interval(
    keyframe_interval_frames: u32,
    frame_rate: crate::video::FrameRate,
) -> EncodeConfig {
    let mut config = default_video_encode_config_for_rpc();

    // キーフレーム間隔を秒に変換（VideoToolbox の duration 指定で使用）
    let keyframe_interval_duration = std::time::Duration::from_secs_f64(
        keyframe_interval_frames as f64
            / (frame_rate.numerator.get() as f64 / frame_rate.denumerator.get() as f64),
    );
    // frame_rate から計算した duration を全プラットフォームで使えるようにする
    let _ = keyframe_interval_duration;

    // openh264: intra_period (フレーム数)
    config.openh264.intra_period = Some(keyframe_interval_frames as usize);

    // VideoToolbox: max_key_frame_interval (フレーム数) + max_key_frame_interval_duration (秒)
    #[cfg(target_os = "macos")]
    {
        config.video_toolbox_h264.max_key_frame_interval =
            std::num::NonZeroU32::new(keyframe_interval_frames);
        config.video_toolbox_h264.max_key_frame_interval_duration =
            Some(keyframe_interval_duration);
        config.video_toolbox_h265.max_key_frame_interval =
            std::num::NonZeroU32::new(keyframe_interval_frames);
        config.video_toolbox_h265.max_key_frame_interval_duration =
            Some(keyframe_interval_duration);
    }

    // NVENC: idr_period (H264 / HEVC / AV1)
    #[cfg(feature = "nvcodec")]
    {
        if let shiguredo_nvcodec::CodecConfig::H264(ref mut c) = config.nvcodec_h264.codec {
            c.idr_period = Some(keyframe_interval_frames);
        }
        if let shiguredo_nvcodec::CodecConfig::Hevc(ref mut c) = config.nvcodec_h265.codec {
            c.idr_period = Some(keyframe_interval_frames);
        }
        if let shiguredo_nvcodec::CodecConfig::Av1(ref mut c) = config.nvcodec_av1.codec {
            c.idr_period = Some(keyframe_interval_frames);
        }
    }

    config
}

pub async fn create_audio_processor(
    handle: &crate::MediaPipelineHandle,
    input_track_id: crate::TrackId,
    output_track_id: crate::TrackId,
    codec: crate::types::CodecName,
    bitrate_bps: std::num::NonZeroUsize,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    let processor_id = processor_id
        .unwrap_or_else(|| crate::ProcessorId::new(format!("audioEncoder:{input_track_id}")));
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("audio_encoder"),
            move |h| async move {
                #[cfg(feature = "fdk-aac")]
                let fdk_aac_lib = h.config().fdk_aac_lib.clone();
                let encoder = AudioEncoder::new(
                    codec,
                    bitrate_bps,
                    #[cfg(feature = "fdk-aac")]
                    fdk_aac_lib,
                    h.stats(),
                )?;
                encoder.run(h, input_track_id, output_track_id).await
            },
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}

pub async fn create_video_processor(
    handle: &crate::MediaPipelineHandle,
    input_track_id: crate::TrackId,
    output_track_id: crate::TrackId,
    codec: crate::types::CodecName,
    bitrate_bps: std::num::NonZeroUsize,
    frame_rate: crate::video::FrameRate,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    create_video_processor_with_params(
        handle,
        input_track_id,
        output_track_id,
        codec,
        bitrate_bps,
        frame_rate,
        None,
        processor_id,
    )
    .await
}

/// エンコードパラメータを指定してビデオエンコーダプロセッサを作成する。
/// `encode_params` が `None` の場合はデフォルト値を使用する。
#[expect(
    clippy::too_many_arguments,
    reason = "encode_params の指定が必要なため引数が多い"
)]
pub async fn create_video_processor_with_params(
    handle: &crate::MediaPipelineHandle,
    input_track_id: crate::TrackId,
    output_track_id: crate::TrackId,
    codec: crate::types::CodecName,
    bitrate_bps: std::num::NonZeroUsize,
    frame_rate: crate::video::FrameRate,
    encode_params: Option<EncodeConfig>,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    let processor_id = processor_id
        .unwrap_or_else(|| crate::ProcessorId::new(format!("videoEncoder:{input_track_id}")));
    let options = VideoEncoderOptions {
        codec,
        engines: None,
        bitrate: bitrate_bps.get(),
        width: crate::types::EvenUsize::ZERO,
        height: crate::types::EvenUsize::ZERO,
        frame_rate,
        encode_params: encode_params.unwrap_or_else(default_video_encode_config_for_rpc),
    };
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new(crate::media_pipeline::PROCESSOR_TYPE_VIDEO_ENCODER),
            move |h| async move {
                let encoder =
                    VideoEncoder::new(&options, h.config().openh264_lib.clone(), h.stats())?;
                encoder.run(h, input_track_id, output_track_id).await
            },
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::encoder::test_helpers::make_encoder_sink_with_counters;
    use crate::video::{FrameRate, VideoFormat, VideoFrame, VideoFrameSize};

    // 圧縮済み VideoFrame を最小限のフィールドで組み立てる。
    // OutputSink / poll_output_sync の契約テストでは keyframe フラグ以外の値
    // (sample_entry / size / timestamp 等) は分岐判定に影響しない。
    fn compressed_video_frame(keyframe: bool) -> VideoFrame {
        VideoFrame {
            data: vec![0, 1, 2, 3],
            format: VideoFormat::H264,
            keyframe,
            size: Some(VideoFrameSize {
                width: 64,
                height: 64,
            }),
            timestamp: Duration::from_millis(0),
            sample_entry: None,
        }
    }

    // ---- R-2: OutputSink 契約テスト ----
    // encoder 版 OutputSink は decoder 版と異なり keyframe 判定と 2 counter の
    // 同時 inc を持つ。 既存の sample_entry テストは raw_i420_frame が全入力
    // keyframe=true (test_helpers::raw_i420_frame) のため keyframe 分岐の
    // 退行を検出できない。 ここで契約を単独で固定する。

    #[test]
    fn output_sink_emit_ok_keyframe_increments_both_counters() {
        // keyframe=true では total_output_metric と total_output_keyframe_metric の両方が +1 する契約。
        let (sink, mut rx, total, keyframe) = make_encoder_sink_with_counters();
        sink.emit_ok(compressed_video_frame(true));
        assert_eq!(total.get(), 1, "total_output_metric が inc されていない");
        assert_eq!(
            keyframe.get(),
            1,
            "keyframe=true なのに total_output_keyframe_metric が inc されていない"
        );
        let received = rx
            .try_recv()
            .expect("emit_ok したフレームが rx に届いていない");
        let frame = received.expect("emit_ok は Ok で送るのに Err が届いた");
        assert!(frame.keyframe, "受信したフレームの keyframe フラグが false");
    }

    #[test]
    fn output_sink_emit_ok_non_keyframe_increments_total_only() {
        // keyframe=false では total_output_metric のみ +1、 total_output_keyframe_metric は 0 のまま。
        // 「常に両カウンタを inc する」退行を検出できることが本テストの要点。
        let (sink, mut rx, total, keyframe) = make_encoder_sink_with_counters();
        sink.emit_ok(compressed_video_frame(false));
        assert_eq!(
            total.get(),
            1,
            "total_output_metric は非 keyframe でも inc されるべき"
        );
        assert_eq!(
            keyframe.get(),
            0,
            "非 keyframe で total_output_keyframe_metric が誤って inc された"
        );
        let received = rx
            .try_recv()
            .expect("emit_ok したフレームが rx に届いていない");
        let frame = received.expect("emit_ok は Ok で送るのに Err が届いた");
        assert!(!frame.keyframe, "受信したフレームの keyframe フラグが true");
    }

    #[test]
    fn output_sink_emit_err_does_not_increment_counters() {
        // emit_err はメトリクスを一切増分しない
        // (出力フレーム数 / keyframe 数の意味論をエラーで汚さないため)。
        let (sink, mut rx, total, keyframe) = make_encoder_sink_with_counters();
        sink.emit_err(Error::new("test error"));
        assert_eq!(
            total.get(),
            0,
            "emit_err で total_output_metric が誤って inc された"
        );
        assert_eq!(
            keyframe.get(),
            0,
            "emit_err で total_output_keyframe_metric が誤って inc された"
        );
        let received = rx
            .try_recv()
            .expect("emit_err したエラーが rx に届いていない");
        assert!(received.is_err(), "emit_err は Err で送るのに Ok が届いた");
    }

    #[test]
    fn output_sink_clone_shares_counters_with_original() {
        // clone した sink は原本と同じ counter インスタンス (Arc 内部) を共有し、
        // 双方の emit_ok が同一 counter に累積する契約。
        let (sink, _rx, total, keyframe) = make_encoder_sink_with_counters();
        let cloned = sink.clone();
        sink.emit_ok(compressed_video_frame(true));
        cloned.emit_ok(compressed_video_frame(false));
        assert_eq!(
            total.get(),
            2,
            "clone した sink の inc が原本と counter を共有していない"
        );
        assert_eq!(
            keyframe.get(),
            1,
            "keyframe 1 件 + 非 keyframe 1 件で keyframe counter が 1 でない"
        );
    }

    #[test]
    #[should_panic(expected = "encoder output sink receiver dropped before sink")]
    fn output_sink_emit_ok_panics_when_receiver_dropped() {
        // rx が sink より先に drop されるのは AsyncVideoEncoder の drop 順制御下では
        // 発生しない構造だが、 万一発生した場合は fail-fast で panic する契約
        // (unreachable! で「不変条件違反 = バグ」を明示する)。
        let (sink, rx, _total, _keyframe) = make_encoder_sink_with_counters();
        drop(rx);
        sink.emit_ok(compressed_video_frame(true));
    }

    // ---- R-3: AsyncVideoEncoder::poll_output_sync 分岐テスト ----
    // inner=None のまま sink 経由で rx にメッセージを流し込んで
    // poll_output_sync の分岐 (Processed / Err / Pending / Finished) を検証する。
    // Disconnected 分岐は sink と rx が AsyncVideoEncoder 内で同居する構造上
    // 通常運用では起きず、 発生時は unreachable! で panic する。 テストで再現する
    // には sink 側 tx の強制 drop が必要で public API では成立しない。
    // その契約自体は上記 output_sink_emit_ok_panics_when_receiver_dropped でカバー済み。

    fn new_uninitialized_encoder() -> AsyncVideoEncoder {
        let options = VideoEncoderOptions {
            codec: CodecName::Vp8,
            engines: None,
            bitrate: 100_000,
            width: EvenUsize::truncating_new(64),
            height: EvenUsize::truncating_new(64),
            frame_rate: FrameRate {
                numerator: NonZeroUsize::MIN.saturating_add(29),
                denumerator: NonZeroUsize::MIN,
            },
            encode_params: default_video_encode_config_for_rpc(),
        };
        AsyncVideoEncoder::new(&options, None, crate::stats::Stats::new())
            .expect("AsyncVideoEncoder::new が失敗した")
    }

    #[test]
    fn poll_output_sync_returns_processed_when_frame_available() {
        // rx にフレームが届いている場合は Processed(MediaFrame::video(frame)) を返す。
        let mut encoder = new_uninitialized_encoder();
        encoder.sink.emit_ok(compressed_video_frame(true));
        let output = encoder
            .poll_output_sync()
            .expect("poll_output_sync が失敗した");
        assert!(
            matches!(output, EncoderRunOutput::Processed(_)),
            "フレーム到達時に Processed が返っていない"
        );
    }

    #[test]
    fn poll_output_sync_propagates_error_from_rx() {
        // rx に Err が届いている場合はそのまま Err を伝播する (Ok(Err(e)) 分岐)。
        let mut encoder = new_uninitialized_encoder();
        encoder.sink.emit_err(Error::new("encoder error"));
        let result = encoder.poll_output_sync();
        assert!(
            result.is_err(),
            "sink.emit_err の Err が poll_output_sync で伝播されていない"
        );
    }

    #[test]
    fn poll_output_sync_returns_pending_when_empty_and_not_eos() {
        // rx が空 + eos=false の分岐 (TryRecvError::Empty + !eos)。
        let mut encoder = new_uninitialized_encoder();
        assert!(!encoder.eos, "テスト前提: 未初期化 encoder は eos=false");
        let output = encoder
            .poll_output_sync()
            .expect("poll_output_sync が失敗した");
        assert!(
            matches!(output, EncoderRunOutput::Pending),
            "空 rx + eos=false で Pending が返っていない"
        );
    }

    #[test]
    fn poll_output_sync_returns_finished_when_empty_and_eos() {
        // rx が空 + eos=true の分岐 (TryRecvError::Empty + eos)。
        let mut encoder = new_uninitialized_encoder();
        encoder.eos = true;
        let output = encoder
            .poll_output_sync()
            .expect("poll_output_sync が失敗した");
        assert!(
            matches!(output, EncoderRunOutput::Finished),
            "空 rx + eos=true で Finished が返っていない"
        );
    }
}
