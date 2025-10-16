use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

use orfail::OrFail;

use crate::{
    audio,
    json::JsonObject,
    layout_decode_params::LayoutDecodeParams,
    layout_encode_params::LayoutEncodeParams,
    layout_region::{self, RawRegion, Region},
    metadata::{ArchiveMetadata, ContainerFormat, RecordingMetadata, SourceId, SourceInfo},
    types::{CodecName, EngineName, EvenUsize},
    video::FrameRate,
};

pub const DEFAULT_LAYOUT_JSON: &str = include_str!("../layout-examples/compose-default.jsonc");

/// トリム開始時刻から終了時刻へのマップ
#[derive(Debug, Default, Clone)]
pub struct TrimSpans(BTreeMap<Duration, Duration>);

impl TrimSpans {
    pub fn new(spans: BTreeMap<Duration, Duration>) -> Self {
        Self(spans)
    }

    pub fn contains(&self, timestamp: Duration) -> bool {
        if let Some((&start, &end)) = self.0.range(..=timestamp).next_back() {
            (start..end).contains(&timestamp)
        } else {
            false
        }
    }
}

/// 合成レイアウト
#[derive(Debug, Clone)]
pub struct Layout {
    pub base_path: PathBuf,
    // z-pos 順に並んだリージョン列
    pub video_regions: Vec<Region>,

    pub trim_spans: TrimSpans,
    pub resolution: Resolution,

    pub audio_source_ids: BTreeSet<SourceId>,
    pub sources: BTreeMap<SourceId, AggregatedSourceInfo>,

    pub audio_codec: CodecName,
    pub video_codec: CodecName,
    pub audio_bitrate: Option<NonZeroUsize>,
    pub video_bitrate: Option<usize>,
    pub video_encoders: Vec<EngineName>,
    pub video_h264_decoder: Option<EngineName>,
    pub video_h265_decoder: Option<EngineName>,
    pub video_vp8_decoder: Option<EngineName>,
    pub video_vp9_decoder: Option<EngineName>,
    pub video_av1_decoder: Option<EngineName>,
    pub encode_params: LayoutEncodeParams,
    pub decode_params: LayoutDecodeParams,
    pub frame_rate: FrameRate,
}

impl Layout {
    /// レイアウト JSON ファイルで指示されたレイアウトを作成する
    pub fn from_layout_json_file(
        base_path: PathBuf,
        layout_file_path: &Path,
    ) -> orfail::Result<Self> {
        let raw: RawLayout = crate::json::parse_file(layout_file_path).or_fail()?;
        raw.into_layout(base_path).or_fail()
    }

    /// レイアウト JSON 文字列で指示されたレイアウトを作成する
    pub fn from_layout_json_str(
        base_path: PathBuf,
        layout_json_text: &str,
    ) -> orfail::Result<Self> {
        let raw: RawLayout = crate::json::parse_str(layout_json_text).or_fail()?;
        raw.into_layout(base_path).or_fail()
    }

    pub fn from_layout_json_file_or_default(
        base_path: PathBuf,
        layout_file_path: Option<&Path>,
        default_layout_json: &str,
    ) -> orfail::Result<Self> {
        if let Some(layout_file_path) = layout_file_path {
            Layout::from_layout_json_file(base_path, layout_file_path).or_fail()
        } else {
            Layout::from_layout_json_str(base_path, default_layout_json).or_fail()
        }
    }

    /// recording.report から合成レイアウトを作成する
    pub fn from_recording_report(
        report_file_path: &Path,
        report: &RecordingMetadata,
        audio_only: bool,
        max_columns: usize,
    ) -> orfail::Result<Self> {
        let base_path = std::path::absolute(report_file_path)
            .or_fail()?
            .parent()
            .or_fail()?
            .to_path_buf();

        // layout 未指定の場合にはセルの解像度は固定
        // （ただし内枠分が引かれるのでもう少し小さくなることがある）
        let cell_width = 320;
        let cell_height = 240;

        let (rows, columns) = if audio_only {
            (1, 1)
        } else {
            layout_region::decide_grid_dimensions(0, max_columns, report.archives.len())
        };

        // 全体の解像度を求める（キリを良くするために内枠のことはここでは考慮しない）
        let width = columns * cell_width;
        let height = rows * cell_height;

        let source_paths = report
            .archive_metadata_paths()
            .or_fail()?
            .into_iter()
            .map(|path| path.file_name().or_fail().map(PathBuf::from))
            .collect::<orfail::Result<Vec<_>>>()?;
        let video_layout = nojson::json(|f| {
            f.object(|f| {
                if audio_only {
                    return Ok(());
                }
                f.member(
                    "grid",
                    nojson::json(|f| {
                        f.object(|f| {
                            f.member("video_sources", &source_paths)?;
                            f.member("border_pixels", 0)?;
                            f.member("max_columns", max_columns)
                        })
                    }),
                )
            })
        });

        let layout_json = nojson::json(|f| {
            f.object(|f| {
                f.member("audio_sources", &source_paths)?;
                f.member("video_layout", &video_layout)?;
                f.member("resolution", format!("{width}x{height}"))
            })
        });
        let raw: RawLayout = crate::json::parse_str(&layout_json.to_string()).or_fail()?;
        raw.into_layout(base_path).or_fail()
    }

