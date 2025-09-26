use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    time::Duration,
};

use orfail::OrFail;

use crate::{
    json::JsonObject,
    layout::{AggregatedSourceInfo, AssignedSource, Resolution},
    metadata::{SourceId, SourceInfo},
    types::{EvenUsize, PixelPosition},
    video::VideoFrame,
};

// セルの枠線のデフォルトのピクセル数
const DEFAULT_BORDER_PIXELS: EvenUsize = EvenUsize::truncating_new(2);

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
    pub inner_border_pixels: EvenUsize,
    pub background_color: [u8; 3], // RGB
}

impl Region {
    pub fn decide_frame_size(&self, frame: &VideoFrame) -> (EvenUsize, EvenUsize) {
        let width_ratio = self.grid.cell_width.get() as f64 / frame.width as f64;
        let height_ratio = self.grid.cell_height.get() as f64 / frame.height as f64;
        let ratio = if width_ratio < height_ratio {
            // 横に合わせて上下に黒帯を入れる
            width_ratio
        } else {
            // 縦に合わせて左右に黒帯を入れる
            height_ratio
        };
        (
            EvenUsize::truncating_new((frame.width as f64 * ratio).floor() as usize),
            EvenUsize::truncating_new((frame.height as f64 * ratio).floor() as usize),
        )
    }

    pub fn cell_position(&self, cell_index: usize) -> PixelPosition {
        let cell = self.grid.get_cell_position(cell_index);
        let mut x = self.position.x + self.left_border_pixels;
        let mut y = self.position.y + self.top_border_pixels;

        x += self.grid.cell_width * cell.column + self.inner_border_pixels * cell.column;
        y += self.grid.cell_height * cell.row + self.inner_border_pixels * cell.row;

        PixelPosition { x, y }
    }
}

/// 映像リージョン（生データ）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRegion {
    cells_excluded: Vec<usize>,
    height: usize,
    max_columns: usize,
    max_rows: usize,
    reuse: ReuseKind,
    video_sources: Vec<std::path::PathBuf>,
    video_sources_excluded: Vec<std::path::PathBuf>,
    width: usize,
    cell_width: usize,
    cell_height: usize,
    x_pos: usize,
    y_pos: usize,
    z_pos: isize,
    // セルの枠線のピクセル数
    // なお外枠のピクセル数は、解像度やその他の要因によって、これより大きくなったり小さくなったりすることがある
    border_pixels: EvenUsize,

    // 以降は開発者向けの undoc 項目
    background_color: [u8; 3], // RGB (デフォルトは `[0, 0, 0]`）
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for RawRegion {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let object = JsonObject::new(value)?;
        Ok(Self {
            video_sources: object.get_required("video_sources")?,
            cells_excluded: object.get("cells_excluded")?.unwrap_or_default(),
            width: object.get("width")?.unwrap_or_default(),
            height: object.get("height")?.unwrap_or_default(),
            max_columns: object.get("max_columns")?.unwrap_or_default(),
            max_rows: object.get("max_rows")?.unwrap_or_default(),
            reuse: object.get("reuse")?.unwrap_or_default(),
            video_sources_excluded: object.get("video_sources_excluded")?.unwrap_or_default(),
            cell_width: object.get("cell_width")?.unwrap_or_default(),
            cell_height: object.get("cell_height")?.unwrap_or_default(),
            x_pos: object.get("x_pos")?.unwrap_or_default(),
            y_pos: object.get("y_pos")?.unwrap_or_default(),
            z_pos: object.get("z_pos")?.unwrap_or_default(),
            border_pixels: object
                .get("border_pixels")?
                .unwrap_or(DEFAULT_BORDER_PIXELS),
            background_color: object.get("background_color")?.unwrap_or_default(),
        })
    }
}

