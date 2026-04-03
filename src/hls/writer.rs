use std::collections::VecDeque;
use std::io::{BufWriter, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::codec_string::CodecResolutionState;

use crate::obsws::input_registry::HlsSegmentFormat;

/// ファイル拡張子から content-type を返す
fn content_type_for_filename(filename: &str) -> &'static str {
    if filename.ends_with(".m3u8") {
        "application/vnd.apple.mpegurl"
    } else if filename.ends_with(".ts") {
        "video/mp2t"
    } else if filename.ends_with(".mp4") || filename.ends_with(".m4s") {
        "video/mp4"
    } else {
        "application/octet-stream"
    }
}

/// HLS writer の統計値
struct HlsWriterStats {
    total_input_video_frame_count: crate::stats::StatsCounter,
    total_input_audio_frame_count: crate::stats::StatsCounter,
    total_segment_count: crate::stats::StatsCounter,
    total_segment_byte_size: crate::stats::StatsCounter,
    total_deleted_segment_count: crate::stats::StatsCounter,
    current_retained_segment_count: crate::stats::StatsGauge,
}

pub enum HlsWriterRpcMessage {
    /// 入力を明示的に閉じ、finalize / cleanup に進ませる。
    /// 上流の残フレームをすべて受け切ることまでは保証しない。
    Finish {
        reply_tx: tokio::sync::oneshot::Sender<()>,
    },
}

impl HlsWriterStats {
    fn new(stats: &mut crate::stats::Stats) -> Self {
        Self {
            total_input_video_frame_count: stats.counter("total_input_video_frame_count"),
            total_input_audio_frame_count: stats.counter("total_input_audio_frame_count"),
            total_segment_count: stats.counter("total_segment_count"),
            total_segment_byte_size: stats.counter("total_segment_byte_size"),
            total_deleted_segment_count: stats.counter("total_deleted_segment_count"),
            current_retained_segment_count: stats.gauge("current_retained_segment_count"),
        }
    }
}

/// S3 操作のステータスコード別カウンタ
struct S3StatusCounters {
    stats: crate::stats::Stats,
    metric_name: &'static str,
}

impl S3StatusCounters {
    fn new(stats: &crate::stats::Stats, metric_name: &'static str) -> Self {
        Self {
            stats: stats.clone(),
            metric_name,
        }
    }

    fn record(&self, status_code: u16) {
        // Stats を都度 clone してラベル付きカウンタを取得する。
        // Stats の内部で同一キーは共有されるため、同じ status_code への加算は 1 つの系列に集約される。
        // clone + set_default_label のコストは S3 の HTTP 往復に比べて無視できる。
        let mut stats = self.stats.clone();
        stats.set_default_label("status_code", &status_code.to_string());
        stats.counter(self.metric_name).inc();
    }
}

/// HLS 出力先のストレージ抽象
enum HlsStorage {
    Filesystem(FilesystemStorage),
    S3(Box<S3Storage>),
}

struct FilesystemStorage {
    output_directory: PathBuf,
}

struct S3Storage {
    client: crate::s3::S3HttpClient,
    bucket: String,
    prefix: String,
    put_counts: S3StatusCounters,
    delete_counts: S3StatusCounters,
    put_error_count: crate::stats::StatsCounter,
    delete_error_count: crate::stats::StatsCounter,
}

impl S3Storage {
    /// prefix とファイル名から S3 オブジェクトキーを構築する
    fn object_key(&self, filename: &str) -> String {
        if self.prefix.is_empty() {
            filename.to_owned()
        } else {
            format!("{}/{filename}", self.prefix)
        }
    }
}

impl HlsStorage {
    /// セグメントファイルを書き出す
    async fn write_segment(&self, filename: &str, data: &[u8]) -> crate::Result<()> {
        match self {
            Self::Filesystem(fs) => {
                let path = fs.output_directory.join(filename);
                let mut file = BufWriter::new(std::fs::File::create(&path).map_err(|e| {
                    crate::Error::new(format!(
                        "failed to create segment file {}: {e}",
                        path.display()
                    ))
                })?);
                file.write_all(data)
                    .map_err(|e| crate::Error::new(format!("failed to write segment: {e}")))?;
                file.flush()
                    .map_err(|e| crate::Error::new(format!("failed to flush segment: {e}")))?;
                Ok(())
            }
            Self::S3(s3) => {
                let key = s3.object_key(filename);
                let content_type = content_type_for_filename(filename);
                let request = s3
                    .client
                    .client()
                    .put_object()
                    .bucket(&s3.bucket)
                    .key(&key)
                    .body(data.to_vec())
                    .content_type(content_type)
                    .build_request()?;
                match s3.client.execute(&request).await {
                    Ok(response) => {
                        s3.put_counts.record(response.status_code);
                        if !response.is_success() {
                            return Err(crate::Error::new(format!(
                                "S3 PutObject failed for {key}: status={}",
                                response.status_code
                            )));
                        }
                    }
                    Err(e) => {
                        s3.put_error_count.inc();
                        return Err(e);
                    }
                }
                Ok(())
            }
        }
    }

    /// プレイリストを書き出す（filesystem はアトミック rename、S3 は直接上書き）
    async fn write_playlist(&self, filename: &str, content: &[u8]) -> crate::Result<()> {
        match self {
            Self::Filesystem(fs) => {
                let playlist_path = fs.output_directory.join(filename);
                let tmp_path = fs.output_directory.join(format!(".{filename}.tmp"));
                std::fs::write(&tmp_path, content).map_err(|e| {
                    crate::Error::new(format!(
                        "failed to write temporary playlist {}: {e}",
                        tmp_path.display()
                    ))
                })?;
                std::fs::rename(&tmp_path, &playlist_path).map_err(|e| {
                    crate::Error::new(format!(
                        "failed to rename playlist {} -> {}: {e}",
                        tmp_path.display(),
                        playlist_path.display()
                    ))
                })?;
                Ok(())
            }
            Self::S3(s3) => {
                let key = s3.object_key(filename);
                let content_type = content_type_for_filename(filename);
                let request = s3
                    .client
                    .client()
                    .put_object()
                    .bucket(&s3.bucket)
                    .key(&key)
                    .body(content.to_vec())
                    .content_type(content_type)
                    .build_request()?;
                match s3.client.execute(&request).await {
                    Ok(response) => {
                        s3.put_counts.record(response.status_code);
                        if !response.is_success() {
                            return Err(crate::Error::new(format!(
                                "S3 PutObject failed for {key}: status={}",
                                response.status_code
                            )));
                        }
                    }
                    Err(e) => {
                        s3.put_error_count.inc();
                        return Err(e);
                    }
                }
                Ok(())
            }
        }
    }

