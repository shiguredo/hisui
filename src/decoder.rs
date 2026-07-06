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

/// `drain_audio_decoder_output()` の結果
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

/// 内部デコーダーが出力フレーム / エラーを `VideoDecoder` 内の受信側 (`output_rx`) に流すための送信側の型エイリアス
pub type DecoderOutputSender = tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>;

/// `VideoDecoder` 内部で内部デコーダーからの出力フレーム / エラーを受け取る受信側の型エイリアス
pub type DecoderOutputReceiver = tokio::sync::mpsc::UnboundedReceiver<crate::Result<VideoFrame>>;

/// 内部デコーダーが出力フレーム / エラーを `VideoDecoder` 内の受信側 (`output_rx`) に流すためのシンク。
///
/// 出力フレーム (`emit_ok`) 送信時に `total_output_metric` の増分を物理的に強制ペアリングする。
/// エラー (`emit_err`) 送信時はメトリクスを増分しない (出力フレーム数の意味論を汚さないため)。
///
/// `unreachable!()` 検出契約: シンクと `output_rx` は `VideoDecoder` 内で同居するため、
/// 送信失敗 (受信側 drop) は構造上到達不能な不変条件違反 = バグ。 通常運用では起こらない。
/// 同じ理由で `poll_output` の `Disconnected` 分岐は `unreachable!()` で潰す
/// (`next_decoded_frame` の `None` 返却も構造上起こらないが、 こちらは `Option` を
/// そのまま返す)。
#[derive(Debug, Clone)]
pub struct OutputSink {
    tx: DecoderOutputSender,
    total_output_metric: crate::stats::StatsCounter,
}

impl OutputSink {
    pub fn new(tx: DecoderOutputSender, total_output_metric: crate::stats::StatsCounter) -> Self {
        Self {
            tx,
            total_output_metric,
        }
    }

    /// 出力フレームを 1 件送信して `total_output_metric` を 1 だけ増分する。
    pub fn emit_ok(&self, frame: VideoFrame) {
        if self.tx.send(Ok(frame)).is_err() {
            unreachable!("decoder output sink receiver dropped before sink (bug)");
        }
        // 送信成功後に増分することで「送信できなかったフレームをカウントする」嘘を物理的に防ぐ。
        self.total_output_metric.inc();
    }

    /// エラーを 1 件送信する (メトリクスは増分しない)。
    pub fn emit_err(&self, err: crate::Error) {
        if self.tx.send(Err(err)).is_err() {
            unreachable!("decoder output sink receiver dropped before sink (bug)");
        }
    }
}

/// 内部チャンネルベースの映像デコーダー
///
/// decoder task loop (mp4 reader / RTSP / RTMP / SRT) や `run` (processor 経路) からは
/// `handle_input_sample` / `poll_output` 経由で同期的に駆動し、 直接利用するときは
/// `next_decoded_frame` で非同期に取得する。
///
/// **注意**: 非同期な内部デコーダー (Nvcodec 等) 使用時、 `VideoDecoder` を drop する前に
/// EOS + drain (`handle_input_sample(None)` + `poll_output` ループ) を完走させないと、
/// コールバックが drop 中に emit した残物とメトリクス (`total_output_video_frame_count`) が
/// 乖離する可能性がある (エラー時の warm-up 中止経路等で発生し得る)。
#[derive(Debug)]
pub struct VideoDecoder {
    engine_metric: crate::stats::StatsString,
    codec_metric: crate::stats::StatsString,
    total_input_video_frame_count_metric: crate::stats::StatsCounter,
    eos: bool,

    // 以下 2 フィールドの宣言順は drop 順を意図的に制御している (Rust 言語仕様で drop 順 = 宣言順)。
    // `inner` を `output_rx` より先に drop することで、 非同期な内部デコーダー (Nvcodec) の
    // worker drop 中にコールバックが `sink.emit_ok` → `tx.send` した際に `output_rx` が
    // まだ alive で send が成功する。 逆順にすると `emit_ok` の `unreachable!()` が発火する。
    inner: VideoDecoderInner,
    output_rx: DecoderOutputReceiver,
}

