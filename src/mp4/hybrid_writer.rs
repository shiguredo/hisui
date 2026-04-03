use std::{
    fs::File,
    io::{BufWriter, Seek, SeekFrom, Write},
    path::Path,
    time::Duration,
};

use shiguredo_mp4::boxes::{Brand, FreeBox, FtypBox, MdatBox};
use shiguredo_mp4::mux::{
    Fmp4SegmentMuxer, Mp4FileMuxer, Mp4FileMuxerOptions, SegmentMuxerOptions,
};
use shiguredo_mp4::{BoxHeader, BoxSize, Decode, Encode};

use crate::{TrackId, audio::AudioFrame, video::VideoFrame};

use super::writer::{
    DEFAULT_SAMPLE_DURATION, InputTrackKind, MAX_CHUNK_DURATION, MAX_INPUT_QUEUE_GAP,
    Mp4WriterRpcMessage, Mp4WriterStats, TIMESCALE, WriterCore, WriterRunOutput,
    recv_mp4_writer_rpc_message_or_pending,
};

// hybrid MP4 のリカバリ用 moov を格納するための free ボックスの予約サイズ（バイト単位）
//
// 録画中に fMP4 用の moov をこの領域内に定期的に上書きすることで、
// クラッシュ時にも再生可能なファイルを維持する。
// moov のサイズはトラック数やサンプル数に依存するが、
// fMP4 の moov は mvex/trex ベースで stbl が空のため比較的小さい。
const HYBRID_FREE_BOX_RESERVED_SIZE: usize = 512 * 1024;

// ftyp ボックスの予約サイズ（バイト単位）
//
// 初期 ftyp は isom/iso2/mp41 の 3 ブランドで 28 バイトだが、
// finalize 時にコーデック固有のブランド（avc1, hev1 等）が追加されうるため、
// 拡張用のスペースを確保しておく。
const FTYP_RESERVED_SIZE: u64 = 64;

// フラグメントを自動フラッシュするまでの最大蓄積時間
const HYBRID_FRAGMENT_MAX_DURATION: Duration = Duration::from_secs(2);

/// Hybrid MP4 ライター
///
/// 録画中は fMP4（fragmented MP4）形式で書き込み、
/// 正常終了時に標準 MP4 に変換する。
/// プロセスクラッシュ時には、最後に flush 済みのフラグメントまでは
/// 再生可能なファイルが残る。
/// ただし、直近の未 flush 区間（途中のフラグメント）は失われうる。
///
/// ファイルレイアウト:
/// - 録画中: `[ftyp][moov(fMP4用)][free][moof1][mdat1][moof2][mdat2]...`
/// - finalize 後: `[ftyp][mdat(全データ)][moov(標準MP4)]`
#[derive(Debug)]
pub struct HybridMp4Writer {
    file: BufWriter<File>,

    // fMP4 フラグメント生成用
    fmp4_muxer: Fmp4SegmentMuxer,
    // 最初の実フラッシュ前に recovery 用 moov を先行更新するための専用 muxer
    initial_recovery_muxer: Option<Fmp4SegmentMuxer>,
    // finalize 時の標準 MP4 moov 生成用
    mp4_muxer: Mp4FileMuxer,

    // ファイルレイアウト
    mdat_start_offset: u64,
    free_box_total_size: u64,
    next_position: u64,

    // フラグメントバッファ（トラック別に管理する）
    // Fmp4SegmentMuxer は同一トラックのサンプルが mdat 内で連続配置されることを要求するため、
    // 映像と音声を分離して蓄積し、フラッシュ時に [映像データ][音声データ] として結合する。
    fragment_video_payload: Vec<u8>,
    fragment_audio_payload: Vec<u8>,
    fragment_video_samples: Vec<shiguredo_mp4::mux::Sample>,
    fragment_audio_samples: Vec<shiguredo_mp4::mux::Sample>,
    // フラグメントがカバーする実時間の範囲（自動フラッシュ判定用）
    fragment_start_timestamp: Option<Duration>,
    fragment_end_timestamp: Option<Duration>,
    fragment_accumulated_duration: Duration,
    // エンコーダーは sample_entry を初回のみ付与することがあるため保持する
    last_audio_sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>,
    last_video_sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>,
    has_flushed_fragment: bool,

    // Mp4Writer と共有する入力キュー・一時停止管理・統計情報
    core: WriterCore,
}

