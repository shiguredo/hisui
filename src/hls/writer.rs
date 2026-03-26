use std::collections::VecDeque;
use std::io::{BufWriter, Write};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use crate::obsws::input_registry::HlsSegmentFormat;

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
    output_directory: PathBuf,
    segment_duration_target: f64,
    max_retained_segments: usize,
    segment_sequence: u64,
    retained_segments: VecDeque<RetainedSegment>,
    format_state: FormatState,
    /// 現在のセグメントの共通情報
    current_segment_info: Option<CurrentSegmentInfo>,
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
    /// 現在のセグメントのライター
    current_writer: Option<TsPacketWriter<BufWriter<std::fs::File>>>,
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
}

#[derive(Debug)]
struct RetainedSegment {
    filename: String,
    duration: f64,
}

impl HlsWriter {
    fn new(
        output_directory: PathBuf,
        segment_duration_target: f64,
        max_retained_segments: usize,
        segment_format: HlsSegmentFormat,
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
                }))
            }
        };

        Ok(Self {
            output_directory,
            segment_duration_target,
            max_retained_segments,
            segment_sequence: 0,
            retained_segments: VecDeque::new(),
            format_state,
            current_segment_info: None,
        })
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
        input_audio_track_id: Option<crate::TrackId>,
        input_video_track_id: Option<crate::TrackId>,
    ) -> crate::Result<()> {
        let mut audio_rx = input_audio_track_id.map(|id| handle.subscribe_track(id));
        let mut video_rx = input_video_track_id.map(|id| handle.subscribe_track(id));
        handle.notify_ready();

        loop {
            if audio_rx.is_none() && video_rx.is_none() {
                break;
            }

            tokio::select! {
                msg = crate::future::recv_or_pending(&mut audio_rx) => {
                    match msg {
                        crate::Message::Media(crate::MediaFrame::Audio(frame)) => {
                            if let Err(e) = self.handle_audio_frame(&frame) {
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
                            if let Err(e) = self.handle_video_frame(&frame) {
                                tracing::warn!("HLS video frame error: {}", e.display());
                            }
                        }
                        crate::Message::Eos => {
                            video_rx = None;
                        }
                        _ => {}
                    }
                }
            }
        }

        // EOS 受信後: 現在のセグメントを finalize してから cleanup
        if let Err(e) = self.finalize_current_segment() {
            tracing::warn!("HLS finalize error on EOS: {}", e.display());
        }
        self.cleanup();
        Ok(())
    }

    /// ビデオフレーム処理。
    /// キーフレームかつセグメント尺が target を超えていたらセグメントを切り替える。
    fn handle_video_frame(&mut self, frame: &crate::VideoFrame) -> crate::Result<()> {
        // キーフレームでセグメント切り替え判定
        if frame.keyframe
            && let Some(ref info) = self.current_segment_info
        {
            let elapsed = frame
                .timestamp
                .saturating_sub(info.start_timestamp)
                .as_secs_f64();
            if elapsed >= self.segment_duration_target {
                self.finalize_current_segment()?;
            }
        }

        // セグメントが無ければ新規作成（キーフレームで開始）
        if self.current_segment_info.is_none() {
            if !frame.keyframe {
                return Ok(());
            }
            self.start_new_segment(frame.timestamp)?;
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
                    Pid::new(VIDEO_PID).unwrap(),
                    StreamId::new_video(StreamId::VIDEO_MIN).unwrap(),
                    &annexb_data,
                    Some(pts),
                    true,
                )?;
            }
            FormatState::Fmp4(state) => {
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
                // duration は次のフレーム到着時に確定するので、仮に 0 を入れる
                state.current_samples.push(shiguredo_mp4::mux::Sample {
                    track_kind: shiguredo_mp4::TrackKind::Video,
                    sample_entry: frame.sample_entry.clone(),
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
    fn handle_audio_frame(&mut self, frame: &crate::AudioFrame) -> crate::Result<()> {
        if self.current_segment_info.is_none() {
            return Ok(());
        }

        match &mut self.format_state {
            FormatState::MpegTs(state) => {
                // sample_entry が来たら保持する
                if frame.sample_entry.is_some() {
                    state
                        .last_audio_sample_entry
                        .clone_from(&frame.sample_entry);
                }
                // raw AAC → ADTS 変換
                let adts_data = wrap_raw_aac_in_adts(&frame.data, &state.last_audio_sample_entry)?;
                let pts = duration_to_timestamp(frame.timestamp)?;
                write_pes_packets_mpegts(
                    state,
                    Pid::new(AUDIO_PID).unwrap(),
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
                state.current_samples.push(shiguredo_mp4::mux::Sample {
                    track_kind: shiguredo_mp4::TrackKind::Audio,
                    sample_entry: frame.sample_entry.clone(),
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
                let path = self.output_directory.join(&filename);
                let file = std::fs::File::create(&path).map_err(|e| {
                    crate::Error::new(format!(
                        "failed to create segment file {}: {e}",
                        path.display()
                    ))
                })?;
                let buf_writer = BufWriter::new(file);
                let mut writer = TsPacketWriter::new(buf_writer);
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
    fn finalize_current_segment(&mut self) -> crate::Result<()> {
        let Some(info) = self.current_segment_info.take() else {
            return Ok(());
        };

        match &mut self.format_state {
            FormatState::MpegTs(state) => {
                if let Some(writer) = state.current_writer.take() {
                    let mut inner = writer.into_stream();
                    inner.flush().map_err(|e| {
                        crate::Error::new(format!("failed to flush segment file: {e}"))
                    })?;
                }
            }
            FormatState::Fmp4(state) => {
                if state.current_samples.is_empty() {
                    return Ok(());
                }

                // 末尾サンプルの duration を補完する。
                // 各トラックの最後のサンプルは次フレーム未到着のため duration=0 のまま。
                // 同一トラックの直前サンプルの duration を流用して埋める。
                fixup_last_sample_duration(&mut state.current_samples);

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
                    let init_path = self.output_directory.join(INIT_SEGMENT_FILENAME);
                    std::fs::write(&init_path, &init_bytes).map_err(|e| {
                        crate::Error::new(format!(
                            "failed to write init segment {}: {e}",
                            init_path.display()
                        ))
                    })?;
                    state.init_segment_written = true;
                }

                // セグメントファイルを書き出す（metadata + payload）
                let path = self.output_directory.join(&info.filename);
                let mut file = BufWriter::new(std::fs::File::create(&path).map_err(|e| {
                    crate::Error::new(format!(
                        "failed to create segment file {}: {e}",
                        path.display()
                    ))
                })?);
                file.write_all(&metadata).map_err(|e| {
                    crate::Error::new(format!("failed to write fMP4 metadata: {e}"))
                })?;
                file.write_all(&state.current_payload)
                    .map_err(|e| crate::Error::new(format!("failed to write fMP4 payload: {e}")))?;
                file.flush()
                    .map_err(|e| crate::Error::new(format!("failed to flush fMP4 segment: {e}")))?;

                state.current_samples.clear();
                state.current_payload.clear();
            }
        }

        let duration = info
            .last_timestamp
            .saturating_sub(info.start_timestamp)
            .as_secs_f64();
        let duration = duration.max(0.001);

        self.retained_segments.push_back(RetainedSegment {
            filename: info.filename,
            duration,
        });

        // 保持数超過分の古いセグメントを先に削除してから playlist を書き出す。
        // この順序にしないと、playlist が削除済みセグメントを参照してしまう。
        while self.retained_segments.len() > self.max_retained_segments {
            if let Some(old) = self.retained_segments.pop_front() {
                let path = self.output_directory.join(&old.filename);
                if let Err(e) = std::fs::remove_file(&path) {
                    tracing::warn!("failed to remove old segment {}: {e}", path.display());
                }
            }
        }

        self.write_playlist()?;

        Ok(())
    }

    /// M3U8 プレイリストを書き出す。
    /// 一時ファイルに書いてから rename してアトミックに更新する。
    fn write_playlist(&self) -> crate::Result<()> {
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

        let playlist_path = self.output_directory.join(PLAYLIST_FILENAME);
        let tmp_path = self.output_directory.join(".playlist.m3u8.tmp");

        std::fs::write(&tmp_path, content.as_bytes()).map_err(|e| {
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

    /// 停止時に全生成ファイルを削除する
    fn cleanup(&self) {
        let playlist_path = self.output_directory.join(PLAYLIST_FILENAME);
        if let Err(e) = std::fs::remove_file(&playlist_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("failed to remove playlist {}: {e}", playlist_path.display());
        }
        let tmp_path = self.output_directory.join(".playlist.m3u8.tmp");
        let _ = std::fs::remove_file(&tmp_path);

        // fMP4 の場合は init segment も削除
        if self.is_fmp4() {
            let init_path = self.output_directory.join(INIT_SEGMENT_FILENAME);
            if let Err(e) = std::fs::remove_file(&init_path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!("failed to remove init segment {}: {e}", init_path.display());
            }
        }

        for seg in &self.retained_segments {
            let path = self.output_directory.join(&seg.filename);
            if let Err(e) = std::fs::remove_file(&path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!("failed to remove segment {}: {e}", path.display());
            }
        }
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
                program_map_pid: Pid::new(PMT_PID).unwrap(),
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
            pid: Pid::new(PMT_PID).unwrap(),
            transport_scrambling_control: TransportScramblingControl::NotScrambled,
            continuity_counter: cc,
        },
        adaptation_field: None,
        payload: Some(TsPayload::Pmt(Pmt {
            program_num: 1,
            pcr_pid: Some(Pid::new(VIDEO_PID).unwrap()),
            version_number: VersionNumber::new(),
            program_info: vec![],
            es_info: vec![
                EsInfo {
                    stream_type: StreamType::H264,
                    elementary_pid: Pid::new(VIDEO_PID).unwrap(),
                    descriptors: vec![],
                },
                EsInfo {
                    stream_type: StreamType::AdtsAac,
                    elementary_pid: Pid::new(AUDIO_PID).unwrap(),
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

/// Duration を mpeg2ts の Timestamp (90kHz) に変換する
fn duration_to_timestamp(d: Duration) -> crate::Result<Timestamp> {
    let ticks = (d.as_secs_f64() * Timestamp::RESOLUTION as f64) as u64;
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

/// HLS writer プロセッサの設定
pub struct HlsWriterConfig {
    pub output_directory: PathBuf,
    pub input_audio_track_id: Option<crate::TrackId>,
    pub input_video_track_id: Option<crate::TrackId>,
    pub segment_duration: f64,
    pub max_retained_segments: usize,
    pub segment_format: HlsSegmentFormat,
}

/// HLS writer プロセッサを作成する
pub async fn create_processor(
    handle: &crate::MediaPipelineHandle,
    config: HlsWriterConfig,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    if config.input_audio_track_id.is_none() && config.input_video_track_id.is_none() {
        return Err(crate::Error::new(
            "inputAudioTrackId or inputVideoTrackId is required".to_owned(),
        ));
    }

    let processor_id = processor_id.unwrap_or_else(|| crate::ProcessorId::new("hlsWriter"));
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("hls_writer"),
            move |h| async move {
                let writer = HlsWriter::new(
                    config.output_directory,
                    config.segment_duration,
                    config.max_retained_segments,
                    config.segment_format,
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
