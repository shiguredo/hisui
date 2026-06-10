//! `src/tune.rs` および `src/tune/` のエラーパス・境界の単体テスト
//!
//! 不変条件の検証は PBT (pbt/tests/prop_tune) が担うため、ここでは PBT で表しにくい
//! エラーパスや状態遷移 (ロック競合・破損行スキップ・中断再開・未知 trial への tell) を検証する。

use std::collections::BTreeMap;

use hisui::tune::json_value::{JsonObjectMemberPath, JsonValue};
use hisui::tune::storage::{self, LockGuard, TrialRecord, TrialResult};
use hisui::tune::{SearchSpace, Trial, TrialValues, Tuner};

/// 2 つのパラメータを持つ単純な探索空間を作る
fn build_test_search_space() -> SearchSpace {
    let json = r#"{
        "a": { "min": 0, "max": 10 },
        "b": ["x", "y", "z"]
    }"#;
    hisui::json::parse_str(json).expect("探索空間 JSON はパースできる")
}

/// テスト用の試行レコードを 1 件作る (params は trial_number を値に持つ単一パラメータ)
fn sample_record(trial_number: usize, result: TrialResult) -> TrialRecord {
    let mut params = BTreeMap::new();
    params.insert(
        "a".parse::<JsonObjectMemberPath>().expect("パスは無謬"),
        JsonValue::Integer(trial_number as i64),
    );
    TrialRecord {
        trial_number,
        params,
        result,
    }
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
fn stale_lock_is_reclaimed() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できる");
    let lock_path = dir.path().join("test.lock");

    // 既に終了した (reap 済みの) プロセスの PID を用意する
    let mut child = std::process::Command::new("true")
        .spawn()
        .expect("子プロセスを起動できる");
    let dead_pid = child.id();
    child.wait().expect("子プロセスの終了を待てる");

    // その死んだ PID を持つロックファイルを置く (中断で残った stale ロックを模擬)
    std::fs::write(&lock_path, format!("{dead_pid}\n")).expect("ロックファイルを書ける");

    // 保持者プロセスが既に終了しているので、acquire は stale ロックを自動回収して成功する
    let guard = LockGuard::acquire(lock_path);
    assert!(
        guard.is_ok(),
        "死んだプロセスの stale ロックは自動回収されること"
    );
}

#[test]
fn truncated_last_line_is_skipped() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できる");
    let jsonl_path = dir.path().join("test.jsonl");

    // 正常な 1 行 + 異常終了で途中まで書かれた最終行、というファイルを用意する
    let record = sample_record(
        0,
        TrialResult::Complete(TrialValues {
            elapsed_seconds: 1.5,
            vmaf_mean: 90.0,
        }),
    );
    std::fs::write(
        &jsonl_path,
        format!("{}\n{{ \"trial_number\": 1, \"par", nojson::Json(&record)),
    )
    .expect("途中切れの最終行を含むファイルを書ける");

    // 追記中の異常終了で途中切れになった最終行はスキップされ、正常な 1 件だけが読み込まれる
    let loaded = storage::load_trials(&jsonl_path).expect("読み込みは成功する");
    assert_eq!(loaded.len(), 1, "途中切れの最終行はスキップされること");
    assert_eq!(loaded[0], record);
}

#[test]
fn malformed_middle_line_is_an_error() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できる");
    let jsonl_path = dir.path().join("test.jsonl");

    // 履歴ファイルは追記専用なので、中間行が壊れるのは正常系では起こらない (外部編集や FS 破損)。
    // 正常な行・壊れた中間行・正常な最終行、という並びを用意する。
    let record0 = sample_record(
        0,
        TrialResult::Complete(TrialValues {
            elapsed_seconds: 1.0,
            vmaf_mean: 90.0,
        }),
    );
    let record2 = sample_record(
        2,
        TrialResult::Complete(TrialValues {
            elapsed_seconds: 2.0,
            vmaf_mean: 80.0,
        }),
    );
    std::fs::write(
        &jsonl_path,
        format!(
            "{}\n{{ \"trial_number\": 1, broken\n{}\n",
            nojson::Json(&record0),
            nojson::Json(&record2)
        ),
    )
    .expect("中間行が壊れたファイルを書ける");

    // 中間行の破損は「読めるところまで」で誤魔化さず、エラーになる
    let result = storage::load_trials(&jsonl_path);
    assert!(result.is_err(), "中間行が壊れている場合はエラーになること");
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

#[test]
fn append_then_load_roundtrips() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できる");
    let jsonl_path = dir.path().join("test.jsonl");

    // 成功・失敗を混ぜた複数レコードを順に追記する
    let records = vec![
        sample_record(
            0,
            TrialResult::Complete(TrialValues {
                elapsed_seconds: 1.5,
                vmaf_mean: 90.0,
            }),
        ),
        sample_record(1, TrialResult::Fail),
        sample_record(
            2,
            TrialResult::Complete(TrialValues {
                elapsed_seconds: 2.0,
                vmaf_mean: 88.0,
            }),
        ),
    ];
    for record in &records {
        storage::append_trial(&jsonl_path, record).expect("追記できる");
    }

    // 追記したレコードが順序通りすべて読み戻せる
    let loaded = storage::load_trials(&jsonl_path).expect("読み込みは成功する");
    assert_eq!(loaded, records, "追記したレコードが順序通り読み戻せること");
}

#[test]
fn apply_params_to_layout_fails_when_path_missing() {
    // params のパスがレイアウトに存在しない場合はエラーになる
    let mut params = BTreeMap::new();
    params.insert(
        "missing"
            .parse::<JsonObjectMemberPath>()
            .expect("パスは無謬"),
        JsonValue::Integer(1),
    );
    let trial = Trial { number: 0, params };

    // 当該パスを含まない空オブジェクトのレイアウトに適用する
    let mut layout = JsonValue::Object(BTreeMap::new());
    let result = trial.apply_params_to_layout(&mut layout);
    assert!(
        result.is_err(),
        "レイアウトに無いパスへの適用は失敗すること"
    );
}

#[test]
fn search_space_rejects_non_array_non_object_distribution() {
    // パラメータ定義が配列 (カテゴリカル) でもオブジェクト (数値範囲) でもない場合はエラー
    let json = r#"{ "a": 42 }"#;
    let result = hisui::json::parse_str::<SearchSpace>(json);
    assert!(
        result.is_err(),
        "配列でもオブジェクトでもない定義はパースエラーになること"
    );
}
