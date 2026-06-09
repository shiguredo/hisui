use hisui::tune::json_value::{JsonNumber, JsonObjectMemberPath, JsonValue};
use hisui::tune::nsga2::{
    Individual, crowding_distance, dominates, generate_child, non_dominated_sort, sample_random,
};
use hisui::tune::{ParameterDistribution, SearchSpace};
use proptest::prelude::*;

// NSGA-II のコア純粋関数 (非劣ソート・混雑度距離) の不変条件を検証する。
//
// 同値・タイ・境界 (目的値が等しいケース) でドミナンス判定の `<` / `<=` 取り違えが
// 起きやすいため、これらを乱数生成された点集合で重点的に検証する。

// 2 目的の点集合を生成する戦略
//
// 値の範囲を狭めにとることで、同値・タイのケースが頻繁に現れるようにしている
// (ドミナンス判定の境界バグをあぶり出すため)。
fn points_strategy() -> impl Strategy<Value = Vec<[f64; 2]>> {
    let coord = -5.0f64..5.0f64;
    prop::collection::vec((coord.clone(), coord).prop_map(|(a, b)| [a, b]), 1..40)
}

proptest! {
    // rank 0 (パレートフロント) 上の任意の 2 点は互いに非劣である
    #[test]
    fn pareto_front_mutually_nondominated(points in points_strategy()) {
        let ranks = non_dominated_sort(&points);
        let front0: Vec<usize> = ranks
            .iter()
            .enumerate()
            .filter(|(_, r)| **r == 0)
            .map(|(i, _)| i)
            .collect();

        // フロント上の任意の 2 点 a, b について、どちらも他方を支配しない
        for &a in &front0 {
            for &b in &front0 {
                if a == b {
                    continue;
                }
                prop_assert!(!dominates(&points[a], &points[b]));
            }
        }
    }

    // rank が 1 以上の点は、必ず 1 つ下の rank の点に支配されている
    #[test]
    fn dominated_by_better_front(points in points_strategy()) {
        let ranks = non_dominated_sort(&points);
        for i in 0..points.len() {
            if ranks[i] == 0 {
                continue;
            }
            // rank[i] - 1 の点のうち少なくとも 1 つが i を支配する
            let dominated_by_prev_front = (0..points.len()).any(|j| {
                ranks[j] + 1 == ranks[i] && dominates(&points[j], &points[i])
            });
            prop_assert!(dominated_by_prev_front);
        }
    }

    // rank が小さい点は、より大きい rank の点に支配されない
    #[test]
    fn smaller_rank_not_dominated_by_larger(points in points_strategy()) {
        let ranks = non_dominated_sort(&points);
        for i in 0..points.len() {
            for j in 0..points.len() {
                if ranks[i] < ranks[j] {
                    prop_assert!(!dominates(&points[j], &points[i]));
                }
            }
        }
    }

    // 混雑度距離は入力と同じ長さを返し、端点は無限大・それ以外は非負の有限値になる
    #[test]
    fn crowding_distance_basic_properties(points in points_strategy()) {
        let dist = crowding_distance(&points);
        prop_assert_eq!(dist.len(), points.len());

        let n = points.len();
        if n >= 3 {
            // 各目的軸の端点が無限大になるため、無限大は最低 2 個存在する
            let inf_count = dist.iter().filter(|d| d.is_infinite()).count();
            prop_assert!(inf_count >= 2);
        }

        // 無限大でない距離は非負かつ有限
        for d in &dist {
            if !d.is_infinite() {
                prop_assert!(d.is_finite());
                prop_assert!(*d >= 0.0);
            }
        }
    }
}

// `dominates` の定義そのものの健全性 (反射律・非対称性) を確認する単体的なプロパティ
proptest! {
    // 同一点は自分自身を支配しない (狭義のパレート支配)
    #[test]
    fn point_does_not_dominate_itself(a in -5.0f64..5.0, b in -5.0f64..5.0) {
        let p = [a, b];
        prop_assert!(!dominates(&p, &p));
    }

    // a が b を支配するなら、b は a を支配しない (非対称性)
    #[test]
    fn domination_is_asymmetric(
        a0 in -5.0f64..5.0, a1 in -5.0f64..5.0,
        b0 in -5.0f64..5.0, b1 in -5.0f64..5.0,
    ) {
        let a = [a0, a1];
        let b = [b0, b1];
        if dominates(&a, &b) {
            prop_assert!(!dominates(&b, &a));
        }
    }

    // a が b を、b が c を支配するなら、a は c を支配する (推移律)
    #[test]
    fn dominance_is_transitive(
        a0 in -5.0f64..5.0, a1 in -5.0f64..5.0,
        b0 in -5.0f64..5.0, b1 in -5.0f64..5.0,
        c0 in -5.0f64..5.0, c1 in -5.0f64..5.0,
    ) {
        let (a, b, c) = ([a0, a1], [b0, b1], [c0, c1]);
        if dominates(&a, &b) && dominates(&b, &c) {
            prop_assert!(dominates(&a, &c));
        }
    }

    // 非劣ソートの rank は 0 から始まり歯抜けがない
    // (フロントを 1 つずつ剥がす実装の構造的な不変条件)
    #[test]
    fn ranks_are_contiguous(points in points_strategy()) {
        let ranks = non_dominated_sort(&points);
        if let Some(&max_rank) = ranks.iter().max() {
            for r in 0..=max_rank {
                prop_assert!(ranks.contains(&r), "rank {r} が歯抜けになっている");
            }
        }
    }
}

