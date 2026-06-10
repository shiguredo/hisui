use std::collections::BTreeMap;

use crate::tune::json_value::{JsonNumber, JsonObjectMemberPath, JsonValue};
use crate::tune::rng;
use crate::tune::{ParameterDistribution, SearchSpace, TrialValues};

// NSGA-II (Deb et al., 2002 "A Fast and Elitist Multiobjective Genetic Algorithm: NSGA-II") の自前実装
//
// hisui が使うのは 2 目的 (合成時間 minimize / VMAF 平均 maximize) の多目的最適化のみ。
// 非劣ソート・混雑度距離・混雑度比較・遺伝演算子は論文に従う。一方、論文の世代交代
// (R_t = P_t ∪ Q_t から生存選択する) は採らず、累積した全成功個体を母集団とする方式に
// 簡略化している (select_parents / generate_child 参照)。

/// 集団サイズ
///
/// 誘導探索 (交叉・突然変異) は成功試行がこの数に達してから始まり、それまでは一様ランダム
/// サンプリングになる。hisui では 1 試行が合成 + VMAF 評価で高コストなため、限られた試行数でも
/// 誘導フェーズに早く入れるよう、論文の実験値 (100) より小さめにしている。
pub const POPULATION_SIZE: usize = 20;

// 以下の遺伝演算パラメータは論文の real-coded NSGA-II 実験で使われた値に合わせている。

/// 交叉確率 (論文の p_c)
const CROSSOVER_PROB: f64 = 0.9;

/// SBX の分布指数 (論文の eta_c)
const SBX_ETA: f64 = 20.0;

/// polynomial mutation の分布指数 (論文の eta_m)
const MUTATION_ETA: f64 = 20.0;

/// NSGA-II が扱う 1 個体 (パラメータと、最小化方向に揃えた目的値)
#[derive(Debug, Clone)]
pub struct Individual {
    pub params: BTreeMap<JsonObjectMemberPath, JsonValue>,
    /// 最小化方向に揃えた 2 目的の値 (`[elapsed_seconds, -vmaf_mean]`)
    pub objectives: [f64; 2],
}

impl Individual {
    /// 評価値を最小化方向に揃えた個体を作る
    pub fn new(params: BTreeMap<JsonObjectMemberPath, JsonValue>, values: &TrialValues) -> Self {
        Self {
            params,
            objectives: values.to_objectives(),
        }
    }
}

/// 点 `a` が点 `b` を (パレート) 支配するかどうかを返す
///
/// 両目的とも最小化前提。`a` が全目的で `b` 以下、かつ少なくとも 1 目的で `b` 未満なら支配する。
///
/// NOTE: 目的値は有限前提。NaN を渡すと `<` / `<=` が常に false になり、その点が誰にも
/// 支配されず rank 0 (パレートフロント) に居座る。hisui では VMAF・合成時間とも有限値しか
/// 流れないため検査しないが、有限性が保証されない目的を足す際は注意すること。
pub fn dominates(a: &[f64; 2], b: &[f64; 2]) -> bool {
    let all_le = a[0] <= b[0] && a[1] <= b[1];
    let any_lt = a[0] < b[0] || a[1] < b[1];
    all_le && any_lt
}

/// 非劣ソートを行い、各点のフロント番号 (rank) を返す
///
/// 論文の fast-non-dominated-sort に相当する。各点について「自分を支配する点の数」(論文の
/// n_p) と「自分が支配する点の集合」(論文の S_p) を求め、n_p == 0 の点を最良フロントとして
/// 順に剥がしていく。rank 0 が最良フロント (論文は第 1 フロントを rank 1 とするが 0 始まりにする)。
/// 返り値は入力と同じ順序の rank 配列。
pub fn non_dominated_sort(points: &[[f64; 2]]) -> Vec<usize> {
    let n = points.len();
    let mut rank = vec![0usize; n];

    // domination_count が論文の n_p、dominated_set が論文の S_p
    let mut domination_count = vec![0usize; n];
    let mut dominated_set: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut current_front: Vec<usize> = Vec::new();

    for p in 0..n {
        for q in 0..n {
            if p == q {
                continue;
            }
            if dominates(&points[p], &points[q]) {
                dominated_set[p].push(q);
            } else if dominates(&points[q], &points[p]) {
                domination_count[p] += 1;
            }
        }
        if domination_count[p] == 0 {
            rank[p] = 0;
            current_front.push(p);
        }
    }

    // フロントを 1 つずつ剥がしていく
    let mut front_index = 0;
    while !current_front.is_empty() {
        let mut next_front: Vec<usize> = Vec::new();
        for &p in &current_front {
            for &q in &dominated_set[p] {
                domination_count[q] -= 1;
                if domination_count[q] == 0 {
                    rank[q] = front_index + 1;
                    next_front.push(q);
                }
            }
        }
        front_index += 1;
        current_front = next_front;
    }

    rank
}