    /// ファイルを削除する（best-effort、エラーは warning のみ）
    async fn delete_file(&self, filename: &str) {
        match self {
            Self::Filesystem(fs) => {
                let path = fs.output_directory.join(filename);
                if let Err(e) = std::fs::remove_file(&path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!("failed to remove {}: {e}", path.display());
                }
            }
            Self::S3(s3) => {
                let key = s3.object_key(filename);
                match s3
                    .client
                    .client()
                    .delete_object()
                    .bucket(&s3.bucket)
                    .key(&key)
                    .build_request()
                {
                    Ok(request) => match s3.client.execute(&request).await {
                        Ok(response) => {
                            s3.delete_counts.record(response.status_code);
                            if !response.is_success() {
                                tracing::warn!(
                                    "S3 DeleteObject failed for {key}: status={}",
                                    response.status_code
                                );
                            }
                        }
                        Err(e) => {
                            s3.delete_error_count.inc();
                            tracing::warn!("failed to delete S3 object {key}: {}", e.display());
                        }
                    },
                    Err(e) => {
                        tracing::warn!("failed to build DeleteObject for {key}: {e}");
                    }
                }
            }
        }
    }
}

use mpeg2ts::es::{StreamId, StreamType};
use mpeg2ts::pes::PesHeader;
use mpeg2ts::time::{ClockReference, Timestamp};
use mpeg2ts::ts::payload::{Bytes, Pat, Pes, Pmt};
use mpeg2ts::ts::{
    AdaptationField, ContinuityCounter, EsInfo, Pid, ProgramAssociation,
    TransportScramblingControl, TsHeader, TsPacket, TsPacketWriter, TsPayload, VersionNumber,
    WriteTsPacket,
};

/// TS の PID 割り当て
const PMT_PID: u16 = 0x1000;
const VIDEO_PID: u16 = 0x100;
const AUDIO_PID: u16 = 0x101;

/// プレイリストファイル名
const PLAYLIST_FILENAME: &str = "playlist.m3u8";
/// fMP4 の init segment ファイル名
const INIT_SEGMENT_FILENAME: &str = "init.mp4";

/// fMP4 用のタイムスケール（マイクロ秒単位）
const FMP4_TIMESCALE: NonZeroU32 = NonZeroU32::MIN.saturating_add(1_000_000 - 1);

/// HLS セグメントライター。
/// エンコード済みの H.264 + AAC フレームを MPEG-TS または fMP4 セグメントに分割し、
/// M3U8 プレイリストを管理する。
struct HlsWriter {
    storage: HlsStorage,
    segment_duration_target: f64,
    max_retained_segments: usize,
    segment_sequence: u64,
    retained_segments: VecDeque<RetainedSegment>,
    format_state: FormatState,
    /// 現在のセグメントの共通情報
    current_segment_info: Option<CurrentSegmentInfo>,
    /// マニフェストに記載する codec string の解決状態
    codec_resolution: CodecResolutionState,
    /// ABR マスタープレイリスト用の codec string 通知 channel。
    /// 送信は 1 回のみ。送信後および non-ABR 時は None。
    codec_string_sender: Option<tokio::sync::oneshot::Sender<crate::codec_string::CodecString>>,
    stats: HlsWriterStats,
}

/// セグメントの共通情報（フォーマット非依存）
struct CurrentSegmentInfo {
    filename: String,
    start_timestamp: Duration,
    last_timestamp: Duration,
}

/// フォーマット固有の状態
enum FormatState {
    MpegTs(Box<MpegTsState>),
    Fmp4(Box<Fmp4State>),
}

/// MPEG-TS フォーマット固有の状態
struct MpegTsState {
    /// 現在のセグメントのライター（バッファに蓄積し、finalize 時に storage に書き出す）
    current_writer: Option<TsPacketWriter<Vec<u8>>>,
    pat_cc: ContinuityCounter,
    pmt_cc: ContinuityCounter,
    video_cc: ContinuityCounter,
    audio_cc: ContinuityCounter,
    /// 最後に受信したビデオの sample_entry（SPS/PPS 注入用）
    last_video_sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>,
    /// 最後に受信したオーディオの sample_entry（ADTS ヘッダ生成用）
    last_audio_sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>,
}

/// fMP4 フォーマット固有の状態
struct Fmp4State {
    muxer: shiguredo_mp4::mux::Fmp4SegmentMuxer,
    init_segment_written: bool,
    /// 現在のセグメントに蓄積中のサンプルと payload
    current_samples: Vec<shiguredo_mp4::mux::Sample>,
    current_payload: Vec<u8>,
    /// 前回のビデオフレームのタイムスタンプ（duration 計算用）
    last_video_timestamp: Option<Duration>,
    /// 前回のオーディオフレームのタイムスタンプ（duration 計算用）
    last_audio_timestamp: Option<Duration>,
    /// 最後に受信したビデオの sample_entry（セグメント跨ぎで保持）
    last_video_sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>,
    /// 最後に受信したオーディオの sample_entry（セグメント跨ぎで保持）
    last_audio_sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>,
}

#[derive(Debug)]
struct RetainedSegment {
    filename: String,
    duration: f64,
}

impl HlsWriter {
    fn new(
        storage: HlsStorage,
        segment_duration_target: f64,
        max_retained_segments: usize,
        segment_format: HlsSegmentFormat,
        codec_string_sender: Option<tokio::sync::oneshot::Sender<crate::codec_string::CodecString>>,
        stats: HlsWriterStats,
    ) -> crate::Result<Self> {
        let format_state = match segment_format {
            HlsSegmentFormat::MpegTs => FormatState::MpegTs(Box::new(MpegTsState {
                current_writer: None,
                pat_cc: ContinuityCounter::new(),
                pmt_cc: ContinuityCounter::new(),
                video_cc: ContinuityCounter::new(),
                audio_cc: ContinuityCounter::new(),
                last_video_sample_entry: None,
                last_audio_sample_entry: None,
            })),
            HlsSegmentFormat::Fmp4 => {
                let muxer = shiguredo_mp4::mux::Fmp4SegmentMuxer::new().map_err(|e| {
                    crate::Error::new(format!("failed to create fMP4 segment muxer: {e}"))
                })?;
                FormatState::Fmp4(Box::new(Fmp4State {
                    muxer,
                    init_segment_written: false,
                    current_samples: Vec::new(),
                    current_payload: Vec::new(),
                    last_video_timestamp: None,
                    last_audio_timestamp: None,
                    last_video_sample_entry: None,
                    last_audio_sample_entry: None,
                }))
            }
        };

        Ok(Self {
            storage,
            segment_duration_target,
            max_retained_segments,
            segment_sequence: 0,
            retained_segments: VecDeque::new(),
            format_state,
            current_segment_info: None,
            codec_resolution: CodecResolutionState::Pending,
            codec_string_sender,
            stats,
        })
    }