impl HybridMp4Writer {
    pub fn new<P: AsRef<Path>>(
        path: P,
        input_audio_track_id: Option<TrackId>,
        input_video_track_id: Option<TrackId>,
        mut stats: crate::stats::Stats,
    ) -> crate::Result<Self> {
        let creation_timestamp = std::time::UNIX_EPOCH.elapsed()?;

        // fMP4 フラグメント生成用 muxer
        let fmp4_muxer =
            Fmp4SegmentMuxer::with_options(SegmentMuxerOptions { creation_timestamp })?;

        // finalize 時の標準 MP4 moov 生成用 muxer
        let mut mp4_muxer = Mp4FileMuxer::with_options(Mp4FileMuxerOptions {
            creation_timestamp,
            reserved_moov_box_size: 0,
        })?;

        // ファイルを作成
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(path)?;

        // ftyp ボックスを書き出す
        // finalize 時にコーデック固有ブランドで更新するため、拡張用の free ボックスを後続に配置する
        let ftyp_box = FtypBox {
            major_brand: Brand::ISOM,
            minor_version: 0,
            compatible_brands: vec![Brand::ISOM, Brand::ISO2, Brand::MP41],
        };
        let ftyp_bytes = ftyp_box.encode_to_vec()?;
        file.write_all(&ftyp_bytes)?;

        // ftyp 拡張用の free ボックスを書き出す
        if let Some(free_payload_size) = FTYP_RESERVED_SIZE
            .checked_sub(ftyp_bytes.len() as u64)
            .and_then(|padding| padding.checked_sub(8))
        {
            let ftyp_padding = FreeBox {
                payload: vec![0; free_payload_size as usize],
            };
            file.write_all(&ftyp_padding.encode_to_vec()?)?;
        }
        // mdat ヘッダおよびリカバリ用 moov の書き込み先オフセット
        let mdat_start_offset = FTYP_RESERVED_SIZE;

        // リカバリ用 moov の予約領域（free ボックス）を書き出す
        let free_box = FreeBox {
            payload: vec![0; HYBRID_FREE_BOX_RESERVED_SIZE],
        };
        let free_bytes = free_box.encode_to_vec()?;
        let free_box_total_size = free_bytes.len() as u64;
        file.write_all(&free_bytes)?;

        let next_position = mdat_start_offset + free_box_total_size;

        // mp4_muxer の内部 next_position を実ファイル位置に合わせる
        // mp4_muxer は initial_boxes_bytes 分の next_position を持っているので、
        // 実ファイル位置との差分を advance_position で調整する
        let mp4_initial_size = mp4_muxer.initial_boxes_bytes().len() as u64;
        if next_position > mp4_initial_size {
            mp4_muxer.advance_position(next_position - mp4_initial_size)?;
        }

        let stats = Mp4WriterStats::new(&mut stats, 0);

        Ok(Self {
            file: BufWriter::new(file),
            fmp4_muxer,
            initial_recovery_muxer: Some(Fmp4SegmentMuxer::with_options(SegmentMuxerOptions {
                creation_timestamp,
            })?),
            mp4_muxer,
            mdat_start_offset,
            free_box_total_size,
            next_position,
            fragment_video_payload: Vec::new(),
            fragment_audio_payload: Vec::new(),
            fragment_video_samples: Vec::new(),
            fragment_audio_samples: Vec::new(),
            fragment_start_timestamp: None,
            fragment_end_timestamp: None,
            fragment_accumulated_duration: Duration::ZERO,
            last_audio_sample_entry: None,
            last_video_sample_entry: None,
            has_flushed_fragment: false,
            core: WriterCore::new(input_audio_track_id, input_video_track_id, stats),
        })
    }

    /// 統計情報を返す
    ///
    /// `recoverable_media_duration()` は異常終了時に回復可能な範囲
    /// （最後に flush 済みのフラグメントまで）を表す。
    /// 一方で `current_duration()` は未 flush 区間も含む現在の論理尺を返す。
    pub fn stats(&self) -> &Mp4WriterStats {
        &self.core.stats
    }

    /// 現在の論理尺を返す
    ///
    /// この値には未 flush 区間も含まれるため、異常終了時の回復保証範囲とは一致しない。
    pub fn current_duration(&self) -> Duration {
        self.core
            .stats
            .total_audio_track_duration()
            .max(self.core.stats.total_video_track_duration())
    }

    /// フラグメントにビデオサンプルを追加する
    fn append_video_to_fragment(&mut self, frame: &VideoFrame, duration: Duration) {
        // 映像 payload 内の相対オフセット（フラッシュ時に音声分をオフセットして最終位置を計算する）
        let offset_in_video = self.fragment_video_payload.len() as u64;
        self.fragment_video_payload.extend_from_slice(&frame.data);

        if self.core.stats.video_codec().is_none()
            && let Some(name) = frame.format.codec_name()
        {
            self.core.stats.set_video_codec(name);
        }

        if frame.sample_entry.is_some() {
            self.last_video_sample_entry.clone_from(&frame.sample_entry);
        }

        self.fragment_video_samples
            .push(shiguredo_mp4::mux::Sample {
                track_kind: shiguredo_mp4::TrackKind::Video,
                sample_entry: frame
                    .sample_entry
                    .clone()
                    .or_else(|| self.last_video_sample_entry.clone()),
                keyframe: frame.keyframe,
                timescale: TIMESCALE,
                duration: duration.as_micros() as u32,
                composition_time_offset: None,
                data_offset: offset_in_video,
                data_size: frame.data.len(),
            });

        self.core.stats.add_video_sample(frame.data.len(), duration);
        self.core.last_video_duration = Some(duration);
        self.update_fragment_time_range(frame.timestamp, duration);
        self.update_unflushed_fragment_metrics();
    }

    /// フラグメントにオーディオサンプルを追加する
    fn append_audio_to_fragment(&mut self, sample: &AudioFrame, duration: Duration) {
        let offset_in_audio = self.fragment_audio_payload.len() as u64;
        self.fragment_audio_payload.extend_from_slice(&sample.data);

        if self.core.stats.audio_codec().is_none()
            && let Some(name) = sample.format.codec_name()
        {
            self.core.stats.set_audio_codec(name);
        }

        if sample.sample_entry.is_some() {
            self.last_audio_sample_entry
                .clone_from(&sample.sample_entry);
        }

        self.fragment_audio_samples
            .push(shiguredo_mp4::mux::Sample {
                track_kind: shiguredo_mp4::TrackKind::Audio,
                sample_entry: sample
                    .sample_entry
                    .clone()
                    .or_else(|| self.last_audio_sample_entry.clone()),
                keyframe: true,
                timescale: TIMESCALE,
                duration: duration.as_micros() as u32,
                composition_time_offset: None,
                data_offset: offset_in_audio,
                data_size: sample.data.len(),
            });

        self.core
            .stats
            .add_audio_sample(sample.data.len(), duration);
        self.core.last_audio_duration = Some(duration);
        self.update_fragment_time_range(sample.timestamp, duration);
        self.update_unflushed_fragment_metrics();
    }

    fn has_fragment_samples(&self) -> bool {
        !self.fragment_video_samples.is_empty() || !self.fragment_audio_samples.is_empty()
    }