    pub fn video_bitrate_bps(&self) -> usize {
        self.video_bitrate
            .unwrap_or_else(|| 200 * self.video_source_ids().count() * 1024)
    }

    pub fn audio_bitrate_bps(&self) -> NonZeroUsize {
        self.audio_bitrate
            .unwrap_or(NonZeroUsize::new(audio::DEFAULT_BITRATE).expect("infallible"))
    }

    pub fn has_audio(&self) -> bool {
        self.audio_source_ids().next().is_some()
    }

    pub fn has_video(&self) -> bool {
        self.video_source_ids().next().is_some()
    }

    pub fn audio_source_ids(&self) -> impl '_ + Iterator<Item = &SourceId> {
        self.audio_source_ids.iter()
    }

    pub fn video_source_ids(&self) -> impl '_ + Iterator<Item = &SourceId> {
        self.video_regions
            .iter()
            .flat_map(|region| region.grid.assigned_sources.keys())
    }

    pub fn duration(&self) -> Duration {
        self.sources
            .values()
            .map(|s| s.stop_timestamp)
            .max()
            .unwrap_or_default()
    }

    fn trim_duration(&self) -> Duration {
        self.trim_spans
            .0
            .iter()
            .map(|(start, end)| end.saturating_sub(*start))
            .fold(Duration::ZERO, |acc, duration| acc.saturating_add(duration))
    }

    pub fn output_duration(&self) -> Duration {
        self.duration().saturating_sub(self.trim_duration())
    }
}

#[derive(Debug, Clone)]
struct RawLayout {
    audio_sources: Vec<PathBuf>,
    audio_sources_excluded: Vec<PathBuf>,
    video_layout: BTreeMap<String, RawRegion>,
    trim: bool,
    resolution: Option<Resolution>,
    audio_bitrate: Option<NonZeroUsize>,
    video_bitrate: Option<usize>,
    audio_codec: CodecName,
    video_codec: CodecName,
    video_encoders: Vec<EngineName>,
    video_h264_decoder: Option<EngineName>,
    video_h265_decoder: Option<EngineName>,
    video_vp8_decoder: Option<EngineName>,
    video_vp9_decoder: Option<EngineName>,
    video_av1_decoder: Option<EngineName>,
    encode_params: LayoutEncodeParams,
    decode_params: LayoutDecodeParams,
    frame_rate: FrameRate,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for RawLayout {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let object = JsonObject::new(value)?;

        if let Some(video_layout) = value.to_member("video_layout")?.get() {
            // 事前にリージョン名の重複をチェックする
            let mut region_names = BTreeSet::new();
            for (name, _) in video_layout.to_object()? {
                if !region_names.insert(name.to_unquoted_string_str()?) {
                    return Err(name.invalid("duplicate region name"));
                }
            }
        }

        Ok(Self {
            audio_sources: object.get("audio_sources")?.unwrap_or_default(),
            audio_sources_excluded: object.get("audio_sources_excluded")?.unwrap_or_default(),
            video_layout: object.get("video_layout")?.unwrap_or_default(),
            trim: object.get("trim")?.unwrap_or(true),
            resolution: object.get("resolution")?,
            audio_bitrate: object.get("audio_bitrate")?,
            video_bitrate: if let Some(bitrate) = object.get("video_bitrate")? {
                Some(bitrate)
            } else {
                // レガシー版との互換性維持のための "bitrate" フィールドも考慮する
                // こちらは kbps 単位なのでパース後に変換する
                object.get::<usize>("bitrate")?.map(|v| v * 1024)
            },
            video_codec: object
                .get_with("video_codec", |v| {
                    v.to_unquoted_string_str()
                        .and_then(|s| CodecName::parse_video(&s).map_err(|e| v.invalid(e)))
                })?
                .unwrap_or(CodecName::Vp9),
            audio_codec: object
                .get_with("audio_codec", |v| {
                    v.to_unquoted_string_str()
                        .and_then(|s| CodecName::parse_audio(&s).map_err(|e| v.invalid(e)))
                })?
                .unwrap_or(CodecName::Opus),
            video_encoders: object
                .get_with("video_encoders", |v| {
                    v.to_array()?.map(EngineName::parse_video_encoder).collect()
                })?
                .unwrap_or_else(|| EngineName::DEFAULT_VIDEO_ENCODERS.to_vec()),
            video_h264_decoder: object
                .get_with("video_h264_decoder", EngineName::parse_video_h264_decoder)?,
            video_h265_decoder: object
                .get_with("video_h265_decoder", EngineName::parse_video_h265_decoder)?,
            video_vp8_decoder: object
                .get_with("video_vp8_decoder", EngineName::parse_video_vp8_decoder)?,
            video_vp9_decoder: object
                .get_with("video_vp9_decoder", EngineName::parse_video_vp9_decoder)?,
            video_av1_decoder: object
                .get_with("video_av1_decoder", EngineName::parse_video_av1_decoder)?,
            frame_rate: object
                .get_with("frame_rate", |v| {
                    v.as_raw_str().parse().map_err(|e| v.invalid(e))
                })?
                .unwrap_or(FrameRate::FPS_25),

            // エンコードパラメータ群はトップレベルに配置されているので object を経由せずに value を直接変換する
            encode_params: value.try_into()?,
            decode_params: value.try_into()?,
        })
    }
}

