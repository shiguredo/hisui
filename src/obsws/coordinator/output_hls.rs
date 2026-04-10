//! HLS ライブ出力の output エンジン。
//! Program 出力を HLS セグメント + M3U8 プレイリストとして出力するための processor 起動・停止を行う。
//! ABR (Adaptive Bitrate) 対応として複数 variant の並行処理を管理する。

use std::time::Duration;

use nojson::DisplayJson as _;

use super::ObswsCoordinator;
use super::ObswsProgramOutputContext;
use super::output::{
    OutputOperationOutcome, build_s3_client, terminate_and_wait, wait_or_terminate,
};
use super::output_registry::{ObswsRecordTrackRun, OutputRun, OutputSettings};
use crate::{ProcessorId, TrackId};

// -----------------------------------------------------------------------
// HLS 設定型
// -----------------------------------------------------------------------

/// HLS 出力の設定。
/// SetOutputSettings で各フィールドを変更可能。
pub const DEFAULT_HLS_SEGMENT_DURATION_SECS: f64 = 2.0;
pub const DEFAULT_HLS_MAX_RETAINED_SEGMENTS: usize = 6;
pub const DEFAULT_HLS_VIDEO_BITRATE_BPS: usize = 2_000_000;
pub const DEFAULT_HLS_AUDIO_BITRATE_BPS: usize = 128_000;

/// HLS ABR のバリアント定義。
/// バリアントごとにビットレートと解像度を指定する。
#[derive(Debug, Clone, PartialEq)]
pub struct HlsVariant {
    /// ビデオビットレート (bps)
    pub video_bitrate_bps: usize,
    /// オーディオビットレート (bps)
    pub audio_bitrate_bps: usize,
    /// ビデオ幅（省略時はミキサーのキャンバスサイズを使用）
    pub width: Option<crate::types::EvenUsize>,
    /// ビデオ高さ（省略時はミキサーのキャンバスサイズを使用）
    pub height: Option<crate::types::EvenUsize>,
}

impl Default for HlsVariant {
    fn default() -> Self {
        Self {
            video_bitrate_bps: DEFAULT_HLS_VIDEO_BITRATE_BPS,
            audio_bitrate_bps: DEFAULT_HLS_AUDIO_BITRATE_BPS,
            width: None,
            height: None,
        }
    }
}

impl nojson::DisplayJson for HlsVariant {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            f.member("videoBitrate", self.video_bitrate_bps)?;
            f.member("audioBitrate", self.audio_bitrate_bps)?;
            if let Some(width) = self.width {
                f.member("width", width.get())?;
            }
            if let Some(height) = self.height {
                f.member("height", height.get())?;
            }
            Ok(())
        })
        .fmt(f)
    }
}

/// HLS セグメントのフォーマット
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HlsSegmentFormat {
    /// MPEG-TS (.ts)
    #[default]
    MpegTs,
    /// Fragmented MP4 (.m4s + init.mp4)
    Fmp4,
}

impl HlsSegmentFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MpegTs => "mpegts",
            Self::Fmp4 => "fmp4",
        }
    }
}

impl std::str::FromStr for HlsSegmentFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mpegts" => Ok(Self::MpegTs),
            "fmp4" => Ok(Self::Fmp4),
            _ => Err(format!(
                "segmentFormat must be \"mpegts\" or \"fmp4\", got \"{s}\""
            )),
        }
    }
}

/// HLS 出力先の設定
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HlsDestination {
    /// ローカルファイルシステムへの出力
    Filesystem { directory: String },
    /// S3 互換オブジェクトストレージへの出力
    S3 {
        bucket: String,
        prefix: String,
        region: String,
        endpoint: Option<String>,
        use_path_style: bool,
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
        /// オブジェクトのライフタイム（日数）。
        /// 指定時はバケットに lifecycle ルールを設定する（セーフティネット用途）。
        /// 明示的な DeleteObject は常に実行する。
        lifetime_days: Option<u32>,
    },
}

impl nojson::DisplayJson for HlsDestination {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::Filesystem { directory } => nojson::object(|f| {
                f.member("type", "filesystem")?;
                f.member("directory", directory)
            })
            .fmt(f),
            Self::S3 {
                bucket,
                prefix,
                region,
                endpoint,
                use_path_style,
                // 認証情報はレスポンスに含めない
                access_key_id: _,
                secret_access_key: _,
                session_token: _,
                lifetime_days,
            } => nojson::object(|f| {
                f.member("type", "s3")?;
                f.member("bucket", bucket)?;
                f.member("prefix", prefix)?;
                f.member("region", region)?;
                if let Some(endpoint) = endpoint {
                    f.member("endpoint", endpoint)?;
                }
                f.member("usePathStyle", *use_path_style)?;
                if let Some(days) = lifetime_days {
                    f.member("lifetimeDays", *days)?;
                }
                Ok(())
            })
            .fmt(f),
        }
    }
}

