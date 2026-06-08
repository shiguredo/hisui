use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use crate::tune::TrialValues;
use crate::tune::json_value::{JsonObjectMemberPath, JsonValue};

// 試行履歴の永続化 (単一の JSON Lines ファイル) と、多重起動防止のロックファイル管理。
//
// optuna の SQLite ストレージは使わず、1 トライアル完了ごとに 1 行 1 JSON オブジェクトを
// 追記する。分散・並列最適化は非対応で、一度に 1 プロセスのみが書き込む前提 (issue 0010)。

/// 試行 1 件の結果
#[derive(Debug, Clone, PartialEq)]
pub enum TrialResult {
    /// 成功 (評価値あり)
    Complete(TrialValues),
    /// 失敗
    Fail,
}

/// JSON Lines に保存する試行 1 件のレコード
#[derive(Debug, Clone, PartialEq)]
pub struct TrialRecord {
    pub trial_number: usize,
    pub params: BTreeMap<JsonObjectMemberPath, JsonValue>,
    pub result: TrialResult,
}

impl nojson::DisplayJson for TrialRecord {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("trial_number", self.trial_number)?;
            f.member(
                "params",
                nojson::json(|f| {
                    f.object(|f| {
                        for (path, value) in &self.params {
                            f.member(path.to_string(), value)?;
                        }
                        Ok(())
                    })
                }),
            )?;
            match &self.result {
                TrialResult::Complete(values) => {
                    f.member("state", "complete")?;
                    f.member("elapsed_seconds", values.elapsed_seconds)?;
                    f.member("vmaf_mean", values.vmaf_mean)?;
                }
                TrialResult::Fail => {
                    f.member("state", "fail")?;
                }
            }
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for TrialRecord {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let trial_number: usize = value.to_member("trial_number")?.required()?.try_into()?;

        // params はキーがパス文字列・値が JsonValue のオブジェクト
        // (nojson は BTreeMap<K: FromStr, V: TryFrom> へ変換できる)
        let params = value.to_member("params")?.required()?.try_into()?;

        let state: String = value.to_member("state")?.required()?.try_into()?;
        let result = match state.as_str() {
            "complete" => {
                let elapsed_seconds: f64 =
                    value.to_member("elapsed_seconds")?.required()?.try_into()?;
                let vmaf_mean: f64 = value.to_member("vmaf_mean")?.required()?.try_into()?;
                TrialResult::Complete(TrialValues {
                    elapsed_seconds,
                    vmaf_mean,
                })
            }
            "fail" => TrialResult::Fail,
            other => {
                return Err(value.invalid(format!("unknown trial state: {other}")));
            }
        };

        Ok(Self {
            trial_number,
            params,
            result,
        })
    }
}

/// JSON Lines ファイルを読み込み、保存済みの試行レコードを返す
///
/// ファイルが存在しない場合は空の `Vec` を返す。
/// 異常終了で最終行が途中で切れているケースに備え、パースに失敗した行は警告を出して
/// スキップする (読めるところまで再開する)。
pub fn load_trials(path: &Path) -> crate::Result<Vec<TrialRecord>> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(crate::Error::new(format!(
                "failed to read trials file {}: {e}",
                path.display()
            )));
        }
    };

    let mut trials = Vec::new();
    for (line_index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match nojson::RawJson::parse(line) {
            Ok(json) => match TrialRecord::try_from(json.value()) {
                Ok(record) => trials.push(record),
                Err(e) => {
                    tracing::warn!(
                        "skip malformed trial record at line {}: {e}",
                        line_index + 1
                    );
                }
            },
            Err(e) => {
                tracing::warn!("skip malformed trial line at line {}: {e}", line_index + 1);
            }
        }
    }
    Ok(trials)
}

/// 試行レコードを JSON Lines ファイルに 1 行追記する
pub fn append_trial(path: &Path, record: &TrialRecord) -> crate::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| {
            crate::Error::new(format!(
                "failed to open trials file {} for append: {e}",
                path.display()
            ))
        })?;
    let line = nojson::Json(record).to_string();
    writeln!(file, "{line}").map_err(|e| {
        crate::Error::new(format!(
            "failed to append trial record to {}: {e}",
            path.display()
        ))
    })?;
    Ok(())
}

/// 多重起動を防ぐためのロックファイル
///
/// 生成時にロックファイルをアトミックに作成し、`Drop` 時に削除する (RAII)。
/// 途中でエラーやパニックが起きても `Drop` で削除されるが、`std::process::exit` や
/// シグナル (SIGINT / SIGTERM) では `Drop` が走らずロックが残る。残存時は手動削除が必要。
#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
}

impl LockGuard {
    /// ロックファイルをアトミックに作成して獲得する
    ///
    /// すでに存在する場合はエラーを返す (存在確認 → 作成の TOCTOU を避けるため
    /// `create_new` を使う)。
    pub fn acquire(path: PathBuf) -> crate::Result<Self> {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => Ok(Self { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(crate::Error::new(format!(
                    "lock file already exists: {}. \
                     Another `hisui tune` process may be running. \
                     If it has already finished, remove this file manually to resume \
                     (the .jsonl history is preserved).",
                    path.display()
                )))
            }
            Err(e) => Err(crate::Error::new(format!(
                "failed to create lock file {}: {e}",
                path.display()
            ))),
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.path) {
            tracing::warn!("failed to remove lock file {}: {e}", self.path.display());
        }
    }
}