    /// ビデオの codec string が確定した際に状態を遷移させる。
    fn resolve_video_codec(&mut self, video: String) {
        if let Some(cs) = self.codec_resolution.resolve_video(video)
            && let Some(sender) = self.codec_string_sender.take()
        {
            let _ = sender.send(cs);
        }
    }

    /// オーディオの codec string が確定した際に状態を遷移させる。
    fn resolve_audio_codec(&mut self, audio: String) {
        if let Some(cs) = self.codec_resolution.resolve_audio(audio)
            && let Some(sender) = self.codec_string_sender.take()
        {
            let _ = sender.send(cs);
        }
    }

    fn is_fmp4(&self) -> bool {
        matches!(self.format_state, FormatState::Fmp4(_))
    }

    /// セグメントファイルの拡張子
    fn segment_extension(&self) -> &'static str {
        match self.format_state {
            FormatState::MpegTs(_) => "ts",
            FormatState::Fmp4(_) => "m4s",
        }
    }

    /// メインの受信ループ
    async fn run(
        mut self,
        handle: crate::ProcessorHandle,
        input_audio_track_id: crate::TrackId,
        input_video_track_id: crate::TrackId,
    ) -> crate::Result<()> {
        let mut audio_rx = Some(handle.subscribe_track(input_audio_track_id));
        let mut video_rx = Some(handle.subscribe_track(input_video_track_id));
        let (rpc_tx, mut rpc_rx) = tokio::sync::mpsc::unbounded_channel();
        handle.register_rpc_sender(rpc_tx).await.map_err(|e| {
            crate::Error::new(format!("failed to register hls writer RPC sender: {e}"))
        })?;
        let mut rpc_rx_enabled = true;

        handle.notify_ready();

        // 起動直後に上流 video encoder へキーフレーム要求を送る。
        // HLS writer は video track が必須のためガード不要。
        if let Err(e) = crate::encoder::request_upstream_video_keyframe(
            &handle.pipeline_handle(),
            handle.processor_id(),
            "hls_writer_start",
        )
        .await
        {
            tracing::warn!(
                "failed to request keyframe for HLS writer start: {}",
                e.display()
            );
        }

        loop {
            if audio_rx.is_none() && video_rx.is_none() {
                break;
            }

            tokio::select! {
                msg = crate::future::recv_or_pending(&mut audio_rx) => {
                    match msg {
                        crate::Message::Media(crate::MediaFrame::Audio(frame)) => {
                            if let Err(e) = self.handle_audio_frame(&frame).await {
                                tracing::warn!("HLS audio frame error: {}", e.display());
                            }
                        }
                        crate::Message::Eos => {
                            audio_rx = None;
                        }
                        _ => {}
                    }
                }
                msg = crate::future::recv_or_pending(&mut video_rx) => {
                    match msg {
                        crate::Message::Media(crate::MediaFrame::Video(frame)) => {
                            if let Err(e) = self.handle_video_frame(&frame).await {
                                tracing::warn!("HLS video frame error: {}", e.display());
                            }
                        }
                        crate::Message::Eos => {
                            video_rx = None;
                        }
                        _ => {}
                    }
                }
                rpc_message = recv_hls_writer_rpc_message_or_pending(
                    rpc_rx_enabled.then_some(&mut rpc_rx)
                ) => {
                    let Some(rpc_message) = rpc_message else {
                        rpc_rx_enabled = false;
                        continue;
                    };
                    match rpc_message {
                        HlsWriterRpcMessage::Finish { reply_tx } => {
                            // 入力購読を閉じてループ脱出後の finalize / cleanup を実行する。
                            // 上流の残フレーム排出は別途保証していない。
                            audio_rx = None;
                            video_rx = None;
                            let _ = reply_tx.send(());
                        }
                    }
                }
            }
        }

        // EOS 受信後: 現在のセグメントを finalize してから cleanup
        if let Err(e) = self.finalize_current_segment().await {
            tracing::warn!("HLS finalize error on EOS: {}", e.display());
        }
        self.cleanup().await;
        Ok(())
    }

    /// ビデオフレーム処理。
    /// キーフレームかつセグメント尺が target を超えていたらセグメントを切り替える。
    async fn handle_video_frame(&mut self, frame: &crate::VideoFrame) -> crate::Result<()> {
        self.stats.total_input_video_frame_count.inc();
        // キーフレームでセグメント切り替え判定
        if frame.keyframe
            && let Some(ref info) = self.current_segment_info
        {
            let elapsed = frame
                .timestamp
                .saturating_sub(info.start_timestamp)
                .as_secs_f64();
            if elapsed >= self.segment_duration_target {
                self.finalize_current_segment().await?;
            }
        }

        // セグメントが無ければ新規作成（キーフレームで開始）
        if self.current_segment_info.is_none() {
            if !frame.keyframe {
                return Ok(());
            }
            self.start_new_segment(frame.timestamp)?;
        }

        // SampleEntry から正確な codec string を確定する
        if let Some(ref entry) = frame.sample_entry
            && !matches!(
                self.codec_resolution,
                CodecResolutionState::VideoOnly(_) | CodecResolutionState::Resolved(_)
            )
            && let Some(codec_str) =
                crate::codec_string::video_codec_string_from_sample_entry(entry)
        {
            self.resolve_video_codec(codec_str);
        }

        match &mut self.format_state {
            FormatState::MpegTs(state) => {
                // sample_entry が来たら保持する（エンコーダーは初回のみ付与する場合がある）
                if frame.sample_entry.is_some() {
                    state
                        .last_video_sample_entry
                        .clone_from(&frame.sample_entry);
                }
                // length-prefixed NALU → Annex B 変換 + キーフレーム時の SPS/PPS 注入
                let annexb_data = convert_length_prefixed_to_annexb(
                    &frame.data,
                    &state.last_video_sample_entry,
                    frame.keyframe,
                )?;
                let pts = duration_to_timestamp(frame.timestamp)?;
                write_pes_packets_mpegts(
                    state,
                    Pid::new(VIDEO_PID).expect("VIDEO_PID is a valid PID"),
                    StreamId::new_video(StreamId::VIDEO_MIN)
                        .expect("VIDEO_MIN is a valid video stream ID"),
                    &annexb_data,
                    Some(pts),
                    true,
                )?;
            }
            FormatState::Fmp4(state) => {
                // sample_entry が来たら保持する（エンコーダーは初回のみ付与する場合がある）
                if frame.sample_entry.is_some() {
                    state
                        .last_video_sample_entry
                        .clone_from(&frame.sample_entry);
                }
                // 前のビデオサンプルの duration を確定させる
                if let Some(prev_ts) = state.last_video_timestamp {
                    let duration = frame.timestamp.saturating_sub(prev_ts).as_micros() as u32;
                    if let Some(last) = state
                        .current_samples
                        .iter_mut()
                        .rfind(|s| s.track_kind == shiguredo_mp4::TrackKind::Video)
                    {
                        last.duration = duration;
                    }
                }
                let data_offset = state.current_payload.len() as u64;
                state.current_payload.extend_from_slice(&frame.data);
                // フレームの sample_entry が None なら保持済みの値を使う
                let sample_entry = frame
                    .sample_entry
                    .clone()
                    .or_else(|| state.last_video_sample_entry.clone());
                state.current_samples.push(shiguredo_mp4::mux::Sample {
                    track_kind: shiguredo_mp4::TrackKind::Video,
                    sample_entry,
                    keyframe: frame.keyframe,
                    timescale: FMP4_TIMESCALE,
                    duration: 0,
                    composition_time_offset: None,
                    data_offset,
                    data_size: frame.data.len(),
                });
                state.last_video_timestamp = Some(frame.timestamp);
            }
        }

        if let Some(ref mut info) = self.current_segment_info {
            info.last_timestamp = frame.timestamp;
        }

        Ok(())
    }

    /// オーディオフレーム処理
    async fn handle_audio_frame(&mut self, frame: &crate::AudioFrame) -> crate::Result<()> {
        self.stats.total_input_audio_frame_count.inc();
        // 最初の video keyframe より前に audio が流れ始めることがある。
        // その場合でも、初回だけ付与される sample_entry は保持しておかないと、
        // セグメント開始後の AAC フレーム群から codec 情報が失われる。

        // SampleEntry から正確な codec string を確定する
        if let Some(ref entry) = frame.sample_entry
            && !matches!(
                self.codec_resolution,
                CodecResolutionState::AudioOnly(_) | CodecResolutionState::Resolved(_)
            )
            && let Some(codec_str) =
                crate::codec_string::audio_codec_string_from_sample_entry(entry)
        {
            self.resolve_audio_codec(codec_str);
        }

        if frame.sample_entry.is_some() {
            match &mut self.format_state {
                FormatState::MpegTs(state) => {
                    state
                        .last_audio_sample_entry
                        .clone_from(&frame.sample_entry);
                }
                FormatState::Fmp4(state) => {
                    state
                        .last_audio_sample_entry
                        .clone_from(&frame.sample_entry);
                }
            }
        }

        if self.current_segment_info.is_none() {
            return Ok(());
        }

        match &mut self.format_state {
            FormatState::MpegTs(state) => {
                // raw AAC → ADTS 変換
                let adts_data = wrap_raw_aac_in_adts(&frame.data, &state.last_audio_sample_entry)?;
                let pts = duration_to_timestamp(frame.timestamp)?;
                write_pes_packets_mpegts(
                    state,
                    Pid::new(AUDIO_PID).expect("AUDIO_PID is a valid PID"),
                    StreamId::new(StreamId::AUDIO_MIN),
                    &adts_data,
                    Some(pts),
                    false,
                )?;
            }
            FormatState::Fmp4(state) => {
                // 前のオーディオサンプルの duration を確定させる
                if let Some(prev_ts) = state.last_audio_timestamp {
                    let duration = frame.timestamp.saturating_sub(prev_ts).as_micros() as u32;
                    if let Some(last) = state
                        .current_samples
                        .iter_mut()
                        .rfind(|s| s.track_kind == shiguredo_mp4::TrackKind::Audio)
                    {
                        last.duration = duration;
                    }
                }
                let data_offset = state.current_payload.len() as u64;
                state.current_payload.extend_from_slice(&frame.data);
                let sample_entry = frame
                    .sample_entry
                    .clone()
                    .or_else(|| state.last_audio_sample_entry.clone());
                state.current_samples.push(shiguredo_mp4::mux::Sample {
                    track_kind: shiguredo_mp4::TrackKind::Audio,
                    sample_entry,
                    keyframe: true,
                    timescale: FMP4_TIMESCALE,
                    duration: 0,
                    composition_time_offset: None,
                    data_offset,
                    data_size: frame.data.len(),
                });
                state.last_audio_timestamp = Some(frame.timestamp);
            }
        }

        if let Some(ref mut info) = self.current_segment_info {
            info.last_timestamp = frame.timestamp;
        }

        Ok(())
    }

    /// 新しいセグメントを開始する
    fn start_new_segment(&mut self, timestamp: Duration) -> crate::Result<()> {
        let sequence = self.segment_sequence;
        self.segment_sequence += 1;
        let ext = self.segment_extension();
        let filename = format!("segment-{sequence:06}.{ext}");

        match &mut self.format_state {
            FormatState::MpegTs(state) => {
                let mut writer = TsPacketWriter::new(Vec::new());
                write_pat(state, &mut writer)?;
                write_pmt(state, &mut writer)?;
                state.current_writer = Some(writer);
            }
            FormatState::Fmp4(state) => {
                // fMP4: samples と payload をクリアして蓄積開始
                state.current_samples.clear();
                state.current_payload.clear();
            }
        }

        self.current_segment_info = Some(CurrentSegmentInfo {
            filename,
            start_timestamp: timestamp,
            last_timestamp: timestamp,
        });

        Ok(())
    }

    /// 現在のセグメントを完了し、プレイリストを更新する
    async fn finalize_current_segment(&mut self) -> crate::Result<()> {
        let Some(info) = self.current_segment_info.take() else {
            return Ok(());
        };

        match &mut self.format_state {
            FormatState::MpegTs(state) => {
                if let Some(writer) = state.current_writer.take() {
                    let buf = writer.into_stream();
                    self.stats.total_segment_byte_size.add(buf.len() as u64);
                    self.storage.write_segment(&info.filename, &buf).await?;
                }
            }
            FormatState::Fmp4(state) => {
                if state.current_samples.is_empty() {
                    return Ok(());
                }

                // muxer は各トラックの最初の sample に sample_entry があることを要求する。
                // エンコーダーは sample_entry を最初のフレームにしか付けないため、
                // セグメント開始直後のタイミング次第では current_samples 側で欠落し得る。
                // ここで最後に観測した sample_entry から補完しておく。
                fill_missing_sample_entries(
                    &mut state.current_samples,
                    &state.last_video_sample_entry,
                    &state.last_audio_sample_entry,
                );

                // 末尾サンプルの duration を補完する。
                // 各トラックの最後のサンプルは次フレーム未到着のため duration=0 のまま。
                // 同一トラックの直前サンプルの duration を流用して埋める。
                fixup_last_sample_duration(&mut state.current_samples);

                // mdat payload をトラックごとに連続配置し、data_offset を再計算する。
                // Fmp4SegmentMuxer は同一トラックの sample data が mdat 内で
                // 連続していることを要求する。
                let reordered_payload =
                    reorder_payload_by_track(&mut state.current_samples, &state.current_payload);

                // moof + mdat ヘッダを生成
                let metadata = state
                    .muxer
                    .create_media_segment_metadata(&state.current_samples)
                    .map_err(|e| {
                        crate::Error::new(format!("failed to create fMP4 segment metadata: {e}"))
                    })?;

                // init segment がまだ書かれていなければ書き出す
                if !state.init_segment_written {
                    let init_bytes = state.muxer.init_segment_bytes().map_err(|e| {
                        crate::Error::new(format!("failed to create fMP4 init segment: {e}"))
                    })?;
                    self.storage
                        .write_segment(INIT_SEGMENT_FILENAME, &init_bytes)
                        .await?;
                    state.init_segment_written = true;
                }

                // セグメントファイルを書き出す（metadata + payload）
                let mut segment_data = Vec::with_capacity(metadata.len() + reordered_payload.len());
                segment_data.extend_from_slice(&metadata);
                segment_data.extend_from_slice(&reordered_payload);
                self.stats
                    .total_segment_byte_size
                    .add(segment_data.len() as u64);
                self.storage
                    .write_segment(&info.filename, &segment_data)
                    .await?;

                state.current_samples.clear();
                state.current_payload.clear();
            }
        }

        let duration = info
            .last_timestamp
            .saturating_sub(info.start_timestamp)
            .as_secs_f64();
        let duration = duration.max(0.001);

        self.stats.total_segment_count.inc();

        self.retained_segments.push_back(RetainedSegment {
            filename: info.filename,
            duration,
        });

        // 保持数超過分の古いセグメントを先に削除してから playlist を書き出す。
        // この順序にしないと、playlist が削除済みセグメントを参照してしまう。
        while self.retained_segments.len() > self.max_retained_segments {
            if let Some(old) = self.retained_segments.pop_front() {
                self.storage.delete_file(&old.filename).await;
                self.stats.total_deleted_segment_count.inc();
            }
        }

        self.stats
            .current_retained_segment_count
            .set(self.retained_segments.len() as i64);

        self.write_playlist().await?;

        Ok(())
    }

    /// M3U8 プレイリストを書き出す。
    async fn write_playlist(&self) -> crate::Result<()> {
        if self.retained_segments.is_empty() {
            return Ok(());
        }

        let media_sequence = self.segment_sequence as usize - self.retained_segments.len();

        let max_duration = self
            .retained_segments
            .iter()
            .map(|s| s.duration)
            .fold(0.0f64, f64::max);
        let target_duration = max_duration.ceil() as u64;
        let target_duration = target_duration.max(1);

        let mut content = String::new();
        content.push_str("#EXTM3U\n");

        if self.is_fmp4() {
            // fMP4 は HLS v7 で規定
            content.push_str("#EXT-X-VERSION:7\n");
        } else {
            content.push_str("#EXT-X-VERSION:3\n");
        }

        content.push_str(&format!("#EXT-X-TARGETDURATION:{target_duration}\n"));
        content.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{media_sequence}\n"));

        // fMP4 の場合は init segment への参照を追加
        if self.is_fmp4() {
            content.push_str(&format!("#EXT-X-MAP:URI=\"{INIT_SEGMENT_FILENAME}\"\n"));
        }

        for seg in &self.retained_segments {
            content.push_str(&format!("#EXTINF:{:.3},\n", seg.duration));
            content.push_str(&seg.filename);
            content.push('\n');
        }

        self.storage
            .write_playlist(PLAYLIST_FILENAME, content.as_bytes())
            .await?;

        Ok(())
    }

    /// 停止時に全生成ファイルを削除する
    async fn cleanup(&self) {
        self.storage.delete_file(PLAYLIST_FILENAME).await;

        // filesystem の場合のみ一時ファイルも削除する
        if let HlsStorage::Filesystem(fs) = &self.storage {
            let tmp_path = fs.output_directory.join(".playlist.m3u8.tmp");
            let _ = std::fs::remove_file(&tmp_path);
        }

        // fMP4 の場合は init segment も削除
        if self.is_fmp4() {
            self.storage.delete_file(INIT_SEGMENT_FILENAME).await;
        }

        for seg in &self.retained_segments {
            self.storage.delete_file(&seg.filename).await;
        }
    }
}