// 探索空間 (SearchSpace) を生成する戦略。
// 整数 Numeric・浮動小数 Numeric・カテゴリカルを混在させ、min <= max を保証する。
fn distribution_strategy() -> impl Strategy<Value = ParameterDistribution> {
    prop_oneof![
        (-1000i64..1000, 0i64..1000).prop_map(|(lo, span)| ParameterDistribution::Numeric {
            min: JsonNumber::Integer(lo),
            max: JsonNumber::Integer(lo + span),
        }),
        (-1000.0f64..1000.0, 0.0f64..1000.0).prop_map(|(lo, span)| {
            ParameterDistribution::Numeric {
                min: JsonNumber::Float(lo),
                max: JsonNumber::Float(lo + span),
            }
        }),
        prop::collection::vec("[a-z]{1,4}".prop_map(JsonValue::String), 1..5)
            .prop_map(ParameterDistribution::Categorical),
    ]
}

fn search_space_strategy() -> impl Strategy<Value = SearchSpace> {
    prop::collection::btree_map(
        "[a-z][a-z0-9]{0,4}".prop_map(|s| {
            s.parse::<JsonObjectMemberPath>()
                .expect("JsonObjectMemberPath::from_str is infallible")
        }),
        distribution_strategy(),
        1..6,
    )
    .prop_map(|params| SearchSpace { params })
}

// 値が分布の制約 (整数分布なら範囲内の整数、浮動小数分布なら範囲内の浮動小数、
// カテゴリカルなら選択肢のいずれか) を満たすことを検証する。
fn assert_value_in_distribution(
    value: &JsonValue,
    dist: &ParameterDistribution,
) -> Result<(), TestCaseError> {
    match dist {
        ParameterDistribution::Numeric { min, max } => match (min, max) {
            (JsonNumber::Integer(lo), JsonNumber::Integer(hi)) => match value {
                JsonValue::Integer(v) => {
                    prop_assert!(lo <= v && v <= hi, "整数 {v} が [{lo}, {hi}] の範囲外");
                }
                other => prop_assert!(false, "整数分布なのに整数以外: {other:?}"),
            },
            _ => {
                let (lo, hi) = (min.to_f64(), max.to_f64());
                match value {
                    JsonValue::Float(v) => {
                        prop_assert!(
                            lo <= *v && *v <= hi,
                            "浮動小数 {v} が [{lo}, {hi}] の範囲外"
                        );
                    }
                    other => prop_assert!(false, "浮動小数分布なのに浮動小数以外: {other:?}"),
                }
            }
        },
        ParameterDistribution::Categorical(choices) => {
            prop_assert!(
                choices.contains(value),
                "カテゴリ値 {value:?} が選択肢に含まれない"
            );
        }
    }
    Ok(())
}

proptest! {
    // sample_random の結果は探索空間のキー集合と一致し、各値が分布制約を満たす
    #[test]
    fn sample_random_respects_space(space in search_space_strategy()) {
        let params = sample_random(&space).expect("サンプリングは成功する");
        prop_assert_eq!(params.len(), space.params.len());
        for (path, dist) in &space.params {
            let value = params.get(path).expect("各パラメータがサンプリングされる");
            assert_value_in_distribution(value, dist)?;
        }
    }

    // generate_child の結果も探索空間の制約を満たす。
    // 個体数を 1..120 で振り、select_parents の「全件採用」と「混雑度で詰める」両分岐を踏む。
    #[test]
    fn generate_child_respects_space(space in search_space_strategy(), n in 1usize..120) {
        let individuals: Vec<Individual> = (0..n)
            .map(|i| {
                let params = sample_random(&space).expect("サンプリングは成功する");
                // 目的値は全個体が互いに非劣になるよう直線上に配置する
                Individual {
                    params,
                    objectives: [i as f64, (n - i) as f64],
                }
            })
            .collect();
        let child = generate_child(&space, &individuals).expect("子個体生成は成功する");
        prop_assert_eq!(child.len(), space.params.len());
        for (path, dist) in &space.params {
            let value = child.get(path).expect("各パラメータが生成される");
            assert_value_in_distribution(value, dist)?;
        }
    }
}