impl RawLayout {
    fn into_layout(self, base_path: PathBuf) -> orfail::Result<Layout> {
        let base_path = base_path.canonicalize().or_fail_with(|e| {
            format!(
                "failed to canonicalize base dir {}: {e}",
                base_path.display()
            )
        })?;

        // 利用するソース一覧を確定して、情報を読み込む
        let mut audio_source_ids = BTreeSet::new();
        let mut sources = BTreeMap::<SourceId, AggregatedSourceInfo>::new();
        let resolved = resolve_source_and_media_path_pairs(
            &base_path,
            &self.audio_sources,
            &self.audio_sources_excluded,
        )
        .or_fail()?;
        for (source, media_path) in resolved {
            sources
                .entry(source.id.clone())
                .or_default()
                .update(&source, &media_path);
            audio_source_ids.insert(source.id);
        }

        let mut video_regions = Vec::new();
        for (_region_name, raw_region) in self.video_layout {
            let region = raw_region
                .into_region(
                    &base_path,
                    &mut sources,
                    self.resolution,
                    resolve_source_and_media_path_pairs,
                )
                .or_fail()?;
            video_regions.push(region);
        }
        video_regions.sort_by_key(|r| r.z_pos);

        // 解像度の決定やバリデーションを行う
        let resolution = if let Some(resolution) = self.resolution {
            resolution
        } else if video_regions.is_empty() {
            // 音声のみの場合は使われないので何でもいい
            Resolution::new(Resolution::MIN, Resolution::MIN).or_fail()?
        } else {
            // "resolution" が未指定の場合はリージョンから求める
            let mut resolution = Resolution::new(Resolution::MIN, Resolution::MIN).or_fail()?;
            for region in &video_regions {
                resolution.width = resolution.width.max(region.position.x + region.width);
                resolution.height = resolution.height.max(region.position.y + region.height);
            }
            resolution
        };

        let trim_spans = decide_trim_spans(&sources, !self.trim);

        for source in sources.values_mut() {
            source.merge_overlapping_sources().or_fail()?;
        }

        Ok(Layout {
            base_path,
            video_regions,
            trim_spans,
            resolution,
            audio_source_ids,
            sources,
            audio_codec: self.audio_codec,
            video_codec: self.video_codec,
            audio_bitrate: self.audio_bitrate,
            video_bitrate: self.video_bitrate,
            video_encoders: self.video_encoders,
            video_h264_decoder: self.video_h264_decoder,
            video_h265_decoder: self.video_h265_decoder,
            video_vp8_decoder: self.video_vp8_decoder,
            video_vp9_decoder: self.video_vp9_decoder,
            video_av1_decoder: self.video_av1_decoder,
            encode_params: self.encode_params,
            decode_params: self.decode_params,
            frame_rate: self.frame_rate,
        })
    }
}