impl VideoDecoder {
    pub fn new(options: VideoDecoderOptions, mut compose_stats: crate::stats::Stats) -> Self {
        let engine_metric = compose_stats.string("engine");
        let codec_metric = compose_stats.string("codec");
        let total_input_video_frame_count_metric =
            compose_stats.counter("total_input_video_frame_count");
        let total_output_metric = compose_stats.counter("total_output_video_frame_count");
        compose_stats.flag("error").set(false);
        let (tx, output_rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = OutputSink::new(tx, total_output_metric);
        Self {
            engine_metric,
            codec_metric,
            total_input_video_frame_count_metric,
            eos: false,
            inner: VideoDecoderInner::Initial { options, sink },
            output_rx,
        }
    }

    /// decoder task loop / `run` から呼ぶ同期入力 API。
    ///
    /// `inner.decode()` / `inner.finish()` 内で発生した同期 `Err` は `?` 直返しで同期返却する。
    /// 内部デコーダーのコールバック等で非同期に発生した `Err` は `sink.emit_err()` 経由で
    /// チャンネルに流れ、 後続の `poll_output` の `try_recv` で受信される。
    pub fn handle_input_sample(&mut self, sample: Option<MediaFrame>) -> Result<()> {
        if let Some(sample) = sample {
            let frame = sample.expect_video()?;
            self.total_input_video_frame_count_metric.inc();
            self.inner
                .decode(&frame, &self.codec_metric, &self.engine_metric)?;
        } else {
            self.eos = true;
            self.inner.finish()?;
        }
        Ok(())
    }

    /// decoder task loop / `run` から呼ぶ同期 poll。
    ///
    /// `try_recv` の結果を射影する: `Ok(Ok)` → `Processed`、 `Ok(Err)` → `Err`、
    /// `Empty` は `eos` と組み合わせて `Finished` (eos) / `Pending` (非 eos)、
    /// `Disconnected` は構造上到達不能で `unreachable!()`。
    pub fn poll_output(&mut self) -> Result<DecoderRunOutput> {
        match self.output_rx.try_recv() {
            Ok(Ok(frame)) => Ok(DecoderRunOutput::Processed(MediaFrame::video(frame))),
            Ok(Err(e)) => Err(e),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if self.eos {
                    // `handle_input_sample(None)` 経由で同期・非同期どちらの内部デコーダーも
                    // フラッシュ完了しているため、 `eos` 時点でチャンネル内の残物はない。
                    Ok(DecoderRunOutput::Finished)
                } else {
                    Ok(DecoderRunOutput::Pending)
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // `OutputSink` の同居不変条件により構造上到達不能 (詳細は `OutputSink` の docstring 参照)。
                unreachable!(
                    "decoder output channel disconnected unexpectedly (sink dropped before rx)"
                )
            }
        }
    }

    /// デコード済みフレームを非同期に取得する。
    ///
    /// - `Some(Ok(frame))`: 正常フレーム
    /// - `Some(Err(e))`: 内部デコーダーからのエラー
    /// - `None`: 全ての送信側が drop された
    ///
    /// 現状の実装では EOS 経路で sink を drop しないため `None` は構造上到達しないが、
    /// 将来 EOS を非同期経路で通知する形が必要になった際に `None` を EOS シグナルとして
    /// 活用できるよう `Option` を維持している。
    pub async fn next_decoded_frame(&mut self) -> Option<crate::Result<VideoFrame>> {
        self.output_rx.recv().await
    }

    /// processor モデル (`ProcessorHandle` + subscribe / publish) 用の駆動 API。
    ///
    /// 入力トラックを subscribe し、 `handle_input_sample` / `poll_output` の drain ループで
    /// デコード結果を出力トラックへ流す。
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

            match message {
                Message::Media(sample) => self.handle_input_sample(Some(sample))?,
                Message::Eos => self.handle_input_sample(None)?,
                Message::Syn(_) => {}
            }

            loop {
                match self.poll_output()? {
                    DecoderRunOutput::Processed(sample) => {
                        if !output_tx.send_media(sample) {
                            output_tx.send_eos();
                            return Ok(());
                        }
                    }
                    DecoderRunOutput::Pending => break,
                    DecoderRunOutput::Finished => {
                        output_tx.send_eos();
                        return Ok(());
                    }
                }
            }

            if is_eos {
                return Err(Error::new("video decoder still pending after EOS"));
            }
        }
    }

    /// codec とライブラリの利用可否に応じて候補となる engine のリストを返す。
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