/// 1 つのフロント内の各点の混雑度距離を返す
///
/// 論文の crowding-distance-assignment に相当する。各目的軸をそのフロント内の min-max
/// (論文の f_m^max / f_m^min) で正規化したうえで隣接点の距離を加算する (2 目的のスケール差の
/// 影響を排除する)。各目的軸での端点 (最小・最大) の距離は無限大とする。
pub fn crowding_distance(front: &[[f64; 2]]) -> Vec<f64> {
    let n = front.len();
    let mut distance = vec![0.0f64; n];
    if n == 0 {
        return distance;
    }
    if n <= 2 {
        // 端点しかないので全て無限大扱いにする
        for d in distance.iter_mut() {
            *d = f64::INFINITY;
        }
        return distance;
    }

    for m in [0usize, 1] {
        // この目的軸の値でインデックスをソートする
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&i, &j| front[i][m].total_cmp(&front[j][m]));

        // 端点は無限大
        distance[order[0]] = f64::INFINITY;
        distance[order[n - 1]] = f64::INFINITY;

        let min = front[order[0]][m];
        let max = front[order[n - 1]][m];
        let range = max - min;
        if range == 0.0 {
            // この軸では差がないので寄与なし
            continue;
        }

        for k in 1..n - 1 {
            let prev = front[order[k - 1]][m];
            let next = front[order[k + 1]][m];
            distance[order[k]] += (next - prev) / range;
        }
    }

    distance
}

/// 累積した成功個体から親世代 (上位 `POPULATION_SIZE` 個) を選抜する
///
/// 非劣ソートでフロントに分け、混雑度距離も加味して上位を選ぶ。
/// これまでの全成功個体をアーカイブとして母集団に使うので、見つかったパレート最適解は
/// 結果 (best_trials) から失われない。集団を N 個に保つ論文の生存選択 (P_t ∪ Q_t, elitism)
/// とはこの点で機構が異なる。フロントが POPULATION_SIZE を超える場合の混雑度距離による間引き
/// は標準 NSGA-II と同じで、フロント内部の解は親から外れうる (各目的の端点は必ず残る)。
/// issue 0010 参照。
fn select_parents(individuals: &[Individual]) -> Vec<Individual> {
    if individuals.len() <= POPULATION_SIZE {
        return individuals.to_vec();
    }

    let points: Vec<[f64; 2]> = individuals.iter().map(|i| i.objectives).collect();
    let ranks = non_dominated_sort(&points);

    // rank ごとにインデックスをまとめる
    let max_rank = ranks.iter().copied().max().unwrap_or(0);
    let mut fronts: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for (idx, &r) in ranks.iter().enumerate() {
        fronts[r].push(idx);
    }

    let mut selected: Vec<Individual> = Vec::new();
    for front in fronts {
        if selected.len() + front.len() <= POPULATION_SIZE {
            // フロントごと丸ごと採用できる
            for idx in front {
                selected.push(individuals[idx].clone());
            }
        } else {
            // 入りきらないフロントは混雑度距離が大きい順に詰める
            let front_points: Vec<[f64; 2]> = front.iter().map(|&i| points[i]).collect();
            let dist = crowding_distance(&front_points);
            let mut ordered: Vec<(usize, f64)> = front.iter().copied().zip(dist).collect();
            ordered.sort_by(|a, b| b.1.total_cmp(&a.1));
            let remaining = POPULATION_SIZE - selected.len();
            for (idx, _) in ordered.into_iter().take(remaining) {
                selected.push(individuals[idx].clone());
            }
            break;
        }
        if selected.len() == POPULATION_SIZE {
            break;
        }
    }

    selected
}

