use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    time::Duration,
};

use hisui::{
    layout::{self, AggregatedSourceInfo, AssignedSource, Resolution},
    layout_region::{assign_sources, decide_grid_dimensions, decide_required_cells, ReuseKind},
    metadata::{SourceId, SourceInfo},
};
use orfail::OrFail;

#[test]
fn valid_resolutions() -> orfail::Result<()> {
    let valid_jsons = [r#""16x16""#, r#""3840x3840""#];
    for json in valid_jsons {
        json.parse::<nojson::Json<Resolution>>().or_fail()?;
    }

    // 値は 2 の倍数に丸められる
    for i in 0..2 {
        let json = format!(r#""{}x{}""#, 32 + i, 32 + i);
        let v = json.parse::<nojson::Json<Resolution>>().or_fail()?;
        assert_eq!(v.0.width().get(), 32);
        assert_eq!(v.0.height().get(), 32);
    }

    Ok(())
}

#[test]
fn decide_grid_dimensions_works() {
    // max_rows / max_columns の両方が未指定の場合
    assert_eq!(decide_grid_dimensions(0, 0, 1), (1, 1));
    assert_eq!(decide_grid_dimensions(0, 0, 2), (1, 2));
    assert_eq!(decide_grid_dimensions(0, 0, 3), (2, 2));
    assert_eq!(decide_grid_dimensions(0, 0, 4), (2, 2));
    assert_eq!(decide_grid_dimensions(0, 0, 5), (2, 3));
    assert_eq!(decide_grid_dimensions(0, 0, 6), (2, 3));
    assert_eq!(decide_grid_dimensions(0, 0, 7), (3, 3));
    assert_eq!(decide_grid_dimensions(0, 0, 9), (3, 3));
    assert_eq!(decide_grid_dimensions(0, 0, 10), (3, 4));
    assert_eq!(decide_grid_dimensions(0, 0, 12), (3, 4));
    assert_eq!(decide_grid_dimensions(0, 0, 17), (4, 5));
    assert_eq!(decide_grid_dimensions(0, 0, 20), (4, 5));

    // max_rows / max_columns の片方が未指定の場合
    assert_eq!(decide_grid_dimensions(1, 0, 1), (1, 1));
    assert_eq!(decide_grid_dimensions(0, 1, 1), (1, 1));
    assert_eq!(decide_grid_dimensions(1, 0, 2), (1, 2));
    assert_eq!(decide_grid_dimensions(0, 1, 2), (2, 1));
    assert_eq!(decide_grid_dimensions(1, 0, 3), (1, 3));
    assert_eq!(decide_grid_dimensions(0, 1, 3), (3, 1));
    assert_eq!(decide_grid_dimensions(2, 0, 4), (2, 2));
    assert_eq!(decide_grid_dimensions(0, 2, 4), (2, 2));
    assert_eq!(decide_grid_dimensions(2, 0, 5), (2, 3));
    assert_eq!(decide_grid_dimensions(0, 2, 5), (3, 2));
    assert_eq!(decide_grid_dimensions(2, 0, 6), (2, 3));
    assert_eq!(decide_grid_dimensions(0, 2, 6), (3, 2));
    assert_eq!(decide_grid_dimensions(2, 0, 7), (2, 4));
    assert_eq!(decide_grid_dimensions(0, 2, 7), (4, 2));
    assert_eq!(decide_grid_dimensions(2, 0, 9), (2, 5));
    assert_eq!(decide_grid_dimensions(0, 2, 9), (5, 2));
    assert_eq!(decide_grid_dimensions(2, 0, 12), (2, 6));
    assert_eq!(decide_grid_dimensions(0, 2, 12), (6, 2));

    // max_rows / max_columns の両方が指定されている場合
    assert_eq!(decide_grid_dimensions(1, 1, 1), (1, 1));
    assert_eq!(decide_grid_dimensions(1, 2, 1), (1, 1));
    assert_eq!(decide_grid_dimensions(2, 2, 1), (1, 1));
    assert_eq!(decide_grid_dimensions(1, 1, 2), (1, 1));
    assert_eq!(decide_grid_dimensions(1, 2, 2), (1, 2));
    assert_eq!(decide_grid_dimensions(2, 2, 2), (1, 2));
    assert_eq!(decide_grid_dimensions(1, 1, 3), (1, 1));
    assert_eq!(decide_grid_dimensions(1, 2, 3), (1, 2));
    assert_eq!(decide_grid_dimensions(2, 2, 3), (2, 2));
    assert_eq!(decide_grid_dimensions(1, 1, 4), (1, 1));
    assert_eq!(decide_grid_dimensions(1, 2, 4), (1, 2));
    assert_eq!(decide_grid_dimensions(2, 2, 4), (2, 2));
    assert_eq!(decide_grid_dimensions(1, 1, 5), (1, 1));
    assert_eq!(decide_grid_dimensions(1, 2, 5), (1, 2));
    assert_eq!(decide_grid_dimensions(2, 2, 5), (2, 2));
    assert_eq!(decide_grid_dimensions(1, 7, 9), (1, 7));
    assert_eq!(decide_grid_dimensions(2, 7, 9), (2, 5));
    assert_eq!(decide_grid_dimensions(3, 7, 9), (3, 3));
}

#[test]
fn decide_required_cells_works() {
    // https://s3.amazonaws.com/com.twilio.prod.twilio-docs/images/composer_understanding_trim.original.png
    let source0 = source(0, 2);
    let source1 = source(1, 1);
    let source2 = source(4, 6);
    let source3 = source(5, 7);
    let source4 = source(6, 8);
    let sources = [source0, source1, source2, source3, source4]
        .into_iter()
        .map(|s| {
            (
                s.id.clone(),
                AggregatedSourceInfo {
                    id: s.id,
                    start_timestamp: s.start_timestamp,
                    stop_timestamp: s.stop_timestamp,
                    audio: true,
                    video: true,
                    format: Default::default(),
                    media_paths: Default::default(),
                },
            )
        })
        .collect();

    // [再利用あり] 除外セルなし
    let kind = ReuseKind::ShowOldest;
    let cells_excluded = [];
    assert_eq!(decide_required_cells(&sources, kind, &cells_excluded), 3);
    assert_eq!(
        decide_required_cells(
            &sources.clone().into_iter().take(2).collect(),
            kind,
            &cells_excluded
        ),
        2
    );

    // [再利用あり] 除外セルあり
    let cells_excluded = [1, 3];
    assert_eq!(decide_required_cells(&sources, kind, &cells_excluded), 5);

    let cells_excluded = [2];
    assert_eq!(decide_required_cells(&sources, kind, &cells_excluded), 4);

    // [再利用あり] 除外セルがあるけど、範囲外なので考慮されない
    let cells_excluded = [3];
    assert_eq!(decide_required_cells(&sources, kind, &cells_excluded), 3);

    // [再利用なし] 除外セルなし
    let kind = ReuseKind::None;
    let cells_excluded = [];
    assert_eq!(decide_required_cells(&sources, kind, &cells_excluded), 5); // ソース数と同じ
    assert_eq!(
        decide_required_cells(
            &sources.clone().into_iter().take(2).collect(),
            kind,
            &cells_excluded
        ),
        2
    ); // ソース数と同じ

    // [再利用なし] 除外セルあり
    let cells_excluded = [1, 3];
    assert_eq!(decide_required_cells(&sources, kind, &cells_excluded), 7); // ソース数 + 除外セル数（範囲内）

    let cells_excluded = [2];
    assert_eq!(decide_required_cells(&sources, kind, &cells_excluded), 6); // ソース数 + 除外セル数（範囲内）

    // [再利用なし] 除外セルがあるけど、範囲外なので考慮されない
    let cells_excluded = [5, 10];
    assert_eq!(decide_required_cells(&sources, kind, &cells_excluded), 5); // ソース数と同じ（除外セルは範囲外）

    // [再利用なし] 空のソース
    let empty_sources = BTreeMap::new();
    assert_eq!(decide_required_cells(&empty_sources, kind, &[]), 0);
    assert_eq!(decide_required_cells(&empty_sources, kind, &[1, 2]), 0); // 除外セルがあっても0
}

#[test]
fn assign_sources_works() {
    // https://s3.amazonaws.com/com.twilio.prod.twilio-docs/images/composer_understanding_trim.original.png
    let source0 = source(0, 2);
    let source1 = source(1, 1);
    let source2 = source(4, 6);
    let source3 = source(5, 7);
    let source4 = source(6, 8);
    let sources = [source0, source1, source2, source3, source4]
        .into_iter()
        .map(|s| {
            (
                s.id.clone(),
                AggregatedSourceInfo {
                    id: s.id.clone(),
                    start_timestamp: s.start_timestamp,
                    stop_timestamp: s.stop_timestamp,
                    audio: true,
                    video: true,
                    format: Default::default(),
                    media_paths: Default::default(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    // このテストでは除外セルはなし
    let cells_excluded = [];

    // 指定された時間に指定されたセルに割り当てられたソースのインデックスを返す
    fn get_assigned_source(
        assigned: &HashMap<SourceId, AssignedSource>,
        sources: &BTreeMap<SourceId, AggregatedSourceInfo>,
        timestamp: u64,
        cell_index: usize,
    ) -> Option<usize> {
        let t = Duration::from_secs(timestamp);
        sources
            .values()
            .enumerate()
            .filter(|(_i, s)| (s.start_timestamp..=s.stop_timestamp).contains(&t))
            .filter_map(|(i, s)| assigned.get(&s.id).map(|v| (i, v)))
            .filter(|(_i, s)| s.cell_index == cell_index)
            .map(|(i, s)| (s.priority, i))
            .min()
            .map(|(_priority, i)| i)
    }

    // 1x1 region, ReuseKind::None
    let assigned = assign_sources(
        ReuseKind::None,
        sources.values().cloned().collect(),
        1,
        &cells_excluded,
    );
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 0), None);

    // 1x1 region, ReuseKind::ShowOldest
    let assigned = assign_sources(
        ReuseKind::ShowOldest,
        sources.values().cloned().collect(),
        1,
        &cells_excluded,
    );
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 0), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 0), Some(4));

    // 1x1 region, ReuseKind::ShowNewest
    let assigned = assign_sources(
        ReuseKind::ShowNewest,
        sources.values().cloned().collect(),
        1,
        &cells_excluded,
    );
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 0), Some(1));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 0), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 0), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 0), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 0), Some(4));

    // 1x2 region, ReuseKind::None
    let assigned = assign_sources(
        ReuseKind::None,
        sources.values().cloned().collect(),
        2,
        &cells_excluded,
    );
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 1), Some(1));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 1), None);

    // 1x2 region, ReuseKind::ShowOldest
    let assigned = assign_sources(
        ReuseKind::ShowOldest,
        sources.values().cloned().collect(),
        2,
        &cells_excluded,
    );
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 1), Some(1));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 0), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 0), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 1), None);

    // 1x2 region, ReuseKind::ShowNewest
    let assigned = assign_sources(
        ReuseKind::ShowNewest,
        sources.values().cloned().collect(),
        2,
        &cells_excluded,
    );
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 1), Some(1));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 0), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 0), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 0), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 1), None);

    // 2x3 region, ReuseKind::None
    let assigned = assign_sources(
        ReuseKind::None,
        sources.values().cloned().collect(),
        2 * 3,
        &cells_excluded,
    );
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 4), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 5), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 1), Some(1));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 4), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 5), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 4), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 5), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 4), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 5), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 2), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 4), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 5), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 2), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 3), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 4), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 5), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 2), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 3), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 4), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 5), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 3), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 4), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 5), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 4), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 5), None);

    // 2x2 region, ReuseKind::ShowOldest
    let assigned = assign_sources(
        ReuseKind::ShowOldest,
        sources.values().cloned().collect(),
        2 * 2,
        &cells_excluded,
    );
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 1), Some(1));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 2), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 2), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 2), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 3), None);

    // 2x2 region, ReuseKind::ShowNewest
    let assigned = assign_sources(
        ReuseKind::ShowNewest,
        sources.values().cloned().collect(),
        2 * 2,
        &cells_excluded,
    );
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 0, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 1), Some(1));
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 1, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 0), Some(0));
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 2, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 3, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 4, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 2), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 5, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 0), Some(2));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 2), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 6, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 1), Some(3));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 2), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 7, 3), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 0), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 1), None);
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 2), Some(4));
    assert_eq!(get_assigned_source(&assigned, &sources, 8, 3), None);
}