impl HlsDestination {
    /// state file 用: 認証情報を含めて JSON オブジェクトとして出力する
    pub fn fmt_with_credentials(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::Filesystem { directory } => nojson::object(|f| {
                f.member("type", "filesystem")?;
                f.member("directory", directory)
            })
            .fmt(f),
            Self::S3 {
                bucket,
                prefix,
                region,
                endpoint,
                use_path_style,
                access_key_id,
                secret_access_key,
                session_token,
                lifetime_days,
            } => nojson::object(|f| {
                f.member("type", "s3")?;
                f.member("bucket", bucket)?;
                f.member("prefix", prefix)?;
                f.member("region", region)?;
                if let Some(endpoint) = endpoint {
                    f.member("endpoint", endpoint)?;
                }
                f.member("usePathStyle", *use_path_style)?;
                f.member(
                    "credentials",
                    nojson::object(|f| {
                        f.member("accessKeyId", access_key_id)?;
                        f.member("secretAccessKey", secret_access_key)?;
                        if let Some(token) = session_token {
                            f.member("sessionToken", token)?;
                        }
                        Ok(())
                    }),
                )?;
                if let Some(days) = lifetime_days {
                    f.member("lifetimeDays", *days)?;
                }
                Ok(())
            })
            .fmt(f),
        }
    }

    /// GetOutputStatus 用の outputPath を生成する
    pub fn output_path(&self) -> String {
        match self {
            Self::Filesystem { directory } => {
                let path = std::path::PathBuf::from(directory).join("playlist.m3u8");
                path.display().to_string()
            }
            Self::S3 { bucket, prefix, .. } => {
                if prefix.is_empty() {
                    format!("s3://{bucket}/playlist.m3u8")
                } else {
                    format!("s3://{bucket}/{prefix}/playlist.m3u8")
                }
            }
        }
    }

    /// バリアント用のサブパスを生成する
    pub fn variant_path(&self, index: usize) -> String {
        match self {
            Self::Filesystem { directory } => std::path::PathBuf::from(directory)
                .join(format!("variant_{index}"))
                .display()
                .to_string(),
            Self::S3 { prefix, .. } => {
                if prefix.is_empty() {
                    format!("variant_{index}")
                } else {
                    format!("{prefix}/variant_{index}")
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObswsHlsSettings {
    // StartOutput 時に必須。登録時点では未指定も許容する。
    pub destination: Option<HlsDestination>,
    /// セグメントの目標尺（秒）
    pub segment_duration: f64,
    /// プレイリストに保持するセグメント数
    pub max_retained_segments: usize,
    /// セグメントフォーマット
    pub segment_format: HlsSegmentFormat,
    /// ABR バリアント定義。
    /// 要素が 1 つの場合は non-ABR（マスタープレイリストを生成しない）。
    pub variants: Vec<HlsVariant>,
}

impl Default for ObswsHlsSettings {
    fn default() -> Self {
        Self {
            destination: None,
            segment_duration: DEFAULT_HLS_SEGMENT_DURATION_SECS,
            max_retained_segments: DEFAULT_HLS_MAX_RETAINED_SEGMENTS,
            segment_format: HlsSegmentFormat::default(),
            variants: vec![HlsVariant::default()],
        }
    }
}

impl nojson::DisplayJson for ObswsHlsSettings {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        nojson::object(|f| {
            if let Some(destination) = &self.destination {
                f.member("destination", destination)?;
            }
            f.member("segmentDuration", self.segment_duration)?;
            f.member("maxRetainedSegments", self.max_retained_segments)?;
            f.member("segmentFormat", self.segment_format.as_str())?;
            f.member(
                "variants",
                nojson::array(|f| {
                    for variant in &self.variants {
                        f.element(variant)?;
                    }
                    Ok(())
                }),
            )
        })
        .fmt(f)
    }
}

// -----------------------------------------------------------------------
// Run 型
// -----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsHlsRun {
    pub destination: HlsDestination,
    /// バリアントごとの実行情報
    pub variant_runs: Vec<ObswsHlsVariantRun>,
}

impl ObswsHlsRun {
    /// ABR（マスタープレイリストあり）かどうかを返す
    pub fn is_abr(&self) -> bool {
        self.variant_runs.len() > 1
    }
}

/// HLS ABR バリアントごとの実行情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObswsHlsVariantRun {
    pub video: ObswsRecordTrackRun,
    pub audio: ObswsRecordTrackRun,
    /// 解像度変換が必要なバリアントのスケーラープロセッサ ID
    pub scaler_processor_id: Option<ProcessorId>,
    /// スケーラー出力トラック ID
    pub scaled_track_id: Option<TrackId>,
    pub writer_processor_id: ProcessorId,
    /// バリアントの出力パス（filesystem: ディレクトリパス、S3: prefix）
    pub variant_path: String,
}

// -----------------------------------------------------------------------
// 設定パース
// -----------------------------------------------------------------------

/// HLS 設定をパースして新しい `ObswsHlsSettings` を返す。
/// 省略されたフィールドは existing の値を維持する。
pub(crate) fn parse_hls_settings_update(
    output_settings: &nojson::RawJsonValue<'_, '_>,
    existing: &ObswsHlsSettings,
) -> Result<ObswsHlsSettings, String> {
    parse_hls_settings_inner(*output_settings, existing)
}

fn parse_hls_settings_inner(
    output_settings: nojson::RawJsonValue<'_, '_>,
    existing: &ObswsHlsSettings,
) -> Result<ObswsHlsSettings, String> {
    // destination オブジェクトのパース
    let destination: Option<HlsDestination> = if let Some(dest_value) = output_settings
        .to_member("destination")
        .map_err(|e| e.to_string())?
        .optional()
    {
        let dest_type: String = dest_value
            .to_member("type")
            .map_err(|e| e.to_string())?
            .required()
            .map_err(|_| "destination.type is required".to_owned())?
            .try_into()
            .map_err(|e: nojson::JsonParseError| e.to_string())?;

        match dest_type.as_str() {
            "filesystem" => {
                let directory: String = dest_value
                    .to_member("directory")
                    .map_err(|e| e.to_string())?
                    .required()
                    .map_err(|_| "destination.directory is required for filesystem".to_owned())?
                    .try_into()
                    .map_err(|e: nojson::JsonParseError| e.to_string())?;
                if directory.is_empty() {
                    return Err("destination.directory must not be empty".to_owned());
                }
                Some(HlsDestination::Filesystem { directory })
            }
            "s3" => {
                let parsed = super::output_registry::parse_obsws_s3_destination(dest_value)?;
                Some(HlsDestination::S3 {
                    bucket: parsed.bucket,
                    prefix: parsed.prefix,
                    region: parsed.region,
                    endpoint: parsed.endpoint,
                    use_path_style: parsed.use_path_style,
                    access_key_id: parsed.access_key_id,
                    secret_access_key: parsed.secret_access_key,
                    session_token: parsed.session_token,
                    lifetime_days: parsed.lifetime_days,
                })
            }
            _ => {
                return Err(format!(
                    "destination.type must be \"filesystem\" or \"s3\", got \"{dest_type}\""
                ));
            }
        }
    } else {
        None
    };

    let segment_duration: Option<f64> = output_settings
        .to_member("segmentDuration")
        .map_err(|e| e.to_string())?
        .optional()
        .map(|v| v.try_into())
        .transpose()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;

    let max_retained_segments: Option<usize> = output_settings
        .to_member("maxRetainedSegments")
        .map_err(|e| e.to_string())?
        .optional()
        .map(|v| v.try_into())
        .transpose()
        .map_err(|e: nojson::JsonParseError| e.to_string())?;

    let segment_format_str: Option<String> =
        super::output_registry::optional_non_empty_string_member(output_settings, "segmentFormat")
            .map_err(|e| e.to_string())?;

    // variants 配列のパース
    let variants: Option<Vec<HlsVariant>> = if let Some(variants_value) = output_settings
        .to_member("variants")
        .map_err(|e| e.to_string())?
        .optional()
    {
        let mut variants = Vec::new();
        for item in variants_value.to_array().map_err(|e| e.to_string())? {
            let video_bitrate: usize = item
                .to_member("videoBitrate")
                .map_err(|e| e.to_string())?
                .required()
                .map_err(|_| "variants[].videoBitrate is required".to_owned())?
                .try_into()
                .map_err(|e: nojson::JsonParseError| e.to_string())?;
            let audio_bitrate: usize = item
                .to_member("audioBitrate")
                .map_err(|e| e.to_string())?
                .required()
                .map_err(|_| "variants[].audioBitrate is required".to_owned())?
                .try_into()
                .map_err(|e: nojson::JsonParseError| e.to_string())?;
            let width: Option<usize> = item
                .to_member("width")
                .map_err(|e| e.to_string())?
                .optional()
                .map(|v| v.try_into())
                .transpose()
                .map_err(|e: nojson::JsonParseError| e.to_string())?;
            let height: Option<usize> = item
                .to_member("height")
                .map_err(|e| e.to_string())?
                .optional()
                .map(|v| v.try_into())
                .transpose()
                .map_err(|e: nojson::JsonParseError| e.to_string())?;

            if video_bitrate == 0 {
                return Err("variants[].videoBitrate must be positive".to_owned());
            }
            if audio_bitrate == 0 {
                return Err("variants[].audioBitrate must be positive".to_owned());
            }
            let width = match width {
                Some(0) => return Err("variants[].width must be positive".to_owned()),
                Some(w) => {
                    Some(crate::types::EvenUsize::new(w).ok_or("variants[].width must be even")?)
                }
                None => None,
            };
            let height = match height {
                Some(0) => return Err("variants[].height must be positive".to_owned()),
                Some(h) => {
                    Some(crate::types::EvenUsize::new(h).ok_or("variants[].height must be even")?)
                }
                None => None,
            };
            // width と height は両方指定するか両方省略する必要がある
            if width.is_some() != height.is_some() {
                return Err(
                    "variants[].width and variants[].height must both be specified or both omitted"
                        .to_owned(),
                );
            }

            variants.push(HlsVariant {
                video_bitrate_bps: video_bitrate,
                audio_bitrate_bps: audio_bitrate,
                width,
                height,
            });
        }
        if variants.is_empty() {
            return Err("variants must not be empty".to_owned());
        }
        Some(variants)
    } else {
        None
    };

    if let Some(duration) = segment_duration
        && duration <= 0.0
    {
        return Err("segmentDuration must be positive".to_owned());
    }
    if let Some(count) = max_retained_segments
        && count == 0
    {
        return Err("maxRetainedSegments must be at least 1".to_owned());
    }
    let segment_format = match segment_format_str {
        Some(ref s) => s.parse::<HlsSegmentFormat>()?,
        None => existing.segment_format,
    };

    Ok(ObswsHlsSettings {
        destination: destination.or(existing.destination.clone()),
        segment_duration: segment_duration.unwrap_or(existing.segment_duration),
        max_retained_segments: max_retained_segments.unwrap_or(existing.max_retained_segments),
        segment_format,
        variants: variants.unwrap_or_else(|| existing.variants.clone()),
    })
}

impl ObswsCoordinator {
    pub(crate) async fn handle_start_hls(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        let Some(output) = self.outputs.get(output_name) else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_RESOURCE_NOT_FOUND,
                    "Output not found",
                ),
            );
        };
        let OutputSettings::Hls(hls_settings) = &output.settings else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Output is not an HLS output",
                ),
            );
        };
        let hls_settings = hls_settings.clone();

        if output.runtime.active {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_RUNNING,
                    "HLS is already active",
                ),
            );
        }
        let Some(ref destination) = hls_settings.destination else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "Missing outputSettings.destination field",
                ),
            );
        };
        if hls_settings.variants.is_empty() {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_INVALID_REQUEST_FIELD,
                    "variants must not be empty",
                ),
            );
        }
        let run_id = self.next_output_run_id;
        self.next_output_run_id = self.next_output_run_id.wrapping_add(1);
        let program_output = ObswsProgramOutputContext {
            video_track_id: self.program_output.video_track_id.clone(),
            audio_track_id: self.program_output.audio_track_id.clone(),
            canvas_width: self.state.canvas_width(),
            canvas_height: self.state.canvas_height(),
            frame_rate: self.state.frame_rate(),
        };
        let is_abr = hls_settings.variants.len() > 1;
        let variant_runs: Vec<ObswsHlsVariantRun> = hls_settings
            .variants
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                let variant_label = format!("v{i}");
                let video = ObswsRecordTrackRun::new(
                    output_name,
                    run_id,
                    &format!("{variant_label}_video"),
                    &program_output.video_track_id,
                );
                let audio = ObswsRecordTrackRun::new(
                    output_name,
                    run_id,
                    &format!("{variant_label}_audio"),
                    &program_output.audio_track_id,
                );
                // variant ごとの fps 調整が必要になった場合は、この後段に映像整形を追加する。
                let needs_scaler = variant.width.zip(variant.height).is_some_and(|(w, h)| {
                    w != program_output.canvas_width || h != program_output.canvas_height
                });
                let scaler_processor_id = if needs_scaler {
                    Some(crate::ProcessorId::new(format!(
                        "output:{output_name}:{variant_label}_scaler:{run_id}"
                    )))
                } else {
                    None
                };
                let scaled_track_id = if needs_scaler {
                    Some(crate::TrackId::new(format!(
                        "output:{output_name}:{variant_label}_scaled_video:{run_id}"
                    )))
                } else {
                    None
                };
                let writer_processor_id = crate::ProcessorId::new(format!(
                    "output:{output_name}:{variant_label}_hls_writer:{run_id}"
                ));
                let variant_path = if is_abr {
                    destination.variant_path(i)
                } else {
                    match destination {
                        HlsDestination::Filesystem { directory } => directory.clone(),
                        HlsDestination::S3 { prefix, .. } => prefix.clone(),
                    }
                };
                ObswsHlsVariantRun {
                    video,
                    audio,
                    scaler_processor_id,
                    scaled_track_id,
                    writer_processor_id,
                    variant_path,
                }
            })
            .collect();
        let run = ObswsHlsRun {
            destination: destination.clone(),
            variant_runs,
        };
        // ランタイム状態を active にする
        if let Some(output) = self.outputs.get_mut(output_name) {
            output.runtime.active = true;
            output.runtime.started_at = Some(std::time::Instant::now());
            output.runtime.run = Some(OutputRun::Hls(run.clone()));
        }
        // filesystem の場合のみ出力ディレクトリを作成する
        if let HlsDestination::Filesystem { directory } = destination
            && let Err(e) = std::fs::create_dir_all(directory)
        {
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
            let error_comment = format!("Failed to create HLS output directory: {e}");
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    &error_comment,
                ),
            );
        }
        // S3 + lifetimeDays 指定時はバケットに lifecycle ルールを設定する
        if let HlsDestination::S3 {
            bucket,
            prefix,
            region,
            endpoint,
            use_path_style,
            access_key_id,
            secret_access_key,
            session_token,
            lifetime_days: Some(days),
        } = destination
        {
            let s3_client = build_s3_client(
                region,
                access_key_id,
                secret_access_key,
                session_token.as_deref(),
                endpoint.as_deref(),
                *use_path_style,
            );
            match s3_client {
                Ok(client) => {
                    // prefix スコープの expiration ルールを設定する
                    let rule_id = format!("hisui-hls-{}", prefix.replace('/', "-"));
                    let rule = shiguredo_s3::types::LifecycleRule {
                        id: Some(rule_id),
                        status: shiguredo_s3::types::ExpirationStatus::Enabled,
                        filter: Some(shiguredo_s3::types::LifecycleRuleFilter {
                            prefix: Some(prefix.clone()),
                            tag: None,
                            object_size_greater_than: None,
                            object_size_less_than: None,
                            and: None,
                        }),
                        expiration: Some(shiguredo_s3::types::LifecycleExpiration {
                            days: Some(*days as i32),
                            date: None,
                            expired_object_delete_marker: None,
                        }),
                        transitions: None,
                        noncurrent_version_transitions: None,
                        noncurrent_version_expiration: None,
                        abort_incomplete_multipart_upload: None,
                    };
                    let request = client
                        .client()
                        .put_bucket_lifecycle_configuration()
                        .bucket(bucket)
                        .rule(rule)
                        .build_request();
                    match request {
                        Ok(req) => match client.execute(&req).await {
                            Ok(response) if !response.is_success() => {
                                tracing::warn!(
                                    "PutBucketLifecycleConfiguration failed: status={}",
                                    response.status_code
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "failed to set S3 lifecycle configuration: {}",
                                    e.display()
                                );
                            }
                            _ => {}
                        },
                        Err(e) => {
                            tracing::warn!(
                                "failed to build PutBucketLifecycleConfiguration request: {e}"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "failed to build S3 client for lifecycle configuration: {}",
                        e.display()
                    );
                }
            }
        }
        let Some(pipeline_handle) = self.pipeline_handle.as_ref() else {
            if let Some(output) = self.outputs.get_mut(output_name) {
                output.runtime.active = false;
                output.runtime.started_at = None;
                output.runtime.run = None;
            }
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                    "Pipeline is not initialized",
                ),
            );
        };
        match start_hls_processors(pipeline_handle, &program_output, &run, &hls_settings).await {
            Ok(master_playlist_task) => {
                if let Some(output) = self.outputs.get_mut(output_name) {
                    output.runtime.background_task = master_playlist_task;
                }
            }
            Err(e) => {
                if let Some(output) = self.outputs.get_mut(output_name) {
                    output.runtime.active = false;
                    output.runtime.started_at = None;
                    output.runtime.run = None;
                }
                let _ = stop_processors_staged_hls(pipeline_handle, &run).await;
                let error_comment = format!("Failed to start HLS: {}", e.display());
                return OutputOperationOutcome::failure(
                    crate::obsws::response::build_request_response_error(
                        request_type,
                        request_id,
                        crate::obsws::protocol::REQUEST_STATUS_REQUEST_PROCESSING_FAILED,
                        &error_comment,
                    ),
                );
            }
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_start_output_response(request_id),
            None,
        )
    }

    pub(crate) async fn handle_stop_hls(
        &mut self,
        request_type: &str,
        request_id: &str,
        output_name: &str,
    ) -> OutputOperationOutcome {
        // run を取得してランタイム状態をリセット
        let run = self.outputs.get_mut(output_name).and_then(|o| {
            if let Some(handle) = o.runtime.background_task.take() {
                handle.abort();
            }
            let run = o.runtime.run.take();
            o.runtime.active = false;
            o.runtime.started_at = None;
            match run {
                Some(OutputRun::Hls(r)) => Some(r),
                _ => None,
            }
        });
        let Some(run) = run else {
            return OutputOperationOutcome::failure(
                crate::obsws::response::build_request_response_error(
                    request_type,
                    request_id,
                    crate::obsws::protocol::REQUEST_STATUS_OUTPUT_NOT_RUNNING,
                    "HLS is not active",
                ),
            );
        };
        if let Some(pipeline_handle) = self.pipeline_handle.as_ref()
            && let Err(e) = stop_processors_staged_hls(pipeline_handle, &run).await
        {
            tracing::warn!("failed to stop HLS processors: {}", e.display());
        }
        OutputOperationOutcome::success(
            crate::obsws::response::build_stop_output_response(request_id),
            None,
        )
    }
}

