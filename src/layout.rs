use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

use orfail::OrFail;

use crate::{
    metadata::{ArchiveMetadata, ContainerFormat, RecordingMetadata, SourceId, SourceInfo},
    types::{EvenUsize, PixelPosition},
    video::{FrameRate, VideoFrame},
};

// セルの枠線のピクセル数
// なお外枠のピクセル数は、解像度やその他の要因によって、これより大きくなったり小さくなったりすることがある
const BORDER_PIXELS: EvenUsize = EvenUsize::truncating_new(2);

/// 合成レイアウト
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub base_path: PathBuf,
    // z-pos 順に並んだリージョン列
    pub video_regions: Vec<Region>,
    // トリム開始時刻から終了時刻へのマップ
    pub trim_spans: BTreeMap<Duration, Duration>,
    pub resolution: Resolution,

    pub bitrate_kbps: usize,

    pub audio_source_ids: BTreeSet<SourceId>,
    pub sources: BTreeMap<SourceId, AggregatedSourceInfo>,

    // 以降は JSON には含まれないフィールド
    pub fps: FrameRate,
}

impl Layout {
    /// レイアウト JSON ファイルで指示されたレイアウトを作成する
    pub fn from_layout_json(
        layout_file_path: &Path,
        json: &str,
        fps: FrameRate,
    ) -> orfail::Result<Self> {
        let base_path = layout_file_path.parent().or_fail()?.to_path_buf();
        let raw: RawLayout = json.parse().map(|nojson::Json(v)| v).or_fail()?;
        raw.into_layout(base_path, fps).or_fail()
    }