/// ソースのセルへの割り当て情報
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignedSource {
    /// 割り当て先のセルのインデックス
    pub cell_index: usize,

    /// ソースの優先度
    ///
    /// 時間的にオーバーラップしている複数のソースが同じセルに割り当てられた場合には、
    /// 優先度が高い（値が小さい）方が合成時に使用される
    pub priority: usize,
}

impl AssignedSource {
    pub fn new(cell_index: usize, priority: usize) -> Self {
        Self {
            cell_index,
            priority,
        }
    }
}

/// 映像の解像度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub width: EvenUsize,
    pub height: EvenUsize,
}

impl Resolution {
    /// `width` ないし `height` の最小値
    pub const MIN: usize = 16;

    /// `width` ないし `height` の最大値
    pub const MAX: usize = 3840;

    /// [`Resolution`] インスタンスを生成する
    pub fn new(width: usize, height: usize) -> orfail::Result<Self> {
        let range = Self::MIN..=Self::MAX;
        range
            .contains(&width)
            .or_fail_with(|()| format!("width {width} is out of range"))?;
        range
            .contains(&height)
            .or_fail_with(|()| format!("height {height} is out of range"))?;
        Ok(Self {
            width: EvenUsize::truncating_new(width),
            height: EvenUsize::truncating_new(height),
        })
    }

    /// 横幅を返す
    ///
    /// 結果の値については、以下が保証されている:
    /// - `MIN` と `MAX` の範囲内に収まっている
    /// - 2 の倍数に丸められている
    pub fn width(self) -> EvenUsize {
        self.width
    }

    /// 縦幅を返す
    ///
    /// 結果の値については、以下が保証されている:
    /// - `MIN` と `MAX` の範囲内に収まっている
    /// - 2 の倍数に丸められている
    pub fn height(self) -> EvenUsize {
        self.height
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for Resolution {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let s = value.to_unquoted_string_str()?;

        let Some((Ok(width), Ok(height))) = s.split_once('x').map(|(w, h)| (w.parse(), h.parse()))
        else {
            return Err(value.invalid(format!("invalid resolution: {s}")));
        };

        Self::new(width, height)
            .map_err(|e| value.invalid(format!("invalid resolution: {}", e.message)))
    }
}

pub fn resolve_source_and_media_path_pairs(
    base_path: &Path,
    sources: &[PathBuf],
    excluded: &[PathBuf],
) -> orfail::Result<Vec<(SourceInfo, PathBuf)>> {
    let resolved_paths = resolve_source_paths(base_path, sources, excluded).or_fail()?;

    let mut resolved = Vec::new();
    for path in resolved_paths {
        let archive = ArchiveMetadata::from_file(&path).or_fail()?;
        let mut media_path = path.clone();
        match archive.format {
            ContainerFormat::Webm => media_path.set_extension("webm"),
            ContainerFormat::Mp4 => media_path.set_extension("mp4"),
        };
        resolved.push((archive.source_info(), media_path));
    }

    Ok(resolved)
}

/// ワイルドカードを含むソースのパスの解決を行う
pub fn resolve_source_paths(
    base_path: &Path,
    sources: &[PathBuf],
    excluded: &[PathBuf],
) -> orfail::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for source in sources {
        for path in glob(base_path.join(source)).or_fail()? {
            // 後段のチェックなどを簡単にするためにパスを正規化する
            let path = path.canonicalize().or_fail_with(|e| {
                format!(
                    "failed to canonicalize source file path {}: {e}",
                    path.display()
                )
            })?;

            // base_path の範囲外を参照しているパスがあったらエラー
            path.starts_with(base_path).or_fail_with(|()| {
                format!(
                    "source path '{}' is outside the base dir '{}'",
                    path.display(),
                    base_path.display()
                )
            })?;

            paths.push(path);
        }
    }

    // 重複エントリは除去する
    let mut known = HashSet::new();
    paths.retain(|path| known.insert(path.clone()));

    let excluded = excluded
        .iter()
        .map(|p| std::path::absolute(base_path.join(p)).or_fail())
        .collect::<orfail::Result<Vec<_>>>()?;

    paths.retain(|path0| {
        for path1 in &excluded {
            if path0.parent() != path1.parent() {
                continue;
            }

            let (Some(name0), Some(name1)) = (
                path0.file_name().and_then(|n| n.to_str()),
                path1.file_name().and_then(|n| n.to_str()),
            ) else {
                continue;
            };

            if is_wildcard_name_matched(name1, name0) {
                return false;
            }
        }
        true
    });