    fn update_fragment_time_range(&mut self, timestamp: Duration, duration: Duration) {
        let sample_end = timestamp.saturating_add(duration);
        self.fragment_start_timestamp = Some(
            self.fragment_start_timestamp
                .map_or(timestamp, |current| current.min(timestamp)),
        );
        self.fragment_end_timestamp = Some(
            self.fragment_end_timestamp
                .map_or(sample_end, |current| current.max(sample_end)),
        );
        self.fragment_accumulated_duration = self
            .fragment_start_timestamp
            .zip(self.fragment_end_timestamp)
            .map_or(Duration::ZERO, |(start, end)| end.saturating_sub(start));
    }

    fn update_unflushed_fragment_metrics(&self) {
        self.core
            .stats
            .set_current_unflushed_fragment_duration(self.fragment_accumulated_duration);
        self.core
            .stats
            .set_current_unflushed_video_sample_count(self.fragment_video_samples.len() as u64);
        self.core
            .stats
            .set_current_unflushed_audio_sample_count(self.fragment_audio_samples.len() as u64);
    }

    fn update_recoverable_media_metrics(&self) {
        self.core.stats.add_flushed_fragment();
        self.core
            .stats
            .set_recoverable_media_duration(self.current_duration());
    }

    /// 蓄積されたフラグメントをファイルに書き出す
    ///
    /// mdat payload 内のレイアウトは [映像データ][音声データ] の順。
    /// Fmp4SegmentMuxer は同一トラックのサンプルが連続配置されることを要求するため、
    /// トラックごとに分離して蓄積したデータをこの順序で結合する。
    fn flush_fragment(&mut self) -> crate::Result<()> {
        if !self.has_fragment_samples() {
            return Ok(());
        }

        // fmp4_muxer 用のサンプルリストを構築する
        // レイアウト: [映像サンプル群][音声サンプル群]
        // 音声の data_offset は映像データの合計サイズ分だけオフセットする
        let video_total_size = self.fragment_video_payload.len() as u64;
        let mut fmp4_samples = self.fragment_video_samples.clone();
        for sample in &self.fragment_audio_samples {
            let mut s = sample.clone();
            s.data_offset += video_total_size;
            fmp4_samples.push(s);
        }

        // fmp4_muxer でメディアセグメントメタデータ（moof + mdat ヘッダ）を生成する
        let metadata_bytes = self
            .fmp4_muxer
            .create_media_segment_metadata(&fmp4_samples)?;

        // ファイルに metadata + payload を書き出す
        // payload は [映像データ][音声データ] の順
        self.file.seek(SeekFrom::Start(self.next_position))?;
        self.file.write_all(&metadata_bytes)?;
        let payload_start = self.next_position + metadata_bytes.len() as u64;
        self.file.write_all(&self.fragment_video_payload)?;
        self.file.write_all(&self.fragment_audio_payload)?;

        // mp4_muxer: metadata 分のギャップを進める
        self.mp4_muxer
            .advance_position(metadata_bytes.len() as u64)?;

        // mp4_muxer: 映像サンプル → 音声サンプルの順で追加する
        // data_offset は実ファイル上の絶対位置
        let mut offset = payload_start;
        for sample in &self.fragment_video_samples {
            let mut s = sample.clone();
            s.data_offset = offset;
            self.mp4_muxer.append_sample(&s)?;
            offset += s.data_size as u64;
        }
        for sample in &self.fragment_audio_samples {
            let mut s = sample.clone();
            s.data_offset = offset;
            self.mp4_muxer.append_sample(&s)?;
            offset += s.data_size as u64;
        }

        self.next_position = offset;

        // フラグメントバッファをクリア
        self.fragment_video_payload.clear();
        self.fragment_audio_payload.clear();
        self.fragment_video_samples.clear();
        self.fragment_audio_samples.clear();
        self.fragment_start_timestamp = None;
        self.fragment_end_timestamp = None;
        self.fragment_accumulated_duration = Duration::ZERO;
        self.has_flushed_fragment = true;
        self.initial_recovery_muxer = None;
        self.update_unflushed_fragment_metrics();
        // リカバリ用 moov を更新する
        self.update_recovery_moov()?;
        self.update_recoverable_media_metrics();

        Ok(())
    }

    /// pending サンプルから recovery 用 moov を先行生成する
    ///
    /// pending サンプルは通常、次のサンプル到着時に duration が確定してからフラグメントに追加されるが、
    /// 最初のサンプルだけを受け取った直後にクラッシュすると、フラグメントもリカバリ用 moov も
    /// 書かれないまま終了してしまう。
    /// これを防ぐために、pending を仮の duration で初回専用の recovery muxer にだけ反映して
    /// recovery 用 moov を生成する。
    ///
    /// ここでは moov だけを先行更新し、対応する moof / mdat と payload 自体は
    /// flush 時までディスクへ書かない。
    /// そのため、異常終了時に回復できるのは最後に flush 済みのフラグメントまでであり、
    /// 直近の未 flush 区間が失われることは許容する。
    /// 最初の実フラグメントを flush した後は、この経路は無効化される。
    fn maybe_flush_initial_pending(&mut self) -> crate::Result<()> {
        if self.has_flushed_fragment {
            return Ok(());
        }
        let mut samples = Vec::new();
        let mut data_offset = 0;

        if let Some(pending) = self.core.pending_video_frame.as_ref()
            && let Some(sample_entry) = pending
                .sample_entry
                .clone()
                .or_else(|| self.last_video_sample_entry.clone())
        {
            samples.push(shiguredo_mp4::mux::Sample {
                track_kind: shiguredo_mp4::TrackKind::Video,
                sample_entry: Some(sample_entry),
                keyframe: pending.keyframe,
                timescale: TIMESCALE,
                duration: DEFAULT_SAMPLE_DURATION.as_micros() as u32,
                composition_time_offset: None,
                data_offset,
                data_size: pending.data.len(),
            });
            data_offset += pending.data.len() as u64;
        }

        if let Some(pending) = self.core.pending_audio_sample.as_ref()
            && let Some(sample_entry) = pending
                .sample_entry
                .clone()
                .or_else(|| self.last_audio_sample_entry.clone())
        {
            samples.push(shiguredo_mp4::mux::Sample {
                track_kind: shiguredo_mp4::TrackKind::Audio,
                sample_entry: Some(sample_entry),
                keyframe: true,
                timescale: TIMESCALE,
                duration: DEFAULT_SAMPLE_DURATION.as_micros() as u32,
                composition_time_offset: None,
                data_offset,
                data_size: pending.data.len(),
            });
        }

        if !samples.is_empty() {
            let Some(mut muxer) = self.initial_recovery_muxer.take() else {
                return Ok(());
            };
            let result = (|| -> crate::Result<()> {
                muxer.create_media_segment_metadata(&samples)?;
                self.update_recovery_moov_from_muxer(&muxer)
            })();
            self.initial_recovery_muxer = Some(muxer);
            result?;
        }
        Ok(())
    }