/// binary トーナメント選択で 1 個体のインデックスを選ぶ
///
/// 異なる 2 個体を引き、crowded-comparison operator ([`crowded_compare`]) で優劣を決める。
fn tournament_select(ranks: &[usize], distances: &[f64]) -> crate::Result<usize> {
    let n = ranks.len();
    let a = rng::gen_index(n)?;
    // 異なる 2 個体を競わせるため b は a と別のインデックスにする
    // (集団が 1 個体しかない場合だけは a 自身を返す。今の呼び出し経路では到達しないが、
    // rejection ループが無限に回らないようにするための防御)
    let b = if n <= 1 {
        a
    } else {
        let mut b = rng::gen_index(n)?;
        while b == a {
            b = rng::gen_index(n)?;
        }
        b
    };
    // crowded-comparison で優劣を決める。完全同値 (rank も距離も同じ) のときは a を採る。
    match crowded_compare(ranks[a], distances[a], ranks[b], distances[b]) {
        std::cmp::Ordering::Greater => Ok(b),
        _ => Ok(a),
    }
}

/// crowded-comparison operator (論文の ≻n) で 2 個体の優劣を比較する
///
/// rank が小さい方を優先し、同 rank なら混雑度距離が大きい方を優先する。a を基準とした
/// 順序を返す (`Less` なら a 優先、`Greater` なら b 優先、`Equal` なら rank・距離とも同値)。
/// rng を使わない純粋関数なので、選抜の優劣判定だけを切り出して単体テストで検証できる。
fn crowded_compare(rank_a: usize, dist_a: f64, rank_b: usize, dist_b: f64) -> std::cmp::Ordering {
    // rank は昇順 (小さいほど良い)、同 rank では距離の降順 (大きいほど良い)
    rank_a.cmp(&rank_b).then(dist_b.total_cmp(&dist_a))
}

/// 探索空間から各パラメータを一様ランダムにサンプリングする (初期集団用)
pub fn sample_random(
    search_space: &SearchSpace,
) -> crate::Result<BTreeMap<JsonObjectMemberPath, JsonValue>> {
    let mut params = BTreeMap::new();
    for (path, dist) in &search_space.params {
        params.insert(path.clone(), sample_one(dist)?);
    }
    Ok(params)
}

/// 1 つの分布から一様ランダムに値をサンプリングする
fn sample_one(dist: &ParameterDistribution) -> crate::Result<JsonValue> {
    match dist {
        ParameterDistribution::Numeric { min, max } => match (min, max) {
            // 両端が整数なら整数として扱う
            (JsonNumber::Integer(lo), JsonNumber::Integer(hi)) => {
                Ok(JsonValue::Integer(rng::gen_range_i64(*lo, *hi)?))
            }
            _ => {
                let lo = min.to_f64();
                let hi = max.to_f64();
                Ok(JsonValue::Float(lo + rng::gen_unit_f64()? * (hi - lo)))
            }
        },
        ParameterDistribution::Categorical(choices) => {
            if choices.is_empty() {
                return Err(crate::Error::new("categorical distribution has no choices"));
            }
            let idx = rng::gen_index(choices.len())?;
            Ok(choices[idx].clone())
        }
    }
}

/// 累積した成功個体から、交叉 + 突然変異で子個体のパラメータを 1 つ生成する
pub fn generate_child(
    search_space: &SearchSpace,
    individuals: &[Individual],
) -> crate::Result<BTreeMap<JsonObjectMemberPath, JsonValue>> {
    // 親世代を選抜し、その rank / 混雑度距離をトーナメント選択用に求める
    let parents = select_parents(individuals);
    let points: Vec<[f64; 2]> = parents.iter().map(|i| i.objectives).collect();
    let ranks = non_dominated_sort(&points);

    // 混雑度距離は rank (フロント) ごとに計算する
    let mut distances = vec![0.0f64; parents.len()];
    let max_rank = ranks.iter().copied().max().unwrap_or(0);
    for r in 0..=max_rank {
        let front_idx: Vec<usize> = (0..parents.len()).filter(|&i| ranks[i] == r).collect();
        let front_points: Vec<[f64; 2]> = front_idx.iter().map(|&i| points[i]).collect();
        let front_dist = crowding_distance(&front_points);
        for (k, &i) in front_idx.iter().enumerate() {
            distances[i] = front_dist[k];
        }
    }

    // 2 親をトーナメント選択する
    let p1 = &parents[tournament_select(&ranks, &distances)?];
    let p2 = &parents[tournament_select(&ranks, &distances)?];

    // 交叉
    let mut child = if rng::gen_bool(CROSSOVER_PROB)? {
        crossover(search_space, &p1.params, &p2.params)?
    } else {
        p1.params.clone()
    };

    // 突然変異 (各パラメータを確率 1 / パラメータ数 で変異させる。論文の p_m = 1/n に相当)
    let mutation_prob = if search_space.params.is_empty() {
        0.0
    } else {
        1.0 / search_space.params.len() as f64
    };
    for (path, dist) in &search_space.params {
        if rng::gen_bool(mutation_prob)?
            && let Some(value) = child.get_mut(path)
        {
            *value = mutate_one(dist, value)?;
        }
    }

    Ok(child)
}