impl RawRegion {
    pub fn into_region<F>(
        mut self,
        base_path: &Path,
        sources: &mut BTreeMap<SourceId, AggregatedSourceInfo>,
        resolution: Option<Resolution>,
        resolve: F,
    ) -> orfail::Result<Region>
    where
        F: Fn(&Path, &[PathBuf], &[PathBuf]) -> orfail::Result<Vec<(SourceInfo, PathBuf)>>,
    {
        if self.width != 0 && self.cell_width != 0 {
            return Err(orfail::Failure::new(
                "cannot specify both 'width' and 'cell_width' for the same region".to_owned(),
            ));
        }

        if self.height != 0 && self.cell_height != 0 {
            return Err(orfail::Failure::new(
                "cannot specify both 'height' and 'cell_height' for the same region".to_owned(),
            ));
        }

        let resolved =
            resolve(base_path, &self.video_sources, &self.video_sources_excluded).or_fail()?;

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

        let max_sources = decide_required_cells(&grid_sources, self.reuse, &self.cells_excluded);
        let (rows, columns) = decide_grid_dimensions(self.max_rows, self.max_columns, max_sources);
        let assigned = assign_sources(
            self.reuse,
            grid_sources.values().cloned().collect(),
            rows * columns,
            &self.cells_excluded,
        );

        if self.cell_width != 0 {
            let horizontal_inner_borders = self.border_pixels.get() * (columns - 1);
            let grid_width = self.cell_width * columns + horizontal_inner_borders;

            // 外枠を考慮
            self.width = if resolution
                .is_some_and(|r| grid_width + self.border_pixels.get() * 2 <= r.width.get())
            {
                grid_width + self.border_pixels.get() * 2
            } else {
                grid_width
            };
        }

        if self.cell_height != 0 {
            let vertical_inner_borders = self.border_pixels.get() * (rows - 1);
            let grid_height = self.cell_height * rows + vertical_inner_borders;

            // 外枠を考慮
            self.height = if resolution
                .is_some_and(|r| grid_height + self.border_pixels.get() * 2 <= r.height.get())
            {
                grid_height + self.border_pixels.get() * 2
            } else {
                grid_height
            };
        }

        // 解像度を確定する
        let resolution = if let Some(resolution) = resolution {
            resolution
        } else {
            // 全体の解像度が未指定の場合には、リージョンの width / height から求める
            (self.width > 0).or_fail_with(|()| {
                "Region width must be specified when resolution is not set".to_owned()
            })?;
            (self.height > 0).or_fail_with(|()| {
                "Region height must be specified when resolution is not set".to_owned()
            })?;

            // リージョンの位置とサイズから必要な解像度を計算
            let required_width = self.x_pos + self.width;
            let required_height = self.y_pos + self.height;
            Resolution::new(required_width, required_height).or_fail()?
        };

        // 高さの確認と調整
        if self.height != 0 {
            (16..=resolution.height.get())
                .contains(&self.height)
                .or_fail_with(|()| {
                    format!(
                        "video_layout region height is out of range: {}",
                        self.height
                    )
                })?;
            self.height -= self.height % 2; // 偶数にする
        } else {
            // 0 は自動計算を意味する
            self.height = resolution.height.get().saturating_sub(self.y_pos);
        }

        // 幅の確認と調整
        if self.width != 0 {
            (16..=resolution.width.get())
                .contains(&self.width)
                .or_fail_with(|()| {
                    format!("video_layout region width is out of range: {}", self.width)
                })?;
            self.width -= self.width % 2; // 偶数にする
        } else {
            // 0 は自動計算を意味する
            self.width = resolution.width.get().saturating_sub(self.x_pos);
        }

        // y_pos の確認
        (self.y_pos + self.height <= resolution.height.get()).or_fail_with(|()| {
            format!(
                "video_layout region y_pos + height ({}) exceeds resolution height ({})",
                self.y_pos + self.height,
                resolution.height.get()
            )
        })?;

        // x_pos の確認
        (self.x_pos + self.width <= resolution.width.get()).or_fail_with(|()| {
            format!(
                "video_layout region x_pos + width ({}) exceeds resolution width ({})",
                self.x_pos + self.width,
                resolution.width.get()
            )
        })?;

        // z_pos の確認
        (-99..=99).contains(&self.z_pos).or_fail_with(|()| {
            format!("video_layout region z_pos is out of range: {}", self.z_pos)
        })?;

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
            inner_border_pixels: self.border_pixels,
            background_color: self.background_color,
        })
    }

    fn decide_cell_resolution_and_borders(
        &self,
        rows: usize,
        columns: usize,
        resolution: Resolution,
    ) -> orfail::Result<(EvenUsize, EvenUsize, EvenUsize, EvenUsize)> {
        let mut grid_width = self.width;
        let mut grid_height = self.height;
        if grid_width != resolution.width.get() {
            grid_width = grid_width
                .checked_sub(self.border_pixels.get() * 2)
                .or_fail_with(|()| {
                    format!(
                        "vertical outer border size ({}*2) are larger than grid width {grid_width}",
                        self.border_pixels.get(),
                    )
                })?;
        }
        if grid_height != resolution.height.get() {
            grid_height = grid_height
                .checked_sub(self.border_pixels.get() * 2)
                .or_fail_with(|()| {
                    format!(
                        "horizontal outer border size ({}*2) are larger than grid height {grid_height}",
                        self.border_pixels.get(),
                    )
                })?;
        }

        let horizontal_inner_borders = self.border_pixels.get() * (columns - 1);
        let grid_width_without_inner_borders = grid_width.saturating_sub(horizontal_inner_borders);

        let cell_width = EvenUsize::truncating_new(grid_width_without_inner_borders / columns);

        let vertical_inner_borders = self.border_pixels.get() * (rows - 1);
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

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ReuseKind {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        match value.to_unquoted_string_str()?.as_ref() {
            "none" => Ok(Self::None),
            "show_oldest" => Ok(Self::ShowOldest),
            "show_newest" => Ok(Self::ShowNewest),
            v => Err(value.invalid(format!("unknown reuse kind: {v}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cell {
    Excluded,
    Fresh,
    Used(Duration), // 値は stop_timestamp
}

/// 行列の最大値指定と最大同時ソース数をもとに、実際に使用するグリッドの行数と列数を求める
///
/// なお max_rows ないし max_columns で 0 が指定されたら未指定扱いとなる
///
/// また、`max_rows * max_columns < max_sources` となる場合には `(max_rows, max_columns)` が結果として返される
pub fn decide_grid_dimensions(
    mut max_rows: usize,
    mut max_columns: usize,
    max_sources: usize,
) -> (usize, usize) {
    if max_rows == 0 {
        max_rows = usize::MAX;
    }
    if max_columns == 0 {
        max_columns = usize::MAX;
    }

    if max_rows == usize::MAX && max_columns == usize::MAX {
        // 以下の方針で制約がない場合の行列数を求める:
        // - `max_sources` を保持可能なサイズを確保する
        // - できるだけ正方形に近くなるようにする:
        //   - ただし、列は行よりも一つ値が大きくてもいい
        let columns = (max_sources as f32).sqrt().ceil().max(1.0) as usize;
        let mut rows = (columns - 1).max(1);
        if rows * columns < max_sources {
            // 正方形にしないと `max_sources` を保持できない
            rows += 1;
        }
        (rows, columns)
    } else if max_columns <= max_rows {
        // 列を先に埋める
        let columns = max_sources.min(max_columns).max(1);
        let rows = max_sources.div_ceil(max_columns).min(max_rows).max(1);
        (rows, columns)
    } else {
        // 行を先に埋める
        let rows = max_sources.min(max_rows).max(1);
        let columns = max_sources.div_ceil(max_rows).min(max_columns).max(1);
        (rows, columns)
    }
}

/// 全てのソースを表示するために必要なセルの数を求める
pub fn decide_required_cells(
    sources: &BTreeMap<SourceId, AggregatedSourceInfo>,
    reuse: ReuseKind,
    cells_excluded: &[usize],
) -> usize {
    let mut max = if reuse == ReuseKind::None {
        // セルの再利用をしない場合は、各ソースにつき一つのセルが消費されるため、
        // ソースと同じ数だけのセルが必要となる
        sources.len()
    } else {
        // セルを再利用する場合には、同時に表示されるソースの数だけのセルがあればいい
        // （各時刻で重複するソースの最大数を計算）
        //
        // なお、ソース数はどんなに多くても数十オーダーだと思うので非効率だけど分かりやすい実装にしている
        sources
            .values()
            .map(|s0| {
                sources
                    .values()
                    .filter(|s1| s0.is_overlapped_with(s1))
                    .count()
            })
            .max()
            .unwrap_or_default()
    };

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

/// ソースのセルへの割り当てを行う
pub fn assign_sources(
    reuse: ReuseKind,
    mut sources: Vec<AggregatedSourceInfo>,
    cells: usize,
    cells_excluded: &[usize],
) -> HashMap<SourceId, AssignedSource> {
    let mut cells = vec![Cell::Fresh; cells];
    for &i in cells_excluded {
        if i < cells.len() {
            cells[i] = Cell::Excluded;
        }
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