    /// 蓄積時間が閾値を超えた場合にフラグメントをフラッシュする
    fn maybe_flush_fragment_by_duration(&mut self) -> crate::Result<()> {
        if self.fragment_accumulated_duration >= HYBRID_FRAGMENT_MAX_DURATION
            && self.has_fragment_samples()
        {
            self.flush_fragment()?;
        }
        Ok(())
    }

    /// free ボックス領域にリカバリ用の fMP4 moov を書き込む
    fn update_recovery_moov(&mut self) -> crate::Result<()> {
        let muxer = self.fmp4_muxer.clone();
        self.update_recovery_moov_from_muxer(&muxer)
    }

    /// 指定された muxer の状態から recovery 用の fMP4 moov を書き込む
    fn update_recovery_moov_from_muxer(&mut self, muxer: &Fmp4SegmentMuxer) -> crate::Result<()> {
        let init_bytes = match muxer.init_segment_bytes() {
            Ok(bytes) => bytes,
            Err(shiguredo_mp4::mux::MuxError::EmptyTracks) => {
                return Ok(());
            }
            Err(e) => {
                return Err(crate::Error::new(format!(
                    "failed to get init segment: {e}"
                )));
            }
        };

        // init_segment_bytes（ftyp + moov）から moov 部分を取得する
        let moov_bytes = extract_moov_from_init_segment(&init_bytes)?;
        let moov_size = moov_bytes.len() as u64;

        // free ボックスの最小サイズは 8 バイト（ヘッダのみ）
        const MIN_FREE_BOX_SIZE: u64 = 8;

        if moov_size + MIN_FREE_BOX_SIZE > self.free_box_total_size
            && moov_size != self.free_box_total_size
        {
            tracing::warn!(
                moov_size,
                free_box_total_size = self.free_box_total_size,
                "recovery moov exceeds reserved free box size, skipping update"
            );
            return Ok(());
        }

        // free 領域に [moov][free(残余パディング)] を書き込む
        self.file.seek(SeekFrom::Start(self.mdat_start_offset))?;
        self.file.write_all(moov_bytes)?;

        let remaining = self.free_box_total_size - moov_size;
        if let Some(payload_size) = remaining.checked_sub(MIN_FREE_BOX_SIZE) {
            let free_box = FreeBox {
                payload: vec![0; payload_size as usize],
            };
            self.file.write_all(&free_box.encode_to_vec()?)?;
        }

        self.file.flush()?;
        self.core.stats.add_recovery_moov_update();

        Ok(())
    }

    /// 録画を finalize して標準 MP4 に変換する
    fn finalize(&mut self) -> crate::Result<()> {
        // 残りのフラグメントをフラッシュ
        self.flush_fragment()?;

        // mp4_muxer を finalize して標準 MP4 の moov と更新済み ftyp を取得する
        let finalized = self.mp4_muxer.finalize()?;
        let actual_moov_size = finalized.moov_box_size() as u64;
        self.core.stats.set_actual_moov_box_size(actual_moov_size);

        let pairs: Vec<_> = finalized.offset_and_bytes_pairs().collect();

        // head_boxes（更新済み ftyp + free）をファイル先頭に書き戻す
        // Mp4FileMuxer が構築した head_boxes にはコーデック固有の compatible brand を含む
        // ftyp が入っている。hybrid MP4 では ftyp 部分のみを抽出して予約領域に書き込む。
        if let Some((_, head_boxes)) = pairs.first() {
            let ftyp_size = match BoxHeader::decode(head_boxes) {
                Ok((header, _)) => header.box_size.get(),
                Err(e) => {
                    tracing::warn!("failed to decode ftyp box header: {e}");
                    0
                }
            };
            if ftyp_size > 0 && ftyp_size <= self.mdat_start_offset {
                self.file.seek(SeekFrom::Start(0))?;
                self.file.write_all(&head_boxes[..ftyp_size as usize])?;

                // ftyp と mdat_start_offset の間を free ボックスで埋める
                if let Some(free_payload_size) = self
                    .mdat_start_offset
                    .checked_sub(ftyp_size)
                    .and_then(|padding| padding.checked_sub(8))
                {
                    let free_box = FreeBox {
                        payload: vec![0; free_payload_size as usize],
                    };
                    self.file.write_all(&free_box.encode_to_vec()?)?;
                }
            }
        }

        // free ボックスのヘッダを mdat ヘッダに書き換える
        // hybrid MP4 では録画中の recovery 用予約領域と、各フラグメントの metadata
        // オーバーヘッドを詰め直さず、そのまま最終 MP4 の mdat 内に残す。
        let mdat_size = self.next_position - self.mdat_start_offset;
        let mdat_header = BoxHeader {
            box_type: MdatBox::TYPE,
            box_size: BoxSize::U64(mdat_size),
        };
        self.file.seek(SeekFrom::Start(self.mdat_start_offset))?;
        self.file.write_all(&mdat_header.encode_to_vec()?)?;

        // moov を EOF に追記する
        // moov は offset が最大のエントリ
        self.file.seek(SeekFrom::Start(self.next_position))?;
        if let Some((_offset, moov_bytes)) = pairs.iter().max_by_key(|(offset, _)| *offset) {
            self.file.write_all(moov_bytes)?;
        }

        self.file.flush()?;

        Ok(())
    }