/// 2 親のパラメータ集合を交叉して子のパラメータ集合を作る
fn crossover(
    search_space: &SearchSpace,
    p1: &BTreeMap<JsonObjectMemberPath, JsonValue>,
    p2: &BTreeMap<JsonObjectMemberPath, JsonValue>,
) -> crate::Result<BTreeMap<JsonObjectMemberPath, JsonValue>> {
    let mut child = BTreeMap::new();
    for (path, dist) in &search_space.params {
        // 親に対応する値がなければ (理論上は起きない) ランダムサンプリングで埋める
        let (Some(v1), Some(v2)) = (p1.get(path), p2.get(path)) else {
            child.insert(path.clone(), sample_one(dist)?);
            continue;
        };
        let value = match dist {
            ParameterDistribution::Numeric { min, max } => {
                crossover_numeric(&NumericRange::new(min, max), v1, v2)?
            }
            // カテゴリカルは SBX を適用できないので uniform crossover (親のどちらかを選ぶ)
            ParameterDistribution::Categorical(_) => {
                if rng::gen_bool(0.5)? {
                    v1.clone()
                } else {
                    v2.clone()
                }
            }
        };
        child.insert(path.clone(), value);
    }
    Ok(child)
}

/// 数値パラメータの SBX 交叉
fn crossover_numeric(
    range: &NumericRange,
    v1: &JsonValue,
    v2: &JsonValue,
) -> crate::Result<JsonValue> {
    let x1 = json_value_to_f64(v1).unwrap_or(range.lo);
    let x2 = json_value_to_f64(v2).unwrap_or(range.hi);

    // SBX (Simulated Binary Crossover)。生成される 2 子のうち 1 つを採用する。
    //
    // これは Deb & Agrawal (1995) の原型 SBX で、変数境界を考慮しない (unbounded)。
    // NSGA-II 参照実装は親と境界の距離で beta を補正し範囲外の子を出にくくするが、
    // ここでは簡略化のため補正せず、範囲外に出た子は finalize の clamp で境界へ丸める
    // (端点に確率質量が偏るが、極小レンジを特別扱いしない方針。issue 0010 参照)。
    // なお polynomial mutation (mutate_one) は境界考慮版を使っている。
    let u = rng::gen_unit_f64()?;
    let beta = if u <= 0.5 {
        (2.0 * u).powf(1.0 / (SBX_ETA + 1.0))
    } else {
        (1.0 / (2.0 * (1.0 - u))).powf(1.0 / (SBX_ETA + 1.0))
    };
    let c1 = 0.5 * ((1.0 + beta) * x1 + (1.0 - beta) * x2);
    let c2 = 0.5 * ((1.0 - beta) * x1 + (1.0 + beta) * x2);
    let child = if rng::gen_bool(0.5)? { c1 } else { c2 };

    Ok(range.finalize(child))
}