#[test]
fn invalid_resolutions() -> orfail::Result<()> {
    let invalid_jsons = [
        // width が小さすぎる
        r#""15x20""#,
        // width が大きすぎる
        r#""3841x20""#,
        // height が小さすぎる
        r#""100x15""#,
        // height が大きすぎる
        r#""30x3841""#,
        // width がない
        r#""x100""#,
        // height がない
        r#""100x""#,
        // width が float
        r#""100.0x100""#,
        // height が float
        r#""100x100.0""#,
    ];
    for json in invalid_jsons {
        assert!(json.parse::<nojson::Json<Resolution>>().is_err());
    }
    Ok(())
}

fn source(start: u64, end: u64) -> SourceInfo {
    SourceInfo {
        id: SourceId::new(&format!("{start}_{end}")),
        start_timestamp: Duration::from_secs(start),
        stop_timestamp: Duration::from_secs(end),

        // 以下はダミー値
        audio: true,
        video: true,
        format: Default::default(),
    }
}

#[test]
fn source_wildcard() -> orfail::Result<()> {
    let base_path = PathBuf::from("testdata/files/").canonicalize().or_fail()?;
    let to_absolute = |path| std::path::absolute(base_path.join(path)).or_fail();

    // ソースパスを明示的に指定
    let resolved = layout::resolve_source_paths(
        &base_path,
        &[PathBuf::from("bar-0.json"), PathBuf::from("foo-1.json")],
        &[],
    )
    .or_fail()?;
    assert_eq!(
        resolved,
        &[to_absolute("bar-0.json")?, to_absolute("foo-1.json")?]
    );

    // ソースパスと除外パスを明示的に指定
    let resolved = layout::resolve_source_paths(
        &base_path,
        &[PathBuf::from("bar-0.json"), PathBuf::from("foo-1.json")],
        &[
            PathBuf::from("foo-1.json"),
            PathBuf::from("foo-2.json"), // こっちはマッチしない
        ],
    )
    .or_fail()?;
    assert_eq!(resolved, &[to_absolute("bar-0.json")?]);

    // ソースパスをワイルドカードで指定
    let resolved =
        layout::resolve_source_paths(&base_path, &[PathBuf::from("foo-*.json")], &[]).or_fail()?;
    assert_eq!(
        resolved,
        &[
            to_absolute("foo-0.json")?,
            to_absolute("foo-1.json")?,
            to_absolute("foo-2.json")?
        ]
    );

    // ソースパスと除外パスをワイルドカードで指定
    let resolved = layout::resolve_source_paths(
        &base_path,
        &[PathBuf::from("*")],
        &[PathBuf::from("*-1.json")],
    )
    .or_fail()?;
    assert_eq!(
        resolved,
        &[
            to_absolute("bar-0.json")?,
            to_absolute("bar-2.json")?,
            to_absolute("baz-0.json")?,
            to_absolute("baz-2.json")?,
            to_absolute("foo-0.json")?,
            to_absolute("foo-2.json")?
        ]
    );

    // ワイルドカードと通常パスの混合
    let resolved = layout::resolve_source_paths(
        &base_path,
        &[
            PathBuf::from("foo-2.json"),
            PathBuf::from("bar-*.json"),
            PathBuf::from("baz-0.json"),
        ],
        &[PathBuf::from("bar-2.json"), PathBuf::from("baz-*.json")],
    )
    .or_fail()?;
    assert_eq!(
        resolved,
        &[
            to_absolute("foo-2.json")?,
            to_absolute("bar-0.json")?,
            to_absolute("bar-1.json")?,
        ]
    );

    Ok(())
}

#[test]
fn source_path_outside_base_dir_error() -> orfail::Result<()> {
    // ベースディレクトリの外をレイアウトの中で参照した場合にはエラーにする
    let base_path = PathBuf::from("testdata/files/").canonicalize().or_fail()?;

    let result =
        layout::resolve_source_paths(&base_path, &[PathBuf::from("../layouts/layout0.json")], &[]);
    dbg!(&result);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("outside the base dir"));

    Ok(())
}