    fn handle_next_audio_and_video(&mut self) -> crate::Result<bool> {
        self.flush_pending_audio_if_ready()?;
        self.flush_pending_video_if_ready()?;

        let audio_timestamp = self.core.input_audio_queue.front().map(|x| x.timestamp);
        let video_timestamp = self.core.input_video_queue.front().map(|x| x.timestamp);
        match (audio_timestamp, video_timestamp) {
            (None, None) => {
                if self.core.pending_audio_sample.is_some()
                    || self.core.pending_video_frame.is_some()
                {
                    return Ok(true);
                }
                // 全入力の処理が完了 → finalize
                self.finalize()?;
                return Ok(false);
            }
            (None, Some(_)) => {
                self.process_next_video_frame()?;
            }
            (Some(_), None) => {
                self.process_next_audio_sample()?;
            }
            (Some(audio_timestamp), Some(video_timestamp)) => {
                let should_process_audio = (self.core.appending_video_chunk
                    && video_timestamp.saturating_sub(audio_timestamp) > MAX_CHUNK_DURATION)
                    || (!self.core.appending_video_chunk && video_timestamp > audio_timestamp);
                if should_process_audio {
                    self.process_next_audio_sample()?;
                } else {
                    self.process_next_video_frame()?;
                }
            }
        }

        Ok(true)
    }

    fn process_next_video_frame(&mut self) -> crate::Result<()> {
        let frame = self
            .core
            .input_video_queue
            .pop_front()
            .ok_or_else(|| crate::Error::new("video input queue is unexpectedly empty"))?;

        // pending フレームを現在のフラグメントに追加する（flush の前に行うことで
        // GOP の最後のフレームが正しく前のフラグメントに含まれるようにする）
        if let Some(pending) = self.core.pending_video_frame.as_ref() {
            let duration = WriterCore::sample_duration_from_timestamps(
                pending.timestamp,
                frame.timestamp,
                self.core.last_video_duration,
            );
            let pending = pending.clone();
            self.append_video_to_fragment(&pending, duration);
        }

        // キーフレーム到着時にフラグメントを区切る
        // pending は既にフラグメントに追加済みなので、ここでフラッシュすると
        // 前の GOP が完全な形でフラグメントに書き出される
        if frame.keyframe && self.has_fragment_samples() {
            self.flush_fragment()?;
        }

        // 蓄積時間が閾値を超えた場合もフラッシュする
        self.maybe_flush_fragment_by_duration()?;

        self.core.pending_video_frame = Some(frame);
        self.core.appending_video_chunk = true;

        // 最初のサンプルしか届いていない段階でも recovery 用 moov を先行更新する。
        self.maybe_flush_initial_pending()?;

        Ok(())
    }

    fn process_next_audio_sample(&mut self) -> crate::Result<()> {
        let data = self
            .core
            .input_audio_queue
            .pop_front()
            .ok_or_else(|| crate::Error::new("audio input queue is unexpectedly empty"))?;

        if let Some(pending) = self.core.pending_audio_sample.as_ref() {
            let duration = WriterCore::sample_duration_from_timestamps(
                pending.timestamp,
                data.timestamp,
                self.core.last_audio_duration,
            );
            let pending = pending.clone();
            self.append_audio_to_fragment(&pending, duration);

            self.maybe_flush_fragment_by_duration()?;
        }
        self.core.pending_audio_sample = Some(data);
        self.core.appending_video_chunk = false;

        self.maybe_flush_initial_pending()?;

        Ok(())
    }

    fn flush_pending_audio_if_ready(&mut self) -> crate::Result<()> {
        if self.core.input_audio_track_id.is_none()
            && self.core.input_audio_queue.is_empty()
            && self.core.pending_audio_sample.is_some()
        {
            let duration = self
                .core
                .last_audio_duration
                .unwrap_or(DEFAULT_SAMPLE_DURATION);
            let pending = self
                .core
                .pending_audio_sample
                .take()
                .expect("pending audio sample is unexpectedly empty");
            self.append_audio_to_fragment(&pending, duration);
        }
        Ok(())
    }

    fn flush_pending_video_if_ready(&mut self) -> crate::Result<()> {
        if self.core.input_video_track_id.is_none()
            && self.core.input_video_queue.is_empty()
            && self.core.pending_video_frame.is_some()
        {
            let duration = self
                .core
                .last_video_duration
                .unwrap_or(DEFAULT_SAMPLE_DURATION);
            let pending = self
                .core
                .pending_video_frame
                .take()
                .expect("pending video frame is unexpectedly empty");
            self.append_video_to_fragment(&pending, duration);
        }
        Ok(())
    }
}

/// init_segment_bytes（ftyp + moov）から moov 部分のバイト列を取得する
fn extract_moov_from_init_segment(init_bytes: &[u8]) -> crate::Result<&[u8]> {
    let (header, _) = BoxHeader::decode(init_bytes)?;
    let ftyp_size = header.box_size.get() as usize;
    if ftyp_size > init_bytes.len() {
        return Err(crate::Error::new("ftyp box size exceeds init segment size"));
    }
    Ok(&init_bytes[ftyp_size..])
}