/// HLS 用プロセッサを起動する
/// 戻り値は ABR マスタープレイリスト書き出しタスクの JoinHandle（ABR でない場合は None）。
/// 呼び出し元は JoinHandle を保持し、出力停止時に abort() すること。
async fn start_hls_processors(
    pipeline_handle: &crate::MediaPipelineHandle,
    program_output: &ObswsProgramOutputContext,
    run: &ObswsHlsRun,
    hls_settings: &ObswsHlsSettings,
) -> crate::Result<Option<tokio::task::JoinHandle<()>>> {
    // HLS 用にキーフレーム間隔を設定する。
    // segment_duration に合わせたフレーム数を計算し、エンコーダーに事前通知する。
    let fps = program_output.frame_rate.numerator.get() as f64
        / program_output.frame_rate.denumerator.get() as f64;
    let keyframe_interval_frames = (hls_settings.segment_duration * fps).ceil() as u32;
    let keyframe_interval_frames = keyframe_interval_frames.max(1);
    let encode_params = crate::encoder::encode_config_with_keyframe_interval(
        keyframe_interval_frames,
        program_output.frame_rate,
    );

    let is_abr = run.is_abr();

    // ABR の場合、各 variant writer が SampleEntry から codec string を確定したら
    // oneshot channel 経由で通知を受け取り、全 variant の値がそろってからマスタープレイリストを書き出す。
    let mut codec_string_receivers = Vec::new();

    // バリアントごとにスケーラー、エンコーダー、ライターを起動する
    for (i, (variant, variant_run)) in hls_settings
        .variants
        .iter()
        .zip(run.variant_runs.iter())
        .enumerate()
    {
        // filesystem かつ ABR の場合はバリアントのサブディレクトリを作成する
        if is_abr && let HlsDestination::Filesystem { .. } = run.destination {
            std::fs::create_dir_all(&variant_run.variant_path).map_err(|e| {
                crate::Error::new(format!(
                    "failed to create variant directory {}: {e}",
                    variant_run.variant_path
                ))
            })?;
        }

        // 解像度変換が必要な場合はスケーラーを挿入する
        let video_encoder_input_track = if let (Some(scaler_id), Some(scaled_track_id)) = (
            &variant_run.scaler_processor_id,
            &variant_run.scaled_track_id,
        ) {
            let width = variant.width.expect("infallible: scaler requires width");
            let height = variant.height.expect("infallible: scaler requires height");
            crate::scaler::create_processor(
                pipeline_handle,
                crate::scaler::VideoScalerConfig {
                    input_track_id: program_output.video_track_id.clone(),
                    output_track_id: scaled_track_id.clone(),
                    width,
                    height,
                },
                Some(scaler_id.clone()),
            )
            .await?;
            scaled_track_id.clone()
        } else {
            variant_run.video.source_track_id.clone()
        };

        // ビデオエンコーダー
        crate::encoder::create_video_processor_with_params(
            pipeline_handle,
            video_encoder_input_track,
            variant_run.video.encoded_track_id.clone(),
            crate::types::CodecName::H264,
            std::num::NonZeroUsize::new(variant.video_bitrate_bps)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            program_output.frame_rate,
            Some(encode_params.clone()),
            Some(variant_run.video.encoder_processor_id.clone()),
        )
        .await?;

        // オーディオエンコーダー（HLS 仕様で AAC 必須）
        crate::encoder::create_audio_processor(
            pipeline_handle,
            program_output.audio_track_id.clone(),
            variant_run.audio.encoded_track_id.clone(),
            crate::types::CodecName::Aac,
            std::num::NonZeroUsize::new(variant.audio_bitrate_bps)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            Some(variant_run.audio.encoder_processor_id.clone()),
        )
        .await?;

        // HLS ライター
        let storage_config = match &run.destination {
            HlsDestination::Filesystem { .. } => crate::hls::writer::HlsStorageConfig::Filesystem {
                output_directory: std::path::PathBuf::from(&variant_run.variant_path),
            },
            HlsDestination::S3 {
                bucket,
                region,
                endpoint,
                use_path_style,
                access_key_id,
                secret_access_key,
                session_token,
                ..
            } => {
                let client = build_s3_client(
                    region,
                    access_key_id,
                    secret_access_key,
                    session_token.as_deref(),
                    endpoint.as_deref(),
                    *use_path_style,
                )?;
                crate::hls::writer::HlsStorageConfig::S3 {
                    client,
                    bucket: bucket.clone(),
                    prefix: variant_run.variant_path.clone(),
                }
            }
        };
        // ABR の場合は codec string 通知用の channel を作成する
        let codec_string_sender = if is_abr {
            let (tx, rx) = tokio::sync::oneshot::channel();
            codec_string_receivers.push(rx);
            Some(tx)
        } else {
            None
        };

        crate::hls::writer::create_processor(
            pipeline_handle,
            crate::hls::writer::HlsWriterConfig {
                storage: storage_config,
                input_audio_track_id: variant_run.audio.encoded_track_id.clone(),
                input_video_track_id: variant_run.video.encoded_track_id.clone(),
                segment_duration: hls_settings.segment_duration,
                max_retained_segments: hls_settings.max_retained_segments,
                segment_format: hls_settings.segment_format,
                codec_string_sender,
            },
            Some(variant_run.writer_processor_id.clone()),
        )
        .await?;

        tracing::info!(
            variant = i,
            video_bitrate = variant.video_bitrate_bps,
            audio_bitrate = variant.audio_bitrate_bps,
            directory = %variant_run.variant_path,
            "HLS variant processor started"
        );
    }

    // ABR の場合は各 variant writer が SampleEntry から codec string を確定するのを待ち、
    // 全 variant の codec string が一致することを検証してからマスタープレイリストを書き出す。
    if is_abr {
        let master_variants: Vec<crate::hls::writer::MasterPlaylistVariant> = hls_settings
            .variants
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                let width = variant
                    .width
                    .map(|w| w.get() as u32)
                    .unwrap_or(program_output.canvas_width.get() as u32);
                let height = variant
                    .height
                    .map(|h| h.get() as u32)
                    .unwrap_or(program_output.canvas_height.get() as u32);
                crate::hls::writer::MasterPlaylistVariant {
                    bandwidth: variant.video_bitrate_bps as u64 + variant.audio_bitrate_bps as u64,
                    width,
                    height,
                    playlist_uri: format!("variant_{i}/playlist.m3u8"),
                }
            })
            .collect();

        let destination = run.destination.clone();

        let handle = tokio::spawn(async move {
            // 全 variant の codec string を収集する
            let mut codec_strings = Vec::with_capacity(codec_string_receivers.len());
            for (i, rx) in codec_string_receivers.into_iter().enumerate() {
                match rx.await {
                    Ok(cs) => codec_strings.push(cs),
                    Err(_) => {
                        tracing::warn!(
                            variant = i,
                            "HLS variant writer dropped codec string sender before resolving codecs"
                        );
                        return;
                    }
                }
            }

            // 全 variant の codec string が一致することを検証する
            let Some(first) = codec_strings.first() else {
                return;
            };
            for (i, cs) in codec_strings.iter().enumerate().skip(1) {
                if cs.video != first.video || cs.audio != first.audio {
                    tracing::error!(
                        variant = i,
                        expected_video = %first.video,
                        expected_audio = %first.audio,
                        actual_video = %cs.video,
                        actual_audio = %cs.audio,
                        "HLS ABR variant codec string mismatch: \
                         all variants must produce identical codec strings"
                    );
                    return;
                }
            }

            let master_content =
                crate::hls::writer::build_master_playlist_content(&master_variants, first);
            match &destination {
                HlsDestination::Filesystem { directory } => {
                    if let Err(e) = crate::hls::writer::write_master_playlist(
                        &std::path::PathBuf::from(directory),
                        &master_variants,
                        first,
                    ) {
                        tracing::error!(error = ?e, "failed to write HLS master playlist");
                    }
                }
                HlsDestination::S3 {
                    bucket,
                    prefix,
                    region,
                    endpoint,
                    use_path_style,
                    access_key_id,
                    secret_access_key,
                    session_token,
                    ..
                } => {
                    let s3_client = match build_s3_client(
                        region,
                        access_key_id,
                        secret_access_key,
                        session_token.as_deref(),
                        endpoint.as_deref(),
                        *use_path_style,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(error = ?e, "failed to create S3 client for HLS master playlist");
                            return;
                        }
                    };
                    let key = if prefix.is_empty() {
                        "playlist.m3u8".to_owned()
                    } else {
                        format!("{prefix}/playlist.m3u8")
                    };
                    let request = match s3_client
                        .client()
                        .put_object()
                        .bucket(bucket)
                        .key(&key)
                        .body(master_content.into_bytes())
                        .content_type("application/vnd.apple.mpegurl")
                        .build_request()
                    {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::error!(error = ?e, "failed to build S3 PutObject request for HLS master playlist");
                            return;
                        }
                    };
                    match s3_client.execute(&request).await {
                        Ok(response) if !response.is_success() => {
                            tracing::error!(
                                status = response.status_code,
                                "S3 PutObject failed for HLS master playlist {key}"
                            );
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "failed to upload HLS master playlist to S3");
                        }
                        _ => {}
                    }
                }
            }
        });
        Ok(Some(handle))
    } else {
        Ok(None)
    }
}