async fn recv_hls_writer_rpc_message_or_pending(
    rpc_rx: Option<&mut tokio::sync::mpsc::UnboundedReceiver<HlsWriterRpcMessage>>,
) -> Option<HlsWriterRpcMessage> {
    if let Some(rpc_rx) = rpc_rx {
        rpc_rx.recv().await
    } else {
        std::future::pending().await
    }
}

// --- MPEG-TS 固有の関数群 ---

fn write_pat<W: Write>(
    state: &mut MpegTsState,
    writer: &mut TsPacketWriter<W>,
) -> crate::Result<()> {
    let cc = state.pat_cc;
    state.pat_cc.increment();
    let packet = TsPacket {
        header: TsHeader {
            transport_error_indicator: false,
            transport_priority: false,
            pid: Pid::from(0u8),
            transport_scrambling_control: TransportScramblingControl::NotScrambled,
            continuity_counter: cc,
        },
        adaptation_field: None,
        payload: Some(TsPayload::Pat(Pat {
            transport_stream_id: 1,
            version_number: VersionNumber::new(),
            table: vec![ProgramAssociation {
                program_num: 1,
                program_map_pid: Pid::new(PMT_PID).expect("PMT_PID is a valid PID"),
            }],
        })),
    };
    writer
        .write_ts_packet(&packet)
        .map_err(|e| crate::Error::new(format!("failed to write PAT: {e}")))?;
    Ok(())
}