#[derive(Debug)]
enum VideoDecoderInner {
    Initial {
        options: VideoDecoderOptions,
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
            matches!(decoder.inner, VideoDecoderInner::Libvpx(_)),
            "Libvpx デコーダーを期待したが {:?} を得た",
            std::mem::discriminant(&decoder.inner)
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
            matches!(decoder.inner, VideoDecoderInner::Dav1d(_)),
            "Dav1d デコーダーを期待したが {:?} を得た",
            std::mem::discriminant(&decoder.inner)
        );
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
        let is_valid = matches!(decoder.inner, VideoDecoderInner::Libvpx(_))
            || matches!(decoder.inner, VideoDecoderInner::VideoToolbox(_));
        #[cfg(not(target_os = "macos"))]
        let is_valid = matches!(decoder.inner, VideoDecoderInner::Libvpx(_));
        assert!(
            is_valid,
            "Libvpx または VideoToolbox デコーダーを期待したが {:?} を得た",
            std::mem::discriminant(&decoder.inner)
        );
    }

    /// テスト用の最小 `VideoFrame` を作る (data は任意のバイト列、 timestamp は 0)
    fn make_test_video_frame(data: Vec<u8>) -> VideoFrame {
        VideoFrame {
            data,
            format: VideoFormat::Vp9,
            keyframe: true,
            size: None,
            timestamp: Duration::ZERO,
            sample_entry: None,
        }
    }

    /// `emit_ok` はフレームを受信側 (`rx`) に送信し、 `total_output_metric` を 1 増分する
    #[test]
    fn output_sink_emit_ok_sends_frame_and_increments_metric() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut stats = crate::stats::Stats::new();
        let counter = stats.counter("test_total_output");
        let sink = OutputSink::new(tx, counter.clone());

        let frame = make_test_video_frame(vec![1, 2, 3]);
        sink.emit_ok(frame);

        match rx.try_recv() {
            Ok(Ok(received)) => {
                assert_eq!(
                    received.data,
                    vec![1, 2, 3],
                    "送信したフレームと一致するはず"
                );
            }
            other => panic!("Ok(Ok(_)) を期待したが {other:?} を受信した"),
        }
        assert_eq!(
            counter.get(),
            1,
            "emit_ok を 1 回呼ぶとカウンターが 1 増分されるはず"
        );
    }

    /// `emit_err` はエラーを受信側 (`rx`) に送信するが、 `total_output_metric` は増分しない
    ///
    /// (「emit_ok だけがカウンターを増分する」契約の回帰検出。
    /// この契約が崩れるとメトリクス二重計上 / 不正計上の温床になる)
    #[test]
    fn output_sink_emit_err_sends_error_without_incrementing_metric() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut stats = crate::stats::Stats::new();
        let counter = stats.counter("test_total_output");
        let sink = OutputSink::new(tx, counter.clone());

        sink.emit_err(crate::Error::new("test sink error"));

        match rx.try_recv() {
            Ok(Err(e)) => {
                let msg = e.display().to_string();
                assert!(
                    msg.contains("test sink error"),
                    "送信したエラーメッセージが含まれているはず: {msg}"
                );
            }
            other => panic!("Ok(Err(_)) を期待したが {other:?} を受信した"),
        }
        assert_eq!(counter.get(), 0, "emit_err はカウンターを増分しないはず");
    }

    /// `OutputSink::clone()` で複製した 2 つのシンクから emit しても、 同一の受信側 (`rx`) で受信できる
    ///
    /// (`NvcodecDecoder` の `build_handler` が `sink.clone()` をコールバッククロージャに move する
    /// 設計の不変条件を回帰検出する)
    #[test]
    fn output_sink_clone_shares_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut stats = crate::stats::Stats::new();
        let counter = stats.counter("test_total_output");
        let sink_a = OutputSink::new(tx, counter.clone());
        let sink_b = sink_a.clone();

        sink_a.emit_ok(make_test_video_frame(vec![0xAA]));
        sink_b.emit_ok(make_test_video_frame(vec![0xBB]));

        let first = rx.try_recv().expect("1 件目を受信できるはず");
        let second = rx.try_recv().expect("2 件目を受信できるはず");
        let first = first.expect("Ok のはず");
        let second = second.expect("Ok のはず");
        assert_eq!(first.data, vec![0xAA], "FIFO 順で sink_a 由来が先");
        assert_eq!(second.data, vec![0xBB], "FIFO 順で sink_b 由来が後");
        assert_eq!(
            counter.get(),
            2,
            "2 つの sink (clone でも同一 metric を共有) から各 1 回 emit_ok で合計 2 inc"
        );
    }

    /// 受信側 `rx` を先に drop した後の `emit_ok` は `unreachable!()` で panic する
    ///
    /// (構造体不変条件: シンクと受信側は `VideoDecoder` 内で同居するため、
    /// 通常運用ではこの状況に到達しない。 万一シンクと受信側の所有関係を将来変更してしまった場合に
    /// 静かに失敗させず即時 panic でバグを検出する)
    #[test]
    #[should_panic(expected = "decoder output sink receiver dropped before sink")]
    fn output_sink_emit_ok_panics_when_receiver_dropped() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut stats = crate::stats::Stats::new();
        let counter = stats.counter("test_total_output");
        let sink = OutputSink::new(tx, counter);
        drop(rx); // rx を先に drop する

        sink.emit_ok(make_test_video_frame(vec![1, 2, 3]));
    }

    /// `poll_output` の Empty + eos==true 分岐: EOS 受信後で channel 空なら `Finished` を返す
    #[test]
    fn poll_output_returns_finished_when_eos_and_channel_empty() {
        let mut decoder =
            VideoDecoder::new(VideoDecoderOptions::default(), crate::stats::Stats::new());
        // EOS で eos=true に遷移させる (inner は Initial のまま、 channel も空)。
        // Initial バリアントの `finish()` は no-op (実バックエンド未初期化のため
        // フラッシュ対象が存在しない) なので、 EOS を受けても sink には何も emit されず、
        // `output_rx` は Empty のまま、 `self.eos = true` だけがセットされる。
        // したがって直後の `poll_output` は Empty + eos==true 分岐に確定で入る。
        decoder
            .handle_input_sample(None)
            .expect("EOS は Initial でも Ok");

        assert!(
            matches!(decoder.poll_output(), Ok(DecoderRunOutput::Finished)),
            "Empty + eos==true で Finished を期待した"
        );
    }

    /// `poll_output` の Empty + eos==false 分岐: 初期状態 (channel 空、 eos 未設定) なら `Pending` を返す
    #[test]
    fn poll_output_returns_pending_when_not_eos_and_channel_empty() {
        let mut decoder =
            VideoDecoder::new(VideoDecoderOptions::default(), crate::stats::Stats::new());
        // handle_input_sample を一度も呼ばない (eos=false、 channel 空)

        assert!(
            matches!(decoder.poll_output(), Ok(DecoderRunOutput::Pending)),
            "Empty + eos==false で Pending を期待した"
        );
    }

    /// `poll_output` の Ok(Err(_)) 分岐: 非同期な内部デコーダーのコールバックが
    /// `sink.emit_err()` 経由でチャンネルに流したエラーが、 同期経路で `Err` として返却されることを検証する。
    ///
    /// この経路は `VideoDecoder::run` の drain ループが Nvcodec の非同期エラーを拾い上げる
    /// 唯一の同期契約であり、 silent に潰れる形の改修 (例: `Err(e) => Ok(Pending)`) が
    /// 混入しても integration test では実 Err ケースを再現しにくいため、 単体テストで担保する。
    #[test]
    fn poll_output_returns_err_when_emit_err_received() {
        let mut decoder =
            VideoDecoder::new(VideoDecoderOptions::default(), crate::stats::Stats::new());

        // Initial バリアント内のシンクを取り出してチャンネルに Err を流す
        let sink = match &decoder.inner {
            VideoDecoderInner::Initial { sink, .. } => sink.clone(),
            _ => panic!("初期状態は Initial バリアントが期待される"),
        };
        sink.emit_err(crate::Error::new("test callback error"));

        match decoder.poll_output() {
            Err(e) => {
                let msg = e.display().to_string();
                assert!(
                    msg.contains("test callback error"),
                    "予期したエラーメッセージが含まれていない: {msg}"
                );
            }
            Ok(DecoderRunOutput::Processed(_)) => panic!("Err を期待したが Processed を受信した"),
            Ok(DecoderRunOutput::Pending) => panic!("Err を期待したが Pending を受信した"),
            Ok(DecoderRunOutput::Finished) => panic!("Err を期待したが Finished を受信した"),
        }
    }
}
