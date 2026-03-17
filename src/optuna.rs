use std::{
    collections::BTreeMap,
    process::{Command, Stdio},
};

// Optuna のパラメータチューニング用の JSON 値型群
//
// nojson には所有権を持つ値型がないため、レイアウト JSON のクローン・パス指定での変更・
// 再シリアライズを行うために独自の型を定義している

#[derive(Debug, Clone, Copy)]
pub enum JsonNumber {
    Integer(i64),
    Float(f64),
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for JsonNumber {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        match value.kind() {
            nojson::JsonValueKind::Integer => {
                let int_value = value
                    .as_integer_str()?
                    .parse::<i64>()
                    .map_err(|e| value.invalid(e))?;
                Ok(JsonNumber::Integer(int_value))
            }
            nojson::JsonValueKind::Float => {
                let float_value = value
                    .as_float_str()?
                    .parse::<f64>()
                    .map_err(|e| value.invalid(e))?;
                Ok(JsonNumber::Float(float_value))
            }
            _ => Err(value.invalid("expected a number (integer or float)")),
        }
    }
}

impl nojson::DisplayJson for JsonNumber {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            JsonNumber::Integer(v) => v.fmt(f),
            JsonNumber::Float(v) => v.fmt(f),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for JsonValue {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        match value.kind() {
            nojson::JsonValueKind::Null => Ok(JsonValue::Null),
            nojson::JsonValueKind::Boolean => Ok(JsonValue::Boolean(value.try_into()?)),
            nojson::JsonValueKind::Integer => Ok(JsonValue::Integer(value.try_into()?)),
            nojson::JsonValueKind::Float => Ok(JsonValue::Float(value.try_into()?)),
            nojson::JsonValueKind::String => Ok(JsonValue::String(value.try_into()?)),
            nojson::JsonValueKind::Array => Ok(JsonValue::Array(value.try_into()?)),
            nojson::JsonValueKind::Object => Ok(JsonValue::Object(value.try_into()?)),
        }
    }
}

impl nojson::DisplayJson for JsonValue {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            JsonValue::Null => None::<()>.fmt(f),
            JsonValue::Boolean(v) => v.fmt(f),
            JsonValue::Integer(v) => v.fmt(f),
            JsonValue::Float(v) => v.fmt(f),
            JsonValue::String(v) => v.fmt(f),
            JsonValue::Array(v) => v.fmt(f),
            JsonValue::Object(v) => v.fmt(f),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct JsonObjectMemberPath(Vec<String>);

impl JsonObjectMemberPath {
    pub fn get<'a>(&self, mut value: &'a JsonValue) -> Option<&'a JsonValue> {
        for name in &self.0 {
            let JsonValue::Object(object) = value else {
                return None;
            };
            value = object.get(name)?;
        }
        Some(value)
    }

    pub fn get_mut<'a>(&self, mut value: &'a mut JsonValue) -> Option<&'a mut JsonValue> {
        for name in &self.0 {
            let JsonValue::Object(object) = value else {
                return None;
            };
            value = object.get_mut(name)?;
        }
        Some(value)
    }
}

impl std::str::FromStr for JsonObjectMemberPath {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.split('.').map(|s| s.to_owned()).collect()))
    }
}

impl std::fmt::Display for JsonObjectMemberPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.join("."))
    }
}

/// Optuna のスタディ関連操作を行いやすくするための構造体
#[derive(Debug)]
pub struct OptunaStudy {
    study_name: String,
    storage_url: String,
    last_best_trials: Vec<BestTrial>,
}

impl OptunaStudy {
    /// optuna コマンドが利用可能かどうかをチェックする
    pub fn check_optuna_availability() -> crate::Result<()> {
        let output = Command::new("optuna")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();

        match output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);

                let mut error_msg = format!(
                    "`$ optuna --version` command failed with exit code {}",
                    exit_code
                );

                if !stderr.trim().is_empty() {
                    error_msg.push_str(&format!("\nstderr: {}", stderr.trim()));
                }

                error_msg.push_str("\nPlease ensure optuna is properly installed and configured");

                Err(crate::Error::new(error_msg))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(crate::Error::new(
                "optuna command not found. Please install optuna and ensure it's in your PATH",
            )),
            Err(e) => Err(crate::Error::new(format!(
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
    pub fn create_study(&self) -> crate::Result<()> {
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
            .map_err(|e| {
                crate::Error::new(format!(
                    "failed to execute `$ optuna create-study` command: {e}"
                ))
            })?;
        output
            .status
            .success()
            .then_some(())
            .ok_or_else(|| crate::Error::new("`$ optuna create-study` command failed"))?;
        Ok(())
    }

    /// 次に探索すべきパラメータセットを問い合わせる
    pub fn ask(&self, search_space: &SearchSpace) -> crate::Result<Trial> {
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
            .map_err(|e| {
                crate::Error::new(format!("failed to execute `$ optuna ask` command: {e}"))
            })?;
        output
            .status
            .success()
            .then_some(())
            .ok_or_else(|| crate::Error::new("`$ optuna ask` command failed"))?;

        let stdout = String::from_utf8(output.stdout)?;
        crate::json::parse_str(&stdout)
    }

    /// 探索結果（成功応答）を optuna に伝える
    pub fn tell(&self, trial_number: usize, values: &TrialValues) -> crate::Result<()> {
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
            .map_err(|e| {
                crate::Error::new(format!("failed to execute `$ optuna tell` command: {e}"))
            })?;
        output
            .status
            .success()
            .then_some(())
            .ok_or_else(|| crate::Error::new("`$ optuna tell` command failed"))?;
        Ok(())
    }

    /// 探索結果（失敗応答）を optuna に伝える
    pub fn tell_fail(&self, trial_number: usize) -> crate::Result<()> {
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
            .map_err(|e| {
                crate::Error::new(format!("failed to execute `$ optuna tell` command: {e}"))
            })?;
        output
            .status
            .success()
            .then_some(())
            .ok_or_else(|| crate::Error::new("`$ optuna tell` command failed"))?;
        Ok(())
    }

    /// 現時点でのパレートフロント（最適解の集合）を取得する
    pub fn get_best_trials(&mut self) -> crate::Result<(bool, Vec<BestTrial>)> {
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
            .map_err(|e| {
                crate::Error::new(format!(
                    "failed to execute `$ optuna best-trials` command: {e}"
                ))
            })?;
        output
            .status
            .success()
            .then_some(())
            .ok_or_else(|| crate::Error::new("`$ optuna best-trials` command failed"))?;

        let stdout = String::from_utf8(output.stdout)?;
        let trials: Vec<BestTrial> = crate::json::parse_str(&stdout)?;
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
    pub fn apply_params_to_layout(&self, layout: &mut JsonValue) -> crate::Result<()> {
        for (path, value) in &self.params {
            *path.get_mut(layout).ok_or_else(|| {
                crate::Error::new(format!("target JSON path not found in layout: {}", path))
            })? = value.clone();
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
        } else if value.kind().is_object() {
            Ok(Self::Numeric {
                min: value.to_member("min")?.required()?.try_into()?,
                max: value.to_member("max")?.required()?.try_into()?,
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
