use std::collections::VecDeque;
use std::io::{BufWriter, Write};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

/// ファイル拡張子から content-type を返す
fn content_type_for_filename(filename: &str) -> &'static str {
    if filename.ends_with(".mpd") {
        "application/dash+xml"
    } else if filename.ends_with(".mp4") || filename.ends_with(".m4s") {
        "video/mp4"
    } else {
        "application/octet-stream"
    }
}

/// DASH writer の統計値
struct DashWriterStats {
    total_input_video_frame_count: crate::stats::StatsCounter,
    total_input_audio_frame_count: crate::stats::StatsCounter,
    total_segment_count: crate::stats::StatsCounter,
    total_segment_byte_size: crate::stats::StatsCounter,
    total_deleted_segment_count: crate::stats::StatsCounter,
    current_retained_segment_count: crate::stats::StatsGauge,
}

impl DashWriterStats {
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
        let mut stats = self.stats.clone();
        stats.set_default_label("status_code", &status_code.to_string());
        stats.counter(self.metric_name).inc();
    }
}

/// DASH 出力先のストレージ抽象
enum DashStorage {
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

impl DashStorage {
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
                    .build_request()
                    .map_err(|e| {
                        crate::Error::new(format!("failed to build S3 PutObject request: {e}"))
                    })?;
                match s3.client.execute(&request).await {
                    Ok(response) => {
                        s3.put_counts.record(response.status_code);
                        if !response.is_success() {
                            return Err(crate::Error::new(format!(
                                "S3 PutObject failed for {key}: status={}",
                                response.status_code
                            )));
                        }
                        Ok(())
                    }
                    Err(e) => {
                        s3.put_error_count.inc();
                        Err(crate::Error::new(format!(
                            "S3 PutObject failed for {key}: {}",
                            e.display()
                        )))
                    }
                }
            }
        }
    }

    /// マニフェストファイルを書き出す（filesystem ではアトミック更新）
    async fn write_manifest(&self, filename: &str, data: &[u8]) -> crate::Result<()> {
        match self {
            Self::Filesystem(fs) => {
                let final_path = fs.output_directory.join(filename);
                let tmp_path = fs.output_directory.join(format!(".{filename}.tmp"));
                std::fs::write(&tmp_path, data).map_err(|e| {
                    crate::Error::new(format!(
                        "failed to write temporary manifest {}: {e}",
                        tmp_path.display()
                    ))
                })?;
                std::fs::rename(&tmp_path, &final_path).map_err(|e| {
                    crate::Error::new(format!(
                        "failed to rename manifest {} -> {}: {e}",
                        tmp_path.display(),
                        final_path.display()
                    ))
                })?;
                Ok(())
            }
            Self::S3(_) => {
                // S3 ではセグメントと同じ PUT で上書きする
                self.write_segment(filename, data).await
            }
        }
    }

    /// ファイルを削除する（ベストエフォート）
    async fn delete_file(&self, filename: &str) {
        match self {
            Self::Filesystem(fs) => {
                let path = fs.output_directory.join(filename);
                if let Err(e) = std::fs::remove_file(&path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!("failed to delete {}: {e}", path.display());
                }
            }
            Self::S3(s3) => {
                let key = s3.object_key(filename);
                let request = match s3
                    .client
                    .client()
                    .delete_object()
                    .bucket(&s3.bucket)
                    .key(&key)
                    .build_request()
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("failed to build S3 DeleteObject request for {key}: {e}");
                        return;
                    }
                };
                match s3.client.execute(&request).await {
                    Ok(response) => {
                        s3.delete_counts.record(response.status_code);
                    }
                    Err(e) => {
                        s3.delete_error_count.inc();
                        tracing::warn!("S3 DeleteObject failed for {key}: {}", e.display());
                    }
                }
            }
        }
    }
}

/// MPD マニフェストファイル名
const MANIFEST_FILENAME: &str = "manifest.mpd";
/// fMP4 の init segment ファイル名
const INIT_SEGMENT_FILENAME: &str = "init.mp4";

/// fMP4 用のタイムスケール（マイクロ秒単位）
const FMP4_TIMESCALE: NonZeroU32 = NonZeroU32::MIN.saturating_add(1_000_000 - 1);

/// DASH セグメントライター。
/// エンコード済みの H.264 + AAC フレームを fMP4 セグメントに分割し、
/// MPD マニフェストを管理する。
struct DashWriter {
    storage: DashStorage,
    segment_duration_target: f64,
    max_retained_segments: usize,
    segment_sequence: u64,
    retained_segments: VecDeque<RetainedDashSegment>,
    // fMP4 状態
    muxer: shiguredo_mp4::mux::Fmp4SegmentMuxer,
    init_segment_written: bool,
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
    /// 現在のセグメントの共通情報
    current_segment_info: Option<CurrentSegmentInfo>,
    /// MPD の availabilityStartTime（ライター起動時の UTC 時刻）
    availability_start_time: String,
    /// ABR 時は結合 MPD を coordinator が書き出すため、ライター側では MPD を書かない
    skip_mpd: bool,
    stats: DashWriterStats,
}