/// 数値パラメータの polynomial mutation
fn mutate_one(dist: &ParameterDistribution, value: &JsonValue) -> crate::Result<JsonValue> {
    match dist {
        ParameterDistribution::Numeric { min, max } => {
            let range = NumericRange::new(min, max);
            let (lo, hi) = (range.lo, range.hi);
            if hi <= lo {
                // レンジが無いので変異しようがない
                return Ok(range.finalize(lo));
            }
            let x = json_value_to_f64(value).unwrap_or(lo).clamp(lo, hi);

            let delta1 = (x - lo) / (hi - lo);
            let delta2 = (hi - x) / (hi - lo);
            let u = rng::gen_unit_f64()?;
            let mut_pow = 1.0 / (MUTATION_ETA + 1.0);
            let deltaq = if u < 0.5 {
                let xy = 1.0 - delta1;
                let val = 2.0 * u + (1.0 - 2.0 * u) * xy.powf(MUTATION_ETA + 1.0);
                val.powf(mut_pow) - 1.0
            } else {
                let xy = 1.0 - delta2;
                let val = 2.0 * (1.0 - u) + 2.0 * (u - 0.5) * xy.powf(MUTATION_ETA + 1.0);
                1.0 - val.powf(mut_pow)
            };
            let mutated = x + deltaq * (hi - lo);
            Ok(range.finalize(mutated))
        }
        // カテゴリカルは選択肢集合から一様再サンプリングする
        ParameterDistribution::Categorical(_) => sample_one(dist),
    }
}

/// 数値分布の範囲。交叉・突然変異で共通して使う。
struct NumericRange {
    lo: f64,
    hi: f64,
    is_integer: bool,
}

impl NumericRange {
    /// 探索空間の数値分布から範囲を作る
    fn new(min: &JsonNumber, max: &JsonNumber) -> Self {
        Self {
            lo: min.to_f64(),
            hi: max.to_f64(),
            is_integer: matches!((min, max), (JsonNumber::Integer(_), JsonNumber::Integer(_))),
        }
    }

    /// SBX / mutation の結果を範囲内にクランプし、整数なら丸めて `JsonValue` にする
    fn finalize(&self, value: f64) -> JsonValue {
        let clamped = value.clamp(self.lo, self.hi);
        if self.is_integer {
            JsonValue::Integer(clamped.round() as i64)
        } else {
            JsonValue::Float(clamped)
        }
    }
}