// HybridMp4Writer の async 入力処理と RPC ハンドリング
impl HybridMp4Writer {
    fn poll_output(&mut self) -> crate::Result<WriterRunOutput> {
        loop {
            let waiting_video =
                self.core.input_video_track_id.is_some() && self.core.input_video_queue.is_empty();
            let waiting_audio =
                self.core.input_audio_track_id.is_some() && self.core.input_audio_queue.is_empty();

            if waiting_video && waiting_audio {
                return Ok(WriterRunOutput::Pending {
                    awaiting_track_kind: None,
                });
            } else if waiting_video && self.core.input_audio_track_id.is_none() {
                return Ok(WriterRunOutput::Pending {
                    awaiting_track_kind: Some(InputTrackKind::Video),
                });
            } else if waiting_audio && self.core.input_video_track_id.is_none() {
                return Ok(WriterRunOutput::Pending {
                    awaiting_track_kind: Some(InputTrackKind::Audio),
                });
            }

            let in_progress = self.handle_next_audio_and_video()?;

            if !in_progress {
                return Ok(WriterRunOutput::Finished);
            }
        }
    }

    pub async fn run(
        mut self,
        handle: crate::ProcessorHandle,
        input_audio_track_id: Option<crate::TrackId>,
        input_video_track_id: Option<crate::TrackId>,
    ) -> crate::Result<()> {
        let mut audio_rx = input_audio_track_id.map(|id| handle.subscribe_track(id));
        let mut video_rx = input_video_track_id.map(|id| handle.subscribe_track(id));
        let (rpc_tx, mut rpc_rx) = tokio::sync::mpsc::unbounded_channel();
        handle.register_rpc_sender(rpc_tx).await.map_err(|e| {
            crate::Error::new(format!("failed to register mp4 writer RPC sender: {e}"))
        })?;

        handle.notify_ready();

        if video_rx.is_some()
            && let Err(e) = crate::encoder::request_upstream_video_keyframe(
                &handle.pipeline_handle(),
                handle.processor_id(),
                "hybrid_mp4_writer_start",
            )
            .await
        {
            tracing::warn!(
                "failed to request keyframe for hybrid mp4 writer start: {}",
                e.display()
            );
        }
        let mut rpc_rx_enabled = true;

        let mut output = self.poll_output()?;
        loop {
            match output {
                WriterRunOutput::Finished => break,
                WriterRunOutput::Pending {
                    awaiting_track_kind,
                } => {
                    if audio_rx.is_none() && video_rx.is_none() {
                        output = self.poll_output()?;
                        continue;
                    }

                    match awaiting_track_kind {
                        Some(InputTrackKind::Audio) if audio_rx.is_some() => {
                            tokio::select! {
                                msg = crate::future::recv_or_pending(&mut audio_rx) => {
                                    self.handle_audio_message(msg, &mut audio_rx)?;
                                }
                                rpc_message = recv_mp4_writer_rpc_message_or_pending(
                                    rpc_rx_enabled.then_some(&mut rpc_rx)
                                ) => {
                                    if self.handle_rpc_message(rpc_message, &mut rpc_rx_enabled)? {
                                        audio_rx = None;
                                        video_rx = None;
                                    }
                                }
                            }
                        }
                        Some(InputTrackKind::Video) if video_rx.is_some() => {
                            tokio::select! {
                                msg = crate::future::recv_or_pending(&mut video_rx) => {
                                    self.handle_video_message(msg, &mut video_rx)?;
                                }
                                rpc_message = recv_mp4_writer_rpc_message_or_pending(
                                    rpc_rx_enabled.then_some(&mut rpc_rx)
                                ) => {
                                    if self.handle_rpc_message(rpc_message, &mut rpc_rx_enabled)? {
                                        audio_rx = None;
                                        video_rx = None;
                                    }
                                }
                            }
                        }
                        _ => {
                            let audio_len = self.core.input_audio_queue.len();
                            let video_len = self.core.input_video_queue.len();
                            let mut suppress_audio = false;
                            let mut suppress_video = false;
                            if audio_rx.is_some() && video_rx.is_some() {
                                if audio_len > video_len + MAX_INPUT_QUEUE_GAP {
                                    suppress_audio = true;
                                } else if video_len > audio_len + MAX_INPUT_QUEUE_GAP {
                                    suppress_video = true;
                                }
                            }

                            tokio::select! {
                                msg = crate::future::recv_or_pending(&mut audio_rx), if !suppress_audio => {
                                    self.handle_audio_message(msg, &mut audio_rx)?;
                                }
                                msg = crate::future::recv_or_pending(&mut video_rx), if !suppress_video => {
                                    self.handle_video_message(msg, &mut video_rx)?;
                                }
                                rpc_message = recv_mp4_writer_rpc_message_or_pending(
                                    rpc_rx_enabled.then_some(&mut rpc_rx)
                                ) => {
                                    if self.handle_rpc_message(rpc_message, &mut rpc_rx_enabled)? {
                                        audio_rx = None;
                                        video_rx = None;
                                    }
                                }
                            }
                        }
                    }
                    output = self.poll_output()?;
                }
            }
        }

        Ok(())
    }