/// HLS 用プロセッサを段階的に停止する。
/// Program 出力は共有なので、variant 後段の processor のみを停止する。
async fn stop_processors_staged_hls(
    pipeline_handle: &crate::MediaPipelineHandle,
    run: &ObswsHlsRun,
) -> crate::Result<()> {
    // NOTE:
    // ライブ用途では StopOutput / ToggleOutput への応答遅延を避けることを優先し、
    // ここでは writer に finalize / cleanup を先行させる。
    // この経路は上流 encoder / scaler の完全 drain を保証しないため、
    // 停止直前の数フレームが最終セグメントに含まれない可能性がある。
    //
    // TODO:
    // 末尾欠損まで解消するには、writer を先に閉じるのではなく、
    // 上流から EOS 相当を伝播させる明示的な finish 経路が必要になる。
    // terminate_processor() は abort ベースで停止するだけなので、
    // encoder / scaler の残フレーム排出には使えない。
    let writer_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .map(|vr| vr.writer_processor_id.clone())
        .collect();
    for writer_id in &writer_ids {
        finish_hls_writer_rpc(pipeline_handle, writer_id).await;
    }
    wait_or_terminate(pipeline_handle, &writer_ids, Duration::from_secs(5)).await?;

    let encoder_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .flat_map(|vr| {
            [
                vr.video.encoder_processor_id.clone(),
                vr.audio.encoder_processor_id.clone(),
            ]
        })
        .collect();
    terminate_and_wait(pipeline_handle, &encoder_ids).await?;

    let scaler_ids: Vec<crate::ProcessorId> = run
        .variant_runs
        .iter()
        .filter_map(|vr| vr.scaler_processor_id.clone())
        .collect();
    if !scaler_ids.is_empty() {
        terminate_and_wait(pipeline_handle, &scaler_ids).await?;
    }

    // ABR の場合はマスタープレイリストとバリアントディレクトリを削除する
    if run.is_abr() {
        match &run.destination {
            HlsDestination::Filesystem { directory } => {
                let master_playlist_path =
                    std::path::PathBuf::from(directory).join("playlist.m3u8");
                if let Err(e) = std::fs::remove_file(&master_playlist_path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(
                        "failed to remove master playlist {}: {e}",
                        master_playlist_path.display()
                    );
                }
                // バリアントのサブディレクトリも削除する（ライターが中身を削除済みなので空のはず）
                for vr in &run.variant_runs {
                    if let Err(e) = std::fs::remove_dir(&vr.variant_path)
                        && e.kind() != std::io::ErrorKind::NotFound
                    {
                        tracing::warn!(
                            "failed to remove variant directory {}: {e}",
                            vr.variant_path
                        );
                    }
                }
            }
            HlsDestination::S3 {
                bucket,
                prefix,
                region,
                endpoint,
                use_path_style,
                access_key_id,
                secret_access_key,
                session_token,
                ..
            } => {
                // マスタープレイリストを DeleteObject で削除する
                // バリアント「ディレクトリ」の削除は不要（S3 にディレクトリ概念なし）
                if let Ok(s3_client) = build_s3_client(
                    region,
                    access_key_id,
                    secret_access_key,
                    session_token.as_deref(),
                    endpoint.as_deref(),
                    *use_path_style,
                ) {
                    let key = if prefix.is_empty() {
                        "playlist.m3u8".to_owned()
                    } else {
                        format!("{prefix}/playlist.m3u8")
                    };
                    match s3_client
                        .client()
                        .delete_object()
                        .bucket(bucket)
                        .key(&key)
                        .build_request()
                    {
                        Ok(request) => match s3_client.execute(&request).await {
                            Ok(response) if !response.is_success() => {
                                tracing::warn!(
                                    "S3 DeleteObject failed for master playlist {key}: status={}",
                                    response.status_code
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "failed to delete S3 master playlist {key}: {}",
                                    e.display()
                                );
                            }
                            _ => {}
                        },
                        Err(e) => {
                            tracing::warn!(
                                "failed to build DeleteObject for master playlist {key}: {e}"
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// HLS writer に Finish RPC を送り、finalize / cleanup を促す。
/// これは writer 側の入力購読を閉じるためのもので、上流の完全 drain は保証しない。
/// 失敗時は terminate にフォールバックする。
async fn finish_hls_writer_rpc(
    pipeline_handle: &crate::MediaPipelineHandle,
    processor_id: &crate::ProcessorId,
) {
    const RETRY_TIMEOUT: Duration = Duration::from_millis(500);
    const RETRY_INTERVAL: Duration = Duration::from_millis(10);
    let deadline = tokio::time::Instant::now() + RETRY_TIMEOUT;

    loop {
        match pipeline_handle
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<
                crate::hls::writer::HlsWriterRpcMessage,
            >>(processor_id)
            .await
        {
            Ok(sender) => {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                let _ = sender.send(crate::hls::writer::HlsWriterRpcMessage::Finish { reply_tx });
                let _ = reply_rx.await;
                return;
            }
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(_) => {
                let _ = pipeline_handle.terminate_processor(processor_id.clone()).await;
                return;
            }
        }
    }
}
