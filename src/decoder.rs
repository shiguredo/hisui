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

use self::dav1d::Dav1dDecoder;
use self::libvpx::LibvpxDecoder;
#[cfg(feature = "nvcodec")]
use self::nvcodec::NvcodecDecoder;
use self::openh264::Openh264Decoder;
use self::opus::OpusDecoder;
#[cfg(target_os = "macos")]
use self::video_toolbox::VideoToolboxDecoder;
use crate::{
    Error, Message, ProcessorHandle, Result, TrackId,
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

/// inner デコーダーが出力フレーム / エラーを `AsyncVideoDecoder` 内の `rx` に流すための Sender 型エイリアス
pub type DecoderOutputSender = tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>;

/// inner デコーダーが出力フレーム / エラーを `AsyncVideoDecoder` 内の `rx` に流すための sink。
///
/// inner は `OutputSink` 1 個だけ持てば良く、 出力フレーム送信と metric 計上を物理的に強制ペアリングする。
/// `tests/e2e.rs` 等の integration test (= 別 crate) から inner を直接構築する経路があるため `pub` で公開する。
/// フィールドは private に保ち、 構築は `OutputSink::new` 経由に統一する (別 crate からの struct literal は不可)。
#[derive(Debug, Clone)]
pub struct OutputSink {
    tx: DecoderOutputSender,
    total_output_metric: crate::stats::StatsCounter,
}

impl OutputSink {
    /// 別 crate からも構築できる pub コンストラクタ
    pub fn new(tx: DecoderOutputSender, total_output_metric: crate::stats::StatsCounter) -> Self {
        Self {
            tx,
            total_output_metric,
        }
    }

    /// 出力フレームを 1 件送信して `total_output_video_frame_count_metric` を 1 inc する。
    ///
    /// `tx.send` 失敗 (= Receiver が drop された) は構造体不変条件違反 = bug のため `debug_assert!` で潰す
    /// (`AsyncVideoDecoder` 内で sink と rx は同居するため、 通常時には起こらない)。
    pub fn emit_ok(&self, frame: VideoFrame) {
        self.total_output_metric.inc();
        let send_result = self.tx.send(Ok(frame));
        debug_assert!(
            send_result.is_ok(),
            "decoder output sink receiver dropped before sink (bug)"
        );
    }

    /// エラーを 1 件送信する (metric は inc しない)。
    ///
    /// `tx.send` 失敗 (= Receiver が drop された) は構造体不変条件違反 = bug のため `debug_assert!` で潰す。
    pub fn emit_err(&self, err: crate::Error) {
        let send_result = self.tx.send(Err(err));
        debug_assert!(
            send_result.is_ok(),
            "decoder output sink receiver dropped before sink (bug)"
        );
    }
}

/// 内部 channel ベースの非同期映像デコーダー
///
/// inner デコーダーは出力フレームを sink 経由で内部 channel (`rx`) に push する。
/// sink 自体は `VideoDecoderInner::Initial` variant 内に保持し、 `Initial` → 実 variant 遷移時に
/// `sink.clone()` を実 inner コンストラクタへ渡す。
/// 同期 wrap (`VideoDecoder`) からは `handle_input_sample_sync` / `poll_output_sync` 経由で
/// 同期 API として利用、 直接利用するときは `next_decoded_frame_async` で非同期に取得する。
#[derive(Debug)]
pub struct AsyncVideoDecoder {
    inner: VideoDecoderInner,
    rx: tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>,
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    eos: bool,
}

impl AsyncVideoDecoder {
    pub fn new(options: VideoDecoderOptions, mut compose_stats: crate::stats::Stats) -> Self {
        let engine_metric = compose_stats.string("engine");
        let codec_metric = compose_stats.string("codec");
        let total_input_video_frame_count_metric =
            compose_stats.counter("total_input_video_frame_count");
        let total_output_metric = compose_stats.counter("total_output_video_frame_count");
        compose_stats.flag("error").set(false);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = OutputSink::new(tx, total_output_metric);
        Self {
            inner: VideoDecoderInner::Initial { options, sink },
            rx,
            engine_metric,
            codec_metric,
            total_input_video_frame_count_metric,
            eos: false,
        }
    }

    /// 同期 wrap (`VideoDecoder`) から呼ぶ同期入力 API。
    ///
    /// `inner.decode()` / `inner.finish()` 内で発生した同期 Err は `?` 直返しで同期返却する。
    /// Nvcodec callback の Err は `sink.emit_err()` 経由で channel に流れ、 後続の
    /// `poll_output_sync` の `try_recv` で受信される。
    pub fn handle_input_sample_sync(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        if let Some(sample) = sample {
            let frame = sample.expect_video()?;
            self.total_input_video_frame_count_metric.inc();
            // `VideoDecoderInner` の各 variant は内部に sink を内包する設計のため、
            // ここでは sink を引数で渡さない (Initial 遷移時に Initial variant 内の sink を
            // `initialize_decoder` 経由で実 inner コンストラクタへ clone 渡し)。
            self.inner
                .decode(&frame, &self.codec_metric, &self.engine_metric)?;
        } else {
            self.eos = true;
            self.inner.finish()?;
        }
        Ok(())
    }

    /// 同期 wrap (`VideoDecoder`) から呼ぶ同期 poll。
    ///
    /// 既存 `poll_output()` の戻り値型と意味論を完全維持。 `try_recv` の Empty / Disconnected を
    /// `eos` と組み合わせて判定する。
    pub fn poll_output_sync(&mut self) -> Result<DecoderRunOutput> {
        match self.rx.try_recv() {
            Ok(Ok(frame)) => Ok(DecoderRunOutput::Processed(MediaFrame::video(frame))),
            Ok(Err(e)) => Err(e),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if self.eos {
                    // 既存実装は eos で即 Finished を返していた。 wrap 構造では同期 inner の emit が
                    // すべて `handle_input_sample_sync` 内で完了し、 Nvcodec も `finish()` が
                    // flush 待ち合わせ済のため、 eos に至った時点で sink 内の残物はない。
                    Ok(DecoderRunOutput::Finished)
                } else {
                    Ok(DecoderRunOutput::Pending)
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // sink (= tx) は `inner` (Initial variant 内 sink、 もしくは実 inner の sink フィールド、
                // もしくは NvcodecDecoder の callback closure) 内で生存しており、 さらに `OutputSink::clone`
                // で配布された複製も生存しているため、 `AsyncVideoDecoder` 自身が live な間は
                // すべての tx が drop される経路はない。 したがって Disconnected は構造上到達不能な
                // 不変条件違反 (= bug) であり、 silent に Err で覆い隠さず即時 panic で検出する。
                unreachable!(
                    "decoder output channel disconnected unexpectedly (sink dropped before rx)"
                )
            }
        }
    }

    /// 非同期入力 API (新規)。
    ///
    /// `None` 返却は `tx` が drop された場合のみ (構造体不変条件違反 = bug、 `OutputSink::emit_*`
    /// 側の `debug_assert!` で既に検出されているはず)。
    pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>> {
        self.rx.recv().await
    }

    /// engine 選択ロジック本体 (既存 `VideoDecoder::get_engines` のロジックを移植)。
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

/// 同期 API を提供する映像デコーダー (`AsyncVideoDecoder` の薄い wrap)。
///
/// 既存の外部 API (`new`, `handle_input_sample`, `poll_output`, `run`, `handle_input_message`,
/// `get_engines`) の挙動は維持する。 内部は `AsyncVideoDecoder` への delegate。
#[derive(Debug)]
pub struct VideoDecoder {
    inner_decoder: AsyncVideoDecoder,
}

impl VideoDecoder {
    pub fn new(options: VideoDecoderOptions, compose_stats: crate::stats::Stats) -> Self {
        Self {
            inner_decoder: AsyncVideoDecoder::new(options, compose_stats),
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

            match drain_video_decoder_output(&mut self, &mut output_tx)? {
                DrainResult::PipelineClosed | DrainResult::Finished => {
                    output_tx.send_eos();
                    break;
                }
                DrainResult::Pending => {}
            }

            if is_eos {
                return Err(Error::new("video decoder still pending after EOS"));
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
        self.inner_decoder.handle_input_sample_sync(sample)
    }

    pub fn poll_output(&mut self) -> Result<DecoderRunOutput> {
        self.inner_decoder.poll_output_sync()
    }

    pub fn get_engines(codec: CodecName, is_openh264_available: bool) -> Vec<EngineName> {
        AsyncVideoDecoder::get_engines(codec, is_openh264_available)
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

pub fn drain_video_decoder_output(
    decoder: &mut VideoDecoder,
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

#[derive(Debug)]
enum VideoDecoderInner {
    Initial {
        options: VideoDecoderOptions,
        // Initial → 実 variant 遷移時に inner コンストラクタへ clone を渡す。
        sink: OutputSink,
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
    fn initialize_decoder(
        &mut self,
        frame: &VideoFrame,
        codec_metric: &crate::stats::StatsString,
        engine_metric: &crate::stats::StatsString,
        options: VideoDecoderOptions,
        sink: OutputSink,
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
                *self =
                    NvcodecDecoder::new_h264(&options.decode_params, sink).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::H265) => {
                *self =
                    NvcodecDecoder::new_h265(&options.decode_params, sink).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Vp8) => {
                *self = NvcodecDecoder::new_vp8(&options.decode_params, sink).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Vp9) => {
                *self = NvcodecDecoder::new_vp9(&options.decode_params, sink).map(Self::Nvcodec)?;
            }
            #[cfg(feature = "nvcodec")]
            (Some(EngineName::Nvcodec), CodecName::Av1) => {
                *self = NvcodecDecoder::new_av1(&options.decode_params, sink).map(Self::Nvcodec)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H264) => {
                *self = VideoToolboxDecoder::new_h264(frame, sink)
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::H265) => {
                *self = VideoToolboxDecoder::new_h265(frame, sink)
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::Vp9) => {
                *self = VideoToolboxDecoder::new_vp9(frame, sink)
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            #[cfg(target_os = "macos")]
            (Some(EngineName::VideoToolbox), CodecName::Av1) => {
                *self = VideoToolboxDecoder::new_av1(frame, sink)
                    .map(Box::new)
                    .map(Self::VideoToolbox)?;
            }
            (Some(EngineName::Openh264), CodecName::H264) => {
                let lib = options.openh264_lib.ok_or_else(|| {
                    crate::Error::new("OpenH264 library is required for H.264 decoding")
                })?;
                *self = Openh264Decoder::new(lib.clone(), sink).map(Self::Openh264)?;
            }
            (Some(EngineName::Libvpx), CodecName::Vp8) => {
                *self = LibvpxDecoder::new_vp8(sink).map(Self::Libvpx)?;
            }
            (Some(EngineName::Libvpx), CodecName::Vp9) => {
                *self = LibvpxDecoder::new_vp9(sink).map(Self::Libvpx)?;
            }
            (Some(EngineName::Dav1d), CodecName::Av1) => {
                *self = Dav1dDecoder::new(sink).map(Self::Dav1d)?;
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
            Self::Initial { options, sink } => {
                // OutputSink / VideoDecoderOptions ともに Clone は cheap (内部 Arc bump のみ)。
                let options = options.clone();
                let sink = sink.clone();
                self.initialize_decoder(frame, codec_metric, engine_metric, options, sink)?;
                self.decode(frame, codec_metric, engine_metric)
            }
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
    #[test]
    fn vp9_without_size_skips_video_toolbox() {
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
        let mut decoder = VideoDecoder::new(options_without_nvcodec(engines), stats);
        // デコーダーを初期化する（空データなのでデコード自体は失敗するが、エンジン選択は成功するはず）
        let _ = decoder.handle_input_sample(Some(MediaFrame::video(frame)));

        assert!(
            matches!(decoder.inner_decoder.inner, VideoDecoderInner::Libvpx(_)),
            "expected Libvpx decoder, got {:?}",
            std::mem::discriminant(&decoder.inner_decoder.inner)
        );
    }

    /// size: None の AV1 フレームで VideoToolbox がスキップされ Dav1d が選ばれることを確認する
    #[test]
    fn av1_without_size_skips_video_toolbox() {
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
        let mut decoder = VideoDecoder::new(options_without_nvcodec(engines), stats);
        let _ = decoder.handle_input_sample(Some(MediaFrame::video(frame)));

        assert!(
            matches!(decoder.inner_decoder.inner, VideoDecoderInner::Dav1d(_)),
            "expected Dav1d decoder, got {:?}",
            std::mem::discriminant(&decoder.inner_decoder.inner)
        );
    }

    /// `AsyncVideoDecoder::next_decoded_frame_async` で sink から emit された frame を受信できることを確認する
    ///
    /// inner を実 codec で初期化すると fixture が必要になるため、 `Initial` variant 内の sink を
    /// pattern matching で取り出して直接 `emit_ok` を呼ぶ形で検証する。 これは `AsyncVideoDecoder` の
    /// sink → channel → `next_decoded_frame_async` の経路が正しく繋がっているかの smoke test。
    #[tokio::test(flavor = "multi_thread")]
    async fn async_video_decoder_next_decoded_frame_async_returns_emitted_frame() -> Result<()> {
        let options = VideoDecoderOptions::default();
        let stats = crate::stats::Stats::new();
        let mut decoder = AsyncVideoDecoder::new(options, stats);

        // Initial variant 内の sink を取り出して直接 emit する (実 inner を初期化せずに channel 経路だけ検証)
        let sink = match &decoder.inner {
            VideoDecoderInner::Initial { sink, .. } => sink.clone(),
            _ => panic!("初期状態は Initial variant が期待される"),
        };

        let test_frame = VideoFrame {
            data: vec![1, 2, 3],
            format: VideoFormat::Vp9,
            keyframe: true,
            size: Some(VideoFrameSize {
                width: 16,
                height: 16,
            }),
            timestamp: Duration::from_millis(0),
            sample_entry: None,
        };
        sink.emit_ok(test_frame.clone());

        match decoder.next_decoded_frame_async().await {
            Some(Ok(frame)) => {
                assert_eq!(frame.data, vec![1, 2, 3]);
                assert_eq!(frame.format, VideoFormat::Vp9);
            }
            other => panic!("正常フレーム (Some(Ok(_))) を期待したが {other:?} を受信した"),
        }

        Ok(())
    }

    /// size ありの VP9 フレームでは macOS 対応環境なら VideoToolbox、非対応なら Libvpx が選ばれることを確認する
    #[test]
    fn vp9_with_size_selects_available_engine() {
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
        let mut decoder = VideoDecoder::new(options_without_nvcodec(engines), stats);
        let _ = decoder.handle_input_sample(Some(MediaFrame::video(frame)));

        // size ありなら VideoToolbox がスキップされずにエンジン選択が行われることを確認する。
        // どちらが選ばれるかは実行環境の VP9 ハードウェアデコード対応状況に依存するため、
        // ここでは「いずれかの有効なエンジンが選択されること」のみを検証する。
        #[cfg(target_os = "macos")]
        let is_valid = matches!(decoder.inner_decoder.inner, VideoDecoderInner::Libvpx(_))
            || matches!(
                decoder.inner_decoder.inner,
                VideoDecoderInner::VideoToolbox(_)
            );
        #[cfg(not(target_os = "macos"))]
        let is_valid = matches!(decoder.inner_decoder.inner, VideoDecoderInner::Libvpx(_));
        assert!(
            is_valid,
            "expected Libvpx or VideoToolbox decoder, got {:?}",
            std::mem::discriminant(&decoder.inner_decoder.inner)
        );
    }
}