    fn handle_rpc_message(
        &mut self,
        rpc_message: Option<Mp4WriterRpcMessage>,
        rpc_rx_enabled: &mut bool,
    ) -> crate::Result<bool> {
        let Some(rpc_message) = rpc_message else {
            *rpc_rx_enabled = false;
            return Ok(false);
        };

        match rpc_message {
            Mp4WriterRpcMessage::Pause { reply_tx } => {
                let _ = reply_tx.send(self.core.pause_recording());
            }
            Mp4WriterRpcMessage::Resume { reply_tx } => {
                let _ = reply_tx.send(self.core.resume_recording());
            }
            Mp4WriterRpcMessage::Finish { reply_tx } => {
                let _ = reply_tx.send(());
                *rpc_rx_enabled = false;
                self.core.input_video_track_id = None;
                self.core.input_audio_track_id = None;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn handle_audio_message(
        &mut self,
        msg: crate::Message,
        audio_rx: &mut Option<crate::MessageReceiver>,
    ) -> crate::Result<()> {
        match msg {
            crate::Message::Media(crate::MediaFrame::Audio(sample)) => {
                self.core.stats.add_received_audio_data();
                if self.core.input_audio_track_id.is_some() {
                    self.core.handle_input_sample(
                        InputTrackKind::Audio,
                        Some(crate::MediaFrame::Audio(sample)),
                    )?;
                }
            }
            crate::Message::Eos => {
                self.core.stats.add_received_audio_eos();
                if self.core.input_audio_track_id.is_some() {
                    self.core.handle_input_sample(InputTrackKind::Audio, None)?;
                }
                *audio_rx = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_video_message(
        &mut self,
        msg: crate::Message,
        video_rx: &mut Option<crate::MessageReceiver>,
    ) -> crate::Result<()> {
        match msg {
            crate::Message::Media(crate::MediaFrame::Video(sample)) => {
                self.core.stats.add_received_video_data();
                if self.core.input_video_track_id.is_some() {
                    self.core.handle_input_sample(
                        InputTrackKind::Video,
                        Some(crate::MediaFrame::Video(sample)),
                    )?;
                }
            }
            crate::Message::Eos => {
                self.core.stats.add_received_video_eos();
                if self.core.input_video_track_id.is_some() {
                    self.core.handle_input_sample(InputTrackKind::Video, None)?;
                }
                *video_rx = None;
            }
            _ => {}
        }
        Ok(())
    }
}

pub async fn create_processor(
    handle: &crate::MediaPipelineHandle,
    output_path: std::path::PathBuf,
    input_audio_track_id: Option<crate::TrackId>,
    input_video_track_id: Option<crate::TrackId>,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    if input_audio_track_id.is_none() && input_video_track_id.is_none() {
        return Err(crate::Error::new(
            "inputAudioTrackId or inputVideoTrackId is required".to_owned(),
        ));
    }

    let is_mp4 = output_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"));
    if !is_mp4 {
        return Err(crate::Error::new(format!(
            "outputPath must be an mp4 file: {}",
            output_path.display()
        )));
    }

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        return Err(crate::Error::new(format!(
            "outputPath parent directory does not exist: {}",
            parent.display()
        )));
    }

    let processor_id = processor_id.unwrap_or_else(|| crate::ProcessorId::new("hybridMp4Writer"));
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("hybrid_mp4_writer"),
            move |h| async move {
                let writer = HybridMp4Writer::new(
                    &output_path,
                    input_audio_track_id.clone(),
                    input_video_track_id.clone(),
                    h.stats(),
                )?;
                writer
                    .run(h, input_audio_track_id, input_video_track_id)
                    .await
            },
        )
        .await
        .map_err(|e| crate::Error::new(format!("{e}: {processor_id}")))?;
    Ok(processor_id)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::mp4::writer::DEFAULT_SAMPLE_DURATION;

    use crate::{
        audio::{AudioFormat, Channels, SampleRate},
        types::EvenUsize,
        video::VideoFormat,
    };

    fn make_hybrid_writer() -> crate::Result<(tempfile::TempDir, HybridMp4Writer)> {
        let temp_dir = tempfile::tempdir()?;
        let output_path = temp_dir.path().join("test.mp4");
        let writer = HybridMp4Writer::new(
            &output_path,
            Some(TrackId::new("audio")),
            Some(TrackId::new("video")),
            crate::stats::Stats::new(),
        )?;
        Ok((temp_dir, writer))
    }

    fn make_audio_frame(sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>) -> AudioFrame {
        AudioFrame {
            data: vec![0x11, 0x22, 0x33],
            format: AudioFormat::Aac,
            channels: Channels::STEREO,
            sample_rate: SampleRate::HZ_48000,
            timestamp: Duration::ZERO,
            sample_entry,
        }
    }

    fn make_video_frame(sample_entry: Option<shiguredo_mp4::boxes::SampleEntry>) -> VideoFrame {
        VideoFrame {
            data: vec![0x00, 0x00, 0x00, 0x01],
            format: VideoFormat::Av1,
            keyframe: true,
            size: Some(crate::video::VideoFrameSize {
                width: 16,
                height: 16,
            }),
            timestamp: Duration::ZERO,
            sample_entry,
        }
    }

    #[test]
    fn hybrid_writer_keeps_audio_sample_entry_across_fragments() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;
        let sample_entry = crate::audio::aac::create_mp4a_sample_entry(
            &[0x12, 0x10],
            SampleRate::HZ_48000,
            Channels::STEREO,
        )?;

        writer.append_audio_to_fragment(
            &make_audio_frame(Some(sample_entry.clone())),
            DEFAULT_SAMPLE_DURATION,
        );
        writer.flush_fragment()?;
        writer.append_audio_to_fragment(&make_audio_frame(None), DEFAULT_SAMPLE_DURATION);

        assert_eq!(writer.fragment_audio_samples.len(), 1);
        assert_eq!(
            writer.fragment_audio_samples[0].sample_entry,
            Some(sample_entry)
        );
        Ok(())
    }

    #[test]
    fn hybrid_writer_keeps_video_sample_entry_across_fragments() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;
        let sample_entry = crate::video::av1::av1_sample_entry(
            EvenUsize::MIN_CELL_SIZE,
            EvenUsize::MIN_CELL_SIZE,
            &[0x0A],
        );

        writer.append_video_to_fragment(
            &make_video_frame(Some(sample_entry.clone())),
            DEFAULT_SAMPLE_DURATION,
        );
        writer.flush_fragment()?;
        writer.append_video_to_fragment(&make_video_frame(None), DEFAULT_SAMPLE_DURATION);

        assert_eq!(writer.fragment_video_samples.len(), 1);
        assert_eq!(
            writer.fragment_video_samples[0].sample_entry,
            Some(sample_entry)
        );
        Ok(())
    }

    #[test]
    fn hybrid_writer_consumes_audio_queue_before_waiting_for_video() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;
        writer
            .core
            .input_audio_queue
            .push_back(Arc::new(make_audio_frame(None)));

        let output = writer.poll_output()?;

        assert!(matches!(
            output,
            WriterRunOutput::Pending {
                awaiting_track_kind: None
            }
        ));
        assert!(writer.core.pending_audio_sample.is_some());
        Ok(())
    }

    #[test]
    fn hybrid_writer_does_not_duplicate_initial_pending_audio_sample() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;
        let sample_entry = crate::audio::aac::create_mp4a_sample_entry(
            &[0x12, 0x10],
            SampleRate::HZ_48000,
            Channels::STEREO,
        )?;
        let first = AudioFrame {
            timestamp: Duration::ZERO,
            ..make_audio_frame(Some(sample_entry))
        };
        let second = AudioFrame {
            timestamp: DEFAULT_SAMPLE_DURATION,
            ..make_audio_frame(None)
        };

        writer.core.pending_audio_sample = Some(Arc::new(first.clone()));
        writer.maybe_flush_initial_pending()?;

        assert!(writer.fragment_audio_samples.is_empty());
        assert!(writer.core.pending_audio_sample.is_some());

        writer.core.input_audio_queue.push_back(Arc::new(second));
        writer.process_next_audio_sample()?;

        assert_eq!(writer.fragment_audio_samples.len(), 1);
        assert_eq!(writer.fragment_audio_samples[0].data_size, first.data.len());
        Ok(())
    }

