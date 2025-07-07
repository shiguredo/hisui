use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use orfail::OrFail;

use crate::{
    layout::{
        AggregatedSourceInfo, BORDER_PIXELS, Grid, Resolution, ReuseKind, assign_sources,
        decide_grid_dimensions, decide_max_simultaneous_sources,
        resolve_source_and_media_path_pairs,
    },
    metadata::SourceId,
    types::{EvenUsize, PixelPosition},
    video::VideoFrame,
};

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
    cell_width: usize,  // TODO: ユニットテスト追加
    cell_height: usize, // TODO: ユニットテスト追加
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
            [
                cells_excluded,
                height,
                max_columns,
                max_rows,
                reuse,
                video_sources_excluded,
                width,
                cell_width,
                cell_height,
                x_pos,
                y_pos,
                z_pos,
            ],
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
                "cell_width",
                "cell_height",
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
            cell_width: cell_width
                .map(|v| v.try_to())
                .transpose()?
                .unwrap_or_default(),
            cell_height: cell_height
                .map(|v| v.try_to())
                .transpose()?
                .unwrap_or_default(),
            x_pos: x_pos.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
            y_pos: y_pos.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
            z_pos: z_pos.map(|v| v.try_to()).transpose()?.unwrap_or_default(),
        })
    }
}

impl RawRegion {
    pub fn into_region(
        mut self,
        base_path: &Path,
        sources: &mut BTreeMap<SourceId, AggregatedSourceInfo>,
        resolution: Option<Resolution>,
    ) -> orfail::Result<Region> {
        if self.width != 0 && self.cell_width != 0 {
            return Err(orfail::Failure::new(
                "Cannot specify both 'width' and 'cell_width' for the same region".to_owned(),
            ));
        }

        if self.height != 0 && self.cell_height != 0 {
            return Err(orfail::Failure::new(
                "Cannot specify both 'height' and 'cell_height' for the same region".to_owned(),
            ));
        }

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

        if self.cell_width != 0 {
            let horizontal_inner_borders = BORDER_PIXELS.get() * (columns - 1);
            let grid_width = self.cell_width * columns + horizontal_inner_borders;

            // 外枠を考慮
            self.width = if resolution
                .is_some_and(|r| grid_width + BORDER_PIXELS.get() * 2 <= r.width.get())
            {
                grid_width + BORDER_PIXELS.get() * 2
            } else {
                grid_width
            };
        }

        if self.cell_height != 0 {
            let vertical_inner_borders = BORDER_PIXELS.get() * (rows - 1);
            let grid_height = self.cell_height * rows + vertical_inner_borders;

            // 外枠を考慮
            self.height = if resolution
                .is_some_and(|r| grid_height + BORDER_PIXELS.get() * 2 <= r.height.get())
            {
                grid_height + BORDER_PIXELS.get() * 2
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
        (0..resolution.height.get())
            .contains(&self.y_pos)
            .or_fail_with(|()| {
                format!("video_layout region y_pos is out of range: {}", self.y_pos)
            })?;

        // x_pos の確認
        (0..resolution.width.get())
            .contains(&self.x_pos)
            .or_fail_with(|()| {
                format!("video_layout region x_pos is out of range: {}", self.x_pos)
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