/// `JsonValue` が数値なら `f64` として取り出す
fn json_value_to_f64(v: &JsonValue) -> Option<f64> {
    match v {
        JsonValue::Integer(i) => Some(*i as f64),
        JsonValue::Float(f) => Some(*f),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;
    use std::collections::BTreeMap;

    use super::*;

    // 目的値だけを持つ個体を作る (params は選抜に影響しないので空にする)
    fn individual(objective0: f64, objective1: f64) -> Individual {
        Individual {
            params: BTreeMap::new(),
            objectives: [objective0, objective1],
        }
    }

    // 親集団に「第 1 目的が指定の整数値」を持つ個体が含まれるかを返す。
    // テストの目的値はすべて整数なので、float の == を避けて i64 に落として厳密比較する。
    fn contains_objective0(parents: &[Individual], objective0: i64) -> bool {
        parents.iter().any(|p| p.objectives[0] as i64 == objective0)
    }

    // crowded-comparison は rank が小さい方を、混雑度距離に関係なく優先する
    #[test]
    fn crowded_compare_prioritizes_smaller_rank() {
        // a は rank 0 で距離 0、b は rank 1 で距離無限大。それでも rank の小さい a が勝つ
        assert_eq!(crowded_compare(0, 0.0, 1, f64::INFINITY), Ordering::Less);
        // 引数を入れ替えると rank 0 側 (b) が勝つ
        assert_eq!(crowded_compare(1, f64::INFINITY, 0, 0.0), Ordering::Greater);
    }

    // 同 rank では混雑度距離が大きい方を優先し、rank も距離も同値なら Equal を返す
    #[test]
    fn crowded_compare_prioritizes_larger_distance_within_same_rank() {
        assert_eq!(crowded_compare(2, 5.0, 2, 1.0), Ordering::Less);
        assert_eq!(crowded_compare(2, 1.0, 2, 5.0), Ordering::Greater);
        assert_eq!(crowded_compare(2, 1.0, 2, 1.0), Ordering::Equal);
    }

    // 個体数が POPULATION_SIZE 以下なら、全個体がそのまま親になる (早期 return 経路)
    #[test]
    fn select_parents_keeps_all_within_population_size() {
        // 互いに非劣な個体を POPULATION_SIZE 個ちょうど作る
        let individuals: Vec<Individual> = (0..POPULATION_SIZE)
            .map(|i| individual(i as f64, (POPULATION_SIZE - i) as f64))
            .collect();
        let parents = select_parents(&individuals);
        assert_eq!(
            parents.len(),
            POPULATION_SIZE,
            "POPULATION_SIZE 以下なら全個体が親になること"
        );
    }

    // 上位フロントを丸ごと採用する分岐を踏み、支配された劣個体が淘汰されることを確認する (elitism)
    #[test]
    fn select_parents_drops_dominated_individual() {
        // 互いに非劣な rank 0 フロントを POPULATION_SIZE 個作る
        let mut individuals: Vec<Individual> = (0..POPULATION_SIZE)
            .map(|i| individual(i as f64, (POPULATION_SIZE - i) as f64))
            .collect();
        // どの rank 0 個体にも両目的で劣る (= 支配される) 個体を 1 つ加える
        individuals.push(individual(1_000.0, 1_000.0));

        let parents = select_parents(&individuals);

        assert_eq!(
            parents.len(),
            POPULATION_SIZE,
            "親は POPULATION_SIZE 個に絞られること"
        );
        assert!(
            !contains_objective0(&parents, 1_000),
            "支配された劣個体は親集団から淘汰されること"
        );
    }

    // 上位フロントを全採用したうえで、入りきらない下位フロントを混雑度距離で詰める分岐を踏む。
    // 大域最良 (rank 0) と下位フロントの端点 (混雑度距離が無限大) が残ることを確認する。
    #[test]
    fn select_parents_packs_overflow_front_by_crowding_distance() {
        // rank 0 は [0,0] の 1 個だけ ([0,0] が以降の全個体を支配する)
        let mut individuals = vec![individual(0.0, 0.0)];
        // rank 1 は互いに非劣な直線上の点を POPULATION_SIZE より多く作る
        let front1_len = POPULATION_SIZE + 5;
        individuals
            .extend((0..front1_len).map(|i| individual((i + 1) as f64, (front1_len - i) as f64)));

        let parents = select_parents(&individuals);

        assert_eq!(
            parents.len(),
            POPULATION_SIZE,
            "親は POPULATION_SIZE 個に絞られること"
        );
        // rank 0 の大域最良は必ず残る (elitism)
        assert!(
            contains_objective0(&parents, 0),
            "rank 0 の大域最良は必ず親に残ること"
        );
        // rank 1 フロントの両端点 (混雑度距離が無限大) も残る
        assert!(
            contains_objective0(&parents, 1),
            "下位フロントの端点 (第 1 目的が最小) は残ること"
        );
        assert!(
            contains_objective0(&parents, front1_len as i64),
            "下位フロントの端点 (第 1 目的が最大) は残ること"
        );
    }

    // NumericRange::finalize は整数レンジでは範囲内にクランプして最近接へ丸め、Integer を返す
    #[test]
    fn numeric_range_finalize_rounds_and_clamps_integer() {
        let range = NumericRange::new(&JsonNumber::Integer(0), &JsonNumber::Integer(10));
        // 範囲内の小数は最近接整数へ丸める (round は 0.5 を 0 から遠い側へ丸める)
        assert_eq!(range.finalize(3.4), JsonValue::Integer(3));
        assert_eq!(range.finalize(3.6), JsonValue::Integer(4));
        assert_eq!(range.finalize(2.5), JsonValue::Integer(3));
        // 範囲外は端点へクランプしてから丸める
        assert_eq!(range.finalize(-5.0), JsonValue::Integer(0));
        assert_eq!(range.finalize(100.0), JsonValue::Integer(10));
    }

    // 浮動小数レンジでは丸めず、範囲内にクランプして Float を返す
    #[test]
    fn numeric_range_finalize_clamps_float_without_rounding() {
        let range = NumericRange::new(&JsonNumber::Float(0.0), &JsonNumber::Float(1.0));
        // 範囲内はそのまま返す
        assert_eq!(range.finalize(0.25), JsonValue::Float(0.25));
        // 範囲外は端点へクランプする
        assert_eq!(range.finalize(-1.0), JsonValue::Float(0.0));
        assert_eq!(range.finalize(2.0), JsonValue::Float(1.0));
    }
}
