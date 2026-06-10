pub mod json_value;
pub mod nsga2;
pub mod rng;
pub mod storage;

use std::collections::BTreeMap;
use std::path::PathBuf;

use self::json_value::{JsonNumber, JsonObjectMemberPath, JsonValue};
use self::nsga2::Individual;
use self::storage::{LockGuard, TrialRecord, TrialResult};

// 2 目的 (合成時間 minimize / VMAF 平均 maximize) の多目的最適化を NSGA-II で行うモジュール。
//
// 最適化と試行履歴の永続化を hisui 内で完結させる。
// アルゴリズムの詳細は nsga2 モジュール、履歴・ロックの永続化は storage モジュールを参照。

/// トライアルの情報 (ask が返す、次に評価すべきパラメータセット)
#[derive(Debug)]
pub struct Trial {
    pub number: usize,
    pub params: BTreeMap<JsonObjectMemberPath, JsonValue>,
}

impl Trial {
    /// 提案されたパラメータセットを使ってレイアウトを更新する
    pub fn apply_params_to_layout(&self, layout: &mut JsonValue) -> crate::Result<()> {
        for (path, value) in &self.params {
            *path.get_mut(layout).ok_or_else(|| {
                crate::Error::new(format!("target JSON path not found in layout: {}", path))
            })? = value.clone();
        }
        Ok(())
    }
}

/// トライアルの評価結果
#[derive(Debug, Clone, PartialEq)]
pub struct TrialValues {
    pub elapsed_seconds: f64,
    pub vmaf_mean: f64,
}

impl TrialValues {
    /// NSGA-II 用に最小化方向へ揃えた 2 目的の値を返す
    ///
    /// 合成時間は最小化目的なのでそのまま、VMAF 平均は最大化目的なので符号を反転して、
    /// 両目的を最小化として扱えるようにする。この方向の規約はここに一元化する。
    fn to_objectives(&self) -> [f64; 2] {
        [self.elapsed_seconds, -self.vmaf_mean]
    }
}

