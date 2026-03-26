use std::collections::VecDeque;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Duration;

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

/// HLS セグメントライター。
/// エンコード済みの H.264 + AAC フレームを MPEG-TS セグメントに分割し、
/// M3U8 プレイリストを管理する。
struct HlsWriter {
    output_directory: PathBuf,
    segment_duration_target: f64,
    max_retained_segments: usize,
    segment_sequence: u64,
    current_segment: Option<CurrentSegment>,
    retained_segments: VecDeque<RetainedSegment>,
    /// PAT/PMT 用の continuity counter（セグメント跨ぎで連続）
    pat_cc: ContinuityCounter,
    pmt_cc: ContinuityCounter,
    /// PES 用の continuity counter（セグメント跨ぎで連続）
    video_cc: ContinuityCounter,
    audio_cc: ContinuityCounter,
}

struct CurrentSegment {
    writer: TsPacketWriter<BufWriter<std::fs::File>>,
    filename: String,
    start_timestamp: Duration,
    last_timestamp: Duration,
    byte_count: u64,
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
    ) -> Self {
        Self {
            output_directory,
            segment_duration_target,
            max_retained_segments,
            segment_sequence: 0,
            current_segment: None,
            retained_segments: VecDeque::new(),
            pat_cc: ContinuityCounter::new(),
            pmt_cc: ContinuityCounter::new(),
            video_cc: ContinuityCounter::new(),
            audio_cc: ContinuityCounter::new(),
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
        if frame.keyframe {
            if let Some(ref seg) = self.current_segment {
                let elapsed = frame
                    .timestamp
                    .saturating_sub(seg.start_timestamp)
                    .as_secs_f64();
                if elapsed >= self.segment_duration_target {
                    self.finalize_current_segment()?;
                }
            }
        }

        // セグメントが無ければ新規作成（キーフレームで開始）
        if self.current_segment.is_none() {
            if !frame.keyframe {
                // キーフレーム以外では新セグメントを開始しない
                return Ok(());
            }
            self.start_new_segment(frame.timestamp)?;
        }

        let pts = duration_to_timestamp(frame.timestamp)?;
        self.write_pes_packets(
            Pid::new(VIDEO_PID).unwrap(),
            StreamId::new_video(StreamId::VIDEO_MIN).unwrap(),
            &frame.data,
            Some(pts),
            true, // ビデオ
        )?;

        if let Some(ref mut seg) = self.current_segment {
            seg.last_timestamp = frame.timestamp;
        }

        Ok(())
    }

    /// オーディオフレーム処理
    fn handle_audio_frame(&mut self, frame: &crate::AudioFrame) -> crate::Result<()> {
        if self.current_segment.is_none() {
            // ビデオのキーフレームがまだ来ていない場合はスキップ
            return Ok(());
        }

        let pts = duration_to_timestamp(frame.timestamp)?;
        self.write_pes_packets(
            Pid::new(AUDIO_PID).unwrap(),
            StreamId::new(StreamId::AUDIO_MIN),
            &frame.data,
            Some(pts),
            false, // オーディオ
        )?;

        if let Some(ref mut seg) = self.current_segment {
            seg.last_timestamp = frame.timestamp;
        }

        Ok(())
    }

    /// 新しいセグメントファイルを開始する
    fn start_new_segment(&mut self, timestamp: Duration) -> crate::Result<()> {
        let sequence = self.segment_sequence;
        self.segment_sequence += 1;
        let filename = format!("segment-{sequence:06}.ts");
        let path = self.output_directory.join(&filename);

        let file = std::fs::File::create(&path).map_err(|e| {
            crate::Error::new(format!(
                "failed to create segment file {}: {e}",
                path.display()
            ))
        })?;
        let buf_writer = BufWriter::new(file);
        let mut writer = TsPacketWriter::new(buf_writer);

        // セグメント先頭に PAT と PMT を書き出す
        self.write_pat(&mut writer)?;
        self.write_pmt(&mut writer)?;

        self.current_segment = Some(CurrentSegment {
            writer,
            filename,
            start_timestamp: timestamp,
            last_timestamp: timestamp,
            byte_count: 0,
        });

        Ok(())
    }

    /// PAT (Program Association Table) を書き出す
    fn write_pat<W: Write>(&mut self, writer: &mut TsPacketWriter<W>) -> crate::Result<()> {
        let cc = self.pat_cc;
        self.pat_cc.increment();
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

    /// PMT (Program Map Table) を書き出す
    fn write_pmt<W: Write>(&mut self, writer: &mut TsPacketWriter<W>) -> crate::Result<()> {
        let cc = self.pmt_cc;
        self.pmt_cc.increment();
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
    /// 大きなフレームは PesStart + 複数の PesContinuation に分割される。
    fn write_pes_packets(
        &mut self,
        pid: Pid,
        stream_id: StreamId,
        data: &[u8],
        pts: Option<Timestamp>,
        is_video: bool,
    ) -> crate::Result<()> {
        let seg = self
            .current_segment
            .as_mut()
            .ok_or_else(|| crate::Error::new("no active segment".to_owned()))?;

        let cc = if is_video {
            &mut self.video_cc
        } else {
            &mut self.audio_cc
        };

        // PES ヘッダを構築
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

        // PES ヘッダのサイズを計算
        // optional header: 3 バイト (flags + header_data_length) + PTS(5) + DTS(5)
        let optional_header_len: usize = 3 + pts.map_or(0, |_| 5) + pes_header.dts.map_or(0, |_| 5);
        // PES パケットの総ヘッダサイズ: start_code(3) + stream_id(1) + packet_len(2) + optional_header(N)
        let pes_header_size = 3 + 1 + 2 + optional_header_len;
        let total_pes_size = pes_header_size + data.len();

        // pes_packet_len: PES パケット長 (stream_id + packet_len の後から)
        // = optional_header_len + data.len()
        // 0 はビデオでの unbounded を意味する
        let pes_packet_len = if total_pes_size - 6 > u16::MAX as usize {
            0 // unbounded (ビデオでは一般的)
        } else {
            (total_pes_size - 6) as u16
        };

        // 最初の TS パケット: PesStart
        // TS パケットの payload は最大 184 バイト (188 - 4 header)
        // PES の先頭データを Bytes に収める
        let max_first_payload = Bytes::MAX_SIZE - pes_header_size;
        let first_data_len = data.len().min(max_first_payload);

        let first_data = Bytes::new(&data[..first_data_len])
            .map_err(|e| crate::Error::new(format!("failed to create PES start data: {e}")))?;

        let current_cc = *cc;
        cc.increment();

        // PCR を最初のビデオパケットの adaptation field に含める
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

        seg.writer
            .write_ts_packet(&start_packet)
            .map_err(|e| crate::Error::new(format!("failed to write PES start packet: {e}")))?;
        seg.byte_count += TsPacket::SIZE as u64;

        // 残りのデータを PesContinuation パケットで送る
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

            seg.writer.write_ts_packet(&cont_packet).map_err(|e| {
                crate::Error::new(format!("failed to write PES continuation packet: {e}"))
            })?;
            seg.byte_count += TsPacket::SIZE as u64;
            offset += chunk_len;
        }

        Ok(())
    }

    /// 現在のセグメントを完了し、プレイリストを更新する
    fn finalize_current_segment(&mut self) -> crate::Result<()> {
        let Some(seg) = self.current_segment.take() else {
            return Ok(());
        };

        // ファイルをフラッシュして閉じる
        let mut inner = seg.writer.into_stream();
        inner
            .flush()
            .map_err(|e| crate::Error::new(format!("failed to flush segment file: {e}")))?;
        drop(inner);

        let duration = seg
            .last_timestamp
            .saturating_sub(seg.start_timestamp)
            .as_secs_f64();
        // 最低限の duration を保証する
        let duration = duration.max(0.001);

        self.retained_segments.push_back(RetainedSegment {
            filename: seg.filename,
            duration,
        });

        // プレイリストを更新
        self.write_playlist()?;

        // 保持数超過分の古いセグメントを削除
        while self.retained_segments.len() > self.max_retained_segments {
            if let Some(old) = self.retained_segments.pop_front() {
                let path = self.output_directory.join(&old.filename);
                if let Err(e) = std::fs::remove_file(&path) {
                    tracing::warn!("failed to remove old segment {}: {e}", path.display());
                }
            }
        }

        Ok(())
    }

    /// M3U8 プレイリストを書き出す。
    /// 一時ファイルに書いてから rename してアトミックに更新する。
    fn write_playlist(&self) -> crate::Result<()> {
        if self.retained_segments.is_empty() {
            return Ok(());
        }

        // media sequence は最も古いセグメントの番号
        let media_sequence = self.segment_sequence as usize - self.retained_segments.len();

        // EXT-X-TARGETDURATION は最大セグメント尺の切り上げ整数
        let max_duration = self
            .retained_segments
            .iter()
            .map(|s| s.duration)
            .fold(0.0f64, f64::max);
        let target_duration = max_duration.ceil() as u64;
        let target_duration = target_duration.max(1);

        let mut content = String::new();
        content.push_str("#EXTM3U\n");
        content.push_str(&format!("#EXT-X-VERSION:3\n"));
        content.push_str(&format!("#EXT-X-TARGETDURATION:{target_duration}\n"));
        content.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{media_sequence}\n"));

        for seg in &self.retained_segments {
            content.push_str(&format!("#EXTINF:{:.3},\n", seg.duration));
            content.push_str(&seg.filename);
            content.push('\n');
        }

        // アトミック書き込み: 一時ファイル → rename
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
        // プレイリストを削除
        let playlist_path = self.output_directory.join(PLAYLIST_FILENAME);
        if let Err(e) = std::fs::remove_file(&playlist_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("failed to remove playlist {}: {e}", playlist_path.display());
            }
        }
        // 一時ファイルも削除
        let tmp_path = self.output_directory.join(".playlist.m3u8.tmp");
        let _ = std::fs::remove_file(&tmp_path);

        // 保持中のセグメントを削除
        for seg in &self.retained_segments {
            let path = self.output_directory.join(&seg.filename);
            if let Err(e) = std::fs::remove_file(&path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!("failed to remove segment {}: {e}", path.display());
                }
            }
        }
    }
}

/// Duration を mpeg2ts の Timestamp (90kHz) に変換する
fn duration_to_timestamp(d: Duration) -> crate::Result<Timestamp> {
    let ticks = (d.as_secs_f64() * Timestamp::RESOLUTION as f64) as u64;
    let ticks = ticks % (Timestamp::MAX + 1); // ラップアラウンド
    Timestamp::new(ticks).map_err(|e| crate::Error::new(format!("invalid timestamp: {e}")))
}

/// HLS writer プロセッサを作成する
pub async fn create_processor(
    handle: &crate::MediaPipelineHandle,
    output_directory: PathBuf,
    input_audio_track_id: Option<crate::TrackId>,
    input_video_track_id: Option<crate::TrackId>,
    segment_duration: f64,
    max_retained_segments: usize,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    if input_audio_track_id.is_none() && input_video_track_id.is_none() {
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
                let writer =
                    HlsWriter::new(output_directory, segment_duration, max_retained_segments);
                writer
                    .run(h, input_audio_track_id, input_video_track_id)
                    .await
            },
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}
