use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    path::PathBuf,
    process::{Command, Stdio},
};

use orfail::OrFail;

use crate::{
    json::{JsonNumber, JsonObject, JsonValue},
    subcommand_vmaf,
};

const DEFAULT_LAYOUT_JSON: &str = include_str!("../layout-examples/tune-vp8.json");
const DEFAULT_SEARCH_SPACE_JSON: &str = include_str!("../search-space-examples/full.json");

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let layout_file_path: Option<PathBuf> = noargs::opt("layout-file")
        .short('l')
        .ty("PATH")
        .doc(concat!(
            "パラメータ調整に使用するレイアウトファイルを指定します\n",
            "\n",
            "省略された場合には hisui/layout-examples/tune-vp8.json が使用されます",
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let search_space_file_path: Option<PathBuf> = noargs::opt("search-space-file")
        .short('s')
        .ty("PATH")
        .doc(concat!(
            "探索空間定義ファイル（JSON）のパスを指定します\n",
            "\n",
            "省略された場合には hisui/search-space-examples/full.json が使用されます",
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let tune_working_dir: PathBuf = noargs::opt("tune-working-dir")
        .ty("PATH")
        .default("hisui-tune/")
        .doc(concat!(
            "チューニング用に使われる作業ディレクトリを指定します\n",
            "\n",
            "相対パスの場合は ROOT_DIR が起点となります"
        ))
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let study_name: String = noargs::opt("study-name")
        .ty("NAME")
        .default("hisui-tune")
        .doc("Optuna の study 名を指定します")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let trial_count: usize = noargs::opt("trial-count")
        .short('n')
        .ty("INTEGER")
        .default("100")
        .doc("実行する試行回数を指定します")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパスを指定します")
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let no_progress_bar: bool = noargs::flag("no-progress-bar")
        .short('P')
        .doc("指定された場合は、調整の進捗を非表示にします")
        .take(&mut args)
        .is_present();
    let max_cpu_cores: Option<NonZeroUsize> = noargs::opt("max-cpu-cores")
        .short('c')
        .ty("INTEGER")
        .env("HISUI_MAX_CPU_CORES")
        .doc(concat!(
            "調整処理を行うプロセスが使用するコア数の上限を指定します\n",
            "（未指定時には上限なし）\n",
            "\n",
            "NOTE: macOS ではこの引数は無視されます",
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let frame_count: usize = noargs::opt("frame-count")
        .short('f')
        .ty("FRAMES")
        // 全体の実行時間に大きく影響するので vmaf コマンドに比べてデフォルト値が小さめにしておく
        .default("300")
        .doc("調整用にエンコードする映像フレームの数を指定します")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let root_dir: PathBuf = noargs::arg("ROOT_DIR")
        .example("/path/to/archive/RECORDING_ID/")
        .doc(concat!(
            "調整処理を行う際のルートディレクトリを指定します\n",
            "\n",
            "レイアウトファイル内に記載された相対パスの基点は、このディレクトリとなります。\n",
            "また、レイアウト内で、",
            "このディレクトリの外のファイルが参照された場合にはエラーとなります。"
        ))
        .take(&mut args)
        .then(|a| -> Result<_, Box<dyn std::error::Error>> {
            let path: PathBuf = a.value().parse()?;

            if matches!(a, noargs::Arg::Example { .. }) {
            } else if !path.exists() {
                return Err("no such directory".into());
            } else if !path.is_dir() {
                return Err("not a directory".into());
            }

            Ok(path)
        })?;

    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    // 最初に optuna と vmaf コマンドが利用可能かどうかをチェックする
    check_optuna_availability().or_fail()?;
    subcommand_vmaf::check_vmaf_availability().or_fail()?;

    // 必要なら tune_working_dir を作る
    let tune_working_dir = root_dir.join(tune_working_dir);
    if !tune_working_dir.exists() {
        std::fs::create_dir_all(&tune_working_dir).or_fail_with(|e| {
            format!(
                "failed to create tune working directory {}: {e}",
                tune_working_dir.display()
            )
        })?;
    }

    // レイアウトファイル（テンプレート）を読み込む
    let layout_template: JsonValue = if let Some(path) = &layout_file_path {
        std::fs::read_to_string(path)
            .or_fail()?
            .parse()
            .map(|nojson::Json(v)| v)
            .or_fail()?
    } else {
        DEFAULT_LAYOUT_JSON
            .parse()
            .map(|nojson::Json(v)| v)
            .or_fail()?
    };
    log::debug!("template: {layout_template:?}");

    // 探索空間ファイルを読み込む
    let search_space_json_string = if let Some(path) = &search_space_file_path {
        std::fs::read_to_string(path).or_fail()?
    } else {
        DEFAULT_SEARCH_SPACE_JSON.to_owned()
    };
    let search_space_raw_json = nojson::RawJson::parse(&search_space_json_string).or_fail()?;
    let mut search_space = SearchSpace::new(search_space_raw_json.value()).or_fail()?;

    // レイアウトテンプレートの処理に不要なエントリは捨てる
    search_space
        .items
        .retain(|path, _| matches!(path.get(&layout_template), Some(JsonValue::Null)));
    log::debug!("search space: {search_space:?}");

    // optuna の study を作る
    let storage_url = format!("sqlite:///{}", tune_working_dir.join("optuna.db").display());
    let optuna = Optuna::new(study_name.clone(), storage_url);
    optuna.create_study().or_fail()?;

    for _ in 0..trial_count {
        let params = optuna.ask(&search_space).or_fail()?;
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct JsonObjectMemberPath(Vec<String>);

impl JsonObjectMemberPath {
    fn get<'a>(&self, mut value: &'a JsonValue) -> Option<&'a JsonValue> {
        for name in &self.0 {
            let JsonValue::Object(object) = value else {
                return None;
            };
            value = object.get(name)?;
        }
        Some(value)
    }
}

#[derive(Debug)]
struct SearchSpace {
    items: BTreeMap<JsonObjectMemberPath, SearchSpaceItem>,
}

impl SearchSpace {
    fn new(root: nojson::RawJsonValue<'_, '_>) -> Result<Self, nojson::JsonParseError> {
        let mut items = BTreeMap::new();
        for (key, value) in root.to_object()? {
            let path = JsonObjectMemberPath(
                key.to_unquoted_string_str()?
                    .split('.')
                    .map(|s| s.to_owned())
                    .collect(),
            );
            let item = SearchSpaceItem::new(value)?;
            items.insert(path, item);
        }
        Ok(Self { items })
    }

    fn to_ask_search_space(&self) -> String {
        nojson::json(|f| {
            f.object(|f| {
                for (path, item) in &self.items {
                    f.member(
                        path.0.join("."),
                        nojson::json(|f| item.to_optuna_distribution(f)),
                    )?;
                }
                Ok(())
            })
        })
        .to_string()
    }
}

#[derive(Debug)]
enum SearchSpaceItem {
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

fn check_optuna_availability() -> orfail::Result<()> {
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

#[derive(Debug)]
struct Optuna {
    study_name: String,
    storage_url: String,
}

impl Optuna {
    fn new(study_name: String, storage_url: String) -> Self {
        Self {
            study_name,
            storage_url,
        }
    }

    fn create_study(&self) -> orfail::Result<()> {
        let output = Command::new("optuna")
            .arg("create-study")
            .arg("--study-name")
            .arg(&self.study_name)
            .arg("--storage")
            .arg(&self.storage_url)
            .arg("--skip-if-exists")
            // 「エンコード時間の最小化」と「VMAF スコアの最大化」
            .arg("--directions")
            .arg("minimize")
            .arg("maximize")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .or_fail_with(|e| format!("failed to execute optuna create-study command: {e}"))?;

        output.status.success().or_fail()?;
        Ok(())
    }

    fn ask(
        &self,
        search_space: &SearchSpace,
    ) -> orfail::Result<BTreeMap<JsonObjectMemberPath, JsonValue>> {
        let search_space_json = search_space.to_ask_search_space();
        log::debug!("ask search space: {search_space_json}");

        // for (path, item) in &search_space.items {
        //     let distribution_json = match item {
        //         SearchSpaceItem::Number { min, max } => {
        //             serde_json::json!({
        //                 "name": "FloatDistribution",
        //                 "attributes": {
        //                     "low": min,
        //                     "high": max,
        //                     "step": null,
        //                     "log": false
        //                 }
        //             })
        //         }
        //         SearchSpaceItem::Categorical(values) => {
        //             serde_json::json!({
        //                 "name": "CategoricalDistribution",
        //                 "attributes": {
        //                     "choices": values
        //                 }
        //             })
        //         }
        //     };

        //     let path_key = path.0.join(".");
        //     search_space_json.insert(path_key, distribution_json);
        // }

        // let search_space_str = serde_json::to_string(&search_space_json)
        //     .or_fail_with(|e| format!("failed to serialize search space: {e}"))?;

        // // Execute optuna ask command
        // let output = Command::new("optuna")
        //     .arg("ask")
        //     .arg("--storage")
        //     .arg(&self.storage_url)
        //     .arg("--study-name")
        //     .arg(&self.study_name)
        //     .arg("--search-space")
        //     .arg(&search_space_str)
        //     .stdout(Stdio::piped())
        //     .stderr(Stdio::piped())
        //     .output()
        //     .or_fail_with(|e| format!("failed to execute optuna ask command: {e}"))?;

        // if !output.status.success() {
        //     let stderr = String::from_utf8_lossy(&output.stderr);
        //     return Err(orfail::Failure::new(format!(
        //         "optuna ask command failed: {stderr}"
        //     )));
        // }

        // // Parse the output to extract parameters
        // let stdout = String::from_utf8_lossy(&output.stdout);

        // // The output contains a JSON line with the trial info
        // // We need to find the JSON line (it's the last line that starts with '{')
        // let json_line = stdout
        //     .lines()
        //     .filter(|line| line.trim().starts_with('{'))
        //     .last()
        //     .ok_or_else(|| orfail::Failure::new("no JSON output found from optuna ask command"))?;

        // // Parse the JSON to extract parameters
        // let trial_info: serde_json::Value = serde_json::from_str(json_line)
        //     .or_fail_with(|e| format!("failed to parse optuna ask output: {e}"))?;

        // let params = trial_info
        //     .get("params")
        //     .and_then(|p| p.as_object())
        //     .ok_or_else(|| orfail::Failure::new("no params found in optuna ask output"))?;

        // // Convert the parameters back to our format
        // let mut result = BTreeMap::new();
        // for (key, value) in params {
        //     let path = JsonObjectMemberPath(key.split('.').map(|s| s.to_owned()).collect());
        //     let json_value = match value {
        //         serde_json::Value::Number(n) => {
        //             if let Some(f) = n.as_f64() {
        //                 JsonValue::Number(JsonNumber::from_f64(f).unwrap())
        //             } else if let Some(i) = n.as_i64() {
        //                 JsonValue::Number(JsonNumber::from_i64(i).unwrap())
        //             } else {
        //                 return Err(orfail::Failure::new("unsupported number type"));
        //             }
        //         }
        //         serde_json::Value::String(s) => JsonValue::String(s.clone()),
        //         serde_json::Value::Bool(b) => JsonValue::Bool(*b),
        //         serde_json::Value::Null => JsonValue::Null,
        //         _ => return Err(orfail::Failure::new("unsupported parameter type")),
        //     };
        //     result.insert(path, json_value);
        // }

        //Ok(result)
        todo!()
    }
}