fn write_pmt<W: Write>(
    state: &mut MpegTsState,
    writer: &mut TsPacketWriter<W>,
) -> crate::Result<()> {
    let cc = state.pmt_cc;
    state.pmt_cc.increment();
    let packet = TsPacket {
        header: TsHeader {
            transport_error_indicator: false,
            transport_priority: false,
            pid: Pid::new(PMT_PID).expect("PMT_PID is a valid PID"),
            transport_scrambling_control: TransportScramblingControl::NotScrambled,
            continuity_counter: cc,
        },
        adaptation_field: None,
        payload: Some(TsPayload::Pmt(Pmt {
            program_num: 1,
            pcr_pid: Some(Pid::new(VIDEO_PID).expect("VIDEO_PID is a valid PID")),
            version_number: VersionNumber::new(),
            program_info: vec![],
            es_info: vec![
                EsInfo {
                    stream_type: StreamType::H264,
                    elementary_pid: Pid::new(VIDEO_PID).expect("VIDEO_PID is a valid PID"),
                    descriptors: vec![],
                },
                EsInfo {
                    stream_type: StreamType::AdtsAac,
                    elementary_pid: Pid::new(AUDIO_PID).expect("AUDIO_PID is a valid PID"),
                    descriptors: vec![],
                },
            ],
        })),
    };
    writer
        .write_ts_packet(&packet)
        .map_err(|e| crate::Error::new(format!("failed to write PMT: {e}")))?;
    Ok(())
}

