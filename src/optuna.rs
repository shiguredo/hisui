use std::{
    collections::BTreeMap,
    process::{Command, Stdio},
};

use orfail::OrFail;

use crate::json::{JsonNumber, JsonObject, JsonObjectMemberPath, JsonValue};

#[derive(Debug)]
pub struct Optuna {
    pub study_name: String,
    storage_url: String,
}

impl Optuna {
    pub fn check_optuna_availability() -> orfail::Result<()> {
        let output = Command::new("optuna")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output();

        match output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(_) => Err(orfail::Failure::new(
                "optuna command failed to execute properly",
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(orfail::Failure::new(
                "optuna command not found. Please install optuna and ensure it's in your PATH",
            )),
            Err(e) => Err(orfail::Failure::new(format!(
                "failed to check optuna availability: {e}"
            ))),
        }
    }

    pub fn new(study_name: String, storage_url: String) -> Self {
        Self {
            study_name,
            storage_url,
        }
    }

    pub fn create_study(&self) -> orfail::Result<()> {
        let output = Command::new("optuna")
            .arg("create-study")
            .arg("--study-name")
            .arg(&self.study_name)
            .arg("--storage")
            .arg(&self.storage_url)
            .arg("--skip-if-exists")
            // 「エンコード効率（何倍速変換か）の最小化」と「VMAF スコアの最大化」
            .arg("--directions")
            .arg("maximize")
            .arg("maximize")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .or_fail_with(|e| format!("failed to execute optuna create-study command: {e}"))?;

        output.status.success().or_fail()?;
        Ok(())
    }

    pub fn ask(&self, search_space: &SearchSpace) -> orfail::Result<AskOutput> {
        let search_space_json = search_space.to_ask_search_space();
        log::debug!("ask search space: {search_space_json}");

        // Execute optuna ask command
        let output = Command::new("optuna")
            .arg("ask")
            .arg("--storage")
            .arg(&self.storage_url)
            .arg("--study-name")
            .arg(&self.study_name)
            .arg("--search-space")
            .arg(&search_space_json)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .or_fail_with(|e| format!("failed to execute optuna ask command: {e}"))?;

        output.status.success().or_fail_with(|()| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!("optuna ask command failed: {stderr}")
        })?;

        // Parse the output to extract parameters
        let stdout = String::from_utf8(output.stdout).or_fail()?;
        let output = stdout.parse().map(|nojson::Json(v)| v).or_fail()?;
        log::info!("ask result: {output:?}");

        Ok(output)
    }

    pub fn tell(&self, trial_number: usize, metrics: &TrialMetrics) -> orfail::Result<()> {
        let output = Command::new("optuna")
            .arg("tell")
            .arg("--storage")
            .arg(&self.storage_url)
            .arg("--study-name")
            .arg(&self.study_name)
            .arg("--trial-number")
            .arg(trial_number.to_string())
            .arg("--values")
            .arg(&metrics.encoding_speed_ratio.to_string())
            .arg(&metrics.vmaf_mean.to_string())
            .arg("--state")
            .arg("complete")
            // optuna tell コマンドは「実験的機能です」という警告を出すけど、
            // Hisui 側で対処できるものでもなく、ノイジーなだけなので抑制する
            .env("PYTHONWARNINGS", "ignore")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .or_fail_with(|e| format!("failed to execute optuna tell command: {e}"))?;

        output
            .status
            .success()
            .or_fail_with(|()| "optuna tell command failed".to_string())?;

        Ok(())
    }

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
            .or_fail_with(|e| format!("failed to execute optuna tell command: {e}"))?;

        output
            .status
            .success()
            .or_fail_with(|()| "optuna tell command failed".to_string())?;

