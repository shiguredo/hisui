use std::collections::BTreeMap;

use hisui::tune::TrialValues;
use hisui::tune::json_value::{JsonObjectMemberPath, JsonValue};
use hisui::tune::storage::{TrialRecord, TrialResult};
use proptest::prelude::*;

// JSON Lines に保存する試行レコードのシリアライズ / デシリアライズのラウンドトリップを検証する。
// 特に「整数値が整数のまま保たれる」ことを担保する (浮動小数として読み戻されるとバグ)。

// params の値として使うスカラー値の戦略
//
// `JsonValue::Float` は「整数に見える値」(例: 3.0) が整数として書き出されると
// 読み戻し時に `JsonValue::Integer` になりラウンドトリップが崩れるため、ここでは含めない。
// 浮動小数のラウンドトリップは `TrialValues` 側 (f64 として読み戻す) で検証される。
fn scalar_value() -> impl Strategy<Value = JsonValue> {
    prop_oneof![
        Just(JsonValue::Null),
        any::<bool>().prop_map(JsonValue::Boolean),
        (-100_000i64..100_000).prop_map(JsonValue::Integer),
        "[a-zA-Z0-9 _-]{0,8}".prop_map(JsonValue::String),
    ]
}

// パラメータの値の戦略 (スカラー、またはスカラーの浅い配列)
fn param_value() -> impl Strategy<Value = JsonValue> {
    prop_oneof![
        scalar_value(),
        prop::collection::vec(scalar_value(), 0..4).prop_map(JsonValue::Array),
    ]
}

// パラメータのパスキーの戦略 (ドットを含むセグメントは作らない)
fn path_key() -> impl Strategy<Value = JsonObjectMemberPath> {
    "[a-z][a-z0-9_]{0,5}(\\.[a-z][a-z0-9_]{0,5}){0,2}".prop_map(|s| {
        s.parse()
            .expect("JsonObjectMemberPath::from_str is infallible")
    })
}

// params (パス -> 値) の戦略
fn params() -> impl Strategy<Value = BTreeMap<JsonObjectMemberPath, JsonValue>> {
    prop::collection::btree_map(path_key(), param_value(), 0..6)
}

// 試行結果の戦略
fn trial_result() -> impl Strategy<Value = TrialResult> {
    prop_oneof![
        (0.0f64..100_000.0, 0.0f64..100.0).prop_map(|(elapsed_seconds, vmaf_mean)| {
            TrialResult::Complete(TrialValues {
                elapsed_seconds,
                vmaf_mean,
            })
        }),
        Just(TrialResult::Fail),
    ]
}

// 試行レコードの戦略
fn trial_record() -> impl Strategy<Value = TrialRecord> {
    (0usize..100_000, params(), trial_result()).prop_map(|(trial_number, params, result)| {
        TrialRecord {
            trial_number,
            params,
            result,
        }
    })
}

proptest! {
    // 試行レコードは JSON へシリアライズして読み戻すと元に戻る
    #[test]
    fn trial_record_roundtrip(record in trial_record()) {
        let serialized = nojson::Json(&record).to_string();
        let parsed = nojson::RawJson::parse(&serialized)
            .expect("シリアライズした JSON は必ずパースできる");
        let decoded = TrialRecord::try_from(parsed.value())
            .expect("シリアライズした試行レコードは必ずデコードできる");
        prop_assert_eq!(decoded, record);
    }

    // 1 行 1 レコードとして JSON Lines にしても各行が独立してパースできる
    #[test]
    fn json_lines_each_line_parses(records in prop::collection::vec(trial_record(), 0..10)) {
        let mut lines = String::new();
        for record in &records {
            lines.push_str(&nojson::Json(record).to_string());
            lines.push('\n');
        }

        let mut decoded = Vec::new();
        for line in lines.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let parsed = nojson::RawJson::parse(line).expect("各行はパースできる");
            decoded.push(TrialRecord::try_from(parsed.value()).expect("各行はデコードできる"));
        }
        prop_assert_eq!(decoded, records);
    }
}
