use std::{
    collections::BTreeMap,
    process::{Command, Stdio},
};

use orfail::OrFail;

use crate::json::{JsonNumber, JsonObject, JsonObjectMemberPath, JsonValue};

/// Optuna のスタディ関連操作を行いやすくするための構造体
#[derive(Debug)]
pub struct OptunaStudy {
    study_name: String,
    storage_url: String,
    last_best_trials: Vec<BestTrial>,
}

impl OptunaStudy {
    /// optuna コマンドが利用可能かどうかをチェックする
    pub fn check_optuna_availability() -> orfail::Result<()> {
        let output = Command::new("optuna")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output();

        match output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(_) => Err(orfail::Failure::new(
                "`$ optuna --version` command failed to execute properly",
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(orfail::Failure::new(
                "optuna command not found. Please install optuna and ensure it's in your PATH",
            )),
            Err(e) => Err(orfail::Failure::new(format!(
                "failed to check optuna availability: {e}"
            ))),
        }
    }

    /// 新しい `OptunaStudy` インスタンスを生成する
    pub fn new(study_name: String, storage_url: String) -> Self {
        Self {
            study_name,
            storage_url,
            last_best_trials: Vec::new(),
        }
    }

    /// スタディ名を返す
    pub fn study_name(&self) -> &str {
        &self.study_name
    }

    /// スタディを作成する
    pub fn create_study(&self) -> orfail::Result<()> {
        let output = Command::new("optuna")
            .arg("create-study")
            .arg("--study-name")
            .arg(&self.study_name)
            .arg("--storage")
            .arg(&self.storage_url)
            .arg("--skip-if-exists") // すでに同じ名前のものが存在する場合には作成しない
            .arg("--directions")
            .arg("minimize") // 合成処理時間の最小化
            .arg("maximize") // VMAF スコアの最大か
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .output()
            .or_fail_with(|e| format!("failed to execute `$ optuna create-study` command: {e}"))?;
        output
            .status
            .success()
            .or_fail_with(|()| "`$ optuna create-study` command failed".to_owned())?;
        Ok(())
    }

    /// 次に探索すべきパラメータセットを問い合わせる
    pub fn ask(&self, search_space: &SearchSpace) -> orfail::Result<Trial> {
        let output = Command::new("optuna")
            .arg("ask")
            .arg("--storage")
            .arg(&self.storage_url)
            .arg("--study-name")
            .arg(&self.study_name)
            .arg("--search-space")
            .arg(search_space.to_optuna_search_space())
            // optuna ask コマンドは「実験的機能です」という警告を出すけど、
            // Hisui 側で対処できるものでもなく、ノイジーなだけなので抑制する
            .env("PYTHONWARNINGS", "ignore")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .or_fail_with(|e| format!("failed to execute `$ optuna ask` command: {e}"))?;
        output
            .status
            .success()
            .or_fail_with(|()| "`$ optuna ask` command failed".to_owned())?;

        let stdout = String::from_utf8(output.stdout).or_fail()?;
        crate::json::parse_str(&stdout).or_fail()
    }

    /// 探索結果（成功応答）を optuna に伝える
    pub fn tell(&self, trial_number: usize, values: &TrialValues) -> orfail::Result<()> {
        let output = Command::new("optuna")
            .arg("tell")
            .arg("--storage")
            .arg(&self.storage_url)
            .arg("--study-name")
            .arg(&self.study_name)
            .arg("--trial-number")
            .arg(trial_number.to_string())
            .arg("--values")
            .arg(values.elapsed_seconds.to_string())
            .arg(values.vmaf_mean.to_string())
            .arg("--state")
            .arg("complete")
            // optuna tell コマンドは「実験的機能です」という警告を出すけど、
            // Hisui 側で対処できるものでもなく、ノイジーなだけなので抑制する
            .env("PYTHONWARNINGS", "ignore")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .or_fail_with(|e| format!("failed to execute `$ optuna tell` command: {e}"))?;
        output
            .status
            .success()
            .or_fail_with(|()| "`$ optuna tell` command failed".to_owned())?;
        Ok(())
    }

    /// 探索結果（失敗応答）を optuna に伝える
    pub fn tell_fail(&self, trial_number: usize) -> orfail::Result<()> {
        let output = Command::new("optuna")
            .arg("tell")
            .arg("--storage")
            .arg(&self.storage_url)
            .arg("--study-name")
            .arg(&self.study_name)
            .arg("--trial-number")
            .arg(trial_number.to_string())
            .arg("--state")
            .arg("fail")
            // optuna tell コマンドは「実験的機能です」という警告を出すけど、
            // Hisui 側で対処できるものでもなく、ノイジーなだけなので抑制する
            .env("PYTHONWARNINGS", "ignore")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .or_fail_with(|e| format!("failed to execute `$ optuna tell` command: {e}"))?;
        output
            .status
            .success()
            .or_fail_with(|()| "`$ optuna tell` command failed".to_owned())?;
        Ok(())
    }

    /// 現時点でのパレートフロント（最適解の集合）を取得する
    pub fn get_best_trials(&mut self) -> orfail::Result<(bool, Vec<BestTrial>)> {
        let output = Command::new("optuna")
            .arg("best-trials")
            .arg("--storage")
            .arg(&self.storage_url)
            .arg("--study-name")
            .arg(&self.study_name)
            .arg("-f")
            .arg("json")
            // optuna best-trials コマンドは「実験的機能です」という警告を出すけど、
            // Hisui 側で対処できるものでもなく、ノイジーなだけなので抑制する
            .env("PYTHONWARNINGS", "ignore")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .or_fail_with(|e| format!("failed to execute `$ optuna best-trials` command: {e}"))?;
        output
            .status
            .success()
            .or_fail_with(|()| "`$ optuna best-trials` command failed".to_owned())?;

        let stdout = String::from_utf8(output.stdout).or_fail()?;
        let trials: Vec<BestTrial> = crate::json::parse_str(&stdout).or_fail()?;
        let updated = self.last_best_trials != trials;
        self.last_best_trials = trials.clone();
        Ok((updated, trials))
    }
}

/// トライアルの情報
#[derive(Debug)]
pub struct Trial {
    pub number: usize,
    pub params: BTreeMap<JsonObjectMemberPath, JsonValue>,
}

impl Trial {
    /// Optuna が提案したパラメータセットを使ってレイアウトを更新する
    pub fn apply_params_to_layout(&self, layout: &mut JsonValue) -> orfail::Result<()> {
        for (path, value) in &self.params {
            *path.get_mut(layout).or_fail()? = value.clone();
        }
        Ok(())
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for Trial {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        Ok(Self {
            number: value.to_member("number")?.required()?.try_into()?,
            params: value.to_member("params")?.required()?.try_into()?,
        })
    }
}

/// トライアルの評価結果
#[derive(Debug, Clone, PartialEq)]
pub struct TrialValues {
    pub elapsed_seconds: f64,
    pub vmaf_mean: f64,
}

/// 探索空間
#[derive(Debug)]
pub struct SearchSpace {
    pub params: BTreeMap<JsonObjectMemberPath, ParameterDistribution>,
}

impl SearchSpace {
    /// Optuna 形式の探索空間 JSON に変換する
    pub fn to_optuna_search_space(&self) -> String {
        nojson::json(|f| {
            f.object(|f| {
                for (path, item) in &self.params {
                    f.member(path, nojson::json(|f| item.to_optuna_distribution(f)))?;
                }
                Ok(())
            })
        })
        .to_string()
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for SearchSpace {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        Ok(Self {
            params: value.try_into()?,
        })
    }
}

/// 各パラメータの探索空間定義
#[derive(Debug)]
pub enum ParameterDistribution {
    Numeric { min: JsonNumber, max: JsonNumber },
    Categorical(Vec<JsonValue>),
}

impl ParameterDistribution {
    /// Optuna がサポートする形式の JSON に変換する
    fn to_optuna_distribution(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            ParameterDistribution::Numeric {
                min: JsonNumber::Integer(min),
                max: JsonNumber::Integer(max),
            } => {
                // 両方整数なら IntDistribution
                f.object(|f| {
                    f.member("name", "IntDistribution")?;
                    f.member(
                        "attributes",
                        nojson::json(|f| {
                            f.object(|f| {
                                f.member("low", min)?;
                                f.member("high", max)
                            })
                        }),
                    )
                })?;
            }
            ParameterDistribution::Numeric { min, max } => {
                // それ以外の数値は FloatDistribution
                f.object(|f| {
                    f.member("name", "FloatDistribution")?;
                    f.member(
                        "attributes",
                        nojson::json(|f| {
                            f.object(|f| {
                                f.member("low", min)?;
                                f.member("high", max)
                            })
                        }),
                    )
                })?;
            }
            ParameterDistribution::Categorical(choices) => {
                f.object(|f| {
                    f.member("name", "CategoricalDistribution")?;
                    f.member(
                        "attributes",
                        nojson::json(|f| f.object(|f| f.member("choices", choices))),
                    )
                })?;
            }
        }
        Ok(())
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ParameterDistribution {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        if value.kind().is_array() {
            Ok(Self::Categorical(value.try_into()?))
        } else if let Ok(object) = JsonObject::new(value) {
            Ok(Self::Numeric {
                min: object.get_required("min")?,
                max: object.get_required("max")?,
            })
        } else {
            Err(value.invalid("not JSON array or JSON object"))
        }
    }
}

impl nojson::DisplayJson for ParameterDistribution {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            ParameterDistribution::Numeric { min, max } => f.object(|f| {
                f.member("min", min)?;
                f.member("max", max)
            }),
            ParameterDistribution::Categorical(choices) => f.value(choices),
        }
    }
}

/// パレートフロント上に位置しているトライアルの情報
#[derive(Debug, Clone, PartialEq)]
pub struct BestTrial {
    pub number: usize,
    pub values: TrialValues,
    pub params: BTreeMap<String, JsonValue>,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for BestTrial {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let values: [f64; 2] = value.to_member("values")?.required()?.try_into()?;
        Ok(Self {
            number: value.to_member("number")?.required()?.try_into()?,
            values: TrialValues {
                elapsed_seconds: values[0],
                vmaf_mean: values[1],
            },
            params: value.to_member("params")?.required()?.try_into()?,
        })
    }
}