/// セグメントの共通情報
struct CurrentSegmentInfo {
    filename: String,
    start_timestamp: Duration,
    last_timestamp: Duration,
}

#[derive(Debug)]
struct RetainedDashSegment {
    filename: String,
    duration: f64,
    /// セグメント開始時刻（マイクロ秒単位、timescale と同じ）
    start_time_us: u64,
    /// セグメント長（マイクロ秒単位、timescale と同じ）
    duration_us: u64,
}

/// 現在の UTC 時刻を ISO 8601 形式（秒精度）で返す
fn format_utc_now() -> String {
    let now = std::time::SystemTime::now();
    let since_epoch = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let secs = since_epoch.as_secs();

    // 日時の分解
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // グレゴリオ暦の計算（Unix epoch = 1970-01-01 からの日数）
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Unix epoch からの日数を (year, month, day) に変換する
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // アルゴリズム: https://howardhinnant.github.io/date_algorithms.html#civil_from_days
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

impl DashWriter {
    fn new(
        storage: DashStorage,
        segment_duration_target: f64,
        max_retained_segments: usize,
        skip_mpd: bool,
        stats: DashWriterStats,
    ) -> crate::Result<Self> {
        let muxer = shiguredo_mp4::mux::Fmp4SegmentMuxer::new()
            .map_err(|e| crate::Error::new(format!("failed to create fMP4 segment muxer: {e}")))?;

        Ok(Self {
            storage,
            segment_duration_target,
            max_retained_segments,
            segment_sequence: 0,
            retained_segments: VecDeque::new(),
            muxer,
            init_segment_written: false,
            current_samples: Vec::new(),
            current_payload: Vec::new(),
            last_video_timestamp: None,
            last_audio_timestamp: None,
            last_video_sample_entry: None,
            last_audio_sample_entry: None,
            current_segment_info: None,
            availability_start_time: format_utc_now(),
            skip_mpd,
            stats,
        })
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

        handle.notify_ready();

        // 起動直後に上流 video encoder へキーフレーム要求を送る
        if let Err(e) = crate::encoder::request_upstream_video_keyframe(
            &handle.pipeline_handle(),
            handle.processor_id(),
            "dash_writer_start",
        )
        .await
        {
            tracing::warn!(
                "failed to request keyframe for DASH writer start: {}",
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
                                tracing::warn!("DASH audio frame error: {}", e.display());
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
                                tracing::warn!("DASH video frame error: {}", e.display());
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
        if let Err(e) = self.finalize_current_segment().await {
            tracing::warn!("DASH finalize error on EOS: {}", e.display());
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

        // sample_entry が来たら保持する（エンコーダーは初回のみ付与する場合がある）
        if frame.sample_entry.is_some() {
            self.last_video_sample_entry.clone_from(&frame.sample_entry);
        }
        // 前のビデオサンプルの duration を確定させる
        if let Some(prev_ts) = self.last_video_timestamp {
            // Duration::as_micros() は u128 を返すが、fMP4 の sample duration は u32。
            // フレーム間隔が約 71.5 分（u32::MAX μs ≈ 4294 秒）を超えると
            // as u32 キャストで黙って切り捨てられ、誤った duration が書かれる。
            // 通常のライブストリーミングではまず発生しないが、ネットワーク断や
            // ソースの一時停止からの復帰で理論上は起こり得る。
            // 発生した場合、該当セグメントの再生タイミングがずれるが、
            // 次のキーフレームでセグメントが切り替わるため影響は限定的。
            // ここでは u32::MAX にサチュレーションして安全側に倒す。
            let duration =
                frame.timestamp.saturating_sub(prev_ts).as_micros().min(u32::MAX as u128) as u32;
            if let Some(last) = self
                .current_samples
                .iter_mut()
                .rfind(|s| s.track_kind == shiguredo_mp4::TrackKind::Video)
            {
                last.duration = duration;
            }
        }
        let data_offset = self.current_payload.len() as u64;
        self.current_payload.extend_from_slice(&frame.data);
        // フレームの sample_entry が None なら保持済みの値を使う
        let sample_entry = frame
            .sample_entry
            .clone()
            .or_else(|| self.last_video_sample_entry.clone());
        self.current_samples.push(shiguredo_mp4::mux::Sample {
            track_kind: shiguredo_mp4::TrackKind::Video,
            sample_entry,
            keyframe: frame.keyframe,
            timescale: FMP4_TIMESCALE,
            duration: 0,
            composition_time_offset: None,
            data_offset,
            data_size: frame.data.len(),
        });
        self.last_video_timestamp = Some(frame.timestamp);

        if let Some(ref mut info) = self.current_segment_info {
            info.last_timestamp = frame.timestamp;
        }

        Ok(())
    }

    /// オーディオフレーム処理
    async fn handle_audio_frame(&mut self, frame: &crate::AudioFrame) -> crate::Result<()> {
        self.stats.total_input_audio_frame_count.inc();
        // 最初の video keyframe より前に audio が流れ始めることがある。
        // その場合でも、初回だけ付与される sample_entry は保持しておく。
        if frame.sample_entry.is_some() {
            self.last_audio_sample_entry.clone_from(&frame.sample_entry);
        }

        if self.current_segment_info.is_none() {
            return Ok(());
        }

        // 前のオーディオサンプルの duration を確定させる（サチュレーションについてはビデオ側のコメント参照）
        if let Some(prev_ts) = self.last_audio_timestamp {
            let duration =
                frame.timestamp.saturating_sub(prev_ts).as_micros().min(u32::MAX as u128) as u32;
            if let Some(last) = self
                .current_samples
                .iter_mut()
                .rfind(|s| s.track_kind == shiguredo_mp4::TrackKind::Audio)
            {
                last.duration = duration;
            }
        }
        let data_offset = self.current_payload.len() as u64;
        self.current_payload.extend_from_slice(&frame.data);
        let sample_entry = frame
            .sample_entry
            .clone()
            .or_else(|| self.last_audio_sample_entry.clone());
        self.current_samples.push(shiguredo_mp4::mux::Sample {
            track_kind: shiguredo_mp4::TrackKind::Audio,
            sample_entry,
            keyframe: true,
            timescale: FMP4_TIMESCALE,
            duration: 0,
            composition_time_offset: None,
            data_offset,
            data_size: frame.data.len(),
        });
        self.last_audio_timestamp = Some(frame.timestamp);

        if let Some(ref mut info) = self.current_segment_info {
            info.last_timestamp = frame.timestamp;
        }

        Ok(())
    }

    /// 新しいセグメントを開始する
    fn start_new_segment(&mut self, timestamp: Duration) -> crate::Result<()> {
        let sequence = self.segment_sequence;
        self.segment_sequence += 1;
        let filename = format!("segment-{sequence:06}.m4s");

        // samples と payload をクリアして蓄積開始
        self.current_samples.clear();
        self.current_payload.clear();

        self.current_segment_info = Some(CurrentSegmentInfo {
            filename,
            start_timestamp: timestamp,
            last_timestamp: timestamp,
        });

        Ok(())
    }

    /// 現在のセグメントを完了し、MPD マニフェストを更新する
    async fn finalize_current_segment(&mut self) -> crate::Result<()> {
        let Some(info) = self.current_segment_info.take() else {
            return Ok(());
        };

        if self.current_samples.is_empty() {
            return Ok(());
        }

        // muxer は各トラックの最初の sample に sample_entry があることを要求する。
        // エンコーダーは sample_entry を最初のフレームにしか付けないため、
        // セグメント開始直後のタイミング次第では current_samples 側で欠落し得る。
        // ここで最後に観測した sample_entry から補完しておく。
        fill_missing_sample_entries(
            &mut self.current_samples,
            &self.last_video_sample_entry,
            &self.last_audio_sample_entry,
        );

        // 末尾サンプルの duration を補完する
        fixup_last_sample_duration(&mut self.current_samples);

        // mdat payload をトラックごとに連続配置し、data_offset を再計算する
        let reordered_payload =
            reorder_payload_by_track(&mut self.current_samples, &self.current_payload);

        // moof + mdat ヘッダを生成
        let metadata = self
            .muxer
            .create_media_segment_metadata(&self.current_samples)
            .map_err(|e| {
                crate::Error::new(format!("failed to create fMP4 segment metadata: {e}"))
            })?;

        // init segment がまだ書かれていなければ書き出す
        if !self.init_segment_written {
            let init_bytes = self.muxer.init_segment_bytes().map_err(|e| {
                crate::Error::new(format!("failed to create fMP4 init segment: {e}"))
            })?;
            self.storage
                .write_segment(INIT_SEGMENT_FILENAME, &init_bytes)
                .await?;
            self.init_segment_written = true;
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

        self.current_samples.clear();
        self.current_payload.clear();

        let duration = info
            .last_timestamp
            .saturating_sub(info.start_timestamp)
            .as_secs_f64();
        let duration = duration.max(0.001);

        let start_time_us = info.start_timestamp.as_micros() as u64;
        let duration_us = info
            .last_timestamp
            .saturating_sub(info.start_timestamp)
            .as_micros() as u64;
        let duration_us = duration_us.max(1000); // 最低 1ms

        self.stats.total_segment_count.inc();

        self.retained_segments.push_back(RetainedDashSegment {
            filename: info.filename,
            duration,
            start_time_us,
            duration_us,
        });

        // 保持数超過分の古いセグメントを先に削除してからマニフェストを書き出す
        while self.retained_segments.len() > self.max_retained_segments {
            if let Some(old) = self.retained_segments.pop_front() {
                self.storage.delete_file(&old.filename).await;
                self.stats.total_deleted_segment_count.inc();
            }
        }

        self.stats
            .current_retained_segment_count
            .set(self.retained_segments.len() as i64);

        if !self.skip_mpd {
            self.write_mpd().await?;
        }

        Ok(())
    }

    /// MPD マニフェストを書き出す
    async fn write_mpd(&self) -> crate::Result<()> {
        if self.retained_segments.is_empty() {
            return Ok(());
        }

        let timescale = FMP4_TIMESCALE.get() as u64;

        // SegmentTimeline エントリを構築
        let timeline: Vec<shiguredo_mpd::TimelineEntry> = self
            .retained_segments
            .iter()
            .map(|seg| shiguredo_mpd::TimelineEntry {
                t: Some(seg.start_time_us),
                d: seg.duration_us,
                r: 0,
                k: None,
            })
            .collect();

        // timeShiftBufferDepth = 保持セグメント合計尺
        let buffer_depth: f64 = self.retained_segments.iter().map(|s| s.duration).sum();

        let start_number = self.segment_sequence - self.retained_segments.len() as u64;

        let mpd = shiguredo_mpd::Mpd {
            id: None,
            presentation_type: shiguredo_mpd::PresentationType::Dynamic,
            media_presentation_duration: None,
            min_buffer_time: self.segment_duration_target,
            minimum_update_period: Some(self.segment_duration_target),
            availability_start_time: Some(self.availability_start_time.clone()),
            availability_end_time: None,
            time_shift_buffer_depth: Some(buffer_depth),
            suggested_presentation_delay: Some(self.segment_duration_target * 2.0),
            publish_time: None,
            max_segment_duration: Some(self.segment_duration_target),
            max_subsegment_duration: None,
            profiles: "urn:mpeg:dash:profile:isoff-live:2011".to_owned(),
            base_urls: Vec::new(),
            utc_timings: Vec::new(),
            locations: Vec::new(),
            service_descriptions: Vec::new(),
            content_steering: None,
            patch_locations: Vec::new(),
            essential_properties: Vec::new(),
            supplemental_properties: Vec::new(),
            metrics: Vec::new(),
            periods: vec![shiguredo_mpd::Period {
                id: Some("0".to_owned()),
                start: Some(0.0),
                duration: None,
                xlink_href: None,
                xlink_actuate: None,
                base_urls: Vec::new(),
                supplemental_properties: Vec::new(),
                essential_properties: Vec::new(),
                asset_identifier: None,
                event_streams: Vec::new(),
                preselections: Vec::new(),
                subsets: Vec::new(),
                segment_base: None,
                segment_list: None,
                segment_template: None,
                adaptation_sets: vec![shiguredo_mpd::AdaptationSet {
                    id: Some(0),
                    mime_type: Some("video/mp4".to_owned()),
                    // TODO: エンコーダーの SPS から正確な profile/level を取得すべきだが、
                    // マニフェスト生成時点ではその情報がないため暫定値を使用する。
                    // avc1.42e01f = H.264 Baseline Profile Level 3.1, mp4a.40.2 = AAC-LC
                    codecs: Some("avc1.42e01f,mp4a.40.2".to_owned()),
                    content_type: Some(shiguredo_mpd::ContentType::Video),
                    lang: None,
                    width: None,
                    height: None,
                    frame_rate: None,
                    min_width: None,
                    min_height: None,
                    min_frame_rate: None,
                    min_bandwidth: None,
                    max_width: None,
                    max_height: None,
                    max_frame_rate: None,
                    max_bandwidth: None,
                    audio_sampling_rate: None,
                    par: None,
                    sar: None,
                    profiles: None,
                    scan_type: None,
                    start_with_sap: Some(1),
                    max_playout_rate: None,
                    selection_priority: None,
                    supplemental_codecs: None,
                    maximum_sap_period: None,
                    segment_profiles: None,
                    coding_dependency: None,
                    segment_alignment: true,
                    subsegment_alignment: false,
                    bitstream_switching: None,
                    base_urls: Vec::new(),
                    roles: Vec::new(),
                    accessibilities: Vec::new(),
                    audio_channel_configurations: Vec::new(),
                    labels: Vec::new(),
                    group_labels: Vec::new(),
                    essential_properties: Vec::new(),
                    supplemental_properties: Vec::new(),
                    viewpoints: Vec::new(),
                    frame_packings: Vec::new(),
                    inband_event_streams: Vec::new(),
                    producer_reference_times: Vec::new(),
                    content_components: Vec::new(),
                    segment_sequence_properties: Vec::new(),
                    event_streams: Vec::new(),
                    content_protections: Vec::new(),
                    segment_base: None,
                    segment_list: None,
                    segment_template: Some(shiguredo_mpd::SegmentTemplate {
                        media: Some("segment-$Number%06d$.m4s".to_owned()),
                        initialization: Some("init.mp4".to_owned()),
                        index: None,
                        timescale,
                        duration: None,
                        start_number,
                        end_number: None,
                        presentation_time_offset: 0,
                        availability_time_offset: None,
                        availability_time_complete: None,
                        bitstream_switching_source_url: None,
                        bitstream_switching_range: None,
                        segment_timeline: Some(timeline),
                    }),
                    representations: vec![shiguredo_mpd::Representation {
                        id: "0".to_owned(),
                        bandwidth: 0, // 正確な帯域幅は不明だが必須フィールド
                        width: None,
                        height: None,
                        codecs: None,
                        frame_rate: None,
                        audio_sampling_rate: None,
                        mime_type: None,
                        sar: None,
                        quality_ranking: None,
                        dependency_id: None,
                        max_playout_rate: None,
                        scan_type: None,
                        start_with_sap: None,
                        profiles: None,
                        coding_dependency: None,
                        supplemental_codecs: None,
                        codec_private_data: None,
                        media_stream_structure_id: None,
                        maximum_sap_period: None,
                        segment_profiles: None,
                        base_urls: Vec::new(),
                        audio_channel_configurations: Vec::new(),
                        essential_properties: Vec::new(),
                        supplemental_properties: Vec::new(),
                        frame_packings: Vec::new(),
                        inband_event_streams: Vec::new(),
                        producer_reference_times: Vec::new(),
                        segment_sequence_properties: Vec::new(),
                        sub_representations: Vec::new(),
                        content_protections: Vec::new(),
                        segment_base: None,
                        segment_list: None,
                        segment_template: None,
                    }],
                }],
            }],
        };

        let xml = shiguredo_mpd::write(&mpd);
        self.storage
            .write_manifest(MANIFEST_FILENAME, xml.as_bytes())
            .await?;

        Ok(())
    }

    /// 停止時に全生成ファイルを削除する
    async fn cleanup(&self) {
        // ABR 時は結合 MPD を coordinator が管理するため、ライター側では MPD を削除しない
        if !self.skip_mpd {
            self.storage.delete_file(MANIFEST_FILENAME).await;

            // filesystem の場合のみ一時ファイルも削除する
            if let DashStorage::Filesystem(fs) = &self.storage {
                let tmp_path = fs
                    .output_directory
                    .join(format!(".{MANIFEST_FILENAME}.tmp"));
                let _ = std::fs::remove_file(&tmp_path);
            }
        }

        // init segment を削除
        self.storage.delete_file(INIT_SEGMENT_FILENAME).await;

        for seg in &self.retained_segments {
            self.storage.delete_file(&seg.filename).await;
        }
    }
}

// --- fMP4 ヘルパー関数群 ---

/// mdat payload をトラックごとに連続配置し、samples の data_offset を再計算する。
/// Fmp4SegmentMuxer は同一トラックの sample data が mdat 内で連続していることを要求する。
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
        return;
    }

    let last_idx = track_samples[track_samples.len() - 1];
    if samples[last_idx].duration == 0 {
        let prev_idx = track_samples[track_samples.len() - 2];
        samples[last_idx].duration = samples[prev_idx].duration;
    }
}

// --- 公開 API ---

pub enum DashStorageConfig {
    /// ローカルファイルシステム
    Filesystem { output_directory: PathBuf },
    /// S3 互換オブジェクトストレージ
    S3 {
        client: crate::s3::S3HttpClient,
        bucket: String,
        prefix: String,
    },
}

pub struct DashWriterConfig {
    pub storage: DashStorageConfig,
    pub input_audio_track_id: crate::TrackId,
    pub input_video_track_id: crate::TrackId,
    pub segment_duration: f64,
    pub max_retained_segments: usize,
    /// ABR 時は結合 MPD を coordinator が書き出すため、ライター側では MPD を書かない
    pub skip_mpd: bool,
}

/// DASH writer プロセッサを作成する
pub async fn create_processor(
    handle: &crate::MediaPipelineHandle,
    config: DashWriterConfig,
    processor_id: Option<crate::ProcessorId>,
) -> crate::Result<crate::ProcessorId> {
    let processor_id = processor_id.unwrap_or_else(|| crate::ProcessorId::new("dashWriter"));
    handle
        .spawn_processor(
            processor_id.clone(),
            crate::ProcessorMetadata::new("dash_writer"),
            move |h| async move {
                let mut stats = h.stats();
                let writer_stats = DashWriterStats::new(&mut stats);
                let storage = match config.storage {
                    DashStorageConfig::Filesystem { output_directory } => {
                        DashStorage::Filesystem(FilesystemStorage { output_directory })
                    }
                    DashStorageConfig::S3 {
                        client,
                        bucket,
                        prefix,
                    } => DashStorage::S3(Box::new(S3Storage {
                        client,
                        bucket,
                        prefix,
                        put_counts: S3StatusCounters::new(&stats, "total_s3_put_count"),
                        delete_counts: S3StatusCounters::new(&stats, "total_s3_delete_count"),
                        put_error_count: stats.clone().counter("total_s3_put_error_count"),
                        delete_error_count: stats.clone().counter("total_s3_delete_error_count"),
                    })),
                };
                let writer = DashWriter::new(
                    storage,
                    config.segment_duration,
                    config.max_retained_segments,
                    config.skip_mpd,
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

/// 結合 MPD のバリアント情報
pub struct CombinedMpdVariant {
    /// バリアントの合計帯域幅（ビデオ + オーディオ、bps）
    pub bandwidth: u64,
    /// ビデオ幅
    pub width: u32,
    /// ビデオ高さ
    pub height: u32,
    /// セグメントファイルのパステンプレート（例: "variant_0/segment-$Number%06d$.m4s"）
    pub media_path: String,
    /// init segment のパス（例: "variant_0/init.mp4"）
    pub init_path: String,
}

/// ABR 用の結合 MPD の内容を生成する。
/// duration ベースの SegmentTemplate を使い、各 Representation にバリアント固有のパスを設定する。
pub fn build_combined_mpd_content(
    variants: &[CombinedMpdVariant],
    segment_duration: f64,
    max_retained_segments: usize,
) -> String {
    let timescale = FMP4_TIMESCALE.get() as u64;
    let duration_scaled = (segment_duration * timescale as f64).round() as u64;
    let buffer_depth = segment_duration * max_retained_segments as f64;

    let representations: Vec<shiguredo_mpd::Representation> = variants
        .iter()
        .enumerate()
        .map(|(i, v)| shiguredo_mpd::Representation {
            id: format!("variant_{i}"),
            bandwidth: v.bandwidth,
            width: Some(v.width),
            height: Some(v.height),
            codecs: None,
            frame_rate: None,
            audio_sampling_rate: None,
            mime_type: None,
            sar: None,
            quality_ranking: None,
            dependency_id: None,
            max_playout_rate: None,
            scan_type: None,
            start_with_sap: None,
            profiles: None,
            coding_dependency: None,
            supplemental_codecs: None,
            codec_private_data: None,
            media_stream_structure_id: None,
            maximum_sap_period: None,
            segment_profiles: None,
            base_urls: Vec::new(),
            audio_channel_configurations: Vec::new(),
            essential_properties: Vec::new(),
            supplemental_properties: Vec::new(),
            frame_packings: Vec::new(),
            inband_event_streams: Vec::new(),
            producer_reference_times: Vec::new(),
            segment_sequence_properties: Vec::new(),
            sub_representations: Vec::new(),
            content_protections: Vec::new(),
            segment_base: None,
            segment_list: None,
            segment_template: Some(shiguredo_mpd::SegmentTemplate {
                media: Some(v.media_path.clone()),
                initialization: Some(v.init_path.clone()),
                index: None,
                timescale,
                duration: Some(duration_scaled),
                start_number: 0,
                end_number: None,
                presentation_time_offset: 0,
                availability_time_offset: None,
                availability_time_complete: None,
                bitstream_switching_source_url: None,
                bitstream_switching_range: None,
                segment_timeline: None,
            }),
        })
        .collect();

    let mpd = shiguredo_mpd::Mpd {
        id: None,
        presentation_type: shiguredo_mpd::PresentationType::Dynamic,
        media_presentation_duration: None,
        min_buffer_time: segment_duration,
        minimum_update_period: Some(segment_duration),
        availability_start_time: Some(format_utc_now()),
        availability_end_time: None,
        time_shift_buffer_depth: Some(buffer_depth),
        suggested_presentation_delay: Some(segment_duration * 2.0),
        publish_time: None,
        max_segment_duration: Some(segment_duration),
        max_subsegment_duration: None,
        profiles: "urn:mpeg:dash:profile:isoff-live:2011".to_owned(),
        base_urls: Vec::new(),
        utc_timings: Vec::new(),
        locations: Vec::new(),
        service_descriptions: Vec::new(),
        content_steering: None,
        patch_locations: Vec::new(),
        essential_properties: Vec::new(),
        supplemental_properties: Vec::new(),
        metrics: Vec::new(),
        periods: vec![shiguredo_mpd::Period {
            id: Some("0".to_owned()),
            start: Some(0.0),
            duration: None,
            xlink_href: None,
            xlink_actuate: None,
            base_urls: Vec::new(),
            supplemental_properties: Vec::new(),
            essential_properties: Vec::new(),
            asset_identifier: None,
            event_streams: Vec::new(),
            preselections: Vec::new(),
            subsets: Vec::new(),
            segment_base: None,
            segment_list: None,
            segment_template: None,
            adaptation_sets: vec![shiguredo_mpd::AdaptationSet {
                id: Some(0),
                mime_type: Some("video/mp4".to_owned()),
                codecs: Some("avc1.42e01f,mp4a.40.2".to_owned()),
                content_type: Some(shiguredo_mpd::ContentType::Video),
                lang: None,
                width: None,
                height: None,
                frame_rate: None,
                min_width: None,
                min_height: None,
                min_frame_rate: None,
                min_bandwidth: None,
                max_width: None,
                max_height: None,
                max_frame_rate: None,
                max_bandwidth: None,
                audio_sampling_rate: None,
                par: None,
                sar: None,
                profiles: None,
                scan_type: None,
                start_with_sap: Some(1),
                max_playout_rate: None,
                selection_priority: None,
                supplemental_codecs: None,
                maximum_sap_period: None,
                segment_profiles: None,
                coding_dependency: None,
                segment_alignment: true,
                subsegment_alignment: false,
                bitstream_switching: None,
                base_urls: Vec::new(),
                roles: Vec::new(),
                accessibilities: Vec::new(),
                audio_channel_configurations: Vec::new(),
                labels: Vec::new(),
                group_labels: Vec::new(),
                essential_properties: Vec::new(),
                supplemental_properties: Vec::new(),
                viewpoints: Vec::new(),
                frame_packings: Vec::new(),
                inband_event_streams: Vec::new(),
                producer_reference_times: Vec::new(),
                content_components: Vec::new(),
                segment_sequence_properties: Vec::new(),
                event_streams: Vec::new(),
                content_protections: Vec::new(),
                segment_base: None,
                segment_list: None,
                segment_template: None,
                representations,
            }],
        }],
    };

    shiguredo_mpd::write(&mpd)
}

/// ABR 用の結合 MPD をファイルシステムに書き出す。
/// 一時ファイルに書いてから rename してアトミックに更新する。
pub fn write_combined_mpd(
    output_directory: &std::path::Path,
    variants: &[CombinedMpdVariant],
    segment_duration: f64,
    max_retained_segments: usize,
) -> crate::Result<()> {
    let content = build_combined_mpd_content(variants, segment_duration, max_retained_segments);

    let mpd_path = output_directory.join(MANIFEST_FILENAME);
    let tmp_path = output_directory.join(format!(".{MANIFEST_FILENAME}.tmp"));

    std::fs::write(&tmp_path, content.as_bytes()).map_err(|e| {
        crate::Error::new(format!(
            "failed to write temporary combined MPD {}: {e}",
            tmp_path.display()
        ))
    })?;

    std::fs::rename(&tmp_path, &mpd_path).map_err(|e| {
        crate::Error::new(format!(
            "failed to rename combined MPD {} -> {}: {e}",
            tmp_path.display(),
            mpd_path.display()
        ))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- 結合 MPD 生成テスト ---

    #[test]
    fn combined_mpd_contains_multiple_representations() {
        let variants = vec![
            CombinedMpdVariant {
                bandwidth: 2_128_000,
                width: 1920,
                height: 1080,
                media_path: "variant_0/segment-$Number%06d$.m4s".to_owned(),
                init_path: "variant_0/init.mp4".to_owned(),
            },
            CombinedMpdVariant {
                bandwidth: 1_064_000,
                width: 1280,
                height: 720,
                media_path: "variant_1/segment-$Number%06d$.m4s".to_owned(),
                init_path: "variant_1/init.mp4".to_owned(),
            },
        ];
        let xml = build_combined_mpd_content(&variants, 2.0, 6);
        let mpd = shiguredo_mpd::parse(&xml).expect("combined MPD must be valid XML");

        assert_eq!(
            mpd.presentation_type,
            shiguredo_mpd::PresentationType::Dynamic
        );
        assert_eq!(mpd.periods.len(), 1);

        let period = &mpd.periods[0];
        assert_eq!(period.adaptation_sets.len(), 1);

        let adaptation_set = &period.adaptation_sets[0];
        assert_eq!(adaptation_set.representations.len(), 2);

        // バリアント 0 の Representation を検証
        let rep0 = &adaptation_set.representations[0];
        assert_eq!(rep0.id, "variant_0");
        assert_eq!(rep0.bandwidth, 2_128_000);
        assert_eq!(rep0.width, Some(1920));
        assert_eq!(rep0.height, Some(1080));
        let tmpl0 = rep0
            .segment_template
            .as_ref()
            .expect("SegmentTemplate must exist");
        assert_eq!(
            tmpl0.media.as_deref(),
            Some("variant_0/segment-$Number%06d$.m4s")
        );
        assert_eq!(tmpl0.initialization.as_deref(), Some("variant_0/init.mp4"));

        // バリアント 1 の Representation を検証
        let rep1 = &adaptation_set.representations[1];
        assert_eq!(rep1.id, "variant_1");
        assert_eq!(rep1.bandwidth, 1_064_000);
        assert_eq!(rep1.width, Some(1280));
        assert_eq!(rep1.height, Some(720));
        let tmpl1 = rep1
            .segment_template
            .as_ref()
            .expect("SegmentTemplate must exist");
        assert_eq!(
            tmpl1.media.as_deref(),
            Some("variant_1/segment-$Number%06d$.m4s")
        );
        assert_eq!(tmpl1.initialization.as_deref(), Some("variant_1/init.mp4"));
    }

    #[test]
    fn combined_mpd_uses_duration_based_segment_template() {
        let variants = vec![CombinedMpdVariant {
            bandwidth: 2_000_000,
            width: 1920,
            height: 1080,
            media_path: "variant_0/segment-$Number%06d$.m4s".to_owned(),
            init_path: "variant_0/init.mp4".to_owned(),
        }];
        let xml = build_combined_mpd_content(&variants, 2.0, 6);
        let mpd = shiguredo_mpd::parse(&xml).expect("combined MPD must be valid XML");

        let rep = &mpd.periods[0].adaptation_sets[0].representations[0];
        let tmpl = rep
            .segment_template
            .as_ref()
            .expect("SegmentTemplate must exist");

        // duration ベース（SegmentTimeline は使わない）
        let timescale = FMP4_TIMESCALE.get() as u64;
        assert_eq!(tmpl.timescale, timescale);
        assert_eq!(tmpl.duration, Some((2.0 * timescale as f64).round() as u64));
        assert!(tmpl.segment_timeline.is_none());
        assert_eq!(tmpl.start_number, 0);
    }

    #[test]
    fn combined_mpd_time_shift_buffer_depth_matches_config() {
        let variants = vec![CombinedMpdVariant {
            bandwidth: 2_000_000,
            width: 1920,
            height: 1080,
            media_path: "v0/seg-$Number%06d$.m4s".to_owned(),
            init_path: "v0/init.mp4".to_owned(),
        }];
        let xml = build_combined_mpd_content(&variants, 3.0, 5);
        let mpd = shiguredo_mpd::parse(&xml).expect("combined MPD must be valid XML");

        // timeShiftBufferDepth = segment_duration * max_retained_segments
        assert_eq!(mpd.time_shift_buffer_depth, Some(15.0));
        assert_eq!(mpd.min_buffer_time, 3.0);
        assert_eq!(mpd.minimum_update_period, Some(3.0));
    }

    // --- outputPath テスト ---

    #[test]
    fn dash_destination_filesystem_output_path_returns_manifest_mpd() {
        let dest = crate::obsws::input_registry::DashDestination::Filesystem {
            directory: "/tmp/dash-output".to_owned(),
        };
        assert_eq!(dest.output_path(), "/tmp/dash-output/manifest.mpd");
    }

    #[test]
    fn dash_destination_s3_output_path_returns_manifest_mpd() {
        let dest = crate::obsws::input_registry::DashDestination::S3 {
            bucket: "my-bucket".to_owned(),
            prefix: "live/stream1".to_owned(),
            region: "us-east-1".to_owned(),
            endpoint: None,
            use_path_style: false,
            access_key_id: "key".to_owned(),
            secret_access_key: "secret".to_owned(),
            session_token: None,
            lifetime_days: None,
        };
        assert_eq!(
            dest.output_path(),
            "s3://my-bucket/live/stream1/manifest.mpd"
        );
    }

    #[test]
    fn dash_destination_s3_empty_prefix_output_path() {
        let dest = crate::obsws::input_registry::DashDestination::S3 {
            bucket: "my-bucket".to_owned(),
            prefix: String::new(),
            region: "us-east-1".to_owned(),
            endpoint: None,
            use_path_style: false,
            access_key_id: "key".to_owned(),
            secret_access_key: "secret".to_owned(),
            session_token: None,
            lifetime_days: None,
        };
        assert_eq!(dest.output_path(), "s3://my-bucket/manifest.mpd");
    }
}