    /// recording.report から合成レイアウトを作成する
    pub fn from_recording_report(
        report_file_path: &Path,
        report: &RecordingMetadata,
        audio_only: bool,
        max_columns: usize,
        fps: FrameRate,
    ) -> orfail::Result<Self> {
        let base_path = report_file_path.parent().or_fail()?.to_path_buf();

        // layout 未指定の場合にはセルの解像度は固定
        // （ただし内枠分が引かれるのでもう少し小さくなることがある）
        let cell_width = 320;
        let cell_height = 240;

        let (rows, columns) = if audio_only {
            (1, 1)
        } else {
            decide_grid_dimensions(0, max_columns, report.archives.len())
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
        let raw: RawLayout = layout_json
            .to_string()
            .parse()
            .map(|nojson::Json(v)| v)
            .or_fail()?;
        raw.into_layout(base_path, fps).or_fail()
    }

    pub fn video_bitrate_bps(&self) -> usize {
        if self.bitrate_kbps == 0 {
            (200 * self.video_source_ids().count()) * 1024
        } else {
            self.bitrate_kbps * 1024
        }
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

    pub fn is_in_trim_span(&self, timestamp: Duration) -> bool {
        if let Some((&start, &end)) = self.trim_spans.range(..=timestamp).next_back() {
            (start..end).contains(&timestamp)
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawLayout {
    audio_sources: Vec<PathBuf>,
    audio_sources_excluded: Vec<PathBuf>,
    video_layout: BTreeMap<String, RawRegion>,
    trim: bool,
    resolution: Resolution,
    bitrate: usize,
}

impl<'text> nojson::FromRawJsonValue<'text> for RawLayout {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        let ([audio_sources, resolution], [audio_sources_excluded, video_layout, trim, bitrate]) =
            value.to_fixed_object(
                ["audio_sources", "resolution"],
                ["audio_sources_excluded", "video_layout", "trim", "bitrate"],
            )?;
        Ok(Self {
            audio_sources: audio_sources.try_to()?,
            audio_sources_excluded: audio_sources_excluded
                .map(|v| v.try_to())
                .transpose()?
                .unwrap_or_default(),
            video_layout: video_layout
                .map(|v| v.try_to())
                .transpose()?
                .unwrap_or_default(),
            trim: trim.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
            resolution: resolution.try_to()?,
            bitrate: bitrate.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
        })
    }
}

impl RawLayout {
    fn into_layout(mut self, base_path: PathBuf, fps: FrameRate) -> orfail::Result<Layout> {
        for (name, region) in &mut self.video_layout {
            // Check height.
            if region.height != 0 {
                (16..=self.resolution.height.get())
                    .contains(&region.height)
                    .or_fail_with(|()| {
                        format!(
                            "video_layout.{name}.height is out of range: {}",
                            region.height
                        )
                    })?;
                region.height -= region.height % 2;
            } else {
                // 0 の場合は自動で求める
                region.height = self.resolution.height.get().saturating_sub(region.y_pos);
            }

            // Check width.
            if region.width != 0 {
                (16..=self.resolution.width.get())
                    .contains(&region.width)
                    .or_fail_with(|()| {
                        format!(
                            "video_layout.{name}.width is out of range: {}",
                            region.width
                        )
                    })?;
                region.width -= region.width % 2;
            } else {
                // 0 の場合は自動で求める
                region.width = self.resolution.width.get().saturating_sub(region.x_pos);
            }

            // Check y_pos.
            (0..self.resolution.height.get())
                .contains(&region.y_pos)
                .or_fail_with(|()| {
                    format!(
                        "video_layout.{name}.y_pos is out of range: {}",
                        region.y_pos
                    )
                })?;

            // Check x_pos.
            (0..self.resolution.width.get())
                .contains(&region.x_pos)
                .or_fail_with(|()| {
                    format!(
                        "video_layout.{name}.x_pos is out of range: {}",
                        region.x_pos
                    )
                })?;

            // Check z_pos.
            (-99..=99).contains(&region.z_pos).or_fail_with(|()| {
                format!(
                    "video_layout.{name}.z_pos is out of range: {}",
                    region.z_pos
                )
            })?;
        }

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
                .into_region(&base_path, &mut sources, &self.resolution)
                .or_fail()?;
            video_regions.push(region);
        }
        video_regions.sort_by_key(|r| r.z_pos);

        let mut trim_spans = BTreeMap::new();
        if self.trim {
            trim_spans = decide_trim_spans(&sources);
        }

        Ok(Layout {
            base_path,
            video_regions,
            trim_spans,
            resolution: self.resolution,
            bitrate_kbps: self.bitrate,
            audio_source_ids,
            sources,
            fps,
        })
    }
}

/// 映像リージョン
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub grid: Grid,
    pub source_ids: BTreeSet<SourceId>,
    pub height: EvenUsize,
    pub width: EvenUsize,
    pub position: PixelPosition,
    pub z_pos: isize,
    pub top_border_pixels: EvenUsize,
    pub left_border_pixels: EvenUsize,
}

impl Region {
    pub fn decide_frame_size(&self, frame: &VideoFrame) -> (EvenUsize, EvenUsize) {
        let width_ratio = self.grid.cell_width.get() as f64 / frame.width.get() as f64;
        let height_ratio = self.grid.cell_height.get() as f64 / frame.height.get() as f64;
        let ratio = if width_ratio < height_ratio {
            // 横に合わせて上下に黒帯を入れる
            width_ratio
        } else {
            // 縦に合わせて左右に黒帯を入れる
            height_ratio
        };
        (
            EvenUsize::truncating_new((frame.width.get() as f64 * ratio).floor() as usize),
            EvenUsize::truncating_new((frame.height.get() as f64 * ratio).floor() as usize),
        )
    }

    pub fn cell_position(&self, cell_index: usize) -> PixelPosition {
        let cell = self.grid.get_cell_position(cell_index);
        let mut x = self.position.x + self.left_border_pixels;
        let mut y = self.position.y + self.top_border_pixels;

        x += self.grid.cell_width * cell.column + BORDER_PIXELS * cell.column;
        y += self.grid.cell_height * cell.row + BORDER_PIXELS * cell.row;

        PixelPosition { x, y }
    }
}

/// 映像リージョン
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawRegion {
    cells_excluded: Vec<usize>,
    height: usize,
    max_columns: usize,
    max_rows: usize,
    reuse: ReuseKind,
    video_sources: Vec<PathBuf>,
    video_sources_excluded: Vec<PathBuf>,
    width: usize,
    x_pos: usize,
    y_pos: usize,
    z_pos: isize,
}

impl<'text> nojson::FromRawJsonValue<'text> for RawRegion {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        let (
            [video_sources],
            [cells_excluded, height, max_columns, max_rows, reuse, video_sources_excluded, width, x_pos, y_pos, z_pos],
        ) = value.to_fixed_object(
            ["video_sources"],
            [
                "cells_excluded",
                "height",
                "max_columns",
                "max_rows",
                "reuse",
                "video_sources_excluded",
                "width",
                "x_pos",
                "y_pos",
                "z_pos",
            ],
        )?;
        Ok(Self {
            video_sources: video_sources.try_to()?,
            cells_excluded: cells_excluded
                .map(|v| v.try_to())
                .transpose()?
                .unwrap_or_default(),
            height: height.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
            max_columns: max_columns
                .map(|v| v.try_to())
                .transpose()?
                .unwrap_or_default(),
            max_rows: max_rows
                .map(|v| v.try_to())
                .transpose()?
                .unwrap_or_default(),
            reuse: reuse.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
            video_sources_excluded: video_sources_excluded
                .map(|v| v.try_to())
                .transpose()?
                .unwrap_or_default(),
            width: width.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
            x_pos: x_pos.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
            y_pos: y_pos.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
            z_pos: z_pos.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
        })
    }
}

impl RawRegion {
    fn into_region(
        self,
        base_path: &Path,
        sources: &mut BTreeMap<SourceId, AggregatedSourceInfo>,
        resolution: &Resolution,
    ) -> orfail::Result<Region> {
        let resolved = resolve_source_and_media_path_pairs(
            base_path,
            &self.video_sources,
            &self.video_sources_excluded,
        )
        .or_fail()?;

        let mut source_ids = BTreeSet::new();
        for (source, media_path) in resolved {
            sources
                .entry(source.id.clone())
                .or_default()
                .update(&source, &media_path);
            source_ids.insert(source.id);
        }

        let mut grid_sources = sources.clone();
        grid_sources.retain(|id, _| source_ids.contains(id));

        let max_sources = decide_max_simultaneous_sources(&grid_sources, &self.cells_excluded);
        let (rows, columns) = decide_grid_dimensions(self.max_rows, self.max_columns, max_sources);
        let assigned = assign_sources(
            self.reuse,
            grid_sources.values().cloned().collect(),
            rows * columns,
            &self.cells_excluded,
        );

        let (cell_width, cell_height, top_border_pixels, left_border_pixels) = self
            .decide_cell_resolution_and_borders(rows, columns, resolution)
            .or_fail()?;
        let grid = Grid {
            assigned_sources: assigned,
            rows,
            columns,
            cell_width,
            cell_height,
        };

        Ok(Region {
            grid,
            source_ids,
            height: EvenUsize::truncating_new(self.height),
            width: EvenUsize::truncating_new(self.width),
            position: PixelPosition {
                x: EvenUsize::truncating_new(self.x_pos), // 偶数位置に丸める
                y: EvenUsize::truncating_new(self.y_pos), // 同上
            },
            z_pos: self.z_pos,
            top_border_pixels,
            left_border_pixels,
        })
    }