/// PES データを TS パケットに分割して書き出す。
fn write_pes_packets_mpegts(
    state: &mut MpegTsState,
    pid: Pid,
    stream_id: StreamId,
    data: &[u8],
    pts: Option<Timestamp>,
    is_video: bool,
) -> crate::Result<()> {
    let writer = state
        .current_writer
        .as_mut()
        .ok_or_else(|| crate::Error::new("no active MPEG-TS segment".to_owned()))?;

    let cc = if is_video {
        &mut state.video_cc
    } else {
        &mut state.audio_cc
    };

    let pes_header = PesHeader {
        stream_id,
        priority: false,
        data_alignment_indicator: true,
        copyright: false,
        original_or_copy: false,
        pts,
        dts: None,
        escr: None,
    };

    let optional_header_len: usize = 3 + pts.map_or(0, |_| 5) + pes_header.dts.map_or(0, |_| 5);
    let pes_header_size = 3 + 1 + 2 + optional_header_len;
    let total_pes_size = pes_header_size + data.len();

    let pes_packet_len = if total_pes_size - 6 > u16::MAX as usize {
        0
    } else {
        (total_pes_size - 6) as u16
    };

    let max_first_payload = Bytes::MAX_SIZE - pes_header_size;
    let first_data_len = data.len().min(max_first_payload);

    let first_data = Bytes::new(&data[..first_data_len])
        .map_err(|e| crate::Error::new(format!("failed to create PES start data: {e}")))?;

    let current_cc = *cc;
    cc.increment();

    let adaptation_field = if is_video {
        pts.map(|pts_val| AdaptationField {
            discontinuity_indicator: false,
            random_access_indicator: false,
            es_priority_indicator: false,
            pcr: Some(ClockReference::from(pts_val)),
            opcr: None,
            splice_countdown: None,
            transport_private_data: Vec::new(),
            extension: None,
        })
    } else {
        None
    };

    let start_packet = TsPacket {
        header: TsHeader {
            transport_error_indicator: false,
            transport_priority: false,
            pid,
            transport_scrambling_control: TransportScramblingControl::NotScrambled,
            continuity_counter: current_cc,
        },
        adaptation_field,
        payload: Some(TsPayload::PesStart(Pes {
            header: pes_header,
            pes_packet_len,
            data: first_data,
        })),
    };

    writer
        .write_ts_packet(&start_packet)
        .map_err(|e| crate::Error::new(format!("failed to write PES start packet: {e}")))?;

    let mut offset = first_data_len;
    while offset < data.len() {
        let remaining = data.len() - offset;
        let chunk_len = remaining.min(Bytes::MAX_SIZE);
        let chunk = Bytes::new(&data[offset..offset + chunk_len]).map_err(|e| {
            crate::Error::new(format!("failed to create PES continuation data: {e}"))
        })?;

        let current_cc = *cc;
        cc.increment();

        let cont_packet = TsPacket {
            header: TsHeader {
                transport_error_indicator: false,
                transport_priority: false,
                pid,
                transport_scrambling_control: TransportScramblingControl::NotScrambled,
                continuity_counter: current_cc,
            },
            adaptation_field: None,
            payload: Some(TsPayload::PesContinuation(chunk)),
        };

        writer.write_ts_packet(&cont_packet).map_err(|e| {
            crate::Error::new(format!("failed to write PES continuation packet: {e}"))
        })?;
        offset += chunk_len;
    }

    Ok(())
}

/// fMP4 セグメントの末尾サンプルの duration を補完する。
/// 各トラックの最後のサンプルが duration=0 の場合、同一トラックの直前サンプルの duration を流用する。
/// mdat payload をトラックごとに連続配置し、samples の data_offset を再計算する。
/// Fmp4SegmentMuxer は同一トラックの sample data が mdat 内で連続していることを要求する。
/// 到着順（audio/video 混在）の payload を、video → audio の順に並べ替えた新しい payload を返す。
fn reorder_payload_by_track(
    samples: &mut [shiguredo_mp4::mux::Sample],
    original_payload: &[u8],
) -> Vec<u8> {
    let mut reordered = Vec::with_capacity(original_payload.len());

    // Video のデータを先に配置
    for sample in samples
        .iter_mut()
        .filter(|s| s.track_kind == shiguredo_mp4::TrackKind::Video)
    {
        let new_offset = reordered.len() as u64;
        let start = sample.data_offset as usize;
        let end = start + sample.data_size;
        reordered.extend_from_slice(&original_payload[start..end]);
        sample.data_offset = new_offset;
    }

    // Audio のデータを次に配置
    for sample in samples
        .iter_mut()
        .filter(|s| s.track_kind == shiguredo_mp4::TrackKind::Audio)
    {
        let new_offset = reordered.len() as u64;
        let start = sample.data_offset as usize;
        let end = start + sample.data_size;
        reordered.extend_from_slice(&original_payload[start..end]);
        sample.data_offset = new_offset;
    }

    reordered
}

/// fMP4 muxer に渡す前に、欠落している sample_entry を最後に観測した値で補完する。
fn fill_missing_sample_entries(
    samples: &mut [shiguredo_mp4::mux::Sample],
    last_video_sample_entry: &Option<shiguredo_mp4::boxes::SampleEntry>,
    last_audio_sample_entry: &Option<shiguredo_mp4::boxes::SampleEntry>,
) {
    for sample in samples.iter_mut() {
        if sample.sample_entry.is_some() {
            continue;
        }
        match sample.track_kind {
            shiguredo_mp4::TrackKind::Video => {
                sample.sample_entry = last_video_sample_entry.clone();
            }
            shiguredo_mp4::TrackKind::Audio => {
                sample.sample_entry = last_audio_sample_entry.clone();
            }
        }
    }
}