        Ok(())
    }

    pub fn get_best_trials(&self) -> orfail::Result<Vec<BestTrial>> {
        let output = Command::new("optuna")
            .arg("best-trials")
            .arg("--storage")
            .arg(&self.storage_url)
            .arg("--study-name")
            .arg(&self.study_name)
            .arg("-f")
            .arg("json")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .or_fail_with(|e| format!("failed to execute optuna best-trials command: {e}"))?;

        output.status.success().or_fail_with(|()| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!("optuna best-trials command failed: {stderr}")
        })?;

        let stdout = String::from_utf8(output.stdout).or_fail()?;
        let trials: Vec<BestTrial> =
            Vec::<BestTrial>::try_from(nojson::RawJson::parse(&stdout).or_fail()?.value())
                .or_fail()?;

        Ok(trials)
    }
}

#[derive(Debug)]
pub struct AskOutput {
    pub trial_number: usize,
    pub params: BTreeMap<JsonObjectMemberPath, JsonValue>,
}

impl AskOutput {
    pub fn update_layout(&self, layout: &mut JsonValue) -> orfail::Result<()> {
        for (path, new_value) in &self.params {
            *path.get_mut(layout).or_fail()? = new_value.clone();
        }
        Ok(())
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for AskOutput {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        Ok(Self {
            trial_number: value.to_member("number")?.required()?.try_into()?,
            params: value.to_member("params")?.required()?.try_into()?,
        })
    }
}

#[derive(Debug)]
pub struct TrialMetrics {
    pub encoding_speed_ratio: f64,
    pub vmaf_mean: f64,
}

#[derive(Debug)]
pub struct SearchSpace {
    pub items: BTreeMap<JsonObjectMemberPath, SearchSpaceItem>,
}

impl SearchSpace {
    pub fn new(root: nojson::RawJsonValue<'_, '_>) -> Result<Self, nojson::JsonParseError> {
        let mut items = BTreeMap::new();
        for (key, value) in root.to_object()? {
            let path = key.to_unquoted_string_str()?.parse().expect("infallible");
            let item = SearchSpaceItem::new(value)?;
            items.insert(path, item);
        }
        Ok(Self { items })
    }

    pub fn to_ask_search_space(&self) -> String {
        nojson::json(|f| {
            f.object(|f| {
                for (path, item) in &self.items {
                    f.member(path, nojson::json(|f| item.to_optuna_distribution(f)))?;
                }
                Ok(())
            })
        })
        .to_string()
    }
}

#[derive(Debug)]
pub enum SearchSpaceItem {
    Number { min: JsonNumber, max: JsonNumber },
    Categorical(Vec<JsonValue>),
}

impl SearchSpaceItem {
    fn new(value: nojson::RawJsonValue<'_, '_>) -> Result<Self, nojson::JsonParseError> {
        if value.kind().is_array() {
            Ok(Self::Categorical(value.try_into()?))
        } else if let Ok(object) = JsonObject::new(value) {
            Ok(Self::Number {
                min: object.get_required("min")?,
                max: object.get_required("max")?,
            })
        } else {
            Err(value.invalid("not JSON array or JSON object"))
        }
    }

    fn to_optuna_distribution(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            SearchSpaceItem::Number {
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
            SearchSpaceItem::Number { min, max } => {
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
            SearchSpaceItem::Categorical(choices) => {
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

impl nojson::DisplayJson for SearchSpaceItem {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            SearchSpaceItem::Number { min, max } => f.object(|f| {
                f.member("min", min)?;
                f.member("max", max)
            }),
            SearchSpaceItem::Categorical(choices) => f.value(choices),
        }
    }
}

#[derive(Debug)]
pub struct BestTrial {
    pub number: usize,
    pub values: Vec<f64>,
    pub duration: String,
    pub params: BTreeMap<String, JsonValue>,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for BestTrial {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        Ok(Self {
            number: value.to_member("number")?.required()?.try_into()?,
            values: value.to_member("values")?.required()?.try_into()?,
            duration: value.to_member("duration")?.required()?.try_into()?,
            params: value.to_member("params")?.required()?.try_into()?,
        })
    }
}