/// 探索空間
#[derive(Debug)]
pub struct SearchSpace {
    pub params: BTreeMap<JsonObjectMemberPath, ParameterDistribution>,
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

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ParameterDistribution {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        if value.kind().is_array() {
            Ok(Self::Categorical(value.try_into()?))
        } else if value.kind().is_object() {
            let min: JsonNumber = value.to_member("min")?.required()?.try_into()?;
            let max: JsonNumber = value.to_member("max")?.required()?.try_into()?;
            // 以降のサンプリング・交叉・突然変異は min <= max かつ有限であることを前提とする
            // （f64::clamp や rng::gen_range_i64 は min > max でパニックするため、ここで弾く）。
            // 整数同士は f64 への変換による精度落ちを避けて i64 のまま比較する。
            let valid = match (min, max) {
                (JsonNumber::Integer(lo), JsonNumber::Integer(hi)) => lo <= hi,
                _ => {
                    let (lo, hi) = (min.to_f64(), max.to_f64());
                    lo.is_finite() && hi.is_finite() && lo <= hi
                }
            };
            if !valid {
                return Err(value.invalid("numeric range must be finite and satisfy min <= max"));
            }
            Ok(Self::Numeric { min, max })
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
    pub params: BTreeMap<JsonObjectMemberPath, JsonValue>,
}

/// NSGA-II による多目的最適化のチューナー
///
/// `ask` (次のパラメータを問い合わせ) → 評価 → `tell` (結果を伝える) のループで使う。
/// 試行履歴は JSON Lines ファイルに永続化され、既存ファイルがあれば続きから最適化する。
#[derive(Debug)]
pub struct Tuner {
    jsonl_path: PathBuf,
    // ロックは保持するだけで Drop 時に解放される
    _lock: LockGuard,
    trials: Vec<TrialRecord>,
    // ask 済みだが tell 待ちのパラメータ (trial_number -> params)
    pending: BTreeMap<usize, BTreeMap<JsonObjectMemberPath, JsonValue>>,
    next_trial_number: usize,
    last_best_trials: Vec<BestTrial>,
}

impl Tuner {
    /// チューナーを開く
    ///
    /// `name` は探索履歴ファイルを区別するための名前で、作業ディレクトリ内の
    /// `<name>.jsonl` (履歴) と `<name>.lock` (多重起動防止) に対応する。
    /// ロックファイルを獲得し、既存の JSON Lines 履歴を読み込む。
    /// 履歴があれば trial 採番をその続きから行う。
    pub fn new(name: String, working_dir: PathBuf) -> crate::Result<Self> {
        let jsonl_path = working_dir.join(format!("{name}.jsonl"));
        let lock_path = working_dir.join(format!("{name}.lock"));
        let lock = LockGuard::acquire(lock_path)?;

        let trials = storage::load_trials(&jsonl_path)?;
        let next_trial_number = trials
            .iter()
            .map(|t| t.trial_number)
            .max()
            .map(|n| n + 1)
            .unwrap_or(0);

        Ok(Self {
            jsonl_path,
            _lock: lock,
            trials,
            pending: BTreeMap::new(),
            next_trial_number,
            last_best_trials: Vec::new(),
        })
    }

    /// 永続化済みのトライアル件数 (成功・失敗の両方を含む) を返す
    ///
    /// `--trial-count` の「合計到達ベース」判定に使う。
    pub fn trial_count(&self) -> usize {
        self.trials.len()
    }

    /// 次に探索すべきパラメータセットを問い合わせる
    pub fn ask(&mut self, search_space: &SearchSpace) -> crate::Result<Trial> {
        // 成功したトライアルだけを NSGA-II の個体として扱う
        let individuals = self.successful_individuals();

        let params = if individuals.len() < nsga2::POPULATION_SIZE {
            // 世代 0 が埋まるまでは一様ランダムサンプリング (初期集団)
            nsga2::sample_random(search_space)?
        } else {
            // 累積した成功個体から交叉 + 突然変異で子個体を生成する
            nsga2::generate_child(search_space, &individuals)?
        };

        let number = self.next_trial_number;
        self.next_trial_number += 1;
        self.pending.insert(number, params.clone());

        Ok(Trial { number, params })
    }

    /// 探索結果 (成功) を伝える
    pub fn tell(&mut self, trial_number: usize, values: &TrialValues) -> crate::Result<()> {
        // 目的値は有限前提 (NaN は NSGA-II の支配判定を壊し、その個体が rank 0 に居座る)。
        // hisui の評価値は VMAF・合成時間とも常に有限なので、debug ビルドでのみ番兵として検査する。
        debug_assert!(
            values.elapsed_seconds.is_finite() && values.vmaf_mean.is_finite(),
            "trial values must be finite"
        );
        let params = self.take_pending(trial_number)?;
        let record = TrialRecord {
            trial_number,
            params,
            result: TrialResult::Complete(values.clone()),
        };
        storage::append_trial(&self.jsonl_path, &record)?;
        self.trials.push(record);
        Ok(())
    }

    /// 探索結果 (失敗) を伝える
    pub fn tell_fail(&mut self, trial_number: usize) -> crate::Result<()> {
        let params = self.take_pending(trial_number)?;
        let record = TrialRecord {
            trial_number,
            params,
            result: TrialResult::Fail,
        };
        storage::append_trial(&self.jsonl_path, &record)?;
        self.trials.push(record);
        Ok(())
    }

    /// 現時点のパレートフロント (最適解の集合) を取得する
    ///
    /// 前回取得時から内容が変化したかどうかを `bool` で返す
    /// (呼び出し側が「更新時のみ表示」を制御するため)。
    pub fn get_best_trials(&mut self) -> crate::Result<(bool, Vec<BestTrial>)> {
        let best_trials = self.compute_best_trials();
        let updated = self.last_best_trials != best_trials;
        self.last_best_trials = best_trials.clone();
        Ok((updated, best_trials))
    }

    /// tell 待ちのパラメータを取り出す (pending から remove する)
    ///
    /// この remove は後続の append (失敗しうる) より前に行われるため、append が失敗すると
    /// pending を消費したままその試行が宙に浮く。ただし現在の呼び出し側は tell / tell_fail の
    /// エラーを `?` で伝播してプロセスを終了するので、この状態は観測されない。tell エラーを
    /// 捕捉して継続する呼び出し側を追加する場合は、remove を append 成功後に遅らせること。
    fn take_pending(
        &mut self,
        trial_number: usize,
    ) -> crate::Result<BTreeMap<JsonObjectMemberPath, JsonValue>> {
        self.pending.remove(&trial_number).ok_or_else(|| {
            crate::Error::new(format!(
                "tell called for unknown or already-told trial number: {trial_number}"
            ))
        })
    }

    /// 成功したトライアルを NSGA-II の個体に変換する
    fn successful_individuals(&self) -> Vec<Individual> {
        self.trials
            .iter()
            .filter_map(|t| match &t.result {
                TrialResult::Complete(values) => Some(Individual::new(t.params.clone(), values)),
                TrialResult::Fail => None,
            })
            .collect()
    }

    /// 成功トライアルからパレートフロント上のものを抽出する
    fn compute_best_trials(&self) -> Vec<BestTrial> {
        let completed: Vec<(&TrialRecord, &TrialValues)> = self
            .trials
            .iter()
            .filter_map(|t| match &t.result {
                TrialResult::Complete(values) => Some((t, values)),
                TrialResult::Fail => None,
            })
            .collect();
        if completed.is_empty() {
            return Vec::new();
        }

        // 最小化方向に揃えた目的値で非劣ソートし、rank 0 (フロント) を取り出す
        let points: Vec<[f64; 2]> = completed.iter().map(|(_, v)| v.to_objectives()).collect();
        let ranks = nsga2::non_dominated_sort(&points);

        completed
            .iter()
            .zip(ranks.iter())
            .filter(|(_, rank)| **rank == 0)
            .map(|(entry, _)| {
                let (record, values) = entry;
                BestTrial {
                    number: record.trial_number,
                    values: (*values).clone(),
                    params: record.params.clone(),
                }
            })
            .collect()
    }
}