fn fixup_last_sample_duration(samples: &mut [shiguredo_mp4::mux::Sample]) {
    // ビデオの末尾を補完
    fixup_last_sample_duration_for_track(samples, shiguredo_mp4::TrackKind::Video);
    // オーディオの末尾を補完
    fixup_last_sample_duration_for_track(samples, shiguredo_mp4::TrackKind::Audio);
}

fn fixup_last_sample_duration_for_track(
    samples: &mut [shiguredo_mp4::mux::Sample],
    track_kind: shiguredo_mp4::TrackKind,
) {
    let track_samples: Vec<usize> = samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.track_kind == track_kind)
        .map(|(i, _)| i)
        .collect();

    if track_samples.len() < 2 {
        // サンプルが 1 つ以下なら補完する情報がないので何もしない
        // （duration=0 は fMP4 上は許容される）
        return;
    }

    let last_idx = track_samples[track_samples.len() - 1];
    if samples[last_idx].duration == 0 {
        let prev_idx = track_samples[track_samples.len() - 2];
        samples[last_idx].duration = samples[prev_idx].duration;
    }
}

/// Duration を mpeg2ts の Timestamp (90kHz) に変換する。
/// 浮動小数点を避けて整数演算で計算する。
fn duration_to_timestamp(d: Duration) -> crate::Result<Timestamp> {
    let ticks = d.as_secs() * Timestamp::RESOLUTION
        + u64::from(d.subsec_nanos()) * Timestamp::RESOLUTION / 1_000_000_000;
    let ticks = ticks % (Timestamp::MAX + 1);
    Timestamp::new(ticks).map_err(|e| crate::Error::new(format!("invalid timestamp: {e}")))
}

/// MP4 形式の length-prefixed NALU を Annex B 形式に変換する。
/// MPEG-TS では Annex B（start code prefix 付き）が必要。
/// キーフレームの場合は sample_entry から SPS/PPS を抽出して先頭に注入する。
fn convert_length_prefixed_to_annexb(
    data: &[u8],
    sample_entry: &Option<shiguredo_mp4::boxes::SampleEntry>,
    keyframe: bool,
) -> crate::Result<Vec<u8>> {
    let length_size = match sample_entry {
        Some(shiguredo_mp4::boxes::SampleEntry::Avc1(avc1)) => {
            avc1.avcc_box.length_size_minus_one.get() as usize + 1
        }
        _ => 4, // デフォルトは 4 バイト
    };

    let start_code: &[u8] = &[0x00, 0x00, 0x00, 0x01];
    let mut result = Vec::with_capacity(data.len());

    // キーフレームの場合は SPS/PPS を先頭に注入する。
    // エンコーダーは SPS/PPS を sample_entry にのみ格納し、フレーム本体には含めない場合がある。
    // MPEG-TS ではセグメント先頭のキーフレームに SPS/PPS が必要。
    if keyframe && let Some(shiguredo_mp4::boxes::SampleEntry::Avc1(avc1)) = sample_entry {
        for sps in &avc1.avcc_box.sps_list {
            result.extend_from_slice(start_code);
            result.extend_from_slice(sps);
        }
        for pps in &avc1.avcc_box.pps_list {
            result.extend_from_slice(start_code);
            result.extend_from_slice(pps);
        }
    }

    let mut offset = 0;
    while offset + length_size <= data.len() {
        let nalu_len = match length_size {
            1 => data[offset] as usize,
            2 => u16::from_be_bytes([data[offset], data[offset + 1]]) as usize,
            3 => {
                ((data[offset] as usize) << 16)
                    | ((data[offset + 1] as usize) << 8)
                    | (data[offset + 2] as usize)
            }
            4 => u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize,
            _ => {
                return Err(crate::Error::new(format!(
                    "unsupported NALU length size: {length_size}"
                )));
            }
        };
        offset += length_size;

        if offset + nalu_len > data.len() {
            return Err(crate::Error::new(format!(
                "NALU length {nalu_len} exceeds remaining data {} at offset {}",
                data.len() - offset,
                offset
            )));
        }

        result.extend_from_slice(start_code);
        result.extend_from_slice(&data[offset..offset + nalu_len]);
        offset += nalu_len;
    }

    Ok(result)
}

/// Raw AAC フレームに ADTS ヘッダを付与する。
/// MPEG-TS では ADTS 付きの AAC が必要。
/// SampleEntry から AudioSpecificConfig を取得し、ADTS ヘッダを構築する。
fn wrap_raw_aac_in_adts(
    data: &[u8],
    sample_entry: &Option<shiguredo_mp4::boxes::SampleEntry>,
) -> crate::Result<Vec<u8>> {
    // SampleEntry から audio_object_type, sampling_frequency_index, channel_configuration を取得
    let (audio_object_type, sampling_frequency_index, channel_configuration) =
        extract_aac_config(sample_entry)?;

    let frame_length = (data.len() + 7) as u16; // ADTS ヘッダ 7 バイト + raw AAC データ

    // ADTS ヘッダ構築 (7 バイト、CRC なし)
    let mut header = [0u8; 7];
    // syncword (12 bits): 0xFFF
    header[0] = 0xFF;
    // syncword (4) + ID (1, MPEG-4) + layer (2, 00) + protection_absent (1, no CRC)
    header[1] = 0xF1;
    // profile (2, audio_object_type - 1) + sampling_frequency_index (4) + private_bit (1) + channel_configuration_high (1)
    let profile = audio_object_type.saturating_sub(1);
    header[2] =
        (profile << 6) | (sampling_frequency_index << 2) | ((channel_configuration >> 2) & 0x01);
    // channel_configuration_low (2) + original_copy (1) + home (1) + copyright_id_bit (1) + copyright_id_start (1) + frame_length_high (2)
    header[3] = ((channel_configuration & 0x03) << 6) | ((frame_length >> 11) as u8 & 0x03);
    // frame_length_mid (8)
    header[4] = ((frame_length >> 3) & 0xFF) as u8;
    // frame_length_low (3) + buffer_fullness_high (5)
    header[5] = ((frame_length & 0x07) as u8) << 5 | 0x1F; // buffer fullness = 0x7FF (VBR)
    // buffer_fullness_low (6) + number_of_raw_data_blocks (2, 0 = 1 block)
    header[6] = 0xFC; // 0x7FF の下位 6 bit = 0x3F << 2 = 0xFC

    let mut result = Vec::with_capacity(7 + data.len());
    result.extend_from_slice(&header);
    result.extend_from_slice(data);
    Ok(result)
}

