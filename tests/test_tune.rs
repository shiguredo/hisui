//! `src/tune.rs` および `src/tune/` のエラーパス・境界の単体テスト
//!
//! 不変条件の検証は PBT (pbt/tests/prop_tune) が担うため、ここでは PBT で表しにくい
//! エラーパスや状態遷移 (ロック競合・破損行スキップ・中断再開・未知 trial への tell) を検証する。

use std::collections::BTreeMap;

use hisui::tune::json_value::{JsonObjectMemberPath, JsonValue};
use hisui::tune::storage::{self, TrialRecord, TrialResult};
use hisui::tune::{SearchSpace, TrialValues, Tuner};

/// 2 つのパラメータを持つ単純な探索空間を作る
fn build_test_search_space() -> SearchSpace {
    let json = r#"{
        "a": { "min": 0, "max": 10 },
        "b": ["x", "y", "z"]
    }"#;
    hisui::json::parse_str(json).expect("探索空間 JSON はパースできる")
}

#[test]
fn numeric_range_rejects_min_greater_than_max() {
    // min > max の数値範囲は探索空間のパース時点で拒否される
    // (これを通すと後段のサンプリング・交叉でパニックする)
    let json = r#"{ "a": { "min": 10, "max": 0 } }"#;
    let result = hisui::json::parse_str::<SearchSpace>(json);
    assert!(
        result.is_err(),
        "min > max の数値範囲はパースエラーになること"
    );
}

#[test]
fn lock_prevents_concurrent_start_and_releases_on_drop() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できる");
    let path = dir.path().to_path_buf();

    // 1 つ目のチューナーがロックを保持している間は、同名チューナーを開けない
    let tuner1 = Tuner::new("test".to_owned(), path.clone()).expect("最初のチューナーは開ける");
    let tuner2 = Tuner::new("test".to_owned(), path.clone());
    assert!(tuner2.is_err(), "ロック保持中は二重に開けないこと");

    // ロックを解放すれば再度開ける
    drop(tuner1);
    let tuner3 = Tuner::new("test".to_owned(), path);
    assert!(tuner3.is_ok(), "ロック解放後は再取得できること");
}

#[test]
fn malformed_lines_are_skipped() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できる");
    let jsonl_path = dir.path().join("test.jsonl");

    // 正常なレコードを 1 件書き込む
    let mut params = BTreeMap::new();
    params.insert(
        "a".parse::<JsonObjectMemberPath>().expect("パスは無謬"),
        JsonValue::Integer(3),
    );
    let record = TrialRecord {
        trial_number: 0,
        params,
        result: TrialResult::Complete(TrialValues {
            elapsed_seconds: 1.5,
            vmaf_mean: 90.0,
        }),
    };
    storage::append_trial(&jsonl_path, &record).expect("正常レコードを追記できる");

    // 異常終了で途中まで書かれた壊れた行を末尾に足す
    std::fs::write(
        &jsonl_path,
        format!("{}\n{{ \"trial_number\": 1, \"par", nojson::Json(&record)),
    )
    .expect("壊れた行を含むファイルを書ける");

    // 壊れた行はスキップされ、正常な 1 件だけが読み込まれる
    let loaded = storage::load_trials(&jsonl_path).expect("読み込みは成功する");
    assert_eq!(loaded.len(), 1, "壊れた行はスキップされること");
    assert_eq!(loaded[0], record);
}

#[test]
fn resume_continues_count_and_numbering() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できる");
    let path = dir.path().to_path_buf();
    let search_space = build_test_search_space();

    // 1 回試行してから閉じる
    let first_number = {
        let mut tuner = Tuner::new("test".to_owned(), path.clone()).expect("チューナーを開ける");
        assert_eq!(tuner.trial_count(), 0, "最初は履歴ゼロ");
        let trial = tuner.ask(&search_space).expect("ask できる");
        tuner
            .tell(
                trial.number,
                &TrialValues {
                    elapsed_seconds: 2.0,
                    vmaf_mean: 80.0,
                },
            )
            .expect("tell できる");
        trial.number
    };
    assert_eq!(first_number, 0, "最初の trial 番号は 0");

    // 再開すると過去の履歴を引き継ぎ、採番は続きから始まる
    let mut tuner = Tuner::new("test".to_owned(), path).expect("再開できる");
    assert_eq!(tuner.trial_count(), 1, "再開時に既存の 1 件を引き継ぐこと");
    let trial = tuner.ask(&search_space).expect("再開後も ask できる");
    assert_eq!(trial.number, 1, "採番は既存の最大 + 1 から続くこと");
}

#[test]
fn tell_for_unknown_trial_errors() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できる");
    let mut tuner =
        Tuner::new("test".to_owned(), dir.path().to_path_buf()).expect("チューナーを開ける");

    // ask していない trial 番号への tell は失敗する
    let result = tuner.tell(
        999,
        &TrialValues {
            elapsed_seconds: 1.0,
            vmaf_mean: 50.0,
        },
    );
    assert!(result.is_err(), "未知の trial 番号への tell は失敗すること");
}

#[test]
fn failed_trials_counted_but_excluded_from_best() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できる");
    let search_space = build_test_search_space();
    let mut tuner =
        Tuner::new("test".to_owned(), dir.path().to_path_buf()).expect("チューナーを開ける");

    // 1 件目は失敗させる
    let trial0 = tuner.ask(&search_space).expect("ask できる");
    tuner.tell_fail(trial0.number).expect("tell_fail できる");

    // 2 件目は成功させる
    let trial1 = tuner.ask(&search_space).expect("ask できる");
    tuner
        .tell(
            trial1.number,
            &TrialValues {
                elapsed_seconds: 1.0,
                vmaf_mean: 95.0,
            },
        )
        .expect("tell できる");

    // 失敗・成功の両方が件数 (採番ベース) には数えられる
    assert_eq!(tuner.trial_count(), 2, "失敗も件数には数えること");

    // ベストトライアルには成功した 1 件だけが含まれる
    let (_, best) = tuner
        .get_best_trials()
        .expect("ベストトライアルを取得できる");
    assert_eq!(best.len(), 1, "成功した試行だけがベストに入ること");
    assert_eq!(best[0].number, trial1.number);
}