    fn decide_cell_resolution_and_borders(
        &self,
        rows: usize,
        columns: usize,
        resolution: &Resolution,
    ) -> orfail::Result<(EvenUsize, EvenUsize, EvenUsize, EvenUsize)> {
        let mut grid_width = self.width;
        let mut grid_height = self.height;
        if grid_width != resolution.width.get() {
            grid_width = grid_width.checked_sub(BORDER_PIXELS.get() * 2).or_fail()?;
        }
        if grid_height != resolution.height.get() {
            grid_height = grid_height.checked_sub(BORDER_PIXELS.get() * 2).or_fail()?;
        }

        let horizontal_inner_borders = BORDER_PIXELS.get() * (columns - 1);
        let grid_width_without_inner_borders = grid_width.saturating_sub(horizontal_inner_borders);

        let cell_width = EvenUsize::truncating_new(grid_width_without_inner_borders / columns);

        let vertical_inner_borders = BORDER_PIXELS.get() * (rows - 1);
        let grid_height_without_inner_borders = grid_height.saturating_sub(vertical_inner_borders);

        let cell_height = EvenUsize::truncating_new(grid_height_without_inner_borders / rows);

        let vertical_outer_borders =
            self.height - (cell_height.get() * rows + vertical_inner_borders);
        let horizontal_outer_borders =
            self.width - (cell_width.get() * columns + horizontal_inner_borders);

        let top_border_pixels = EvenUsize::truncating_new(vertical_outer_borders / 2);
        let left_border_pixels = EvenUsize::truncating_new(horizontal_outer_borders / 2);
        Ok((
            cell_width,
            cell_height,
            top_border_pixels,
            left_border_pixels,
        ))
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
    fn new(cell_index: usize, priority: usize) -> Self {
        Self {
            cell_index,
            priority,
        }
    }
}

/// 各リージョンのグリッドの情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    /// 各ソースがどのセルに割り当てられているか
    pub assigned_sources: HashMap<SourceId, AssignedSource>,