/// SampleEntry から AAC の設定情報を抽出する
fn extract_aac_config(
    sample_entry: &Option<shiguredo_mp4::boxes::SampleEntry>,
) -> crate::Result<(u8, u8, u8)> {
    let Some(shiguredo_mp4::boxes::SampleEntry::Mp4a(mp4a)) = sample_entry else {
        // SampleEntry が無い場合のフォールバック: AAC-LC, 48kHz, stereo
        return Ok((2, 3, 2));
    };

    let Some(ref dec_specific_info) = mp4a.esds_box.es.dec_config_descr.dec_specific_info else {
        return Ok((2, 3, 2));
    };

    let asc = &dec_specific_info.payload;
    if asc.len() < 2 {
        return Ok((2, 3, 2));
    }

    let audio_object_type = (asc[0] >> 3) & 0x1F;
    let sampling_frequency_index = ((asc[0] & 0x07) << 1) | (asc[1] >> 7);
    let channel_configuration = (asc[1] >> 3) & 0x0F;

    Ok((
        audio_object_type,
        sampling_frequency_index,
        channel_configuration,
    ))
}

/// HLS writer プロセッサの設定。
///
/// 現在は映像と音声の両方を必須としている。
/// 将来的に映像のみ・音声のみに対応する場合は、以下を修正すること:
/// - このフィールドを `Option<TrackId>` に戻す
/// - `HlsWriter::run()` の subscribe 部分を条件分岐にする
/// - `write_pmt()` の ES info を実際の入力トラックに応じて構築する
/// - fMP4 の `reorder_payload_by_track()` で存在しないトラックをスキップする
pub enum HlsStorageConfig {
    /// ローカルファイルシステム
    Filesystem { output_directory: PathBuf },
    /// S3 互換オブジェクトストレージ
    S3 {
        client: crate::s3::S3HttpClient,
        bucket: String,
        prefix: String,
    },
}

pub struct HlsWriterConfig {
    pub storage: HlsStorageConfig,
    pub input_audio_track_id: crate::TrackId,
    pub input_video_track_id: crate::TrackId,
    pub segment_duration: f64,
    pub max_retained_segments: usize,
    pub segment_format: HlsSegmentFormat,
    /// ABR マスタープレイリスト用の codec string 通知 channel。
    /// ABR 時のみ coordinator が受信側を持ち、全 variant の codec 確定後にマスタープレイリストを書き出す。
    /// non-ABR では不要（None を渡すこと）。
    pub codec_string_sender: Option<tokio::sync::oneshot::Sender<crate::codec_string::CodecString>>,
}

/// HLS writer プロセッサを作成する
pub async fn create_processor(
    handle: &crate::MediaPipelineHandle,
    config: HlsWriterConfig,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    let processor_id = processor_id.unwrap_or_else(|| crate::ProcessorId::new("hlsWriter"));
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("hls_writer"),
            move |h| async move {
                let mut stats = h.stats();
                let writer_stats = HlsWriterStats::new(&mut stats);
                let storage = match config.storage {
                    HlsStorageConfig::Filesystem { output_directory } => {
                        HlsStorage::Filesystem(FilesystemStorage { output_directory })
                    }
                    HlsStorageConfig::S3 {
                        client,
                        bucket,
                        prefix,
                    } => HlsStorage::S3(Box::new(S3Storage {
                        client,
                        bucket,
                        prefix,
                        put_counts: S3StatusCounters::new(&stats, "total_s3_put_count"),
                        delete_counts: S3StatusCounters::new(&stats, "total_s3_delete_count"),
                        put_error_count: stats.clone().counter("total_s3_put_error_count"),
                        delete_error_count: stats.clone().counter("total_s3_delete_error_count"),
                    })),
                };
                let writer = HlsWriter::new(
                    storage,
                    config.segment_duration,
                    config.max_retained_segments,
                    config.segment_format,
                    config.codec_string_sender,
                    writer_stats,
                )?;
                writer
                    .run(h, config.input_audio_track_id, config.input_video_track_id)
                    .await
            },
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}

/// マスタープレイリストのバリアント情報
pub struct MasterPlaylistVariant {
    /// バリアントの合計帯域幅（ビデオ + オーディオ、bps）
    pub bandwidth: u64,
    /// ビデオ幅
    pub width: u32,
    /// ビデオ高さ
    pub height: u32,
    /// バリアントのメディアプレイリスト URI（例: "variant_0/playlist.m3u8"）
    pub playlist_uri: String,
}

/// ABR 用のマスタープレイリスト（Multivariant Playlist）を書き出す。
/// 一時ファイルに書いてから rename してアトミックに更新する。
/// マスタープレイリストの内容を生成する
pub fn build_master_playlist_content(
    variants: &[MasterPlaylistVariant],
    codecs: &crate::codec_string::CodecString,
) -> String {
    let playlist = shiguredo_m3u8::multivariant::MultivariantPlaylist {
        version: None,
        independent_segments: true,
        start: None,
        variable_definitions: Vec::new(),
        content_steering: None,
        variant_streams: variants
            .iter()
            .map(|v| shiguredo_m3u8::multivariant::VariantStream {
                bandwidth: v.bandwidth,
                average_bandwidth: None,
                codecs: Some(codecs.as_combined()),
                supplemental_codecs: None,
                resolution: Some(shiguredo_m3u8::multivariant::Resolution {
                    width: v.width,
                    height: v.height,
                }),
                frame_rate: None,
                hdcp_level: None,
                allowed_cpc: None,
                video_range: None,
                audio: None,
                video: None,
                subtitles: None,
                closed_captions: None,
                name: None,
                stable_variant_id: None,
                pathway_id: None,
                uri: v.playlist_uri.clone(),
            })
            .collect(),
        renditions: Vec::new(),
        i_frame_streams: Vec::new(),
        session_data: Vec::new(),
        session_keys: Vec::new(),
    };

    shiguredo_m3u8::write_multivariant_playlist(&playlist)
}

/// ABR 用のマスタープレイリストをファイルシステムに書き出す。
/// 一時ファイルに書いてから rename してアトミックに更新する。
pub fn write_master_playlist(
    output_directory: &Path,
    variants: &[MasterPlaylistVariant],
    codecs: &crate::codec_string::CodecString,
) -> crate::Result<()> {
    let content = build_master_playlist_content(variants, codecs);

    let playlist_path = output_directory.join(PLAYLIST_FILENAME);
    let tmp_path = output_directory.join(".playlist.m3u8.tmp");

    std::fs::write(&tmp_path, content.as_bytes()).map_err(|e| {
        crate::Error::new(format!(
            "failed to write temporary master playlist {}: {e}",
            tmp_path.display()
        ))
    })?;

    std::fs::rename(&tmp_path, &playlist_path).map_err(|e| {
        crate::Error::new(format!(
            "failed to rename master playlist {} -> {}: {e}",
            tmp_path.display(),
            playlist_path.display()
        ))
    })?;

    Ok(())
}