    Ok(paths)
}

fn media_file_exists<P: AsRef<Path>>(source_file_path: P) -> bool {
    let mut source_file_path = source_file_path.as_ref().to_path_buf();
    for ext in ["webm", "mp4"] {
        if source_file_path.set_extension(ext) && source_file_path.exists() {
            return true;
        }
    }
    false
}

fn glob(path: PathBuf) -> orfail::Result<Vec<PathBuf>> {
    if !path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains('*'))
    {
        // 名前部分にワイルドカードを含んでいないなら通常のパスとして扱う
        path.exists()
            .or_fail_with(|()| format!("no such source file: {}", path.display()))?;
        media_file_exists(&path)
            .or_fail_with(|()| format!("no media file for the source: {}", path.display()))?;
        return Ok(vec![path]);
    }

    // ここまで来たら名前部分にワイルドカードを含んでいるので展開する
    let wildcard_name = path.file_name().and_then(|name| name.to_str()).or_fail()?;

    let parent = path.parent().or_fail()?;
    parent
        .exists()
        .or_fail_with(|()| format!("no such source file directory: {}", parent.display()))?;

    let mut paths = Vec::new();
    for entry in path.parent().or_fail()?.read_dir().or_fail()? {
        let entry = entry.or_fail()?;
        if !entry
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| is_wildcard_name_matched(wildcard_name, name))
        {
            continue;
        }
        if !media_file_exists(entry.path()) {
            // 対応するメディアファイルが存在しない場合には、ワイルドカードの展開結果には含めない
            //
            // これは典型的には分割録画で `split-archive-*.json` と指定したら
            // `split-archive-end-*.json` ファイルも展開結果に含まれてしまうのを防止するための処理
            //
            // 分割録画では普通に発生する状況なので、警告ではなくデバッグでログを出すに留めている
            log::debug!(
                "skipping source file '{}' as no corresponding media file exists",
                entry.path().display()
            );
            continue;
        }
        paths.push(entry.path());
    }

    // read_dir() の結果の順番は環境によって変わるかもしれないので、
    // ここで結果をソートして常に同じ順番となるようにしておく
    paths.sort();

    Ok(paths)
}

fn is_wildcard_name_matched(wildcard_name: &str, mut name: &str) -> bool {
    let mut is_first = true;
    for token in wildcard_name.split('*') {
        if is_first {
            if !name.starts_with(token) {
                return false;
            }
            name = &name[token.len()..];
            is_first = false;
        } else {
            let Some(i) = name.find(token) else {
                return false;
            };
            name = &name[i + token.len()..];
        }
    }
    wildcard_name.ends_with('*') || name.is_empty()
}