    /// グリッドの行数
    pub rows: usize,

    /// グリッドの列数
    pub columns: usize,

    /// セルの幅
    pub cell_width: EvenUsize,

    /// セルの高さ
    pub cell_height: EvenUsize,
}

impl Grid {
    /// セルのインデックスから、対応する行列位置を返す
    pub fn get_cell_position(&self, cell_index: usize) -> CellPosition {
        let row = cell_index / self.columns;
        let column = cell_index % self.columns;
        CellPosition { row, column }
    }

    pub fn assign_source(&mut self, source_id: SourceId, cell_index: usize, priority: usize) {
        self.assigned_sources
            .insert(source_id, AssignedSource::new(cell_index, priority));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellPosition {
    pub row: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cell {
    Excluded,
    Fresh,
    Used(Duration), // 値は stop_timestamp
}

/// 各セルの再利用方法
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ReuseKind {
    /// 再利用しない
    None,

    /// 再利用する（競合時には開始時刻が早い映像ソースが優先される）
    #[default]
    ShowOldest,

    /// 再利用する（競合時には開始時刻が遅い映像ソースが優先される）
    ShowNewest,
}

impl<'text> nojson::FromRawJsonValue<'text> for ReuseKind {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        match value.to_unquoted_string_str()?.as_ref() {
            "none" => Ok(Self::None),
            "show_oldest" => Ok(Self::ShowOldest),
            "show_newest" => Ok(Self::ShowNewest),
            v => Err(nojson::JsonParseError::invalid_value(
                value,
                format!("unknown reuse kind: {v}"),
            )),
        }
    }
}

/// 映像の解像度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    width: EvenUsize,
    height: EvenUsize,
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

impl<'text> nojson::FromRawJsonValue<'text> for Resolution {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        let s = value.to_unquoted_string_str()?;

        let Some((Ok(width), Ok(height))) = s.split_once('x').map(|(w, h)| (w.parse(), h.parse()))
        else {
            return Err(nojson::JsonParseError::invalid_value(
                value,
                format!("invalid resolution: {s}"),
            ));
        };

        Self::new(width, height).map_err(|e| {
            nojson::JsonParseError::invalid_value(
                value,
                format!("invalid resolution: {}", e.message),
            )
        })
    }
}

/// 最大同時ソース数を求める
pub fn decide_max_simultaneous_sources(
    sources: &BTreeMap<SourceId, AggregatedSourceInfo>,
    cells_excluded: &[usize],
) -> usize {
    // ソース数はどんなに多くても数十オーダーだと思うので非効率だけど分かりやすい実装にしている
    let mut max = sources
        .values()
        .map(|s0| {
            sources
                .values()
                .filter(|s1| s0.is_overlapped_with(s1))
                .count()
        })
        .max()
        .unwrap_or_default();

    // 除外セルの分を考慮する
    for cell_index in BTreeSet::from_iter(cells_excluded.iter().copied()) {
        if cell_index < max {
            max += 1;
        } else {
            // そもそも範囲外のセルが除外指定されている場合はここに来る
            break;
        }
    }

    max
}

/// 行列の最大値指定と最大同時ソース数をもとに、実際に使用するグリッドの行数と列数を求める
///
/// なお max_rows ないし max_columns で 0 が指定されたら未指定扱いとなる
pub fn decide_grid_dimensions(
    mut max_rows: usize,
    mut max_columns: usize,
    max_sources: usize,
) -> (usize, usize) {
    // まずは以下の方針で制約がない場合の行列数を求める:
    // - `max_sources` を保持可能なサイズを確保する
    // - できるだけ正方形に近くなるようにする:
    //   - ただし、列は行よりも一つ値が大きくてもいい
    let mut columns = (max_sources as f32).sqrt().ceil().max(1.0) as usize;
    let mut rows = (columns - 1).max(1);
    if rows * columns < max_sources {
        // 正方形にしないと `max_sources` を保持できない
        rows += 1;
    }

    // 以降では `max_rows` と `max_columns` を考慮した調整を行う

    // コードを単純にするために、未指定は `usize::MAX` 扱いにする
    if max_rows == 0 {
        max_rows = usize::MAX;
    }
    if max_columns == 0 {
        max_columns = usize::MAX;
    }

    // 行と列のどちらを基準にして調整を行うかを決める
    let row_based_adjustment = match (rows <= max_rows, columns <= max_columns) {
        (true, true) => return (rows, columns), // 制約を破っていないならここで終わり
        (false, true) => true,                  // 行の制約が破れているので行基準
        (true, false) => false,                 // 列の制約が破れているので列基準
        (false, false) => {
            // 両方ダメなら正方形に近くなるように最大値が小さい方を基準とする
            // (max_rows == max_columns の場合はどっちが基準となってもいい)
            max_rows < max_columns
        }
    };

    // 基準となった方に従い調整を行う
    if row_based_adjustment {
        rows = max_rows;
        columns = ((max_sources as f32 / rows as f32).ceil() as usize).clamp(1, max_columns);
    } else {
        columns = max_columns;
        rows = ((max_sources as f32 / columns as f32).ceil() as usize).clamp(1, max_rows);
    }

    (rows, columns)
}

/// ソースのセルへの割り当てを行う
pub fn assign_sources(
    reuse: ReuseKind,
    mut sources: Vec<AggregatedSourceInfo>,
    cells: usize,
    cells_excluded: &[usize],
) -> HashMap<SourceId, AssignedSource> {
    let mut cells = vec![Cell::Fresh; cells];
    for &i in cells_excluded {
        cells[i] = Cell::Excluded;
    }
    sources.sort_by_key(|x| (x.start_timestamp, x.stop_timestamp));

    let mut assigned = HashMap::new();
    let mut priority = usize::MAX / 2;
    for source in &sources {
        match reuse {
            ReuseKind::None => {
                // Fresh なセルを探す
                if let Some(i) = cells.iter().position(|c| *c == Cell::Fresh) {
                    cells[i] = Cell::Used(source.stop_timestamp);
                    assigned.insert(source.id.clone(), AssignedSource::new(i, priority));
                } else {
                    // もう割り当て可能なセルがないのでここで終了
                    break;
                }
            }
            ReuseKind::ShowOldest | ReuseKind::ShowNewest => {
                // Fresh or Used だけどもう期間を過ぎているセルを探す
                if let Some(i) = cells.iter().position(|c| match *c {
                    Cell::Fresh => true,
                    Cell::Used(stop_timestamp) => stop_timestamp < source.start_timestamp,
                    _ => false,
                }) {
                    cells[i] = Cell::Used(source.stop_timestamp);
                    assigned.insert(source.id.clone(), AssignedSource::new(i, priority));
                    continue;
                }

                // 現時点で利用可能なセルがない場合には、終了時刻が一番早いセルを探す
                if let Some((i, stop_timestamp)) = cells
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| {
                        if let Cell::Used(stop_timestamp) = *c {
                            Some((i, stop_timestamp))
                        } else {
                            None
                        }
                    })
                    .min_by_key(|(i, t)| (*t, *i))
                {
                    cells[i] = Cell::Used(stop_timestamp.max(source.stop_timestamp));
                    if reuse == ReuseKind::ShowOldest {
                        priority += 1; // 古い方を優先する
                    } else {
                        priority -= 1; // 新しい方を優先する
                    }
                    assigned.insert(source.id.clone(), AssignedSource::new(i, priority));
                }
            }
        }
    }
    assigned
}

fn resolve_source_and_media_path_pairs(
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
        let path = std::path::absolute(base_path.join(source)).or_fail()?;
        paths.extend(glob(path).or_fail()?);
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

fn glob(path: PathBuf) -> orfail::Result<Vec<PathBuf>> {
    if !path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains('*'))
    {
        // 名前部分にワイルドカードを含んでいないなら通常のパスとして扱う
        path.exists()
            .or_fail_with(|()| format!("No such source file: {}", path.display()))?;
        return Ok(vec![path]);
    }

    // ここまで来たら名前部分にワイルドカードを含んでいるので展開する
    let wildcard_name = path.file_name().and_then(|name| name.to_str()).or_fail()?;

    let parent = path.parent().or_fail()?;
    parent
        .exists()
        .or_fail_with(|()| format!("No such source file directory: {}", parent.display()))?;

    let mut paths = Vec::new();
    for entry in path.parent().or_fail()?.read_dir().or_fail()? {
        let entry = entry.or_fail()?;
        if entry
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| is_wildcard_name_matched(wildcard_name, name))
        {
            paths.push(entry.path());
        }
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

fn decide_trim_spans<'a>(
    sources: &BTreeMap<SourceId, AggregatedSourceInfo>,
) -> BTreeMap<Duration, Duration> {
    // 時刻順でソートする
    let mut sources = sources
        .values()
        .map(|s| (s.start_timestamp, s.stop_timestamp))
        .collect::<Vec<_>>();
    sources.sort();

    let mut trim_spans = BTreeMap::new();
    let mut now = Duration::ZERO;
    for (start_timestamp, stop_timestamp) in sources {
        if now < start_timestamp {
            // 次のソースの開始時刻との間にギャップがあるのでトリムする
            trim_spans.insert(now, start_timestamp);
            now = stop_timestamp;
        } else {
            now = now.max(stop_timestamp);
        }
    }

    trim_spans
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AggregatedSourceInfo {
    // 以下のフィールド群は全ての分割ファイルで値が同じになる
    pub id: SourceId,
    pub format: ContainerFormat,
    pub audio: bool,
    pub video: bool,

    // 開始時刻（分割録画の場合には一番小さな値）
    pub start_timestamp: Duration,

    // 終了時刻（分割録画の場合には一番小さな値）
    pub stop_timestamp: Duration,

    // 録画ファイルのパスと対応する情報（分割録画の場合には複数になる）
    pub media_paths: BTreeMap<PathBuf, SourceInfo>,
}

impl AggregatedSourceInfo {
    fn update(&mut self, source_info: &SourceInfo, media_path: &Path) {
        self.id = source_info.id.clone();
        self.format = source_info.format;
        self.audio = source_info.audio;
        self.video = source_info.video;
        self.start_timestamp = self.start_timestamp.min(source_info.start_timestamp);
        self.stop_timestamp = self.stop_timestamp.max(source_info.stop_timestamp);
        self.media_paths
            .insert(media_path.to_path_buf(), source_info.clone());
    }

    // 二つのソースが時間的に重なる部分があるかどうかを求める
    fn is_overlapped_with(&self, other: &Self) -> bool {
        if self.start_timestamp <= other.start_timestamp {
            other.start_timestamp <= self.stop_timestamp
        } else {
            self.start_timestamp <= other.stop_timestamp
        }
    }
}

// 非公開構造体のテストは layout_test.rs ではなくこっちでやる
#[cfg(test)]
mod tests {
    use super::*;

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
            let _: RawLayout = serde_json::from_str(json).or_fail()?;
        }
        Ok(())
    }
}
