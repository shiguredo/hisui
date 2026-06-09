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
// 1 トライアル完了ごとに 1 行 1 JSON オブジェクトを追記する。
// 分散・並列最適化は非対応で、一度に 1 プロセスのみが書き込む前提 (issue 0010)。

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
            f.member("params", &self.params)?;
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
/// 履歴ファイルは追記専用なので、途中で切れ得るのは最終行 (最後の追記中の異常終了) だけである。
/// そのため最終の非空行の破損だけは警告を出してスキップし (読めるところまで再開する)、それ以外の
/// 中間行の破損は外部編集や FS 破損などの異常とみなしてエラーにする (データ欠損を黙認しない)。
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

    // 追記中の異常終了で途中切れになり得るのは最終の非空行だけ。この行の破損だけを許容する。
    let last_nonempty_index = content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, _)| index)
        .last();

    let mut trials = Vec::new();
    for (line_index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        // Json<T> の FromStr はパースと TryFrom を一括で行うため、
        // JSON 構文エラーとデコードエラーの両方をこの 1 つの match で扱える
        match line.parse::<nojson::Json<TrialRecord>>() {
            Ok(json) => trials.push(json.0),
            Err(e) => {
                if Some(line_index) == last_nonempty_index {
                    // 最終行の途中切れ (追記中の異常終了) は正常系として読めるところまでで再開する
                    tracing::warn!("skip truncated last trial line {}: {e}", line_index + 1);
                } else {
                    // 追記専用ファイルで中間行が壊れているのは異常 (外部編集・FS 破損など)
                    return Err(crate::Error::new(format!(
                        "malformed trial line at line {} (not the last line) in {}: {e}",
                        line_index + 1,
                        path.display()
                    )));
                }
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
/// 生成時にロックファイルをアトミックに作成して自プロセスの PID を書き込み、`Drop` 時に
/// 削除する (RAII)。`std::process::exit` やシグナル (SIGINT / SIGTERM)・クラッシュでは
/// `Drop` が走らずロックが残るが、その場合は次回起動時に「保持者プロセスが既に終了している」
/// ことを検出して自動的に奪取するため、手動削除は不要。
#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
}

impl LockGuard {
    /// ロックを獲得する
    ///
    /// ロックファイルをアトミックに作成 (存在確認 → 作成の TOCTOU を避けるため `create_new`)
    /// して PID を書き込む。すでに存在する場合は、その保持者プロセスが生きていれば多重起動と
    /// みなしてエラーを返し、既に終了していれば中断で残った stale ロックとみなして奪取する。
    pub fn acquire(path: PathBuf) -> crate::Result<Self> {
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    // 後続の stale 判定に使うため自プロセスの PID を書き込む
                    writeln!(file, "{}", std::process::id()).map_err(|e| {
                        crate::Error::new(format!(
                            "failed to write PID to lock file {}: {e}",
                            path.display()
                        ))
                    })?;
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_holder_is_alive(&path) {
                        return Err(crate::Error::new(format!(
                            "lock file already exists and its owner process is still running: {}. \
                             Another `hisui tune` process may be using this history.",
                            path.display()
                        )));
                    }
                    // 保持者が既に終了している (Ctrl+C やクラッシュで残った) stale ロックなので奪取する
                    tracing::warn!("reclaiming stale lock file {}", path.display());
                    match std::fs::remove_file(&path) {
                        Ok(()) => {}
                        // 別プロセスが先に消していた場合はそのまま再試行する
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => {
                            return Err(crate::Error::new(format!(
                                "failed to remove stale lock file {}: {e}",
                                path.display()
                            )));
                        }
                    }
                    // ループ先頭に戻って作成をやり直す
                }
                Err(e) => {
                    return Err(crate::Error::new(format!(
                        "failed to create lock file {}: {e}",
                        path.display()
                    )));
                }
            }
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

/// ロックファイルに記録された PID のプロセスが生きているかどうかを返す
///
/// PID が読めない (空・破損・旧フォーマット) 場合は安全側に倒して「生きている」とみなす
/// (誤って奪取しないため、その場合は従来どおりエラーになる)。
fn lock_holder_is_alive(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return true;
    };
    let Ok(pid) = content.trim().parse::<i32>() else {
        return true;
    };
    process_is_alive(pid)
}

/// 指定 PID のプロセスが存在するかどうかを返す
fn process_is_alive(pid: i32) -> bool {
    // kill(pid, 0) はシグナルを送らずに存在確認だけを行う。
    // 戻り値 0 なら存在、エラーが ESRCH なら不在、EPERM (別ユーザー) ならプロセスは存在する。
    // SAFETY: kill は引数を読むだけで、不正なメモリアクセスは起こさない。
    let ret = unsafe { libc::kill(pid, 0) };
    if ret == 0 {
        true
    } else {
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
}