fn decide_trim_spans(
    sources: &BTreeMap<SourceId, AggregatedSourceInfo>,
    trim_first_gap_only: bool,
) -> TrimSpans {
    // 時刻順でソートする
    let mut sources = sources
        .values()
        .map(|s| (s.start_timestamp, s.stop_timestamp))
        .collect::<Vec<_>>();
    sources.sort();

    let mut trim_spans = BTreeMap::new();
    let mut now = Duration::ZERO;
    for (start_timestamp, stop_timestamp) in sources {
        if trim_first_gap_only && now != Duration::ZERO {
            // レイアウト JSON で `trim: false` が指定された場合にはここにくる
            //
            // なお `trim` の値に関わらず冒頭部分のトリムは常に行われる
            break;
        }

        if now < start_timestamp {
            // 次のソースの開始時刻との間にギャップがあるのでトリムする
            trim_spans.insert(now, start_timestamp);
            now = stop_timestamp;
        } else {
            now = now.max(stop_timestamp);
        }
    }

    TrimSpans(trim_spans)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregatedSourceInfo {
    // 以下のフィールド群は全ての分割ファイルで値が同じになる
    pub id: SourceId,
    pub format: ContainerFormat,
    pub audio: bool,
    pub video: bool,

    // 開始時刻（分割録画の場合には一番小さな値）
    pub start_timestamp: Duration,

    // 終了時刻（分割録画の場合には一番大きな値）
    pub stop_timestamp: Duration,

    // 録画ファイルのパスと対応する情報（分割録画の場合には複数になる）
    pub media_paths: BTreeMap<PathBuf, SourceInfo>,
}

impl AggregatedSourceInfo {
    pub fn update(&mut self, source_info: &SourceInfo, media_path: &Path) {
        self.id = source_info.id.clone();
        self.format = source_info.format;
        self.audio = source_info.audio;
        self.video = source_info.video;
        self.start_timestamp = self.start_timestamp.min(source_info.start_timestamp);
        self.stop_timestamp = self.stop_timestamp.max(source_info.stop_timestamp);
        self.media_paths
            .insert(media_path.to_path_buf(), source_info.clone());
    }

    // 一括録画と分割録画のファイルを統合するためのメソッド
    // 重なる期間があるソースは長い方を採用する
    pub fn merge_overlapping_sources(&mut self) -> orfail::Result<()> {
        let mut sources_by_timespan: Vec<_> = self.media_paths.iter().collect();

        // 開始時刻でソート、次に長さでソート（長い方が先）
        sources_by_timespan.sort_by(|a, b| {
            a.1.start_timestamp.cmp(&b.1.start_timestamp).then_with(|| {
                // 長い方（= 終了時刻が後の方）が先
                a.1.stop_timestamp.cmp(&b.1.stop_timestamp).reverse()
            })
        });

        // 重複期間を除去する
        let mut merged_sources: BTreeMap<PathBuf, SourceInfo> = BTreeMap::new();
        let mut last_stop_timestamp = Duration::ZERO;
        for (path, info) in sources_by_timespan {
            if info.start_timestamp < last_stop_timestamp {
                continue;
            }

            merged_sources.insert(path.clone(), info.clone());
            last_stop_timestamp = info.stop_timestamp;
        }

        // マージされたソースでmedia_pathsを更新
        self.media_paths = merged_sources;

        Ok(())
    }

    // 二つのソースが時間的に重なる部分があるかどうかを求める
    pub fn is_overlapped_with(&self, other: &Self) -> bool {
        if self.start_timestamp <= other.start_timestamp {
            other.start_timestamp <= self.stop_timestamp
        } else {
            self.start_timestamp <= other.stop_timestamp
        }
    }

    pub fn timestamp_sorted_media_paths(&self) -> Vec<PathBuf> {
        let mut paths = self
            .media_paths
            .iter()
            .map(|(path, info)| (info.start_timestamp, path.clone()))
            .collect::<Vec<_>>();
        paths.sort_by_key(|x| x.0);
        paths.into_iter().map(|x| x.1).collect()
    }
}

impl Default for AggregatedSourceInfo {
    fn default() -> Self {
        // [NOTE] デフォルトでのインスタンス生成後に、最低でも一回は `update()` が呼びだされる前提
        Self {
            id: SourceId::default(),
            format: ContainerFormat::default(),
            audio: false,
            video: false,
            start_timestamp: Duration::MAX, // 後で `min` を取るので最初は最大値
            stop_timestamp: Duration::ZERO, // 後で `max` を取るので最初は最小値
            media_paths: BTreeMap::default(),
        }
    }
}

// 非公開構造体のテストは layout_test.rs ではなくこっちでやる
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::metadata::{ContainerFormat, SourceId, SourceInfo};

    #[test]
    fn load_layout_jsons() -> orfail::Result<()> {
        let jsons = [
            include_str!("../testdata/layouts/layout0.json"),
            include_str!("../testdata/layouts/layout1.json"),
            include_str!("../testdata/layouts/layout2.json"),
            include_str!("../testdata/layouts/layout3.json"),
            include_str!("../testdata/layouts/layout4.json"),
            include_str!("../testdata/layouts/layout5.json"),
            include_str!("../testdata/layouts/layout6.json"),
            include_str!("../testdata/layouts/layout7.json"),
        ];
        for json in jsons {
            // Layout をロードしようとすると関連する archive.json も用意する必要があって
            // 手間なのでここでは RawLayout を使っている
            json.parse::<nojson::Json<RawLayout>>().or_fail()?;
        }
        Ok(())
    }

    #[test]
    fn test_duplicate_region_name() -> orfail::Result<()> {
        let json = include_str!("../testdata/layouts/error-layout-duplicate-region-name.json");
        let e = json.parse::<nojson::Json<RawLayout>>().err().or_fail()?;
        let error_message = e.to_string();
        assert!(error_message.contains("duplicate region name"));
        Ok(())
    }

    #[test]
    fn test_merge_overlapping_sources_no_overlap() -> orfail::Result<()> {
        let mut aggregated = create_test_aggregated_source_info();

        // 重複しないソースを追加
        let source1 = create_test_source_info(Duration::from_secs(0), Duration::from_secs(10));
        let source2 = create_test_source_info(Duration::from_secs(20), Duration::from_secs(30));

        aggregated.update(&source1, Path::new("path1.webm"));
        aggregated.update(&source2, Path::new("path2.webm"));

        aggregated.merge_overlapping_sources().or_fail()?;

        // 両方のソースが保持されるべき
        assert_eq!(aggregated.media_paths.len(), 2);
        assert_eq!(aggregated.start_timestamp, Duration::from_secs(0));
        assert_eq!(aggregated.stop_timestamp, Duration::from_secs(30));

        Ok(())
    }

    #[test]
    fn test_merge_overlapping_sources_complete_containment() -> orfail::Result<()> {
        let mut aggregated = create_test_aggregated_source_info();

        // 一方が他方を完全に含むソースを追加
        let long_source = create_test_source_info(Duration::from_secs(0), Duration::from_secs(20));
        let short_source = create_test_source_info(Duration::from_secs(5), Duration::from_secs(15));

        aggregated.update(&long_source, Path::new("long.webm"));
        aggregated.update(&short_source, Path::new("short.webm"));

        aggregated.merge_overlapping_sources().or_fail()?;

        // 長いソースのみが残るべき
        assert_eq!(aggregated.media_paths.len(), 1);
        assert!(aggregated.media_paths.contains_key(Path::new("long.webm")));
        assert_eq!(aggregated.start_timestamp, Duration::from_secs(0));
        assert_eq!(aggregated.stop_timestamp, Duration::from_secs(20));

        Ok(())
    }

    #[test]
    fn test_merge_overlapping_sources_empty() -> orfail::Result<()> {
        let mut aggregated = create_test_aggregated_source_info();

        aggregated.merge_overlapping_sources().or_fail()?;

        // 空の場合を適切に処理すべき
        assert_eq!(aggregated.media_paths.len(), 0);
        assert_eq!(aggregated.start_timestamp, Duration::MAX);
        assert_eq!(aggregated.stop_timestamp, Duration::ZERO);

        Ok(())
    }

    #[test]
    fn test_merge_overlapping_sources_partial_overlap() -> orfail::Result<()> {
        let mut aggregated = create_test_aggregated_source_info();

        // 部分的に重複するソースを追加
        let source1 = create_test_source_info(Duration::from_secs(0), Duration::from_secs(15));
        let source2 = create_test_source_info(Duration::from_secs(10), Duration::from_secs(20));

        aggregated.update(&source1, Path::new("source1.webm"));
        aggregated.update(&source2, Path::new("source2.webm"));

        aggregated.merge_overlapping_sources().or_fail()?;

        // 開始時刻順でソートし、長い方を優先する
        // source1 (0-15) が先に処理され、source2 (10-20) は source1 の終了時刻 (15) より前に開始するため除外される
        assert_eq!(aggregated.media_paths.len(), 1);
        assert!(
            aggregated
                .media_paths
                .contains_key(Path::new("source1.webm"))
        );
        // タイムスタンプは update() で設定されるので変更されない
        assert_eq!(aggregated.start_timestamp, Duration::from_secs(0));
        assert_eq!(aggregated.stop_timestamp, Duration::from_secs(20)); // updateで設定された最大値

        Ok(())
    }

    #[test]
    fn test_merge_overlapping_sources_identical_duration() -> orfail::Result<()> {
        let mut aggregated = create_test_aggregated_source_info();

        // 同じ長さだが異なる開始時刻のソースを追加
        let source1 = create_test_source_info(Duration::from_secs(0), Duration::from_secs(10));
        let source2 = create_test_source_info(Duration::from_secs(5), Duration::from_secs(15));

        aggregated.update(&source1, Path::new("source1.webm"));
        aggregated.update(&source2, Path::new("source2.webm"));

        aggregated.merge_overlapping_sources().or_fail()?;

        // 開始時刻が早い source1 が残り、source2 は除外される
        assert_eq!(aggregated.media_paths.len(), 1);
        assert!(
            aggregated
                .media_paths
                .contains_key(Path::new("source1.webm"))
        );
        assert_eq!(aggregated.start_timestamp, Duration::from_secs(0));
        assert_eq!(aggregated.stop_timestamp, Duration::from_secs(15)); // updateで設定された最大値

        Ok(())
    }

    #[test]
    fn test_merge_overlapping_sources_multiple_overlaps() -> orfail::Result<()> {
        let mut aggregated = create_test_aggregated_source_info();

        // 様々な重複パターンを持つ複数のソースを追加
        let source1 = create_test_source_info(Duration::from_secs(0), Duration::from_secs(30));
        let source2 = create_test_source_info(Duration::from_secs(5), Duration::from_secs(15));
        let source3 = create_test_source_info(Duration::from_secs(10), Duration::from_secs(25));
        let source4 = create_test_source_info(Duration::from_secs(40), Duration::from_secs(50));

        aggregated.update(&source1, Path::new("source1.webm"));
        aggregated.update(&source2, Path::new("source2.webm"));
        aggregated.update(&source3, Path::new("source3.webm"));
        aggregated.update(&source4, Path::new("source4.webm"));

        aggregated.merge_overlapping_sources().or_fail()?;

        // - source1 (0-30) が最初に処理される (開始時刻0、終了時刻30で最長)
        // - source2,3は source1の終了時刻30より前に開始するため除外
        // - source4 (40-50) は source1の終了時刻30より後に開始するため残る
        assert_eq!(aggregated.media_paths.len(), 2);
        assert!(
            aggregated
                .media_paths
                .contains_key(Path::new("source1.webm"))
        );
        assert!(
            aggregated
                .media_paths
                .contains_key(Path::new("source4.webm"))
        );
        assert_eq!(aggregated.start_timestamp, Duration::from_secs(0));
        assert_eq!(aggregated.stop_timestamp, Duration::from_secs(50));

        Ok(())
    }

    #[test]
    fn test_merge_overlapping_sources_different_durations_same_start() -> orfail::Result<()> {
        let mut aggregated = create_test_aggregated_source_info();

        // 同じ開始時刻で異なる長さのソースを追加
        let short_source = create_test_source_info(Duration::from_secs(0), Duration::from_secs(10));
        let long_source = create_test_source_info(Duration::from_secs(0), Duration::from_secs(20));

        aggregated.update(&short_source, Path::new("short.webm"));
        aggregated.update(&long_source, Path::new("long.webm"));

        aggregated.merge_overlapping_sources().or_fail()?;

        // 同じ開始時刻の場合、長い方（終了時刻が後の方）が優先される
        assert_eq!(aggregated.media_paths.len(), 1);
        assert!(aggregated.media_paths.contains_key(Path::new("long.webm")));
        assert_eq!(aggregated.start_timestamp, Duration::from_secs(0));
        assert_eq!(aggregated.stop_timestamp, Duration::from_secs(20));

        Ok(())
    }

    #[test]
    fn test_merge_overlapping_sources_sequential() -> orfail::Result<()> {
        let mut aggregated = create_test_aggregated_source_info();

        // 連続するが重複しないソースを追加
        let source1 = create_test_source_info(Duration::from_secs(0), Duration::from_secs(10));
        let source2 = create_test_source_info(Duration::from_secs(10), Duration::from_secs(20)); // 境界で接触
        let source3 = create_test_source_info(Duration::from_secs(21), Duration::from_secs(30)); // 1秒の隙間

        aggregated.update(&source1, Path::new("source1.webm"));
        aggregated.update(&source2, Path::new("source2.webm"));
        aggregated.update(&source3, Path::new("source3.webm"));

        aggregated.merge_overlapping_sources().or_fail()?;

        // 境界で接触するソースは重複とみなさないため、すべてのソースが残る
        assert_eq!(aggregated.media_paths.len(), 3);
        assert!(
            aggregated
                .media_paths
                .contains_key(Path::new("source1.webm"))
        );
        assert!(
            aggregated
                .media_paths
                .contains_key(Path::new("source2.webm"))
        );
        assert!(
            aggregated
                .media_paths
                .contains_key(Path::new("source3.webm"))
        );

        Ok(())
    }

    // テスト用ヘルパー関数
    fn create_test_aggregated_source_info() -> AggregatedSourceInfo {
        AggregatedSourceInfo {
            id: SourceId::new("test_source"),
            format: ContainerFormat::Webm,
            audio: true,
            video: true,
            start_timestamp: Duration::MAX,
            stop_timestamp: Duration::ZERO,
            media_paths: BTreeMap::new(),
        }
    }

    fn create_test_source_info(start: Duration, stop: Duration) -> SourceInfo {
        SourceInfo {
            id: SourceId::new("test_source"),
            format: ContainerFormat::Webm,
            audio: true,
            video: true,
            start_timestamp: start,
            stop_timestamp: stop,
        }
    }
}