    #[test]
    fn hybrid_writer_recovery_guarantee_stops_at_last_flushed_fragment() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;

        let first = AudioFrame {
            timestamp: Duration::ZERO,
            ..make_audio_frame(Some(crate::audio::aac::create_mp4a_sample_entry(
                &[0x12, 0x10],
                SampleRate::HZ_48000,
                Channels::STEREO,
            )?))
        };
        let second = AudioFrame {
            timestamp: DEFAULT_SAMPLE_DURATION,
            ..make_audio_frame(None)
        };
        let third = AudioFrame {
            timestamp: DEFAULT_SAMPLE_DURATION.saturating_mul(2),
            ..make_audio_frame(None)
        };

        writer.core.pending_audio_sample = Some(Arc::new(first));
        writer.core.input_audio_queue.push_back(Arc::new(second));
        writer.process_next_audio_sample()?;
        writer.flush_fragment()?;

        assert_eq!(writer.stats().total_flushed_fragment_count(), 1);
        assert_eq!(
            writer.stats().recoverable_media_duration(),
            DEFAULT_SAMPLE_DURATION
        );
        assert_eq!(
            writer.stats().current_unflushed_fragment_duration(),
            Duration::ZERO
        );

        writer.core.input_audio_queue.push_back(Arc::new(third));
        writer.process_next_audio_sample()?;

        assert_eq!(
            writer.stats().recoverable_media_duration(),
            DEFAULT_SAMPLE_DURATION
        );
        assert_eq!(
            writer.stats().current_unflushed_fragment_duration(),
            DEFAULT_SAMPLE_DURATION
        );
        assert_eq!(writer.stats().current_unflushed_audio_sample_count(), 1);
        assert_eq!(
            writer.current_duration(),
            DEFAULT_SAMPLE_DURATION.saturating_mul(2)
        );
        Ok(())
    }

    #[test]
    fn hybrid_writer_disables_initial_recovery_path_after_first_flush() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;
        let sample_entry = crate::audio::aac::create_mp4a_sample_entry(
            &[0x12, 0x10],
            SampleRate::HZ_48000,
            Channels::STEREO,
        )?;
        writer.core.pending_audio_sample = Some(Arc::new(AudioFrame {
            timestamp: Duration::ZERO,
            ..make_audio_frame(Some(sample_entry))
        }));

        writer.maybe_flush_initial_pending()?;
        assert!(writer.initial_recovery_muxer.is_some());

        writer.core.input_audio_track_id = None;
        writer.flush_pending_audio_if_ready()?;
        writer.flush_fragment()?;

        assert!(writer.has_flushed_fragment);
        assert!(writer.initial_recovery_muxer.is_none());

        let recovery_updates = writer.stats().total_recovery_moov_update_count();
        writer.maybe_flush_initial_pending()?;
        assert_eq!(
            writer.stats().total_recovery_moov_update_count(),
            recovery_updates
        );
        Ok(())
    }

    #[test]
    fn hybrid_writer_does_not_double_update_recovery_moov_after_flush() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;
        let sample_entry = crate::audio::aac::create_mp4a_sample_entry(
            &[0x12, 0x10],
            SampleRate::HZ_48000,
            Channels::STEREO,
        )?;
        writer.core.pending_audio_sample = Some(Arc::new(AudioFrame {
            timestamp: Duration::ZERO,
            ..make_audio_frame(Some(sample_entry))
        }));
        writer.maybe_flush_initial_pending()?;
        writer
            .core
            .input_audio_queue
            .push_back(Arc::new(AudioFrame {
                timestamp: HYBRID_FRAGMENT_MAX_DURATION.saturating_add(DEFAULT_SAMPLE_DURATION),
                ..make_audio_frame(None)
            }));

        writer.process_next_audio_sample()?;

        assert_eq!(writer.stats().total_recovery_moov_update_count(), 2);
        Ok(())
    }

    #[test]
    fn hybrid_writer_fragment_duration_uses_wall_clock_span() -> crate::Result<()> {
        let (_temp_dir, mut writer) = make_hybrid_writer()?;

        writer.append_video_to_fragment(
            &VideoFrame {
                timestamp: Duration::ZERO,
                ..make_video_frame(None)
            },
            DEFAULT_SAMPLE_DURATION,
        );
        writer.append_audio_to_fragment(
            &AudioFrame {
                timestamp: Duration::ZERO,
                ..make_audio_frame(None)
            },
            DEFAULT_SAMPLE_DURATION,
        );

        assert_eq!(
            writer.stats().current_unflushed_fragment_duration(),
            DEFAULT_SAMPLE_DURATION
        );
        assert_eq!(writer.stats().current_unflushed_video_sample_count(), 1);
        assert_eq!(writer.stats().current_unflushed_audio_sample_count(), 1);
        Ok(())
    }
}
